//! Tests for view history (back/forward/home/promote), the commit picker, and
//! directory folding in the sidebar.

use super::keys::handle_key;
use crate::diff::compute_hunks;
use crate::model::{Changeset, DiffFile, FileStatus, LayoutMode, Stats};
use crate::tui::app::{App, Focus};
use crate::tui::theme::ThemeName;
use crate::tui::view::ViewKind;
use crate::tui::{rows, ui};
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Terminal;
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

#[test]
fn commit_picker_loads_real_commit_and_switches_view() {
    // Drives the real git load path against the crate's own repo.
    let cs = sample();
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut app = App::with_launch(
        &cs,
        LayoutMode::Stack,
        ThemeName::Dark,
        Some(dir),
        ViewKind::Local,
        true,
        None,
        None,
    );
    assert!(app.is_review(), "launched on local review");

    app.open_commit_palette();
    assert!(app.commit_palette_open());
    assert!(
        !app.palette().unwrap().matches.is_empty(),
        "picker lists commits from HEAD"
    );

    // Pick the first commit → loads it and pushes a browse view.
    app.palette_pick(0);
    assert!(!app.palette_open(), "picking closes the palette");
    assert_eq!(app.session.cursor, 1, "a new view was pushed");
    assert!(
        matches!(app.kind(), ViewKind::Commit(_)),
        "now viewing a commit"
    );
    assert!(!app.is_review(), "a browsed commit is not a review session");
    assert!(!app.cs().files.is_empty(), "the commit's diff loaded");

    // C returns to the local review with its state intact.
    app.view_home();
    assert_eq!(app.session.cursor, 0);
    assert!(app.is_review());
}

#[test]
fn browse_view_disables_viewed_until_promoted() {
    let cs = sample();
    let mut app = App::new(&cs); // home is Local → a review session
    assert!(app.is_review());
    app.push_test_view(&cs, ViewKind::Commit("abc".into()), false);
    assert!(!app.is_review(), "browsed commit is not a review session");

    app.toggle_viewed();
    assert!(
        app.state().viewed.iter().all(|v| !*v),
        "v is inert while browsing"
    );
    assert!(
        !app.next_unviewed(),
        "next-unviewed is inert while browsing"
    );

    app.promote_review();
    assert!(app.is_review(), "R promotes the browse view to a review");
    app.toggle_viewed();
    assert!(
        app.state().viewed.iter().any(|v| *v),
        "v works after promotion"
    );
}

#[test]
fn review_progress_survives_browsing() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    app.state_mut().selected = 2;
    app.toggle_viewed(); // mark file 2 reviewed in the home review
    assert!(app.state().viewed[2]);

    app.push_test_view(&cs, ViewKind::Commit("c1".into()), false);
    assert!(
        !app.state().viewed[2],
        "browse view has its own (empty) viewed state"
    );

    app.view_back(); // back to the home review
    assert!(app.state().viewed[2], "review progress restored on return");
}

#[test]
fn view_back_forward_restores_position() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    app.viewport_h = 8;
    app.scroll_view(6);
    let home_scroll = app.state().scroll;
    assert!(home_scroll > 0);

    app.push_test_view(&cs, ViewKind::Commit("c1".into()), false);
    assert_eq!(app.state().scroll, 0, "new view starts at the top");

    app.view_back();
    assert_eq!(app.session.cursor, 0);
    assert_eq!(app.state().scroll, home_scroll, "home scroll restored");

    app.view_forward();
    assert_eq!(app.session.cursor, 1, "forward returns to the pushed view");
}

#[test]
fn new_view_truncates_forward_history() {
    let cs = sample();
    let mut app = App::new(&cs);
    app.push_test_view(&cs, ViewKind::Commit("a".into()), false);
    app.push_test_view(&cs, ViewKind::Commit("b".into()), false);
    app.view_back(); // cursor at the "a" view, "b" is forward
    assert_eq!(app.session.cursor, 1);
    app.push_test_view(&cs, ViewKind::Commit("c".into()), false);
    assert_eq!(app.session.cursor, 2);
    assert_eq!(
        app.session.views.len(),
        3,
        "forward 'b' was truncated by the new view"
    );
    app.view_forward();
    assert_eq!(app.session.cursor, 2, "nothing forward of the newest view");
}

#[test]
fn home_inert_when_launched_on_commit() {
    let cs = sample();
    // Launched directly on a commit (browse) → no home review.
    let mut app = App::with_launch(
        &cs,
        LayoutMode::Stack,
        ThemeName::Dark,
        None,
        ViewKind::Commit("h".into()),
        false,
        None,
        None,
    );
    assert!(!app.session.home_reviewable());
    app.push_test_view(&cs, ViewKind::Commit("x".into()), false);
    app.view_home();
    assert_eq!(
        app.session.cursor, 1,
        "C is inert when there is no home review"
    );
}

#[test]
fn home_returns_when_launched_local() {
    let cs = sample();
    let mut app = App::new(&cs); // Local home
    app.push_test_view(&cs, ViewKind::Commit("x".into()), false);
    assert_eq!(app.session.cursor, 1);
    app.view_home();
    assert_eq!(app.session.cursor, 0, "C returns to the local home view");
    assert!(app.is_review());
}

// ---- directory collapse ------------------------------------------------

/// Files across three directories, path-sorted as `enumerate` produces:
/// root.rs (0), lib/c.rs (1), src/a.rs (2), src/b.rs (3).
fn multi_dir() -> Changeset {
    Changeset {
        source: "working tree".into(),
        files: vec![
            file("root.rs", "a\n", "b\n", FileStatus::Modified),
            file("lib/c.rs", "a\n", "b\n", FileStatus::Modified),
            file("src/a.rs", "a\n", "b\n", FileStatus::Modified),
            file("src/b.rs", "a\n", "b\n", FileStatus::Modified),
        ],
    }
}

fn drawn(cs: &Changeset) -> (App, Terminal<TestBackend>) {
    let mut app = App::new(cs);
    let mut term = Terminal::new(TestBackend::new(90, 20)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    (app, term)
}

#[test]
fn z_folds_and_unfolds_with_cursor_landing() {
    let cs = multi_dir();
    let (mut app, _term) = drawn(&cs);
    app.goto_file(2); // src/a.rs
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
    assert!(
        app.state().collapsed.contains("src"),
        "z folds the file's directory"
    );
    assert_eq!(
        app.state().selected_dir.as_deref(),
        Some("src"),
        "cursor lands on the placeholder"
    );
    let has_src_header = app
        .plan()
        .rows
        .iter()
        .any(|r| matches!(r, rows::Row::FileHeader(i) if *i >= 2));
    assert!(!has_src_header, "folded src files leave no body headers");
    // z again unfolds, landing on the directory's first file.
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
    assert!(!app.state().collapsed.contains("src"), "z again unfolds");
    assert_eq!(
        app.state().selected_file(),
        Some(2),
        "cursor lands on the directory's first file"
    );
}

#[test]
fn finishing_a_directory_auto_folds_and_advances() {
    let cs = multi_dir();
    let (mut app, _term) = drawn(&cs);
    app.goto_file(2); // src/a.rs
    handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
    assert!(
        !app.state().collapsed.contains("src"),
        "directory not complete after one file"
    );
    app.goto_file(3); // src/b.rs — the last src file
    handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
    assert!(
        app.state().collapsed.contains("src"),
        "completing the directory auto-folds it"
    );
    assert_eq!(
        app.state().selected_file(),
        Some(0),
        "cursor advances to the next unviewed file"
    );
}

#[test]
fn manual_unfold_of_auto_folded_directory_persists() {
    let cs = multi_dir();
    let (mut app, mut term) = drawn(&cs);
    app.goto_file(2);
    handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
    app.goto_file(3);
    handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
    assert!(
        app.state().collapsed.contains("src"),
        "auto-folded after completion"
    );
    // Step onto the src placeholder (nav = File0, File1, Dir(src)) and unfold.
    app.sidebar_move(2);
    assert_eq!(
        app.state().selected_dir.as_deref(),
        Some("src"),
        "cursor on the placeholder"
    );
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
    assert!(
        !app.state().collapsed.contains("src"),
        "manual unfold removes the fold"
    );
    // A redraw must not re-fold the hand-expanded directory.
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(
        !app.state().collapsed.contains("src"),
        "manual unfold persists across redraws"
    );
}

#[test]
fn file_actions_are_inert_on_a_placeholder() {
    let cs = multi_dir();
    let (mut app, _term) = drawn(&cs);
    app.goto_file(2);
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE); // fold src, placeholder selected
    let viewed_before = app.state().viewed.clone();
    handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
    assert_eq!(
        app.state().viewed,
        viewed_before,
        "v does nothing on a placeholder"
    );
    handle_key(&mut app, KeyCode::Char('p'), KeyModifiers::NONE);
    assert!(!app.peek_open(), "peek does not open on a placeholder");
}

#[test]
fn d_to_flat_from_placeholder_lands_on_first_file() {
    use crate::tui::sidebar::Grouping;
    let cs = multi_dir();
    let (mut app, _term) = drawn(&cs);
    app.goto_file(2);
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE); // placeholder src selected
    handle_key(&mut app, KeyCode::Char('D'), KeyModifiers::NONE); // → flat list
    assert_eq!(app.grouping, Grouping::Flat);
    assert_eq!(
        app.state().selected_file(),
        Some(2),
        "placeholder converts to src's first file"
    );
    assert!(
        app.state().collapsed.contains("src"),
        "folds are kept for when grouping returns"
    );
}

#[test]
fn fuzzy_jump_into_a_folded_directory_unfolds_it() {
    let cs = multi_dir();
    let (mut app, _term) = drawn(&cs);
    app.goto_file(2);
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE); // fold src
    assert!(app.state().collapsed.contains("src"));
    app.open_palette();
    for c in "src/b".chars() {
        app.palette_input(c);
    }
    app.palette_confirm();
    assert!(
        !app.state().collapsed.contains("src"),
        "jumping by path unfolds the directory"
    );
    assert_eq!(
        app.state().selected_file(),
        Some(3),
        "lands on the jumped-to file"
    );
}

#[test]
fn folded_directory_renders_placeholder_in_both_panes() {
    let cs = multi_dir();
    let (mut app, mut term) = drawn(&cs);
    app.goto_file(2);
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE); // fold src
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let sb_w = app.sidebar_area.width;
    let (mut sidebar, mut body) = (String::new(), String::new());
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let s = buf[(x, y)].symbol();
            if x < sb_w {
                sidebar.push_str(s);
            } else {
                body.push_str(s);
            }
        }
    }
    assert!(
        sidebar.contains("2 files"),
        "sidebar shows the folded placeholder: {sidebar}"
    );
    assert!(
        body.contains("files hidden"),
        "body shows the folded placeholder: {body}"
    );
    assert!(
        !body.contains("src/a.rs"),
        "folded src files are absent from the body: {body}"
    );
    assert!(
        !body.contains("src/b.rs"),
        "folded src files are absent from the body: {body}"
    );
}

#[test]
fn placeholder_shows_reviewed_progress() {
    let cs = multi_dir();
    let (mut app, mut term) = drawn(&cs);
    app.goto_file(2); // src/a.rs
    handle_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE); // review one of src's two files
    app.goto_file(2);
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE); // fold src (not auto: b.rs unreviewed)
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let sb_w = app.sidebar_area.width;
    let mut sidebar = String::new();
    for y in 0..buf.area.height {
        for x in 0..sb_w.min(buf.area.width) {
            sidebar.push_str(buf[(x, y)].symbol());
        }
    }
    assert!(
        sidebar.contains("1/2 files"),
        "placeholder shows reviewed/total: {sidebar}"
    );
}

#[test]
fn folding_keeps_focus_on_the_file_list() {
    let cs = multi_dir();
    let (mut app, _term) = drawn(&cs);
    app.toggle_focus(); // into the sidebar
    assert_eq!(app.focus(), Focus::Sidebar);
    app.sidebar_move(2); // → src/a.rs, focus stays on the list
    assert_eq!(app.focus(), Focus::Sidebar);
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE); // fold
    assert_eq!(
        app.focus(),
        Focus::Sidebar,
        "fold keeps focus on the file list"
    );
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE); // unfold
    assert_eq!(
        app.focus(),
        Focus::Sidebar,
        "unfold keeps focus on the file list"
    );
}

#[test]
fn fold_state_is_restored_with_the_view() {
    let cs = multi_dir();
    let (mut app, _term) = drawn(&cs);
    app.goto_file(2);
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE); // fold src in the home view
    assert!(app.state().collapsed.contains("src"));
    // Switch to another view, then return — the fold is per-view state.
    app.push_test_view(&cs, ViewKind::Commit("x".into()), false);
    assert!(
        !app.state().collapsed.contains("src"),
        "the pushed view has its own (empty) folds"
    );
    app.view_back();
    assert!(
        app.state().collapsed.contains("src"),
        "returning restores the home view's folds"
    );
}
