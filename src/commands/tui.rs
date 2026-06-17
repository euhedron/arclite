//! `arc tui` — the human-facing, inline **cockpit** over arclite (the CLI stays the agent/automation
//! interface). arc is not a chat and has no sessions: this is a launchpad for firing targeted audit /
//! analysis runs and observing them, running *alongside* the dev agents — so the shape borrows the
//! *mechanics* of the Claude Code / Codex TUIs (inline viewport, a `/` command palette, a persistent
//! footer that carries run status everywhere, a live status view) but not their conversation model.
//!
//! Rendering is **inline** (stock ratatui `Viewport::Inline` — drawn in the normal terminal buffer, the
//! shell's scrollback preserved above; NOT an alt-screen takeover, matching both reference CLIs).
//!
//! Runtime shape (gitui's model): a dedicated **input thread** (blocking `event::read`) and a **tick
//! thread** both feed one `std::sync::mpsc<Msg>`; the main loop blocks on it, applies the message via
//! [`update`], and redraws once. A `Tick` re-reads live state — so views refresh in place rather than
//! the user re-running `arc status`, and the footer's active-run count stays current on every view.
//!
//! State is a route plus optional overlays (the `/` palette, the launch gate): [`render`] is a pure
//! function of [`App`], tested headlessly with `TestBackend`; the interactive loop itself needs a real
//! terminal. Launching a run spawns the `arc` binary as a subprocess and renders its own output — the
//! cockpit is a second front-end over the CLI, not a reimplementation of it.
#![deny(clippy::print_stdout, clippy::print_stderr)] // never print while the TUI owns the terminal

use std::io::IsTerminal;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph, Row, Table};
use ratatui::{Frame, TerminalOptions, Viewport};

use crate::cli::{GlobalArgs, TuiArgs};
use crate::runs::ActiveRun;

/// Default seconds between live refreshes when `--interval` is omitted. One second keeps the registry
/// view current without busy-spinning; `--interval` overrides it (and is echoed in `--help`).
pub const DEFAULT_INTERVAL_SECS: f64 = 1.0;

/// Lines reserved for the inline live region. Tall enough for a section plus the global footer; the
/// shell's scrollback stays visible above it. (Dynamic height that grows with the section is a known
/// refinement.)
const VIEWPORT_HEIGHT: u16 = 16;

/// Width of the command-palette popup — wide enough for the longest command name plus its one-line
/// description without dominating a narrow terminal ([`centered`] clamps it to the available width).
const PALETTE_WIDTH: u16 = 56;

/// Width of the command-name column in the palette list, so each name's description aligns.
const PALETTE_NAME_WIDTH: usize = 10;

/// Width of the launch-gate modal — wider than the palette to fit the dry-run preview's parameter
/// lines ([`centered`] clamps it to the terminal; longer lines still truncate at the border).
const LAUNCH_WIDTH: u16 = 72;

/// Height of the home masthead box: border (2) + its two lines (name+version, then the directory).
const MASTHEAD_HEIGHT: u16 = 4;

/// arclite's version, shown on the home masthead (as the agent CLIs head their opening screens).
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A typed input/event — the single funnel into [`update`]. The input + tick threads both send these.
enum Msg {
    /// A raw terminal event (key, resize, …) from the input thread.
    Input(Event),
    /// The refresh tick: re-read live state.
    Tick,
    /// A launch's dry-run finished on a worker thread: its preview text, or an error to show.
    LaunchPreview(Result<String, String>),
}

/// Which section is on screen. The cockpit opens on [`Route::Home`] (a launchpad), not a section — the
/// palette navigates between sections.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    Home,
    Status,
    Config,
}

/// A `/`-palette command: launch an AI run (a verb), open a view, or quit. Listed in *presentation*
/// order (NOT alpha-sorted — the popup preserves it, per the codex `command_popup` convention); the
/// launchable verbs lead, since firing a run is the cockpit's primary act. `ALL` is the single source.
#[derive(Clone, Copy)]
enum Command {
    Audit,
    Critique,
    Suggest,
    Summarize,
    Extract,
    Evolve,
    Status,
    Config,
    Home,
    Quit,
}

impl Command {
    const ALL: &'static [Command] = &[
        Command::Audit,
        Command::Critique,
        Command::Suggest,
        Command::Summarize,
        Command::Extract,
        Command::Evolve,
        Command::Status,
        Command::Config,
        Command::Home,
        Command::Quit,
    ];

    /// The typed name the palette prefix-matches; for a verb it's also the CLI subcommand spawned.
    fn name(self) -> &'static str {
        match self {
            // Verb names are the CLI subcommand names — single-sourced via the cli.rs NAME_* consts
            // (used by clap too), so a rename can't drift the spawn/palette from `--help`.
            Command::Audit => crate::cli::NAME_AUDIT,
            Command::Critique => crate::cli::NAME_CRITIQUE,
            Command::Suggest => crate::cli::NAME_SUGGEST,
            Command::Summarize => crate::cli::NAME_SUMMARIZE,
            Command::Extract => crate::cli::NAME_EXTRACT,
            Command::Evolve => crate::cli::NAME_EVOLVE,
            // Views/quit are TUI-only — no CLI subcommand — so their names live here.
            Command::Status => "status",
            Command::Config => "config",
            Command::Home => "home",
            Command::Quit => "quit",
        }
    }

    /// One-line help shown beside the name in the palette.
    fn description(self) -> &'static str {
        match self {
            // The launchable verbs share their text with clap `--help` via the cli.rs `VERB_*`
            // consts, so the palette and the CLI can't drift.
            Command::Audit => crate::cli::VERB_AUDIT,
            Command::Critique => crate::cli::VERB_CRITIQUE,
            Command::Suggest => crate::cli::VERB_SUGGEST,
            Command::Summarize => crate::cli::VERB_SUMMARIZE,
            Command::Extract => crate::cli::VERB_EXTRACT,
            Command::Evolve => crate::cli::VERB_EVOLVE,
            // Views and quit are TUI-only (no CLI subcommand), so their hints live here.
            Command::Status => "live view of in-flight runs",
            Command::Config => "settings and active layers",
            Command::Home => "the launchpad",
            Command::Quit => "leave the cockpit",
        }
    }

    /// Apply the chosen command: a verb starts a launch (dry-run → gate); status/config/home open a
    /// view; quit quits. The only place a palette selection acts.
    fn apply(self, app: &mut App) {
        match self {
            Command::Home => app.route = Route::Home,
            Command::Status => app.route = Route::Status,
            Command::Config => app.open_config(),
            Command::Quit => app.should_quit = true,
            verb => app.start_launch(verb),
        }
    }
}

/// The `/` command palette overlay: the query typed so far and the highlighted match. Open only when
/// `App::palette` is `Some`. Prefix-match (not fuzzy) over [`Command::ALL`], preserving its order.
struct Palette {
    query: String,
    selected: usize,
}

impl Palette {
    fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
        }
    }

    /// Commands whose name starts with the current query, in [`Command::ALL`] order. An empty query
    /// matches everything (so bare `/` lists the full set).
    fn matches(&self) -> Vec<Command> {
        Command::ALL
            .iter()
            .copied()
            .filter(|c| c.name().starts_with(self.query.as_str()))
            .collect()
    }

    /// Keep `selected` within the current match list after the query changes.
    fn reclamp(&mut self) {
        let n = self.matches().len();
        if self.selected >= n {
            self.selected = n.saturating_sub(1);
        }
    }
}

/// All TUI state. [`render`] reads it and never mutates it; [`update`] is the only mutator.
struct App {
    route: Route,
    status: Snapshot,
    palette: Option<Palette>,
    /// The in-flight launch (dry-run → gate), or `None`. When present it overlays everything.
    launch: Option<Launch>,
    should_quit: bool,
    /// A clone of the loop's `mpsc` sender, handed to launch worker threads so a dry-run reports back.
    tx: mpsc::Sender<Msg>,
    /// The directory the cockpit was launched in — the repo its runs target — shown on home.
    cwd: String,
    /// The settings shown by the config view, loaded when it's opened; `None` until then.
    config: Option<ConfigView>,
}

impl App {
    fn new(tx: mpsc::Sender<Msg>, cwd: String) -> Self {
        Self {
            route: Route::Home,
            status: Snapshot::read(),
            palette: None,
            launch: None,
            should_quit: false,
            tx,
            cwd,
            config: None,
        }
    }

    /// Begin launching a verb: show the gate as "preparing" and spawn its dry-run on a worker thread,
    /// which reports the preview back via [`Msg::LaunchPreview`]. The dry-run spends nothing.
    fn start_launch(&mut self, verb: Command) {
        self.palette = None;
        self.launch = Some(Launch {
            verb,
            stage: LaunchStage::Preparing,
        });
        let tx = self.tx.clone();
        let name = verb.name();
        thread::spawn(move || {
            let _ = tx.send(Msg::LaunchPreview(dry_run_preview(name)));
        });
    }

    /// Open the config view, loading the resolved settings + active layers for the launch directory.
    /// Re-loaded on each entry (so an external edit shows on return). Read-only for now; the same
    /// `resolved` projection backs `arc config list`, so the two can't drift.
    fn open_config(&mut self) {
        self.route = Route::Config;
        self.config = Some(
            match crate::commands::config::resolved(std::path::Path::new(&self.cwd)) {
                Ok(r) => ConfigView::Loaded {
                    values: r
                        .values
                        .into_iter()
                        .map(|(k, v)| {
                            (
                                k.to_owned(),
                                v.unwrap_or_else(|| crate::settings::UNSET.to_owned()),
                            )
                        })
                        .collect(),
                    layers: r.layers,
                },
                Err(e) => ConfigView::Error(format!("{e:#}")),
            },
        );
    }
}

/// An in-flight launch: which verb, and how far along. v1 ends at the gate; confirming to the real
/// run is the next cut.
struct Launch {
    verb: Command,
    stage: LaunchStage,
}

/// How far a [`Launch`] has progressed.
enum LaunchStage {
    /// The dry-run is running on a worker thread; awaiting its preview.
    Preparing,
    /// The preview is ready — the cost gate shows it for confirm/cancel.
    Confirming { preview: String },
    /// The dry-run failed; show why, and let the user dismiss.
    Failed { error: String },
}

/// The settings shown by the config view, loaded on entering it: the resolved defaults and active
/// layers, or the error if `.arc/settings.json` couldn't be loaded.
enum ConfigView {
    Loaded {
        /// (key, resolved value or "(unset)") for every settable default.
        values: Vec<(String, String)>,
        /// The active settings-file layers (user then project), empty if none.
        layers: Vec<String>,
    },
    Error(String),
}

/// Run a verb's dry-run as a subprocess of this same `arc` binary and return its **preview** — the
/// header (params + estimate) arc prints before the appended prompt — for the gate, or an error
/// string. Zero spend (`--dry-run`); the cockpit shows arc's own output rather than re-deriving it.
fn dry_run_preview(verb: &str) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't locate the arc binary: {e}"))?;
    let output = std::process::Command::new(exe)
        .args([verb, ".", "--dry-run"])
        .output()
        .map_err(|e| format!("couldn't start the dry-run: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("the dry-run failed: {}", stderr.trim()));
    }
    let out = String::from_utf8_lossy(&output.stdout).into_owned();
    // arc prints the preview header, a blank line, then the full prompt — take everything before that
    // first blank line (or all of it, should a run ever emit no prompt section).
    let header = out
        .split_once("\n\n")
        .map_or(out.as_str(), |(head, _)| head);
    Ok(header.trim_end().to_owned())
}

/// What the live status reflects at one instant: the in-flight runs, how many registry entries couldn't
/// be read, the reference time for ages, and any error reading the registry — surfaced, never hidden.
/// Re-read every tick so both the status view *and* the global footer's count stay current.
struct Snapshot {
    active: Vec<ActiveRun>,
    unreadable: usize,
    now: u64,
    error: Option<String>,
}

impl Snapshot {
    /// Read the run registry fresh. A read failure is captured into `error` (and shown) rather than
    /// torn up the loop — a transient registry error shouldn't collapse the live view.
    fn read() -> Self {
        let now = crate::log::now_secs();
        match crate::runs::active() {
            Ok((active, unreadable)) => Self {
                active,
                unreadable: unreadable.len(),
                now,
                error: None,
            },
            Err(e) => Self {
                active: Vec::new(),
                unreadable: 0,
                now,
                error: Some(format!("{e:#}")),
            },
        }
    }
}

/// The `tui` command. Owns the terminal (inline viewport) for its duration and restores it on exit
/// (and on panic, via the panic hook `ratatui::try_init_with_options` installs).
pub fn run(args: &TuiArgs, _global: &GlobalArgs) -> anyhow::Result<()> {
    // A TUI needs an interactive terminal — fail cleanly rather than entering raw mode against a pipe
    // (which would hang or corrupt non-interactive output). This is also why `--json` has no meaning here.
    anyhow::ensure!(
        std::io::stdout().is_terminal() && std::io::stdin().is_terminal(),
        "`arc tui` needs an interactive terminal (stdin/stdout are not a TTY)"
    );
    anyhow::ensure!(
        args.interval > 0.0,
        "--interval must be a positive number of seconds"
    );
    let interval = Duration::from_secs_f64(args.interval);

    // Inline viewport: the live region renders in the normal buffer; scrollback above is preserved.
    let mut terminal = ratatui::try_init_with_options(TerminalOptions {
        viewport: Viewport::Inline(VIEWPORT_HEIGHT),
    })
    .context("failed to initialize the terminal")?;
    let result = event_loop(&mut terminal, interval);
    // Clear the live region before restoring. Stock inline leaves the final frame in place (good for a
    // transcript — codex/Claude do that); but a frozen *live* frame just reads as stale, so wipe it and
    // let the shell prompt resume clean (codex likewise clears its live region on exit).
    let _ = terminal.clear();
    ratatui::restore(); // always restore — even if the loop errored (a panic is handled by the hook)
    result
}

/// Spawn the input + tick threads, then drive the draw/recv/update loop until quit. Both threads feed
/// one `mpsc`; the loop blocks on it, so a tick (live refresh) or a keypress each wakes exactly one
/// redraw. The threads end when the receiver drops (their `send` fails) or with the process.
fn event_loop(terminal: &mut ratatui::DefaultTerminal, interval: Duration) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<Msg>();

    // Input thread: the sole reader of stdin. `event::read` blocks; each event becomes a `Msg`.
    let input_tx = tx.clone();
    thread::spawn(move || {
        while let Ok(event) = event::read() {
            if input_tx.send(Msg::Input(event)).is_err() {
                break; // receiver gone — the loop exited
            }
        }
    });
    // Tick thread: drives the live refresh.
    let tick_tx = tx.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(interval);
            if tick_tx.send(Msg::Tick).is_err() {
                break;
            }
        }
    });

    // The cockpit's target directory (shown on home) — surface a genuine failure to read it rather
    // than masking it. The app keeps the original sender, cloned into each launch worker.
    let cwd = std::env::current_dir()
        .context("cannot determine the working directory")?
        .display()
        .to_string();
    let mut app = App::new(tx, cwd);
    while !app.should_quit {
        terminal.draw(|frame| render(frame, &app))?;
        // `recv` errors only when every sender has dropped; the tick thread keeps one alive for the
        // whole loop, so an error here means a sender thread panicked — surface that loudly (the panic
        // hook restores the terminal) rather than exiting as if the user quit, which would hide the bug.
        let msg = rx
            .recv()
            .expect("a sender thread panicked (input/tick hold a sender for the loop's lifetime)");
        update(&mut app, msg);
    }
    Ok(())
}

/// The one place `App` state changes. Ticks refresh live state; key presses route through [`handle_key`].
fn update(app: &mut App, msg: Msg) {
    match msg {
        Msg::Tick => app.status = Snapshot::read(),
        Msg::Input(Event::Key(key)) if key.kind == KeyEventKind::Press => handle_key(app, key),
        Msg::Input(_) => {} // resize/focus/release → just redraw next iteration
        Msg::LaunchPreview(result) => {
            // Fold the dry-run's outcome into the gate — unless the launch was cancelled before it
            // arrived (then `launch` is `None` and the preview is simply dropped).
            if let Some(launch) = app.launch.as_mut() {
                launch.stage = match result {
                    Ok(preview) => LaunchStage::Confirming { preview },
                    Err(error) => LaunchStage::Failed { error },
                };
            }
        }
    }
}

/// Layered key routing: the launch gate (when open) gets first claim, then the palette overlay, then
/// the section/global bindings. This is the seam where sections add their own keys as they land,
/// always below the overlays and above the global quit.
fn handle_key(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // The launch gate is the top overlay. Esc / Ctrl-C here is a HARD CANCEL of the launch (never
    // proceed, never quit the app): a spend gate's cancel must always mean "don't spend".
    if app.launch.is_some() {
        if key.code == KeyCode::Esc || (ctrl && key.code == KeyCode::Char('c')) {
            app.launch = None;
        }
        return; // every other key is inert while the gate is up (confirm→run is the next cut)
    }
    // Ctrl-C quits from anywhere else — the universal escape hatch.
    if ctrl && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }
    if app.palette.is_some() {
        handle_palette_key(app, key.code);
        return;
    }
    match key.code {
        KeyCode::Char('/') => app.palette = Some(Palette::new()),
        KeyCode::Char('q') => app.should_quit = true,
        // Esc backs out one level: a section returns to home; home leaves the cockpit.
        KeyCode::Esc => match app.route {
            Route::Home => app.should_quit = true,
            _ => app.route = Route::Home,
        },
        _ => {} // sections refresh on the tick — no manual refresh key needed
    }
}

/// Keys while the palette is open: edit the query, move the selection, run the choice, or dismiss.
fn handle_palette_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => app.palette = None,
        KeyCode::Enter => {
            // Pull the choice out first so the `app.palette` borrow ends before `apply` mutates `app`.
            let chosen = app
                .palette
                .as_ref()
                .and_then(|p| p.matches().get(p.selected).copied());
            app.palette = None;
            if let Some(cmd) = chosen {
                cmd.apply(app);
            }
        }
        KeyCode::Up => {
            if let Some(p) = app.palette.as_mut() {
                p.selected = p.selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(p) = app.palette.as_mut() {
                let n = p.matches().len();
                if n > 0 {
                    p.selected = (p.selected + 1).min(n - 1);
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(p) = app.palette.as_mut() {
                p.query.pop();
                p.reclamp();
            }
        }
        KeyCode::Char(ch) => {
            if let Some(p) = app.palette.as_mut() {
                p.query.push(ch);
                p.reclamp();
            }
        }
        _ => {}
    }
}

/// Render one frame from `app` — pure (state in, frame out). Every view is a body + the global footer;
/// the palette, when open, draws as an overlay on top.
fn render(frame: &mut Frame, app: &App) {
    let [body, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());

    match app.route {
        Route::Home => render_home(frame, body, &app.cwd),
        Route::Status => render_status(frame, &app.status, body),
        Route::Config => render_config(
            frame,
            app.config
                .as_ref()
                .expect("config is loaded when its route is active"),
            body,
        ),
    }
    render_footer(frame, footer, app);

    if let Some(palette) = &app.palette {
        render_palette(frame, palette, frame.area());
    }
    if let Some(launch) = &app.launch {
        render_launch(frame, launch, frame.area());
    }
}

/// The home view the TUI opens on — a compact masthead (name + version, then the target directory),
/// echoing how the agent CLIs head their opening screens. The footer carries the live state and key
/// hints, so home doesn't repeat them; the space below is the open launchpad.
fn render_home(frame: &mut Frame, area: Rect, cwd: &str) {
    let [masthead, _] =
        Layout::vertical([Constraint::Length(MASTHEAD_HEIGHT), Constraint::Min(0)]).areas(area);
    let lines = vec![
        Line::from(format!("arc {VERSION}")).bold(),
        Line::from(cwd.to_owned()).dim(),
    ];
    frame.render_widget(Paragraph::new(lines).block(Block::bordered()), masthead);
}

/// Column widths for the live-run table, sized to each field's content: the command verb, a flexing
/// repo cell (takes the row's slack), the model id, then the compact age/turns/tools/chars counters —
/// positionally paired with the header labels in [`render_status`].
const STATUS_COLUMN_WIDTHS: [Constraint; 7] = [
    Constraint::Length(10), // command
    Constraint::Min(12),    // repo (flexes to fill the row)
    Constraint::Length(18), // model id
    Constraint::Length(6),  // age
    Constraint::Length(6),  // turns
    Constraint::Length(6),  // tool calls
    Constraint::Length(9),  // output chars
];

/// The live run-registry view: a header and a table of in-flight runs (or a message). The footer is
/// global now, so this owns only the section body.
fn render_status(frame: &mut Frame, snap: &Snapshot, area: Rect) {
    let [header, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

    frame.render_widget(Line::from("live status").bold(), header);

    if let Some(err) = &snap.error {
        frame.render_widget(
            Paragraph::new(format!("run registry unreadable: {err}")).block(Block::bordered()),
            body,
        );
    } else if snap.active.is_empty() {
        frame.render_widget(
            Paragraph::new("no runs in flight").block(Block::bordered()),
            body,
        );
    } else {
        let rows = snap.active.iter().map(|r| {
            Row::new([
                r.command.clone(),
                crate::log::repo_basename(&r.repo).to_owned(),
                r.model.clone(),
                r.age_display(snap.now),
                r.turns.to_string(),
                r.tool_calls.to_string(),
                r.output_chars.to_string(),
            ])
        });
        let mut title = format!("{} running", snap.active.len());
        if snap.unreadable > 0 {
            title.push_str(&format!(
                " · {}",
                crate::runs::unreadable_entries(snap.unreadable)
            ));
        }
        let table = Table::new(rows, STATUS_COLUMN_WIDTHS)
            .header(
                Row::new(["command", "repo", "model", "age", "turns", "tools", "chars"])
                    .style(Style::new().bold()),
            )
            .block(Block::bordered().title(title));
        frame.render_widget(table, body);
    }
}

/// Column widths for the config table: the dotted setting key (the longest being
/// `defaults.codex_reasoning_effort`), then the resolved value taking the row's slack.
const CONFIG_COLUMN_WIDTHS: [Constraint; 2] = [Constraint::Length(32), Constraint::Min(10)];

/// The config view: every resolved default (after user-then-project layering) and the active settings
/// layers — the projection `arc config list` prints, shown here. Read-only for now; editing is the
/// follow-up (likely arrow-key pickers, shared with the launch-config cut).
fn render_config(frame: &mut Frame, config: &ConfigView, area: Rect) {
    let [header, body, layers_line] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(Line::from("config").bold(), header);

    match config {
        ConfigView::Error(e) => frame.render_widget(
            Paragraph::new(format!("settings unreadable: {e}")).block(Block::bordered()),
            body,
        ),
        ConfigView::Loaded { values, layers } => {
            let rows = values
                .iter()
                .map(|(key, value)| Row::new([key.clone(), value.clone()]));
            let table = Table::new(rows, CONFIG_COLUMN_WIDTHS)
                .header(Row::new(["setting", "value"]).style(Style::new().bold()))
                .block(Block::bordered());
            frame.render_widget(table, body);

            frame.render_widget(
                Line::from(format!(
                    "layers: {}",
                    crate::join_or(layers, crate::settings::NO_LAYERS)
                ))
                .dim(),
                layers_line,
            );
        }
    }
}

/// The persistent footer — present on every view. Carries the at-a-glance active-run count and the
/// contextual key hints.
fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let n = app.status.active.len();
    let runs = if n == 0 {
        "idle".to_owned()
    } else {
        format!("● {n} running")
    };

    let hints = if let Some(launch) = &app.launch {
        // The gate's only keys are Esc / Ctrl-C; "cancel" while a launch is pending, "dismiss" for a
        // failed dry-run (nothing in flight to cancel). The modal itself shows no hint — this is it.
        match &launch.stage {
            LaunchStage::Failed { .. } => "esc dismiss",
            _ => "esc cancel",
        }
    } else if app.palette.is_some() {
        "↑↓ select · enter run · esc close"
    } else {
        match app.route {
            Route::Home => "/ commands · q quit",
            Route::Status | Route::Config => "/ commands · esc back · q quit",
        }
    };

    frame.render_widget(Line::from(format!("{runs}  ·  {hints}")).dim(), area);
}

/// The `/` command palette: a centered popup with the typed query and the prefix-matched commands,
/// the selection highlighted. Drawn over whatever section is active.
fn render_palette(frame: &mut Frame, palette: &Palette, area: Rect) {
    let matches = palette.matches();
    // Height: border (2) + input line (1) + one row per match (at least one, for the empty message).
    let rows = matches.len().max(1) as u16;
    let rect = centered(area, PALETTE_WIDTH, rows + 3);
    frame.render_widget(Clear, rect); // punch through whatever's underneath

    let block = Block::bordered().title("commands");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let [input_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    frame.render_widget(Line::from(format!("/{}", palette.query)), input_area);

    if matches.is_empty() {
        frame.render_widget(Line::from("no matching command").dim(), list_area);
        return;
    }
    let lines: Vec<Line> = matches
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let text = format!(
                "{:<width$} {}",
                c.name(),
                c.description(),
                width = PALETTE_NAME_WIDTH
            );
            if i == palette.selected {
                Line::from(format!("› {text}")).bold()
            } else {
                Line::from(format!("  {text}")).dim()
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list_area);
}

/// The launch gate: a centered modal showing the chosen verb's dry-run preview (its parameters + the
/// token/cost estimate) to confirm or cancel before any spend. v1 ends here — cancel only; wiring
/// confirm to the real run is the next cut.
fn render_launch(frame: &mut Frame, launch: &Launch, area: Rect) {
    let verb = launch.verb.name();
    let (title, body) = match &launch.stage {
        LaunchStage::Preparing => (
            format!("launch {verb} · preparing"),
            "estimating (dry run)…".to_owned(),
        ),
        LaunchStage::Confirming { preview } => {
            (format!("launch {verb} · confirm"), preview.clone())
        }
        LaunchStage::Failed { error } => (format!("launch {verb} · failed"), error.clone()),
    };

    let lines: Vec<Line> = body.lines().map(Line::from).collect();
    let height = (lines.len() as u16 + 2).min(area.height); // border (2) + body; the footer holds the hint
    let rect = centered(area, LAUNCH_WIDTH, height);
    frame.render_widget(Clear, rect); // punch through whatever's underneath

    let block = Block::bordered().title(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// A `width`×`height` rect centered within `area` (clamped to fit) — for the palette popup and the
/// launch-gate modal.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// An empty live snapshot at a fixed reference time.
    fn empty_snapshot() -> Snapshot {
        Snapshot {
            active: Vec::new(),
            unreadable: 0,
            now: 100,
            error: None,
        }
    }

    /// One in-flight run, for the status-view and footer-count tests.
    fn one_run_snapshot() -> Snapshot {
        Snapshot {
            active: vec![ActiveRun {
                pid: 42,
                index: 0,
                command: "audit".to_owned(),
                repo: "/home/x/ida".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                started_at: 90,
                turns: 3,
                tool_calls: 1,
                output_chars: 1200,
            }],
            unreadable: 0,
            now: 100,
            error: None,
        }
    }

    fn app_with(route: Route, status: Snapshot) -> App {
        // A launch worker would send here; the tests never start one, so the receiver is dropped.
        let (tx, _rx) = mpsc::channel();
        App {
            route,
            status,
            palette: None,
            launch: None,
            should_quit: false,
            tx,
            cwd: ".".to_owned(),
            config: None,
        }
    }

    /// Render the whole frame (section + footer + any overlay) to an in-memory backend, as one string.
    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    fn press(code: KeyCode) -> Msg {
        Msg::Input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    #[test]
    fn footer_shows_active_count_on_every_view() {
        // The count is in the footer regardless of which section is on screen.
        for route in [Route::Home, Route::Status] {
            let text = screen(&app_with(route, one_run_snapshot()), 80, 12);
            assert!(
                text.contains("1 running"),
                "route should show the run count in the footer"
            );
        }
        // …and reads "idle" when nothing is in flight.
        let idle = screen(&app_with(Route::Home, empty_snapshot()), 80, 12);
        assert!(idle.contains("idle"));
    }

    #[test]
    fn status_view_renders_the_run_table() {
        let text = screen(&app_with(Route::Status, one_run_snapshot()), 100, 12);
        assert!(text.contains("live status"));
        assert!(text.contains("audit"));
        assert!(text.contains("ida")); // basename of the repo path
        assert!(text.contains("10s")); // now - started_at
        assert!(text.contains("1 running"));
    }

    #[test]
    fn status_registry_error_is_surfaced() {
        let mut snap = empty_snapshot();
        snap.error = Some("permission denied".to_owned());
        let text = screen(&app_with(Route::Status, snap), 80, 12);
        assert!(text.contains("unreadable"));
        assert!(text.contains("permission denied"));
    }

    #[test]
    fn slash_opens_palette_and_typing_filters() {
        let mut app = app_with(Route::Home, empty_snapshot());
        update(&mut app, press(KeyCode::Char('/')));
        assert!(app.palette.is_some(), "/ should open the palette");
        // Bare palette lists every command.
        let all = screen(&app, 80, 14);
        assert!(all.contains("status") && all.contains("quit") && all.contains("home"));
        // Typing prefix-filters: "st" → status only.
        update(&mut app, press(KeyCode::Char('s')));
        update(&mut app, press(KeyCode::Char('t')));
        let filtered = screen(&app, 80, 14);
        assert!(filtered.contains("status"));
        assert!(!filtered.contains("the launchpad")); // "home"'s description gone
        assert!(!filtered.contains("leave the cockpit")); // "quit"'s description gone
    }

    #[test]
    fn palette_enter_navigates_and_closes() {
        let mut app = app_with(Route::Home, empty_snapshot());
        for code in [
            KeyCode::Char('/'),
            KeyCode::Char('s'),
            KeyCode::Char('t'),
            KeyCode::Char('a'),
            KeyCode::Enter,
        ] {
            update(&mut app, press(code));
        }
        assert!(app.palette.is_none(), "enter should close the palette");
        assert_eq!(
            app.route,
            Route::Status,
            "/status should navigate to the status view"
        );
    }

    #[test]
    fn palette_esc_closes_without_navigating() {
        let mut app = app_with(Route::Status, empty_snapshot());
        update(&mut app, press(KeyCode::Char('/')));
        update(&mut app, press(KeyCode::Esc));
        assert!(app.palette.is_none());
        assert_eq!(
            app.route,
            Route::Status,
            "esc should dismiss the palette, not change the view"
        );
    }

    #[test]
    fn esc_backs_out_section_to_home_then_quits() {
        let mut app = app_with(Route::Status, empty_snapshot());
        update(&mut app, press(KeyCode::Esc));
        assert_eq!(app.route, Route::Home, "esc in a section returns home");
        assert!(!app.should_quit);
        update(&mut app, press(KeyCode::Esc));
        assert!(app.should_quit, "esc at home leaves the cockpit");
    }

    #[test]
    fn quit_keys_set_should_quit() {
        for msg in [
            press(KeyCode::Char('q')),
            Msg::Input(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            ))),
        ] {
            let mut app = app_with(Route::Home, empty_snapshot());
            update(&mut app, msg);
            assert!(app.should_quit);
        }
    }

    #[test]
    fn ctrl_c_quits_even_with_palette_open() {
        let mut app = app_with(Route::Home, empty_snapshot());
        update(&mut app, press(KeyCode::Char('/')));
        update(
            &mut app,
            Msg::Input(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            ))),
        );
        assert!(app.should_quit, "ctrl-c is the universal escape hatch");
    }
}
