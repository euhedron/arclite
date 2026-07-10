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
//! terminal. Launching spawns the `arc` binary as a subprocess: the dry-run preview captures and
//! renders arc's own output, and a confirmed run fires in the background (observed via `status` and
//! the `log` view). The cockpit is a second front-end over the CLI, not a reimplementation of it.
#![deny(clippy::print_stdout, clippy::print_stderr)] // never print while the TUI owns the terminal

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Row, Table, Wrap};
use ratatui::{Frame, TerminalOptions, Viewport};
use serde_json::Value;

use crate::cli::{GlobalArgs, TuiArgs};
use crate::commands::usage::Rollup;
use crate::runs::ActiveRun;

/// Default seconds between live refreshes when `--interval` is omitted. One second keeps the registry
/// view current without busy-spinning; `--interval` overrides it (and is echoed in `--help`).
pub const DEFAULT_INTERVAL_SECS: f64 = 1.0;

/// Lines reserved for the inline live region (clamped to the terminal's height). Larger than a compact
/// home needs, so the browse views (log, status) aren't squished; scrollback stays visible above. A
/// viewport that grows with content isn't first-class in ratatui (it's fixed at creation), so a bigger
/// fixed budget is the lever.
const VIEWPORT_HEIGHT: u16 = 24;

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

/// Height of a single-row text line — a header, hint, footer, or input. Named so the rows a view
/// reserves for these (and [`LIST_ROWS`]) stay single-sourced with the `Constraint::Length` splits
/// that lay them out, rather than recurring as bare `1`s.
const LINE: u16 = 1;

/// Rows a bordered popup box spends on its top + bottom border — named (like [`LINE`]) so a popup's
/// height math (border + content) doesn't recur as a bare `2`.
const BORDER: u16 = 2;

/// arclite's version, shown on the home masthead (as the agent CLIs head their opening screens).
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A typed input/event — the single funnel into [`update`]. The input + tick threads both send these.
enum Msg {
    /// A raw terminal event (key, resize, …) from the input thread.
    Input(Event),
    /// The refresh tick: re-read live state.
    Tick,
    /// A launch's dry-run finished on a worker thread: which verb it previewed (the launch it may fold
    /// into must still be for that verb — a late preview must not dress a newer launch), and its
    /// preview text or an error to show.
    LaunchPreview {
        verb: &'static str,
        result: Result<String, String>,
    },
    /// The startup update check finished: `Some(version)` if a newer release is published, else `None`.
    UpdateChecked(Option<String>),
}

/// Which section is on screen. The cockpit opens on [`Route::Home`] (a launchpad), not a section — the
/// palette navigates between sections.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    Home,
    Status,
    Config,
    Rules,
    Log,
    Usage,
    Doctor,
}

/// A `/`-palette command: launch an AI run (a verb, carrying its [`crate::commands::verbs`] registry
/// entry), open a view, or quit. Listed in *presentation* order (NOT alpha-sorted — the popup preserves
/// it, per the codex `command_popup` convention); the `run` sub-menu's verbs come from `verbs::ALL`, so
/// the palette can't drift from the CLI's verb set.
#[derive(Clone, Copy)]
enum Command {
    Run,
    /// A synthesis verb, carrying its entry from the [`crate::commands::verbs::ALL`] registry — the run
    /// sub-menu is built from that registry, so the palette can't drift from the CLI's verb set.
    Verb(&'static crate::commands::verbs::Verb),
    Status,
    Config,
    Rules,
    Log,
    Usage,
    Doctor,
    Home,
    Quit,
}

impl Command {
    /// The top-level palette entries: the `run` group (the synthesis verbs live under it, mirroring
    /// the CLI's `arc run <verb>`) plus the deterministic views and quit. Presentation order.
    const TOP: &'static [Command] = &[
        Command::Run,
        Command::Status,
        Command::Config,
        Command::Rules,
        Command::Log,
        Command::Usage,
        Command::Doctor,
        Command::Home,
        Command::Quit,
    ];

    /// The typed name the palette prefix-matches; for a verb it's also the CLI subcommand spawned.
    fn name(self) -> &'static str {
        match self {
            // The run group's name is single-sourced with clap + the launcher via cli::NAME_RUN.
            Command::Run => crate::cli::NAME_RUN,
            // A verb's name comes from its registry entry (single-sourced via verbs/cli's NAME_*, used
            // by clap too), so a rename can't drift the spawn/palette from `--help`.
            Command::Verb(v) => v.name(),
            // Views and quit open an in-process view (not an `arc run` subprocess like the verbs),
            // and their names are defined here rather than reused from a cli NAME_* constant.
            Command::Status => "status",
            Command::Config => "config",
            Command::Rules => "rules",
            Command::Log => "log",
            Command::Usage => "usage",
            Command::Doctor => "doctor",
            Command::Home => "home",
            Command::Quit => "quit",
        }
    }

    /// One-line help shown beside the name in the palette.
    fn description(self) -> &'static str {
        match self {
            Command::Run => "choose a synthesis verb to run",
            // A verb's hint is its registry entry's `about` (the clap `VERB_*`), so the palette and
            // CLI can't drift.
            Command::Verb(v) => v.about(),
            // Views and quit open an in-process view (not a subprocess launch), so their hints live here.
            Command::Status => "live view of in-flight runs",
            Command::Config => "settings and active layers",
            Command::Rules => "the ruleset in play: sources and rule provenance",
            Command::Log => "browse completed runs and their results",
            Command::Usage => "spend + token rollup from the run log",
            Command::Doctor => "runtime, environment & tooling check",
            Command::Home => "the launchpad",
            Command::Quit => "leave the cockpit",
        }
    }

    /// Apply the chosen command: a verb starts a launch (dry-run → gate); status/config/home open a
    /// view; quit quits. The only place a palette selection acts.
    fn apply(self, app: &mut App) {
        match self {
            // `run` drills into the verb sub-menu in the palette handler; it never reaches apply,
            // which acts on a leaf selection.
            Command::Run => {
                unreachable!("Command::Run is a palette drill-in, handled before apply")
            }
            Command::Home => app.route = Route::Home,
            Command::Status => app.route = Route::Status,
            Command::Config => app.open_config(),
            Command::Rules => app.open_rules(),
            Command::Log => app.open_log(),
            Command::Usage => app.open_usage(),
            Command::Doctor => app.open_doctor(),
            Command::Quit => app.should_quit = true,
            // The only remaining variant is a launchable verb. Naming it explicitly (rather than a
            // catch-all) means a Command variant added later can't silently fall through to a launch —
            // it fails to compile here until handled.
            Command::Verb(_) => app.start_launch(self),
        }
    }
}

/// Which level of the `/` palette is showing: the top-level commands, or the `run` sub-menu of
/// synthesis verbs (mirroring the CLI's `arc run <verb>` grouping).
#[derive(Clone, Copy, PartialEq, Debug)]
enum PaletteLevel {
    Top,
    Run,
}

impl PaletteLevel {
    /// The commands listed at this level, in presentation order. The `run` sub-menu is built from the
    /// [`crate::commands::verbs::ALL`] registry, so it can't drift from the CLI's verb set; the top
    /// level is the fixed set of views plus the `run` group.
    fn commands(self) -> Vec<Command> {
        match self {
            PaletteLevel::Top => Command::TOP.to_vec(),
            PaletteLevel::Run => crate::commands::verbs::ALL
                .iter()
                .map(|&v| Command::Verb(v))
                .collect(),
        }
    }
}

/// The `/` command palette overlay: which level it's on, the query typed so far, and the highlighted
/// match. Open only when `App::palette` is `Some`. Prefix-match (not fuzzy) over the current level's
/// commands, preserving their order.
struct Palette {
    level: PaletteLevel,
    query: String,
    selected: usize,
}

impl Palette {
    fn new() -> Self {
        Self {
            level: PaletteLevel::Top,
            query: String::new(),
            selected: 0,
        }
    }

    /// Commands at the current level whose name starts with the query, in presentation order. An empty
    /// query matches everything (so a bare level lists its full set).
    fn matches(&self) -> Vec<Command> {
        self.level
            .commands()
            .iter()
            .copied()
            .filter(|c| c.name().starts_with(self.query.as_str()))
            .collect()
    }

    /// Switch to another level — drill into the `run` sub-menu, or back out to the top — resetting the
    /// query and selection for the new list.
    fn set_level(&mut self, level: PaletteLevel) {
        self.level = level;
        self.query.clear();
        self.selected = 0;
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
    /// The `log` view's state (records + cursor + optional drilled-in detail), loaded when it's opened.
    log: Option<LogView>,
    /// The usage view's rollup (or an error message), loaded when it's opened; rendered as tables.
    usage: Option<Result<Rollup, String>>,
    /// A home-masthead warning when the launch dir is a poor place to run arc (home folder / not a git
    /// repo), else None. Computed once at startup (filesystem probes) so render stays pure.
    cwd_note: Option<String>,
    /// The launch dir, home-abbreviated for the masthead — precomputed in `new` (display_path probes
    /// the home dir) so render stays a pure function of state.
    cwd_display: String,
    /// A newer published release the startup check found (`Some(version)`), surfaced in the footer;
    /// `None` until the check reports, and when up to date or the check failed.
    update: Option<String>,
    /// The doctor view's report as rendered text, loaded when the view is opened; `None` until then.
    /// `Err` if the environment probe failed (e.g. an unreadable cwd), shown in the view.
    doctor: Option<Result<String, String>>,
    /// The rules view's state (the resolved ruleset + cursor + an optional open rule), loaded when
    /// the view is opened; `None` until then.
    rules: Option<RulesView>,
    /// Scroll offset over the doctor report, reset on entry — a tool-rich machine's report can outrun
    /// the inline viewport.
    report_scroll: u16,
}

impl App {
    fn new(tx: mpsc::Sender<Msg>, cwd: String) -> Self {
        let cwd_note = cwd_warning(Path::new(&cwd));
        let cwd_display = crate::display_path(&cwd);
        // Check for a newer release off the main thread (it's a network call); the footer flags it when
        // the result arrives. Best-effort — a failed check (offline, etc.) simply never notifies.
        let update_tx = tx.clone();
        thread::spawn(move || {
            let _ = update_tx.send(Msg::UpdateChecked(crate::commands::update::newer_release()));
        });
        Self {
            route: Route::Home,
            status: Snapshot::read(),
            palette: None,
            launch: None,
            should_quit: false,
            tx,
            cwd,
            config: None,
            log: None,
            usage: None,
            cwd_note,
            cwd_display,
            update: None,
            doctor: None,
            rules: None,
            report_scroll: 0,
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
            let _ = tx.send(Msg::LaunchPreview {
                verb: name,
                result: dry_run_preview(name),
            });
        });
    }

    /// Confirm the previewed launch: spawn the real run (no `--dry-run`) as a background process and
    /// close the gate. The run writes its own marker — so `status` and the footer track it live — and
    /// logs its result for the `log` view: the cockpit's fire-and-observe model. Its output goes to
    /// null (the durable record is the log, not the cockpit's terminal, which the TUI owns); a detached
    /// thread reaps it, and if the cockpit exits first the run is orphaned and finishes on its own.
    /// A no-op unless the gate is at the confirm stage (Enter while still preparing waits).
    fn confirm_launch(&mut self) {
        let Some(verb) = self.launch.as_ref().and_then(|l| {
            matches!(l.stage, LaunchStage::Confirming { .. }).then_some(l.verb.name())
        }) else {
            return;
        };
        let spawned = launch_command(verb).and_then(|mut cmd| {
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        });
        match spawned {
            Ok(child) => {
                thread::spawn(move || {
                    let mut child = child;
                    let _ = child.wait();
                });
                self.launch = None;
            }
            Err(e) => {
                if let Some(launch) = self.launch.as_mut() {
                    launch.stage = LaunchStage::Failed {
                        error: format!("couldn't launch `{verb}`: {e}"),
                    };
                }
            }
        }
    }

    /// Open the `log` view, loading the completed-run records (newest first) for browsing. Re-loaded
    /// on each entry (so a run that finished since shows on return), mirroring `open_config`. Read-only;
    /// the list rows and the drill-in detail reuse `arc log`'s own projections, so they can't drift.
    fn open_log(&mut self) {
        self.route = Route::Log;
        let (runs, unparsed) = match crate::log::records_newest_first() {
            Ok((records, unparsed)) => (Ok(records), unparsed),
            Err(e) => (Err(format!("{e:#}")), 0),
        };
        self.log = Some(LogView {
            runs,
            unparsed,
            now: crate::log::now_secs(),
            selected: 0,
            offset: 0,
            detail: None,
        });
    }

    /// Open the config view, loading the resolved settings + active layers for the launch directory.
    /// Re-loaded on each entry (so an external edit shows on return); the same `resolved` projection
    /// backs `arc config list`, so the two can't drift. Rows edit in place — see [`App::save_config`].
    fn open_config(&mut self) {
        self.route = Route::Config;
        self.config = Some(load_config_view(&self.cwd, 0));
    }

    /// Persist an edited setting through the same validated write path as `arc config set` (the
    /// project layer), then re-resolve the view with the cursor kept — the shown values are re-read
    /// from disk, never assumed from the write. A rejected value keeps the edit open with the error
    /// on the info line.
    fn save_config(&mut self, key: &str, value: &str, keep: usize) {
        match crate::commands::config::set_value(Path::new(&self.cwd), key, value, false) {
            Ok(_) => self.config = Some(load_config_view(&self.cwd, keep)),
            Err(e) => {
                if let Some(ConfigView::Loaded { error, .. }) = self.config.as_mut() {
                    *error = Some(format!("{e:#}"));
                }
            }
        }
    }

    /// Open the usage view, loading the run-log rollup. Re-loaded on each entry (so a run since shows),
    /// mirroring `open_config`; the same `usage::rollup` backs `arc usage`, so the two can't drift.
    fn open_usage(&mut self) {
        self.route = Route::Usage;
        self.usage = Some(
            crate::commands::usage::rollup()
                .map(|(rollup, _)| rollup)
                .map_err(|e| format!("{e:#}")),
        );
    }

    /// Open the doctor view, probing the environment fresh (re-run on each entry, like the other
    /// views). The same `doctor::gather`/`human` back `arc doctor`, so the two can't drift.
    fn open_doctor(&mut self) {
        self.route = Route::Doctor;
        self.report_scroll = 0;
        self.doctor = Some(
            crate::commands::doctor::gather()
                .map(|r| crate::commands::doctor::human(&r))
                .map_err(|e| format!("{e:#}")),
        );
    }

    /// Open the rules view, resolving the active ruleset fresh (re-run on each entry, like the other
    /// views) into a browsable list — cursor over the rules, Enter opens one to read, space toggles
    /// one on/off. The same `rules::resolved` backs `arc rules`, so the two can't drift.
    fn open_rules(&mut self) {
        self.route = Route::Rules;
        self.rules = Some(RulesView::load(&self.cwd, 0, 0));
    }

    /// Toggle the selected rule between enabled and disabled: rewrite the settings' disabled list
    /// through the same validated write path as `arc config set disabled_rules` (the project layer),
    /// then re-resolve the view with the cursor kept — the shown state is re-read from disk, never
    /// assumed from the write. A failed load or write is surfaced on the info line.
    fn toggle_selected_rule(&mut self) {
        let Some(view) = self.rules.as_ref() else {
            return;
        };
        let Ok(report) = view.report.as_ref() else {
            return;
        };
        let Some(entry) = report.rules.get(view.selected) else {
            return;
        };
        let id = entry.id.clone();
        let (keep_selected, keep_offset) = (view.selected, view.offset);
        // The merged (user + project) disabled list ± the toggled id, written whole to the project
        // layer — whole-list overlay is the layering rule this list already follows.
        let toggled = match crate::settings::Settings::load(Path::new(&self.cwd)) {
            Ok(settings) => {
                let mut ids = settings.disabled_rules;
                match ids.iter().position(|d| d == &id) {
                    Some(i) => {
                        ids.remove(i);
                    }
                    None => ids.push(id),
                }
                ids.join(",")
            }
            Err(e) => {
                if let Some(v) = self.rules.as_mut() {
                    v.notice = Some(format!("{e:#}"));
                }
                return;
            }
        };
        if let Err(e) = crate::commands::config::set_value(
            Path::new(&self.cwd),
            "disabled_rules",
            &toggled,
            false,
        ) {
            if let Some(v) = self.rules.as_mut() {
                v.notice = Some(format!("{e:#}"));
            }
            return;
        }
        self.rules = Some(RulesView::load(&self.cwd, keep_selected, keep_offset));
    }
}

/// The rules view's state: the resolved ruleset (or the resolution error), the cursor + scroll offset
/// over the rule list, when a rule is opened — which rule and how far it's scrolled — and a failed
/// toggle-write's error for the info line.
struct RulesView {
    report: Result<crate::commands::rules::Report, String>,
    selected: usize,
    offset: usize,
    detail: Option<RuleDetail>,
    /// A failed toggle's load/write error, shown on the info line until the next action.
    notice: Option<String>,
}

/// One opened rule: its index into the report's rules (valid by construction — set from a cursor that
/// only ever points at a real row) and a scroll offset, since a rule body can outrun the inline body.
struct RuleDetail {
    index: usize,
    scroll: u16,
}

impl RulesView {
    /// Resolve the ruleset into a fresh view with the cursor at `selected`/`offset` (clamped) — one
    /// loader for opening the view and re-resolving after a toggle, so shown state is always re-read.
    fn load(cwd: &str, selected: usize, offset: usize) -> Self {
        let report = crate::commands::rules::resolved(Path::new(cwd), None, None)
            .map_err(|e| format!("{e:#}"));
        let last = match &report {
            Ok(r) => r.rules.len().saturating_sub(1),
            Err(_) => 0,
        };
        Self {
            report,
            selected: selected.min(last),
            offset,
            detail: None,
            notice: None,
        }
    }

    /// Move the cursor to `i`, keeping it inside the visible window.
    fn select(&mut self, i: usize) {
        select_visible(&mut self.selected, &mut self.offset, i);
    }

    /// Open the rule under the cursor for reading. A no-op when no rules resolved.
    fn open_detail(&mut self) {
        let Ok(report) = self.report.as_ref() else {
            return;
        };
        if self.selected < report.rules.len() {
            self.detail = Some(RuleDetail {
                index: self.selected,
                scroll: 0,
            });
        }
    }
}

/// An in-flight launch: which verb, and how far along (preview → confirm → fired in the background,
/// or failed).
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

/// The settings shown by the config view, loaded on entering it: the resolved settings and active
/// layers plus the cursor and any in-progress edit, or the error if `.arc/settings.json` couldn't be
/// loaded.
enum ConfigView {
    Loaded {
        /// (key, resolved value or "(unset)") for every settable key.
        values: Vec<(String, String)>,
        /// The active settings-file layers (user then project), empty if none.
        layers: Vec<String>,
        /// The cursor over the settings rows.
        selected: usize,
        /// An in-progress edit of the selected setting — the input buffer; `None` while browsing.
        /// Saving validates and writes through the same path as `arc config set` (the project layer).
        editing: Option<String>,
        /// The last save attempt's validation error, shown on the info line until the next action.
        error: Option<String>,
    },
    Error(String),
}

/// Load the config view's state: the `arc config list` projection with the cursor at `selected`
/// (clamped), not editing. One loader for opening the view and re-resolving it after a save, so the
/// shown values are always re-read from disk, never assumed from the write.
fn load_config_view(cwd: &str, selected: usize) -> ConfigView {
    match crate::commands::config::resolved(Path::new(cwd)) {
        Ok(r) => ConfigView::Loaded {
            selected: selected.min(r.values.len().saturating_sub(1)),
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
            editing: None,
            error: None,
        },
        Err(e) => ConfigView::Error(format!("{e:#}")),
    }
}

/// Chrome lines a list/detail view (log, rules) spends around its boxed body: a title line above and
/// an information line below — facts only (a count, a path, an id); the key hints live in the global
/// footer, keeping information and controls apart. Named so the views' `Layout`s and [`LIST_ROWS`]
/// derive the body height from the *same* constants and can't silently disagree if the chrome changes.
const LIST_HEADER_LINES: u16 = LINE;
const LIST_INFO_LINES: u16 = LINE;

/// Visible rows inside a list/detail view's bordered box — the inline viewport less the global footer,
/// the header, the info line, and the border. One source for both the cursor math ([`select_visible`],
/// PageUp/Down) and the render window, so they can't disagree on how many rows are on screen.
const LIST_ROWS: usize =
    (VIEWPORT_HEIGHT - LINE - LIST_HEADER_LINES - LIST_INFO_LINES - BORDER) as usize;

/// Move a list cursor to `i`, scrolling `offset` the minimum needed to keep the cursor inside the
/// [`LIST_ROWS`] visible window — the one statement of the keep-visible list math (log + rules lists).
fn select_visible(selected: &mut usize, offset: &mut usize, i: usize) {
    *selected = i;
    if i < *offset {
        *offset = i;
    } else if i >= *offset + LIST_ROWS {
        *offset = i + 1 - LIST_ROWS;
    }
}

/// The `log` view's state: the completed-run records (newest first) or the error reading them, the
/// cursor + scroll offset over the list, and — when drilled in — the selected run's rendered detail.
struct LogView {
    runs: Result<Vec<Value>, String>,
    /// Run-log lines that couldn't be parsed — surfaced in the hint, never silently dropped.
    unparsed: usize,
    /// Reference time for the rows' relative ages, captured at load.
    now: u64,
    selected: usize,
    offset: usize,
    detail: Option<LogDetail>,
}

/// One run's detail screen: its rendered body (the stored result, or a note when none is kept), a
/// scroll offset over it (a result can outrun the inline body), and the run id for the view's info
/// line — `None` when the record carries no usable id (the body says why).
struct LogDetail {
    body: String,
    scroll: u16,
    id: Option<String>,
}

impl LogView {
    /// Move the cursor to `i`, scrolling the window the minimum needed to keep it visible.
    fn select(&mut self, i: usize) {
        select_visible(&mut self.selected, &mut self.offset, i);
    }

    /// Open the selected run's detail — its stored result rendered like `arc log <id>`, or a note when
    /// the result isn't kept (predates the store / logging off) or the run carries no id.
    fn open_detail(&mut self) {
        let Ok(runs) = self.runs.as_ref() else { return };
        let Some(record) = runs.get(self.selected) else {
            return;
        };
        let (body, id) = match record.get("id").and_then(Value::as_str) {
            None => (
                "this run predates the result store (no id), so its result wasn't kept".to_owned(),
                None,
            ),
            // The id comes from a log record — a file editable outside the program — so validate it to
            // a safe path segment before it reaches load_stored's path join (as `arc log <id>` does).
            Some(id) if crate::commands::log::ensure_safe_run_id(id).is_err() => (
                format!(
                    "the log record's id `{id}` isn't a usable run id (expected a single path segment)"
                ),
                None,
            ),
            Some(id) => (
                match crate::commands::log::load_stored(id) {
                    Ok(Some(stored)) => crate::commands::log::stored_human(&stored),
                    Ok(None) => format!(
                        "no stored result for `{id}` — it predates the result store, or was run with logging off"
                    ),
                    Err(e) => format!("couldn't load the stored result: {e:#}"),
                },
                Some(id.to_owned()),
            ),
        };
        self.detail = Some(LogDetail {
            body,
            scroll: 0,
            id,
        });
    }

    /// Scroll the open detail by `delta` rows. A no-op while the list (not a detail) is showing.
    fn scroll_detail(&mut self, delta: i32) {
        if let Some(detail) = self.detail.as_mut() {
            detail.scroll = scrolled(detail.scroll, delta);
        }
    }
}

/// A scroll offset moved by `delta` rows — the one clamp for every scrolled text body (the log detail,
/// the doctor/rules reports). Clamped at the top only: the bodies wrap, so their on-screen height isn't
/// known here — let the bottom over-scroll into blank rather than hide wrapped tail lines (a precise
/// bottom clamp needs the rendered line count).
fn scrolled(scroll: u16, delta: i32) -> u16 {
    i32::from(scroll)
        .saturating_add(delta)
        .clamp(0, i32::from(u16::MAX)) as u16
}

/// The base `arc run <verb> .` command both the dry-run preview and the confirmed launch build on —
/// one definition so the previewed run can't drift from the real one (the gate's preview-equals-run
/// guarantee). The caller appends `--dry-run` (preview) or null stdio (launch). Errs if the running
/// `arc` binary can't be located.
fn launch_command(verb: &str) -> std::io::Result<std::process::Command> {
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.args([crate::cli::NAME_RUN, verb, "."]);
    Ok(cmd)
}

/// Run a verb's dry-run as a subprocess of this same `arc` binary and return its **preview** — the
/// header (params + estimate) a human dry-run prints before the prompt — for the gate, or an error
/// string. Zero spend (`--dry-run`); read from the `--json` payload's `preview` field — the
/// machine-readable channel — not parsed out of the human layout (prefer-machine-readable-tool-output).
fn dry_run_preview(verb: &str) -> Result<String, String> {
    let output = launch_command(verb)
        .map_err(|e| format!("can't locate the arc binary: {e}"))?
        .args(["--dry-run", "--json"])
        .output()
        .map_err(|e| format!("couldn't start the dry-run: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("the dry-run failed: {}", stderr.trim()));
    }
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("the dry-run's JSON payload didn't parse: {e}"))?;
    payload
        .get("preview")
        .and_then(serde_json::Value::as_str)
        .map(|preview| preview.trim_end().to_owned())
        .ok_or_else(|| "the dry-run's JSON payload has no `preview` field".to_owned())
}

/// What the live status reflects at one instant: the in-flight runs, how many registry entries couldn't
/// be read, the reference time for ages, and any error reading the registry — surfaced, never hidden.
/// Re-read every tick so both the status view *and* the global footer's count stay current.
struct Snapshot {
    active: Vec<ActiveRun>,
    unreadable: usize,
    now: u64,
    error: Option<String>,
    /// The recently-completed tail (newest-first column cells + the total completed-run count, one
    /// [`recent_completed`] build); `Err` if the log read failed (surfaced in the view, not collapsed
    /// into "nothing recent").
    recent: Result<RecentTail, String>,
}

impl Snapshot {
    /// Read the run registry fresh. A read failure is captured into `error` (and shown) rather than
    /// torn up the loop — a transient registry error shouldn't collapse the live view.
    fn read() -> Self {
        let now = crate::log::now_secs();
        let recent = recent_completed(now);
        match crate::runs::active() {
            Ok((active, unreadable)) => Self {
                active,
                unreadable: unreadable.len(),
                now,
                error: None,
                recent,
            },
            Err(e) => Self {
                active: Vec::new(),
                unreadable: 0,
                now,
                error: Some(format!("{e:#}")),
                recent,
            },
        }
    }
}

/// How many recently-completed runs the status tail shows.
const RECENT_RUNS: usize = 5;

/// Columns in the recently-completed tail: age, command, repo, outcome, cost.
const RECENT_COLS: usize = 5;

/// The recently-completed tail: the newest [`RECENT_RUNS`] rows, plus the total completed-run count so
/// the view can disclose when older runs are elided (the codebase discloses every other elision).
struct RecentTail {
    rows: Vec<[String; RECENT_COLS]>,
    total: usize,
}

/// Build the [`RecentTail`] from the log: the newest rows as `[age, command, repo, outcome, cost]`
/// cells. `age` is relative to `now`; outcome and cost are separate cells (a gate-blocked or errored run
/// still spent, so its cost shows beside the flag). A log-read failure is `Err` (surfaced in the view,
/// not collapsed into an empty tail). Re-read each tick; the log is small, and a tail-only read is a
/// later optimization.
fn recent_completed(now: u64) -> Result<RecentTail, String> {
    let (records, _) = crate::log::records_newest_first().map_err(|e| format!("{e:#}"))?;
    let total = records.len();
    let rows = records
        .iter()
        .take(RECENT_RUNS)
        .map(|r| {
            // How long ago the run finished leads the tail, since recency is its point; the `ts`→age
            // extraction is shared with `arc log`'s row (one missing-`ts` handling, no drift).
            let age = crate::commands::log::record_age(r, now);
            let cmd = crate::log::field(r, "command");
            let repo = crate::log::field(r, "repo");
            // The run's disposition (a column of its own, beside cost): blocked, errored, or ok.
            let outcome = if crate::log::is_blocked(r) {
                "blocked".to_owned()
            } else if crate::log::is_errored(r) {
                "errored".to_owned()
            } else {
                "ok".to_owned()
            };
            let cost = crate::commands::log::cost(r);
            [
                age,
                cmd,
                crate::log::repo_basename(&repo).to_owned(),
                outcome,
                cost,
            ]
        })
        .collect();
    Ok(RecentTail { rows, total })
}

/// The `tui` command. Owns the terminal (inline viewport) for its duration and restores it on exit
/// (and on panic, via the panic hook `ratatui::try_init_with_options` installs).
pub fn run(args: &TuiArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    // The TUI is interactive, not a JSON-emitting command, so reject `--json` rather than accept and
    // silently ignore it (an explicit option dropped is worse than a silent default).
    anyhow::ensure!(
        !global.json,
        "`--json` has no meaning for `arc tui` (it's an interactive view)"
    );
    // A TUI needs an interactive terminal — fail cleanly rather than entering raw mode against a pipe
    // (which would hang or corrupt non-interactive output).
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
    // Clean up the inline region on exit. `ratatui::restore()` only resets terminal modes — for an
    // inline viewport it clears nothing, so without this the last (mostly-blank) frame is stranded in
    // the terminal. `Terminal::clear()` is the inline clear: it moves to the viewport's absolute origin
    // and clears from there down, leaving scrollback above untouched. Park the cursor at that origin so
    // the shell's next prompt reclaims the space the viewport used — no blank gap, no leftover
    // masthead/footer. Order matters: clear + reposition run while raw mode is still on; restore()
    // (which clears nothing) comes last, and runs even if the loop errored (panics go through the hook).
    let _ = terminal.clear();
    let origin = terminal.get_frame().area().as_position();
    let _ = terminal.set_cursor_position(origin);
    let _ = terminal.show_cursor();
    let _ = terminal.backend_mut().flush();
    ratatui::restore();
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
        Msg::UpdateChecked(newer) => app.update = newer,
        Msg::LaunchPreview { verb, result } => {
            // Fold the dry-run's outcome into the gate only if the open launch is still the one it was
            // computed for — cancelled (`launch` is `None`) or replaced by another verb's launch, the
            // stale preview is dropped rather than dressing a newer pending action in an older one's
            // parameters. Verb identity suffices today because a launch is fully determined by its verb
            // (`launch_command` takes nothing else); shaped launches will need a per-launch id.
            if let Some(launch) = app.launch.as_mut()
                && launch.verb.name() == verb
            {
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
    // The launch gate is the top overlay. Enter confirms — fires the real run, the one spend the gate
    // exists to authorize (a no-op until the estimate is in). Esc / Ctrl-C is a HARD CANCEL / dismiss:
    // a spend gate's cancel must always mean "don't spend".
    if app.launch.is_some() {
        match key.code {
            KeyCode::Enter => app.confirm_launch(),
            KeyCode::Esc => app.launch = None,
            KeyCode::Char('c') if ctrl => app.launch = None,
            _ => {}
        }
        return;
    }
    // Ctrl-C quits from anywhere else — the universal escape hatch.
    if ctrl && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }
    // While a config edit is open, every key belongs to the input line — including `/`, `q`, and
    // Esc — so typed text can't trigger the global bindings underneath.
    if app.route == Route::Config
        && matches!(
            &app.config,
            Some(ConfigView::Loaded {
                editing: Some(_),
                ..
            })
        )
    {
        handle_config_edit_key(app, key.code);
        return;
    }
    if app.palette.is_some() {
        handle_palette_key(app, key.code);
        return;
    }
    match key.code {
        KeyCode::Char('/') => app.palette = Some(Palette::new()),
        KeyCode::Char('q') => app.should_quit = true,
        // Esc backs out one level: a section returns to home; home leaves the cockpit — but inside an
        // open log-run or rule detail it returns to that view's list first.
        KeyCode::Esc => {
            if app.route == Route::Log
                && let Some(log) = app.log.as_mut()
                && log.detail.is_some()
            {
                log.detail = None;
            } else if app.route == Route::Rules
                && let Some(rules) = app.rules.as_mut()
                && rules.detail.is_some()
            {
                rules.detail = None;
            } else {
                match app.route {
                    Route::Home => app.should_quit = true,
                    _ => app.route = Route::Home,
                }
            }
        }
        // Sections that own navigation state get their per-key handling here: the log's and the rules
        // view's cursor + detail scroll, the config table's cursor + edit entry, and the doctor
        // report's scroll.
        _ if app.route == Route::Log => handle_log_key(app, key.code),
        _ if app.route == Route::Rules => handle_rules_key(app, key.code),
        _ if app.route == Route::Config => handle_config_key(app, key.code),
        _ if app.route == Route::Doctor => handle_report_key(app, key.code),
        _ => {}
    }
}

/// Keys while the palette is open: edit the query, move the selection, run the choice, or dismiss.
fn handle_palette_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            // Esc backs out one level: from the run sub-menu to the top, else closes the palette.
            match app.palette.as_ref().map(|p| p.level) {
                Some(PaletteLevel::Run) => {
                    if let Some(p) = app.palette.as_mut() {
                        p.set_level(PaletteLevel::Top);
                    }
                }
                _ => app.palette = None,
            }
        }
        KeyCode::Enter => {
            // Pull the choice out first so the `app.palette` borrow ends before mutating `app`.
            let chosen = app
                .palette
                .as_ref()
                .and_then(|p| p.matches().get(p.selected).copied());
            match chosen {
                // `run` drills into the verb sub-menu — the palette stays open at the Run level.
                Some(Command::Run) => {
                    if let Some(p) = app.palette.as_mut() {
                        p.set_level(PaletteLevel::Run);
                    }
                }
                // A leaf command closes the palette and acts.
                Some(cmd) => {
                    app.palette = None;
                    cmd.apply(app);
                }
                None => {}
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

/// Keys while the `log` view is on screen (it owns a cursor, unlike the tick-refreshed sections): move
/// the selection or page through the list, open the highlighted run, or scroll an open detail. Esc is
/// handled one level up (it closes an open detail, else backs out of the view).
fn handle_log_key(app: &mut App, code: KeyCode) {
    let Some(log) = app.log.as_mut() else { return };
    // Detail open: arrows/page scroll the body; Home jumps to the top.
    if log.detail.is_some() {
        if let Some(delta) = scroll_delta(code, LIST_ROWS) {
            log.scroll_detail(delta);
        }
        return;
    }
    // List: move the cursor (keeping it visible) or open the highlighted run.
    let last = match &log.runs {
        Ok(runs) if !runs.is_empty() => runs.len() - 1,
        _ => return, // empty or unreadable — nothing to navigate
    };
    match list_action(code, log.selected, last) {
        Some(ListAction::Select(i)) => log.select(i),
        Some(ListAction::Open) => log.open_detail(),
        None => {}
    }
}

/// Render one frame from `app` — pure (state in, frame out). Every view is a body + the global footer;
/// the palette, when open, draws as an overlay on top.
fn render(frame: &mut Frame, app: &App) {
    let [body, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(LINE)]).areas(frame.area());

    match app.route {
        Route::Home => render_home(frame, body, &app.cwd_display, app.cwd_note.as_deref()),
        Route::Status => render_status(frame, &app.status, body),
        Route::Config => render_config(
            frame,
            app.config
                .as_ref()
                .expect("config is loaded when its route is active"),
            body,
        ),
        Route::Log => render_log(
            frame,
            app.log
                .as_ref()
                .expect("the log view is loaded when its route is active"),
            body,
        ),
        Route::Usage => render_usage(
            frame,
            app.usage
                .as_ref()
                .expect("the usage view is loaded when its route is active"),
            body,
        ),
        Route::Doctor => render_text_report(
            frame,
            body,
            "doctor",
            app.doctor
                .as_ref()
                .expect("the doctor view is loaded when its route is active"),
            app.report_scroll,
        ),
        Route::Rules => render_rules(
            frame,
            app.rules
                .as_ref()
                .expect("the rules view is loaded when its route is active"),
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

/// A warning for the home view if the launch directory is a poor place to run arc — the home folder
/// (a run there scans the whole home tree) or outside any git repo; None for a normal repo. Computed
/// once at startup (it does filesystem probes), so [`render`] stays a pure function of state.
fn cwd_warning(cwd: &Path) -> Option<String> {
    if Some(cwd) == dirs::home_dir().as_deref() {
        return Some(
            "home folder — a run here scans your whole home tree; cd into a repo first".to_owned(),
        );
    }
    // `.try_exists()` not `.exists()`: an unreadable `.git` (a permission hiccup) is "can't tell", not
    // "absent", so it isn't mislabeled "not a repo" (distinguish-absent-from-unreadable). Warn only when
    // every ancestor's `.git` is confirmed missing.
    if !cwd
        .ancestors()
        .any(|a| a.join(".git").try_exists().unwrap_or(true))
    {
        return Some(
            "not a git repository — runs aren't scoped to a project; cd into a repo".to_owned(),
        );
    }
    None
}

/// The home view the TUI opens on — a compact masthead (name + version, the target directory, and a
/// warning when that directory is a poor place to run arc). The footer carries live state and key
/// hints, so home doesn't repeat them; the space below is the open launchpad.
fn render_home(frame: &mut Frame, area: Rect, cwd_display: &str, note: Option<&str>) {
    // The masthead grows by a line when there's a cwd warning to show.
    let height = MASTHEAD_HEIGHT + if note.is_some() { LINE } else { 0 };
    let [masthead, _] =
        Layout::vertical([Constraint::Length(height), Constraint::Min(0)]).areas(area);
    let mut lines = vec![
        Line::from(format!("{} {VERSION}", crate::cli::binary_name())).bold(),
        Line::from(cwd_display).dim(),
    ];
    if let Some(w) = note {
        lines.push(Line::from(w).yellow());
    }
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

/// Column widths for the recently-completed tail — positionally paired with the header in
/// [`render_status`]: age, command, repo, the outcome flag, then cost taking the row's slack (wide
/// enough for the codex "tokens only" wording).
const RECENT_COLUMN_WIDTHS: [Constraint; RECENT_COLS] = [
    Constraint::Length(8),  // age ("12m ago")
    Constraint::Length(10), // command
    Constraint::Length(14), // repo basename
    Constraint::Length(8),  // outcome (blocked / errored / ok)
    Constraint::Min(8),     // cost — separate column, takes the slack
];

/// The live run-registry view: a header and a table of in-flight runs (or a message). The footer is
/// global now, so this owns only the section body.
fn render_status(frame: &mut Frame, snap: &Snapshot, area: Rect) {
    // header line, the in-flight table (flexes to fill), then a short recently-completed tail when
    // there is one — sized below to its rows. The tail renders as a column-aligned table (a header
    // above one row per run), like the active table; a log-read failure surfaces as a single line,
    // and no completed runs collapses the tail to nothing.
    let recent_h = match &snap.recent {
        Ok(tail) if tail.rows.is_empty() => 0,
        Ok(tail) => tail.rows.len() as u16 + LINE + BORDER, // rows + header row + border
        Err(_) => LINE + BORDER,                            // one error line + border
    };
    let [header, active_area, recent_area] = Layout::vertical([
        Constraint::Length(LINE),
        Constraint::Min(0),
        Constraint::Length(recent_h),
    ])
    .areas(area);

    frame.render_widget(Line::from("live status").bold(), header);

    if let Some(err) = &snap.error {
        frame.render_widget(
            Paragraph::new(format!("run registry unreadable: {err}")).block(Block::bordered()),
            active_area,
        );
    } else if snap.active.is_empty() {
        frame.render_widget(
            Paragraph::new("no runs in flight").block(Block::bordered()),
            active_area,
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
        frame.render_widget(table, active_area);
    }

    // The recently-completed tail, column-aligned (see `recent_completed`); a log-read failure shows
    // as a single line instead.
    match &snap.recent {
        Ok(tail) if tail.rows.is_empty() => {}
        Ok(tail) => {
            // Disclose when older runs are elided (showing N of M), matching the codebase's
            // elision-disclosure standard (arc log's "… N older run(s)", inspect's "+N more").
            let title = if tail.total > tail.rows.len() {
                format!("recently completed · {} of {}", tail.rows.len(), tail.total)
            } else {
                "recently completed".to_owned()
            };
            let table = Table::new(
                tail.rows.iter().map(|c| Row::new(c.clone())),
                RECENT_COLUMN_WIDTHS,
            )
            .header(
                Row::new(["age", "command", "repo", "outcome", "cost"]).style(Style::new().bold()),
            )
            .block(Block::bordered().title(title));
            frame.render_widget(table, recent_area);
        }
        Err(e) => {
            frame.render_widget(
                Paragraph::new(format!("run log unreadable: {e}"))
                    .block(Block::bordered().title("recently completed")),
                recent_area,
            );
        }
    }
}

/// Column widths for the config table: the dotted setting key (wide enough for the longest current
/// key), then the resolved value taking the row's slack.
const CONFIG_COLUMN_WIDTHS: [Constraint; 2] = [Constraint::Length(32), Constraint::Min(10)];

/// The config view: every resolved default (after user-then-project layering) and the active settings
/// layers — the projection `arc config list` prints, shown here. Read-only for now; editing is the
/// follow-up (likely arrow-key pickers, shared with the launch-config cut).
fn render_config(frame: &mut Frame, config: &ConfigView, area: Rect) {
    let [header, body, layers_line] = Layout::vertical([
        Constraint::Length(LINE),
        Constraint::Min(0),
        Constraint::Length(LINE),
    ])
    .areas(area);
    frame.render_widget(Line::from("config").bold(), header);

    match config {
        ConfigView::Error(e) => frame.render_widget(
            Paragraph::new(format!("settings unreadable: {e}")).block(Block::bordered()),
            body,
        ),
        ConfigView::Loaded {
            values,
            layers,
            selected,
            editing,
            error,
        } => {
            let rows = values.iter().enumerate().map(|(i, (key, value))| {
                // While editing, the selected row's value cell is the input buffer with a caret.
                let cell = match editing {
                    Some(buffer) if i == *selected => format!("{buffer}█"),
                    _ => value.clone(),
                };
                let row = Row::new([key.clone(), cell]);
                if i == *selected {
                    row.style(Style::new().reversed()) // the cursor
                } else {
                    row
                }
            });
            let table = Table::new(rows, CONFIG_COLUMN_WIDTHS)
                .header(Row::new(["setting", "value"]).style(Style::new().bold()))
                .block(Block::bordered());
            frame.render_widget(table, body);

            // A rejected edit's error outranks the routine layers fact until the next action.
            let info = match error {
                Some(e) => Line::from(e.clone()).dim(),
                None => Line::from(format!(
                    "layers: {}",
                    crate::join_or(layers, crate::settings::NO_LAYERS)
                ))
                .dim(),
            };
            frame.render_widget(info, layers_line);
        }
    }
}

/// A compact token count for table cells — `16.2M`, `412.8K`, `999` — so the big token sums fit a
/// column instead of wrapping the way the CLI's full-width numbers do.
fn compact(n: u64) -> String {
    // SI abbreviation thresholds: a million abbreviates to `M`, a thousand to `K`. Named so each
    // boundary and the divisor it implies are a single value, not a bare literal repeated inline.
    const MILLION: u64 = 1_000_000;
    const THOUSAND: u64 = 1_000;
    if n >= MILLION {
        format!("{:.1}M", n as f64 / MILLION as f64)
    } else if n >= THOUSAND {
        format!("{:.1}K", n as f64 / THOUSAND as f64)
    } else {
        n.to_string()
    }
}

/// Period-table columns: the window label, run/blocked/errored counts, the compact token breakdown,
/// then cost taking the slack.
const USAGE_PERIOD_WIDTHS: [Constraint; 9] = [
    Constraint::Length(6),
    Constraint::Length(5),
    Constraint::Length(5),
    Constraint::Length(5),
    Constraint::Length(6),
    Constraint::Length(8),
    Constraint::Length(8),
    Constraint::Length(6),
    Constraint::Min(8),
];

/// By-command columns: the verb, its run count, and all-time cost.
const USAGE_COMMAND_WIDTHS: [Constraint; 3] = [
    Constraint::Length(12),
    Constraint::Length(6),
    Constraint::Min(8),
];

/// The doctor view's body: a bold title over the report — or its error, prefixed "<title> unreadable"
/// — in a bordered, wrapped paragraph, scrolled by `scroll`.
fn render_text_report(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    report: &Result<String, String>,
    scroll: u16,
) {
    let [header, body] =
        Layout::vertical([Constraint::Length(LINE), Constraint::Min(0)]).areas(area);
    frame.render_widget(Line::from(title).bold(), header);
    let text = match report {
        Ok(report) => report.clone(),
        Err(e) => format!("{title} unreadable: {e}"),
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::bordered())
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        body,
    );
}

/// Rows of the doctor report visible at once — the inline viewport less the footer, the view header,
/// and the bordered body's frame — the PageUp/PageDown step for its scroll.
const REPORT_ROWS: usize = (VIEWPORT_HEIGHT - LINE - LINE - BORDER) as usize;

/// Map a key to a scroll delta for a text body — line steps, page jumps of `page`, Home to the top;
/// `None` for keys that don't scroll. The one keymap for every scrolled body (log detail, rule detail,
/// doctor report), so the views can't drift on navigation.
fn scroll_delta(code: KeyCode, page: usize) -> Option<i32> {
    Some(match code {
        KeyCode::Up => -1,
        KeyCode::Down => 1,
        KeyCode::PageUp => -(page as i32),
        KeyCode::PageDown => page as i32,
        KeyCode::Home => i32::MIN,
        _ => return None,
    })
}

/// What a key means over a cursored list: move the cursor to a target row, or open the selected one.
enum ListAction {
    Select(usize),
    Open,
}

/// Map a key over a cursored list of `last + 1` rows with the cursor at `selected` — the one keymap
/// for every list view (log, rules), so their navigation can't drift. `None` for keys the list
/// doesn't handle.
fn list_action(code: KeyCode, selected: usize, last: usize) -> Option<ListAction> {
    Some(match code {
        KeyCode::Up => ListAction::Select(selected.saturating_sub(1)),
        KeyCode::Down => ListAction::Select((selected + 1).min(last)),
        KeyCode::PageUp => ListAction::Select(selected.saturating_sub(LIST_ROWS)),
        KeyCode::PageDown => ListAction::Select((selected + LIST_ROWS).min(last)),
        KeyCode::Home => ListAction::Select(0),
        KeyCode::End => ListAction::Select(last),
        KeyCode::Enter => ListAction::Open,
        _ => return None,
    })
}

/// Keys while the doctor report is on screen: scroll it — a tool-rich machine's report can outrun the
/// inline viewport, and clipping the tail silently would misreport the very state the view exists to
/// show. Esc is handled one level up.
fn handle_report_key(app: &mut App, code: KeyCode) {
    if let Some(delta) = scroll_delta(code, REPORT_ROWS) {
        app.report_scroll = scrolled(app.report_scroll, delta);
    }
}

/// Keys while the `rules` view is on screen: move the cursor or page through the rule list, open the
/// highlighted rule, toggle it on/off with space, or scroll an open rule's body. Esc is handled one
/// level up (it closes an open rule, else backs out of the view).
fn handle_rules_key(app: &mut App, code: KeyCode) {
    // The toggle is staged out of the view borrow — it rewrites settings and replaces the whole view.
    let mut toggle = false;
    {
        let Some(view) = app.rules.as_mut() else {
            return;
        };
        if let Some(detail) = view.detail.as_mut() {
            if let Some(delta) = scroll_delta(code, LIST_ROWS) {
                detail.scroll = scrolled(detail.scroll, delta);
            }
            return;
        }
        let last = match &view.report {
            Ok(report) if !report.rules.is_empty() => report.rules.len() - 1,
            _ => return, // empty or unresolvable — nothing to navigate
        };
        match code {
            KeyCode::Char(' ') => toggle = true,
            _ => match list_action(code, view.selected, last) {
                Some(ListAction::Select(i)) => view.select(i),
                Some(ListAction::Open) => view.open_detail(),
                None => {}
            },
        }
    }
    if toggle {
        app.toggle_selected_rule();
    }
}

/// Keys while browsing the config table: move the cursor, or open the selected setting for editing —
/// prefilled with the current value (an unset one starts empty). Esc is handled one level up.
fn handle_config_key(app: &mut App, code: KeyCode) {
    let Some(ConfigView::Loaded {
        values,
        selected,
        editing,
        error,
        ..
    }) = app.config.as_mut()
    else {
        return;
    };
    let last = values.len().saturating_sub(1);
    match code {
        KeyCode::Up => *selected = selected.saturating_sub(1),
        KeyCode::Down => *selected = (*selected + 1).min(last),
        KeyCode::Home => *selected = 0,
        KeyCode::End => *selected = last,
        KeyCode::Enter => {
            let current = &values[*selected].1;
            *editing = Some(if current == crate::settings::UNSET {
                String::new()
            } else {
                current.clone()
            });
            *error = None;
        }
        _ => {}
    }
}

/// Keys while a config edit is open: edit the buffer, save (validated and written via the shared
/// `arc config set` path, then re-resolved), or cancel. Runs ahead of the global bindings, so typed
/// text can't quit the cockpit or open the palette.
fn handle_config_edit_key(app: &mut App, code: KeyCode) {
    // The save is staged out of the view borrow — it rewrites settings and replaces the whole view.
    let mut save: Option<(String, String, usize)> = None;
    if let Some(ConfigView::Loaded {
        values,
        selected,
        editing,
        error,
        ..
    }) = app.config.as_mut()
        && let Some(buffer) = editing.as_mut()
    {
        match code {
            KeyCode::Enter => save = Some((values[*selected].0.clone(), buffer.clone(), *selected)),
            KeyCode::Esc => {
                *editing = None;
                *error = None;
            }
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) => buffer.push(c),
            _ => {}
        }
    }
    if let Some((key, value, keep)) = save {
        app.save_config(&key, &value, keep);
    }
}

/// The rules view: the resolved ruleset as a browsable list — cursor over the rule ids, Enter opens a
/// rule's body to read — or the resolution error. Loaded on entry by [`App::open_rules`]. Each screen
/// is a boxed body between the title line and an information line (the list: rule count + any
/// skipped-source disclosure; an open rule: its full path) — facts only, controls in the global footer.
///
/// Per-row provenance appears at the *pool* level (a rule's parent directory), and only when pools
/// differ — that's when which source won an id can vary. A rule's filename stem is its id, so a full
/// per-row path would say the id twice; with one shared pool the rows are bare ids. (The CLI's
/// `arc rules` keeps full per-line paths deliberately — the grep-friendly, agent-first form.)
fn render_rules(frame: &mut Frame, view: &RulesView, area: Rect) {
    let [header, body, info] = Layout::vertical([
        Constraint::Length(LIST_HEADER_LINES),
        Constraint::Min(0),
        Constraint::Length(LIST_INFO_LINES),
    ])
    .areas(area);

    let report = match &view.report {
        Ok(report) => report,
        Err(e) => {
            frame.render_widget(Line::from("rules").bold(), header);
            frame.render_widget(
                Paragraph::new(format!("rules unresolvable: {e}")).block(Block::bordered()),
                body,
            );
            return;
        }
    };

    // Drilled into one rule: its body, scrolled; the info line carries the rule's full path (and its
    // off state, when disabled).
    if let Some(detail) = &view.detail {
        let rule = &report.rules[detail.index];
        frame.render_widget(Line::from(format!("rules · {}", rule.id)).bold(), header);
        frame.render_widget(
            Paragraph::new(rule.body.clone())
                .block(Block::bordered())
                .wrap(Wrap { trim: false })
                .scroll((detail.scroll, 0)),
            body,
        );
        let mut info_text = rule.source.clone();
        if rule.disabled {
            info_text.push_str(" · disabled");
        }
        frame.render_widget(Line::from(info_text).dim(), info);
        return;
    }

    // The rule list.
    frame.render_widget(
        Line::from(format!("rules · {}", report.description)).bold(),
        header,
    );
    // A rule's pool: the directory its file lives in (an ad-hoc root-level file falls back to `.`).
    let pool = |source: &str| match Path::new(source).parent() {
        Some(p) if !p.as_os_str().is_empty() => p.display().to_string(),
        _ => ".".to_owned(),
    };
    // One pool shared by every rule → a per-row pool column would be pure repetition; show bare ids.
    let one_pool = report.rules.split_first().is_none_or(|(first, rest)| {
        let p = pool(&first.source);
        rest.iter().all(|r| pool(&r.source) == p)
    });
    if report.rules.is_empty() {
        frame.render_widget(
            Paragraph::new("no rules resolve from the active ruleset").block(Block::bordered()),
            body,
        );
    } else {
        // With several pools, pad the ids to one column so the dimmed pools align instead of raggedly
        // trailing each id.
        let id_width = report
            .rules
            .iter()
            .map(|r| r.id.chars().count())
            .max()
            .unwrap_or(0);
        let end = (view.offset + LIST_ROWS).min(report.rules.len());
        let rows: Vec<Line> = report.rules[view.offset..end]
            .iter()
            .enumerate()
            .map(|(i, r)| {
                // A two-char gutter marks a disabled rule, and its whole row dims — the off state
                // reads at a glance without breaking the id/pool column alignment.
                let gutter = if r.disabled { "✗ " } else { "  " };
                let line = if one_pool {
                    Line::from(format!("{gutter}{}", r.id))
                } else {
                    Line::from(vec![
                        Span::from(format!("{gutter}{:<id_width$}  ", r.id)),
                        Span::from(pool(&r.source)).dim(),
                    ])
                };
                let line = if r.disabled { line.dim() } else { line };
                if view.offset + i == view.selected {
                    line.reversed() // the cursor
                } else {
                    line
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(rows).block(Block::bordered()), body);
    }
    // A failed toggle's error outranks the routine facts until the next action.
    if let Some(notice) = &view.notice {
        frame.render_widget(Line::from(notice.clone()).dim(), info);
        return;
    }
    let disabled = report.rules.iter().filter(|r| r.disabled).count();
    let mut info_text = format!("{} rule(s)", report.rules.len());
    if disabled > 0 {
        info_text.push_str(&format!(" · {disabled} disabled"));
    }
    if !report.disabled_unmatched.is_empty() {
        info_text.push_str(&format!(
            " · {} disabled id(s) match no rule",
            report.disabled_unmatched.len()
        ));
    }
    if !report.skipped.is_empty() {
        info_text.push_str(&format!(" · {} source(s) skipped", report.skipped.len()));
    }
    frame.render_widget(Line::from(info_text).dim(), info);
}

/// The usage view: the run-log spend/token rollup `arc usage` computes, rendered as tables — periods
/// (hour/day/week/all-time) and per-command — instead of the CLI's flat text, with the codex/missing
/// disclosures below. Re-loaded on entry (so a run since shows), and the same `usage::rollup` payload
/// backs the CLI, so the two can't drift.
fn render_usage(frame: &mut Frame, usage: &Result<Rollup, String>, area: Rect) {
    let [header, body] =
        Layout::vertical([Constraint::Length(LINE), Constraint::Min(0)]).areas(area);
    frame.render_widget(Line::from("usage").bold(), header);

    let rollup = match usage {
        Ok(r) => r,
        Err(e) => {
            frame.render_widget(
                Paragraph::new(format!("usage unreadable: {e}")).block(Block::bordered()),
                body,
            );
            return;
        }
    };

    // periods and by-command on top (each fixed, sized to its rows), then the notes take the remaining
    // space (Min) so a long disclosure wraps onto multiple lines instead of being clipped at the border.
    let [periods_area, commands_area, notes_area] = Layout::vertical([
        Constraint::Length(rollup.windows.len() as u16 + LINE + BORDER),
        Constraint::Length(rollup.by_command.len() as u16 + LINE + BORDER),
        Constraint::Min(0),
    ])
    .areas(body);

    let period_rows = rollup.windows.iter().map(|w| {
        Row::new([
            w.window.to_owned(),
            w.runs.to_string(),
            w.blocked.to_string(),
            w.errored.to_string(),
            compact(w.input_tokens),
            compact(w.cache_creation_input_tokens),
            compact(w.cache_read_input_tokens),
            compact(w.output_tokens),
            crate::log::cost_display(w.cost_usd),
        ])
    });
    frame.render_widget(
        Table::new(period_rows, USAGE_PERIOD_WIDTHS)
            .header(
                Row::new([
                    "window", "runs", "blkd", "errd", "in", "cache wr", "cache rd", "out", "cost",
                ])
                .style(Style::new().bold()),
            )
            .block(Block::bordered().title("spend & tokens")),
        periods_area,
    );

    let cmd_rows = rollup.by_command.iter().map(|c| {
        Row::new([
            c.command.clone(),
            c.runs.to_string(),
            crate::log::cost_display(c.cost_usd),
        ])
    });
    frame.render_widget(
        Table::new(cmd_rows, USAGE_COMMAND_WIDTHS)
            .header(Row::new(["command", "runs", "cost"]).style(Style::new().bold()))
            .block(Block::bordered().title("by command (all-time)")),
        commands_area,
    );

    if !rollup.notes.is_empty() {
        let lines: Vec<Line> = rollup
            .notes
            .iter()
            .map(|n| Line::from(n.as_str()).dim())
            .collect();
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered()),
            notes_area,
        );
    }
}

/// The `log` view: a cursor-driven list of completed runs (newest first), or the selected run's detail
/// when drilled in. The rows reuse `arc log`'s projection and the detail reuses `arc log <id>`'s, so a
/// run reads the same in the cockpit as on the CLI.
fn render_log(frame: &mut Frame, log: &LogView, area: Rect) {
    let [header, body, info] = Layout::vertical([
        Constraint::Length(LIST_HEADER_LINES),
        Constraint::Min(0),
        Constraint::Length(LIST_INFO_LINES),
    ])
    .areas(area);

    let runs = match &log.runs {
        Ok(runs) => runs,
        Err(e) => {
            frame.render_widget(Line::from("log").bold(), header);
            frame.render_widget(
                Paragraph::new(format!("run log unreadable: {e}")).block(Block::bordered()),
                body,
            );
            return;
        }
    };

    // Drilled into one run: its rendered result, scrolled; the info line carries the run id.
    if let Some(detail) = &log.detail {
        frame.render_widget(Line::from("log · run detail").bold(), header);
        frame.render_widget(
            Paragraph::new(detail.body.clone())
                .block(Block::bordered())
                .wrap(Wrap { trim: false })
                .scroll((detail.scroll, 0)),
            body,
        );
        if let Some(id) = &detail.id {
            frame.render_widget(Line::from(id.clone()).dim(), info);
        }
        return;
    }

    // The list of runs; the info line carries the count and any unparsed-record disclosure.
    frame.render_widget(Line::from("completed runs").bold(), header);
    if runs.is_empty() {
        frame.render_widget(
            Paragraph::new("no runs logged yet").block(Block::bordered()),
            body,
        );
    } else {
        let end = (log.offset + LIST_ROWS).min(runs.len());
        let rows: Vec<Line> = runs[log.offset..end]
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let line = Line::from(crate::commands::log::row(r, log.now));
                if log.offset + i == log.selected {
                    line.reversed() // the cursor
                } else {
                    line
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(rows).block(Block::bordered()), body);
    }
    let mut info_text = format!("{} run(s)", runs.len());
    if log.unparsed > 0 {
        info_text.push_str(&format!(" · {}", crate::log::unparsed_note(log.unparsed)));
    }
    frame.render_widget(Line::from(info_text).dim(), info);
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
        // The gate's keys by stage (the modal shows no hint — this footer is it): Enter fires the run
        // at the confirm stage; Esc cancels a pending launch or dismisses a failed preview.
        match &launch.stage {
            LaunchStage::Preparing => "esc cancel",
            LaunchStage::Confirming { .. } => "enter run · esc cancel",
            LaunchStage::Failed { .. } => "esc dismiss",
        }
    } else if app.palette.is_some() {
        "↑↓ select · enter run · esc close"
    } else {
        // The stateful views' hints track their mode — the same keys move a list but scroll an opened
        // body, and a config edit owns the keyboard. Info lines carry facts only; every control is
        // named here.
        const SCROLL: &str = "/ commands · ↑↓ scroll · esc back · q quit";
        const BROWSE: &str = "/ commands · ↑↓ move · enter open · esc back · q quit";
        match app.route {
            Route::Home => "/ commands · q quit",
            Route::Doctor => SCROLL,
            Route::Log => {
                if app.log.as_ref().is_some_and(|l| l.detail.is_some()) {
                    SCROLL
                } else {
                    BROWSE
                }
            }
            Route::Rules => {
                if app.rules.as_ref().is_some_and(|r| r.detail.is_some()) {
                    SCROLL
                } else {
                    "/ commands · ↑↓ move · enter open · space toggle · esc back · q quit"
                }
            }
            Route::Config => {
                if matches!(
                    &app.config,
                    Some(ConfigView::Loaded {
                        editing: Some(_),
                        ..
                    })
                ) {
                    "enter save · esc cancel"
                } else {
                    "/ commands · ↑↓ move · enter edit · esc back · q quit"
                }
            }
            Route::Status | Route::Usage => "/ commands · esc back · q quit",
        }
    };

    let status = format!("{runs}  ·  {hints}");
    let line = match &app.update {
        // A newer release found at startup — flag it, undimmed, after the status and key hints.
        Some(version) => Line::from(vec![
            Span::from(status).dim(),
            Span::from(format!("  ·  ⬆ arc {version}")).yellow(),
        ]),
        None => Line::from(status).dim(),
    };
    frame.render_widget(line, area);
}

/// The `/` command palette: a centered popup with the typed query and the prefix-matched commands,
/// the selection highlighted. Drawn over whatever section is active.
fn render_palette(frame: &mut Frame, palette: &Palette, area: Rect) {
    let matches = palette.matches();
    // Height: border (2) + input line (1) + one row per match (at least one, for the empty message).
    let rows = matches.len().max(1) as u16;
    let rect = centered(area, PALETTE_WIDTH, rows + BORDER + LINE);
    frame.render_widget(Clear, rect); // punch through whatever's underneath

    // Breadcrumb the level so the run sub-menu reads as a drill-in, not a separate palette.
    let title = match palette.level {
        PaletteLevel::Top => "commands",
        PaletteLevel::Run => "commands › run",
    };
    let block = Block::bordered().title(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let [input_area, list_area] =
        Layout::vertical([Constraint::Length(LINE), Constraint::Min(0)]).areas(inner);
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
/// token/cost estimate) to confirm or cancel before any spend. Enter fires the real run in the
/// background ([`App::confirm_launch`]); Esc cancels. The footer carries the keys.
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
    let height = (lines.len() as u16 + BORDER).min(area.height); // border + body; the footer holds the hint
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
            recent: Ok(RecentTail {
                rows: Vec::new(),
                total: 0,
            }),
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
            log: None,
            usage: None,
            cwd_note: None,
            cwd_display: ".".to_owned(),
            update: None,
            doctor: None,
            rules: None,
            report_scroll: 0,
        }
    }

    #[test]
    fn footer_flags_a_newer_release() {
        let mut app = app_with(Route::Home, empty_snapshot());
        app.update = Some("9.9.9".to_owned());
        assert!(
            screen(&app, 80, 6).contains("⬆ arc 9.9.9"),
            "the footer should flag a newer release the startup check found"
        );
    }

    #[test]
    fn doctor_view_renders_the_report() {
        let mut app = app_with(Route::Doctor, empty_snapshot());
        app.doctor = Some(Ok("arclite  9.9.9\nos  testos / testarch".to_owned()));
        let rendered = screen(&app, 80, 10);
        assert!(
            rendered.contains("doctor"),
            "the doctor view shows its header"
        );
        assert!(
            rendered.contains("9.9.9"),
            "the doctor view renders the report text"
        );
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

    #[test]
    fn palette_groups_verbs_under_a_run_submenu() {
        let mut app = app_with(Route::Home, empty_snapshot());
        update(&mut app, press(KeyCode::Char('/')));
        // Top level lists the `run` group, not the verbs themselves (mirrors `arc run <verb>`).
        let top: Vec<&str> = app
            .palette
            .as_ref()
            .unwrap()
            .matches()
            .iter()
            .map(|c| c.name())
            .collect();
        assert!(
            top.contains(&crate::cli::NAME_RUN),
            "the run group leads the top level"
        );
        assert!(
            !top.contains(&crate::cli::NAME_CRITIQUE),
            "verbs are grouped under run, not flattened at the top"
        );
        // Selecting `run` drills into the verb sub-menu without closing the palette.
        for ch in crate::cli::NAME_RUN.chars() {
            update(&mut app, press(KeyCode::Char(ch)));
        }
        update(&mut app, press(KeyCode::Enter));
        let sub = app
            .palette
            .as_ref()
            .expect("run opens the verb sub-menu rather than closing");
        assert_eq!(sub.level, PaletteLevel::Run);
        let verbs: Vec<&str> = sub.matches().iter().map(|c| c.name()).collect();
        assert!(
            verbs.contains(&crate::cli::NAME_AUDIT) && verbs.contains(&crate::cli::NAME_CRITIQUE),
            "the run sub-menu lists the synthesis verbs"
        );
        assert!(
            !verbs.contains(&crate::cli::NAME_RUN),
            "the sub-menu is verbs only"
        );
    }

    #[test]
    fn palette_esc_backs_out_of_run_submenu_then_closes() {
        let mut app = app_with(Route::Home, empty_snapshot());
        update(&mut app, press(KeyCode::Char('/')));
        for ch in crate::cli::NAME_RUN.chars() {
            update(&mut app, press(KeyCode::Char(ch)));
        }
        update(&mut app, press(KeyCode::Enter));
        assert_eq!(
            app.palette.as_ref().map(|p| p.level),
            Some(PaletteLevel::Run),
            "selecting run enters the sub-menu"
        );
        update(&mut app, press(KeyCode::Esc));
        assert_eq!(
            app.palette.as_ref().map(|p| p.level),
            Some(PaletteLevel::Top),
            "esc in the sub-menu backs out to the top level"
        );
        update(&mut app, press(KeyCode::Esc));
        assert!(
            app.palette.is_none(),
            "esc at the top level closes the palette"
        );
    }

    #[test]
    fn palette_top_level_includes_usage() {
        let mut app = app_with(Route::Home, empty_snapshot());
        update(&mut app, press(KeyCode::Char('/')));
        let top: Vec<&str> = app
            .palette
            .as_ref()
            .unwrap()
            .matches()
            .iter()
            .map(|c| c.name())
            .collect();
        assert!(
            top.contains(&"usage"),
            "the usage view is a top-level palette entry, like status/config/log"
        );
    }
}
