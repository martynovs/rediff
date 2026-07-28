//! Tests for the single-file peek overlay: opening, mode/context/split toggles,
//! navigation, and its key dispatch (`handle_peek_key`).

use super::keys::{handle_key, BIG_STEP};
use crate::diff::compute_hunks;
use crate::model::{Changeset, DiffFile, FileStatus, Stats};
use crate::tui::app::{App, Overlay};
use crate::tui::peek;
use crate::tui::ui;
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

#[test]
fn peek_preview_opens_content_and_renders() {
    let cs = sample();
    let mut app = App::new(&cs); // selected = 0 → src/auth.rs
    handle_key(&mut app, KeyCode::Char('p'), KeyModifiers::NONE);
    let p = app.peek().expect("peek open");
    assert_eq!(p.mode, peek::PeekMode::Content);

    let mut term = Terminal::new(TestBackend::new(70, 14)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..14 {
        for x in 0..70 {
            out.push_str(buf[(x, y)].symbol());
        }
    }
    assert!(out.contains("preview"), "header shows preview mode");
    assert!(out.contains("login"), "full file content is shown");

    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.peek_open(), "Esc closes the peek");
}

#[test]
fn help_over_peek_retains_the_peek_base() {
    // The layered Mode keeps the base under an overlay: with the peek as the
    // base, opening help and then dismissing it returns to the peek — not to
    // the normal stream. A flat Mode enum (peek/help as siblings) could not
    // express this, because Help would forget what it was over.
    let cs = sample();
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('p'), KeyModifiers::NONE);
    assert!(app.peek_open(), "peek is the active base");
    // Layer help over the peek base (the type permits an overlay over any base).
    app.mode.push_overlay(Overlay::Help);
    assert!(app.help_open());
    // Any key dismisses help…
    handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(!app.help_open(), "help dismissed");
    // …and the base beneath it is retained.
    assert!(
        app.peek_open(),
        "returns to the peek base, not the normal stream"
    );
}

#[test]
fn peek_review_opens_diff_and_context_adjusts() {
    let cs = sample();
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE);
    let p = app.peek().expect("peek open");
    assert_eq!(p.mode, peek::PeekMode::Diff);
    assert!(p.full, "= opens with full context");

    handle_key(&mut app, KeyCode::Char('-'), KeyModifiers::NONE); // compact
    assert!(!app.peek().unwrap().full, "- flips to compact");
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE); // full
    assert!(app.peek().unwrap().full, "= flips back to full");

    handle_key(&mut app, KeyCode::Tab, KeyModifiers::NONE); // diff → blame
    assert_eq!(app.peek().unwrap().mode, peek::PeekMode::Blame);
    handle_key(&mut app, KeyCode::Tab, KeyModifiers::NONE); // blame → content (wraps)
    assert_eq!(app.peek().unwrap().mode, peek::PeekMode::Content);

    app.peek_scroll(5);
    assert_eq!(app.peek().unwrap().state.scroll, 5, "peek scrolls");

    handle_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(!app.peek_open(), "q also closes the peek");
}

#[test]
fn peek_supports_full_navigation() {
    let mut content = String::new();
    for i in 0..100 {
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

    handle_key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    let half = app.peek().unwrap().state.scroll;
    assert!(half > 1, "ctrl-f half-pages the peek (got {half})");
    handle_key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(app.peek().unwrap().state.scroll, 0, "ctrl-b pages back up");

    handle_key(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
    assert!(
        app.peek().unwrap().state.scroll > half,
        "G jumps to the bottom"
    );
    handle_key(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().state.scroll, 0, "g jumps to the top");
}

#[test]
fn peek_context_keys_inert_in_content_mode() {
    let cs = sample();
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('p'), KeyModifiers::NONE); // content preview
    assert_eq!(app.peek().unwrap().mode, peek::PeekMode::Content);
    handle_key(&mut app, KeyCode::Char('-'), KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE);
    // Still in content mode — `-`/`=` don't drop into an (empty) diff.
    assert_eq!(app.peek().unwrap().mode, peek::PeekMode::Content);
    assert!(!app.peek().unwrap().is_empty(), "content stays visible");
}

#[test]
fn peek_diff_header_shows_stat_not_hunk_span() {
    // old a,b,c → new a,X,c,d : +2 (X,d) / -1 (b)
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file(
            "a.rs",
            "a\nb\nc\n",
            "a\nX\nc\nd\n",
            FileStatus::Modified,
        )],
    };
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE);
    let mut term = Terminal::new(TestBackend::new(70, 14)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..14 {
        for x in 0..70 {
            out.push_str(buf[(x, y)].symbol());
        }
    }
    assert!(
        out.contains("+2 -1"),
        "header shows the real change stat: {out}"
    );
    assert!(
        !out.contains("@@"),
        "no misleading @@ hunk header in the peek body"
    );
}

#[test]
fn peek_diff_switches_to_split_layout() {
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file("a.rs", "foo\n", "bar\n", FileStatus::Modified)],
    };
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE); // diff peek
    assert!(!app.peek().unwrap().split_view, "opens unified");
    handle_key(&mut app, KeyCode::Char('m'), KeyModifiers::NONE); // toggle split
    assert!(app.peek().unwrap().split_view, "m flips to split");

    // Render and confirm old/new sit on the same row.
    let mut term = Terminal::new(TestBackend::new(120, 12)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let side_by_side = (0..12u16).any(|y| {
        let row: String = (0..120u16).map(|x| buf[(x, y)].symbol()).collect();
        row.contains("foo") && row.contains("bar")
    });
    assert!(
        side_by_side,
        "split shows old (foo) and new (bar) on the same row"
    );
}

#[test]
fn peek_full_to_compact_keeps_content_on_screen() {
    // A change early in a long file: full has many trailing-context rows,
    // compact has few. Scrolling past the change in full then compacting
    // must not strand the scroll past the (shorter) compact plan.
    let mut old = String::from("change-me\n");
    let mut new = String::from("CHANGED\n");
    for i in 0..80 {
        writeln!(old, "tail {i}").unwrap();
        writeln!(new, "tail {i}").unwrap();
    }
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file("a.rs", &old, &new, FileStatus::Modified)],
    };
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE); // full diff
    let mut term = Terminal::new(TestBackend::new(60, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    handle_key(&mut app, KeyCode::Char('G'), KeyModifiers::NONE); // bottom (past the change)
    handle_key(&mut app, KeyCode::Char('-'), KeyModifiers::NONE); // → compact (shorter)

    let p = app.peek().unwrap();
    let rows = p.plan.rows.len();
    assert!(
        p.state.scroll <= rows.saturating_sub(app.peek_viewport_h),
        "compact keeps a full page"
    );

    // The viewport actually shows content, not blank.
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let nonblank = (0..16u16).any(|y| {
        let row: String = (0..60u16).map(|x| buf[(x, y)].symbol()).collect();
        row.contains("CHANGED") || row.contains("change-me") || row.contains("tail")
    });
    assert!(nonblank, "content is visible after full→compact");
}

#[test]
fn peek_bottom_scroll_keeps_a_full_page() {
    let mut content = String::new();
    for i in 0..60 {
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

    handle_key(&mut app, KeyCode::Char('G'), KeyModifiers::NONE); // bottom
    let p = app.peek().unwrap();
    let rows = p.plan.rows.len();
    // Scroll stops a viewport short of the end, not at rows-1.
    assert!(
        p.state.scroll <= rows.saturating_sub(app.peek_viewport_h),
        "last page stays full"
    );
    assert!(
        rows - p.state.scroll >= app.peek_viewport_h.min(rows),
        "viewport is filled"
    );
}

#[test]
fn peek_toggle_split_after_bottom_scroll_does_not_panic() {
    // Split and unified plans differ in length; scrolling to the bottom in
    // one and toggling must not slice out of range (regression).
    let mut old = String::new();
    let mut new = String::new();
    for i in 0..50 {
        writeln!(old, "a{i}").unwrap();
        writeln!(new, "b{i}").unwrap();
    }
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file("a.rs", &old, &new, FileStatus::Modified)],
    };
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE); // diff
    handle_key(&mut app, KeyCode::Char('m'), KeyModifiers::NONE); // split
    let mut term = Terminal::new(TestBackend::new(120, 14)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    handle_key(&mut app, KeyCode::Char('G'), KeyModifiers::NONE); // bottom of split
    handle_key(&mut app, KeyCode::Char('m'), KeyModifiers::NONE); // → unified (shorter)
                                                                  // Must render without panicking.
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(app.peek().unwrap().state.scroll < app.peek().unwrap().plan.rows.len());
}

#[test]
fn peek_split_hunk_nav_uses_split_positions() {
    // Two modified regions: split pairs the removed/added lines, so the
    // second region sits at a different row than in the unified plan.
    let mut pre = String::new();
    for i in 0..10 {
        writeln!(pre, "ctx{i}").unwrap();
    }
    let mut mid = String::new();
    for i in 0..20 {
        writeln!(mid, "mid{i}").unwrap();
    }
    let old = format!("{pre}a\nb\nc\n{mid}p\nq\nr\n{pre}");
    let new = format!("{pre}X\nY\nZ\n{mid}P\nQ\nR\n{pre}");
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file("a.rs", &old, &new, FileStatus::Modified)],
    };
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE); // full diff (unified)
                                                                  // The single plan is rebuilt per layout, so its change_starts are the
                                                                  // unified positions now and the split positions after `m`.
    let stack_starts = app.peek().unwrap().change_starts.clone();
    handle_key(&mut app, KeyCode::Char('m'), KeyModifiers::NONE); // split
    let split_starts = app.peek().unwrap().change_starts.clone();
    assert_eq!(stack_starts.len(), 2);
    assert_eq!(split_starts.len(), 2);
    assert_ne!(
        stack_starts[1], split_starts[1],
        "second region diverges between layouts"
    );

    handle_key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
    assert_eq!(
        app.peek().unwrap().state.scroll,
        split_starts[0],
        "lands on split region 1"
    );
    handle_key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
    assert_eq!(
        app.peek().unwrap().state.scroll,
        split_starts[1],
        "lands on split region 2"
    );
}

#[test]
fn peek_hunk_nav_jumps_between_hunks() {
    let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
    let new = "X\n2\n3\n4\n5\n6\n7\n8\n9\nY\n"; // two distant changes
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file("a.rs", old, new, FileStatus::Modified)],
    };
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('='), KeyModifiers::NONE); // diff, full context
                                                                  // Full context merges into one hunk, but the two change regions remain.
    assert_eq!(
        app.peek().unwrap().plan.hunk_starts.len(),
        1,
        "full = one hunk"
    );
    assert!(
        app.peek().unwrap().change_starts.len() >= 2,
        "two change regions"
    );

    handle_key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
    let s1 = app.peek().unwrap().state.scroll;
    handle_key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
    let s2 = app.peek().unwrap().state.scroll;
    assert!(s2 > s1, "] advances to the next change region in full mode");
    handle_key(&mut app, KeyCode::Char('['), KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().state.scroll, s1, "[ goes back a region");
}

#[test]
fn peek_keys_cover_fast_scroll_and_paging() {
    let mut content = String::new();
    for i in 0..200 {
        writeln!(content, "line {i}").unwrap();
    }
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file("a.rs", "x\n", &content, FileStatus::Modified)],
    };
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('p'), KeyModifiers::NONE); // content peek
    let mut term = Terminal::new(TestBackend::new(60, 20)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap(); // sets peek_viewport_h

    // J / K fast-scroll by BIG_STEP.
    handle_key(&mut app, KeyCode::Char('J'), KeyModifiers::NONE);
    assert_eq!(
        app.peek().unwrap().state.scroll,
        BIG_STEP as usize,
        "J fast-scrolls"
    );
    handle_key(&mut app, KeyCode::Char('K'), KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().state.scroll, 0, "K fast-scrolls back");

    // Shift+Down / Shift+Up mirror J / K.
    handle_key(&mut app, KeyCode::Down, KeyModifiers::SHIFT);
    assert_eq!(
        app.peek().unwrap().state.scroll,
        BIG_STEP as usize,
        "Shift+Down fast-scrolls"
    );
    handle_key(&mut app, KeyCode::Up, KeyModifiers::SHIFT);
    assert_eq!(
        app.peek().unwrap().state.scroll,
        0,
        "Shift+Up fast-scrolls back"
    );

    // Space / PageDown page down; PageUp pages back.
    handle_key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
    let paged = app.peek().unwrap().state.scroll;
    assert!(paged > 0, "Space pages the peek down");
    handle_key(&mut app, KeyCode::PageUp, KeyModifiers::NONE);
    assert_eq!(app.peek().unwrap().state.scroll, 0, "PageUp pages back");
    handle_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    assert!(
        app.peek().unwrap().state.scroll > 0,
        "PageDown pages the peek down"
    );
}
