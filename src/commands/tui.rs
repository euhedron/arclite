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
//! State is a small route + an optional palette overlay: [`render`] is a pure function of [`App`],
//! tested headlessly with `TestBackend`; the interactive loop itself needs a real terminal. This cut
//! lands the cockpit spine — home, the footer, the `/` palette, and status as one navigable view.
//! Launching real commands with live streaming (and the cost-transparency guardrails that demands) is
//! the next cut on this same runtime — see the `arc-tui-architecture` blueprint.
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
/// refinement — see the blueprint's "inline-height management" note.)
const VIEWPORT_HEIGHT: u16 = 16;

/// Width of the command-palette popup — wide enough for the longest command name plus its one-line
/// description without dominating a narrow terminal ([`centered`] clamps it to the available width).
const PALETTE_WIDTH: u16 = 56;

/// A typed input/event — the single funnel into [`update`]. The input + tick threads both send these.
enum Msg {
    /// A raw terminal event (key, resize, …) from the input thread.
    Input(Event),
    /// The refresh tick: re-read live state.
    Tick,
}

/// Which section is on screen. The cockpit opens on [`Route::Home`] (a launchpad), not a section — the
/// palette navigates between sections. (Runs/usage/rules/config join this as later cuts land them.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    Home,
    Status,
}

/// A slash-palette command: navigate to a section, or quit. Listed in *presentation* order (NOT
/// alpha-sorted — the popup preserves this order, per the codex `command_popup` convention). As real
/// run-launching verbs land they join here, gated by what's valid in the moment; for now it's
/// navigation. `ALL` is the single source of the command set.
#[derive(Clone, Copy)]
enum Command {
    Home,
    Status,
    Quit,
}

impl Command {
    const ALL: &'static [Command] = &[Command::Home, Command::Status, Command::Quit];

    /// The typed name (what the palette prefix-matches, shown after the `/`).
    fn name(self) -> &'static str {
        match self {
            Command::Home => "home",
            Command::Status => "status",
            Command::Quit => "quit",
        }
    }

    /// One-line help shown beside the name in the palette.
    fn description(self) -> &'static str {
        match self {
            Command::Home => "the launchpad",
            Command::Status => "live view of in-flight runs",
            Command::Quit => "leave the cockpit",
        }
    }

    /// Apply the chosen command to the app (navigate / quit). The only place a palette selection acts.
    fn apply(self, app: &mut App) {
        match self {
            Command::Home => app.route = Route::Home,
            Command::Status => app.route = Route::Status,
            Command::Quit => app.should_quit = true,
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
    should_quit: bool,
}

impl App {
    fn new() -> Self {
        Self {
            route: Route::Home,
            status: Snapshot::read(),
            palette: None,
            should_quit: false,
        }
    }
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
    thread::spawn(move || {
        loop {
            thread::sleep(interval);
            if tx.send(Msg::Tick).is_err() {
                break;
            }
        }
    });

    let mut app = App::new();
    while !app.should_quit {
        terminal.draw(|frame| render(frame, &app))?;
        match rx.recv() {
            Ok(msg) => update(&mut app, msg),
            Err(_) => break, // both senders dropped (shouldn't happen) — exit cleanly
        }
    }
    Ok(())
}

/// The one place `App` state changes. Ticks refresh live state; key presses route through [`handle_key`].
fn update(app: &mut App, msg: Msg) {
    match msg {
        Msg::Tick => app.status = Snapshot::read(),
        Msg::Input(Event::Key(key)) if key.kind == KeyEventKind::Press => handle_key(app, key),
        Msg::Input(_) => {} // resize/focus/release → just redraw next iteration
    }
}

/// Layered key routing: the palette overlay (when open) gets first claim on keys; otherwise the
/// section/global bindings apply. This is the seam the blueprint calls out — sections add their own
/// keys here as they land, always below the overlay and above the global quit.
fn handle_key(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Ctrl-C quits from anywhere, even inside the palette — the universal escape hatch.
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
    let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());

    match app.route {
        Route::Home => render_home(frame, body),
        Route::Status => render_status(frame, &app.status, body),
    }
    render_footer(frame, footer, app);

    if let Some(palette) = &app.palette {
        render_palette(frame, palette, frame.area());
    }
}

/// The launchpad: what the cockpit opens on. Names what arc is and points at the palette. (As verbs
/// land this grows into quick-launch + recent runs; today it's the entry signpost.)
fn render_home(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from("arc — cockpit").bold(),
        Line::from(""),
        Line::from("A launchpad for targeted audit & analysis — running alongside your agents."),
        Line::from(""),
        Line::from(vec!["Press  ".into(), "/".bold(), "  for commands.".into()]),
    ];
    frame.render_widget(Paragraph::new(lines).block(Block::bordered()), area);
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
    let [header, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

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
                format!("{}s", snap.now.saturating_sub(r.started_at)),
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

/// The persistent footer — present on every view. Carries the at-a-glance active-run count (the
/// always-visible signal Nik asked for) and the contextual key hints.
fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let n = app.status.active.len();
    let runs = if n == 0 {
        "idle".to_owned()
    } else {
        format!("● {n} running")
    };

    let hints = if app.palette.is_some() {
        "↑↓ select · enter run · esc close"
    } else {
        match app.route {
            Route::Home => "/ commands · q quit",
            Route::Status => "/ commands · esc back · q quit",
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
            let text = format!("{:<10} {}", c.name(), c.description());
            if i == palette.selected {
                Line::from(format!("› {text}")).bold()
            } else {
                Line::from(format!("  {text}")).dim()
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list_area);
}

/// A `width`×`height` rect centered within `area` (clamped to fit). For the palette popup.
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
        App {
            route,
            status,
            palette: None,
            should_quit: false,
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
    fn opens_on_home_launchpad() {
        let text = screen(&app_with(Route::Home, empty_snapshot()), 80, 12);
        assert!(text.contains("arc — cockpit"));
        assert!(text.contains("for commands"));
    }

    #[test]
    fn footer_shows_active_count_on_every_view() {
        // The count is in the footer regardless of which section is on screen.
        for route in [Route::Home, Route::Status] {
            let text = screen(&app_with(route, one_run_snapshot()), 80, 12);
            assert!(text.contains("1 running"), "route should show the run count in the footer");
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
        assert_eq!(app.route, Route::Status, "/status should navigate to the status view");
    }

    #[test]
    fn palette_esc_closes_without_navigating() {
        let mut app = app_with(Route::Status, empty_snapshot());
        update(&mut app, press(KeyCode::Char('/')));
        update(&mut app, press(KeyCode::Esc));
        assert!(app.palette.is_none());
        assert_eq!(app.route, Route::Status, "esc should dismiss the palette, not change the view");
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
