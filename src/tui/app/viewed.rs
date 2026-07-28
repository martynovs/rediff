//! Persisting which files the user has marked reviewed, into the review log.
//!
//! Two rules shape this. It records **only while a review is already open** —
//! `v` is the most-pressed key in the app, and writing on it would create a log
//! exactly as opening lazily exists to avoid. And it restores **by path**:
//! `ViewState.viewed` is positional over `cs.files`, whose order differs between
//! sessions, so an index would mark the wrong files.

use std::collections::HashSet;

use crate::review::Record;
use crate::tui::app::types::App;

impl App {
    /// The paths this view has marked reviewed, sorted.
    ///
    /// Sorted so the record is stable and diffable, and so comparing it against
    /// what the log already holds is a plain equality test.
    fn viewed_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .cs()
            .files
            .iter()
            .zip(&self.state().viewed)
            .filter(|(_, v)| **v)
            .map(|(f, _)| f.path.clone())
            .collect();
        paths.sort_unstable();
        paths
    }

    /// Record the reviewed set, if a review over *this* view's target is open.
    ///
    /// Silent about every refusal: this runs on a keystroke whose job is to mark
    /// a file read, and a review that is not open is the normal case, not an
    /// error to report.
    pub(crate) fn record_viewed(&mut self) {
        let Some(target) = self.review_target() else {
            return;
        };
        let Some(log) = self.review_log.as_ref() else {
            return;
        };
        let Ok(st) = log.state() else { return };
        // No review open, or one over something else: `v` writes nothing.
        if st.open.as_ref().is_none_or(|o| o.target != target) {
            return;
        }
        let paths = self.viewed_paths();
        if paths == st.viewed {
            return;
        }
        // A write failure is not worth interrupting a review for — the flags are
        // already applied in this session, and the next `v` retries.
        #[expect(
            clippy::let_underscore_must_use,
            reason = "best-effort: the in-session flag already applied and the next `v` retries"
        )]
        let _ = log.append(&Record::Viewed { paths });
    }

    /// Seed this view's reviewed flags from the log, by path.
    ///
    /// Called once, at launch. Restoring mid-session would make files collapse
    /// under the user in the middle of reading them.
    pub fn restore_viewed(&mut self) {
        if !self.is_review() {
            return;
        }
        let Some(target) = self.review_target() else {
            return;
        };
        let Some(log) = self.review_log.as_ref() else {
            return;
        };
        let Ok(st) = log.state() else { return };
        // Reviewed state belongs to a review: only the one this view is *about*
        // has anything to say about which of these files were read.
        if st.open.as_ref().is_none_or(|o| o.target != target) || st.viewed.is_empty() {
            return;
        }
        let marked: HashSet<&str> = st.viewed.iter().map(String::as_str).collect();
        let flags: Vec<bool> = self
            .cs()
            .files
            .iter()
            .map(|f| marked.contains(f.path.as_str()))
            .collect();
        if !flags.iter().any(|v| *v) {
            return;
        }
        self.state_mut().viewed = flags;
        // `viewed` drives which files collapse, so the plan is now stale.
        self.session.build_plan(self.layout, self.grouped());
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Changeset, DiffFile, FileStatus, Hunk, LayoutMode, Line, LineKind, Stats};
    use crate::review::Log;
    use crate::tui::app::App;
    use crate::tui::runtime::handle_key_for_test as handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    fn file(path: &str) -> DiffFile {
        DiffFile {
            path: path.into(),
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
        }
    }

    fn cs(paths: &[&str]) -> Changeset {
        Changeset {
            source: "wt".into(),
            files: paths.iter().map(|p| file(p)).collect(),
        }
    }

    /// A review-session app over `paths`, with a log in `dir`.
    fn app_over(dir: &std::path::Path, paths: &[&str]) -> App {
        let mut app = App::with_launch(
            &cs(paths),
            LayoutMode::Stack,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.to_path_buf()),
            crate::tui::ViewKind::Local,
            true,
            None,
            Some(crate::git::LoadRequest::WorkingTree {
                include_untracked: true,
                base: None,
            }),
        );
        app.viewport_h = 12;
        app.attach_review_log(Some(Log::at_worktree(dir)), false);
        app
    }

    fn comment(app: &mut App, body: &str) {
        handle_key(app, KeyCode::Char('A'), KeyModifiers::NONE);
        for c in body.chars() {
            handle_key(app, KeyCode::Char(c), KeyModifiers::NONE);
        }
        handle_key(app, KeyCode::Enter, KeyModifiers::NONE);
    }

    #[test]
    fn marking_reviewed_with_no_review_open_writes_nothing() {
        // `v` is the most-pressed key in the app. Writing on it would create a
        // log exactly as opening lazily exists to avoid.
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_over(dir.path(), &["a.rs", "b.rs"]);
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
        assert!(app.state().viewed[0], "the flag still applies in-session");
        assert!(
            !Log::at_worktree(dir.path()).path().exists(),
            "but no log appears"
        );
    }

    #[test]
    fn marks_survive_a_reopen_and_follow_paths_not_positions() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_over(dir.path(), &["a.rs", "b.rs", "c.rs"]);
        comment(&mut app, "starting a review");

        // Mark the middle file.
        handle_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
        assert_eq!(app.state().viewed, vec![false, true, false]);
        assert_eq!(
            Log::at_worktree(dir.path()).state().unwrap().viewed,
            vec!["b.rs".to_string()]
        );

        // A later session over the *same* files in a different order restores the
        // same file, not the same slot.
        let mut later = app_over(dir.path(), &["c.rs", "b.rs", "a.rs"]);
        later.restore_viewed();
        assert_eq!(
            later.state().viewed,
            vec![false, true, false],
            "b.rs is still the reviewed one"
        );
    }

    #[test]
    fn a_review_over_another_target_neither_records_nor_restores() {
        // Reviewed state belongs to a review. Bleeding one review's into a view
        // of something else would mark files the user has never seen.
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_over(dir.path(), &["a.rs", "b.rs"]);
        comment(&mut app, "worktree review");
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
        assert!(!Log::at_worktree(dir.path())
            .state()
            .unwrap()
            .viewed
            .is_empty());

        // A staged view: same log, different target.
        let mut staged = App::with_launch(
            &cs(&["a.rs", "b.rs"]),
            LayoutMode::Stack,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.path().to_path_buf()),
            crate::tui::ViewKind::Staged,
            true,
            None,
            Some(crate::git::LoadRequest::Staged),
        );
        staged.attach_review_log(Some(Log::at_worktree(dir.path())), false);
        staged.restore_viewed();
        assert_eq!(
            staged.state().viewed,
            vec![false, false],
            "the worktree review's marks do not leak into a staged view"
        );

        let before = std::fs::read_to_string(Log::at_worktree(dir.path()).path()).unwrap();
        handle_key(&mut staged, KeyCode::Char('v'), KeyModifiers::NONE);
        assert_eq!(
            std::fs::read_to_string(Log::at_worktree(dir.path()).path()).unwrap(),
            before,
            "and marking there writes nothing to another review's log"
        );
    }

    #[test]
    fn an_unchanged_set_is_not_re_recorded() {
        // Toggling on and off again lands back where the log already is; a line
        // per keypress would make the log mostly this record.
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_over(dir.path(), &["a.rs", "b.rs"]);
        comment(&mut app, "review");
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
        let raw = std::fs::read_to_string(Log::at_worktree(dir.path()).path()).unwrap();
        assert_eq!(
            raw.lines()
                .filter(|l| l.contains(r#""t":"viewed""#))
                .count(),
            3,
            "on, off, on — each a real change: {raw}"
        );
        // Marking a file that is already marked adds nothing.
        let before = raw.len();
        app.record_viewed();
        assert_eq!(
            std::fs::read_to_string(Log::at_worktree(dir.path()).path())
                .unwrap()
                .len(),
            before
        );
    }

    #[test]
    fn restoring_is_inert_where_there_is_nothing_to_restore() {
        let dir = tempfile::tempdir().unwrap();

        // A browse view.
        let mut browse = App::with_launch(
            &cs(&["a.rs"]),
            LayoutMode::Stack,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.path().to_path_buf()),
            crate::tui::ViewKind::Commit("abc".into()),
            false,
            None,
            Some(crate::git::LoadRequest::Show { rev: "abc".into() }),
        );
        browse.attach_review_log(Some(Log::at_worktree(dir.path())), false);
        browse.restore_viewed();
        assert_eq!(browse.state().viewed, vec![false]);

        // A review session with no log at all, and one with an empty log.
        let mut app = app_over(dir.path(), &["a.rs"]);
        app.attach_review_log(None, false);
        app.restore_viewed();
        app.record_viewed();
        app.attach_review_log(Some(Log::at_worktree(dir.path())), false);
        app.restore_viewed();
        assert_eq!(app.state().viewed, vec![false], "no review, no marks");

        // A view with no load request has no target to match against.
        let mut untargeted = App::with_launch(
            &cs(&["a.rs"]),
            LayoutMode::Stack,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.path().to_path_buf()),
            crate::tui::ViewKind::Local,
            true,
            None,
            None,
        );
        untargeted.attach_review_log(Some(Log::at_worktree(dir.path())), false);
        untargeted.restore_viewed();
        untargeted.record_viewed();
        assert_eq!(untargeted.state().viewed, vec![false]);
    }

    #[test]
    fn a_reviewed_file_that_left_the_changeset_restores_nothing() {
        // The recorded path is gone from this view entirely: restoration must
        // leave every present file unmarked rather than shifting the flags along.
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_over(dir.path(), &["a.rs", "b.rs"]);
        comment(&mut app, "review");
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);

        let mut later = app_over(dir.path(), &["c.rs", "d.rs"]);
        later.restore_viewed();
        assert_eq!(later.state().viewed, vec![false, false]);
    }

    #[test]
    fn an_unreadable_log_neither_records_nor_restores() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_over(dir.path(), &["a.rs"]);
        comment(&mut app, "review");
        // Swap the log for a directory: every read of it now fails.
        let log = Log::at_worktree(dir.path());
        std::fs::remove_file(log.path()).unwrap();
        std::fs::create_dir(log.path()).unwrap();

        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
        assert!(app.state().viewed[0], "the in-session flag still applies");
        app.restore_viewed();
        assert!(app.state().viewed[0], "and restoration leaves it alone");
    }

    #[test]
    fn restoring_collapses_the_files_it_marks() {
        // `viewed` drives which files collapse, so the plan has to be rebuilt —
        // otherwise a restored review opens showing every file expanded and the
        // marks only appear in the sidebar.
        let placeholders = |app: &App| {
            app.plan()
                .rows
                .iter()
                .filter(|r| matches!(r, crate::tui::rows::Row::Collapsed { .. }))
                .count()
        };
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_over(dir.path(), &["a.rs", "b.rs"]);
        comment(&mut app, "review");
        handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
        assert_eq!(placeholders(&app), 1, "the reviewed file's body is folded");

        let mut later = app_over(dir.path(), &["a.rs", "b.rs"]);
        assert_eq!(placeholders(&later), 0, "a fresh view shows every body");
        later.restore_viewed();
        assert_eq!(
            placeholders(&later),
            1,
            "and the restored mark folds its file away again"
        );
    }
}
