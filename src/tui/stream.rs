//! Diff-stream navigation: pure viewport/cursor operations over a view's
//! [`ViewState`] + [`Plan`]. These are free functions — no `App` — so the main
//! stream and the single-file peek can share one navigation model. They touch
//! only the scroll/selection state, the immutable plan, and the viewport
//! geometry; never the loader, highlighter, or view stack.

use crate::model::LayoutMode;
use crate::tui::rows::{self, Plan};
use crate::tui::view::ViewState;

/// One split column's text width (mirrors `draw_split`'s column geometry).
fn split_col_w(viewport_w: usize) -> usize {
    viewport_w.saturating_sub(1) / 2
}

/// Max horizontal scroll: content width beyond the viewport. In split layout the
/// bound is one column's width, since each side pans within its own column.
pub fn max_h_scroll(plan: &Plan, viewport_w: usize) -> usize {
    let visible = if matches!(plan.layout, LayoutMode::Split) {
        split_col_w(viewport_w)
    } else {
        viewport_w
    };
    plan.content_w.saturating_sub(visible)
}

/// Max vertical scroll for a `rows`-row body in a `viewport_h`-tall viewport:
/// the last top that still fills the viewport, so the final page stays full.
pub fn max_scroll_rows(rows: usize, viewport_h: usize) -> usize {
    rows.saturating_sub(viewport_h.max(1))
}

/// Max vertical scroll: the last viewport-top that still shows content.
pub fn max_scroll(plan: &Plan, viewport_h: usize) -> usize {
    max_scroll_rows(plan.rows.len(), viewport_h)
}

/// `Changeset::files` index of the file currently at the top of the viewport.
/// `file_at` returns a *visible ordinal*; map it back through `visible_files`
/// (identity when nothing is folded). Falls back to 0 when no file is visible.
pub fn current_file(st: &ViewState, plan: &Plan) -> usize {
    let ord = rows::file_at(&plan.file_starts, st.scroll);
    plan.visible_files.get(ord).copied().unwrap_or(0)
}

/// `Changeset::files` index of the file the **line cursor** is in.
///
/// Deliberately separate from [`current_file`], which stays scroll-derived: the
/// sticky file header is a property of the viewport top, and `rebuild_plan`
/// re-anchors *scroll* against the file at the top. Those are different
/// questions from "which file is the reader pointing at".
pub fn cursor_file(st: &ViewState, plan: &Plan) -> usize {
    let ord = rows::file_at(&plan.file_starts, st.cursor_row);
    plan.visible_files.get(ord).copied().unwrap_or(0)
}

/// As the line cursor moves, the active (sidebar-highlighted) file follows the
/// file the cursor is in, and the sidebar reveals it.
pub fn anchor_selected(st: &mut ViewState, plan: &Plan) {
    let near = cursor_file(st, plan);
    match rows::dir_at(plan, st.cursor_row) {
        // A folded directory's placeholder has no file, and `file_at` would name
        // whichever file precedes it. Naming that file clears `selected_dir` and
        // makes `z` fold *it* instead of unfolding the directory under the
        // cursor — the accepted `directory-collapse` rule says the placeholder's
        // one verb is unfold.
        Some(dir) => st.select_dir(dir.to_string(), near),
        None => st.select_file(near),
    }
    st.reveal_selected = true;
}

/// Rows actually available for the body: one fewer than the viewport whenever a
/// sticky file header can be pinned.
///
/// `draw_stack` pins the current file's header once it has scrolled off and then
/// draws one row fewer — a condition that depends on `scroll`, which is circular
/// for a `scroll_into_view` about to change it. The way out is that the estimate
/// only has to never be *too large*: reserve the row whenever the plan has any
/// file header at all. When the header turns out not to be pinned we have
/// reserved one row needlessly and scroll one row early, which immediately makes
/// it pinned anyway.
///
/// Deliberately **no `.max(1)` floor**: `draw_stack`'s own `content_height` is an
/// unfloored `saturating_sub(1)`, so at `viewport_h == 1` with a pinned header it
/// draws *nothing*, and a floored estimate would claim one row — an over-estimate,
/// the single thing this must never produce. `viewport_h` is 1 until the first
/// draw, so that is not a hypothetical size.
pub fn usable(plan: &Plan, viewport_h: usize) -> usize {
    if plan.file_starts.is_empty() {
        viewport_h
    } else {
        viewport_h.saturating_sub(1)
    }
}

/// Move `scroll` the minimum distance that puts the cursor on screen. Never
/// re-centres: a step past the edge scrolls exactly one row.
///
/// `usable == 0` means nothing can be drawn, so the guarantee is vacuous rather
/// than violated and the viewport is left alone.
pub fn scroll_into_view(st: &mut ViewState, plan: &Plan, usable: usize) {
    if usable == 0 {
        return;
    }
    if st.cursor_row < st.scroll {
        st.scroll = st.cursor_row;
    } else if st.cursor_row >= st.scroll + usable {
        st.scroll = st.cursor_row + 1 - usable;
    }
    st.scroll = st.scroll.min(max_scroll(plan, usable));
}

/// Clamp scroll, horizontal scroll, and the line cursor into range, then make the
/// cursor visible (after a resize / plan rebuild).
///
/// This is the every-frame net, and it owns the *visibility* half of surviving a
/// rebuild: `build_plan` restores which row the cursor is on, but the cursor and
/// `scroll` are repaired against possibly different files and can drift apart
/// even when each is individually right. `build_plan` cannot do it — it takes no
/// viewport height.
pub fn clamp(st: &mut ViewState, plan: &Plan, usable: usize, viewport_w: usize) {
    st.scroll = st.scroll.min(max_scroll(plan, usable));
    st.h_scroll = st.h_scroll.min(max_h_scroll(plan, viewport_w));
    st.cursor_row = st.cursor_row.min(plan.rows.len().saturating_sub(1));
    scroll_into_view(st, plan, usable);
}

/// Re-anchor a viewport `scroll` after a plan rebuild. Normally the offset within
/// the anchor file is preserved (`new_start + (scroll − old_start)`). A scroll
/// parked above the anchor file's header — the fixed commit-message banner region
/// at the top of the plan (`scroll < old_start`) — is kept exactly where it is.
///
/// That is no longer the last word on where the viewport ends up. `clamp` runs
/// after every rebuild and moves `scroll` to keep the **line cursor** visible, so
/// the banner survives a streaming rebuild only while the cursor is also in it.
/// A cursor parked on a file whose rows move far down the plan takes the viewport
/// with it — the cursor is the reader's position, and showing it wins.
pub fn reanchored(scroll: usize, old_start: usize, new_start: usize) -> usize {
    if scroll < old_start {
        scroll
    } else {
        new_start + (scroll - old_start)
    }
}

pub fn scroll_to(st: &mut ViewState, plan: &Plan, viewport_h: usize, row: usize) {
    st.scroll = row.min(max_scroll(plan, viewport_h));
}

/// Scroll the viewport, leaving the line cursor alone.
///
/// **The peek's, and only the peek's.** The stream moves its cursor instead —
/// see [`move_cursor_by`] and [`scroll_view_by`]. The peek shares `ViewState` but
/// never reads `cursor_row`, so this deliberately does not touch it.
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "scroll rows/viewport heights are bounded by plan size, far below isize::MAX; clamped to >= 0 before the cast back to usize"
)]
pub fn scroll_by(st: &mut ViewState, plan: &Plan, viewport_h: usize, delta: isize) {
    let next = st.scroll as isize + delta;
    st.scroll = next.clamp(0, max_scroll(plan, viewport_h) as isize) as usize;
    anchor_selected(st, plan);
}

/// The last row a cursor may occupy.
fn last_row(plan: &Plan) -> isize {
    #[expect(
        clippy::cast_possible_wrap,
        reason = "plan row counts are bounded by changeset size, far below isize::MAX"
    )]
    let n = plan.rows.len().saturating_sub(1) as isize;
    n
}

/// Move the line cursor by `delta`, with the viewport following only as far as
/// needed. This is `j`/`k`, the arrows, and the page steps.
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "cursor rows are bounded by plan size, far below isize::MAX; clamped to >= 0 before the cast back to usize"
)]
pub fn move_cursor_by(st: &mut ViewState, plan: &Plan, usable: usize, delta: isize) {
    st.cursor_row = (st.cursor_row as isize + delta).clamp(0, last_row(plan)) as usize;
    scroll_into_view(st, plan, usable);
    anchor_selected(st, plan);
}

/// Scroll the viewport and carry the cursor with it, so the cursor keeps its row
/// on screen. This is `J`/`K`, Shift+arrows, and the mouse wheel — gestures that
/// mean "move the view", not "move the cursor".
///
/// The cursor shifts by the delta **actually applied**, not the one requested:
/// `scroll` clamps at `max_scroll` while the cursor clamps at the last row, a gap
/// of `usable - 1`, so shifting by the requested delta would slide the cursor's
/// screen row by one per notch near either end.
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "scroll/cursor rows are bounded by plan size, far below isize::MAX; clamped to >= 0 before the cast back to usize"
)]
pub fn scroll_view_by(st: &mut ViewState, plan: &Plan, usable: usize, delta: isize) {
    let before = st.scroll as isize;
    let next = (before + delta).clamp(0, max_scroll(plan, usable) as isize);
    st.scroll = next as usize;
    let applied = next - before;
    st.cursor_row = (st.cursor_row as isize + applied).clamp(0, last_row(plan)) as usize;
    scroll_into_view(st, plan, usable);
    anchor_selected(st, plan);
}

#[expect(
    clippy::cast_possible_wrap,
    reason = "viewport height is a small terminal dimension, far below isize::MAX"
)]
pub fn page(st: &mut ViewState, plan: &Plan, usable: usize, dir: isize) {
    let step = usable.saturating_sub(1).max(1) as isize;
    move_cursor_by(st, plan, usable, dir * step);
}

#[expect(
    clippy::cast_possible_wrap,
    reason = "viewport height is a small terminal dimension, far below isize::MAX"
)]
pub fn half_page(st: &mut ViewState, plan: &Plan, usable: usize, dir: isize) {
    let step = (usable / 2).max(1) as isize;
    move_cursor_by(st, plan, usable, dir * step);
}

pub fn top(st: &mut ViewState, plan: &Plan) {
    st.scroll = 0;
    st.cursor_row = 0;
    anchor_selected(st, plan);
}

/// The end of the stream: the cursor on the last row, the viewport wherever it
/// must be to show it. That row is always a `Spacer` — every file's rows end with
/// one — which is the honest meaning of "the end".
pub fn bottom(st: &mut ViewState, plan: &Plan, usable: usize) {
    st.cursor_row = plan.rows.len().saturating_sub(1);
    scroll_into_view(st, plan, usable);
    anchor_selected(st, plan);
}

/// Jump to the next hunk **after the cursor**, top-aligning the viewport.
///
/// Searching from `scroll` rather than the cursor would let `]` find a hunk the
/// cursor has already passed and move it backwards.
pub fn next_hunk(st: &mut ViewState, plan: &Plan, usable: usize) {
    if let Some(row) = plan
        .hunk_starts
        .iter()
        .copied()
        .find(|&r| r > st.cursor_row)
    {
        scroll_to(st, plan, usable, row);
        st.cursor_row = row;
        anchor_selected(st, plan);
    }
}

pub fn prev_hunk(st: &mut ViewState, plan: &Plan, usable: usize) {
    if let Some(row) = plan
        .hunk_starts
        .iter()
        .copied()
        .rev()
        .find(|&r| r < st.cursor_row)
    {
        scroll_to(st, plan, usable, row);
        st.cursor_row = row;
        anchor_selected(st, plan);
    }
}

/// Jump the viewport to a file's header row (no selection/focus change — that is
/// the coordinator's job). `idx` is a `Changeset::files` index; it is mapped to
/// its visible ordinal first. A folded file has no row, so the jump is a no-op
/// (the coordinator unfolds before jumping when landing on it by path).
pub fn jump_to_file(st: &mut ViewState, plan: &Plan, usable: usize, idx: usize) {
    if let Some(ord) = plan.visible_ordinal(idx) {
        if let Some(row) = plan.file_starts.get(ord).copied() {
            scroll_to(st, plan, usable, row);
            st.cursor_row = row;
        }
    }
    // A missing target leaves both the viewport and the cursor alone: a
    // half-moved cursor would be worse than not moving.
}

/// Scroll the viewport to a folded directory's placeholder row, and put the
/// cursor on it — otherwise the next step would snap the viewport back, undoing
/// the fold's landing.
///
/// Deliberately does **not** call `anchor_selected`: that would clear
/// `selected_dir` via `select_file`, and the placeholder selection is exactly
/// what makes `z` unfold rather than fold again.
pub fn jump_to_collapsed(st: &mut ViewState, plan: &Plan, usable: usize, dir: &str) {
    if let Some(row) = plan.collapsed_row(dir) {
        scroll_to(st, plan, usable, row);
        st.cursor_row = row;
    }
}

#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "h-scroll columns/viewport widths are bounded by plan size, far below isize::MAX; clamped to >= 0 before the cast back to usize"
)]
pub fn h_scroll_by(st: &mut ViewState, plan: &Plan, viewport_w: usize, delta: isize) {
    let max = max_h_scroll(plan, viewport_w) as isize;
    st.h_scroll = (st.h_scroll as isize + delta).clamp(0, max) as usize;
}

pub fn toggle_wrap(st: &mut ViewState) {
    st.wrap = !st.wrap;
    st.h_scroll = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::rows::Row;

    fn plan_with(rows: usize, file_starts: Vec<usize>) -> Plan {
        let visible_files = (0..file_starts.len()).collect();
        Plan {
            rows: (0..rows).map(|_| Row::Spacer).collect(),
            file_starts,
            visible_files,
            hunk_starts: Vec::new(),
            content_w: 0,
            layout: LayoutMode::Stack,
        }
    }

    #[test]
    fn reanchored_keeps_banner_region_and_preserves_in_file_offset() {
        // Banner region (scroll < old_start): the position is kept verbatim, even
        // when the anchor file's header moves.
        assert_eq!(reanchored(2, 5, 5), 2);
        assert_eq!(reanchored(0, 5, 9), 0);
        assert_eq!(reanchored(4, 5, 9), 4, "still above the header → unchanged");
        // At/after the header: the in-file offset rides to the new header row.
        assert_eq!(reanchored(5, 5, 9), 9, "header → new header");
        assert_eq!(
            reanchored(7, 5, 9),
            11,
            "offset 2 past the header preserved"
        );
    }

    #[test]
    fn scroll_by_clamps_to_the_plan() {
        let plan = plan_with(20, vec![0]);
        let mut st = ViewState::default();
        scroll_by(&mut st, &plan, 5, 1000); // far past the end
        assert_eq!(st.scroll, 15, "clamped to max_scroll = rows - viewport");
        assert_eq!(max_scroll(&plan, 5), 15);
        scroll_by(&mut st, &plan, 5, -1000);
        assert_eq!(st.scroll, 0, "clamped at the top");
    }

    #[test]
    fn jump_to_file_lands_on_the_file_start() {
        let plan = plan_with(20, vec![0, 8, 14]);
        let mut st = ViewState::default();
        jump_to_file(&mut st, &plan, 5, 1);
        assert_eq!(st.scroll, 8, "viewport top sits on file 1's header row");
        jump_to_file(&mut st, &plan, 5, 2);
        assert_eq!(st.scroll, 14);
    }

    #[test]
    fn jump_to_file_is_a_noop_for_an_unknown_file() {
        let plan = plan_with(20, vec![0, 8]);
        let mut st = ViewState {
            scroll: 5,
            ..ViewState::default()
        };
        // A file index that maps to no visible ordinal (folded/out of range)
        // leaves the viewport untouched.
        jump_to_file(&mut st, &plan, 5, 99);
        assert_eq!(st.scroll, 5, "unknown file index leaves scroll put");
    }

    #[test]
    fn usable_reserves_the_sticky_row_and_never_over_estimates() {
        let with_files = plan_with(20, vec![0]);
        let no_files = plan_with(20, Vec::new());
        assert_eq!(
            usable(&with_files, 10),
            9,
            "reserve the pinnable header row"
        );
        assert_eq!(usable(&no_files, 10), 10, "no file header, nothing to pin");
        // The size that matters: `draw_stack` computes `1.saturating_sub(1) == 0`
        // and draws no body rows at all. A `.max(1)` floor here would claim one,
        // which is the over-estimate the whole rule exists to prevent.
        assert_eq!(usable(&with_files, 1), 0, "no floor");
        assert_eq!(usable(&with_files, 0), 0);
    }

    #[test]
    fn motion_inside_the_viewport_does_not_scroll() {
        let plan = plan_with(20, vec![0]);
        let mut st = ViewState::default();
        move_cursor_by(&mut st, &plan, 9, 5);
        assert_eq!(st.cursor_row, 5);
        assert_eq!(st.scroll, 0, "the row was already on screen");
    }

    #[test]
    fn motion_past_the_edge_scrolls_exactly_one_row() {
        let plan = plan_with(20, vec![0]);
        let mut st = ViewState {
            cursor_row: 8,
            ..ViewState::default()
        };
        move_cursor_by(&mut st, &plan, 9, 1);
        assert_eq!(st.cursor_row, 9);
        assert_eq!(st.scroll, 1, "scrolled just enough, not re-centred");
    }

    #[test]
    fn motion_clamps_at_both_ends() {
        let plan = plan_with(20, vec![0]);
        let mut st = ViewState::default();
        move_cursor_by(&mut st, &plan, 9, -100);
        assert_eq!((st.cursor_row, st.scroll), (0, 0), "clamped at the top");
        move_cursor_by(&mut st, &plan, 9, 1000);
        assert_eq!(st.cursor_row, 19, "clamped on the last row");
        assert_eq!(st.scroll, max_scroll(&plan, 9), "which is reachable");
        assert!(
            st.cursor_row < st.scroll + 9,
            "and drawn: cursor {} within [{}, {})",
            st.cursor_row,
            st.scroll,
            st.scroll + 9
        );
    }

    #[test]
    fn nothing_drawable_leaves_the_viewport_alone() {
        // `usable == 0` makes the visibility guarantee vacuous, not violated.
        // This is the pre-first-draw state of every view.
        let plan = plan_with(20, vec![0]);
        let mut st = ViewState {
            scroll: 4,
            cursor_row: 4,
            ..ViewState::default()
        };
        move_cursor_by(&mut st, &plan, 0, 5);
        assert_eq!(st.cursor_row, 9, "the cursor still moves");
        assert_eq!(st.scroll, 4, "the viewport is left alone");
    }

    #[test]
    fn a_scroll_gesture_keeps_the_cursors_screen_row_at_the_bottom() {
        // scroll clamps at max_scroll, the cursor at the last row — different
        // bounds. Shifting by the *requested* delta slides the cursor's screen
        // row one per notch near the end; shifting by the applied delta does not.
        let plan = plan_with(30, vec![0]);
        let mut st = ViewState {
            scroll: 18,
            cursor_row: 25,
            ..ViewState::default()
        };
        let screen_row = st.cursor_row - st.scroll;
        scroll_view_by(&mut st, &plan, 10, 3);
        assert_eq!(st.scroll, 20, "clamped at max_scroll, so only 2 applied");
        assert_eq!(
            st.cursor_row, 27,
            "the cursor moved by 2, not the requested 3"
        );
        assert_eq!(st.cursor_row - st.scroll, screen_row, "same row on screen");
    }

    #[test]
    fn a_scroll_gesture_keeps_the_cursors_screen_row_at_the_top() {
        let plan = plan_with(30, vec![0]);
        let mut st = ViewState {
            scroll: 0,
            cursor_row: 3,
            ..ViewState::default()
        };
        scroll_view_by(&mut st, &plan, 10, -5);
        assert_eq!(
            (st.scroll, st.cursor_row),
            (0, 3),
            "nothing applied, nothing moved"
        );
    }

    #[test]
    fn next_hunk_is_relative_to_the_cursor_not_the_viewport() {
        // The bug this prevents: searching from `scroll` lets `]` find a hunk the
        // cursor has already passed and move it *backwards*.
        let mut plan = plan_with(40, vec![0]);
        plan.hunk_starts = vec![5, 15, 25];
        let mut st = ViewState {
            scroll: 0,
            cursor_row: 20,
            ..ViewState::default()
        };
        next_hunk(&mut st, &plan, 9);
        assert_eq!(st.cursor_row, 25, "the next hunk *after the cursor*");
    }

    #[test]
    fn jump_to_collapsed_places_the_cursor_on_the_placeholder() {
        // Without this the fold lands the viewport and leaves the cursor stale,
        // so the next step snaps the view back and undoes the landing.
        let mut plan = plan_with(20, vec![0, 8]);
        plan.rows[5] = Row::CollapsedDir {
            dir: "src".into(),
            n: 2,
            reviewed: 0,
        };
        let mut st = ViewState::default();
        jump_to_collapsed(&mut st, &plan, 9, "src");
        assert_eq!(st.cursor_row, 5, "the cursor lands on the placeholder");
        assert_eq!(st.scroll, 5, "and the viewport top-aligns on it");
        assert_eq!(
            st.selected_dir, None,
            "and it does not clear a placeholder selection via anchor_selected"
        );
    }

    #[test]
    fn a_jump_with_no_target_moves_neither_viewport_nor_cursor() {
        let plan = plan_with(20, vec![0, 8]);
        let mut st = ViewState {
            scroll: 5,
            cursor_row: 6,
            ..ViewState::default()
        };
        jump_to_file(&mut st, &plan, 9, 99);
        jump_to_collapsed(&mut st, &plan, 9, "nope");
        assert_eq!(
            (st.scroll, st.cursor_row),
            (5, 6),
            "a half-move would be worse"
        );
    }

    #[test]
    fn clamp_pulls_a_stale_cursor_back_into_range_and_into_view() {
        // The every-frame net, and the visibility half of surviving a rebuild.
        let plan = plan_with(20, vec![0]);
        let mut st = ViewState {
            scroll: 0,
            cursor_row: 500,
            ..ViewState::default()
        };
        clamp(&mut st, &plan, 9, 80);
        assert_eq!(st.cursor_row, 19, "clamped into the plan");
        assert!(
            (st.scroll..st.scroll + 9).contains(&st.cursor_row),
            "and made visible"
        );
    }

    /// A plan of `len` blank rows, with a file header iff it has any rows — so
    /// `usable` behaves as it does in a real plan.
    fn grid_plan(len: usize) -> Plan {
        plan_with(len, if len == 0 { Vec::new() } else { vec![0] })
    }

    #[test]
    fn clamp_is_idempotent_across_the_whole_small_grid() {
        // `clamp` runs on every frame, so a non-idempotent one would drift a
        // row per redraw rather than failing outright. Brute-forced rather than
        // sampled, because the interesting cases are the degenerate sizes.
        for len in 0..8usize {
            for usable in 0..6usize {
                for scroll in 0..10usize {
                    for cursor in 0..10usize {
                        let plan = grid_plan(len);
                        let mut once = ViewState {
                            scroll,
                            cursor_row: cursor,
                            ..ViewState::default()
                        };
                        clamp(&mut once, &plan, usable, 80);
                        let mut twice = once.clone();
                        clamp(&mut twice, &plan, usable, 80);
                        assert_eq!(
                            (once.scroll, once.cursor_row),
                            (twice.scroll, twice.cursor_row),
                            "not idempotent: len={len} usable={usable} in=({scroll},{cursor})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn clamp_always_leaves_the_cursor_on_screen() {
        // The invariant everything else leans on: after `clamp`, the cursor is
        // inside the drawn window. Sizes where something *can* be drawn only —
        // `usable == 0` makes the guarantee vacuous by design.
        for len in 1..8usize {
            for usable in 1..6usize {
                for scroll in 0..10usize {
                    for cursor in 0..10usize {
                        let plan = grid_plan(len);
                        let mut st = ViewState {
                            scroll,
                            cursor_row: cursor,
                            ..ViewState::default()
                        };
                        clamp(&mut st, &plan, usable, 80);
                        assert!(
                            st.cursor_row >= st.scroll && st.cursor_row < st.scroll + usable,
                            "cursor off screen: len={len} usable={usable} in=({scroll},{cursor}) out=({},{})",
                            st.scroll,
                            st.cursor_row
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cursor_file_reads_the_cursor_while_current_file_reads_the_viewport() {
        // The two answer different questions and must not be conflated: the
        // sticky header and the scroll re-anchor follow the viewport top, the
        // sidebar highlight follows the cursor.
        let plan = plan_with(20, vec![0, 8, 14]);
        let st = ViewState {
            scroll: 0,
            cursor_row: 9,
            ..ViewState::default()
        };
        assert_eq!(current_file(&st, &plan), 0, "viewport top is in file 0");
        assert_eq!(cursor_file(&st, &plan), 1, "cursor is in file 1");
    }

    #[test]
    fn cursor_file_maps_through_visible_files_when_something_is_folded() {
        // `file_starts` is parallel to `visible_files`, not to `cs.files`, so a
        // fold makes the ordinal and the file index differ.
        let mut plan = plan_with(20, vec![0, 8]);
        plan.visible_files = vec![0, 5]; // files 1..4 folded away
        let st = ViewState {
            cursor_row: 9,
            ..ViewState::default()
        };
        assert_eq!(cursor_file(&st, &plan), 5, "ordinal 1 maps to file 5");
    }

    #[test]
    fn cursor_file_falls_back_to_zero_with_no_visible_file() {
        // A banner row sits above every file start, and an empty plan has no
        // files at all; neither may panic.
        let plan = plan_with(4, Vec::new());
        let st = ViewState {
            cursor_row: 2,
            ..ViewState::default()
        };
        assert_eq!(cursor_file(&st, &plan), 0);
    }

    #[test]
    fn prev_hunk_steps_back_and_clamps_at_the_top() {
        let mut plan = plan_with(40, vec![0]);
        plan.hunk_starts = vec![5, 15, 25];
        // Seeded on the *cursor*: hunk stepping is relative to where the reader
        // is pointing, not to the viewport top, or `]` could find a hunk the
        // cursor has already passed and move it backwards.
        // `scroll` deliberately NOT equal to the cursor: seeded equal, this test
        // cannot tell a cursor-relative search from a viewport-relative one.
        let mut st = ViewState {
            scroll: 0,
            cursor_row: 30,
            ..ViewState::default()
        };
        prev_hunk(&mut st, &plan, 10);
        assert_eq!(st.cursor_row, 25, "lands on the nearest earlier hunk start");
        assert_eq!(st.scroll, 25, "and top-aligns the viewport on it");
        prev_hunk(&mut st, &plan, 10);
        assert_eq!(st.cursor_row, 15, "steps back another hunk");
        // Before the first hunk there is no earlier one → no movement.
        st.cursor_row = 3;
        st.scroll = 3;
        prev_hunk(&mut st, &plan, 10);
        assert_eq!(st.cursor_row, 3, "no earlier hunk → unchanged");
    }
}
