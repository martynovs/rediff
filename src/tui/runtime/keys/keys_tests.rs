use super::*;
use crate::diff::compute_hunks;
use crate::model::{Changeset, DiffFile, FileStatus, Stats};
use crate::tui::app::Overlay;
use crate::tui::ui;
use crate::tui::view::ViewKind;
use ratatui::backend::TestBackend;
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
fn digit_keys_jump_across_visible_files() {
    let cs = big_sample(); // 5 files, scrollable
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    // 1 = first visible, 9 = last visible (of 5 files). Assert the jump
    // target (selected); the last file may not reach the viewport top.
    handle_key(&mut app, KeyCode::Char('9'), KeyModifiers::NONE);
    assert_eq!(app.state().selected, 4, "'9' targets the last visible file");
    handle_key(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
    assert_eq!(
        app.state().selected,
        0,
        "'1' targets the first visible file"
    );
    assert_eq!(app.current_file(), 0);
}

#[test]
fn palette_number_pick_jumps() {
    let cs = sample();
    let mut app = App::new(&cs);
    app.open_palette(); // empty query → all files, README is index 1
                        // matches are in changeset order for an empty query: pick #2
    handle_key(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
    assert!(!app.palette_open(), "picking a number closes the palette");
    assert_eq!(app.current_file(), 1);
}

#[test]
fn d_toggles_sidebar_grouping_keeping_selection() {
    use crate::tui::sidebar::Grouping;
    let cs = sample();
    let mut app = App::new(&cs);
    app.state_mut().selected = 1;
    assert_eq!(
        app.grouping,
        Grouping::ByDir,
        "grouped by directory by default"
    );
    handle_key(&mut app, KeyCode::Char('D'), KeyModifiers::NONE);
    assert_eq!(app.grouping, Grouping::Flat, "D toggles to the flat list");
    assert_eq!(
        app.state().selected,
        1,
        "selection preserved across the toggle"
    );
    handle_key(&mut app, KeyCode::Char('D'), KeyModifiers::NONE);
    assert_eq!(app.grouping, Grouping::ByDir, "D again returns to grouped");
    assert_eq!(app.state().selected, 1, "selection still preserved");
}

#[test]
fn ctrl_arrows_move_between_files() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    handle_key(&mut app, KeyCode::Down, KeyModifiers::CONTROL);
    assert_eq!(app.state().selected, 1, "ctrl-down → next file");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::CONTROL);
    assert_eq!(app.state().selected, 0, "ctrl-up → prev file");
}

#[test]
fn shift_brackets_move_between_files() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    handle_key(&mut app, KeyCode::Char('}'), KeyModifiers::NONE);
    assert_eq!(app.state().selected, 1, "}} → next file");
    handle_key(&mut app, KeyCode::Char('{'), KeyModifiers::NONE);
    assert_eq!(app.state().selected, 0, "{{ → prev file");
    // Plain [ / ] stay hunk navigation, not file navigation.
    handle_key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
    assert_eq!(
        app.state().selected,
        0,
        "plain ] does not change the selected file"
    );
}

#[test]
fn angle_brackets_navigate_view_history() {
    let cs = sample();
    let mut app = App::new(&cs);
    app.push_test_view(&cs, ViewKind::Commit("x".into()), false);
    assert_eq!(app.session.cursor, 1);
    handle_key(&mut app, KeyCode::Char('<'), KeyModifiers::NONE);
    assert_eq!(app.session.cursor, 0, "< → view back");
    handle_key(&mut app, KeyCode::Char('>'), KeyModifiers::NONE);
    assert_eq!(app.session.cursor, 1, "> → view forward");
}

#[test]
fn ctrl_f_b_half_page() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    app.viewport_h = 10;
    handle_key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(app.state().cursor_row, 4, "ctrl-f moves half a page down");
    handle_key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(app.state().cursor_row, 0, "ctrl-b moves half a page up");
}

#[test]
fn ctrl_f_in_sidebar_scrolls_list_keeping_selection() {
    let files: Vec<_> = (0..30)
        .map(|i| file(&format!("f{i}.rs"), "a\n", "b\n", FileStatus::Modified))
        .collect();
    let cs = Changeset {
        source: "wt".into(),
        files,
    };
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(40, 14)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    app.toggle_focus(); // sidebar focus
    let sel_before = app.state().selected;
    let top_before = app.sidebar_top;
    handle_key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    assert_eq!(app.state().selected, sel_before, "selection stays put");
    assert!(app.sidebar_top > top_before, "the file list scrolled");
}

#[test]
fn space_steps_files_shift_space_steps_back() {
    let cs = big_sample(); // 5 files
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert_eq!(app.state().selected, 0);

    handle_key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
    assert_eq!(app.state().selected, 1, "Space → next file");
    handle_key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
    assert_eq!(app.state().selected, 2);
    handle_key(&mut app, KeyCode::Char(' '), KeyModifiers::SHIFT);
    assert_eq!(app.state().selected, 1, "Shift+Space → prev file");
}

#[test]
fn global_keys_cover_remaining_actions() {
    let cs = big_sample(); // 5 files under src/, a Local review
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(80, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    // Tab / BackTab toggle focus.
    handle_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.focus(), Focus::Sidebar, "Tab → sidebar");
    handle_key(&mut app, KeyCode::BackTab, KeyModifiers::NONE);
    assert_eq!(app.focus(), Focus::Stream, "BackTab → stream");

    // s hides / shows the sidebar.
    handle_key(&mut app, KeyCode::Char('s'), KeyModifiers::NONE);
    assert!(app.sidebar_hidden, "s hides the sidebar");
    handle_key(&mut app, KeyCode::Char('s'), KeyModifiers::NONE);
    assert!(!app.sidebar_hidden, "s again shows it");

    // m cycles the layout mode.
    let layout_before = app.layout;
    handle_key(&mut app, KeyCode::Char('m'), KeyModifiers::NONE);
    assert_ne!(app.layout, layout_before, "m cycles the layout");

    // w toggles line wrapping.
    assert!(!app.state().wrap);
    handle_key(&mut app, KeyCode::Char('w'), KeyModifiers::NONE);
    assert!(app.state().wrap, "w enables wrapping");
    handle_key(&mut app, KeyCode::Char('w'), KeyModifiers::NONE);
    assert!(!app.state().wrap, "w again disables it");

    // u jumps to the next unviewed file.
    for i in [0usize, 1, 2] {
        app.state_mut().viewed[i] = true;
    }
    app.state_mut().selected = 0;
    handle_key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);
    assert_eq!(app.current_file(), 3, "u jumps to the next unviewed file");

    // / and f both open the fuzzy file palette.
    handle_key(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
    assert!(app.palette_open(), "/ opens the palette");
    app.palette_close();
    handle_key(&mut app, KeyCode::Char('f'), KeyModifiers::NONE);
    assert!(app.palette_open(), "f opens the palette");
    app.palette_close();

    // c / F open commit pickers; with no repo dir they are inert but routed.
    handle_key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    assert!(!app.commit_palette_open(), "no repo → commit picker inert");
    handle_key(&mut app, KeyCode::Char('F'), KeyModifiers::NONE);
    assert!(!app.palette_open(), "no repo → file history inert");

    // Z collapses every directory.
    handle_key(&mut app, KeyCode::Char('Z'), KeyModifiers::NONE);
    assert!(app.state().collapsed.contains("src"), "Z folds all dirs");

    // C returns to the home view from a pushed view.
    app.push_test_view(&cs, ViewKind::Commit("x".into()), false);
    assert_eq!(app.session.cursor, 1);
    handle_key(&mut app, KeyCode::Char('C'), KeyModifiers::NONE);
    assert_eq!(app.session.cursor, 0, "C returns home");

    // R promotes a browse view to a review.
    app.push_test_view(&cs, ViewKind::Commit("y".into()), false);
    assert!(!app.is_review(), "a pushed browse view is not a review");
    handle_key(&mut app, KeyCode::Char('R'), KeyModifiers::NONE);
    assert!(app.is_review(), "R promotes the browse view");

    // q sets the quit flag.
    handle_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(app.should_quit, "q quits");
}

#[test]
fn sidebar_keys_cover_navigation_and_exit() {
    let files: Vec<_> = (0..10)
        .map(|i| file(&format!("f{i}.rs"), "a\n", "b\n", FileStatus::Modified))
        .collect();
    let cs = Changeset {
        source: "wt".into(),
        files,
    };
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    app.toggle_focus();
    assert_eq!(app.focus(), Focus::Sidebar);

    handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(app.state().selected, 1, "j → next file");
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.state().selected, 2, "Down → next file");
    handle_key(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    assert_eq!(app.state().selected, 1, "k → prev file");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.state().selected, 0, "Up → prev file");
    handle_key(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
    assert_eq!(app.state().selected, 9, "G → last file");
    handle_key(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
    assert_eq!(app.state().selected, 0, "g → first file");
    handle_key(&mut app, KeyCode::End, KeyModifiers::NONE);
    assert_eq!(app.state().selected, 9, "End → last file");
    handle_key(&mut app, KeyCode::Home, KeyModifiers::NONE);
    assert_eq!(app.state().selected, 0, "Home → first file");

    // Enter / l / Right / Esc each drop focus into the stream.
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.focus(), Focus::Stream, "Enter drops into the stream");
    app.toggle_focus();
    handle_key(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
    assert_eq!(app.focus(), Focus::Stream, "l drops into the stream");
    app.toggle_focus();
    handle_key(&mut app, KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(app.focus(), Focus::Stream, "Right drops into the stream");
    app.toggle_focus();
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.focus(), Focus::Stream, "Esc drops into the stream");
}

#[test]
fn stream_keys_cover_paging_hunks_and_extremes() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert_eq!(app.focus(), Focus::Stream);

    handle_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    assert!(app.state().cursor_row > 0, "PageDown advances the stream");
    handle_key(&mut app, KeyCode::PageUp, KeyModifiers::NONE);
    assert_eq!(app.state().cursor_row, 0, "PageUp returns to the top");

    // Hunk jumps still top-align the viewport, and now place the cursor too.
    handle_key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
    let after_hunk = app.state().scroll;
    assert!(after_hunk > 0, "] jumps to the next hunk");
    assert_eq!(
        app.state().cursor_row,
        after_hunk,
        "and the cursor lands on it"
    );
    handle_key(&mut app, KeyCode::Char('['), KeyModifiers::NONE);
    assert!(app.state().scroll <= after_hunk, "[ steps back a hunk");

    handle_key(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
    let bottom = app.state().scroll;
    assert!(bottom > 0, "G jumps to the bottom");
    handle_key(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
    assert_eq!(app.state().scroll, 0, "g jumps to the top");
    handle_key(&mut app, KeyCode::End, KeyModifiers::NONE);
    assert_eq!(app.state().scroll, bottom, "End jumps to the bottom");
    handle_key(&mut app, KeyCode::Home, KeyModifiers::NONE);
    assert_eq!(app.state().scroll, 0, "Home jumps to the top");

    // Esc in the stream quits.
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.should_quit, "Esc in the stream quits");
}

#[test]
fn palette_overlay_keys_cover_edit_and_move() {
    let cs = sample(); // 2 files: src/auth.rs (0), README.md (1)
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
    assert!(app.palette_open());

    // A non-digit char is appended to the query; Backspace removes it.
    handle_key(&mut app, KeyCode::Char('r'), KeyModifiers::NONE);
    assert_eq!(app.palette().unwrap().query, "r", "a letter filters");
    handle_key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(app.palette().unwrap().query, "", "Backspace deletes a char");

    // Down / Up move the highlighted match.
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.palette().unwrap().selected, 1, "Down moves selection");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.palette().unwrap().selected, 0, "Up moves selection");

    // Enter confirms the highlighted match and closes.
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(!app.palette_open(), "Enter confirms and closes");
    assert_eq!(app.current_file(), 0, "jumped to the first match");

    // Esc closes a re-opened palette.
    handle_key(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.palette_open(), "Esc closes the palette");
}

#[test]
fn palette_ignores_unhandled_keys() {
    let cs = sample();
    let mut app = App::new(&cs);
    app.open_palette();
    // A key the palette does not bind (Left) hits the `_` arm: ignored, and
    // the palette stays open with no query change.
    handle_key(&mut app, KeyCode::Left, KeyModifiers::NONE);
    assert!(app.palette_open(), "Left is ignored, palette stays open");
    assert_eq!(app.palette().unwrap().query, "", "no input recorded");
}

#[test]
fn theme_picker_overlay_keys_navigate_grid() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(90, 20)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    handle_key(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
    assert!(matches!(app.mode.overlay(), Some(Overlay::ThemePicker(_))));

    // Arrows and hjkl navigate the grid (live-previewing); the picker stays
    // open across each — covering every theme-picker move arm.
    for code in [
        KeyCode::Right,
        KeyCode::Left,
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Char('l'),
        KeyCode::Char('h'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Tab,
        KeyCode::BackTab,
    ] {
        handle_key(&mut app, code, KeyModifiers::NONE);
        assert!(app.theme_picker_open(), "navigation keeps the picker open");
    }
    // An unhandled key in the picker hits the `_` arm and is a no-op.
    handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(app.theme_picker_open(), "unknown key is a no-op");
}

#[test]
fn help_overlay_any_key_dismisses() {
    let cs = sample();
    let mut app = App::new(&cs);
    app.toggle_help();
    assert!(app.help_open());
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
    assert!(!app.help_open(), "any key dismisses the help overlay");
}

#[test]
fn peek_plain_arrows_scroll_one_line_and_ignore_unbound() {
    let mut content = String::new();
    for i in 0..50 {
        writeln!(content, "line {i}").unwrap();
    }
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file("a.rs", "x\n", &content, FileStatus::Modified)],
    };
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('p'), KeyModifiers::NONE);
    let mut term = Terminal::new(TestBackend::new(60, 20)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap(); // sets peek_viewport_h

    handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().state.scroll, 1, "j scrolls down one");
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().state.scroll, 2, "Down scrolls down one");
    handle_key(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().state.scroll, 1, "k scrolls up one");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().state.scroll, 0, "Up scrolls up one");
    // A peek-unbound key (Left) hits the `_` arm: ignored.
    handle_key(&mut app, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(
        app.peek().unwrap().state.scroll,
        0,
        "Left is a no-op in the peek"
    );
}

#[test]
fn peek_mode_specific_keys_dispatch_only_in_their_mode() {
    use crate::tui::peek::PeekMode;
    let mut content = String::new();
    for i in 0..30 {
        writeln!(content, "line {i}").unwrap();
    }
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file("a.rs", "x\n", &content, FileStatus::Modified)],
    };
    let mut app = App::new(&cs);

    // Diff peek: `m` toggles split and `-` switches to compact — both diff-mode
    // keys, routed by the mode dispatch.
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().mode, PeekMode::Diff);
    let split = app.peek().unwrap().split_view;
    handle_key(&mut app, KeyCode::Char('m'), KeyModifiers::NONE);
    assert_ne!(
        app.peek().unwrap().split_view,
        split,
        "m toggles split in diff mode"
    );
    handle_key(&mut app, KeyCode::Char('-'), KeyModifiers::NONE);
    assert!(
        !app.peek().unwrap().full,
        "- sets compact context in diff mode"
    );
    app.peek_close();

    // Content peek: `m` is not a content-mode key → inert (the dispatch returns
    // false and the shared match ignores it).
    handle_key(&mut app, KeyCode::Char('p'), KeyModifiers::NONE);
    let split = app.peek().unwrap().split_view;
    handle_key(&mut app, KeyCode::Char('m'), KeyModifiers::NONE);
    assert_eq!(
        app.peek().unwrap().split_view,
        split,
        "m does not act in content mode"
    );
    app.peek_close();

    // Blame peek: Enter is routed (no repo here, so nothing opens, but the
    // blame-mode arm is exercised).
    handle_key(&mut app, KeyCode::Char('b'), KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().mode, PeekMode::Blame);
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        !app.commit_msg_open(),
        "Enter routed in blame; no repo → no popup"
    );
}

#[test]
fn commit_message_overlay_routes_scroll_confirm_and_dismiss() {
    use crate::model::CommitMessage;
    use crate::tui::app::{CommitMsg, Overlay};
    let cs = sample();
    let mut app = App::new(&cs);
    let popup = || {
        Overlay::CommitMessage(CommitMsg::new(CommitMessage {
            sha: "abc".into(),
            short: "abc".into(),
            author: "me".into(),
            date: "2026".into(),
            body: "l1\nl2\nl3\nl4\nl5\n".into(),
        }))
    };

    app.mode.push_overlay(popup());
    // j / k scroll the body by one.
    handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(app.commit_msg().unwrap().scroll, 1, "j scrolls down");
    handle_key(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    assert_eq!(app.commit_msg().unwrap().scroll, 0, "k scrolls up");
    // PageDown jumps (clamped to the body), PageUp returns to the top.
    handle_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    assert!(app.commit_msg().unwrap().scroll > 0, "PageDown scrolls");
    handle_key(&mut app, KeyCode::PageUp, KeyModifiers::NONE);
    assert_eq!(app.commit_msg().unwrap().scroll, 0, "PageUp returns to top");
    // An unbound key is ignored, the popup stays open.
    handle_key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
    assert!(app.commit_msg_open(), "unbound key is a no-op");
    // Tab dismisses too (it toggles back to the picker it opened from).
    handle_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert!(!app.commit_msg_open(), "Tab dismisses the popup");
    // Esc dismisses (no stashed overlay → cleared).
    app.mode.push_overlay(popup());
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.commit_msg_open(), "Esc dismisses the popup");

    // Enter confirms: with no repo the switch fails, and the popup stays put
    // (a failed switch must not tear down what the user had open).
    app.mode.push_overlay(popup());
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        app.commit_msg_open(),
        "a failed confirm keeps the popup open"
    );
}

#[test]
fn esc_cancels_a_streaming_load_at_launch() {
    let cs = sample();
    let mut app = App::new(&cs);
    // A zero-job loader still reads as "loading" (the handle is present).
    app.session.loader = Some(crate::tui::loader::Loader::start(
        std::path::PathBuf::new(),
        Vec::new(),
    ));
    assert!(app.loading());
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.should_quit, "cancelling the launch load quits");
}

#[test]
fn b_opens_a_blame_peek_and_placeholder_is_inert() {
    use crate::tui::peek::PeekMode;
    let cs = sample();
    let mut app = App::new(&cs);
    // `b` opens the peek directly in blame mode (no repo here, so it stays empty,
    // but the routing + mode are what this asserts).
    handle_key(&mut app, KeyCode::Char('b'), KeyModifiers::NONE);
    assert!(app.peek_open(), "b opened a peek");
    assert_eq!(app.peek().unwrap().mode, PeekMode::Blame, "in blame mode");
    app.peek_close();

    // On a collapsed-directory placeholder, `b` is inert (no file to blame).
    app.state_mut().select_dir("src".into(), 0);
    handle_key(&mut app, KeyCode::Char('b'), KeyModifiers::NONE);
    assert!(!app.peek_open(), "b is inert on a placeholder");
}

#[test]
fn ctrl_d_u_half_page_and_unbound_ctrl_is_ignored() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    app.viewport_h = 10;
    // ctrl+d / ctrl+u share the f/b half-page arms.
    // A half page is half the *drawn* height, and the cursor is what moves —
    // the viewport follows only once the cursor would leave it.
    handle_key(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert_eq!(app.state().cursor_row, 4, "ctrl+d half-pages down");
    handle_key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(app.state().cursor_row, 0, "ctrl+u half-pages up");
    // An unbound Ctrl chord hits the `_` arm: a no-op.
    let before = app.state().selected;
    handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::CONTROL);
    assert_eq!(app.state().selected, before, "ctrl+x is ignored");
}
