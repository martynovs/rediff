use super::super::appcore::stub_changeset;
use super::*;
use crate::git::LoadRequest;
use crate::model::LayoutMode;
use crate::tui::theme::ThemeName;
use crate::tui::view::ViewKind;
use std::path::PathBuf;
use std::sync::Arc;

fn crate_repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn next_unviewed_is_in_bounds_after_abandon_and_resume() {
    // A review whose load is abandoned (switch away) then resumed (return)
    // must keep `viewed` sized to the fixed file set, so next-unviewed never
    // indexes out of bounds. Pre-refactor, resume re-enumerated and could
    // resize the file set under a stale `viewed`, panicking here.
    let dir = crate_repo();
    let req = LoadRequest::Show { rev: "HEAD".into() };
    let en = crate::git::enumerate(&dir, &req).unwrap();
    if en.stubs.len() < 2 {
        return;
    }
    let cs = stub_changeset(&en);
    let mut app = App::with_launch(
        &cs,
        LayoutMode::Stack,
        ThemeName::Dark,
        Some(dir),
        ViewKind::Commit("HEAD".into()),
        true, // review session
        None,
        Some(req),
    );
    app.session.views[0].stubs = Arc::new(en.stubs);
    let other = (*app.cs()).clone();
    app.push_test_view(&other, ViewKind::Commit("x".into()), false);
    app.view_back();
    assert_eq!(
        app.state().viewed.len(),
        app.cs().files.len(),
        "viewed stays sized to the file set"
    );
    let moved = app.next_unviewed();
    assert!(moved, "lands on an unreviewed file");
    assert!(
        app.state().selected < app.cs().files.len(),
        "selection stays in bounds"
    );
}

// ---- synthetic multi-directory changeset helpers -----------------------

use crate::model::{Changeset, DiffFile, FileStatus, Hunk, Line, Stats};

/// One diffed file with a trivial single-line hunk.
fn dfile(path: &str) -> DiffFile {
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
            lines: vec![Line::added("x".into(), 0)],
        }],
        stats: Stats::default(),
        language: None,
        is_binary: false,
        old_text: None,
        new_text: None,
        content_digest: None,
        diffed: true,
    }
}

/// A path-sorted changeset spanning three directories (`lib`, `src`,
/// `src/tui`) so directory folding/grouping is meaningful. File indices:
/// 0=lib/c.rs 1=lib/d.rs 2=src/a.rs 3=src/b.rs 4=src/tui/e.rs.
fn multi_dir_cs() -> Changeset {
    Changeset {
        source: "t".into(),
        files: vec![
            dfile("lib/c.rs"),
            dfile("lib/d.rs"),
            dfile("src/a.rs"),
            dfile("src/b.rs"),
            dfile("src/tui/e.rs"),
        ],
    }
}

// ---- toggle_fold_dir ---------------------------------------------------

#[test]
fn toggle_fold_dir_folds_and_unfolds_in_grouped_mode() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    assert!(app.grouped(), "default view is grouped");
    // Unfolded → fold_dir branch.
    app.toggle_fold_dir("src");
    assert!(
        app.state().collapsed.contains("src"),
        "fold inserts the dir"
    );
    // Already folded → unfold_dir branch.
    app.toggle_fold_dir("src");
    assert!(
        !app.state().collapsed.contains("src"),
        "second toggle unfolds"
    );
}

#[test]
fn toggle_fold_dir_is_inert_in_flat_view() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    app.toggle_grouping(); // ByDir -> Flat
    assert!(!app.grouped());
    app.toggle_fold_dir("src");
    assert!(
        app.state().collapsed.is_empty(),
        "the flat view has no directories to fold"
    );
}

// ---- fold_all ----------------------------------------------------------

#[test]
fn fold_all_collapses_then_expands_all() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    // All expanded → collapse-all (selected_dir is None, so the dir comes from
    // the selected file's parent via the unwrap_or_else arm).
    app.fold_all();
    for d in ["lib", "src", "src/tui"] {
        assert!(app.state().collapsed.contains(d), "{d} collapsed");
    }
    assert!(
        app.state().selected_dir.is_some(),
        "collapse-all lands on a placeholder"
    );
    // Everything collapsed → expand-all.
    app.fold_all();
    assert!(
        app.state().collapsed.is_empty(),
        "expand-all clears every fold"
    );
    assert!(
        app.state().selected_dir.is_none(),
        "expand-all lands on a file"
    );
}

#[test]
fn fold_all_collapse_uses_an_existing_placeholder_selection() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    // Fold just `src`: the cursor parks on its placeholder (selected_dir = src),
    // while `lib` and `src/tui` stay expanded.
    app.toggle_fold_dir("src");
    assert_eq!(app.state().selected_dir.as_deref(), Some("src"));
    // Some dirs still expanded → collapse-all, taking the placeholder's dir
    // (the `selected_dir.clone()` Some arm).
    app.fold_all();
    assert!(app.state().collapsed.contains("lib"));
    assert!(app.state().collapsed.contains("src/tui"));
    assert_eq!(app.state().selected_dir.as_deref(), Some("src"));
}

#[test]
fn fold_all_is_inert_in_flat_view() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    app.toggle_grouping(); // ByDir -> Flat
    app.fold_all();
    assert!(app.state().collapsed.is_empty());
}

#[test]
fn regrouping_with_a_folded_anchor_keeps_the_scroll() {
    // Persisted folds are ignored while flat; toggling back to grouped hides
    // the anchor file. The rebuild must leave the scroll where it was (clamped)
    // instead of re-anchoring against a made-up start and yanking the viewport
    // to the top of the plan.
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    app.toggle_grouping(); // ByDir -> Flat (folds persist but don't apply)
    app.state_mut().collapsed.insert("src".into());
    // Park the viewport on src/b.rs (index 3) — hidden once "src" folds.
    let start = app.file_starts().get(3).copied().unwrap();
    app.state_mut().scroll = start;
    assert!(start > 0, "the anchor file sits below the top of the plan");
    app.toggle_grouping(); // Flat -> ByDir: the anchor folds away
    assert_eq!(
        app.state().scroll,
        start,
        "anchor gone → the scroll stays put"
    );
}

// ---- set_layout / cycle_mode ------------------------------------------

#[test]
fn set_layout_rebuilds_only_when_the_mode_changes() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    // Stack by default and the configured layout already matches → no-op branch.
    assert!(!app.is_split());
    app.set_layout(80);
    assert!(!app.is_split(), "still stack");
    // Flip the configured layout, then apply it → the plan rebuilds for split.
    app.cycle_mode(); // Stack -> Split
    app.set_layout(80);
    assert!(app.is_split(), "plan rebuilt for split");
    // Re-applying with the plan already split is the no-op branch again.
    app.set_layout(80);
    assert!(app.is_split());
}

#[test]
fn cycle_mode_toggles_between_layouts() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    assert!(matches!(app.layout, LayoutMode::Stack));
    app.cycle_mode();
    assert!(matches!(app.layout, LayoutMode::Split));
    app.cycle_mode();
    assert!(matches!(app.layout, LayoutMode::Stack));
}

// ---- toggle_focus ------------------------------------------------------

#[test]
fn toggle_focus_swaps_stream_and_sidebar() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    assert_eq!(app.focus(), Focus::Stream);
    app.toggle_focus(); // Stream -> Sidebar
    assert_eq!(app.focus(), Focus::Sidebar);
    app.toggle_focus(); // Sidebar -> Stream
    assert_eq!(app.focus(), Focus::Stream);
}

#[test]
fn toggle_focus_is_inert_while_peeking() {
    // Focus only applies to the Normal base; in a Peek base the `if let
    // Base::Normal` guard is false, so toggle_focus is a no-op (and `focus()`
    // still reports Stream).
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    app.open_peek_preview();
    assert!(app.peek_open(), "the peek base is active");
    app.toggle_focus();
    assert_eq!(
        app.focus(),
        Focus::Stream,
        "toggling focus does nothing while peeking"
    );
}

// ---- next_unviewed -----------------------------------------------------

#[test]
fn next_unviewed_returns_false_outside_review() {
    let cs = multi_dir_cs();
    let mut app = App::with_launch(
        &cs,
        LayoutMode::Stack,
        ThemeName::Dark,
        None,
        ViewKind::Local,
        false, // not a review session
        None,
        None,
    );
    assert!(!app.is_review());
    assert!(
        !app.next_unviewed(),
        "next-unviewed is inert outside review"
    );
}

#[test]
fn next_unviewed_advances_to_an_unreviewed_file() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    app.state_mut().select_file(0);
    assert!(app.next_unviewed(), "lands on the next unreviewed file");
    assert!(app.flash.is_none(), "success clears the flash");
    assert!(app.state().selected_file().is_some());
}

#[test]
fn next_unviewed_flashes_all_reviewed_when_nothing_remains() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    let n = app.cs().files.len();
    app.state_mut().viewed = vec![true; n];
    assert!(!app.next_unviewed());
    assert_eq!(app.flash.as_deref(), Some("all reviewed"));
}

#[test]
fn next_unviewed_reports_files_hidden_in_folded_dirs() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    let n = app.cs().files.len();
    app.state_mut().viewed = vec![true; n];
    // Fold `src` and leave one of its files (src/a.rs, index 2) unreviewed: it
    // is hidden in a folded dir, not lost — so the cue counts it.
    app.state_mut().collapsed.insert("src".into());
    app.state_mut().viewed[2] = false;
    assert!(
        !app.next_unviewed(),
        "the only unreviewed file is folded away"
    );
    assert_eq!(
        app.flash.as_deref(),
        Some("none in view · 1 hidden in folded dirs")
    );
}

// ---- toggle_viewed -----------------------------------------------------

#[test]
fn toggle_viewed_is_inert_outside_review() {
    let cs = multi_dir_cs();
    let mut app = App::with_launch(
        &cs,
        LayoutMode::Stack,
        ThemeName::Dark,
        None,
        ViewKind::Local,
        false, // not a review session
        None,
        None,
    );
    app.toggle_viewed();
    assert!(
        app.state().viewed.iter().all(|v| !v),
        "no file is marked viewed outside a review"
    );
}

#[test]
fn toggle_viewed_is_inert_on_a_placeholder() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    // Fold `lib` so the cursor parks on its placeholder (selected_file is None).
    app.toggle_fold_dir("lib");
    assert_eq!(app.state().selected_dir.as_deref(), Some("lib"));
    app.toggle_viewed();
    assert!(
        app.state().viewed.iter().all(|v| !v),
        "toggling on a placeholder marks nothing"
    );
}

#[test]
fn toggle_viewed_marks_and_unmarks_without_completing_the_dir() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    app.state_mut().select_file(0); // lib/c.rs; lib also has lib/d.rs
    app.toggle_viewed();
    assert!(app.state().viewed[0], "marked viewed");
    assert!(
        !app.state().collapsed.contains("lib"),
        "dir not complete → no auto-fold"
    );
    // Toggle off (auto path requires the file to become viewed, so this is inert).
    app.state_mut().select_file(0);
    app.toggle_viewed();
    assert!(!app.state().viewed[0], "unmarked viewed");
    assert!(!app.state().collapsed.contains("lib"));
}

#[test]
fn toggle_viewed_auto_collapses_completed_dir_and_advances() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    // Review lib/c.rs first: lib still has an unreviewed file, so no fold.
    app.state_mut().select_file(0);
    app.toggle_viewed();
    assert!(!app.state().collapsed.contains("lib"));
    // Reviewing lib/d.rs completes lib → auto-fold, then advance to the next
    // unreviewed file (still some in src), so it does not park on a placeholder.
    app.state_mut().select_file(1);
    app.toggle_viewed();
    assert!(
        app.state().collapsed.contains("lib"),
        "completed dir auto-folds"
    );
    assert!(app.state().viewed[1]);
    assert!(
        app.state().selected_file().is_some(),
        "advanced onto an unreviewed file"
    );
}

#[test]
fn toggle_viewed_auto_collapses_and_parks_when_nothing_left() {
    let cs = multi_dir_cs();
    let mut app = App::new(&cs);
    let n = app.cs().files.len();
    // Everything reviewed except lib/d.rs (index 1); lib is otherwise complete.
    app.state_mut().viewed = vec![true; n];
    app.state_mut().viewed[1] = false;
    app.state_mut().select_file(1);
    app.toggle_viewed();
    assert!(
        app.state().collapsed.contains("lib"),
        "completing the last dir auto-folds it"
    );
    assert_eq!(
        app.state().selected_dir.as_deref(),
        Some("lib"),
        "nothing left to review → parks on the new placeholder"
    );
}

#[cfg(test)]
mod cursor_selection_tests {
    use crate::model::{Changeset, DiffFile, FileStatus, Hunk, LayoutMode, Line, LineKind, Stats};
    use crate::tui::app::App;

    fn one_line_file(path: &str) -> DiffFile {
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
                    text: "x".into(),
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
            new_text: None,
            content_digest: None,
            diffed: true,
        }
    }

    #[test]
    fn stepping_off_a_folded_placeholder_and_back_keeps_it_selected() {
        // `file_starts` has no entry for a folded directory, so naming the
        // cursor's file on a placeholder answers with whichever file precedes
        // it — which clears `selected_dir` and makes `z` fold *that* directory
        // instead of unfolding the one under the cursor.
        let cs = Changeset {
            source: "wt".into(),
            files: vec![
                one_line_file("lib/d.rs"),
                one_line_file("src/a.rs"),
                one_line_file("src/b.rs"),
            ],
        };
        let mut app = App::with_mode(&cs, LayoutMode::Stack);
        app.viewport_h = 20;
        app.toggle_fold_dir("src");
        let placeholder = app.state().cursor_row;
        assert_eq!(app.state().selected_dir.as_deref(), Some("src"));

        app.move_cursor(1);
        app.move_cursor(-1);
        assert_eq!(app.state().cursor_row, placeholder, "back on the same row");
        assert_eq!(
            app.state().selected_dir.as_deref(),
            Some("src"),
            "and still selecting the placeholder, so `z` unfolds"
        );

        // The verb the accepted requirement promises: unfold, not fold.
        app.toggle_fold();
        assert!(
            !app.state().collapsed.contains("src"),
            "z on the placeholder unfolds src"
        );
        assert!(
            !app.state().collapsed.contains("lib"),
            "and does not fold the neighbouring directory"
        );
    }

    #[test]
    fn an_empty_changeset_survives_the_file_actions() {
        // `rediff` on a clean tree: `selected` is 0 and `selected_file()` still
        // answers `Some(0)`, so indexing `cs.files` panicked the TUI.
        let cs = Changeset {
            source: "wt".into(),
            files: Vec::new(),
        };
        let mut app = App::with_mode(&cs, LayoutMode::Stack);
        app.toggle_fold();
        app.toggle_viewed();
        app.fold_all();
        assert!(app.state().collapsed.is_empty(), "nothing to fold");
    }
}
