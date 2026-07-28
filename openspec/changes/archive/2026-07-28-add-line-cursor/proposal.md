## Why

rediff has no cursor. `ViewState` carries `scroll` (viewport top), `h_scroll` (a horizontal pan of
the body), and `selected` — a *file* index, re-derived from whichever file sits at the top of the
screen. Motion moves the viewport; nothing points at a line.

Fine for reading, fatal for anything that must act on a line. It surfaced while planning
`tui-review`, whose premise — "comment on the line under the cursor" — rests on state that does not
exist. It belongs here rather than smuggled into that change: it is a navigation feature on its own.

## What Changes

- **A line cursor in the review stream**, `ViewState.cursor_row`, moved by the incremental motion
  keys, drawn distinctly, kept in view. It works identically in unified and side-by-side layouts.
- **BREAKING (behaviour): `j`/`k` no longer scroll until the cursor reaches an edge.** Today every
  motion key moves the viewport. After this, incremental motion moves the cursor within the visible
  rows and only scrolls at the edges. This is the point of the change and it will feel different.
- **Jumps keep top-alignment.** Jump-to-file, next/prev hunk, top and bottom continue to place the
  target at the top of the viewport as the accepted `navigation` requirement says; they additionally
  place the cursor there. Only incremental motion is edge-scrolled.
- **The cursor survives a plan rebuild** — layout toggle, directory fold, reviewed toggle, and a
  streamed diff landing all rebuild the row plan. It re-anchors on `(file, side, line)`; a row
  carrying no such identity keeps its position *within its own file*, so another file growing does
  not move it; content hidden by a fold lands on that directory's placeholder; and the viewport is
  adjusted afterwards so the cursor is always drawn.
- **The unit is the change, not the line.** In side-by-side layout one row holds the removed line and
  the line that replaced it; the cursor names that whole row, and the row is marked as one thing.
  There is no side for the user to choose and no key to choose it with. Acting on the cursor anchors
  to the new side when the change has one and the old side otherwise — which is also the side that
  survives re-resolution, the old side being history.
- **One more row is reachable at the bottom of the stream.** The sticky file header steals a line from
  the drawn area, which today leaves the final plan row unreachable. Honouring the cursor's visibility
  guarantee means accounting for it, so `G` now reaches the last row.

**Explicitly not changing:**

- `selected`, `selected_dir`, and `reveal_selected`. The first two are written by the sidebar and the
  fold model; the third scrolls the *file list*, not the stream. An earlier draft got all three wrong.
- **The peek.** `stream::scroll_by` is left exactly as it is and the peek keeps calling it, so `j` in
  the peek scrolls as it does today while `j` in the stream moves the cursor. The peek's renderer
  *drops* rows — file and hunk headers in diff mode, everything but new-side lines in blame mode — so
  a cursor there needs a row↔drawn-line mapping the stream does not. That is a different piece of
  work, and its real payoff is blame mode, where `Enter` today opens the commit for whatever line
  happens to sit at the top of the screen. Filed as a follow-up, not bundled here.

## Capabilities

### New Capabilities

- `line-cursor`: a per-line cursor over the review stream — its motion, rendering, how it stays in
  view, how it survives a plan rebuild, and how it names a cell in a side-by-side layout.

### Modified Capabilities

- `review-stream`: its accepted **Scrolling** requirement says pressing the scroll keys moves the
  viewport — and `j`/`k` are literally the scroll keys by the app's own catalog (`keymap.rs:103`,
  `b("jk", "scroll")` in `BIND_STREAM`). Incremental motion now moves the cursor instead, so that
  requirement is restated. The *scroll* gestures that remain — the fast-scroll keys and the mouse
  wheel — keep moving the viewport and now carry the cursor with them, so a scroll never leaves the
  cursor behind and the mouse-wheel guarantee stays true.

`navigation` is not modified: jumps keep the top-alignment its accepted requirement specifies, and
the new incremental-motion rule is additive. `file-peek` is not modified: the peek is untouched.

## Impact

- **`src/tui/view.rs`**: one new field, `cursor_row: usize` on `ViewState`. Named `cursor_row`, not
  `cursor`, because `Session.cursor` already means the active view in the stack and
  `selected`/`selected_dir` are already called "the cursor" in `sidebar.rs` and `nav.rs`. The peek
  shares `ViewState` and will carry a `cursor_row` it never reads — the field says so.
- **`src/tui/rows.rs`**: a row reports the identities it can carry. A unified context row carries
  **both** `(file, Old, o)` and `(file, New, n)`, and a split pair carries one per cell — which is
  what lets the layout toggle match one row against two, in both directions. This is machinery for
  the toggle, not a choice offered to the user.
- **`src/tui/stream.rs`**: a new `move_cursor_by` plus `scroll_into_view`; the jumps additionally set
  the cursor; `clamp` gains the visibility invariant. `stream::scroll_by` is **not** converted — it
  becomes the peek's alone.
- **`src/tui/app/nav.rs`, `runtime/keys.rs`, `runtime/events.rs`**: `App::scroll_by` is a *different*
  function with four callers — `j`/`k`, `J`/`K`, Shift-arrows, and the mouse wheel — which now
  diverge, so it splits into `move_cursor` and `scroll_view`, with five call-site edits.
- **`src/tui/session.rs`**: capture-and-restore around the plan build, inside `Session::build_plan` —
  the single funnel every one of the seven `build_plan()` call sites reaches.
- **`src/tui/ui/stream.rs`**: a cursor marker in column `area.x`, which both layouts already leave
  blank; the drawn height accounts for the sticky file header. The row renderers are not modified and
  gain no parameter, so `ui/overlays.rs` is not edited and the peek stays untouched.
- **Test migration is a first-class cost**, measured rather than estimated. Counts below are
  occurrences of `.scroll` and `.selected` respectively: `keys_tests.rs` (21/29), `events.rs` (18/6),
  `render_tests.rs` (13/8), plus `stream.rs`'s own four motion tests. Out of scope and staying green
  untouched: `peek_tests.rs` (21/0), `peekview.rs` (4/11), `peek.rs` (9/0).
- **No new keybinding.** Nothing is added to `handle_global_key`, `handle_stream_key`, or the
  `keymap.rs` catalog — the existing motion keys do all of it.
- **Gates**: `handle_global_key` is at cyclomatic 30 with 100% coverage — CRAP exactly 30 against a
  threshold of 30, so even a coverage dip fails. Nothing here touches it.
