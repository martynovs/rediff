//! Terminal lifecycle and the input/render event loop: setup/restore, the
//! main loop, polling, event dispatch, and mouse handling.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, KeyModifiers,
    KeyboardEnhancementFlags, MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;

use super::keys::handle_key;
use crate::git::{self, LoadRequest};
use crate::model::LayoutMode;
use crate::tui::app::{App, InputContext};
use crate::tui::theme::ThemeName;
use crate::tui::ui;
use crate::tui::view::ViewKind;

/// Run the interactive review, restoring the terminal on exit. The changed-file
/// list is enumerated synchronously (instant) and the per-file diffs stream in
/// on a background pool. `repo_dir` + `kind`/`review`/`base` describe the launch
/// view so the app can load other commits at runtime.
#[expect(
    clippy::too_many_arguments,
    reason = "launch parameters describe the initial review view; grouping them into a struct would not improve clarity"
)]
pub fn run(
    req: &LoadRequest,
    filters: &[String],
    mode: Option<LayoutMode>,
    theme: ThemeName,
    repo_dir: PathBuf,
    kind: ViewKind,
    review: bool,
    base: Option<String>,
) -> anyhow::Result<()> {
    // Enumerate before touching the terminal so an error surfaces normally.
    let en = git::enumerate(&repo_dir, req)?;
    let mut stubs = en.stubs;
    git::apply_stub_filter(&mut stubs, filters);
    let cs = crate::model::Changeset {
        source: en.source.clone(),
        files: stubs
            .iter()
            .map(crate::git::FileStub::as_stub_file)
            .collect(),
    };
    // No explicit mode → stack by default; `m` toggles to split.
    let mode = mode.unwrap_or(LayoutMode::Stack);
    let app_repo_dir = repo_dir.clone();
    let mut app = App::with_launch(
        &cs,
        mode,
        theme,
        Some(repo_dir),
        kind,
        review,
        base,
        Some(req.clone()),
    );
    // The log belongs to the worktree root, not the invocation directory:
    // `rediff` run inside `src/` would otherwise write `src/rediff.jsonl`, which
    // the loader does not filter out. A repository without a worktree simply has
    // no log, and commenting is inert.
    let log = gix::discover(&app_repo_dir)
        .ok()
        .and_then(|repo| crate::review::log_path(&repo))
        .map(crate::review::Log::new);
    app.attach_review_log(log, !filters.is_empty());
    app.begin_load(stubs, false);

    let mut terminal = setup_terminal()?;
    let result = event_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal);
    result
}

/// Enter raw mode + the alternate screen and build the terminal. Best-effort
/// requests the kitty keyboard protocol so modified keys like Shift+Space are
/// reported distinctly; terminals without support ignore it.
fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    #[expect(
        clippy::let_underscore_must_use,
        reason = "best-effort: terminals without kitty keyboard support ignore this and we proceed regardless"
    )]
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

/// Restore the terminal to its pre-launch state (the inverse of `setup_terminal`).
///
/// Best-effort and infallible: every step runs regardless of earlier failures,
/// so one failing cleanup can't leave the terminal stuck in raw mode or the
/// alternate screen. A teardown error on exit isn't actionable, so it's ignored
/// rather than propagated (which would also skip the remaining steps).
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
    #[expect(
        clippy::let_underscore_must_use,
        reason = "best-effort terminal teardown; a failing step must not skip the rest or mask the result"
    )]
    let _ = disable_raw_mode();
    #[expect(
        clippy::let_underscore_must_use,
        reason = "best-effort terminal teardown; a failing step must not skip the rest or mask the result"
    )]
    let _ = execute!(
        terminal.backend_mut(),
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    #[expect(
        clippy::let_underscore_must_use,
        reason = "best-effort terminal teardown; a failing step must not skip the rest or mask the result"
    )]
    let _ = terminal.show_cursor();
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    let mut dirty = true;
    while !app.should_quit {
        redraw_if_dirty(terminal, app, &mut dirty)?;

        // Kick off highlighting for what's on screen; it lands asynchronously.
        app.request_visible();
        app.request_peek_highlight();

        // Install anything the background workers finished (diffs, blame,
        // highlights) — one call, so the loop needn't know each job by name.
        dirty |= app.drain_background();

        if let Some(ev) = read_event(poll_timeout(app))? {
            dirty |= dispatch_event(app, &ev);
        }
    }
    Ok(())
}

/// Redraw the frame when `dirty`, then clear the flag. Keeps the draw call (and
/// its `?`) out of the loop body so the loop's own branch count stays minimal.
fn redraw_if_dirty(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    dirty: &mut bool,
) -> anyhow::Result<()> {
    if *dirty {
        terminal.draw(|f| ui::draw(f, app))?;
        *dirty = false;
    }
    Ok(())
}

/// Block up to `timeout` for the next terminal event, returning `None` on idle.
fn read_event(timeout: Duration) -> anyhow::Result<Option<Event>> {
    if event::poll(timeout)? {
        Ok(Some(event::read()?))
    } else {
        Ok(None)
    }
}

/// Poll briefly while a background job that drives progress is active (a diff
/// load streaming, a blame computing) so it paints promptly; idle otherwise so
/// async highlights still redraw.
fn poll_timeout(app: &App) -> Duration {
    if app.background_active() {
        Duration::from_millis(16)
    } else {
        Duration::from_millis(100)
    }
}

/// Apply one input event, returning whether the frame needs a redraw. Splitting
/// this out of the loop makes the (otherwise terminal-bound) dispatch testable
/// with synthetic events.
fn dispatch_event(app: &mut App, ev: &Event) -> bool {
    match ev {
        // Press and Repeat both act (so holding a key still scrolls); Release is
        // ignored. Repeat only arrives under the enhanced keyboard protocol.
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            handle_key(app, key.code, key.modifiers);
            true
        }
        // Mouse routing follows the same [`App::active_context`] precedence as
        // keys, so no modal can leak wheel/clicks to the view beneath it. The
        // body-scrolling contexts share one wheel decode (± a 3-row step); clicks
        // over them are absorbed.
        Event::Mouse(m) => {
            let wheel = match m.kind {
                MouseEventKind::ScrollDown => Some(3isize),
                MouseEventKind::ScrollUp => Some(-3isize),
                _ => None,
            };
            match app.active_context() {
                // The commit-message popup scrolls its body; clicks are absorbed.
                InputContext::CommitMsg => {
                    if let Some(d) = wheel {
                        app.commit_msg_scroll(d);
                    }
                    true
                }
                // Composing a comment absorbs the mouse entirely: there is
                // nothing to scroll and a click must not reach the diff behind.
                InputContext::Comment => true,
                // While the peek is open, the wheel scrolls it; clicks are ignored.
                InputContext::Peek => {
                    if let Some(d) = wheel {
                        app.peek_scroll(d);
                    }
                    true
                }
                // The remaining overlays absorb the mouse so the wheel/click cannot
                // scroll or select within the diff behind them.
                InputContext::Palette | InputContext::Help | InputContext::ThemePicker => false,
                InputContext::Normal => {
                    apply_mouse(app, *m);
                    true
                }
            }
        }
        Event::Resize(_, _) => true,
        _ => false,
    }
}

/// Mouse handling for the normal view: Ctrl+wheel moves through files, plain
/// wheel scrolls the file list (no selection change) over the panel or the diff
/// stream otherwise, and a click selects.
fn apply_mouse(app: &mut App, m: MouseEvent) {
    let sb = app.sidebar_area;
    let over_sidebar = m.column >= sb.x && m.column < sb.x + sb.width;
    let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);
    match m.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let dir = if matches!(m.kind, MouseEventKind::ScrollDown) {
                1
            } else {
                -1
            };
            if ctrl {
                app.sidebar_move(dir);
            } else if over_sidebar {
                app.sidebar_scroll(dir);
            } else {
                app.scroll_view(dir * 3);
            }
        }
        MouseEventKind::Down(_) => {
            app.click(m.column, m.row);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::compute_hunks;
    use crate::model::{Changeset, DiffFile, FileStatus, Stats};
    use crate::tui::app::Focus;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;
    use std::fmt::Write as _;

    fn file(path: &str, old: &str, new: &str, status: FileStatus) -> DiffFile {
        let (hunks, additions, deletions) = compute_hunks(old, new);
        DiffFile {
            path: path.into(),
            previous_path: None,
            status,
            staged: false,
            hunks,
            stats: Stats {
                additions,
                deletions,
            },
            language: None,
            is_binary: false,
            old_text: (!old.is_empty()).then(|| old.to_string()),
            new_text: (!new.is_empty()).then(|| new.to_string()),
            content_digest: None,
            diffed: true,
        }
    }

    fn sample() -> Changeset {
        Changeset {
            source: "working tree".into(),
            files: vec![
                file(
                    "src/auth.rs",
                    "fn login() {\n    ok()\n}\n",
                    "fn login() {\n    check()\n    ok()\n}\n",
                    FileStatus::Modified,
                ),
                file("README.md", "", "hello\n", FileStatus::Untracked),
            ],
        }
    }

    /// A changeset big enough that the stream actually scrolls.
    fn big_sample() -> Changeset {
        let files = (0..5)
            .map(|i| {
                let mut old = String::new();
                for n in 0..25 {
                    writeln!(old, "line {n}").unwrap();
                }
                let mut new = String::new();
                for n in 0..25 {
                    writeln!(new, "line {n}{}", if n == 5 { " X" } else { "" }).unwrap();
                }
                file(&format!("src/file{i}.rs"), &old, &new, FileStatus::Modified)
            })
            .collect();
        Changeset {
            source: "wt".into(),
            files,
        }
    }

    // ---- event dispatch / mouse / poll-timeout -----------------------------

    #[test]
    fn dispatch_event_routes_keys_mouse_and_resize() {
        let cs = big_sample();
        let mut app = App::new(&cs);
        let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();

        // A key Press acts and asks for a redraw.
        let press = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        ));
        assert!(dispatch_event(&mut app, &press), "a key press redraws");
        assert_eq!(app.state().cursor_row, 1, "j moved the line cursor");

        // A key Release is ignored: no redraw, no state change.
        let mut rel =
            ratatui::crossterm::event::KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        rel.kind = KeyEventKind::Release;
        assert!(
            !dispatch_event(&mut app, &Event::Key(rel)),
            "a key release is ignored"
        );
        assert_eq!(app.state().cursor_row, 1, "release did not move the cursor");

        // A resize always redraws.
        assert!(
            dispatch_event(&mut app, &Event::Resize(100, 40)),
            "resize redraws"
        );

        // A plain wheel over the stream scrolls the diff.
        let before = app.state().scroll;
        let wheel = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 60,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        assert!(dispatch_event(&mut app, &wheel), "a wheel event redraws");
        assert!(app.state().scroll > before, "the wheel scrolled the stream");

        // An unhandled event (e.g. focus gained) is ignored.
        assert!(
            !dispatch_event(&mut app, &Event::FocusGained),
            "an unhandled event needs no redraw"
        );
    }

    #[test]
    fn dispatch_event_overlay_mouse_capture() {
        use ratatui::crossterm::event::MouseButton;
        let mut content = String::new();
        for i in 0..100 {
            writeln!(content, "line {i}").unwrap();
        }
        let cs = Changeset {
            source: "wt".into(),
            files: vec![file("a.rs", "x\n", &content, FileStatus::Modified)],
        };
        let mut app = App::new(&cs);

        // Peek open: the wheel scrolls the peek; clicks are swallowed but consume.
        app.open_peek_preview();
        let mut term = Terminal::new(TestBackend::new(70, 20)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap(); // sets peek_viewport_h
        let down = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        assert!(dispatch_event(&mut app, &down));
        assert_eq!(
            app.peek().unwrap().state.scroll,
            3,
            "the wheel scrolls the peek by 3"
        );
        let up = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        assert!(dispatch_event(&mut app, &up));
        assert_eq!(app.peek().unwrap().state.scroll, 0, "wheel-up scrolls back");
        let click = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        assert!(
            dispatch_event(&mut app, &click),
            "a click while peeking still consumes the event"
        );
        assert_eq!(app.peek().unwrap().state.scroll, 0, "the click did nothing");
        app.peek_close();

        // Palette open: the mouse is absorbed (no redraw, no effect on the diff).
        app.open_palette();
        assert!(
            !dispatch_event(&mut app, &down),
            "the palette absorbs the mouse"
        );
        assert!(app.palette_open(), "the palette stays open");
        app.palette_close();

        // Help open: the mouse is absorbed too.
        handle_key(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(app.help_open());
        assert!(!dispatch_event(&mut app, &down), "help absorbs the mouse");
        assert!(app.help_open(), "help stays open");
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);

        // Theme picker open: the mouse is absorbed — a wheel/click must not
        // scroll or re-select the stream behind the modal.
        let scroll_before = app.state().scroll;
        app.open_theme_picker();
        assert!(!dispatch_event(&mut app, &down), "picker absorbs the mouse");
        assert_eq!(
            app.state().scroll,
            scroll_before,
            "nothing scrolled beneath the picker"
        );
        assert!(app.theme_picker_open(), "the picker stays open");
    }

    #[test]
    fn commit_msg_popup_takes_the_mouse() {
        use crate::model::CommitMessage;
        use crate::tui::app::{CommitMsg, Overlay};
        use ratatui::crossterm::event::MouseButton;
        let cs = big_sample();
        let mut app = App::new(&cs);
        let body = (1..=30)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.mode
            .push_overlay(Overlay::CommitMessage(CommitMsg::new(CommitMessage {
                sha: "s".into(),
                short: "s".into(),
                author: String::new(),
                date: String::new(),
                body,
            })));
        let stream_before = app.state().scroll;
        let down = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        assert!(
            dispatch_event(&mut app, &down),
            "the popup consumes + redraws"
        );
        assert_eq!(
            app.commit_msg().unwrap().scroll,
            3,
            "the wheel scrolls the popup body"
        );
        assert_eq!(
            app.state().scroll,
            stream_before,
            "the stream underneath did not move"
        );
        // A click is absorbed: nothing beneath the modal changes.
        let click = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        assert!(dispatch_event(&mut app, &click));
        assert_eq!(app.state().scroll, stream_before, "click hit nothing below");
        assert!(app.commit_msg_open(), "the popup stays open");
        // Wheel-up scrolls back.
        let up = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        assert!(dispatch_event(&mut app, &up));
        assert_eq!(app.commit_msg().unwrap().scroll, 0);
    }

    /// A long-file-list app, drawn once so `sidebar_area` is populated. Returns
    /// the app plus a pointer column inside the sidebar and inside the stream.
    fn mouse_app() -> (App, u16, u16, ratatui::layout::Rect) {
        let files: Vec<_> = (0..30)
            .map(|i| {
                let mut new = String::new();
                for n in 0..6 {
                    writeln!(new, "f{i} line {n}").unwrap();
                }
                file(&format!("f{i}.rs"), "a\n", &new, FileStatus::Modified)
            })
            .collect();
        let cs = Changeset {
            source: "wt".into(),
            files,
        };
        let mut app = App::new(&cs);
        let mut term = Terminal::new(TestBackend::new(60, 14)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let sb = app.sidebar_area;
        (app, sb.x + 1, sb.x + sb.width + 2, sb)
    }

    fn wheel(kind: MouseEventKind, column: u16, modifiers: KeyModifiers) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row: 4,
            modifiers,
        }
    }

    #[test]
    fn apply_mouse_wheel_branches() {
        let (mut app, in_sidebar, in_stream, _sb) = mouse_app();

        // Ctrl+wheel steps through files regardless of pointer location.
        apply_mouse(
            &mut app,
            wheel(MouseEventKind::ScrollDown, in_stream, KeyModifiers::CONTROL),
        );
        assert_eq!(app.state().selected, 1, "ctrl+wheel → next file");
        apply_mouse(
            &mut app,
            wheel(MouseEventKind::ScrollUp, in_stream, KeyModifiers::CONTROL),
        );
        assert_eq!(app.state().selected, 0, "ctrl+wheel up → prev file");

        // A plain wheel over the sidebar scrolls the list, selection unchanged.
        let sel_before = app.state().selected;
        let top_before = app.sidebar_top;
        apply_mouse(
            &mut app,
            wheel(MouseEventKind::ScrollDown, in_sidebar, KeyModifiers::NONE),
        );
        assert_eq!(
            app.state().selected,
            sel_before,
            "a sidebar wheel keeps the selection"
        );
        assert!(
            app.sidebar_top > top_before,
            "a sidebar wheel scrolled the file list"
        );

        // A plain wheel over the stream scrolls the diff.
        let scroll_before = app.state().scroll;
        apply_mouse(
            &mut app,
            wheel(MouseEventKind::ScrollDown, in_stream, KeyModifiers::NONE),
        );
        assert!(
            app.state().scroll > scroll_before,
            "a stream wheel scrolled the diff"
        );
    }

    #[test]
    fn apply_mouse_click_and_inert_branches() {
        use ratatui::crossterm::event::MouseButton;
        let (mut app, in_sidebar, in_stream, sb) = mouse_app();

        // A click in the sidebar focuses the file list.
        apply_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: in_sidebar,
                row: sb.y + 2,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(
            app.focus(),
            Focus::Sidebar,
            "a sidebar click focuses the list"
        );

        // A wheel of some other kind (e.g. a moved pointer) is inert.
        let snap = (app.state().selected, app.state().scroll, app.sidebar_top);
        apply_mouse(
            &mut app,
            wheel(MouseEventKind::Moved, in_stream, KeyModifiers::NONE),
        );
        assert_eq!(
            (app.state().selected, app.state().scroll, app.sidebar_top),
            snap,
            "a moved-pointer event changes nothing"
        );
    }

    #[test]
    fn poll_timeout_reflects_loading_state() {
        let cs = sample();
        let mut app = App::new(&cs);
        // Idle: the longer poll so async highlights still repaint.
        assert_eq!(poll_timeout(&app), Duration::from_millis(100));
        // Loading: the brief poll so streaming progress paints promptly. A
        // zero-job loader needs no repo and spawns no workers, yet `loading()`
        // reports true (the loader handle is present).
        app.session.loader = Some(crate::tui::loader::Loader::start(
            std::path::PathBuf::new(),
            Vec::new(),
        ));
        assert!(app.loading(), "a present loader means loading");
        assert_eq!(poll_timeout(&app), Duration::from_millis(16));
    }
}
#[cfg(test)]
mod comment_mouse_tests {
    use super::*;
    use crate::tui::app::App;
    use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};

    /// The comment input must absorb the mouse entirely: a click behind a modal
    /// must not select or refocus, and the wheel must not scroll anything.
    #[test]
    fn the_comment_overlay_absorbs_the_mouse() {
        // Big enough that the viewport can actually scroll — with a plan
        // shorter than the viewport, `max_scroll` is 0 and a leaked wheel event
        // moves nothing, so the test would pass either way.
        let files: Vec<_> = (0..12)
            .map(|i| {
                let body = "a line of content\n".repeat(12);
                crate::testutil::diff_file(&format!("f{i}.rs"), Some(&body))
            })
            .collect();
        let cs = crate::testutil::changeset(files);
        let mut app = App::new(&cs);
        app.viewport_h = 12;
        app.sidebar_area = ratatui::layout::Rect::new(0, 0, 30, 20);
        super::super::keys::handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);

        // Everything a leak could plausibly move.
        let snap = |a: &App| {
            (
                a.state().scroll,
                a.state().cursor_row,
                a.state().selected,
                a.sidebar_top,
                a.focus(),
            )
        };
        let before = snap(&app);

        app.mode.push_overlay(crate::tui::app::Overlay::Comment(
            crate::tui::app::CommentInput::new(None),
        ));

        // Wheel over the stream (past the sidebar), wheel over the sidebar, and
        // a click in each — the three routes `apply_mouse` would take.
        for (kind, col) in [
            (MouseEventKind::ScrollDown, 40),
            (MouseEventKind::ScrollUp, 40),
            (MouseEventKind::ScrollDown, 2),
            (MouseEventKind::Down(MouseButton::Left), 2),
            (MouseEventKind::Down(MouseButton::Left), 40),
        ] {
            let ev = Event::Mouse(MouseEvent {
                kind,
                column: col,
                row: 4,
                modifiers: KeyModifiers::NONE,
            });
            assert!(dispatch_event(&mut app, &ev), "the event is consumed");
            assert_eq!(
                snap(&app),
                before,
                "nothing behind the modal moved ({kind:?} at {col})"
            );
        }
    }
}
