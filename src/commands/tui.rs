//! `arc tui` — the human-facing, inline front-end over arclite (the CLI stays the agent/automation
//! interface). This is the foundation: an **inline** live region (stock ratatui `Viewport::Inline` —
//! drawn in the normal terminal buffer, the shell's scrollback preserved above; NOT an alt-screen
//! takeover, matching Claude Code / Codex), driven by a **sync** loop with no tokio.
//!
//! Runtime shape (gitui's model): a dedicated **input thread** (blocking `event::read`) and a **tick
//! thread** both feed one `std::sync::mpsc<Msg>`; the main loop blocks on it, applies the message via
//! [`update`], and redraws once. A `Tick` re-reads live state — so the view refreshes in place rather
//! than the user re-running `arc status`. `render`/`render_status` are pure functions of state, tested
//! headlessly with `TestBackend`; the interactive loop itself needs a real terminal.
//!
//! This is cut #1 (live Status). Slash-command palette, the other sections (runs/usage/rules/config),
//! drill-in overlays, and Launch-with-live-streaming are the next cuts on this same runtime — see the
//! `arc-tui-architecture` blueprint. The structure is kept deliberately small for one view; the
//! `Msg`/`update`/route split formalizes when the second view lands.
#![deny(clippy::print_stdout, clippy::print_stderr)] // never print while the TUI owns the terminal

use std::io::IsTerminal;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Row, Table};
use ratatui::{Frame, TerminalOptions, Viewport};

use crate::cli::{GlobalArgs, TuiArgs};
use crate::runs::ActiveRun;

/// Default seconds between live refreshes when `--interval` is omitted. One second keeps the registry
/// view current without busy-spinning; `--interval` overrides it (and is echoed in `--help`).
pub const DEFAULT_INTERVAL_SECS: f64 = 1.0;

/// Lines reserved for the inline live region. Tall enough for the status header, a small run table,
/// and the footer; the shell's scrollback stays visible above it. (Dynamic height that grows with the
/// section is a known refinement — see the blueprint's "inline-height management" note.)
const VIEWPORT_HEIGHT: u16 = 16;

/// A typed input/event — the single funnel into [`update`]. The input + tick threads both send these.
enum Msg {
    /// A raw terminal event (key, resize, …) from the input thread.
    Input(Event),
    /// The refresh tick: re-read live state.
    Tick,
}

/// All TUI state. `render` reads it and never mutates it; `update` is the only mutator. (One view today;
/// a `route`/overlay stack joins this when the second section/palette lands.)
struct App {
    status: Snapshot,
    should_quit: bool,
}

impl App {
    fn new() -> Self {
        Self {
            status: Snapshot::read(),
            should_quit: false,
        }
    }
}

/// What the live view shows at one instant: the in-flight runs, how many registry entries couldn't be
/// read, the reference time for ages, and any error reading the registry — surfaced, never hidden.
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
    // transcript — codex/Claude do that); but a frozen *status* frame just reads as stale, so wipe it
    // and let the shell prompt resume clean (codex likewise clears its live composer on exit).
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

/// The one place `App` state changes.
fn update(app: &mut App, msg: Msg) {
    match msg {
        Msg::Tick => app.status = Snapshot::read(),
        Msg::Input(Event::Key(key)) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.should_quit = true;
            }
            _ => {} // the view refreshes on the tick — no manual refresh key needed
        },
        Msg::Input(_) => {} // resize/focus/release → just redraw next iteration
    }
}

/// Render one frame from `app` — pure (state in, frame out). Dispatches to the active section's render
/// (only Status today).
fn render(frame: &mut Frame, app: &App) {
    render_status(frame, &app.status);
}

/// The live run-registry view: header, a table of in-flight runs (or a message), and a footer of keys.
fn render_status(frame: &mut Frame, snap: &Snapshot) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    frame.render_widget(Line::from("arc — live status").bold(), header);

    if let Some(err) = &snap.error {
        frame.render_widget(
            Paragraph::new(format!("run registry unreadable: {err}")).block(Block::bordered()),
            body,
        );
    } else if snap.active.is_empty() {
        frame.render_widget(
            Paragraph::new("no active runs").block(Block::bordered()),
            body,
        );
    } else {
        let rows = snap.active.iter().map(|r| {
            Row::new([
                r.command.clone(),
                basename(&r.repo),
                r.model.clone(),
                format!("{}s", snap.now.saturating_sub(r.started_at)),
                r.turns.to_string(),
                r.tool_calls.to_string(),
                r.output_chars.to_string(),
            ])
        });
        let widths = [
            Constraint::Length(10),
            Constraint::Min(12),
            Constraint::Length(18),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(9),
        ];
        let table = Table::new(rows, widths)
            .header(
                Row::new(["command", "repo", "model", "age", "turns", "tools", "chars"])
                    .style(Style::new().bold()),
            )
            .block(Block::bordered().title(format!("{} active run(s)", snap.active.len())));
        frame.render_widget(table, body);
    }

    let mut hints = String::from("q quit");
    if snap.unreadable > 0 {
        hints.push_str(&format!(
            " · {} unreadable registry entr{}",
            snap.unreadable,
            if snap.unreadable == 1 { "y" } else { "ies" }
        ));
    }
    frame.render_widget(Line::from(hints).dim(), footer);
}

/// The last path segment of a repo path — a compact cell; the full path is in `arc status`/`arc log`.
fn basename(repo: &str) -> String {
    repo.rsplit(['/', '\\']).next().unwrap_or(repo).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Render a status snapshot to an in-memory backend and return the screen as one string.
    fn status_screen(snap: &Snapshot, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render_status(frame, snap)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn empty_registry_says_no_active_runs() {
        let snap = Snapshot {
            active: Vec::new(),
            unreadable: 0,
            now: 100,
            error: None,
        };
        let text = status_screen(&snap, 60, 8);
        assert!(text.contains("arc — live status"));
        assert!(text.contains("no active runs"));
        assert!(text.contains("q quit"));
    }

    #[test]
    fn active_run_is_rendered_with_age_and_progress() {
        let snap = Snapshot {
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
        };
        let text = status_screen(&snap, 100, 8);
        assert!(text.contains("audit"));
        assert!(text.contains("ida")); // basename of the repo path
        assert!(text.contains("10s")); // now - started_at
        assert!(text.contains("1 active run(s)"));
    }

    #[test]
    fn registry_error_is_surfaced() {
        let snap = Snapshot {
            active: Vec::new(),
            unreadable: 0,
            now: 100,
            error: Some("permission denied".to_owned()),
        };
        let text = status_screen(&snap, 80, 8);
        assert!(text.contains("unreadable"));
        assert!(text.contains("permission denied"));
    }

    #[test]
    fn quit_keys_set_should_quit() {
        let key = |code: KeyCode, m: KeyModifiers| {
            Msg::Input(Event::Key(ratatui::crossterm::event::KeyEvent::new(code, m)))
        };
        for (code, mods) in [
            (KeyCode::Char('q'), KeyModifiers::NONE),
            (KeyCode::Esc, KeyModifiers::NONE),
            (KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut app = App::new();
            update(&mut app, key(code, mods));
            assert!(app.should_quit, "{code:?} + {mods:?} should quit");
        }
    }
}
