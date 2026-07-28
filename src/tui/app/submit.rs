//! Closing a review round: pick a verdict preset, edit its text, send.
//!
//! A submit is a lifecycle event, not another comment — it closes the round, and
//! the store models it as its own record. The two-stage overlay (pick, then
//! edit) exists so the body delivered is unambiguously the body the human wrote:
//! a single line with a "cycle preset" key would have to either discard their
//! edits or keep a preset name that no longer describes the text.

use crate::review::{now, Record, Submit};
use crate::tui::app::types::{App, Overlay, SubmitDraft};

impl App {
    /// The submit draft, if the overlay is open.
    pub fn submit_draft(&self) -> Option<&SubmitDraft> {
        match self.mode.overlay() {
            Some(Overlay::Submit(d)) => Some(d),
            _ => None,
        }
    }

    fn submit_draft_mut(&mut self) -> Option<&mut SubmitDraft> {
        match self.mode.overlay_mut() {
            Some(Overlay::Submit(d)) => Some(d),
            _ => None,
        }
    }

    /// Open the submit overlay, if there is a round to close.
    ///
    /// A round is what a submit closes, so there has to be one: submitting into
    /// a log with no review would record an instruction against round zero,
    /// which no consumer can act on.
    pub fn open_submit(&mut self) {
        if !self.is_review() {
            return;
        }
        let Some(log) = self.review_log.as_ref() else {
            self.flash = Some("this repository has no worktree to hold a review log".to_string());
            return;
        };
        let Ok(st) = log.state() else {
            self.flash = Some("could not read the review log".to_string());
            return;
        };
        if st.rounds.is_empty() {
            self.flash = Some("nothing to submit yet — `a` or `A` leaves a review point".into());
            return;
        }
        let presets = self.verdicts.clone();
        self.mode
            .push_overlay(Overlay::Submit(SubmitDraft::new(presets)));
    }

    pub fn submit_move(&mut self, delta: isize) {
        if let Some(d) = self.submit_draft_mut() {
            d.move_by(delta);
        }
    }

    pub fn submit_input(&mut self, c: char) {
        if let Some(d) = self.submit_draft_mut() {
            if let Some(buf) = d.buffer.as_mut() {
                buf.push(c);
            }
            d.refusal = None;
        }
    }

    pub fn submit_backspace(&mut self) {
        if let Some(d) = self.submit_draft_mut() {
            if let Some(buf) = d.buffer.as_mut() {
                buf.pop();
            }
            d.refusal = None;
        }
    }

    /// Back out one stage: editing returns to the preset list, picking closes.
    pub fn submit_cancel(&mut self) {
        match self.submit_draft_mut() {
            Some(d) if d.buffer.is_some() => {
                d.buffer = None;
                d.refusal = None;
            }
            _ => {
                self.mode.pop_overlay();
            }
        }
    }

    /// Advance: picking chooses a preset to edit, editing sends.
    pub fn submit_confirm(&mut self) {
        let Some(d) = self.submit_draft_mut() else {
            return;
        };
        if d.buffer.is_none() {
            d.begin_edit();
            return;
        }
        let (body, preset) = (
            d.buffer.clone().unwrap_or_default(),
            d.current().map(|p| p.name.clone()),
        );
        if body.trim().is_empty() {
            if let Some(d) = self.submit_draft_mut() {
                d.refusal = Some("an empty instruction closes nothing".to_string());
            }
            return;
        }
        match self.record_submit(&body, preset) {
            Ok(round) => {
                self.mode.pop_overlay();
                self.flash = Some(format!("round {round} submitted"));
            }
            // Kept open so the typed instruction is not lost, and reported
            // inside the box — `draw_status` hides the flash under an overlay.
            Err(msg) => {
                if let Some(d) = self.submit_draft_mut() {
                    d.refusal = Some(msg);
                }
            }
        }
    }

    /// Append the `submit` closing the current round. `Err` carries a message.
    fn record_submit(&mut self, body: &str, preset: Option<String>) -> Result<u32, String> {
        let log = self
            .review_log
            .as_ref()
            .ok_or_else(|| "no review log for this repository".to_string())?;
        let st = log
            .state()
            .map_err(|e| format!("could not read the review log: {e}"))?;
        let round = st
            .rounds
            .last()
            .map(|r| r.n)
            .ok_or_else(|| "there is no open round to close".to_string())?;
        log.append(&Record::Submit(Submit {
            round,
            preset,
            // Exactly as written: the preset seeded this text, it does not
            // define it.
            body: body.to_string(),
            at: now(),
        }))
        .map_err(|e| format!("could not write the review log: {e}"))?;
        Ok(round)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::VerdictPreset;
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

    fn app_with_log() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::with_launch(
            &cs_one_file(),
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
        app.verdicts = vec![
            VerdictPreset {
                name: "approve".into(),
                text: "looks good".into(),
            },
            VerdictPreset {
                name: "rework".into(),
                text: "another pass".into(),
            },
        ];
        (dir, app)
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            handle_key(app, KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    /// Leave one review-level comment, which opens the review and round 1.
    fn comment(app: &mut App, body: &str) {
        handle_key(app, KeyCode::Char('A'), KeyModifiers::NONE);
        type_str(app, body);
        handle_key(app, KeyCode::Enter, KeyModifiers::NONE);
    }

    #[test]
    fn submitting_closes_the_round_with_the_edited_text_and_the_preset_name() {
        // Also the "only a review-level comment" case: nothing here is anchored,
        // and the round still closes.
        let (dir, mut app) = app_with_log();
        comment(&mut app, "overall this is fine");

        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        let d = app.submit_draft().expect("the picker opened");
        assert!(d.buffer.is_none(), "stage one is the preset list");
        assert!(d.title().contains("pick a verdict"));

        // Pick the second preset, then edit its text.
        handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let d = app.submit_draft().expect("still open, now editing");
        assert_eq!(
            d.buffer.as_deref(),
            Some("another pass"),
            "pre-filled from the preset"
        );
        assert!(d.title().contains("rework"), "{}", d.title());
        type_str(&mut app, " on the parser");
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        assert!(app.submit_draft().is_none(), "the overlay closed");
        let st = Log::at_worktree(dir.path()).state().unwrap();
        assert_eq!(st.submits.len(), 1);
        let s = &st.submits[0].1;
        assert_eq!(s.round, 1, "it closes the open round");
        assert_eq!(
            s.body, "another pass on the parser",
            "the text sent, not the preset's"
        );
        assert_eq!(
            s.preset.as_deref(),
            Some("rework"),
            "with the preset named alongside, so a script need not parse prose"
        );
        assert!(app.flash.as_deref().unwrap().contains("round 1"));
    }

    #[test]
    fn there_is_nothing_to_submit_before_a_round_exists() {
        let (dir, mut app) = app_with_log();
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(app.submit_draft().is_none(), "no overlay");
        assert!(
            app.flash.as_deref().unwrap().contains("nothing to submit"),
            "{:?}",
            app.flash
        );
        assert!(
            !Log::at_worktree(dir.path()).path().exists(),
            "and no log is created by asking"
        );
    }

    #[test]
    fn esc_steps_back_one_stage_rather_than_discarding_the_instruction() {
        let (_d, mut app) = app_with_log();
        comment(&mut app, "note");
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        type_str(&mut app, "!");
        assert!(app.submit_draft().unwrap().buffer.is_some());

        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        let d = app.submit_draft().expect("still open");
        assert!(d.buffer.is_none(), "back to the preset list");
        // A second Esc closes.
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.submit_draft().is_none());
    }

    #[test]
    fn an_empty_instruction_is_refused_inside_the_box() {
        let (dir, mut app) = app_with_log();
        comment(&mut app, "note");
        app.verdicts = vec![VerdictPreset {
            name: "blank".into(),
            text: "   ".into(),
        }];
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        let d = app.submit_draft().expect("stays open");
        assert!(
            d.refusal.as_deref().unwrap().contains("closes nothing"),
            "{:?}",
            d.refusal
        );
        assert!(Log::at_worktree(dir.path())
            .state()
            .unwrap()
            .submits
            .is_empty());
        // Typing clears the refusal, so a stale reason does not linger.
        type_str(&mut app, "x");
        assert!(app.submit_draft().unwrap().refusal.is_none());
    }

    #[test]
    fn the_picker_clamps_and_q_closes_it() {
        let (_d, mut app) = app_with_log();
        comment(&mut app, "note");
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        for _ in 0..4 {
            handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        }
        assert_eq!(
            app.submit_draft().unwrap().selected,
            1,
            "clamped at the end"
        );
        for _ in 0..4 {
            handle_key(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
        }
        assert_eq!(app.submit_draft().unwrap().selected, 0, "and at the start");
        // An unhandled key does nothing.
        handle_key(&mut app, KeyCode::F(1), KeyModifiers::NONE);
        assert!(app.submit_draft().is_some());
        handle_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(app.submit_draft().is_none());
    }

    #[test]
    fn while_editing_the_preset_is_fixed_and_ctrl_chords_are_not_text() {
        let (_d, mut app) = app_with_log();
        comment(&mut app, "note");
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        // `j`/`k` are text here, not motion — the chosen preset must not change
        // under the words being typed.
        type_str(&mut app, "jk");
        handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        let d = app.submit_draft().unwrap();
        assert_eq!(d.selected, 0, "the preset stays put");
        assert_eq!(d.buffer.as_deref(), Some("looks goodjk"));

        handle_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        handle_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(
            app.submit_draft().unwrap().buffer.as_deref(),
            Some("looks goodj"),
            "a Ctrl chord is not typed into the buffer that gets persisted"
        );
    }

    #[test]
    fn submitting_is_inert_outside_a_review_and_without_a_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut browse = App::with_launch(
            &cs_one_file(),
            LayoutMode::Stack,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.path().to_path_buf()),
            crate::tui::ViewKind::Commit("abc".into()),
            false, // not a review session
            None,
            Some(crate::git::LoadRequest::Show { rev: "abc".into() }),
        );
        browse.attach_review_log(Some(Log::at_worktree(dir.path())), false);
        handle_key(&mut browse, KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(browse.submit_draft().is_none() && browse.flash.is_none());

        let (_d, mut app) = app_with_log();
        app.attach_review_log(None, false);
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(app.submit_draft().is_none());
        assert!(app.flash.as_deref().unwrap().contains("no worktree"));
    }

    #[test]
    fn an_unreadable_log_is_reported_at_both_ends() {
        let (dir, mut app) = app_with_log();
        comment(&mut app, "note");
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        // Swap the log for a directory: reads and writes both fail from here.
        let log = Log::at_worktree(dir.path());
        std::fs::remove_file(log.path()).unwrap();
        std::fs::create_dir(log.path()).unwrap();

        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let d = app.submit_draft().expect("stays open, text intact");
        assert!(
            d.refusal.as_deref().unwrap().contains("could not read"),
            "{:?}",
            d.refusal
        );

        // ...and opening the picker on it says so rather than showing nothing.
        app.submit_cancel();
        app.submit_cancel();
        app.flash = None;
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(app.submit_draft().is_none());
        assert!(app.flash.as_deref().unwrap().contains("could not read"));
    }

    #[test]
    fn the_draft_accessors_are_inert_with_nothing_open() {
        // Reachable only via the router while the overlay is up.
        let (_d, mut app) = app_with_log();
        app.submit_move(1);
        app.submit_input('x');
        app.submit_backspace();
        app.submit_cancel();
        app.submit_confirm();
        assert!(app.submit_draft().is_none());
    }

    #[test]
    fn a_draft_with_no_presets_titles_and_sends_without_one() {
        // `verdict = []` cannot reach here (the config falls back to built-ins),
        // but the empty-list arms are constructed rather than left uncovered.
        let mut d = crate::tui::app::SubmitDraft::new(Vec::new());
        assert!(d.title().contains("pick a verdict"));
        d.begin_edit();
        assert!(d.buffer.is_none(), "nothing to pre-fill from");
        d.buffer = Some("typed by hand".into());
        assert_eq!(d.title(), "submit", "no preset to name");
        assert!(d.current().is_none());
    }

    #[test]
    fn a_write_failure_at_submit_is_a_message_not_a_panic() {
        let (dir, mut app) = app_with_log();
        comment(&mut app, "note");
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        // Readable log, unwritable path: the append fails where the read did not.
        let log = Log::at_worktree(dir.path());
        let mut perms = std::fs::metadata(log.path()).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(log.path(), perms).unwrap();

        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let d = app.submit_draft().expect("stays open");
        assert!(
            d.refusal.as_deref().unwrap().contains("could not write"),
            "{:?}",
            d.refusal
        );
    }

    #[test]
    fn a_round_that_disappeared_under_the_overlay_is_refused() {
        // `open_submit` guarantees a round, so this arm is only reachable when
        // the log is replaced while the overlay is up — which is exactly what
        // another rediff taking over a spent review does.
        let (dir, mut app) = app_with_log();
        comment(&mut app, "note");
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

        std::fs::write(Log::at_worktree(dir.path()).path(), "").unwrap();
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app
            .submit_draft()
            .unwrap()
            .refusal
            .as_deref()
            .unwrap()
            .contains("no open round"));
    }

    #[test]
    fn recording_a_submit_without_a_log_reports_rather_than_panicking() {
        // The `record_submit` no-log arm: reachable only by detaching the log
        // between opening the overlay and confirming it.
        let (_d, mut app) = app_with_log();
        comment(&mut app, "note");
        handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        app.attach_review_log(None, false);
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app
            .submit_draft()
            .unwrap()
            .refusal
            .as_deref()
            .unwrap()
            .contains("no review log"));
    }
}
