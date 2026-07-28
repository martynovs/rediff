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
        self.is_review()
    }

    /// The worktree's log, or a message explaining its absence.
    fn log_or_flash(&mut self) -> Option<&crate::review::Log> {
        if self.review_log.is_none() {
            self.flash = Some("this repository has no worktree to hold a review log".to_string());
        }
        self.review_log.as_ref()
    }

    /// Open the comment input on the line under the cursor (`anchored`) or on the
    /// review as a whole.
    ///
    /// No log I/O happens here. Anchoring is resolved now so an impossible
    /// comment is refused before the user types it.
    pub fn open_comment(&mut self, anchored: bool) {
        if !self.can_comment() || self.log_or_flash().is_none() {
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

        let result = match replacing {
            // An edit supersedes an existing thread, so it neither opens a
            // review nor opens a round: the review is open by construction, and
            // rewording a comment is not new content to hash.
            Some(id) => self.edit_confirm(&id, body),
            None => self.record_thread(anchor, body),
        };
        if let Err(msg) = result {
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

    /// Supersede an existing thread's text, keeping every other field.
    fn edit_confirm(&mut self, id: &str, body: String) -> Result<(), String> {
        let log: &Log = self
            .review_log
            .as_ref()
            .ok_or_else(|| "no review log for this repository".to_string())?;
        let found = crate::tui::reviewlog::supersede(log, id, |t| t.body = body)
            .map_err(|e| format!("could not write the review log: {e}"))?;
        if !found {
            return Err("that review point is no longer in the log".to_string());
        }
        self.refresh_threads();
        Ok(())
    }

    /// Open/attach the review and append a new thread. `Err` carries a message.
    fn record_thread(
        &mut self,
        anchor: Option<crate::review::Anchor>,
        body: String,
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
        log.append(&Record::Thread(Thread {
            id: crate::tui::reviewlog::new_thread_id(),
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

        self.refresh_threads();
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

    /// The thread list, if it is open.
    pub fn thread_list(&self) -> Option<&crate::tui::app::types::ThreadList> {
        match self.mode.overlay() {
            Some(Overlay::Threads(l)) => Some(l),
            _ => None,
        }
    }

    /// Open the list of the review's live threads, resolved against this view.
    pub fn open_threads(&mut self) {
        if !self.can_comment() {
            return;
        }
        let Some(log) = self.log_or_flash() else {
            return;
        };
        let Ok(st) = log.state() else {
            self.flash = Some("could not read the review log".to_string());
            return;
        };
        let cs = self.cs().clone();
        let threads = crate::tui::reviewlog::live_threads(&st, &cs);
        if threads.is_empty() {
            self.flash = Some("no review points yet — `a` comments on a line".to_string());
            return;
        }
        self.mode
            .push_overlay(Overlay::Threads(crate::tui::app::types::ThreadList::new(
                threads,
            )));
    }

    pub fn threads_move(&mut self, delta: isize) {
        if let Some(Overlay::Threads(l)) = self.mode.overlay_mut() {
            l.move_by(delta);
        }
    }

    pub fn threads_close(&mut self) {
        self.mode.pop_overlay();
    }

    /// Reopen the input over the selected thread, pre-filled with its text.
    ///
    /// The anchor comes from the stored record rather than from the list's
    /// resolved position: superseding must not silently re-anchor a comment to
    /// wherever it happens to sit now.
    pub fn threads_edit(&mut self) {
        let Some(id) = self
            .thread_list()
            .and_then(|l| l.current())
            .map(|t| t.id.clone())
        else {
            return;
        };
        let Some(log) = self.review_log.as_ref() else {
            return;
        };
        let Ok(st) = log.state() else {
            self.flash = Some("could not read the review log".to_string());
            return;
        };
        let Some(t) = st.threads.get(&id) else {
            self.flash = Some("that review point is no longer in the log".to_string());
            return;
        };
        let mut input = CommentInput::new(t.thread.anchor.clone());
        input.buffer.clone_from(&t.thread.body);
        input.replacing = Some(id);
        // Pushed *over* the list, so Esc returns to it rather than to the diff.
        self.mode.push_overlay(Overlay::Comment(input));
    }

    /// Retract the selected thread, or restore it if it is already retracted.
    ///
    /// A toggle, not a one-way door: retracting withdraws a comment from every
    /// read path, so an unrepeatable keystroke would be a way to lose a review
    /// point with no way back from inside rediff. The list keeps retracted
    /// threads visible for exactly this reason.
    pub fn threads_retract(&mut self) {
        self.supersede_selected("retract", |t| t.deleted = !t.deleted);
    }

    /// Mark the selected thread resolved, or unresolved if it already is.
    pub fn threads_resolve(&mut self) {
        self.supersede_selected("resolve", |t| t.resolved = !t.resolved);
    }

    /// Apply `edit` to the selected thread and append the superseding record.
    fn supersede_selected(&mut self, what: &str, edit: impl FnOnce(&mut Thread)) {
        let Some(id) = self
            .thread_list()
            .and_then(|l| l.current())
            .map(|t| t.id.clone())
        else {
            return;
        };
        let Some(log) = self.review_log.as_ref() else {
            return;
        };
        match crate::tui::reviewlog::supersede(log, &id, edit) {
            Ok(true) => self.refresh_threads(),
            Ok(false) => self.flash = Some("that review point is no longer in the log".to_string()),
            Err(e) => self.flash = Some(format!("could not {what}: {e}")),
        }
    }

    /// Jump to the selected thread's line and close the list.
    ///
    /// Moves `cursor_row`, not just `scroll`: `stream::clamp` runs every frame
    /// and scrolls the viewport back to the cursor, so a scroll-only jump snaps
    /// straight back — the hazard `jump_to_collapsed` documents.
    pub fn threads_jump(&mut self) {
        // Resolved in one step, and matching `previous_path` as `review::resolve`
        // does: after a rename the anchor still carries the *old* path, so a
        // lookup on `f.path` alone would refuse to jump to a line that is right
        // there.
        let target = self.thread_list().and_then(|l| {
            let (path, side, line) = l.current()?.key.as_ref()?;
            let fi = self.cs().files.iter().position(|f| {
                f.path == *path || f.previous_path.as_deref() == Some(path.as_str())
            })?;
            Some((fi, *side, *line))
        });
        let Some(key) = target else {
            self.flash = Some("that comment's line is not in this diff".to_string());
            return;
        };
        let Some(row) = crate::tui::rows::find_key(self.plan(), key) else {
            self.flash = Some("that comment's line is not shown here".to_string());
            return;
        };
        self.threads_close();
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = crate::tui::stream::usable(&e.plan, vh);
        crate::tui::stream::scroll_to(&mut e.state, &e.plan, usable, row);
        e.state.cursor_row = row;
        crate::tui::stream::anchor_selected(&mut e.state, &e.plan);
    }

    /// Re-read the review's threads: the gutter index, and any open list.
    ///
    /// Called after *our own* appends only. Nothing watches the log, so there is
    /// no other moment at which it can change under us — and the open list has
    /// to be refreshed here too, since editing or retracting from it returns to
    /// a list that would otherwise still show what was just superseded.
    pub(crate) fn refresh_threads(&mut self) {
        let Some(log) = self.review_log.as_ref() else {
            return;
        };
        let Ok(st) = log.state() else { return };
        let cs = self.cs().clone();
        let live = crate::tui::reviewlog::live_threads(&st, &cs);
        self.thread_marks = crate::tui::reviewlog::thread_index(&live, &cs);
        // The selection is an index, and superseding never reorders or removes
        // a thread (`fold` keys by id, in first-appearance order), so the same
        // index still names the same thread. Clamped anyway.
        if let Some(Overlay::Threads(l)) = self.find_thread_list() {
            l.selected = l.selected.min(live.len().saturating_sub(1));
            l.threads = live;
        }
    }

    /// The thread list wherever it sits in the overlay stack.
    ///
    /// Not `overlay_mut`: editing pushes the comment input *over* the list, so
    /// after a confirming edit the list is no longer the topmost overlay.
    fn find_thread_list(&mut self) -> Option<&mut Overlay> {
        self.mode
            .overlays_mut()
            .iter_mut()
            .find(|o| matches!(o, Overlay::Threads(_)))
    }

    /// The encoded review target for the current view, from its `LoadRequest`.
    pub(crate) fn review_target(&self) -> Option<String> {
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
    fn a_comment_marks_its_line_in_the_gutter() {
        let (_d, mut app) = app_with_log();
        assert!(
            app.thread_marks.is_empty(),
            "nothing marked before commenting"
        );
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "look here");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        // Keyed on the cursor's own key type: file index, side, line.
        assert_eq!(
            app.thread_marks.get(&(0, crate::review::Side::New, 1)),
            Some(&1),
            "the commented line is marked: {:?}",
            app.thread_marks
        );
    }

    #[test]
    fn the_thread_list_opens_jumps_and_moves_the_cursor() {
        let (_d, mut app) = app_with_log();
        // Nothing recorded yet: the list says so rather than opening empty.
        handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(app.thread_list().is_none());
        assert!(app.flash.as_deref().unwrap().contains("no review points"));

        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "here");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let commented_row = app.state().cursor_row;

        // Move away, then jump back from the list.
        app.top();
        assert_ne!(app.state().cursor_row, commented_row);
        handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(app.thread_list().unwrap().threads.len(), 1);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        assert!(app.thread_list().is_none(), "the list closes on a jump");
        assert_eq!(
            app.state().cursor_row,
            commented_row,
            "the jump moves the cursor, not just the viewport — `clamp` would \
             scroll straight back otherwise"
        );
    }

    #[test]
    fn a_retracted_thread_is_not_listed_and_a_resolved_one_is_flagged() {
        let (dir, mut app) = app_with_log();
        let log = Log::at_worktree(dir.path());
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "keep me");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        // A retracted thread and a resolved one, written directly.
        for (id, deleted, resolved) in [("gone", true, false), ("done", false, true)] {
            log.append(&crate::review::Record::Thread(crate::review::Thread {
                id: id.into(),
                anchor: None,
                body: id.into(),
                replace: None,
                resolved,
                deleted,
                at: crate::review::now(),
            }))
            .unwrap();
        }

        handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        let list = app.thread_list().unwrap();
        let label = |body: &str| {
            list.threads
                .iter()
                .find(|t| t.body == body)
                .unwrap_or_else(|| panic!("{body} is listed"))
                .state_label()
        };
        // Retracted is listed, and says so — the list is the only place it can
        // be un-retracted from, so hiding it would make `x` a one-way door.
        assert_eq!(label("gone"), "retracted");
        assert_eq!(label("done"), "resolved");

        // ...but it carries no gutter mark: the margin marks lines that still
        // have something to say.
        app.threads_close();
        let marked: usize = app.thread_marks.values().sum();
        assert_eq!(marked, 0, "unanchored threads mark no line anyway");
    }

    #[test]
    fn a_thread_in_a_file_that_is_still_loading_says_loading_not_detached() {
        // The lie `Unresolved` exists to prevent: during a streaming load every
        // anchor in an undiffed file would otherwise read as "your code is gone".
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "on a line");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let st = Log::at_worktree(dir.path()).state().unwrap();
        let mut loading = cs_one_file();
        loading.files[0].diffed = false;
        let live = crate::tui::reviewlog::live_threads(&st, &loading);
        assert_eq!(live[0].state_label(), "loading");

        // ...and genuinely gone is a different word.
        let empty = Changeset {
            source: "wt".into(),
            files: Vec::new(),
        };
        let live = crate::tui::reviewlog::live_threads(&st, &empty);
        assert_eq!(live[0].state_label(), "detached");
    }

    #[test]
    fn the_thread_list_keys_move_close_and_ignore_the_rest() {
        let (_d, mut app) = app_with_log();
        for body in ["one", "two", "three"] {
            handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
            type_str(&mut app, body);
            handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        }
        handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(app.thread_list().unwrap().selected, 0);

        handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.thread_list().unwrap().selected, 2);
        // Clamped, not wrapping.
        handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.thread_list().unwrap().selected, 2);
        handle_key(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.thread_list().unwrap().selected, 0);
        handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.thread_list().unwrap().selected, 0, "clamped at the top");

        // An unhandled key does nothing; `q` closes like Esc.
        handle_key(&mut app, KeyCode::F(1), KeyModifiers::NONE);
        assert!(app.thread_list().is_some());
        handle_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(app.thread_list().is_none());
    }

    #[test]
    fn jumping_to_an_unanchored_or_absent_thread_says_so() {
        let (_d, mut app) = app_with_log();
        // A review-level comment has no line to jump to.
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "overall");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.thread_list().is_some(), "the list stays open");
        assert!(app.flash.as_deref().unwrap().contains("not in this diff"));
    }

    #[test]
    fn jumping_with_no_list_open_is_inert() {
        // Not reachable through the router — the key only dispatches while the
        // list is up — so it is exercised directly rather than left uncovered.
        let (_d, mut app) = app_with_log();
        let before = app.state().cursor_row;
        app.threads_jump();
        assert_eq!(app.state().cursor_row, before);
    }

    #[test]
    fn without_a_log_both_openers_say_why() {
        let (_d, mut app) = app_with_log();
        app.attach_review_log(None, false);

        handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(app.thread_list().is_none(), "no log, nothing to list");
        assert!(app.flash.as_deref().unwrap().contains("no worktree"));

        app.flash = None;
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(
            app.comment_input_state().is_none(),
            "and nothing to write to"
        );
        assert!(app.flash.as_deref().unwrap().contains("no worktree"));

        app.refresh_threads();
        assert!(app.thread_marks.is_empty());
    }

    #[test]
    fn a_view_with_no_load_request_cannot_record_a_target() {
        // `push_test_view` builds a view with `req: None`; nothing in production
        // does, but the arm must not silently write a review against nothing.
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
            None, // no LoadRequest
        );
        app.viewport_h = 12;
        app.attach_review_log(Some(Log::at_worktree(dir.path())), false);
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "anything");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.comment_input_state().is_some(), "stays open");
        assert!(app
            .comment_input_state()
            .unwrap()
            .refusal
            .as_deref()
            .unwrap()
            .contains("no target"));
        assert!(!Log::at_worktree(dir.path()).path().exists());
    }

    #[test]
    fn a_comment_survives_a_rename_and_still_jumps() {
        // `review::resolve` matches `previous_path`, so the anchor keeps the old
        // path. A jump keyed on `f.path` alone would refuse a line that is there.
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "still here");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let mut renamed = cs_one_file();
        renamed.files[0].path = "b.rs".into();
        renamed.files[0].previous_path = Some("a.rs".into());
        let mut app2 = App::with_launch(
            &renamed,
            LayoutMode::Stack,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.path().to_path_buf()),
            crate::tui::ViewKind::Local,
            true,
            None,
            Some(crate::git::LoadRequest::Staged),
        );
        app2.viewport_h = 12;
        app2.attach_review_log(Some(Log::at_worktree(dir.path())), false);

        handle_key(&mut app2, KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(app2.thread_list().unwrap().threads.len(), 1);
        handle_key(&mut app2, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app2.thread_list().is_none(), "the jump succeeded");
        assert!(app2.state().cursor_row > 0, "and landed on the line");
    }

    #[test]
    fn every_thread_state_has_its_own_label() {
        use crate::review::Resolution;
        use crate::tui::reviewlog::LiveThread;
        let mk = |placement, resolved| LiveThread {
            id: "t".into(),
            body: "b".into(),
            resolved,
            deleted: false,
            placement,
            path: Some("a.rs".into()),
            key: None,
        };
        assert_eq!(
            mk(Some(Resolution::Attached { line: 1 }), false).state_label(),
            "here"
        );
        assert_eq!(
            mk(Some(Resolution::Shifted { from: 1, to: 2 }), false).state_label(),
            "moved"
        );
        assert_eq!(
            mk(Some(Resolution::Detached), false).state_label(),
            "detached"
        );
        assert_eq!(
            mk(Some(Resolution::Unresolved), false).state_label(),
            "loading"
        );
        assert_eq!(mk(None, false).state_label(), "review");
        assert_eq!(
            mk(Some(Resolution::Detached), true).state_label(),
            "resolved"
        );
    }

    #[test]
    fn jumping_to_a_thread_whose_file_left_the_view_says_so() {
        // The two remaining arms: the anchored file is not in this changeset,
        // and it is but the line has no row in the current plan.
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "on a line");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        // Same log, a view whose changeset no longer holds that file.
        let empty = Changeset {
            source: "wt".into(),
            files: Vec::new(),
        };
        let mut other = App::with_launch(
            &empty,
            LayoutMode::Stack,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.path().to_path_buf()),
            crate::tui::ViewKind::Local,
            true,
            None,
            Some(crate::git::LoadRequest::Staged),
        );
        other.attach_review_log(Some(Log::at_worktree(dir.path())), false);
        handle_key(&mut other, KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(other.thread_list().is_some(), "the thread still lists");
        handle_key(&mut other, KeyCode::Enter, KeyModifiers::NONE);
        assert!(other.thread_list().is_some(), "and the list stays open");
        assert!(
            other.flash.as_deref().unwrap().contains("not in this diff"),
            "{:?}",
            other.flash
        );
    }

    #[test]
    fn jumping_into_a_collapsed_file_says_the_line_is_not_shown() {
        // The file is still in the changeset, but marking it reviewed collapses
        // its body to a placeholder, so the anchored line has no row to jump to.
        let (_d, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "on a line");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
        assert!(app.state().viewed[0], "the file is collapsed");

        handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.thread_list().is_some(), "the list stays open");
        assert!(
            app.flash.as_deref().unwrap().contains("not shown here"),
            "{:?}",
            app.flash
        );
    }

    #[test]
    fn opening_the_list_on_an_unreadable_log_reports_it() {
        let (dir, mut app) = app_with_log();
        // A directory where the log should be: `state()` fails to read it.
        std::fs::create_dir_all(dir.path().join("rediff.jsonl")).unwrap();
        handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(app.thread_list().is_none());
        assert!(app.flash.is_some(), "the user is told, not left guessing");
    }

    /// Open the thread list on the one recorded thread.
    fn open_list(app: &mut App) {
        handle_key(app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(app.thread_list().is_some(), "the list opened");
    }

    #[test]
    fn an_edit_supersedes_and_both_records_stay_on_disk() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "frist");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        open_list(&mut app);
        handle_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);
        let input = app.comment_input_state().expect("the input reopened");
        assert_eq!(input.buffer, "frist", "pre-filled with the current text");
        assert!(input.replacing.is_some(), "and marked as an edit");
        assert!(input.anchor.is_some(), "carrying the stored anchor");
        for _ in 0..5 {
            handle_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        }
        type_str(&mut app, "first");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let log = Log::at_worktree(dir.path());
        let st = log.state().unwrap();
        assert_eq!(st.threads.len(), 1, "one thread, not two");
        let t = st.threads.values().next().unwrap();
        assert_eq!(t.thread.body, "first", "reads as the new text");
        assert_eq!(
            t.thread.anchor.as_ref().map(|a| a.line),
            Some(1),
            "the anchor carried through the supersede"
        );
        // Both records are on disk: superseding is an append, not a rewrite.
        let raw = std::fs::read_to_string(log.path()).unwrap();
        assert!(raw.contains("frist"), "the earlier entry is retained");
        assert!(raw.contains("\"first\""), "and the later one written");

        // The list beneath was refreshed, not left showing the old text.
        assert!(app.thread_list().is_some(), "Esc-free return to the list");
        assert_eq!(app.thread_list().unwrap().current().unwrap().body, "first");
    }

    #[test]
    fn editing_a_resolved_thread_leaves_it_resolved() {
        // The reason a supersede copies the stored record instead of building a
        // fresh one: every flag the edit does not touch has to carry forward.
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "typo here");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        open_list(&mut app);
        handle_key(&mut app, KeyCode::Char('o'), KeyModifiers::NONE);
        assert_eq!(
            app.thread_list().unwrap().current().unwrap().state_label(),
            "resolved"
        );

        handle_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);
        type_str(&mut app, "!");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let st = Log::at_worktree(dir.path()).state().unwrap();
        let t = st.threads.values().next().unwrap();
        assert_eq!(t.thread.body, "typo here!");
        assert!(t.thread.resolved, "still resolved after an edit");
    }

    #[test]
    fn retracting_withdraws_from_delivery_and_can_be_undone() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "never mind this one");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let log = Log::at_worktree(dir.path());

        open_list(&mut app);
        handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(
            app.thread_list().unwrap().current().unwrap().state_label(),
            "retracted"
        );
        let delivery = crate::review::all(&log.state().unwrap(), app.cs().as_ref());
        assert!(delivery.threads.is_empty(), "not delivered to a consumer");
        // ...and still on disk, both entries.
        let raw = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(
            raw.matches("never mind this one").count(),
            2,
            "the original and its retraction: {raw}"
        );
        // The gutter mark is gone with it.
        assert!(app.thread_marks.is_empty(), "no mark on a retracted line");

        // `x` again restores it: one keystroke must not cost a review point
        // with no way back from inside rediff.
        handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(
            app.thread_list().unwrap().current().unwrap().state_label(),
            "here"
        );
        assert_eq!(
            crate::review::all(&log.state().unwrap(), app.cs().as_ref())
                .threads
                .len(),
            1
        );
        assert_eq!(app.thread_marks.len(), 1, "and the mark comes back");
    }

    #[test]
    fn a_resolved_thread_is_still_delivered_and_flagged() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "handled");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        open_list(&mut app);
        handle_key(&mut app, KeyCode::Char('o'), KeyModifiers::NONE);

        let st = Log::at_worktree(dir.path()).state().unwrap();
        let delivery = crate::review::all(&st, app.cs().as_ref());
        assert_eq!(delivery.threads.len(), 1, "resolving does not withdraw it");
        assert!(delivery.threads[0].thread.resolved, "flagged resolved");

        // And toggles back off.
        handle_key(&mut app, KeyCode::Char('o'), KeyModifiers::NONE);
        let st = Log::at_worktree(dir.path()).state().unwrap();
        assert!(!st.threads.values().next().unwrap().thread.resolved);
    }

    #[test]
    fn thread_actions_are_inert_with_no_list_open() {
        // Reachable only via the router while the list is up, so exercised
        // directly rather than left as a coverage hole.
        let (dir, mut app) = app_with_log();
        app.threads_edit();
        app.threads_retract();
        app.threads_resolve();
        assert!(app.comment_input_state().is_none());
        assert!(!Log::at_worktree(dir.path()).path().exists());
    }

    #[test]
    fn acting_on_a_thread_the_log_no_longer_has_says_so() {
        // The `Ok(false)` arm: the list holds an id the log does not. Only
        // reachable by rewriting the log behind the open list, which is what a
        // second rediff replacing a spent review would do.
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "vanishing");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        open_list(&mut app);

        let log = Log::at_worktree(dir.path());
        std::fs::write(log.path(), "").unwrap();
        app.flash = None;
        handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(
            app.flash
                .as_deref()
                .unwrap()
                .contains("no longer in the log"),
            "{:?}",
            app.flash
        );
        app.flash = None;
        handle_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);
        assert!(app.comment_input_state().is_none(), "nothing to edit");
        assert!(app
            .flash
            .as_deref()
            .unwrap()
            .contains("no longer in the log"));
    }

    #[test]
    fn an_unreadable_log_is_reported_rather_than_swallowed() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(&mut app, "here");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        open_list(&mut app);

        // Replace the log file with a directory: every read of it now fails.
        let log = Log::at_worktree(dir.path());
        std::fs::remove_file(log.path()).unwrap();
        std::fs::create_dir(log.path()).unwrap();

        app.flash = None;
        handle_key(&mut app, KeyCode::Char('o'), KeyModifiers::NONE);
        assert!(
            app.flash.as_deref().unwrap().contains("could not resolve"),
            "{:?}",
            app.flash
        );
        app.flash = None;
        handle_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);
        assert!(app.comment_input_state().is_none());
        assert!(app.flash.as_deref().unwrap().contains("could not read"));
    }

    #[test]
    fn an_edit_confirmed_without_a_log_reports_rather_than_panicking() {
        // The `edit_confirm` no-log arm: only reachable by detaching the log
        // between opening the input and confirming it.
        let (_d, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut app, "x");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        open_list(&mut app);
        handle_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);

        app.attach_review_log(None, false);
        type_str(&mut app, "y");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let input = app.comment_input_state().expect("stays open");
        assert!(input.refusal.as_deref().unwrap().contains("no review log"));
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
