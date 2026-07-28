# Tasks

A per-line cursor over the review stream. Prerequisite for `tui-review`, useful on its own.

The bulk of the risk is not the cursor — it is that the existing `scroll` and `selected` assertions
encode the *current* model. §5 is not cleanup; it is where this change is proved.

**The peek is out of scope.** `stream::scroll_by` is not converted and `peekview.rs`, `peek.rs`, and
`peek_tests.rs` are not edited. If a task here seems to require touching them, stop — something has
gone wrong.

## 1. State and identity

- [x] 1.1 Add `cursor_row: usize` to `ViewState` — **one field, no stored side.** **Not** `cursor`:
  `Session.cursor` is the active view in the stack, and `selected`/`selected_dir` are already called
  "the cursor" throughout `sidebar.rs` and `nav.rs`. Document on the field that the peek shares this
  struct and does not read `cursor_row`.
- [x] 1.2 `row_keys(&Row) -> (Option<Key>, Option<Key>)` where `Key = (usize, Side, u32)` — the
  identities a row carries, old side first. A unified context row carries **both**; a split pair
  carries one per populated cell; a removed or added row carries one; a binary note
  (`old: None, new: None`) and every chrome row carry none. Take `&Row`, not `(plan, row)` — `Row`'s
  variants are public, so every shape is constructible in a test.
- [x] 1.3 `cursor_key(plan, row) -> Option<Key>`: the key to search with — the new side when the row
  carries it, the old side otherwise. No preference parameter and nothing remembered between
  rebuilds; a stored tiebreaker is row-scoped memory that a single field cannot hold, and it goes
  stale the moment `j` leaves the row that set it.
- [x] 1.4 `find_key(plan, key) -> Option<usize>` and `cursor_file(st, plan)` (`file_at` over
  `cursor_row`, then through `visible_files` — the same shape as `current_file`, which stays
  scroll-derived).
- [x] 1.5 Tests: `row_keys` for unified added/removed/context/binary-note, split both cells, a
  one-sided pair, and every chrome variant; a context row is found from *either* key; two rows never
  carry the same key within one plan; `cursor_file` on a body row, a placeholder, and a banner.

## 2. Motion

- [x] 2.1 `usable(plan, viewport_h)` — `viewport_h.saturating_sub(1)` when the plan has any file
  header, else `viewport_h`. **No `.max(1)` floor**: `draw_stack`'s `content_height` is an unfloored
  `saturating_sub(1)` (`ui/stream.rs:203-205`), so at `viewport_h == 1` with a sticky header it draws
  **zero** body rows while a floored `usable` claims one — an over-estimate, which is the one thing
  the rule must never do. `viewport_h == 1` is the constructor default (`appcore.rs:95`), so every
  view is in that state before its first draw.
- [x] 2.1a `usable == 0` means nothing is drawable: `scroll_into_view` returns without touching
  `scroll`. Test it at `viewport_h == 1` both with and without a file header in the plan.
- [x] 2.2 `move_cursor_by(st, plan, usable, delta)`: clamp the cursor to the plan, then
  `scroll_into_view` moving `scroll` only to the edge.
- [x] 2.2a **Every `stream::` call on the stream's path passes `usable(plan, viewport_h)` where it
  passes `viewport_h` today** — `App::clamp` (`appcore.rs:368`), `App::bottom` (`nav.rs:212`), the
  page/half-page steps, the jumps, **and `Session::rebuild_plan`'s own trailing
  `stream::clamp` (`session.rs:230`)**, which takes `viewport_h` as a parameter and never routes
  through `App::clamp`. `stream::max_scroll` itself is unchanged and the peek keeps passing
  `viewport_h` raw. `App::top` (`nav.rs:207`) takes **no** height today and needs none — `stream::top`
  sets `scroll = 0`, which trivially shows row 0.
- [x] 2.2b **`App::clamp` is the one that silently reverts the change if missed**: it runs from
  `reconcile` on every draw, so a `bottom` using `usable` against a clamp using `viewport_h` is
  snapped back one row every frame — invisible to a unit test of `bottom`, visible only once
  something draws. `rebuild_plan`'s own clamp is the same hazard one layer down, and
  `streaming_rebuild_keeps_the_banner_in_view` (`appcore.rs:836`) calls `rebuild_plan` directly with
  no intervening `App::clamp`, so it exercises that path specifically.
- [x] 2.3 **Split `App::scroll_by` (`nav.rs:189`) in two.** It is a single method serving four
  callers — `j`/`k` and arrows (`keys.rs:289-290`), `J`/`K` and Shift-arrows (`keys.rs:285-288`), and
  the mouse wheel (`events.rs:250`) — which must now diverge:
  `App::move_cursor(delta)` for the first, `App::scroll_view(delta)` for the rest. Five call-site
  edits across `keys.rs` and `events.rs`. Also convert `stream::page` / `half_page` (stream-only).
  **Leave `stream::scroll_by` alone** — a different function; it becomes the peek's, and its doc
  comment must say so.
- [x] 2.3a `move_cursor` clamps the cursor to the plan then `scroll_into_view`. **`scroll_view` moves
  `scroll` by the delta and shifts `cursor_row` by the delta *actually applied*** —
  `applied = new_scroll - old_scroll` — not the requested one. `scroll` clamps at `max_scroll` while
  the cursor clamps at `rows.len() - 1`, a gap of `usable - 1`, so shifting by the requested delta
  slides the cursor's screen row by one per notch near either end. Test both ends explicitly.
- [x] 2.3b Both must call `anchor_selected` (see §2.8) — today `stream::scroll_by` does, and dropping
  it stops the sidebar highlight and the file header's active marker following motion.
- [x] 2.4 Jumps keep top-aligning `scroll` — the accepted `navigation` requirement — and additionally
  set the cursor: `jump_to_file`, `jump_to_collapsed`, `next_hunk`, `prev_hunk`, `top`, `bottom`.
  `G`/`bottom` puts the cursor on the last row. `App::top` does **not** change signature.
- [x] 2.4a `jump_to_file` (`stream.rs:141`) and `jump_to_collapsed` (`:150`) no-op on a missing target
  and must leave the cursor **untouched** rather than half-moved. Latent today — every live caller
  (click, `goto_visible_digit`, `next_unviewed`) passes a visible index — so make it explicit rather
  than discovering it when a caller stops guaranteeing that.
- [x] 2.5 `next_hunk`/`prev_hunk` search from `cursor_row`, not `st.scroll` (`stream.rs:118,128`).
  Scroll-relative, `]` can find a hunk behind the cursor and move it backwards.
- [x] 2.6 `stream::clamp` gains the whole cursor invariant: bound `scroll` to the plan, bound
  `cursor_row` to the plan, **then move `scroll` so the cursor is drawn**. It is the every-frame net
  (`ui/frame.rs:153`) and the post-fold cleanup (`nav.rs:50,129,145,186`), and it already takes both
  the plan and the height — which `Session::build_plan` does not. After `clamp`, the cursor is in
  range and visible.
- [x] 2.7 **Do not touch `reveal_selected`.** Its one consumer is `sidebar::window`, where it scrolls
  the *file list*; folding it into cursor-following would delete the sidebar's reveal behaviour.
- [x] 2.8 Point `anchor_selected` at `cursor_file`. Leave every other writer of `selected` alone, and
  keep `jump_to_collapsed`'s existing exemption from it — that exemption is what preserves
  `selected_dir`, and losing it makes `z` fold instead of unfold.
- [x] 2.9 Tests: motion within the viewport does not scroll; past an edge scrolls exactly enough;
  both ends clamp; jumps top-align **and** place the cursor; `]` from a cursor below the viewport top
  does not go backwards.
- [x] 2.9a `G` reaches the last row **and survives a draw** — press `G`, run a full render, then
  assert the cursor is still on the last row and that row is inside the drawn window. A test that
  only calls `bottom()` passes even with §2.2a's clamp bug present.

## 3. Surviving a rebuild

- [x] 3.1 Capture-and-restore **inside `Session::build_plan`** (`session.rs:194`) — the funnel every
  rebuild reaches. Do **not** enumerate callers. View *construction* (`appcore.rs:62`,
  `session.rs:258`) builds a plan outside it, but on a fresh `ViewState::default()` where there is
  nothing to restore.
- [x] 3.2 Capture **classifies** the row; restore is per class. Not a clamp, and not a blind ladder:
  `Line(Key)` a body row → `find_key`, falling through to `InFile` on its own file;
  `Dir(String)` a folded directory's placeholder or its spacer → `Plan::collapsed_row(dir)`;
  `InFile { file, offset }` any other row of a file → `new_file_start + offset`, or
  `collapsed_row(parent_dir(path))` if the file itself folded away;
  `Above(row)` a banner-region row → keep the index, since banner rows precede every `file_start` and
  never move;
  anything unresolved → the clamped index.
- [x] 3.2a **`Dir` must be its own class.** `file_starts` gets an entry only for a real file
  (`rows.rs:158`); a folded directory pushes `CollapsedDir` + `Spacer` without one
  (`rows.rs:148-153`), so `file_at` on such a row reports whatever real file *precedes* it. Treating
  that as the cursor's file fabricates an anchor — with `a/` unfolded and `b/` folded, a cursor on
  `b/`'s placeholder resolves into `a/`, and when `a/`'s diff lands and grows the cursor ends up
  inside `a/`'s body while the folded-placeholder fallback looks up `a/` (not folded) and returns
  `None`. Only the *old* plan knows the row was a `CollapsedDir` and which directory it was, which is
  why capture classifies rather than restore guessing.
- [x] 3.3 Visibility after a rebuild is §2.6's job, **not** `build_plan`'s: `Session::build_plan`
  takes no viewport height, so calling `scroll_into_view` there would mean a signature change through
  every call site. `build_plan` restores identity; `clamp` restores visibility. Do **not** instead
  clamp the *cursor* into the viewport: `set_layout` top-aligns `scroll` to the file header, so that
  would drag a cursor deep in a file to the viewport edge and throw away the position the rebuild just
  preserved.
- [x] 3.4 Leave `Session::rebuild_plan`'s *scroll* re-anchor (`stream::reanchored` against
  `current_file`) exactly as it is. It answers a different question and its tests must stay green.
- [x] 3.5 Tests: unified→split and split→unified both keep the cursor, **including from a context
  row** — the case an earlier draft broke; a round trip from a removed line returns to that *change*
  (its sibling row is correct, and asserting the exact row would be asserting the tiebreaker this
  design deliberately does not have); a removed and an added line at the same number are
  distinguished; **a cursor on a `Pending` placeholder stays with its own file when an earlier file's
  diff lands and grows** — the ladder's step (b), and the test that fails under a bare index clamp;
  folding the cursor's directory lands it on that directory's placeholder, not on an unrelated file;
  after any rebuild the cursor is inside the drawn window; `App::load_current` (a `<`/`>` view switch)
  restores rather than strands it.

## 4. Rendering

- [x] 4.1 Draw the cursor marker in column `area.x` — the column both `draw_stack` (`:179-183`) and
  `draw_split` (`:230-234`) already leave blank by insetting `inner` to `area.x + 1`. Drawn by the
  two `draw_*` functions directly. **`render_row`, `split_row_line`, `cell_spans`, and `emphasize`
  are not modified and gain no parameter** — which also means `ui/overlays.rs` is not edited and the
  peek stays genuinely untouched.
- [x] 4.1a Mind the offset: the marker is placed per **drawn line**, and the sticky header occupies
  drawn line 0 without being a plan row, so drawn line ↔ plan row differs by 1 exactly when the
  header is pinned. Use `window.iter().enumerate()` and `scroll + i`; do **not** reach for
  `lines.len()`, which is the natural choice and is off by one in precisely that case.
- [x] 4.2 `draw_stack`/`draw_split` use `usable` for the drawn window, consistent with §2.1.
- [x] 4.3 Point the scroll percentage (`status_info`, `ui/frame.rs:303`) at the cursor rather than the
  viewport top. Nothing else references `scroll_pct`/`status_info`, and the existing
  `scroll_pct_tracks_the_given_row_count` (`ui/frame.rs:657`) calls the pure function with literals,
  so it is unaffected.
- [x] 4.4 Tests, on the **existing** `render_to_string` helper (`render_tests.rs:72-84`, reads
  `.symbol()`): exactly one drawn line carries the marker, in both layouts; it is the cursor's row;
  the marker is on the right line when the sticky header is pinned (the §4.1a off-by-one).

## 5. Test migration (the real cost)

- [x] 5.1 `stream.rs`'s own four motion tests. `jump_to_file_lands_on_the_file_start` and
  `scroll_by_clamps_to_the_plan` pass only because they never set a cursor — reseed them deliberately
  rather than leaving them accidentally green.
  `prev_hunk_steps_back_and_clamps_at_the_top` seeds `scroll: 30` with a default cursor and must be
  reseeded on the cursor under §2.5.
- [x] 5.2 Work through the measured sites: `keys_tests.rs` (21 `scroll`, 29 `selected` — the bulk),
  `events.rs` (18/6), `render_tests.rs` (13/8), `view_tests.rs` (3/1), `nav_tests.rs` (2/1),
  `overlays_tests.rs` (2/13), `appcore.rs` (4/1) — including
  `streaming_rebuild_keeps_the_banner_in_view` (`appcore.rs:836`), which writes `scroll = 0` and
  asserts it survives a rebuild. Several **write** `state_mut().selected`.
- [x] 5.3 Where a test asserted a scroll value for an incremental motion, assert the **cursor** and
  the visible range instead — that is the behaviour that actually matters and what the old assertions
  were approximating.
- [x] 5.4 Confirm the peek was not touched at all: `peek_tests.rs`, `peekview.rs`, `peek.rs`, and
  `ui/overlays.rs` should show **no diff**. `ui/overlays.rs` is included because §4.1's marker leaves
  `render_row` and `split_row_line` unchanged; if that file needs an edit, the marker was implemented
  inside the row renderers instead of in the `draw_*` functions.

## 6. Gates (per CLAUDE.md, in order)

- [x] 6.1 `cargo clippy --workspace --all-targets` — zero warnings.
- [x] 6.2 `just crap-ci` — confirm `handle_global_key` is byte-identical and still at 30.
- [x] 6.3 `just coverage` — ≥90% per function. The exposure is functions this change *modifies* that
  already sit near the floor: `land_on_file` 90.91 %, `set_layout` 93.33 %, `toggle_grouping`
  93.75 %. One unexercised added line in the first fails the gate. Everything in `stream.rs` is at
  100 % today, but `top`, `bottom`, `page`, `half_page`, `scroll_to`, `clamp`, `anchor_selected`, and
  `current_file` have **no direct unit tests** — they are covered incidentally from two layers up,
  which is what "no free safety net" means in practice. Nothing new approaches CRAP 30: `render_row`
  18 → ~20, `draw_stack`/`draw_split` gain one branch each.
- [x] 6.4 `cargo fmt --all` last.
