//! `arc tui` — an interactive terminal view over arclite. The MVP is a self-refreshing live view of
//! the run registry: `arc status`, but updating in place. State is re-read each tick and rendered
//! immediately; [`render`] is a pure function of [`Snapshot`], so the view is tested headlessly with
//! ratatui's `TestBackend` (the interactive loop itself needs a real terminal). Inline/no-alt-screen
//! mode and result-browsing views (log, results) are open follow-ups.

use std::io::IsTerminal;
use std::time::Duration;

use anyhow::Context;
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Row, Table};

use crate::cli::{GlobalArgs, TuiArgs};
use crate::runs::ActiveRun;

/// Default seconds between live refreshes when `--interval` is omitted. One second keeps the registry
/// view current without busy-spinning; `--interval` overrides it (and is echoed in `--help`).
pub const DEFAULT_INTERVAL_SECS: f64 = 1.0;

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

/// The `tui` command. Owns the terminal for its duration and restores it on exit (and on panic, via
/// the panic hook `ratatui::try_init` installs).
pub fn run(args: &TuiArgs, _global: &GlobalArgs) -> anyhow::Result<()> {
    // A TUI needs an interactive terminal — fail cleanly rather than entering raw mode against a pipe
    // (which would hang or corrupt non-interactive output). This is also why `--json` has no meaning here.
    anyhow::ensure!(
        std::io::stdout().is_terminal() && std::io::stdin().is_terminal(),
        "`arc tui` needs an interactive terminal (stdin/stdout are not a TTY)"
    );
    anyhow::ensure!(args.interval > 0.0, "--interval must be a positive number of seconds");
    let interval = Duration::from_secs_f64(args.interval);

    let mut terminal = ratatui::try_init().context("failed to initialize the terminal")?;
    let result = event_loop(&mut terminal, interval);
    ratatui::restore(); // always restore — even if the loop errored (a panic is handled by the hook)
    result
}

/// Draw the current snapshot, then wait up to `interval` for input: a key is handled at once; on
/// timeout (no input) the snapshot is re-read. So the view refreshes every `interval`, but responds to
/// keys immediately rather than on the next tick.
fn event_loop(terminal: &mut ratatui::DefaultTerminal, interval: Duration) -> anyhow::Result<()> {
    let mut snapshot = Snapshot::read();
    loop {
        terminal.draw(|frame| render(frame, &snapshot))?;
        if event::poll(interval)? {
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('r') => snapshot = Snapshot::read(), // manual refresh
                    _ => {}
                }
            }
            // Non-key events (resize, focus, …) just fall through to a redraw next iteration.
        } else {
            snapshot = Snapshot::read(); // tick: nothing pressed within the interval
        }
    }
    Ok(())
}

/// Render one frame from `snap`. Pure (state in, frame out) so it's testable with `TestBackend`.
fn render(frame: &mut Frame, snap: &Snapshot) {
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
            .header(Row::new(["command", "repo", "model", "age", "turns", "tools", "chars"]).style(Style::new().bold()))
            .block(Block::bordered().title(format!("{} active run(s)", snap.active.len())));
        frame.render_widget(table, body);
    }

    let mut hints = String::from("q quit · r refresh");
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

    /// Render a snapshot to an in-memory backend and return the screen as one string.
    fn screen(snap: &Snapshot, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, snap)).unwrap();
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
        let text = screen(&snap, 60, 8);
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
        let text = screen(&snap, 100, 8);
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
        let text = screen(&snap, 80, 8);
        assert!(text.contains("unreadable"));
        assert!(text.contains("permission denied"));
    }
}
