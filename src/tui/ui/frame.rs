//! Top-level frame orchestration (measure/reconcile/paint), the status bar, and
//! shared color/path helpers used across the rendering submodules.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::model::{FileStatus, LayoutMode};
use crate::tui::app::App;
use crate::tui::keymap;
use crate::tui::theme::Theme;

pub(super) fn rgb(c: crate::highlight::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// Resolve a highlight `Paint` to a concrete color against the active theme: a
/// capture index looks up the theme's syntax table, `Default` is the theme
/// foreground, and `Fixed` is already resolved. This is where deferred,
/// theme-independent tree-sitter spans become colors at render time.
pub(super) fn resolve(paint: crate::highlight::Paint, app: &App) -> crate::highlight::Rgb {
    use crate::highlight::Paint;
    match paint {
        Paint::Capture(i) => app
            .syntax
            .get(i as usize)
            .copied()
            .unwrap_or(app.theme.context_rgb()),
        Paint::Default => app.theme.context_rgb(),
        Paint::Fixed(c) => c,
    }
}

/// The current view's source accent: blue for local/staged, green for commits.
pub(super) fn source_color(app: &App) -> Color {
    if app.source_is_local() {
        app.theme.local
    } else {
        app.theme.commit
    }
}

/// Apply an emphasis background over the char range `[start, end)` of `spans`,
/// splitting spans at the boundaries. `spans` cover the line body text only.
pub(super) fn emphasize(
    spans: Vec<Span<'static>>,
    range: (u32, u32),
    bg: Color,
) -> Vec<Span<'static>> {
    let (start, end) = range;
    let mut out = Vec::with_capacity(spans.len());
    let mut pos = 0u32;
    #[expect(
        clippy::indexing_slicing,
        reason = "lo/hi derive from the clamped overlap [s,e) within this span, so 0 <= lo <= hi <= chars.len()"
    )]
    for sp in spans {
        let chars: Vec<char> = sp.content.chars().collect();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a single rendered line's char count is bounded by terminal width, far below u32::MAX"
        )]
        let len = chars.len() as u32;
        let span_start = pos;
        let span_end = pos + len;
        pos = span_end;
        if span_end <= start || span_start >= end || len == 0 {
            out.push(sp);
            continue;
        }
        let s = start.max(span_start);
        let e = end.min(span_end);
        let lo = (s - span_start) as usize;
        let hi = (e - span_start) as usize;
        let pre: String = chars[..lo].iter().collect();
        let mid: String = chars[lo..hi].iter().collect();
        let post: String = chars[hi..].iter().collect();
        if !pre.is_empty() {
            out.push(Span::styled(pre, sp.style));
        }
        if !mid.is_empty() {
            out.push(Span::styled(mid, sp.style.bg(bg)));
        }
        if !post.is_empty() {
            out.push(Span::styled(post, sp.style));
        }
    }
    out
}

pub(super) fn status_glyph(t: &Theme, s: FileStatus) -> (&'static str, Color) {
    match s {
        FileStatus::Added => ("A", t.added),
        FileStatus::Untracked => ("?", t.purple),
        FileStatus::Modified => ("M", t.warn),
        FileStatus::Deleted => ("D", t.removed),
        FileStatus::Renamed => ("R", t.hunk),
        FileStatus::Copied => ("C", t.hunk),
    }
}

/// Frame layout rectangles, computed once per frame from the terminal area. The
/// measure pass derives these (mutating nothing); the reconcile pass writes the
/// geometry-derived state onto `App`; the paint pass only renders.
pub struct Geometry {
    pub body: Rect,
    pub status: Rect,
    pub sidebar: Rect,
    pub stream: Rect,
    pub show_sidebar: bool,
}

/// Render a frame: measure the layout, reconcile the geometry-derived state onto
/// `App` (an isolated, explicit `&mut` step), then paint. Painting itself is a
/// pure function of `&App` + `Geometry` — it mutates nothing (see [`paint`]).
pub fn draw(frame: &mut Frame, app: &mut App) {
    let geo = measure(app, frame.area());
    reconcile(app, &geo);
    paint(frame, app, &geo);
}

/// Pure layout pass: split the terminal area into body/status and sidebar/stream.
/// Reads layout inputs from `app`, mutates nothing.
pub fn measure(app: &App, area: Rect) -> Geometry {
    let [body, status] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    let show_sidebar = app.sidebar_shown();
    let sidebar_w = if show_sidebar { app.sidebar_w } else { 0 };
    let [sidebar, stream] =
        Layout::horizontal([Constraint::Length(sidebar_w), Constraint::Min(0)]).areas(body);
    Geometry {
        body,
        status,
        sidebar,
        stream,
        show_sidebar,
    }
}

/// Write the frame-derived state back onto `App`: the sidebar hit-test rect, the
/// resolved layout, the viewport extents (clamped), the sidebar window, and the
/// peek viewport. This is the only `&mut` step in rendering; paint follows and
/// mutates nothing.
fn reconcile(app: &mut App, geo: &Geometry) {
    // Remember the sidebar geometry for mouse hit-testing.
    app.sidebar_area = geo.sidebar;
    app.set_layout(geo.stream.width);
    app.viewport_h = geo.stream.height.max(1) as usize;
    // The stack body starts one column in (see draw_stack); the usable text
    // width is what bounds horizontal scrolling.
    app.viewport_w = geo.stream.width.saturating_sub(1) as usize;
    app.clamp();
    if geo.show_sidebar {
        // `selected` is the active file in both panes: the sidebar cursor, the
        // last jump target, or the top-of-viewport file while scrolling. The
        // window only chases it when `reveal_selected` is set, so manual list
        // scrolls stick.
        app.update_sidebar_window(geo.sidebar.height as usize);
    }
    if app.peek_open() {
        // Inner height (minus the box borders) bounds the peek's page scrolling.
        app.peek_viewport_h = geo.body.height.saturating_sub(2) as usize;
    }
    if app.help_open() {
        // Same box math the painter uses, so the scroll clamp cannot drift from
        // what is actually drawn.
        app.help_viewport_h = super::overlays::help_body_rows(geo.body);
    }
    if app.commit_msg_open() {
        // The popup's visible body rows, derived from the same box math the
        // painter uses (one geometry source, so the "stop a page short" scroll
        // clamp can't drift from what is actually drawn).
        app.commit_msg_viewport_h = super::overlays::commit_msg_body_rows(geo.body, app);
    }
}

/// Pure paint pass: render the body, status, and the active overlay (selected by
/// `Mode`) from `&App`. Mutates nothing.
pub fn paint(frame: &mut Frame, app: &App, geo: &Geometry) {
    // Paint the whole frame with the active theme's canvas first, so every panel
    // renders on the theme background. Foreground-only spans preserve it (ratatui
    // patches only the color fields that are set), and panels that want their own
    // background (selection, diff add/del) override it explicitly.
    frame.render_widget(
        Block::default().style(Style::default().bg(app.theme.bg)),
        frame.area(),
    );
    if geo.show_sidebar {
        super::stream::draw_sidebar(frame, geo.sidebar, app);
    }
    if app.is_split() {
        super::stream::draw_split(frame, geo.stream, app);
    } else {
        super::stream::draw_stack(frame, geo.stream, app);
    }
    draw_status(frame, geo.status, app);

    if app.peek_open() {
        super::overlays::draw_peek(frame, geo.body, app);
    }
    if app.palette_open() {
        super::overlays::draw_palette(frame, geo.body, app);
    }
    if app.theme_picker_open() {
        super::overlays::draw_theme_picker(frame, geo.body, app);
    }
    if app.commit_msg_open() {
        super::overlays::draw_commit_message(frame, geo.body, app);
    }
    if app.thread_list().is_some() {
        super::overlays::draw_threads(frame, geo.body, app);
    }
    if app.submit_draft().is_some() {
        super::overlays::draw_submit(frame, geo.body, app);
    }
    if app.comment_input_state().is_some() {
        super::overlays::draw_comment(frame, geo.body, app);
    }
    if app.help_open() {
        super::overlays::draw_help(frame, geo.body, app);
    }
}

/// Scroll position as a percent of a plan with `rows` rows at viewport top
/// `scroll`.
fn scroll_pct(scroll: usize, rows: usize) -> usize {
    if rows <= 1 {
        100
    } else {
        (scroll * 100 / rows.saturating_sub(1)).min(100)
    }
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let fg = if t.dark { Color::Black } else { Color::White };
    let badge = Span::styled(
        format!(" {} ", app.cs().source),
        Style::default()
            .fg(fg)
            .bg(source_color(app))
            .add_modifier(Modifier::BOLD),
    );

    // Help is the one context with no bindings of its own — it just says how to
    // dismiss it.
    if app.help_open() {
        let spans = vec![
            badge,
            Span::styled(" help · key reference ", Style::default().fg(t.muted)),
            Span::styled("any key to close ", Style::default().fg(t.muted)),
        ];
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let mut spans = vec![
        badge,
        Span::styled(status_info(app), Style::default().fg(t.muted)),
    ];
    // Only the base (Normal) context lets a streaming load or a transient
    // flash preempt the key hints: every captured context (peek, popup,
    // palette, theme picker) shows its own bindings — the progress line's
    // "esc/q to cancel" would be a lie there (the router sends those keys to
    // the modal, and `q` would type into a fuzzy query).
    let overlay = app.active_context() != crate::tui::app::InputContext::Normal;
    if !overlay && app.show_progress() {
        let (done, total) = app.load_progress();
        spans.push(Span::styled(
            format!("diffing {done}/{total} · esc/q to cancel "),
            Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
        ));
    } else if let Some(flash) = app.flash.as_ref().filter(|_| !overlay) {
        spans.push(Span::styled(
            format!("{flash} "),
            Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
        ));
    } else {
        // The active view's registered bindings (see App::status_bindings).
        spans.extend(binding_spans(t, app.status_bindings()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The context text shown left of the key hints, selected by the same
/// [`App::active_context`] resolver that picks the bindings — so a new input
/// context forces a decision here too (this match is exhaustive), rather than
/// silently falling through to the base counters.
fn status_info(app: &App) -> String {
    use crate::tui::app::InputContext;
    use crate::tui::peek::PeekMode;
    match app.active_context() {
        InputContext::CommitMsg => app.commit_msg().map_or_else(String::new, |m| {
            format!(" commit · {} · {} ", m.msg.short, m.msg.author)
        }),
        InputContext::Peek => app.peek().map_or_else(String::new, |p| {
            let pct = scroll_pct(p.state.scroll, p.active_rows());
            // Blame's box title already carries the file + commit identity, so
            // the status line doesn't echo the label — just the position.
            if p.mode == PeekMode::Blame {
                format!(" peek · {pct}%  ")
            } else {
                format!(" peek · {} · {pct}%  ", p.label())
            }
        }),
        InputContext::Threads => app.thread_list().map_or_else(String::new, |l| {
            format!(" comments · {}/{} ", l.selected + 1, l.threads.len().max(1))
        }),
        InputContext::Comment => app
            .comment_input_state()
            .map_or_else(String::new, |c| format!(" {} ", c.title())),
        InputContext::Submit => app
            .submit_draft()
            .map_or_else(String::new, |d| format!(" {} ", d.title())),
        // Help never reaches here (draw_status early-returns for it); the
        // palette / theme picker overlay the base, whose counters stay shown.
        InputContext::Help
        | InputContext::Palette
        | InputContext::ThemePicker
        | InputContext::Normal => {
            let file_no = app.current_file() + 1;
            let total = app.cs().files.len().max(1);
            // Percentage tracks the layout actually on screen (split rows differ
            // from stack rows).
            let pct = scroll_pct(app.state().cursor_row, app.plan().rows.len());
            let mode = match app.layout {
                LayoutMode::Split => "split",
                LayoutMode::Stack => "stack",
            };
            // Reviewed count only for review sessions; browse views omit it.
            let reviewed = if app.is_review() {
                format!(" · ✓ {}/{}", app.viewed_count(), total)
            } else {
                String::new()
            };
            format!(" {mode} · {file_no}/{total}{reviewed} · {pct}%  ")
        }
    }
}

/// Render a binding table as status spans: each key in the context color, its
/// description muted, separated by `·`.
fn binding_spans(t: &Theme, bindings: &[keymap::Binding]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, bind) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(t.muted)));
        }
        if !bind.key.is_empty() {
            spans.push(Span::styled(bind.key, Style::default().fg(t.context)));
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(bind.desc, Style::default().fg(t.muted)));
    }
    spans.push(Span::raw(" "));
    spans
}

pub(super) fn display_path(f: &crate::model::DiffFile, max: usize) -> String {
    shorten_path(&f.path, max)
}

/// Abbreviate a directory to its first three chars plus `…` (e.g. `components` →
/// `com…`), but only when that actually saves width — a name of 4 or fewer chars
/// is kept whole (abbreviating it would be the same width, just uglier).
fn abbrev_dir(d: &str) -> String {
    if d.chars().count() > 4 {
        let head: String = d.chars().take(3).collect();
        format!("{head}…")
    } else {
        d.to_string()
    }
}

/// Left-elide a single string to `max` columns, keeping the tail.
fn left_elide(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if len <= max {
        s.to_string()
    } else {
        let tail: String = s.chars().skip(len - max.saturating_sub(1)).collect();
        format!("…{tail}")
    }
}

/// Shorten `path` to fit `max` columns, doing the least mangling that fits:
/// 1. if it already fits, it is returned whole;
/// 2. otherwise directories are abbreviated to two chars + `…` one at a time,
///    left to right, stopping the moment the path fits (so the directories
///    nearest the file stay readable longest);
/// 3. if abbreviating every directory still isn't enough, leading directories
///    are dropped (a leading `…/`), keeping those nearest the file;
/// 4. as a last resort the file name itself is left-elided.
pub(super) fn shorten_path(path: &str, max: usize) -> String {
    let max = max.max(4);
    if path.chars().count() <= max {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let Some((&file, dir_parts)) = parts.split_last() else {
        return path.to_string();
    };
    if dir_parts.is_empty() {
        return left_elide(file, max);
    }

    // Abbreviate directories left → right, one at a time, until it fits.
    let mut segs: Vec<String> = dir_parts.iter().map(|d| (*d).to_string()).collect();
    #[expect(
        clippy::indexing_slicing,
        reason = "i ranges over 0..segs.len() and segs has the same length as dir_parts"
    )]
    for i in 0..segs.len() {
        segs[i] = abbrev_dir(dir_parts[i]);
        let body = format!("{}/{file}", segs.join("/"));
        if body.chars().count() <= max {
            return body;
        }
    }
    // Still too long: drop leading directories, keeping those nearest the file.
    #[expect(
        clippy::indexing_slicing,
        reason = "drop ranges over 1..=segs.len(), so segs[drop..] is always a valid (possibly empty) slice"
    )]
    for drop in 1..=segs.len() {
        let kept = segs[drop..].join("/");
        let body = if kept.is_empty() {
            format!("…/{file}")
        } else {
            format!("…/{kept}/{file}")
        };
        if body.chars().count() <= max {
            return body;
        }
    }
    left_elide(file, max)
}

/// The file name (last path segment) for a header label.
pub(super) fn display_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Clip a string to at most `max` columns, appending `…` when truncated.
pub(super) fn left_clip(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if max == 0 {
        return String::new();
    }
    if n <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Changeset, DiffFile, Stats};
    use crate::tui::app::Focus;
    use crate::tui::theme::ThemeName;
    use crate::tui::view::ViewKind;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// A diffed file with a small real hunk (`a\nb\n` → `a\nc\n`).
    fn dfile(path: &str, status: FileStatus) -> DiffFile {
        let (hunks, additions, deletions) = crate::diff::compute_hunks("a\nb\n", "a\nc\n");
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
            old_text: Some("a\nb\n".into()),
            new_text: Some("a\nc\n".into()),
            content_digest: None,
            diffed: true,
        }
    }

    fn changeset(files: Vec<DiffFile>) -> Changeset {
        Changeset {
            source: "working tree".into(),
            files,
        }
    }

    /// Render the whole frame and flatten the buffer to a newline-joined string.
    fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| super::draw(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        let area = buf.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn status_glyph_covers_every_file_status() {
        let t = Theme::new(ThemeName::Dark);
        assert_eq!(status_glyph(&t, FileStatus::Added).0, "A");
        assert_eq!(status_glyph(&t, FileStatus::Untracked).0, "?");
        assert_eq!(status_glyph(&t, FileStatus::Modified).0, "M");
        assert_eq!(status_glyph(&t, FileStatus::Deleted).0, "D");
        assert_eq!(status_glyph(&t, FileStatus::Renamed).0, "R");
        assert_eq!(status_glyph(&t, FileStatus::Copied).0, "C");
        // The status colors differ between Added and Deleted (sanity on the map).
        assert_eq!(status_glyph(&t, FileStatus::Added).1, t.added);
        assert_eq!(status_glyph(&t, FileStatus::Deleted).1, t.removed);
    }

    #[test]
    fn left_elide_keeps_short_and_tails_long() {
        // Shorter than / equal to max → unchanged.
        assert_eq!(left_elide("abc", 5), "abc");
        assert_eq!(left_elide("abcdef", 6), "abcdef");
        // Longer than max → leading ellipsis + the last max-1 chars.
        let e = left_elide("abcdefghij", 5);
        assert!(e.starts_with('…'), "leading ellipsis: {e}");
        assert!(e.ends_with("ghij"), "keeps the tail: {e}");
        assert_eq!(e.chars().count(), 5, "elided to exactly max columns");
    }

    #[test]
    fn display_path_delegates_to_shorten_path() {
        let f = dfile("src/ui/components/panes/DiffPane.tsx", FileStatus::Modified);
        // Wide budget keeps the path whole.
        assert_eq!(display_path(&f, 80), "src/ui/components/panes/DiffPane.tsx");
        // Tight budget shortens it but keeps the file name.
        let short = display_path(&f, 20);
        assert!(short.chars().count() <= 20, "fits the budget: {short}");
        assert!(short.ends_with("DiffPane.tsx"), "file name kept: {short}");
    }

    #[test]
    fn shorten_path_left_elides_a_long_bare_filename() {
        // No directories: the lone filename is left-elided to fit (dir_parts empty).
        let s = shorten_path("supercalifragilisticexpialidocious.txt", 10);
        assert_eq!(s.chars().count(), 10);
        assert!(s.starts_with('…'), "elided filename: {s}");
    }

    #[test]
    fn shorten_path_falls_back_to_eliding_the_filename() {
        // Even after abbreviating + dropping every directory, a long file name must
        // itself be elided (the final last-resort branch).
        let s = shorten_path("aa/bb/cc/verylongfilenamehere.txt", 10);
        assert_eq!(s.chars().count(), 10);
        assert!(s.starts_with('…'), "filename elided as last resort: {s}");
        assert!(s.ends_with("txt"), "tail of the filename kept: {s}");
    }

    #[test]
    fn source_color_is_local_blue_then_commit_green() {
        let cs = changeset(vec![dfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs); // launches as a local view
        assert_eq!(source_color(&app), app.theme.local);
        // A pushed commit view flips the source accent.
        app.push_test_view(&cs, ViewKind::Commit("abc123".into()), false);
        assert_eq!(source_color(&app), app.theme.commit);
    }

    #[test]
    fn status_bar_shows_layout_file_count_and_source() {
        let cs = changeset(vec![
            dfile("a.rs", FileStatus::Modified),
            dfile("b.rs", FileStatus::Added),
        ]);
        let mut app = App::new(&cs);
        let text = render(&mut app, 90, 12);
        assert!(text.contains("stack"), "stack layout in status: {text}");
        assert!(text.contains("1/2"), "file counter in status: {text}");
        assert!(text.contains("working tree"), "source badge: {text}");
    }

    #[test]
    fn status_bar_split_mode_with_sidebar_focus() {
        let cs = changeset(vec![dfile("a.rs", FileStatus::Modified)]);
        let mut app = App::with_mode(&cs, LayoutMode::Split);
        app.set_focus(Focus::Sidebar);
        let text = render(&mut app, 110, 12);
        assert!(text.contains("split"), "split layout in status: {text}");
    }

    #[test]
    fn status_bar_review_shows_reviewed_counter() {
        let cs = changeset(vec![dfile("a.rs", FileStatus::Modified)]);
        let mut app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            None,
            ViewKind::Local,
            true,
            None,
            None,
        );
        let text = render(&mut app, 90, 12);
        assert!(
            text.contains('✓'),
            "review session shows a viewed count: {text}"
        );
    }

    #[test]
    fn status_bar_flash_preempts_hints() {
        let cs = changeset(vec![dfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.flash = Some("3 hidden in folded dirs".into());
        let text = render(&mut app, 90, 12);
        assert!(text.contains("3 hidden"), "flash cue shown: {text}");
    }

    #[test]
    fn status_bar_peek_reports_peek_context() {
        let cs = changeset(vec![dfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.open_peek_preview();
        let text = render(&mut app, 90, 12);
        assert!(text.contains("peek"), "peek status line: {text}");
    }

    #[test]
    fn status_bar_commit_message_reports_its_context() {
        use crate::model::CommitMessage;
        use crate::tui::app::{CommitMsg, Overlay};
        let cs = changeset(vec![dfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.mode
            .push_overlay(Overlay::CommitMessage(CommitMsg::new(CommitMessage {
                sha: "abc1234deadbeef".into(),
                short: "abc1234".into(),
                author: "Tester".into(),
                date: "2026-06-30".into(),
                body: "Summary".into(),
            })));
        let text = render(&mut app, 90, 12);
        assert!(
            text.contains("commit ·"),
            "commit-message status line: {text}"
        );
        assert!(text.contains("abc1234"), "short sha in status: {text}");
    }

    #[test]
    fn status_bar_help_reports_dismiss() {
        let cs = changeset(vec![dfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.toggle_help();
        let text = render(&mut app, 90, 12);
        assert!(text.contains("help"), "help status line: {text}");
    }

    #[test]
    fn status_bar_theme_picker_uses_theme_hints() {
        let cs = changeset(vec![dfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.open_theme_picker();
        // Rendering with the picker open exercises the theme-hint branch.
        let _ = render(&mut app, 90, 12);
    }

    #[test]
    fn scroll_pct_tracks_the_given_row_count() {
        // Empty/single-row plans read as fully scrolled.
        assert_eq!(scroll_pct(0, 0), 100);
        assert_eq!(scroll_pct(0, 1), 100);
        // Top, middle, and bottom of a real plan.
        assert_eq!(scroll_pct(0, 101), 0);
        assert_eq!(scroll_pct(50, 101), 50);
        assert_eq!(scroll_pct(100, 101), 100);
        // The same scroll yields different percents for different layouts — which
        // is the split-vs-stack bug this fixes (caller passes the active rows).
        assert_ne!(scroll_pct(20, 41), scroll_pct(20, 101));
    }

    #[test]
    fn keeps_path_whole_when_it_fits() {
        // Plenty of room → no abbreviation at all.
        assert_eq!(
            shorten_path("src/ui/components/panes/DiffPane.tsx", 60),
            "src/ui/components/panes/DiffPane.tsx"
        );
        assert_eq!(shorten_path("src/main.rs", 60), "src/main.rs");
        assert_eq!(shorten_path("README.md", 60), "README.md");
    }

    #[test]
    fn abbreviates_left_to_right_only_as_needed() {
        // 37 cols; at 30 only the leftmost long dir needs abbreviating — the dirs
        // nearest the file (and the file) stay whole.
        let s = shorten_path("src/ui/components/panes/DiffPane.tsx", 30);
        assert!(s.chars().count() <= 30, "fits: {s}");
        assert!(
            s.ends_with("panes/DiffPane.tsx"),
            "rightmost dir + file kept whole: {s}"
        );
        assert!(
            s.contains("com…"),
            "the long leading dir is abbreviated: {s}"
        );
        assert!(
            !s.contains("pan…"),
            "the dir nearest the file is not abbreviated yet: {s}"
        );
    }

    #[test]
    fn drops_leading_dirs_keeps_near_file() {
        let s = shorten_path("src/ui/components/panes/DiffPane.tsx", 20);
        assert!(s.starts_with("…/"), "leading dirs dropped: {s}");
        assert!(s.ends_with("DiffPane.tsx"), "filename kept: {s}");
        assert!(s.contains("pan…"), "dir nearest the file kept: {s}");
        assert!(s.chars().count() <= 20);
    }
}
