//! Rendering, scrolling, and app-navigation tests: frame painting, horizontal
//! pan/clamp, sidebar visibility, the theme picker, the fuzzy palette, and help.

use super::keys::{handle_key, BIG_STEP};
use crate::diff::compute_hunks;
use crate::model::{Changeset, DiffFile, FileStatus, LayoutMode, Stats};
use crate::tui::app::{App, Focus};
use crate::tui::theme::ThemeName;
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

fn render_to_string(w: u16, h: u16) -> String {
    let cs = sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn paint_does_not_mutate_app() {
    let cs = sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
    // A full draw reconciles the geometry-derived state onto `app`.
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let snap = (
        app.viewport_h,
        app.viewport_w,
        app.peek_viewport_h,
        app.sidebar_area,
        app.sidebar_top,
        app.sidebar_visible,
        app.sidebar_height,
        app.state().scroll,
        app.state().h_scroll,
        app.state().selected,
        app.plan().rows.len(),
    );
    // The paint pass is pure: measuring + painting from `&app` must change no
    // observable state.
    term.draw(|f| {
        let g = ui::measure(&app, f.area());
        ui::paint(f, &app, &g);
    })
    .unwrap();
    let after = (
        app.viewport_h,
        app.viewport_w,
        app.peek_viewport_h,
        app.sidebar_area,
        app.sidebar_top,
        app.sidebar_visible,
        app.sidebar_height,
        app.state().scroll,
        app.state().h_scroll,
        app.state().selected,
        app.plan().rows.len(),
    );
    assert_eq!(snap, after, "paint must not mutate App");
}

#[test]
fn renders_review_frame() {
    let out = render_to_string(74, 16);
    println!("\n{out}");
    assert!(out.contains("auth.rs"), "sidebar/header shows the file");
    assert!(out.contains('+'), "added lines are rendered");
    assert!(out.contains("hunk"), "stream status hint present");
}

/// Render one frame. These demos exercise the render path and assert on
/// theme-derived, deterministic output (the syntax table, the split divider,
/// diff text) — not on the async highlighter, whose worker builds a full engine
/// and can't be awaited reliably under parallel/CI load (and isn't what these
/// render tests are about). The worker → `drain` → render path is covered by the
/// PTY event-loop test instead.
fn render_once(app: &mut App, term: &mut Terminal<TestBackend>) {
    term.draw(|f| ui::draw(f, app)).unwrap();
}

#[test]
fn renders_highlighted_frame_demo() {
    use ratatui::style::Color;
    let cs = Changeset {
        source: "working tree".into(),
        files: vec![{
            let old = "fn main() {\n    let x = 1;\n}\n";
            let new = "fn main() {\n    // greet\n    let name = \"world\";\n    println!(\"hi {name}\");\n}\n";
            let mut f = file("src/main.rs", old, new, FileStatus::Modified);
            f.language = Some("rust".into());
            f
        }],
    };
    let mut app = App::new(&cs);

    let mut term = Terminal::new(TestBackend::new(72, 12)).unwrap();
    render_once(&mut app, &mut term);

    // Dump the frame as truecolor ANSI so the colors are visible.
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..12u16 {
        for x in 0..72u16 {
            let cell = &buf[(x, y)];
            if let Color::Rgb(r, g, b) = cell.fg {
                write!(out, "\x1b[38;2;{r};{g};{b}m{}", cell.symbol()).unwrap();
            } else {
                write!(out, "\x1b[0m{}", cell.symbol()).unwrap();
            }
        }
        out.push_str("\x1b[0m\n");
    }
    println!("\n{out}");

    // The theme's syntax table resolves capture index 9 (keywords) to a color
    // distinct from default text — this is what `ui::resolve` paints `fn` with
    // once the (async) highlight lands. Asserting the table keeps the test
    // deterministic; the resolve-to-cell render path is covered by the ui tests.
    let kw = app.syntax[9];
    let kw = Color::Rgb(kw.0, kw.1, kw.2);
    assert_ne!(
        kw, app.theme.context,
        "keyword color is distinct from default text"
    );
}

#[test]
fn renders_split_layout_demo() {
    let cs = Changeset {
        source: "working tree".into(),
        files: vec![{
            let old = "fn main() {\n    old_call();\n}\n";
            let new = "fn main() {\n    new_call();\n    extra();\n}\n";
            let mut f = file("src/main.rs", old, new, FileStatus::Modified);
            f.language = Some("rust".into());
            f
        }],
    };
    let mut app = App::with_mode(&cs, LayoutMode::Split);
    let mut term = Terminal::new(TestBackend::new(90, 10)).unwrap();
    render_once(&mut app, &mut term);
    assert!(app.is_split(), "split mode forces side-by-side");

    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..10u16 {
        for x in 0..90u16 {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    println!("\n{out}");
    assert!(out.contains('│'), "split has a column divider");
    assert!(
        out.contains("old_call") && out.contains("new_call"),
        "both sides shown"
    );
}

#[test]
fn sparse_digit_mapping_spreads_evenly() {
    use crate::tui::sidebar::{digit_to_offset, offset_to_digit};
    // 17 visible files → digit 1 hits offset 0, digit 9 hits offset 16.
    assert_eq!(digit_to_offset(1, 17), 0);
    assert_eq!(digit_to_offset(9, 17), 16);
    assert_eq!(digit_to_offset(5, 17), 8);
    assert_eq!(offset_to_digit(0, 17), Some(1));
    assert_eq!(offset_to_digit(16, 17), Some(9));
    // small set maps 1:1
    assert_eq!(digit_to_offset(2, 3), 1);
}

#[test]
fn grouped_sidebar_renders_dir_lines_and_basenames() {
    // Hand-built (bypasses the enumerate sort), so list the files path-sorted.
    let cs = Changeset {
        source: "working tree".into(),
        files: vec![
            file("README.md", "", "x\n", FileStatus::Modified),
            file("src/a.rs", "", "x\n", FileStatus::Modified),
            file("src/b.rs", "", "x\n", FileStatus::Modified),
        ],
    };
    let mut app = App::with_mode(&cs, crate::model::LayoutMode::Stack);
    // Grouped by directory is the default — no toggle needed.
    let mut term = Terminal::new(TestBackend::new(80, 12)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer();
    // Only the sidebar columns — the right pane (the diff body) shows full
    // paths in its file headers, which is not what this test is about.
    let sb_w = app.sidebar_area.width;
    let mut text = String::new();
    for y in 0..buf.area.height {
        for x in 0..sb_w.min(buf.area.width) {
            text.push_str(buf[(x, y)].symbol());
        }
        text.push('\n');
    }
    assert!(text.contains("./"), "root directory line shown: {text}");
    assert!(text.contains("src/"), "src directory line shown: {text}");
    assert!(
        text.contains("a.rs") && text.contains("b.rs"),
        "basenames shown: {text}"
    );
    assert!(
        !text.contains("src/a.rs"),
        "grouped file rows show basenames, not full paths: {text}"
    );
}

#[test]
fn horizontal_scroll_is_bounded_by_content_width() {
    // One file with a long line; pan right past the end and confirm it stops.
    let long: String = "x".repeat(300);
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file(
            "a.rs",
            "short\n",
            &format!("{long}\n"),
            FileStatus::Modified,
        )],
    };
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(80, 12)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    // Pan far right; it must clamp so the line's tail can't go past the edge.
    for _ in 0..200 {
        app.h_scroll_by(8);
    }
    let max = app.plan().content_w.saturating_sub(app.viewport_w);
    assert!(
        app.state().h_scroll <= max,
        "h_scroll {} exceeds max {}",
        app.state().h_scroll,
        max
    );
    assert!(app.state().h_scroll > 0, "a long line is still pannable");
    // Content stays on screen: at most max columns are scrolled away.
    assert!(app.state().h_scroll + app.viewport_w >= app.plan().content_w);
}

#[test]
fn horizontal_scroll_keeps_gutter_pins_content() {
    // A long line with a marker at the start and end; the line number stays
    // visible while the start scrolls away and the end comes into view.
    let new = format!("AAAA {} ZZZZ\n", "m".repeat(120));
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file("a.rs", "old\n", &new, FileStatus::Modified)],
    };
    let mut app = App::new(&cs);
    let (w, h) = (90u16, 12u16);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    for _ in 0..300 {
        app.h_scroll_by(8);
    }
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    assert!(
        out.contains("ZZZZ"),
        "end of the line is reachable by panning"
    );
    assert!(!out.contains("AAAA"), "start of the line scrolled away");
    // The added line's number (1) and '+' sign stay pinned, with panned
    // content ('m') immediately after the gutter.
    assert!(
        out.contains("1 +m"),
        "gutter pinned ahead of panned content: {out}"
    );
}

#[test]
fn split_horizontal_scroll_pans_within_columns() {
    let mut long = String::new();
    for i in 0..40 {
        write!(long, "token{i} ").unwrap();
    }
    let cs = Changeset {
        source: "wt".into(),
        files: vec![file(
            "a.rs",
            "old\n",
            &format!("{long}\n"),
            FileStatus::Modified,
        )],
    };
    let mut app = App::with_mode(&cs, LayoutMode::Split);
    let mut term = Terminal::new(TestBackend::new(100, 12)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(app.is_split());

    for _ in 0..200 {
        app.h_scroll_by(8);
    }
    let col_w = app.viewport_w.saturating_sub(1) / 2;
    let max = app.plan().content_w.saturating_sub(col_w);
    assert!(
        app.state().h_scroll <= max,
        "split h_scroll {} exceeds max {}",
        app.state().h_scroll,
        max
    );
    assert!(app.state().h_scroll > 0, "long line pans in split too");

    // The column divider is still on screen (layout not scrolled off).
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..12 {
        for x in 0..100 {
            out.push_str(buf[(x, y)].symbol());
        }
    }
    assert!(
        out.contains('│'),
        "divider stays visible when panned in split"
    );
}

#[test]
fn viewed_collapses_file_in_plan() {
    let cs = sample();
    let mut app = App::new(&cs);
    let before = app.plan().rows.len();
    app.toggle_viewed(); // file 0 (auth.rs) at top of viewport
    assert!(app.state().viewed[0]);
    assert!(
        app.plan().rows.len() < before,
        "viewed file should collapse its hunks"
    );
    assert!(app.next_unviewed(), "README.md is still unviewed");
    assert_eq!(app.current_file(), 1);
}

#[test]
fn palette_filters_and_jumps() {
    let cs = sample();
    let mut app = App::new(&cs);
    app.open_palette();
    for c in "read".chars() {
        app.palette_input(c);
    }
    let p = app.palette().unwrap();
    assert!(
        p.matches
            .first()
            .is_some_and(|&i| cs.files[i].path.contains("README")),
        "README should be the top match for 'read'"
    );
    app.palette_confirm();
    assert!(!app.palette_open());
    assert_eq!(app.current_file(), 1, "jumped to README");
}

#[test]
fn theme_picker_previews_and_commits() {
    let cs = sample();
    let mut app = App::with_options(
        &cs,
        crate::model::LayoutMode::Stack,
        crate::tui::theme::ThemeName::Dark,
    );
    assert!(app.theme.dark);
    let original = app.theme.name;

    // `t` opens the picker on the dark tab (the active theme is dark).
    handle_key(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
    assert!(app.theme_picker_open());

    // The picker grid renders without panic while open.
    let mut term = Terminal::new(TestBackend::new(90, 20)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    // Tab switches to the light tab and live-previews a light theme.
    handle_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert!(
        !app.theme.dark,
        "switching to the light tab previews a light theme"
    );

    // Esc rolls back to the theme active when the picker opened.
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.theme_picker_open());
    assert_eq!(
        app.theme.name, original,
        "cancel restores the original theme"
    );

    // Re-open, switch tab, and commit keeps the previewed theme. (Commit via
    // the app method, not the Enter key, so the test never writes the real
    // config — persistence is covered in `config`.)
    handle_key(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    let committed = app.theme_picker_commit();
    assert_eq!(committed, Some(app.theme.name));
    assert!(!app.theme_picker_open());
    assert!(!app.theme.dark, "commit keeps the previewed light theme");

    // still renders fine in the committed theme
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
}

#[test]
fn theme_picker_q_closes_and_t_advances() {
    let cs = sample();
    let mut app = App::with_options(
        &cs,
        crate::model::LayoutMode::Stack,
        crate::tui::theme::ThemeName::Dark,
    );
    let original = app.theme.name;

    // `t` opens; `t` again advances to the next theme (live preview).
    handle_key(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
    assert!(app.theme_picker_open());
    assert_ne!(app.theme.name, original, "t advances to the next theme");

    // `q` closes the popup and rolls back, like Esc.
    handle_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(!app.theme_picker_open(), "q closes the picker");
    assert_eq!(
        app.theme.name, original,
        "q rolls back to the original theme"
    );
    assert!(
        !app.should_quit,
        "q in the picker closes it, does not quit the app"
    );
}

#[test]
fn navigation_latency_is_fast() {
    // A large synthetic changeset: 200 files × ~50 lines each.
    let mut files = Vec::new();
    for i in 0..200 {
        let mut old = String::new();
        for n in 0..50 {
            writeln!(old, "line {n}").unwrap();
        }
        let mut new = String::new();
        for n in 0..50 {
            writeln!(new, "line {} {n}", if n == 7 { "X" } else { "" }).unwrap();
        }
        files.push(file(
            &format!("src/file{i}.rs"),
            &old,
            &new,
            FileStatus::Modified,
        ));
    }
    let cs = Changeset {
        source: "bench".into(),
        files,
    };
    let mut app = App::new(&cs);
    app.viewport_h = 40;

    // Time 1000 hunk-nav + scroll operations.
    let start = std::time::Instant::now();
    for i in 0..1000 {
        if i % 2 == 0 {
            app.next_hunk();
        } else {
            app.move_cursor(3);
        }
        if i % 100 == 0 {
            app.top();
        }
    }
    let per_op = start.elapsed().as_secs_f64() * 1000.0 / 1000.0;
    println!("nav latency: {per_op:.4} ms/op over a 200-file changeset");
    assert!(
        per_op < 1.0,
        "navigation should be well under 1ms/op (got {per_op:.4})"
    );
}

#[test]
fn toggle_sidebar_hides_panel() {
    let cs = sample();
    let mut app = App::new(&cs);
    assert!(!app.sidebar_hidden);
    app.toggle_focus(); // into sidebar
    app.toggle_sidebar(); // hide it
    assert!(app.sidebar_hidden);
    assert_eq!(
        app.focus(),
        Focus::Stream,
        "hiding the panel moves focus to the diff"
    );

    // The diff now starts at the left edge; no sidebar file-list column.
    let mut term = Terminal::new(TestBackend::new(60, 10)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..10u16 {
        for x in 0..60u16 {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    assert!(out.contains("src/auth.rs"), "diff still renders");
    assert!(!out.contains('│'), "sidebar divider is gone");

    app.toggle_sidebar();
    assert!(!app.sidebar_hidden);
}

#[test]
fn focusing_hidden_sidebar_reveals_it_temporarily() {
    let cs = sample();
    let mut app = App::new(&cs);
    app.toggle_sidebar(); // hide-mode on, focus → stream
    assert!(app.sidebar_hidden);
    assert!(!app.sidebar_shown(), "hidden while focus is on the diff");

    app.toggle_focus(); // Tab into the sidebar
    assert_eq!(app.focus(), Focus::Sidebar);
    assert!(app.sidebar_shown(), "focusing reveals the hidden sidebar");
    assert!(app.sidebar_hidden, "hide-mode is still sticky");

    app.toggle_focus(); // Tab back to the diff
    assert!(!app.sidebar_shown(), "hidden again when focus leaves it");
}

/// Which drawn lines of the stream carry the cursor marker, by y coordinate.
///
/// The marker is a glyph in the stream area's first column, so the existing
/// text-reading harness can see it — asserting a *background* would have needed
/// cell-style inspection that exists nowhere in this file.
fn marked_lines(term: &Terminal<TestBackend>, stream_x: u16) -> Vec<u16> {
    let buf = term.backend().buffer().clone();
    (0..buf.area.height)
        .filter(|&y| buf[(stream_x, y)].symbol() == "\u{258e}")
        .collect()
}

#[test]
fn exactly_one_row_is_marked_as_current_in_both_layouts() {
    let cs = big_sample();
    for layout in [LayoutMode::Stack, LayoutMode::Split] {
        let mut app = App::new(&cs);
        app.layout = layout;
        let mut term = Terminal::new(TestBackend::new(90, 14)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();

        let stream_x = app.sidebar_area.width;
        let marks = marked_lines(&term, stream_x);
        assert_eq!(marks.len(), 1, "exactly one marked row in {layout:?}");
        assert_eq!(
            marks[0] as usize,
            app.state().cursor_row,
            "the marked line is the cursor's row (viewport at the top)"
        );

        // It follows the cursor rather than staying put.
        handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let moved = marked_lines(&term, stream_x);
        assert_eq!(moved, vec![marks[0] + 1], "the mark moved with the cursor");
    }
}

#[test]
fn the_marker_accounts_for_the_pinned_file_header() {
    // The off-by-one that `lines.len()` would produce: the sticky header takes
    // drawn line 0 without being a plan row, so the marker sits one lower than
    // the cursor's offset from `scroll`.
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(90, 14)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    // Scroll into the first file so its header pins, dragging the cursor along.
    app.scroll_view(4);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(app.state().scroll > 0, "the header is pinned");

    let stream_x = app.sidebar_area.width;
    let marks = marked_lines(&term, stream_x);
    assert_eq!(marks.len(), 1);
    let rel = app.state().cursor_row - app.state().scroll;
    assert_eq!(
        marks[0] as usize,
        rel + 1,
        "shifted down one by the pinned header"
    );
}

#[test]
fn the_peek_draws_no_cursor_marker() {
    // The peek shares `ViewState` but never reads `cursor_row`. This is the
    // tripwire for implementing the marker inside the row renderers instead of
    // the `draw_*` functions, which would light up row 0 of every peek.
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(90, 14)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    app.open_peek_preview();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(app.peek_open(), "the peek is up");

    let buf = term.backend().buffer().clone();
    let any = (0..buf.area.height)
        .any(|y| (0..buf.area.width).any(|x| buf[(x, y)].symbol() == "\u{258e}"));
    assert!(!any, "no cursor marker anywhere while the peek is open");
}

#[test]
fn the_status_percentage_tracks_the_cursor_not_the_viewport() {
    // At the bottom of a file the viewport top is a page behind, so reporting
    // `scroll` reads well under 100% with the cursor on the last row. The
    // percentage answers "how far have I read", which is the cursor.
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(90, 14)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    handle_key(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(
        app.state().cursor_row > app.state().scroll,
        "the viewport top lags the cursor, so the two disagree"
    );

    let buf = term.backend().buffer().clone();
    let mut status = String::new();
    for x in 0..90u16 {
        status.push_str(buf[(x, 13)].symbol());
    }
    assert!(
        status.contains("100%"),
        "G reads 100%, not the viewport's lower figure: {status:?}"
    );
}

#[test]
fn the_comment_overlay_draws_its_title_and_typed_text() {
    let cs = big_sample();
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_launch(
        &cs,
        LayoutMode::Stack,
        crate::tui::theme::ThemeName::Dark,
        Some(dir.path().to_path_buf()),
        crate::tui::ViewKind::Local,
        true,
        None,
        Some(crate::git::LoadRequest::Staged),
    );
    app.attach_review_log(Some(crate::review::Log::at_worktree(dir.path())), false);
    let mut term = Terminal::new(TestBackend::new(90, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
    for c in "needs a test".chars() {
        handle_key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..16u16 {
        for x in 0..90u16 {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    assert!(
        out.contains("needs a test"),
        "the typed text is drawn: {out}"
    );
    assert!(
        out.contains("comment on this review"),
        "and the status line names what is being composed"
    );
}

#[test]
fn the_comment_overlay_shows_a_refusal_and_the_tail_of_a_long_line() {
    // Both paths a user actually hits and neither of which the status bar can
    // show: `draw_status` hides the flash under an overlay, and a comment
    // longer than the box would otherwise scroll off with the cursor.
    let cs = big_sample();
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_launch(
        &cs,
        LayoutMode::Stack,
        crate::tui::theme::ThemeName::Dark,
        Some(dir.path().to_path_buf()),
        crate::tui::ViewKind::Local,
        true,
        None,
        Some(crate::git::LoadRequest::Staged),
    );
    // Filtered, so confirming is refused and the reason must be visible.
    app.attach_review_log(Some(crate::review::Log::at_worktree(dir.path())), true);
    let mut term = Terminal::new(TestBackend::new(80, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
    let long: String = std::iter::repeat_n('x', 200).collect();
    for c in long.chars().chain("TAIL".chars()) {
        handle_key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..16u16 {
        for x in 0..80u16 {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    assert!(
        out.contains("TAIL"),
        "the caret end of a long comment stays visible: {out}"
    );
    assert!(
        out.contains("a review covers a whole target; this view is filtered"),
        "and the whole refusal fits inside the box: {out}"
    );
}

#[test]
fn the_gutter_marks_a_commented_line_and_the_list_renders() {
    // The marker lives in the gutter's trailing separator column, which exists
    // in both layouts — `area.x` is taken by the cursor marker.
    for layout in [LayoutMode::Stack, LayoutMode::Split] {
        let cs = big_sample();
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::with_launch(
            &cs,
            layout,
            crate::tui::theme::ThemeName::Dark,
            Some(dir.path().to_path_buf()),
            crate::tui::ViewKind::Local,
            true,
            None,
            Some(crate::git::LoadRequest::Staged),
        );
        app.attach_review_log(Some(crate::review::Log::at_worktree(dir.path())), false);
        let mut term = Terminal::new(TestBackend::new(100, 16)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();

        // Land on a real diff line, then comment on it.
        handle_key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        for c in "look at this".chars() {
            handle_key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        }
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            !app.thread_marks.is_empty(),
            "a mark was recorded ({layout:?})"
        );
        term.draw(|f| ui::draw(f, &mut app)).unwrap();

        let buf = term.backend().buffer().clone();
        let marked = (0..16u16).any(|y| (0..100u16).any(|x| buf[(x, y)].symbol() == "\u{2022}"));
        assert!(marked, "the gutter shows the comment marker in {layout:?}");

        // And the list draws the body and where it sits.
        handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..16u16 {
            for x in 0..100u16 {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        assert!(
            out.contains("look at this"),
            "the list shows the body: {out}"
        );
        assert!(out.contains("review points"), "and is titled");
    }
}

#[test]
fn the_submit_box_renders_both_stages() {
    let cs = big_sample();
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_launch(
        &cs,
        LayoutMode::Stack,
        crate::tui::theme::ThemeName::Dark,
        Some(dir.path().to_path_buf()),
        crate::tui::ViewKind::Local,
        true,
        None,
        Some(crate::git::LoadRequest::Staged),
    );
    app.attach_review_log(Some(crate::review::Log::at_worktree(dir.path())), false);
    app.verdicts = vec![crate::config::VerdictPreset {
        name: "rework".into(),
        text: "another pass please".into(),
    }];
    let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    handle_key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
    for c in "MYCOMMENT".chars() {
        handle_key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    let text = |term: &Terminal<TestBackend>| {
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..20u16 {
            for x in 0..100u16 {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    };

    // Editing pushes the input over the list and its title says so.
    handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let out = text(&term);
    assert!(out.contains("edit review comment"), "{out}");
    assert!(out.contains("MYCOMMENT"), "pre-filled with the text: {out}");

    // Esc returns to the list, which is drawn again — not to the diff.
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(
        text(&term).contains("review points"),
        "Esc from an edit lands back on the list it was opened from"
    );
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    // Stage one lists the presets; stage two shows the editable instruction.
    handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let out = text(&term);
    assert!(out.contains("pick a verdict"), "{out}");
    assert!(out.contains("another pass please"), "the preset is listed");

    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    for c in "TAIL".chars() {
        handle_key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let out = text(&term);
    assert!(out.contains("submit"), "titled with the stage: {out}");
    assert!(out.contains("another pass pleaseTAIL"), "editable: {out}");
}

#[test]
fn wrap_mode_draws_no_cursor_marker() {
    // One plan row spends several drawn lines under `wrap`, so `cursor_row -
    // scroll` stops being the drawn line. A marker placed by that arithmetic
    // points at an unrelated continuation line — worse than showing none.
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(60, 14)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let stream_x = app.sidebar_area.width;
    assert_eq!(
        marked_lines(&term, stream_x).len(),
        1,
        "marked while unwrapped"
    );

    handle_key(&mut app, KeyCode::Char('w'), KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(app.state().wrap, "wrap is on");
    assert!(
        marked_lines(&term, stream_x).is_empty(),
        "no marker rather than a misplaced one"
    );
}

#[test]
fn g_reaches_the_last_row_and_survives_a_draw() {
    // Deliberately asserted *after* a render. `App::clamp` runs from `reconcile`
    // on every draw; if it bounded `scroll` against the full viewport while
    // `bottom` used the drawn height, it would snap back one row per frame and
    // drag the cursor off the last row with it. A unit test of `bottom()` alone
    // passes with that bug present.
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(80, 14)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    handle_key(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    let last = app.plan().rows.len() - 1;
    assert_eq!(
        app.state().cursor_row,
        last,
        "G lands on the last row and stays"
    );

    let usable = crate::tui::stream::usable(app.plan(), app.viewport_h);
    let scroll = app.state().scroll;
    assert!(
        (scroll..scroll + usable).contains(&last),
        "the last row is drawn: window {scroll}..{} holds {last}",
        scroll + usable
    );
}

#[test]
fn sticky_header_pins_current_file() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(70, 12)).unwrap();
    // Scroll a few rows into the first file, past its header.
    app.scroll_view(4);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(app.state().scroll > 0);

    // The stream's top line (right of the sidebar) shows the current file.
    let buf = term.backend().buffer().clone();
    let mut top = String::new();
    for x in 34..70u16 {
        top.push_str(buf[(x, 0)].symbol());
    }
    assert!(
        top.contains("file0.rs"),
        "current file header pinned at top: {top:?}"
    );
}

#[test]
fn moving_the_cursor_into_a_file_selects_it() {
    // The `j`/`k` path specifically. This used to be covered by a test that
    // called `App::scroll_by` — which *was* the `j` path — but that method split
    // in two, and the test followed the scroll-gesture half, leaving cursor
    // motion's selection anchoring uncovered.
    let cs = big_sample();
    let mut app = App::new(&cs);
    app.viewport_h = 10;
    #[expect(
        clippy::cast_possible_wrap,
        reason = "a file-start row offset in a tiny test changeset is far below isize::MAX"
    )]
    let second = app.plan().file_starts[1] as isize;
    app.move_cursor(second);

    assert_eq!(app.state().selected, 1, "the cursor's file is selected");
    // Asymmetric on purpose: the viewport lags the cursor by a page, so this
    // distinguishes a cursor-derived selection from a viewport-derived one.
    // Seeded with scroll == cursor, both models agree and the test proves nothing.
    assert_eq!(
        app.current_file(),
        0,
        "while the viewport top is still in the first file"
    );

    app.top();
    assert_eq!(
        app.state().selected,
        0,
        "returning to the top reselects the first file"
    );
}

#[test]
fn a_scroll_gesture_also_updates_the_selected_file() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    app.viewport_h = 10;
    #[expect(
        clippy::cast_possible_wrap,
        reason = "a file-start row offset in a tiny test changeset is far below isize::MAX"
    )]
    let second = app.plan().file_starts[1] as isize;
    app.scroll_view(second);
    assert_eq!(app.state().selected, 1, "J/K and the wheel anchor too");
}

#[test]
fn the_cursor_survives_a_layout_toggle_in_both_directions() {
    // The whole "surviving a rebuild" story, at app level. The unit tests cover
    // `capture_cursor`/`restore_cursor`; nothing exercised them through a real
    // rebuild with the cursor anywhere but row 0 — so dropping the restore
    // assignment in `Session::build_plan` went unnoticed.
    //
    // Each direction is checked from a fresh start rather than as a round trip:
    // a unified removed line and the split pair holding it are the *same
    // change*, and coming back the pair's preferred identity is the added side,
    // so a round trip may legitimately land on the change's other row.
    for start_split in [false, true] {
        let cs = big_sample();
        let mut app = if start_split {
            App::with_mode(&cs, LayoutMode::Split)
        } else {
            App::new(&cs)
        };
        app.viewport_h = 12;
        let target = app.plan().file_starts[2] + 4;
        #[expect(
            clippy::cast_possible_wrap,
            reason = "a row offset in a tiny test changeset is far below isize::MAX"
        )]
        let delta = target as isize;
        app.move_cursor(delta);
        let key = crate::tui::rows::cursor_key(app.plan(), app.state().cursor_row)
            .expect("parked on a body row with an identity");

        app.cycle_mode();
        app.set_layout(80);

        let row = app
            .plan()
            .rows
            .get(app.state().cursor_row)
            .expect("cursor_row is clamped into the plan on every rebuild");
        let (old, new) = crate::tui::rows::row_keys(row);
        assert!(
            old == Some(key) || new == Some(key),
            "the cursor's row still carries {key:?} (started split: {start_split})"
        );
    }
}

#[test]
fn selection_survives_focus_toggle() {
    let cs = big_sample(); // 5 files; last ones share the final page
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    app.toggle_focus(); // into sidebar
    app.sidebar_move(100); // to the last file
    assert_eq!(app.state().selected, 4);

    app.focus_stream(); // switch to the diff
    app.toggle_focus(); // back to the sidebar
    assert_eq!(
        app.state().selected,
        4,
        "last-file selection survives the round trip"
    );
}

#[test]
fn help_overlay_toggles_and_renders() {
    let cs = sample();
    let mut app = App::new(&cs);
    handle_key(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
    assert!(app.help_open());

    let (tw, th) = (90u16, 28u16);
    let mut term = Terminal::new(TestBackend::new(tw, th)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..th {
        for x in 0..tw {
            out.push_str(buf[(x, y)].symbol());
        }
    }
    assert!(
        out.contains("pick a commit"),
        "help lists the commit picker"
    );
    assert!(out.contains("review commit"), "help lists R (promote)");

    // Any key *other than* a scroll key dismisses it; j/k scroll, because the
    // catalog no longer fits an 80x24 terminal.
    handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert!(app.help_open(), "j scrolls rather than closing");
    handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(!app.help_open());
}

#[test]
fn the_help_box_scrolls_when_the_catalog_does_not_fit() {
    let cs = sample();
    let mut app = App::new(&cs);
    let (tw, th) = (80u16, 24u16);
    let mut term = Terminal::new(TestBackend::new(tw, th)).unwrap();
    let text = |term: &Terminal<TestBackend>| {
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..th {
            for x in 0..tw {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    };

    handle_key(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let out = text(&term);
    assert!(out.contains("jk scroll"), "the footer says so: {out}");
    assert!(
        !out.contains("blame file"),
        "the last section is off the bottom at this size"
    );

    // Scroll it into view. The `y` key added by this change lives in the same
    // region — a documented key you cannot reach is a key that does not exist.
    for _ in 0..3 {
        handle_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    }
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let out = text(&term);
    assert!(out.contains("blame file"), "scrolled into view: {out}");
    assert!(out.contains("submit the round"), "and so is `y`");

    // Clamped at both ends: scrolling past the end shows the same last screen.
    let end = out.clone();
    handle_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert_eq!(text(&term), end, "clamped at the bottom");
    for _ in 0..9 {
        handle_key(&mut app, KeyCode::PageUp, KeyModifiers::NONE);
    }
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(text(&term).contains("move cursor"), "back to the top");

    // Reopening starts at the top rather than where it was left.
    handle_key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
    assert!(app.help_open(), "space scrolls too");
    handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
    assert_eq!(app.help_scroll, 0);
}

#[test]
fn sidebar_focus_navigates_files() {
    let cs = sample();
    let mut app = App::new(&cs);
    // focus sidebar, move down to the second file, stream should follow
    app.toggle_focus();
    assert_eq!(app.focus(), Focus::Sidebar);
    assert_eq!(app.state().selected, 0);
    app.sidebar_move(1);
    assert_eq!(app.state().selected, 1);
    assert_eq!(app.current_file(), 1, "stream jumped to the selected file");

    // render in sidebar focus shows the selection marker + focus hint
    let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..16 {
        for x in 0..74 {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    println!("\n{out}");
    assert!(out.contains("select"), "sidebar focus hint present");
    assert!(out.contains('▌'), "selection marker present");
}

#[test]
fn theme_picker_arrow_keys_navigate_grid() {
    let cs = sample();
    let mut app = App::with_options(&cs, LayoutMode::Stack, ThemeName::Dark);
    let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
    handle_key(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
    assert!(app.theme_picker_open());
    term.draw(|f| ui::draw(f, &mut app)).unwrap(); // sizes the grid

    // Arrows and hjkl all drive the grid cursor (clamped within the tab); the
    // picker stays open throughout.
    for code in [
        KeyCode::Char('j'),
        KeyCode::Down,
        KeyCode::Char('l'),
        KeyCode::Right,
        KeyCode::Char('h'),
        KeyCode::Left,
        KeyCode::Char('k'),
        KeyCode::Up,
    ] {
        handle_key(&mut app, code, KeyModifiers::NONE);
        assert!(app.theme_picker_open(), "navigation keeps the picker open");
    }
    // BackTab switches tabs (like Tab).
    handle_key(&mut app, KeyCode::BackTab, KeyModifiers::NONE);
    assert!(app.theme_picker_open());

    // Esc cancels without persisting any theme.
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.theme_picker_open(), "Esc cancels the picker");
}

#[test]
fn scroll_step_is_one_shift_is_several() {
    let long = "x".repeat(300);
    let cs = Changeset {
        source: "wt".into(),
        files: (0..12)
            .map(|i| {
                file(
                    &format!("f{i}.rs"),
                    "a\n",
                    &format!("{long}\n{long}\n"),
                    FileStatus::Modified,
                )
            })
            .collect(),
    };
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(90, 12)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    // Vertical: `j` steps the cursor one row without scrolling (it is still on
    // screen); the Shift gestures scroll the viewport and carry the cursor.
    handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(app.state().cursor_row, 1, "j steps the cursor one row");
    assert_eq!(
        app.state().scroll,
        0,
        "and does not scroll — the row is visible"
    );
    handle_key(&mut app, KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert_eq!(app.state().scroll, BIG_STEP as usize, "J scrolls several");
    assert_eq!(
        app.state().cursor_row,
        1 + BIG_STEP as usize,
        "and the cursor keeps its row on screen"
    );
    handle_key(&mut app, KeyCode::Down, KeyModifiers::SHIFT);
    assert_eq!(
        app.state().scroll,
        2 * BIG_STEP as usize,
        "Shift+Down scrolls several"
    );

    // Horizontal: base one column, Shift several.
    handle_key(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
    assert_eq!(app.state().h_scroll, 1, "l pans one column");
    handle_key(&mut app, KeyCode::Char('L'), KeyModifiers::SHIFT);
    assert_eq!(
        app.state().h_scroll,
        1 + BIG_STEP as usize,
        "L pans several"
    );
    handle_key(&mut app, KeyCode::Right, KeyModifiers::SHIFT);
    assert_eq!(
        app.state().h_scroll,
        1 + 2 * BIG_STEP as usize,
        "Shift+Right pans several"
    );
    handle_key(&mut app, KeyCode::Char('h'), KeyModifiers::NONE);
    assert_eq!(
        app.state().h_scroll,
        2 * BIG_STEP as usize,
        "h pans back one column"
    );
}

#[test]
fn stream_keys_cover_scroll_family() {
    let cs = big_sample();
    let mut app = App::new(&cs);
    let mut term = Terminal::new(TestBackend::new(74, 16)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert_eq!(app.focus(), Focus::Stream);

    // Vertical scroll: fast (J/K, Shift+↑↓) and single-step (j/k, ↑↓).
    handle_key(&mut app, KeyCode::Char('J'), KeyModifiers::NONE);
    assert!(app.state().scroll > 0, "J fast-scrolls down");
    handle_key(&mut app, KeyCode::Char('K'), KeyModifiers::NONE);
    assert_eq!(app.state().scroll, 0, "K fast-scrolls back up");
    handle_key(&mut app, KeyCode::Down, KeyModifiers::SHIFT);
    assert!(app.state().scroll > 0, "Shift+Down fast-scrolls");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::SHIFT);
    assert_eq!(app.state().scroll, 0, "Shift+Up fast-scrolls back");
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let one = app.state().cursor_row;
    assert!(one > 0, "Down steps the cursor one row");
    handle_key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert!(app.state().cursor_row > one, "j steps one more");
    handle_key(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.state().cursor_row, 0, "k/Up step back to the first row");
    assert_eq!(app.state().scroll, 0, "and the viewport never left the top");

    // Horizontal pan: every arm runs (clamping is fine — coverage, not motion).
    for (code, mods) in [
        (KeyCode::Char('L'), KeyModifiers::NONE),
        (KeyCode::Char('H'), KeyModifiers::NONE),
        (KeyCode::Right, KeyModifiers::SHIFT),
        (KeyCode::Left, KeyModifiers::SHIFT),
        (KeyCode::Right, KeyModifiers::NONE),
        (KeyCode::Char('l'), KeyModifiers::NONE),
        (KeyCode::Left, KeyModifiers::NONE),
        (KeyCode::Char('h'), KeyModifiers::NONE),
    ] {
        handle_key(&mut app, code, mods);
    }
    assert!(!app.should_quit, "scrolling never quits");
}
