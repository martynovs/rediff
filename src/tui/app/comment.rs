//! Composing review comments: opening the input, editing it, and recording a
//! thread on confirmation.
//!
//! The review is opened **on confirmation, not on opening the input** — so
//! summoning the input and pressing Esc leaves no log behind, which is what
//! "browsing writes nothing" has to mean in practice.

use crate::review::{now, Log, Record, Thread};
use crate::tui::app::types::{App, CommentInput, Overlay};
use crate::tui::reviewlog::{anchor_at, ensure_review};

impl App {
    /// The comment being composed, if the input is open.
    pub fn comment_input_state(&self) -> Option<&CommentInput> {
        match self.mode.overlay() {
            Some(Overlay::Comment(c)) => Some(c),
            _ => None,
        }
    }

    fn comment_input_mut(&mut self) -> Option<&mut CommentInput> {
        match self.mode.overlay_mut() {
            Some(Overlay::Comment(c)) => Some(c),
            _ => None,
        }
    }

    /// Whether this view can take review points at all.
    ///
    /// `Session::is_review()` is the predicate, not a list of view kinds:
    /// `rediff review <rev>` is a `ViewKind::Commit` view that *is* a review
    /// session, and `R` promotes a browse view.
    fn can_comment(&self) -> bool {
        self.is_review() && self.review_log.is_some()
    }

    /// Open the comment input on the line under the cursor (`anchored`) or on the
    /// review as a whole.
    ///
    /// No log I/O happens here. Anchoring is resolved now so an impossible
    /// comment is refused before the user types it.
    pub fn open_comment(&mut self, anchored: bool) {
        if !self.can_comment() {
            return;
        }
        let anchor = if anchored {
            let row = self.state().cursor_row;
            match anchor_at(self.cs().as_ref(), self.plan(), row) {
                Ok(a) => Some(a),
                Err(e) => {
                    self.flash = Some(e.message().to_string());
                    return;
                }
            }
        } else {
            None
        };
        self.mode
            .push_overlay(Overlay::Comment(CommentInput::new(anchor)));
    }

    pub fn comment_input(&mut self, c: char) {
        if let Some(input) = self.comment_input_mut() {
            input.buffer.push(c);
            input.refusal = None;
        }
    }

    pub fn comment_backspace(&mut self) {
        if let Some(input) = self.comment_input_mut() {
            input.buffer.pop();
            input.refusal = None;
        }
    }

    /// Discard the comment, recording nothing.
    pub fn comment_cancel(&mut self) {
        self.mode.pop_overlay();
    }

    /// Record the comment as a thread, opening or attaching to the review first.
    ///
    /// An empty body is treated as a cancel: `Enter` on an untouched input should
    /// not leave a blank review point behind.
    pub fn comment_confirm(&mut self) {
        let Some(input) = self.comment_input_state() else {
            return;
        };
        if input.buffer.trim().is_empty() {
            self.comment_cancel();
            return;
        }
        let (anchor, body, replacing) = (
            input.anchor.clone(),
            input.buffer.clone(),
            input.replacing.clone(),
        );

        if let Err(msg) = self.record_thread(anchor, body, replacing.as_deref()) {
            // Keep the input open so the typed text is not lost, and report
            // *inside* the box — `draw_status` hides the flash while an overlay
            // is up, so a refusal sent there would never be seen.
            if let Some(input) = self.comment_input_mut() {
                input.refusal = Some(msg);
            }
            return;
        }
        self.mode.pop_overlay();
    }

    /// Open/attach the review and append the thread. `Err` carries a message.
    fn record_thread(
        &mut self,
        anchor: Option<crate::review::Anchor>,
        body: String,
        replacing: Option<&str>,
    ) -> Result<(), String> {
        let target = self.review_target().ok_or_else(|| {
            "this view has no target a review can be recorded against".to_string()
        })?;
        let log: &Log = self
            .review_log
            .as_ref()
            .ok_or_else(|| "no review log for this repository".to_string())?;

        let opening = ensure_review(log, self.cs().as_ref(), &target, self.launch_filtered)
            .map_err(|e| e.message())?;

        // A *thread* id, not a review id: `new_review_id` seeds on
        // `(pid, seconds)` alone, so two comments in the same second would share
        // one and `fold` would silently drop the earlier.
        let id = replacing.map_or_else(crate::tui::reviewlog::new_thread_id, ToString::to_string);
        log.append(&Record::Thread(Thread {
            id,
            anchor,
            body,
            // NOT `replacing`: superseding is by reusing the `id`. `replace` is
            // replacement *text* the agent may apply verbatim, so putting a
            // thread id there would hand the agent an id as source code.
            replace: None,
            resolved: false,
            deleted: false,
            at: now(),
        }))
        .map_err(|e| format!("could not write the review log: {e}"))?;

        // The ignore hint rides on a *fresh* review — the only signal the store
        // gives that a log has just appeared in the worktree.
        if opening.opened == crate::review::Opened::Fresh {
            self.flash = Some(
                "review points are written to rediff.jsonl — add it to .gitignore if you don't want it tracked"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// The encoded review target for the current view, from its `LoadRequest`.
    fn review_target(&self) -> Option<String> {
        let req = self.session.views.get(self.session.cursor)?.req.as_ref()?;
        crate::reviewcli::encode(req).ok()
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Changeset, DiffFile, FileStatus, Hunk, LayoutMode, Line, LineKind, Stats};
    use crate::review::Log;
    use crate::tui::app::App;
    use crate::tui::runtime::handle_key_for_test as handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    fn cs_one_file() -> Changeset {
        Changeset {
            source: "wt".into(),
            files: vec![DiffFile {
                path: "a.rs".into(),
                previous_path: None,
                status: FileStatus::Modified,
                staged: false,
                hunks: vec![Hunk {
                    old_start: 1,
                    old_len: 1,
                    new_start: 1,
                    new_len: 1,
                    lines: vec![Line {
                        kind: LineKind::Added,
                        old_lineno: None,
                        new_lineno: Some(1),
                        text: "let x = 1;".into(),
                        emphasis: None,
                    }],
                }],
                stats: Stats {
                    additions: 1,
                    deletions: 0,
                },
                language: None,
                is_binary: false,
                old_text: None,
                new_text: Some("let x = 1;\n".into()),
                content_digest: None,
                diffed: true,
            }],
        }
    }

    /// A review-session app with a log in a scratch dir, cursor on the diff line.
    fn app_with_log() -> (tempfile::TempDir, App) {
        let cs = cs_one_file();
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.path().to_path_buf()),
            crate::tui::ViewKind::Local,
            true,
            None,
            Some(crate::git::LoadRequest::WorkingTree {
                include_untracked: true,
                base: None,
            }),
        );
        app.viewport_h = 12;
        app.attach_review_log(Some(Log::at_worktree(dir.path())), false);
        app.move_cursor(1); // onto the added line
        (dir, app)
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            handle_key(app, KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    #[test]
    fn a_typed_comment_is_recorded_as_an_anchored_thread() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(app.comment_input_state().is_some(), "the input opened");
        type_str(&mut app, "this looks wrong");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.comment_input_state().is_none(), "and closed on save");

        let st = Log::at_worktree(dir.path()).state().unwrap();
        assert_eq!(st.threads.len(), 1);
        let t = st.threads.values().next().unwrap();
        assert_eq!(t.thread.body, "this looks wrong");
        let a = t.thread.anchor.as_ref().expect("anchored to the line");
        assert_eq!((a.path.as_str(), a.line), ("a.rs", 1));
        assert_eq!(a.quote, "let x = 1;", "the line's own text is captured");
        assert_eq!(
            st.open.unwrap().target,
            "worktree",
            "target encoded from req"
        );
    }

    #[test]
    fn browsing_and_discarding_write_nothing() {
        let (dir, mut app) = app_with_log();
        let log = Log::at_worktree(dir.path());
        // Merely opening the app wrote nothing.
        assert!(!log.path().exists());
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "never mind");
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.comment_input_state().is_none());
        assert!(!log.path().exists(), "a discarded comment opens no review");
    }

    #[test]
    fn an_empty_comment_is_a_cancel() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.comment_input_state().is_none());
        assert!(
            !Log::at_worktree(dir.path()).path().exists(),
            "Enter on an untouched input leaves no blank review point"
        );
    }

    #[test]
    fn two_comments_in_the_same_second_both_survive() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "first comment");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "second comment");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        // `fold` keys threads by id, so a shared id silently supersedes: the
        // bytes stay on disk and every consumer reads the fold.
        let st = Log::at_worktree(dir.path()).state().unwrap();
        assert_eq!(st.threads.len(), 2, "both comments survive the fold");
        let bodies: std::collections::HashSet<&str> = st
            .threads
            .values()
            .map(|t| t.thread.body.as_str())
            .collect();
        assert!(bodies.contains("first comment") && bodies.contains("second comment"));
    }

    #[test]
    fn a_review_level_comment_carries_no_anchor() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "looks good overall");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let st = Log::at_worktree(dir.path()).state().unwrap();
        let t = st.threads.values().next().unwrap();
        assert!(t.thread.anchor.is_none(), "the verdict has no anchor");
    }

    #[test]
    fn backspace_edits_the_buffer_and_typing_does_not_reach_the_diff() {
        let (_d, mut app) = app_with_log();
        let before = app.state().cursor_row;
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "abc");
        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(app.comment_input_state().unwrap().buffer, "ab");
        // `j` is a motion key in the base context; here it is just text.
        type_str(&mut app, "j");
        assert_eq!(app.comment_input_state().unwrap().buffer, "abj");
        assert_eq!(app.state().cursor_row, before, "the diff did not move");
    }

    #[test]
    fn commenting_on_a_row_that_is_not_a_line_says_so_and_opens_nothing() {
        let (dir, mut app) = app_with_log();
        app.top(); // the file header
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(app.comment_input_state().is_none(), "no input opened");
        assert!(app.flash.is_some(), "and the user is told why");
        assert!(!Log::at_worktree(dir.path()).path().exists());
    }

    #[test]
    fn a_filtered_view_refuses_on_confirm_and_writes_nothing() {
        let (dir, mut app) = app_with_log();
        app.attach_review_log(Some(Log::at_worktree(dir.path())), true);
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "something");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            app.comment_input_state().is_some(),
            "the input stays open so the typed text is not lost"
        );
        // Reported *in the box*: `draw_status` hides the flash under an overlay,
        // so a refusal on the flash would never reach a frame.
        let refusal = app.comment_input_state().unwrap().refusal.as_deref();
        assert!(refusal.unwrap().contains("whole target"), "{refusal:?}");
        assert!(!Log::at_worktree(dir.path()).path().exists());
    }

    #[test]
    fn the_title_names_the_anchor_and_whether_it_is_an_edit() {
        use crate::review::{Anchor, Side};
        let a = Anchor {
            path: "src/lib.rs".into(),
            side: Side::New,
            line: 42,
            quote: "x".into(),
            before: Vec::new(),
            after: Vec::new(),
        };
        let mut c = crate::tui::app::CommentInput::new(Some(a));
        assert_eq!(c.title(), "comment on src/lib.rs:42");
        c.replacing = Some("t1".into());
        assert_eq!(c.title(), "edit comment on src/lib.rs:42");

        let mut r = crate::tui::app::CommentInput::new(None);
        assert_eq!(r.title(), "comment on this review");
        r.replacing = Some("t1".into());
        assert_eq!(r.title(), "edit review comment");
    }

    #[test]
    fn edit_and_input_keys_are_inert_with_no_input_open() {
        // The `_ => None` arm: these are reachable only via the router while the
        // overlay is up, so they are exercised directly.
        let (_d, mut app) = app_with_log();
        assert!(app.comment_input_state().is_none());
        app.comment_input('x');
        app.comment_backspace();
        app.comment_confirm();
        assert!(app.comment_input_state().is_none(), "still nothing open");
    }

    #[test]
    fn an_unhandled_key_while_composing_does_nothing() {
        let (_d, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "hi");
        handle_key(&mut app, KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(app.comment_input_state().unwrap().buffer, "hi");
    }

    #[test]
    fn an_io_failure_becomes_a_message_rather_than_a_panic() {
        use crate::tui::reviewlog::EnsureError;
        let e: EnsureError = std::io::Error::other("nope").into();
        assert!(e.message().contains("nope"));
    }

    #[test]
    fn a_refusal_is_shown_inside_the_box_where_it_can_be_seen() {
        // `draw_status` suppresses the flash whenever an overlay is up, so a
        // refusal reported there is invisible at exactly the moment the input
        // stays open to report it.
        let (dir, mut app) = app_with_log();
        app.attach_review_log(Some(Log::at_worktree(dir.path())), true);
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "x");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let input = app.comment_input_state().expect("stays open");
        assert!(input.refusal.is_some(), "the refusal rides on the input");
        // ...and editing clears it, so a stale reason does not linger.
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(app.comment_input_state().unwrap().refusal.is_none());
    }

    #[test]
    fn a_second_comment_does_not_reopen_the_review() {
        // `fold` resets the entire state on an `Open`, so appending one per
        // comment restarts the round counter and drops earlier threads from the
        // fold. Asserted on the raw file, because the fold hides the damage.
        let (dir, mut app) = app_with_log();
        for body in ["one", "two"] {
            handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
            type_str(&mut app, body);
            handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        }
        let raw = std::fs::read_to_string(Log::at_worktree(dir.path()).path()).unwrap();
        let opens = raw.lines().filter(|l| l.contains(r#""t":"open""#)).count();
        assert_eq!(opens, 1, "attaching appends no second `open`");
    }

    #[test]
    fn joining_an_agent_opened_review_keeps_its_records() {
        // `keep: true`'s whole purpose. With `keep: false` the log is truncated
        // and the agent's review — label and all — is destroyed.
        let (dir, mut app) = app_with_log();
        let log = Log::at_worktree(dir.path());
        log.append(&crate::review::Record::Open {
            review: "agent1".into(),
            target: "worktree".into(),
            label: Some("please review".into()),
            at: crate::review::now(),
        })
        .unwrap();

        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "joining in");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let st = log.state().unwrap();
        let open = st.open.expect("a review is open");
        assert_eq!(open.review, "agent1", "joined the agent's review");
        assert_eq!(open.label.as_deref(), Some("please review"), "label intact");
        assert_eq!(st.threads.len(), 1);
    }

    #[test]
    fn the_ignore_hint_fires_only_on_a_fresh_review() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "one");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            app.flash
                .as_deref()
                .unwrap_or_default()
                .contains("gitignore"),
            "the first comment says where the log went"
        );
        app.flash = None;
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "two");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.flash.is_none(), "and does not repeat it");
        let _ = dir;
    }

    #[test]
    fn commenting_works_with_the_sidebar_focused() {
        // They used to live in `handle_stream_key`, where they were silently
        // dead after `Tab` — no comment, no message, and the help overlay
        // advertising them unconditionally.
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(app.focus(), crate::tui::app::Focus::Sidebar);

        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(
            app.comment_input_state().is_some(),
            "`a` works from the sidebar"
        );
        type_str(&mut app, "from the list");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let st = Log::at_worktree(dir.path()).state().unwrap();
        assert_eq!(st.threads.len(), 1);
    }

    #[test]
    fn a_ctrl_chord_is_not_typed_into_the_comment() {
        let (_d, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "hi");
        handle_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        handle_key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(
            app.comment_input_state().unwrap().buffer,
            "hi",
            "Ctrl chords are not text in the one buffer that gets persisted"
        );
    }

    #[test]
    fn a_browse_view_takes_no_comments() {
        let cs = cs_one_file();
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.path().to_path_buf()),
            crate::tui::ViewKind::Commit("abc".into()),
            false, // not a review session
            None,
            Some(crate::git::LoadRequest::Show { rev: "abc".into() }),
        );
        app.attach_review_log(Some(Log::at_worktree(dir.path())), false);
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(app.comment_input_state().is_none(), "`a` is inert here");
    }
}
