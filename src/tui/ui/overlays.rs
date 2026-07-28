//! Popups layered over the body: the single-file peek, the help reference, the
//! fuzzy command palette, and the live-preview theme picker.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};
use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::blame::{self, GUTTER_W};
use crate::tui::keymap::{self, HelpSection};
use crate::tui::peek::Peek;
use crate::tui::rows::Row;
use crate::tui::theme::Theme;

use super::frame::{display_name, left_clip, status_glyph};
use super::stream::{render_row, split_row_line};

/// The peek's on-screen row window: the plan rows from `scroll` down, with the
/// top clamped into range (so a scroll parked past the end still shows the last
/// rows). Shared by the diff/content and blame render paths.
fn visible_rows(plan: &crate::tui::rows::Plan, scroll: usize) -> &[Row] {
    let start = scroll.min(plan.rows.len().saturating_sub(1));
    #[expect(
        clippy::indexing_slicing,
        reason = "start is clamped to rows.len()-1 (or 0 when empty), so start <= rows.len()"
    )]
    &plan.rows[start..]
}

/// The single-file peek overlay: a bordered box over the body showing the
/// peeked file's content or diff, scrolled independently.
pub(super) fn draw_peek(frame: &mut Frame, body: Rect, app: &App) {
    use crate::tui::peek::PeekMode;
    let Some(p) = app.peek() else { return };
    let t = &app.theme;
    let accent = if p.origin_local { t.local } else { t.commit };

    frame.render_widget(Clear, body);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(t.bg))
        .title(Span::styled(
            format!(" {} ", p.label()),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(body);
    frame.render_widget(block, body);

    if p.is_empty() {
        let msg = match p.mode {
            // Blame fetches its content on the worker; show progress, not a
            // premature "No content.", while it runs.
            PeekMode::Blame if p.blame_loading() => "Loading…",
            PeekMode::Content | PeekMode::Blame => "No content.",
            PeekMode::Diff => "No differences.",
        };
        frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(t.muted)),
            inner,
        );
        return;
    }

    let h = inner.height as usize;
    // Blame mode draws the whole-file content with a per-line attribution gutter
    // in place of the line-number gutter; everything else uses the shared rows.
    if p.mode == PeekMode::Blame {
        draw_blame_body(frame, inner, app, p, h);
        return;
    }

    let hl = Some(crate::tui::app::PEEK_HL);
    // The plan is already built for the active layout (unified or split). Suppress
    // the file header and the `@@` hunk headers — the box title carries the file
    // and its real +/- stat, so the whole-file hunk span (misleading in full mode)
    // is dropped. A split row is a `Pair`; everything else renders unified.
    let col_w = (inner.width as usize).saturating_sub(1) / 2;
    let mut lines: Vec<Line> = Vec::with_capacity(h);
    for row in visible_rows(&p.plan, p.state.scroll) {
        if lines.len() >= h {
            break;
        }
        match row {
            Row::FileHeader(_) | Row::HunkHeader(_) => {}
            Row::Pair(..) => lines.push(split_row_line(app, row, col_w, 0, hl)),
            other => lines.push(render_row(app, other, p.state.h_scroll, hl, None)),
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Render the blame body: each content line goes through the shared row renderer
/// with a 12-col `name + age` gutter (collapsed across runs of the same commit,
/// painted per-commit) plus a vertical rule in place of the line-number gutter.
/// The cursor line's full identity is in the box title, so the gutter omits the
/// sha and blanks continuation lines.
#[expect(
    clippy::many_single_char_names,
    reason = "p/h/b/i are the conventional peek/geometry/blame-line names for this render"
)]
fn draw_blame_body(frame: &mut Frame, inner: Rect, app: &App, p: &Peek, h: usize) {
    use std::sync::Arc;
    let t = &app.theme;
    let blame_lines = p.blame();
    let now = now_secs();
    let mut lines: Vec<Line> = Vec::with_capacity(h);
    // The commit of the previous *rendered* row, so a run start is measured
    // against what's actually on screen — not the file line above, which may have
    // scrolled off (leaving the whole visible run blank). `None` for the first
    // visible row forces it to be a run start, so attribution always shows.
    let mut prev_commit: Option<&Arc<crate::model::BlameCommit>> = None;
    for row in visible_rows(&p.plan, p.state.scroll) {
        if lines.len() >= h {
            break;
        }
        // Whole-file content rows are unified `Line`s with a new line number;
        // headers/spacers are suppressed (the title carries the file).
        let Row::Line {
            new: Some(lineno), ..
        } = row
        else {
            continue;
        };
        let i = (*lineno as usize).saturating_sub(1);
        let b = blame_lines.get(i);
        // A run starts where the commit differs from the previous visible row — a
        // pointer compare on the shared per-commit handle (O(viewport), zero-cost),
        // never an O(file-lines) table per paint.
        let run_start = match (prev_commit, b) {
            (Some(prev), Some(cur)) => !Arc::ptr_eq(prev, &cur.commit),
            _ => true,
        };
        prev_commit = b.map(|bl| &bl.commit);
        let token = match b {
            Some(b) if run_start => blame::gutter_token(
                &b.commit.author,
                &blame::relative_age(now, b.commit.time_secs),
                GUTTER_W,
            ),
            _ => blame::blank_gutter(GUTTER_W),
        };
        let color = b.map_or(t.muted, |b| blame_color(t, b.commit.color_key));
        let gutter = vec![
            Span::styled(token, Style::default().fg(color)),
            Span::styled(" │ ", Style::default().fg(t.muted)),
        ];
        lines.push(render_row(
            app,
            row,
            p.state.h_scroll,
            Some(crate::tui::app::PEEK_HL),
            Some(gutter),
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Map a commit's stable color key onto a small spread of theme accents, so each
/// run reads as one colored block and adjacent runs differ.
fn blame_color(t: &Theme, key: u64) -> ratatui::style::Color {
    let palette = [t.commit, t.local, t.warn, t.hunk, t.purple, t.added];
    // `key % len` is < len (6), so the conversion and index are always valid.
    let idx = usize::try_from(key % palette.len() as u64).unwrap_or(0);
    #[expect(
        clippy::indexing_slicing,
        reason = "idx = key % palette.len(), always in bounds"
    )]
    let c = palette[idx];
    c
}

/// Current wall-clock time as unix seconds, for relative-age rendering.
fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    #[expect(
        clippy::cast_possible_wrap,
        reason = "seconds since the epoch stay far below i64::MAX for any realistic clock"
    )]
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    secs
}

/// Render one column's sections into styled lines, keys padded to `key_w`.
fn help_column(t: &Theme, sections: &[HelpSection], key_w: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (i, (section, items)) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            *section,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )));
        for (key, desc) in *items {
            lines.push(Line::from(vec![
                Span::styled(format!("{key:<key_w$}"), Style::default().fg(t.context)),
                Span::styled((*desc).to_string(), Style::default().fg(t.muted)),
            ]));
        }
    }
    lines
}

/// Center a `w`×`h` popup in `body` (vertically at 1/`y_div` of the free
/// space), clear it, and draw the accent-bordered frame; returns the inner
/// rect. One frame recipe for every popup. Callers size with floor-then-cap
/// (`max` before `min`) — never `Ord::clamp`, whose min > max asserts on a
/// terminal narrower than the floor.
#[expect(
    clippy::too_many_arguments,
    reason = "the arguments are the popup frame's full visual parameter set, shared by four popups"
)]
fn popup_frame(
    frame: &mut Frame,
    body: Rect,
    w: u16,
    h: u16,
    y_div: u16,
    accent: ratatui::style::Color,
    bg: ratatui::style::Color,
    padding: Padding,
    title: Line<'_>,
) -> Rect {
    let x = body.x + (body.width.saturating_sub(w)) / 2;
    let y = body.y + (body.height.saturating_sub(h)) / y_div.max(1);
    let area = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(bg))
        .padding(padding)
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Paint a popup's reserved last inner row with a dim `muted` hint (on `bg`
/// when the popup owns its canvas); returns the content rect above it.
fn footer_hint(
    frame: &mut Frame,
    inner: Rect,
    muted: ratatui::style::Color,
    bg: Option<ratatui::style::Color>,
    text: &str,
) -> Rect {
    let foot = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    let mut para = Paragraph::new(Span::styled(
        text,
        Style::default().fg(muted).add_modifier(Modifier::DIM),
    ));
    if let Some(bg) = bg {
        para = para.style(Style::default().bg(bg));
    }
    frame.render_widget(para, foot);
    Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    }
}

// Column geometry: key column + widest description, plus a little slack.
const HELP_LEFT_W: u16 = 34;
const HELP_RIGHT_W: u16 = 24;
const HELP_GUTTER: u16 = 4;
const HELP_PAD: u16 = 2;

pub(super) fn draw_help(frame: &mut Frame, body: Rect, app: &App) {
    let t = &app.theme;
    let left = help_column(t, keymap::HELP_LEFT, 13);
    let right = help_column(t, keymap::HELP_RIGHT, 8);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "help rows are a small fixed key-reference list, well below u16::MAX"
    )]
    let content_rows = left.len().max(right.len()) as u16;

    // Box sized to its content (with padding), centered, not full-width.
    let inner_w = HELP_LEFT_W + HELP_GUTTER + HELP_RIGHT_W;
    let w = (inner_w + 2 * HELP_PAD + 2)
        .max(24)
        .min(body.width.saturating_sub(2));
    // content + borders(2) + top pad(1) + blank(1) + footer(1)
    let h = (content_rows + 5).max(6).min(body.height.saturating_sub(2));
    let inner = popup_frame(
        frame,
        body,
        w,
        h,
        4,
        t.accent,
        t.bg,
        Padding::new(HELP_PAD, HELP_PAD, 1, 0),
        Span::styled(
            " Help ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )
        .into(),
    );

    let content = footer_hint(frame, inner, t.muted, None, "any key to close");
    let [lcol, rcol] = Layout::horizontal([
        Constraint::Length(HELP_LEFT_W),
        Constraint::Min(HELP_RIGHT_W),
    ])
    .spacing(HELP_GUTTER)
    .areas(content);
    frame.render_widget(Paragraph::new(left), lcol);
    frame.render_widget(Paragraph::new(right), rcol);
}

#[expect(
    clippy::many_single_char_names,
    reason = "p/t/w/h/x/y/c are the conventional palette + rect-geometry names for this popup layout"
)]
#[expect(
    clippy::too_many_lines,
    reason = "single-pass palette popup render: input row, result list, and footer share one set of layout locals"
)]
pub(super) fn draw_palette(frame: &mut Frame, body: Rect, app: &App) {
    use crate::tui::app::PaletteKind;
    let Some(p) = app.palette() else { return };
    let t = &app.theme;
    let commits = matches!(p.kind, PaletteKind::Commits { .. });
    let accent = if commits { t.commit } else { t.accent };

    // Floor-then-cap (`max` before `min`), not `Ord::clamp` — clamp asserts
    // min <= max and would panic on a terminal narrower than the floor.
    let w = (body.width.saturating_mul(7) / 10)
        .max(30)
        .min(body.width.saturating_sub(2));
    let max_rows = 14u16.min(body.height.saturating_sub(2));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "match counts are bounded by the changeset's file/commit count, far below u16::MAX"
    )]
    let list_rows = (p.matches.len() as u16 + 1).min(max_rows).max(1);
    let h = list_rows + 2;
    let title = match &p.kind {
        PaletteKind::Files => " jump to file ".to_string(),
        PaletteKind::Commits {
            scoped_path: Some(path),
            truncated,
            ..
        } => {
            let cap = if *truncated { " (200+)" } else { "" };
            format!(" history · {}{cap} ", display_name(path))
        }
        PaletteKind::Commits { truncated, .. } => {
            let cap = if *truncated { " · 200+" } else { "" };
            format!(" pick commit · {}{cap} ", p.mode_hint)
        }
    };
    let inner = popup_frame(
        frame,
        body,
        w,
        h,
        3,
        accent,
        t.bg,
        Padding::new(1, 1, 0, 0),
        Span::styled(
            title,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )
        .into(),
    );

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("> ", Style::default().fg(accent)),
        Span::styled(p.query.clone(), Style::default().fg(t.context)),
        Span::styled("▏", Style::default().fg(accent)),
    ]));

    let visible = inner.height.saturating_sub(1) as usize;
    let offset = p.selected.saturating_sub(visible.saturating_sub(1));
    for (row, &mi) in p.matches.iter().enumerate().skip(offset).take(visible) {
        // First nine matches show a press-to-pick number.
        let num = if row < 9 {
            format!("{} ", row + 1)
        } else {
            "  ".to_string()
        };
        let mut spans = vec![Span::styled(
            num,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )];
        match &p.kind {
            PaletteKind::Files => {
                #[expect(
                    clippy::indexing_slicing,
                    reason = "mi is a file-match index into the same changeset's file list, so it is in bounds"
                )]
                let f = &app.cs().files[mi];
                let (glyph, color) = status_glyph(t, f.status);
                spans.push(Span::styled(
                    format!("{glyph} "),
                    Style::default().fg(color),
                ));
                spans.push(Span::styled(f.path.clone(), Style::default().fg(t.context)));
            }
            PaletteKind::Commits { commits, .. } => {
                if let Some(c) = commits.get(mi) {
                    spans.push(Span::styled(
                        format!("{} ", c.short),
                        Style::default().fg(t.commit),
                    ));
                    let avail = inner.width as usize;
                    let summ = left_clip(
                        &c.summary,
                        avail.saturating_sub(c.short.len() + c.date.len() + 6),
                    );
                    spans.push(Span::styled(summ, Style::default().fg(t.context)));
                    spans.push(Span::styled(
                        format!("  {}", c.date),
                        Style::default().fg(t.muted),
                    ));
                }
            }
        }
        let mut line = Line::from(spans);
        if row == p.selected {
            line = line.style(
                Style::default()
                    .bg(t.sel_focus_bg)
                    .add_modifier(Modifier::BOLD),
            );
        }
        lines.push(line);
    }
    if p.matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matches",
            Style::default().fg(t.muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The commit-message popup's box height for a `body_lines`-line message in
/// `body`: tall enough for ~10 lines even when short, capped to the body, with
/// a saturated u16 conversion so a pathologically long body (65k+ lines) can't
/// overflow. One formula, consumed by both the painter and the scroll
/// reconcile (via [`commit_msg_body_rows`]) so they cannot drift.
fn commit_msg_height(body: Rect, body_lines: usize) -> u16 {
    let visible = body_lines.max(10);
    // borders(2) + body + footer(1) + slack. `visible >= 10` already forces this
    // to >= 14, so no explicit floor is needed (unlike popup_frame's `.max(5)`,
    // whose input can be 0); cap to the body so the box never exceeds its rect —
    // otherwise commit_msg_body_rows (the scroll clamp) would overstate the rows.
    let want_h = u16::try_from(visible.saturating_add(4)).unwrap_or(u16::MAX);
    want_h.min(body.height.saturating_sub(2))
}

/// The popup's visible body rows — its box height minus borders (2) and the
/// footer row (1) — which bounds the scroll so the last page stays full.
pub(super) fn commit_msg_body_rows(body: Rect, app: &App) -> usize {
    app.commit_msg().map_or(1, |m| {
        usize::from(commit_msg_height(body, m.body_lines))
            .saturating_sub(3)
            .max(1)
    })
}

/// The shared commit-message popup: a centered, scrollable box showing the
/// commit's short sha · author · date in the title and its full body below.
pub(super) fn draw_commit_message(frame: &mut Frame, body: Rect, app: &App) {
    let Some(m) = app.commit_msg() else { return };
    let t = &app.theme;
    let accent = t.commit;

    // Floor at 20 columns, but never wider than the body allows.
    let w = (body.width.saturating_mul(8) / 10)
        .max(20)
        .min(body.width.saturating_sub(2));
    let body_lines = m.body_lines;
    let h = commit_msg_height(body, body_lines);
    let inner = popup_frame(
        frame,
        body,
        w,
        h,
        3,
        accent,
        t.bg,
        Padding::new(1, 1, 0, 0),
        Span::styled(
            format!(" {} ", m.msg.identity()),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )
        .into(),
    );

    // The footer hint owns the last inner row; window the body to the rest.
    let content = footer_hint(
        frame,
        inner,
        t.muted,
        None,
        &keymap::to_hint(keymap::BIND_COMMITMSG),
    );
    let start = m.scroll.min(body_lines.saturating_sub(1));
    let lines: Vec<Line> = m
        .msg
        .body
        .lines()
        .skip(start)
        .take(content.height as usize)
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(t.context))))
        .collect();
    frame.render_widget(Paragraph::new(lines), content);
}

/// The review's threads: one row each, with where the anchor sits now.
pub(super) fn draw_threads(frame: &mut Frame, body: Rect, app: &App) {
    let Some(list) = app.thread_list() else {
        return;
    };
    let t = &app.theme;
    let accent = t.accent;
    let w = (body.width.saturating_mul(8) / 10)
        .max(30)
        .min(body.width.saturating_sub(2));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a review's thread count is far below u16::MAX; clamped to the body height"
    )]
    let rows = (list.threads.len() as u16).clamp(1, body.height.saturating_sub(4));
    let inner = popup_frame(
        frame,
        body,
        w,
        rows.saturating_add(2),
        3,
        accent,
        t.bg,
        Padding::new(1, 1, 0, 0),
        Line::from(Span::styled(
            format!(" review points · {} ", list.threads.len()),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
    );

    // Window the list so the selection stays visible.
    let h = inner.height as usize;
    let top = list.selected.saturating_sub(h.saturating_sub(1));
    let lines: Vec<Line> = list
        .threads
        .iter()
        .enumerate()
        .skip(top)
        .take(h)
        .map(|(i, th)| {
            let where_ = th.path.as_deref().map_or_else(
                || "review".to_string(),
                |p| match th.placement.as_ref().and_then(|r| r.line()) {
                    Some(n) => format!("{p}:{n}"),
                    None => p.to_string(),
                },
            );
            let spans = vec![
                Span::styled(
                    format!("{:<9} ", th.state_label()),
                    Style::default().fg(if th.resolved { t.muted } else { accent }),
                ),
                Span::styled(format!("{where_}  "), Style::default().fg(t.muted)),
                Span::styled(th.body.clone(), Style::default().fg(t.context)),
            ];
            let line = Line::from(spans);
            if i == list.selected {
                line.style(Style::default().bg(t.sel_focus_bg))
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The comment input: a titled one-line box over whatever summoned it.
pub(super) fn draw_comment(frame: &mut Frame, body: Rect, app: &App) {
    let Some(input) = app.comment_input_state() else {
        return;
    };
    let t = &app.theme;
    let accent = t.accent;
    let w = (body.width.saturating_mul(8) / 10)
        .max(20)
        .min(body.width.saturating_sub(2));
    let inner = popup_frame(
        frame,
        body,
        w,
        4,
        3,
        accent,
        t.bg,
        Padding::new(1, 1, 0, 0),
        Line::from(Span::styled(
            format!(" {} ", input.title()),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
    );
    // Keep the caret in view: show the tail of a buffer wider than the box, so a
    // long comment is not typed blind with the cursor off the end.
    let room = (inner.width as usize).saturating_sub(1);
    let shown: String = {
        let chars: Vec<char> = input.buffer.chars().collect();
        chars
            .get(chars.len().saturating_sub(room)..)
            .unwrap_or_default()
            .iter()
            .collect()
    };
    // A block cursor after the text, so an empty input still shows where typing
    // will land.
    let line = Line::from(vec![
        Span::styled(shown, Style::default().fg(t.context)),
        Span::styled("\u{2588}", Style::default().fg(accent)),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
    // A refusal renders in the box's own footer; the status bar hides the flash
    // while an overlay is up.
    if let Some(msg) = input.refusal.as_deref() {
        let row = Rect {
            y: inner.y.saturating_add(1),
            height: 1,
            ..inner
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg.to_string(),
                Style::default().fg(t.warn),
            ))),
            row,
        );
    }
}

/// The live-preview theme picker: a centered grid of theme names. The cursor
/// cell is highlighted; the whole UI behind the popup already shows the
/// previewed theme. The grid windows vertically so the selection stays visible.
#[expect(
    clippy::many_single_char_names,
    reason = "p/t/w/h/x/y/r/c/i are the conventional picker + rect/grid-geometry names for this popup layout"
)]
pub(super) fn draw_theme_picker(frame: &mut Frame, body: Rect, app: &App) {
    use crate::tui::app::THEME_CELL_W;
    use crate::tui::theme::themes_by_brightness;
    let Some(p) = app.theme_picker() else { return };
    let t = &app.theme;
    let list = themes_by_brightness(p.dark_tab);

    let cols = app.theme_picker_cols();
    let count = list.len();
    let rows = app.theme_picker_rows();

    #[expect(
        clippy::cast_possible_truncation,
        reason = "cols * THEME_CELL_W is the popup's column width in cells, bounded by the terminal width"
    )]
    let inner_w = (cols * THEME_CELL_W) as u16;
    let w = (inner_w + 2).max(10).min(body.width);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "grid row count is bounded by the theme list size, far below u16::MAX"
    )]
    let h = (rows as u16 + 4).max(5).min(body.height);
    // The popup owns its background (the previewed theme's canvas) so the list
    // text contrasts even when the terminal's default background differs. The
    // title doubles as the tab indicator: the active brightness is emphasized.
    let tab = |on: bool| {
        if on {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.muted)
        }
    };
    let title = Line::from(vec![
        Span::styled(
            " theme · ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("dark", tab(p.dark_tab)),
        Span::styled(" / ", Style::default().fg(t.muted)),
        Span::styled("light", tab(!p.dark_tab)),
        Span::raw(" "),
    ]);
    let inner = popup_frame(frame, body, w, h, 3, t.accent, t.bg, Padding::ZERO, title);

    // Reserve the last inner row for the footer hint, then window the grid so the
    // selected row stays visible when the popup is shorter than the full grid.
    // The grid is column-major: cell (row r, col c) holds theme `c * rows + r`,
    // so walking down a column flows into the top of the next.
    let grid_h = (inner.height.saturating_sub(1) as usize).max(1);
    let sel_row = p.selected % rows.max(1);
    let top = sel_row
        .saturating_sub(grid_h - 1)
        .min(rows.saturating_sub(grid_h));
    let mut lines: Vec<Line> = Vec::with_capacity(grid_h);
    for r in top..(top + grid_h).min(rows) {
        let mut spans: Vec<Span> = Vec::with_capacity(cols);
        for c in 0..cols {
            let i = c * rows + r;
            if i >= count {
                continue;
            }
            #[expect(
                clippy::indexing_slicing,
                reason = "the guard above ensures i < count == list.len()"
            )]
            let name = list[i];
            let label = left_clip(name.display(), THEME_CELL_W.saturating_sub(2));
            let mut style = Style::default().fg(t.context);
            if i == p.selected {
                style = style.bg(t.sel_focus_bg).add_modifier(Modifier::BOLD);
            }
            let cell = format!(" {label:<w$}", w = THEME_CELL_W.saturating_sub(1));
            spans.push(Span::styled(cell, style));
        }
        lines.push(Line::from(spans));
    }

    // Paint both regions with the theme background so unwritten cells (gaps,
    // padding) don't fall back to the terminal default.
    let grid = footer_hint(
        frame,
        inner,
        t.muted,
        Some(t.bg),
        &keymap::to_hint(keymap::BIND_THEME),
    );
    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(t.bg)), grid);
}

#[cfg(test)]
mod tests {
    // --- draw_palette render coverage ---------------------------------------

    use crate::model::{Changeset, CommitInfo, DiffFile, FileStatus, Stats};
    use crate::tui::app::{App, Overlay, Palette, PaletteKind};
    use crate::tui::view::ViewKind;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    /// A minimal diffed file; the palette only reads `path` and `status`.
    fn pfile(path: &str, status: FileStatus) -> DiffFile {
        let (hunks, additions, deletions) = crate::diff::compute_hunks("a\n", "a\nb\n");
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
            old_text: Some("a\n".into()),
            new_text: Some("a\nb\n".into()),
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

    fn commit(short: &str, summary: &str, date: &str) -> CommitInfo {
        CommitInfo {
            id: format!("{short}deadbeef0000"),
            short: short.into(),
            summary: summary.into(),
            author: "Tester".into(),
            date: date.into(),
        }
    }

    /// Render `app` to a `TestBackend` buffer of the given size.
    fn render_buf(app: &mut App, w: u16, h: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| super::super::draw(f, app)).unwrap();
        term.backend().buffer().clone()
    }

    /// Flatten a buffer to a newline-joined string of its symbols.
    fn buf_text(buf: &Buffer) -> String {
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
    fn palette_files_lists_matches_and_highlights_selection() {
        let cs = changeset(vec![
            pfile("src/auth.rs", FileStatus::Modified),
            pfile("README.md", FileStatus::Untracked),
            pfile("src/main.rs", FileStatus::Added),
        ]);
        let mut app = App::new(&cs);
        app.open_palette();
        let buf = render_buf(&mut app, 74, 16);
        let text = buf_text(&buf);

        assert!(text.contains("jump to file"), "files palette title: {text}");
        assert!(text.contains("src/auth.rs"), "first file listed");
        assert!(text.contains("README.md"), "second file listed");
        // First nine matches carry a press-to-pick number.
        assert!(text.contains('1'), "row numbers rendered");

        // The selected row (index 0) carries the focus-selection background.
        let sel_bg = app.theme.sel_focus_bg;
        let highlighted =
            (0..buf.area.height).any(|y| (0..buf.area.width).any(|x| buf[(x, y)].bg == sel_bg));
        assert!(highlighted, "selected row is highlighted");
    }

    #[test]
    fn commit_message_popup_survives_narrow_terminals_and_huge_bodies() {
        use crate::model::CommitMessage;
        use crate::tui::app::CommitMsg;
        let cs = changeset(vec![pfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.mode
            .push_overlay(Overlay::CommitMessage(CommitMsg::new(CommitMessage {
                sha: "d".repeat(40),
                short: "ddddddd".into(),
                author: "Tester".into(),
                date: "2026-01-01".into(),
                body: "subject\n\nbody line".into(),
            })));
        // The old width math used Ord::clamp(20, width-2), which panics below
        // 22 columns; narrow panes must render (clipped), not crash.
        for w in [21u16, 12, 5] {
            let _ = render_buf(&mut app, w, 10);
        }
        // A pathologically long body must not overflow the u16 height math
        // (the old `visible as u16 + 4` panicked in debug builds at ~65k lines).
        if let Some(Overlay::CommitMessage(m)) = app.mode.overlay_mut() {
            m.msg.body = "x\n".repeat(70_000);
        }
        let _ = render_buf(&mut app, 80, 24);
        // The regular size still shows the identity title.
        if let Some(Overlay::CommitMessage(m)) = app.mode.overlay_mut() {
            m.msg.body = "subject".into();
        }
        let text = buf_text(&render_buf(&mut app, 74, 16));
        assert!(text.contains("Tester"), "popup title rendered: {text}");
    }

    #[test]
    fn modal_bindings_preempt_the_progress_line() {
        let cs = changeset(vec![pfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.session.loader = Some(crate::tui::loader::Loader::start(
            std::path::PathBuf::new(),
            Vec::new(),
        ));
        app.session.load_started = std::time::Instant::now()
            .checked_sub(crate::tui::app::LOAD_PROGRESS_DELAY * 2)
            .or(Some(std::time::Instant::now()));
        assert!(app.show_progress());
        // Normal context: the progress line preempts the key hints.
        let text = buf_text(&render_buf(&mut app, 74, 16));
        assert!(
            text.contains("diffing"),
            "progress in the base view: {text}"
        );
        // Palette context: the modal's own bindings win — advertising
        // "esc/q to cancel" would be a lie (q types into the fuzzy query).
        app.open_palette();
        let text = buf_text(&render_buf(&mut app, 74, 16));
        assert!(
            !text.contains("diffing"),
            "palette hints replace the progress line: {text}"
        );
    }

    #[test]
    fn popup_scroll_viewport_matches_the_drawn_box() {
        use crate::model::CommitMessage;
        use crate::tui::app::CommitMsg;
        let cs = changeset(vec![pfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.mode
            .push_overlay(Overlay::CommitMessage(CommitMsg::new(CommitMessage {
                sha: "s".into(),
                short: "s".into(),
                author: String::new(),
                date: String::new(),
                body: (1..=10)
                    .map(|i| format!("l{i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            })));
        // Tiny body: floor-then-cap caps the box to body.height-2 = 4 (never
        // exceeding the body), so body_rows = 4 - 3 = 1 — honest against what
        // is actually drawn, not the old h=5 that overshot the 6-row body.
        let tiny = ratatui::layout::Rect::new(0, 0, 40, 6);
        assert_eq!(super::commit_msg_height(tiny, 10), 4, "capped to body-2");
        assert_eq!(super::commit_msg_body_rows(tiny, &app), 1);
        let tall = ratatui::layout::Rect::new(0, 0, 40, 30);
        // Uncapped: want_h = 10.max(10) + 4 = 14 → 11 body rows.
        assert_eq!(super::commit_msg_height(tall, 10), 14);
        assert_eq!(super::commit_msg_body_rows(tall, &app), 11);
    }

    #[test]
    fn palette_survives_a_narrow_terminal() {
        // Same Ord::clamp hazard as the popup (min 30): a pane under 32 columns
        // must render, not panic.
        let cs = changeset(vec![pfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.open_palette();
        for w in [31u16, 20, 5] {
            let _ = render_buf(&mut app, w, 10);
        }
    }

    #[test]
    fn palette_files_filters_on_query() {
        let cs = changeset(vec![
            pfile("src/auth.rs", FileStatus::Modified),
            pfile("README.md", FileStatus::Untracked),
        ]);
        let mut app = App::new(&cs);
        app.open_palette();
        for c in "auth".chars() {
            app.palette_input(c);
        }
        let text = buf_text(&render_buf(&mut app, 74, 16));
        assert!(text.contains('>'), "query input prompt rendered");
        assert!(text.contains("auth"), "typed query echoed");
        assert!(text.contains("src/auth.rs"), "matching file kept");
        assert!(
            !text.contains("README.md"),
            "non-matching file filtered out"
        );
    }

    #[test]
    fn palette_files_shows_no_matches() {
        let cs = changeset(vec![pfile("src/auth.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.open_palette();
        for c in "zzzznope".chars() {
            app.palette_input(c);
        }
        let text = buf_text(&render_buf(&mut app, 74, 16));
        // No fuzzy match for "zzzznope" -> the result list is empty, but the
        // popup still renders its input row with the echoed query.
        assert!(text.contains('>'), "input prompt rendered: {text}");
        assert!(
            text.contains("zzzznope"),
            "typed query echoed with no matches: {text}"
        );
    }

    #[test]
    fn palette_files_windows_to_keep_selection_visible() {
        // More files than fit, with the selection driven to the bottom so the
        // visible window scrolls (offset > 0) and rows past nine render blanks.
        let files = (0..12)
            .map(|i| pfile(&format!("src/file{i:02}.rs"), FileStatus::Modified))
            .collect();
        let cs = changeset(files);
        let mut app = App::new(&cs);
        app.open_palette();
        for _ in 0..11 {
            app.palette_move(1);
        }
        // A short popup forces a small visible window so the list must scroll.
        let text = buf_text(&render_buf(&mut app, 74, 8));
        assert!(
            text.contains("src/file11.rs"),
            "bottom selection visible: {text}"
        );
        assert!(
            !text.contains("src/file00.rs"),
            "top scrolled out of window"
        );
    }

    /// Build an app and drop a ready-made commit palette over it directly — the
    /// real opener needs a live git repo, but `draw_palette` only reads the
    /// `Palette`, so we construct the overlay state by hand.
    fn app_with_commit_palette(cs: &Changeset, kind: PaletteKind, selected: usize) -> App {
        let mut app = App::new(cs);
        let matches: Vec<usize> = match &kind {
            PaletteKind::Commits { commits, .. } => (0..commits.len()).collect(),
            PaletteKind::Files => (0..app.cs().files.len()).collect(),
        };
        let p = Palette {
            kind,
            query: String::new(),
            matches,
            selected,
            mode_hint: "summary",
        };
        app.mode.push_overlay(Overlay::Palette(p));
        app
    }

    #[test]
    fn palette_commits_renders_sha_summary_and_date() {
        let cs = changeset(vec![pfile("src/auth.rs", FileStatus::Modified)]);
        let kind = PaletteKind::Commits {
            commits: vec![
                commit("abc1234", "fix the thing", "2026-06-01"),
                commit("def5678", "add another thing", "2026-06-02"),
            ],
            scoped_path: None,
            truncated: false,
        };
        let mut app = app_with_commit_palette(&cs, kind, 1);
        let buf = render_buf(&mut app, 80, 16);
        let text = buf_text(&buf);

        assert!(text.contains("pick commit"), "commit palette title: {text}");
        assert!(text.contains("abc1234"), "short sha rendered");
        assert!(text.contains("fix the thing"), "summary rendered");
        assert!(text.contains("2026-06-01"), "date rendered");

        // Selection moved to the second row carries the highlight background.
        let sel_bg = app.theme.sel_focus_bg;
        let highlighted =
            (0..buf.area.height).any(|y| (0..buf.area.width).any(|x| buf[(x, y)].bg == sel_bg));
        assert!(highlighted, "selected commit row is highlighted");
    }

    #[test]
    fn palette_commits_scoped_history_title_with_cap() {
        let cs = changeset(vec![pfile("src/auth.rs", FileStatus::Modified)]);
        let kind = PaletteKind::Commits {
            commits: vec![commit("abc1234", "history entry", "2026-06-01")],
            scoped_path: Some("src/auth.rs".into()),
            truncated: true,
        };
        let mut app = app_with_commit_palette(&cs, kind, 0);
        let text = buf_text(&render_buf(&mut app, 80, 16));
        assert!(text.contains("history"), "scoped-history title: {text}");
        assert!(text.contains("auth.rs"), "scoped file name in title");
        assert!(text.contains("200+"), "truncation cap shown");
    }

    #[test]
    fn palette_commits_unscoped_truncated_cap() {
        let cs = changeset(vec![pfile("src/auth.rs", FileStatus::Modified)]);
        let kind = PaletteKind::Commits {
            commits: vec![commit("abc1234", "recent work", "2026-06-01")],
            scoped_path: None,
            truncated: true,
        };
        let mut app = app_with_commit_palette(&cs, kind, 0);
        let text = buf_text(&render_buf(&mut app, 80, 16));
        assert!(text.contains("pick commit"), "unscoped title: {text}");
        assert!(text.contains("200+"), "truncation cap shown");
    }

    // --- draw_commit_message render coverage --------------------------------

    #[test]
    fn commit_message_popup_renders_title_body_and_hint() {
        use crate::model::CommitMessage;
        use crate::tui::app::CommitMsg;
        let cs = changeset(vec![pfile("src/auth.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.mode
            .push_overlay(Overlay::CommitMessage(CommitMsg::new(CommitMessage {
                sha: "abc1234deadbeef".into(),
                short: "abc1234".into(),
                author: "Tester".into(),
                date: "2026-06-30".into(),
                body: "Summary line\n\nBody paragraph here.".into(),
            })));
        let text = buf_text(&render_buf(&mut app, 80, 16));
        assert!(text.contains("abc1234"), "short sha in title: {text}");
        assert!(text.contains("Tester"), "author in title");
        assert!(text.contains("2026-06-30"), "date in title");
        assert!(text.contains("Summary line"), "body rendered");
        assert!(text.contains("open commit"), "popup hint shown");
    }

    // --- draw_blame_body render coverage ------------------------------------

    #[test]
    fn blame_peek_renders_attribution_gutter() {
        use crate::model::LayoutMode;
        use crate::tui::theme::ThemeName;
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // A selectable real-path file; blame reads Cargo.toml at HEAD.
        let cs = changeset(vec![pfile("Cargo.toml", FileStatus::Modified)]);
        let mut app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir),
            ViewKind::Local,
            false,
            None,
            None,
        );
        app.open_peek_blame();
        // Drive the background blame to completion so the gutter fills.
        crate::tui::testutil::drive_blame(&mut app);
        let text = buf_text(&render_buf(&mut app, 100, 24));
        assert!(text.contains("blame"), "blame mode in the title: {text}");
        assert!(text.contains('│'), "the gutter rule is rendered");
    }

    #[test]
    fn blame_peek_shows_loading_while_the_fetch_runs() {
        // With the committed content fetched on the worker, the empty pane must
        // read as in-progress, not "No content.", while the channel is live.
        let cs = changeset(vec![pfile("a.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs); // no repo: blame content stays empty
        app.open_peek_blame();
        let (_tx, rx) = std::sync::mpsc::channel();
        if let crate::tui::app::Base::Peek(p) = &mut app.mode.base {
            p.blame_rx = Some(rx);
        }
        let text = buf_text(&render_buf(&mut app, 74, 16));
        assert!(text.contains("Loading"), "loading placeholder: {text}");
    }

    // --- draw_peek render coverage ------------------------------------------

    /// A diffed file whose old/new sides are identical — its diff is empty.
    fn unchanged(path: &str) -> DiffFile {
        DiffFile {
            path: path.into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks: Vec::new(),
            stats: Stats::default(),
            language: None,
            is_binary: false,
            old_text: Some("same\n".into()),
            new_text: Some("same\n".into()),
            content_digest: None,
            diffed: true,
        }
    }

    /// A diffed file with empty content on both sides (nothing to preview).
    fn blank(path: &str) -> DiffFile {
        DiffFile {
            path: path.into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks: Vec::new(),
            stats: Stats::default(),
            language: None,
            is_binary: false,
            old_text: Some(String::new()),
            new_text: Some(String::new()),
            content_digest: None,
            diffed: true,
        }
    }

    #[test]
    fn peek_content_mode_renders_preview() {
        let cs = changeset(vec![pfile("src/auth.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.open_peek_preview();
        let text = buf_text(&render_buf(&mut app, 74, 16));
        assert!(text.contains("preview"), "preview mode in title: {text}");
        assert!(
            text.contains("auth.rs"),
            "peeked file path in title: {text}"
        );
    }

    #[test]
    fn peek_diff_mode_empty_shows_no_differences() {
        let cs = changeset(vec![unchanged("src/same.rs")]);
        let mut app = App::new(&cs);
        app.open_peek_review();
        let text = buf_text(&render_buf(&mut app, 74, 16));
        assert!(
            text.contains("No differences"),
            "empty diff peek message: {text}"
        );
    }

    #[test]
    fn peek_content_mode_empty_shows_no_content() {
        let cs = changeset(vec![blank("src/empty.rs")]);
        let mut app = App::new(&cs);
        app.open_peek_preview();
        let text = buf_text(&render_buf(&mut app, 74, 16));
        assert!(
            text.contains("No content"),
            "empty content peek message: {text}"
        );
    }

    #[test]
    fn peek_split_diff_renders_pairs() {
        let cs = changeset(vec![pfile("src/auth.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        app.open_peek_review();
        app.peek_toggle_split(); // switch the peek to side-by-side
        let text = buf_text(&render_buf(&mut app, 100, 16));
        assert!(text.contains("auth.rs"), "split peek renders: {text}");
    }

    #[test]
    fn peek_over_commit_view_uses_commit_accent() {
        let cs = changeset(vec![pfile("src/auth.rs", FileStatus::Modified)]);
        let mut app = App::new(&cs);
        // A commit view makes the peek non-local, so it draws with the commit accent.
        app.push_test_view(&cs, ViewKind::Commit("abc123".into()), false);
        app.open_peek_preview();
        let buf = render_buf(&mut app, 74, 16);
        let text = buf_text(&buf);
        assert!(
            text.contains("auth.rs"),
            "peek renders over a commit view: {text}"
        );
        let commit = app.theme.commit;
        let used =
            (0..buf.area.height).any(|y| (0..buf.area.width).any(|x| buf[(x, y)].fg == commit));
        assert!(used, "the commit accent colors the peek box");
    }
}
