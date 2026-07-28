//! The diff body: the navigation sidebar plus the windowed review stream in
//! both stack and split layouts, and the per-row line builders.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::model::{FileStatus, LineKind};
use crate::tui::app::{App, Focus};
use crate::tui::rows::{Row, SplitCell};
use crate::tui::theme::Theme;

use super::frame::{
    display_path, emphasize, resolve, rgb, shorten_path, source_color, status_glyph,
};

#[expect(
    clippy::too_many_lines,
    reason = "single-pass sidebar render: header, grouped file rows, and footer share one set of window/selection locals"
)]
pub(super) fn draw_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    use crate::tui::sidebar::{Grouping, SidebarRow};
    let t = &app.theme;
    let src = source_color(app);
    let focused = app.focus() == Focus::Sidebar;
    let border = if focused { t.accent } else { t.muted };
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sel_file = app.state().selected_file();
    let sel_dir = app.state().selected_dir.clone();
    let width = inner.width as usize;
    let grouped = app.grouping == Grouping::ByDir;
    let rows = app.sidebar_rows();
    let top = app.sidebar_top;
    let end = (top + app.sidebar_visible).min(rows.len());
    #[expect(
        clippy::indexing_slicing,
        reason = "sidebar_top is clamped to the row count by update_sidebar_window, so top <= end <= rows.len()"
    )]
    let win = &rows[top..end];
    // The jump digits spread across the visible *files* only (directory lines and
    // folded placeholders are skipped), so this must match `sidebar::digit_target`.
    let n_files = win.iter().filter(|r| r.file().is_some()).count();
    let mut file_off = 0usize;
    let mut lines = Vec::new();
    for row in win {
        let i = match row {
            // A directory line: the combined parent path, shortened. Root is
            // `./`. Informative chrome — no marker or selection of its own (the
            // placeholder below carries the fold state). Uses `muted` without DIM,
            // which would fade it into the background on light themes.
            SidebarRow::Dir(dir) => {
                let label = if dir.is_empty() {
                    "./".to_string()
                } else {
                    format!("{}/", shorten_path(dir, width.saturating_sub(2)))
                };
                lines.push(Line::from(Span::styled(
                    label,
                    Style::default().fg(t.muted),
                )));
                continue;
            }
            // A folded directory's placeholder: a selectable `▸ N files` line, or
            // `▸ Y/N files` once some of its files are reviewed.
            SidebarRow::CollapsedFiles { dir, n } => {
                let selected = sel_dir.as_deref() == Some(dir.as_str());
                let marker = if selected { "▌" } else { " " };
                let reviewed = app
                    .cs()
                    .files
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| crate::model::parent_dir(&f.path) == dir.as_str())
                    .filter(|(i, _)| app.state().viewed.get(*i).copied().unwrap_or(false))
                    .count();
                let count = if reviewed > 0 {
                    format!("{reviewed}/{n}")
                } else {
                    n.to_string()
                };
                let label = format!("  ▸ {count} file{}", if *n == 1 { "" } else { "s" });
                let mut line = Line::from(vec![
                    Span::styled(marker, Style::default().fg(src)),
                    Span::styled(label, Style::default().fg(t.muted)),
                ]);
                if selected {
                    let bg = if focused { t.sel_focus_bg } else { t.sel_bg };
                    line = line.style(Style::default().bg(bg).add_modifier(Modifier::BOLD));
                }
                lines.push(line);
                continue;
            }
            SidebarRow::File(i) => *i,
        };
        let is_sel = sel_file == Some(i);
        #[expect(
            clippy::indexing_slicing,
            reason = "i is a SidebarRow::File index built from the same file list, so it is in bounds"
        )]
        let f = &app.cs().files[i];
        let viewed = app.state().viewed.get(i).copied().unwrap_or(false);
        let (status_g, status_c) = status_glyph(t, f.status);
        let (glyph, color) = if viewed {
            ("✓", t.added)
        } else {
            (status_g, status_c)
        };
        // Grouped mode shows the basename (the directory is in the header);
        // flat mode shows the shortened full path.
        let name = if grouped {
            shorten_path(crate::model::file_name(&f.path), width.saturating_sub(10))
        } else {
            display_path(f, width.saturating_sub(10))
        };
        // An undiffed file's stats aren't known yet — show a placeholder until
        // its background diff lands. Additions/deletions are colored like the
        // diff body (green/red); a viewed file dims them with its name.
        let (adds, dels) = if f.diffed {
            (
                format!("+{}", f.stats.additions),
                format!("-{}", f.stats.deletions),
            )
        } else {
            ("+?".to_string(), "-?".to_string())
        };
        let (add_fg, del_fg) = if viewed {
            (t.muted, t.muted)
        } else {
            (t.added, t.removed)
        };
        // The selected file always carries the source-colored block (matching
        // the diff header and the status badge), focused or not.
        let marker = if is_sel { "▌" } else { " " };
        // A sparse jump digit spread across the visible files (1=first, 9=last).
        let badge = match crate::tui::sidebar::offset_to_digit(file_off, n_files) {
            Some(d) => format!("{d} "),
            None => "  ".to_string(),
        };
        file_off += 1;
        let name_style = if viewed {
            Style::default().fg(t.muted).add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(t.context)
        };
        let spans = vec![
            Span::styled(marker, Style::default().fg(src)),
            Span::styled(
                badge,
                Style::default().fg(t.muted).add_modifier(Modifier::DIM),
            ),
            Span::styled(
                format!("{glyph} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(name, name_style),
            Span::raw(" "),
            Span::styled(adds, Style::default().fg(add_fg)),
            Span::raw(" "),
            Span::styled(dels, Style::default().fg(del_fg)),
        ];
        let mut line = Line::from(spans);
        if is_sel {
            let bg = if focused { t.sel_focus_bg } else { t.sel_bg };
            line = line.style(Style::default().bg(bg).add_modifier(Modifier::BOLD));
        }
        lines.push(line);
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Glyph marking the line cursor's row, drawn in the one column both layouts
/// leave blank.
const CURSOR_MARK: &str = "\u{258e}";

/// Draw the line-cursor marker in column `area.x`.
///
/// Both `draw_stack` and `draw_split` inset their content to `area.x + 1`, so
/// that column is free — which is why the cursor is a marker rather than a row
/// background. A background cannot work here: the highlighter's spans patch over
/// a line-level style, split builds its background per span with no line-level
/// mechanism at all, and an empty split half and the column divider have no
/// background path to set.
///
/// `sticky` shifts the marker down one, because the pinned file header occupies
/// drawn line 0 without being a plan row.
/// Drawn line the cursor marker belongs on, or `None` when it is off screen.
///
/// Split out from the painting so the arithmetic — including the sticky-header
/// shift, which is the off-by-one `lines.len()` would produce — is reachable
/// without a `Frame`.
fn marker_line(cursor_row: usize, scroll: usize, sticky: bool, height: usize) -> Option<usize> {
    let rel = cursor_row.checked_sub(scroll)?;
    let line = rel + usize::from(sticky);
    (line < height).then_some(line)
}

fn draw_cursor_marker(frame: &mut Frame, area: Rect, app: &App, sticky: bool) {
    let st = app.state();
    // With `wrap` on, one plan row spends several drawn lines, so `cursor_row -
    // scroll` is no longer the drawn line. Painting the marker anyway points at
    // an unrelated continuation line, which is worse than not showing it —
    // `wrap` is out of scope for the cursor's visibility guarantee.
    if st.wrap {
        return;
    }
    if let Some(line) = marker_line(st.cursor_row, st.scroll, sticky, area.height as usize) {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "marker_line guarantees line < area.height, itself a u16"
        )]
        let rect = Rect {
            x: area.x,
            y: area.y + line as u16,
            width: 1,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(CURSOR_MARK).style(Style::default().fg(app.theme.accent)),
            rect,
        );
    }
}

pub(super) fn draw_stack(frame: &mut Frame, area: Rect, app: &App) {
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    if app.plan().rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No changes to review.").style(Style::default().fg(app.theme.muted)),
            inner,
        );
        return;
    }
    let height = inner.height as usize;
    let mut lines = Vec::with_capacity(height);

    // Pin the current file's header at the top once it has scrolled off. The
    // current file's row is found via its visible ordinal (folds aside); no
    // visible file → no sticky header.
    let cf = app.current_file();
    let cf_row = app
        .plan()
        .visible_ordinal(cf)
        .and_then(|o| app.plan().file_starts.get(o).copied());
    let sticky = cf_row.is_some_and(|r| app.state().scroll > r);
    let content_height = if sticky {
        lines.push(file_header_line(app, cf));
        height.saturating_sub(1)
    } else {
        height
    };

    let end = (app.state().scroll + content_height).min(app.plan().rows.len());
    #[expect(
        clippy::indexing_slicing,
        reason = "scroll is clamped by app.clamp() and end = min(scroll+h, len), so scroll <= end <= len"
    )]
    let window = &app.plan().rows[app.state().scroll..end];
    for row in window {
        lines.push(render_row(app, row, app.state().h_scroll, None, None));
    }
    // Horizontal panning is applied per-line to the body (gutter stays fixed),
    // so the paragraph itself is not scrolled.
    let para = Paragraph::new(lines);
    let para = if app.state().wrap {
        para.wrap(Wrap { trim: false })
    } else {
        para
    };
    frame.render_widget(para, inner);
    draw_cursor_marker(frame, area, app, sticky);
}

pub(super) fn draw_split(frame: &mut Frame, area: Rect, app: &App) {
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    if app.plan().rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No changes to review.").style(Style::default().fg(app.theme.muted)),
            inner,
        );
        return;
    }
    let col_w = (inner.width as usize).saturating_sub(1) / 2;
    let height = inner.height as usize;
    let mut lines = Vec::with_capacity(height);

    // Pin the current file's header at the top once it has scrolled off.
    let cf = app.current_file();
    let cf_row = app
        .plan()
        .visible_ordinal(cf)
        .and_then(|o| app.plan().file_starts.get(o).copied());
    let sticky = cf_row.is_some_and(|r| app.state().scroll > r);
    let content_height = if sticky {
        lines.push(file_header_line(app, cf));
        height.saturating_sub(1)
    } else {
        height
    };

    let end = (app.state().scroll + content_height).min(app.plan().rows.len());
    #[expect(
        clippy::indexing_slicing,
        reason = "scroll is clamped by app.clamp() and end = min(scroll+h, len), so scroll <= end <= len"
    )]
    let window = &app.plan().rows[app.state().scroll..end];
    for row in window {
        lines.push(split_row_line(app, row, col_w, app.state().h_scroll, None));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    draw_cursor_marker(frame, area, app, sticky);
}

/// Render one side-by-side row. `hl_override` keys highlighting to the peek slot.
/// (Only `Pair`/chrome rows occur in a split plan; the stacked `Line` variant is
/// rendered blank for exhaustiveness and never reached here.)
pub(super) fn split_row_line<'a>(
    app: &'a App,
    row: &'a Row,
    col_w: usize,
    h_scroll: usize,
    hl_override: Option<usize>,
) -> Line<'a> {
    match row {
        Row::FileHeader(fi) => file_header_line(app, *fi),
        Row::Collapsed(n) => collapsed_line(&app.theme, *n),
        Row::CollapsedDir { dir, n, reviewed } => {
            collapsed_dir_line(&app.theme, dir, *n, *reviewed)
        }
        Row::Pending => pending_line(app),
        Row::Banner(text) => banner_line(&app.theme, text),
        Row::HunkHeader(n) => hunk_header_line(&app.theme, *n),
        Row::Spacer | Row::Line { .. } => Line::from(""),
        Row::Pair(l, r) => {
            // Each side pans within its own column by the same offset.
            let mut spans = cell_spans(app, l.as_ref(), col_w, h_scroll, hl_override);
            spans.push(Span::styled("│", Style::default().fg(app.theme.muted)));
            spans.extend(cell_spans(app, r.as_ref(), col_w, h_scroll, hl_override));
            Line::from(spans)
        }
    }
}

/// The syntax-highlighted (or plain) body spans for one line/side. `hl_idx` is
/// the highlight-cache key (the file index, or the peek's reserved slot).
fn body_spans(
    app: &App,
    hl_idx: usize,
    side_new: bool,
    lineno: Option<u32>,
    text: &str,
    kind: LineKind,
) -> Vec<Span<'static>> {
    let t = &app.theme;
    let hl = app
        .hl
        .get(hl_idx)
        .and_then(|fh| lineno.and_then(|n| fh.line(side_new, n)));
    match hl {
        Some(parts) if !parts.is_empty() => parts
            .iter()
            .map(|p| {
                Span::styled(
                    p.text.clone(),
                    Style::default().fg(rgb(resolve(p.paint, app))),
                )
            })
            .collect(),
        _ => {
            let fg = match kind {
                LineKind::Context => t.context,
                LineKind::Added => t.added,
                LineKind::Removed => t.removed,
            };
            vec![Span::styled(text.to_string(), Style::default().fg(fg))]
        }
    }
}

/// Truncate spans to `width` columns and pad the remainder (with `bg`).
fn clamp_pad(spans: Vec<Span<'static>>, width: usize, bg: Option<Color>) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for sp in spans {
        if used >= width {
            break;
        }
        let chars: Vec<char> = sp.content.chars().collect();
        if used + chars.len() <= width {
            used += chars.len();
            out.push(sp);
        } else {
            let take = width - used;
            #[expect(
                clippy::indexing_slicing,
                reason = "this branch runs only when used < width <= used + chars.len(), so take = width-used is in 1..=chars.len()"
            )]
            let s: String = chars[..take].iter().collect();
            used += take;
            out.push(Span::styled(s, sp.style));
            break;
        }
    }
    if used < width {
        let mut st = Style::default();
        if let Some(bg) = bg {
            st = st.bg(bg);
        }
        out.push(Span::styled(" ".repeat(width - used), st));
    }
    out
}

/// Drop the first `n` display columns from a span list (for horizontal panning).
fn skip_cols(spans: Vec<Span<'static>>, n: usize) -> Vec<Span<'static>> {
    if n == 0 {
        return spans;
    }
    let mut remaining = n;
    let mut out = Vec::with_capacity(spans.len());
    for sp in spans {
        let len = sp.content.chars().count();
        if remaining >= len {
            remaining -= len;
            continue;
        }
        if remaining > 0 {
            let kept: String = sp.content.chars().skip(remaining).collect();
            out.push(Span::styled(kept, sp.style));
            remaining = 0;
        } else {
            out.push(sp);
        }
    }
    out
}

/// Build styled spans for one split cell, panned by `h_scroll` and clamped/padded
/// to `width` columns.
fn cell_spans(
    app: &App,
    cell: Option<&SplitCell>,
    width: usize,
    h_scroll: usize,
    hl_override: Option<usize>,
) -> Vec<Span<'static>> {
    let t = &app.theme;
    let Some(c) = cell else {
        return vec![Span::raw(" ".repeat(width))];
    };

    let bg = match c.kind {
        LineKind::Added => Some(t.add_bg),
        LineKind::Removed => Some(t.del_bg),
        LineKind::Context => None,
    };
    let with_bg = |st: Style| if let Some(bg) = bg { st.bg(bg) } else { st };

    let num = c.lineno.map(|n| n.to_string()).unwrap_or_default();
    let (sign, sign_color) = match c.kind {
        LineKind::Added => ('+', t.added),
        LineKind::Removed => ('-', t.removed),
        LineKind::Context => (' ', t.muted),
    };
    let mut spans = vec![
        Span::styled(format!("{num:>4} "), with_bg(Style::default().fg(t.muted))),
        Span::styled(sign.to_string(), with_bg(Style::default().fg(sign_color))),
    ];

    let hl_idx = hl_override.unwrap_or(c.file);
    let mut body: Vec<Span<'static>> =
        body_spans(app, hl_idx, c.side_new, c.lineno, &c.text, c.kind)
            .into_iter()
            .map(|s| {
                let content = s.content.into_owned();
                Span::styled(content, with_bg(s.style))
            })
            .collect();
    if let Some(r) = c.emphasis {
        let emph = if c.kind == LineKind::Removed {
            t.del_emph
        } else {
            t.add_emph
        };
        body = emphasize(body, r, emph);
    }
    // The gutter (line number + sign) stays fixed; only the code pans.
    spans.extend(skip_cols(body, h_scroll));

    clamp_pad(spans, width, bg)
}

fn file_header_line(app: &App, fi: usize) -> Line<'static> {
    let t = &app.theme;
    #[expect(
        clippy::indexing_slicing,
        reason = "fi is a file index originating from the same changeset's file list, so it is in bounds"
    )]
    let f = &app.cs().files[fi];
    // The active file (the one `v` toggles) gets an accent bar + brighter header.
    let active = app.state().selected_file() == Some(fi);
    let (glyph, color) = status_glyph(t, f.status);
    let title = match (&f.previous_path, f.status) {
        (Some(prev), FileStatus::Renamed | FileStatus::Copied) => format!("{prev} → {}", f.path),
        _ => f.path.clone(),
    };
    let text_fg = t.context;
    let bg = if active { t.sel_bg } else { t.header_bg };
    let marker = if active { "▌" } else { " " };
    // Additions/deletions colored like the diff body (green/red).
    let (adds, dels) = if f.diffed {
        (
            format!("+{}", f.stats.additions),
            format!("-{}", f.stats.deletions),
        )
    } else {
        ("+?".to_string(), "-?".to_string())
    };
    Line::from(vec![
        Span::styled(marker, Style::default().fg(source_color(app))),
        Span::styled(
            format!("{glyph} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            title,
            Style::default().fg(text_fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {adds}"), Style::default().fg(t.added)),
        Span::styled(format!(" {dels}"), Style::default().fg(t.removed)),
    ])
    .style(Style::default().bg(bg))
}

fn collapsed_line(t: &Theme, n: usize) -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "  ⋯ viewed · {n} hunk{} hidden",
            if n == 1 { "" } else { "s" }
        ),
        Style::default().fg(t.muted).add_modifier(Modifier::DIM),
    ))
}

/// A folded directory's body placeholder: `▸ src/api — N files hidden`, or
/// `Y/N files` once some are reviewed, with a `✓` cue when the whole directory is.
fn collapsed_dir_line(t: &Theme, dir: &str, n: usize, reviewed: usize) -> Line<'static> {
    let label = if dir.is_empty() {
        "./".to_string()
    } else {
        format!("{dir}/")
    };
    let fully = n > 0 && reviewed == n;
    let mark = if fully { "✓" } else { "▸" };
    let fg = if fully { t.added } else { t.muted };
    let count = if reviewed > 0 {
        format!("{reviewed}/{n}")
    } else {
        n.to_string()
    };
    Line::from(Span::styled(
        format!(
            "  {mark} {label} — {count} file{} hidden",
            if n == 1 { "" } else { "s" }
        ),
        Style::default().fg(fg).add_modifier(Modifier::DIM),
    ))
}

/// Placeholder body for an undiffed file: a subtle marker, plus the load count
/// once the progress threshold has passed (so fast loads stay indicator-free).
fn pending_line(app: &App) -> Line<'static> {
    let t = &app.theme;
    let text = if app.show_progress() {
        let (done, total) = app.load_progress();
        format!("  ⋯ diffing… {done}/{total}")
    } else {
        "  ⋯".to_string()
    };
    Line::from(Span::styled(
        text,
        Style::default().fg(t.muted).add_modifier(Modifier::DIM),
    ))
}

/// A hunk boundary in the interactive view: a dim `⋯` gap marker (matching the
/// collapsed/pending placeholders). The machine-readable `@@ -a,b +c,d @@` ranges
/// are redundant here — the gutter already numbers every line — so they are shown
/// only in the lazygit-compatible pager/external renderers (`src/render.rs`).
fn hunk_header_line(t: &Theme, lineno: u32) -> Line<'static> {
    // Align the `⋯` under the *left* edge of the surrounding line numbers. The
    // gutter right-aligns numbers in a min-4-wide field (`{num:>4}`), so a d-digit
    // number's leftmost digit sits at column `field_w − d + 1`; place the `⋯` there
    // and pad the field out so the following gutter columns still line up.
    let d = lineno.to_string().len();
    let field_w = d.max(4);
    let left = field_w - d;
    let marker = format!("{}⋯{}", " ".repeat(left), " ".repeat(field_w - left - 1));
    Line::from(Span::styled(
        marker,
        Style::default().fg(t.muted).add_modifier(Modifier::DIM),
    ))
}

/// A commit-message banner line: a commit-accent left rule plus the text, so the
/// message reads as a quoted block distinct from the diff body below it.
fn banner_line(t: &Theme, text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("▎ ", Style::default().fg(t.commit)),
        Span::styled(text.to_string(), Style::default().fg(t.context)),
    ])
}

/// Render one unified row. `h_scroll` is the horizontal pan of the view that
/// owns the row (the main stream's state, or the peek's own) — the gutter stays
/// fixed and only the code body pans. `gutter`, when supplied, replaces the
/// line-number + sign prefix for a `Row::Line` (the blame attribution gutter), so
/// blame is the shared renderer with a different gutter rather than a fork.
pub(super) fn render_row<'a>(
    app: &'a App,
    row: &'a Row,
    h_scroll: usize,
    hl_override: Option<usize>,
    gutter: Option<Vec<Span<'a>>>,
) -> Line<'a> {
    let t = &app.theme;
    match row {
        Row::FileHeader(fi) => file_header_line(app, *fi),
        Row::HunkHeader(n) => hunk_header_line(t, *n),
        Row::Collapsed(n) => collapsed_line(t, *n),
        Row::CollapsedDir { dir, n, reviewed } => collapsed_dir_line(t, dir, *n, *reviewed),
        Row::Pending => pending_line(app),
        Row::Banner(text) => banner_line(t, text),
        #[expect(
            clippy::match_same_arms,
            reason = "Spacer renders blank; the identical Pair arm below is kept separate for its exhaustiveness comment"
        )]
        Row::Spacer => Line::from(""),
        Row::Line {
            file,
            kind,
            old,
            new,
            text,
            emphasis,
        } => {
            let hl_idx = hl_override.unwrap_or(*file);
            let (sign, sign_color, bg) = match kind {
                LineKind::Added => ('+', t.added, Some(t.add_bg)),
                LineKind::Removed => ('-', t.removed, Some(t.del_bg)),
                LineKind::Context => (' ', t.muted, None),
            };
            let (num, lineno, side_new) = match kind {
                LineKind::Removed => (old.map(|n| n.to_string()).unwrap_or_default(), *old, false),
                _ => (new.map(|n| n.to_string()).unwrap_or_default(), *new, true),
            };

            let mut body = body_spans(app, hl_idx, side_new, lineno, text, *kind);
            if let Some(r) = emphasis {
                let emph = if *kind == LineKind::Removed {
                    t.del_emph
                } else {
                    t.add_emph
                };
                body = emphasize(body, *r, emph);
            }
            // The gutter (line number + sign, or a supplied blame gutter) stays
            // fixed; only the code body pans.
            body = skip_cols(body, h_scroll);

            let mut spans = gutter.unwrap_or_else(|| {
                vec![
                    Span::styled(format!("{num:>4} "), Style::default().fg(t.muted)),
                    Span::styled(sign.to_string(), Style::default().fg(sign_color)),
                ]
            });
            spans.extend(body);

            let mut line = Line::from(spans);
            if let Some(bg) = bg {
                line = line.style(Style::default().bg(bg));
            }
            line
        }
        // Split `Pair` rows never occur in a stacked plan; blank for exhaustiveness.
        Row::Pair(..) => Line::from(""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_line_shifts_for_the_pinned_header_and_reports_off_screen() {
        // Cursor three rows below the viewport top, no pinned header.
        assert_eq!(marker_line(13, 10, false, 20), Some(3));
        // With the header pinned it occupies drawn line 0, so everything shifts.
        assert_eq!(marker_line(13, 10, true, 20), Some(4));
        // Off the bottom of the drawn area.
        assert_eq!(marker_line(40, 10, false, 20), None);
        // Exactly one past the last drawn line.
        assert_eq!(marker_line(30, 10, false, 20), None);
        assert_eq!(marker_line(29, 10, false, 20), Some(19));
        // Above the viewport top — clamp keeps this from happening, but the
        // painter must not underflow if it ever does.
        assert_eq!(marker_line(2, 10, false, 20), None);
    }

    use crate::model::{Changeset, DiffFile, LayoutMode, Stats};
    use crate::tui::theme::ThemeName;
    use crate::tui::view::ViewKind;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    /// A diffed file with a small real hunk.
    fn dfile(path: &str) -> DiffFile {
        let (hunks, additions, deletions) = crate::diff::compute_hunks("a\nb\n", "a\nc\n");
        DiffFile {
            path: path.into(),
            previous_path: None,
            status: FileStatus::Modified,
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

    /// A diffed file with many lines, so the viewport can scroll past its header.
    fn bigfile(path: &str) -> DiffFile {
        use std::fmt::Write as _;
        let mut old = String::new();
        for i in 0..40 {
            writeln!(old, "line{i}").unwrap();
        }
        let new: String = (0..40)
            .map(|i| {
                if i == 5 {
                    format!("changed{i}\n")
                } else {
                    format!("line{i}\n")
                }
            })
            .collect();
        let (hunks, additions, deletions) = crate::diff::compute_hunks(&old, &new);
        DiffFile {
            path: path.into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks,
            stats: Stats {
                additions,
                deletions,
            },
            language: None,
            is_binary: false,
            old_text: Some(old),
            new_text: Some(new),
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

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| super::super::draw(f, app)).unwrap();
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
    fn split_row_line_renders_every_row_variant() {
        // Call the per-row builder directly to hit every match arm (several of
        // these never occur in a real split plan but exist for exhaustiveness).
        let cs = changeset(vec![dfile("a.rs")]);
        let app = App::with_mode(&cs, LayoutMode::Split);
        let _ = split_row_line(&app, &Row::FileHeader(0), 20, 0, None);
        let _ = split_row_line(&app, &Row::Collapsed(2), 20, 0, None);
        let _ = split_row_line(
            &app,
            &Row::CollapsedDir {
                dir: "src".into(),
                n: 2,
                reviewed: 1,
            },
            20,
            0,
            None,
        );
        let _ = split_row_line(&app, &Row::Pending, 20, 0, None);
        let _ = split_row_line(&app, &Row::HunkHeader(1), 20, 0, None);
        let _ = split_row_line(&app, &Row::Spacer, 20, 0, None);
        let _ = split_row_line(
            &app,
            &Row::Line {
                file: 0,
                kind: LineKind::Added,
                old: None,
                new: Some(1),
                text: "x".into(),
                emphasis: None,
            },
            20,
            0,
            None,
        );
        // A full Pair with both sides + intra-line emphasis + horizontal pan.
        let left = SplitCell {
            file: 0,
            side_new: false,
            lineno: Some(1),
            kind: LineKind::Removed,
            text: "old text".into(),
            emphasis: Some((0, 3)),
        };
        let right = SplitCell {
            file: 0,
            side_new: true,
            lineno: Some(1),
            kind: LineKind::Added,
            text: "new text".into(),
            emphasis: Some((0, 3)),
        };
        let row = Row::Pair(Some(left), Some(right));
        let line = split_row_line(&app, &row, 20, 1, None);
        assert!(!line.spans.is_empty(), "the pair row produced spans");
    }

    #[test]
    fn stack_empty_plan_shows_placeholder() {
        let cs = changeset(vec![]);
        let mut app = App::new(&cs);
        let text = render(&mut app, 60, 10);
        assert!(text.contains("No changes to review"), "empty stack: {text}");
    }

    #[test]
    fn split_empty_plan_shows_placeholder() {
        let cs = changeset(vec![]);
        let mut app = App::with_mode(&cs, LayoutMode::Split);
        let text = render(&mut app, 80, 10);
        assert!(text.contains("No changes to review"), "empty split: {text}");
    }

    #[test]
    fn hunk_gap_marker_aligns_to_the_surrounding_number_width() {
        let t = Theme::new(ThemeName::Dark);
        let m = |n| {
            hunk_header_line(&t, n)
                .spans
                .iter()
                .map(|sp| sp.content.as_ref())
                .collect::<String>()
        };
        // The `⋯` sits at the leftmost digit column of the (right-aligned) number.
        assert_eq!(m(5), "   ⋯", "1-digit → column 4");
        assert_eq!(m(23), "  ⋯ ", "2-digit → column 3");
        assert_eq!(m(123), " ⋯  ", "3-digit → column 2");
        assert_eq!(m(1234), "⋯   ", "4-digit → column 1");
        assert_eq!(m(12345), "⋯    ", "5-digit → field expands, column 1");
    }

    #[test]
    fn sidebar_colors_additions_green_and_deletions_red() {
        let cs = changeset(vec![dfile("a.rs")]); // +1 / -1
        let mut app = App::new(&cs);
        let mut term = Terminal::new(TestBackend::new(70, 12)).unwrap();
        term.draw(|f| super::super::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let sidebar_w = app.sidebar_w;
        // Within the sidebar columns, the +N stat uses the added color and -N the
        // removed color (the diff body lives to the right of the sidebar).
        let in_sidebar =
            |want| (0..buf.area.height).any(|y| (0..sidebar_w).any(|x| buf[(x, y)].fg == want));
        assert!(in_sidebar(app.theme.added), "additions colored green");
        assert!(in_sidebar(app.theme.removed), "deletions colored red");
    }

    #[test]
    fn stack_renders_diff_lines() {
        let cs = changeset(vec![dfile("a.rs")]);
        let mut app = App::new(&cs);
        let text = render(&mut app, 70, 12);
        assert!(text.contains("a.rs"), "file header rendered: {text}");
    }

    #[test]
    fn stack_sticky_header_and_wrap() {
        let cs = changeset(vec![bigfile("src/big.rs")]);
        let mut app = App::new(&cs);
        app.toggle_wrap(); // exercise the wrap branch
        let _ = render(&mut app, 40, 8); // establish the viewport
        app.state_mut().scroll = 20; // scroll the header off-screen
        app.clamp();
        let text = render(&mut app, 40, 8);
        assert!(
            text.contains("big.rs"),
            "the sticky header keeps the file path on screen: {text}"
        );
    }

    #[test]
    fn split_renders_diff_columns() {
        let cs = changeset(vec![dfile("a.rs")]);
        let mut app = App::with_mode(&cs, LayoutMode::Split);
        let text = render(&mut app, 80, 12);
        assert!(text.contains("a.rs"), "file header rendered: {text}");
    }

    #[test]
    fn split_sticky_header_over_pairs() {
        let cs = changeset(vec![bigfile("src/big.rs")]);
        let mut app = App::with_mode(&cs, LayoutMode::Split);
        let _ = render(&mut app, 90, 8);
        app.state_mut().scroll = 15;
        app.clamp();
        let text = render(&mut app, 90, 8);
        assert!(text.contains("big.rs"), "split sticky header: {text}");
    }

    /// Build an app showing `rev`'s diff as undiffed stubs against the crate's own
    /// repo, with the loader not yet started.
    fn stub_app(rev: &str) -> Option<(App, Vec<crate::git::FileStub>)> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let req = crate::git::LoadRequest::Show { rev: rev.into() };
        let en = crate::git::enumerate(&dir, &req).ok()?;
        if en.stubs.is_empty() {
            return None;
        }
        let cs = Changeset {
            source: en.source.clone(),
            files: en
                .stubs
                .iter()
                .map(crate::git::FileStub::as_stub_file)
                .collect(),
        };
        let app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir),
            ViewKind::Commit(rev.into()),
            false,
            None,
            Some(req),
        );
        Some((app, en.stubs))
    }

    #[test]
    fn pending_line_shows_diffing_count_when_progress_is_active() {
        use std::time::{Duration, Instant};
        let cs = changeset(vec![dfile("a.rs")]);
        let mut app = App::new(&cs);
        // Force the "progress past threshold" state deterministically: a present
        // loader plus a load that started well before the chrome delay.
        app.session.loader = Some(crate::tui::loader::Loader::start(
            std::path::PathBuf::new(),
            Vec::new(),
        ));
        app.session.load_started = Instant::now().checked_sub(Duration::from_millis(200));
        assert!(app.show_progress(), "progress chrome is active");
        let line = pending_line(&app);
        let s: String = line.spans.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(s.contains("diffing"), "the progress branch renders: {s}");
    }

    #[test]
    fn commit_view_renders_message_banner() {
        // A commit launch carries a message banner; its accent rule renders atop
        // the stream (exercises the Banner row arm + banner_line).
        let Some((mut app, _)) = stub_app("HEAD") else {
            return;
        };
        let text = render(&mut app, 100, 30);
        assert!(
            text.contains('▎'),
            "commit-message banner rule rendered: {text}"
        );
    }

    #[test]
    fn loading_stream_shows_progress_chrome() {
        // A live load past the progress threshold renders the diffing count both in
        // the pending placeholders (`pending_line`) and in the status bar.
        let Some((mut app, stubs)) = stub_app("HEAD") else {
            return;
        };
        app.begin_load(stubs, false);
        std::thread::sleep(std::time::Duration::from_millis(150));
        // Do NOT drain — the files stay undiffed so the plan keeps its Pending rows.
        if !app.show_progress() {
            return; // be lenient on timing / empty job sets
        }
        let text = render(&mut app, 100, 20);
        assert!(text.contains("diffing"), "progress chrome shown: {text}");
    }
}
