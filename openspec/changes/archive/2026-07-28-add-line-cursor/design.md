## Context

`ViewState` is `scroll`, `h_scroll`, `wrap`, `selected`, `selected_dir`, `collapsed`,
`reveal_selected`, `viewed`. Motion in `src/tui/stream.rs` sets `st.scroll`; `anchor_selected`
re-derives `selected` as the file at the top of the viewport. The renderer draws
`plan.rows[scroll..end]` and passes no selection to `render_row`.

Four facts about the existing code, each checked rather than assumed, and each of which decides part
of the design:

- **`Session::build_plan` (`session.rs:194`) is the only path that *rebuilds an existing* plan.**
  Three call sites reach it directly — `App::build_plan` (`nav.rs:406`), `Session::rebuild_plan`
  (`session.rs:220`), and `App::load_current` (`appcore.rs:167`) — and six more in `nav.rs` reach it
  through the first.
  View **construction** does build a plan by another route: `App::with_launch` (`appcore.rs:62`) and
  `Session::push_entry` (`session.rs:258`) each call `Plan::build_with_banner` inline. Both seed a
  fresh `ViewState::default()`, so there is no cursor to restore, and `push_view` immediately calls
  `load_current` — which rebuilds through the funnel one statement later. Capture-and-restore belongs
  in `build_plan` and nowhere else, but the claim is about rebuilds, not about every assignment to
  `.plan`.
- **The peek reaches `stream::` through exactly one function.** `App::peek_scroll`
  (`peekview.rs:374`) calls `stream::scroll_by`, and everything else the peek does — `peek_page`,
  `peek_half_page`, `peek_hunk` — is built on top of that inside `peekview.rs`. Every other
  `stream::` motion (`page`, `half_page`, `top`, `bottom`, `next_hunk`, `prev_hunk`, `jump_to_file`,
  `jump_to_collapsed`) is called only from `nav.rs`, i.e. only by the stream.
- **`reveal_selected` does not move the stream.** Its single consumer is `sidebar::window`, where it
  scrolls the *file list* and is then cleared. Folding it into cursor-following would delete the
  sidebar's reveal behaviour.
- **`selected` is not derivable.** Around ten sites write it — `select_file`/`select_dir`,
  `sidebar::apply_nav`/`step` (the mechanism behind `{`/`}`, Space, and sidebar `j`/`k`), `nav.rs`,
  mouse clicks. And `select_file` deliberately clears `selected_dir`, because collapsed placeholders
  are reached only by explicit step, click, or fold. Deriving `selected` from the cursor would make
  `z` fold the fallback file's directory instead of unfolding, violating the accepted
  `directory-collapse` requirement.

## Goals / Non-Goals

**Goals:**
- A visible current row that incremental motion moves, with the viewport following.
- Survives the plan rebuilds that already happen constantly.
- Identical capability in unified and side-by-side layouts, with no new keybinding in either.

**Non-Goals:**
- Changing `selected`, `selected_dir`, or `reveal_selected`. They are the sidebar's and the fold
  model's, and they work.
- Changing jump semantics. Jumps keep top-alignment.
- Changing how `Session::rebuild_plan` re-anchors *scroll*. That is `stream::reanchored`, integer
  offset arithmetic against `current_file`, and it stays exactly as it is.
- The peek. It keeps viewport-only motion; see the decision below.
- Selecting a *range* of lines, and mouse selection.

## Decisions

### The unit is the change, not the line

A side-by-side row holds the removed line and the line that replaced it. Those are two lines of code,
but they are **one change**, and a review comment is about the change. So the cursor names the row;
the row is marked as one thing; there is no side for the user to pick and no key to pick it with.

Acting on the cursor anchors to a fixed rule: **the new side when the change has one, the old side
otherwise.** A deletion carries only an old side, an addition only a new one, and a modification
anchors to the new line. That is also the more durable choice independently of simplicity — the old
side is history and may not exist in a later changeset, so `review::capture` re-resolves more
reliably against the new side.

- *Rejected — an `S` key to move the highlight between cells:* it buys precision the anchor does not
  need. Both cells of a pair are the same place in the diff, the comment text carries the rest, and
  the key would have to be documented, bound, tested, and kept inert in unified layout where the two
  lines are already two rows.

**A caveat on "one change", and its escape hatch.** `emit_split_body`'s `flush` (`rows.rs:266-323`)
pairs removals with insertions **by index**, not by correspondence. For a balanced hunk the two cells
really are one edit. For an unbalanced one — two lines squashed into one, say — row *i* carries the
*i*-th removal beside the *i*-th insertion, which may be unrelated, and the surplus rows have one
cell only. So in side-by-side layout the cursor sometimes names a pairing rather than an edit, and
anchoring takes the new side of it.

This is pre-existing rendering behaviour, not something this change introduces, and it has a
one-keystroke escape: `m` switches to unified layout, where every line is its own row and `j`/`k`
reach each one exactly. That is a better answer than a side key, because it is the answer for
*reading* an unbalanced hunk too.

### A row still reports every identity it can carry — for the layout toggle, not for the user

The cursor's identity is a key: `(file, side, line)`, with `side` reusing `review::Side`. This is
machinery, not a user-facing choice. It exists because toggling `m` has to match **one** row against
**two**:

```
  unified:  - let x = 1;   (f, Old, 5)  ┐
            + let x = 2;   (f, New, 5)  ┘  two rows
  split:    Pair(-1, +2)   carries both     one row
```

The mistake in the previous draft was asking a row for *its* key, taking the answer from the
renderer's gutter mapping (`ui/stream.rs:611`, `_ => (new, true)`). That is a display decision. A row
can carry two identities, and a *context* row does:

```
                 unified plan                           split plan
  context     Line { old: 5, new: 7 }              Pair(L{Old,5}, R{New,7})
                carries (f,Old,5)                    left carries (f,Old,5)
                    and (f,New,7)                   right carries (f,New,7)
                      └───────────  the same two keys  ──────────┘
  removed     Line { old: 5, new: None }  →(f,Old,5)  Pair(L{Old,5}, None) →(f,Old,5)
  added       Line { old: None, new: 7 }  →(f,New,7)  Pair(None, R{New,7}) →(f,New,7)
  binary note Line { old: None, new: None}→ none      (not emitted in split)
  all chrome                              → none                           → none
```

So the row-level function is "which keys does this row carry", and lookup is "which row carries this
key". Split→unified and unified→split then match on the same pair, in both directions. Keyed off the
displayed number instead, a cursor on a split row's *left* cell had nothing to match in a unified
plan — and context rows are the majority of a diff, so the failure was both common and asymmetric.

A key is unique within a plan: hunks do not overlap, so a given `(file, side, line)` occurs at most
once. A binary-note row carries no line number and therefore no key, and falls to the index path by
construction rather than by special case.

### Capture and restore inside `Session::build_plan`

Because that function is the sole rebuild funnel, "the cursor survives a rebuild" lives there — but a
bare index clamp is **not** an adequate fallback, and an earlier draft's pseudocode used one. Capture
**classifies** the cursor's row, and restore is per class:

```
enum CursorAnchor {
    Line(Key),                       // a body row: (file, side, line)
    Dir(String),                     // a folded directory's placeholder, or its spacer
    InFile { file: usize, offset },  // any other row belonging to a file
    Above(usize),                    // a row above the first file: the banner region
}
```

- **`Line`** → `find_key`; if the line is gone, fall through to `InFile` on its own file.
- **`Dir`** → `Plan::collapsed_row(dir)`, which already exists.
- **`InFile`** → `new_file_start + offset`. This is the case that matters most in practice: every
  undiffed file is exactly three keyless rows (`FileHeader`, `Pending`, `Spacer` — `rows.rs:158-172`)
  and every file starts undiffed, so parking on a placeholder while an earlier file's diff lands and
  grows is *ordinary*. A raw index clamp there leaves the cursor pointing into whichever file expanded
  underneath it. If the file itself is gone because its directory folded, try
  `collapsed_row(parent_dir(path))`.
- **`Above`** → keep the index. Banner rows sit above every `file_start` and their range never moves.
- Anything unresolved falls to the clamped index.

**`Dir` must be its own class, not an `InFile`.** `file_starts` gets an entry only for a real file
(`rows.rs:158`); a folded directory pushes `CollapsedDir` and `Spacer` without one (`rows.rs:148-153`).
So `file_at` on a `CollapsedDir` row silently reports whatever real file *precedes* it. Treating that
as the cursor's file fabricates an anchor: with `a/` unfolded and `b/` folded, a cursor on `b/`'s
placeholder resolves to a file in `a/`; when `a/`'s diff lands and grows, the offset repair keeps a
row number that now falls inside `a/`'s body, and the folded-placeholder fallback then looks up `a/`
— which is not folded — and returns `None`. The cursor lands in an unrelated file via the bare index
clamp: precisely the failure the whole scheme exists to prevent.

This is also why capture must classify rather than restore guess: only the *old* plan knows that the
row was a `CollapsedDir`, and only it knows which directory.

Something must then make the cursor visible again. Steps 1–3 and `scroll`'s own re-anchor repair
against possibly *different* anchor files, so they can drift apart even when both are individually
right: the viewport top may sit in file 2 while the cursor sits in file 6, and file 2 growing moves
one and not the other.

**That belongs in `stream::clamp`, not in `build_plan`.** `Session::build_plan(&mut self, layout,
grouped)` takes no viewport height, and `scroll_into_view` needs one — putting it there means a
signature change rippling through every call site, which is most of what made the funnel attractive.
`clamp` already takes both the plan and the height, already runs after every fold
(`nav.rs:50,129,145,186`), and already runs on every frame from `reconcile` (`ui/frame.rs:153`), so a
path that forgets it is corrected by the next draw.

So `clamp` gains one invariant covering both jobs it was going to have: **bound `scroll` to the plan,
bound `cursor_row` to the plan, then move `scroll` so the cursor is drawn.** After `clamp`, the
cursor is in range and visible — which is exactly what the spec asserts and what nothing enforces
today. `build_plan` restores identity; `clamp` restores visibility.

This makes the cursor authoritative over `scroll` in steady state, which is intended: after this
change nothing moves `scroll` without also moving the cursor, so the fix is a no-op except after a
rebuild. `streaming_rebuild_keeps_the_banner_in_view` (`appcore.rs:836`) keeps passing — its cursor
sits at 0, which row 0 of the viewport already shows.

- *Rejected — pulling the cursor into the viewport instead (clamping it to `[scroll, scroll+usable)`):*
  it destroys the headline scenario. `set_layout` top-aligns `scroll` to the current file's header,
  so a cursor 500 rows into that file would be dragged to the viewport edge — the layout toggle would
  "preserve" the cursor and then immediately throw it away. The cursor is the more precise position;
  the viewport follows it.
- *Rejected — calling a re-anchor from each rebuild path:* the previous draft enumerated five, missed
  two, and would have been wrong again the next time a caller was added. Enumeration is the wrong
  shape when a funnel exists.

`find_key` is a linear scan, run once per rebuild. Across a hundred-file streaming load that is tens
of milliseconds of cheap matching in total. If it ever shows up, scanning outward from the previous
index makes it effectively O(1), since a streaming rebuild barely perturbs the plan — but do not
write that until it is needed.

### `scroll_by` is not converted; the stream gets `move_cursor_by`

The peek's only door into `stream::` is `stream::scroll_by`. Leaving that alone and adding a sibling
means the peek is untouched — no row↔drawn-line mapping, no `blame_cursor` change, and
`peek_tests.rs`, `peekview.rs`, and `peek.rs` stay green without edits.

**`App::scroll_by` is a different function and does *not* have two callers — it has four.** `j`/`k`
and the arrows (`keys.rs:289-290`), `J`/`K` and Shift-arrows (`keys.rs:285-288`), and the mouse wheel
(`events.rs:250`) all funnel into it. Since incremental motion and scroll gestures must now diverge,
that one method splits in two:

```
App::move_cursor(delta)   ← j/k, arrows            cursor motion, viewport follows to the edge
App::scroll_view(delta)   ← J/K, Shift-arrows, wheel   viewport scroll, cursor dragged along
```

So the change is *not* one line in `nav.rs`: it is a split there plus five call-site edits across
`keys.rs` and `events.rs`. The peek is unaffected either way — its keys call `App::peek_scroll`,
which reaches `stream::scroll_by` on the peek's own state.

**`scroll_view` drags the cursor by the delta actually applied, not the delta requested.** `scroll`
clamps at `max_scroll` while `cursor_row` clamps at `rows.len() - 1` — a gap of `usable - 1` rows — so
shifting the cursor by the requested delta desynchronises them near either end: with 30 rows,
`usable` 10 and `scroll` 18, a wheel notch of +3 moves `scroll` by 2 (clamped) and the cursor by 3,
and the cursor's screen row slides by one per notch. Computing `applied = new_scroll - old_scroll`
and shifting the cursor by *that* keeps the screen row exactly, which is what the accepted
`review-stream` mouse-wheel scenario now promises.

Every other `stream::` motion is stream-only and can become cursor-aware freely, including `page` and
`half_page` — the peek builds its own on `peek_scroll`.

The cost, stated rather than hidden: `j` scrolls in the peek and moves a cursor in the stream. The
peek's cursor is worth doing on its own terms — in blame mode `blame_cursor` (`peek.rs:224`) already
means "the first drawn line at or below the viewport top" and drives both the box title and `Enter`,
so an *invisible* cursor is already there and a visible one would be a real improvement. That is the
follow-up: "blame mode gets a line cursor", not "the peek inherits the stream's".

After this change `scroll_by`'s only caller is the peek; its doc comment must say so, or it rots.

### No stored side at all: `cursor_key` always prefers the new side

Restoring has to search with *one* key, so a two-identity row needs a rule. The rule is the same one
that governs anchoring: **the new side when the row carries it, the old side otherwise.** Nothing is
remembered between rebuilds.

An earlier draft of this section kept a private tiebreaker — remember which key matched, prefer it
next time — to make unified → split → unified land on the row it started from rather than on that
change's sibling row. It does not work as a single field. Nothing writes it on plain motion, so it
goes stale the moment `j` leaves the row that set it, and the next rebuild applies a side chosen for
some *other* row: cursor matched Old on a removed line in hunk A, walks down to hunk B, toggles
layout, and lands on hunk B's removed line even though nothing about hunk B ever chose Old. It is
row-scoped memory stored globally. Keeping it would mean clearing it in every motion function and
every jump — four touch points and a live correctness trap — to remove a one-row wobble on a unit the
whole design treats as indivisible.

So the round trip returns to the same *change*, which may be its other row. That is the behaviour the
spec states.

This also removes a compile problem the tiebreaker had: `ViewState` is `#[derive(Default)]` across 22
construction sites and `review::Side` (`record.rs:27`) derives no `Default`, so the field would have
had to be `Option<Side>` — and deriving `Default` on `review::Side` instead would have been worse,
since it is a serialized on-disk type and a default side lets an `Anchor` acquire one nobody chose.

### Visibility reserves the sticky header's row unconditionally

`draw_stack` (`ui/stream.rs:197-208`) and `draw_split` both pin the current file's header when
`scroll > cf_row` and then draw one row fewer. That is circular for a `scroll_into_view` that is
about to change `scroll`. The resolution is that the guarantee only needs the drawn height never to
be *over*-estimated:

```rust
// Mirrors draw_stack's `content_height`, with sticky assumed on. NO floor:
// `saturating_sub` is what the renderer does, and flooring diverges from it.
let usable = if plan.file_starts.is_empty() { viewport_h } else { viewport_h.saturating_sub(1) };
```

When the header is sticky this is exact. When it is not — the viewport top sits exactly on a file
header, or in the banner region above the first file — one row is reserved needlessly, and the
viewport scrolls one row earlier than strictly required. In the first of those cases that scroll
immediately makes `scroll > cf_row` true, so the reservation becomes correct the moment it is used.
No fixed point, no iteration, no dependence on a value being mutated.

**There must be no `.max(1)` floor.** An earlier draft had one, by analogy with `max_scroll_rows`,
and it breaks the invariant at the one size where it is load-bearing: with `viewport_h == 1` and a
sticky header, `draw_stack` computes `content_height = 1.saturating_sub(1) = 0` and draws **no** body
rows, while a floored `usable` claims one. That is an over-estimate, the exact thing the rule exists
to prevent — and `viewport_h == 1` is the constructor default (`appcore.rs:95`), so it is the
state every view is in before its first draw.

`usable == 0` means nothing is drawable and the guarantee is vacuous, not violated. `scroll_into_view`
returns without touching `scroll` in that case; the first real draw resizes and fixes it.

(A 1-row stream viewport showing a pinned header and no content at all is arguably a rendering bug in
its own right. It predates this change and is left alone; fixing it would make the reservation exact
rather than merely safe.)

- *Rejected — deriving `cf_row` from the cursor to break the circularity:* it changes which header is
  pinned. With the cursor low on screen in file B while the viewport top is still in file A, the
  pinned header would become B's while the content under it is A's, or vanish entirely. The sticky
  header is a property of the viewport top and stays scroll-derived.

### The stream passes `usable` where it passes `viewport_h`; `max_scroll` does not change

`stream::max_scroll` is one function shared by stream and peek call sites, so it cannot itself know
which bound to apply. The differentiation happens **at the call site**: every `stream::` call on the
stream's path passes `usable(plan, viewport_h)` in place of `viewport_h`, and the peek keeps passing
`viewport_h` raw. Nothing inside `stream::` learns the difference.

That is safe because the split is already clean — `clamp` is reached only through `App::clamp`
(`appcore.rs:368`) on the stream's `ViewState`, `scroll_to` only through the stream's jumps, and
after this change `scroll_by` is the peek's alone. The peek hand-rolls its own clamps
(`peekview.rs:151,365,417`) against `active_rows()` and never touches `max_scroll`.

**`App::clamp` is the call site that matters most, and missing it silently reverts the change.** It
runs from `reconcile` (`ui/frame.rs:153`) on *every* draw. Left on raw `viewport_h` while `bottom`
uses `usable`, the two disagree by exactly one row and the clamp wins every frame: `G` sets
`scroll = 11`, the next draw snaps it to 10, and the cursor placed on the last row is then outside
the drawn window and gets dragged back by the cursor clamp. The bug is invisible in a unit test of
`bottom` and only appears once a frame is drawn.

Given that, the stream's bound makes one more row reachable at the bottom than today — the last plan
row, currently hidden under the sticky header. It is always a `Spacer` (verified: every exit arm of
`Plan::build_with_banner` ends a file with one), so the visible change is one blank line, but it is a
real change to `G`/`End` and to a handful of numeric scroll assertions.

`wrap` — where one plan row occupies several physical lines — is explicitly out of scope for the
visibility guarantee.

### `anchor_selected` is the only thing that changes about `selected`

It starts deriving the file from the cursor's row rather than the viewport top, via a new
`cursor_file`. `stream::current_file` stays scroll-derived, because `Session::rebuild_plan` and
`set_layout` use it as the *scroll* re-anchor and the sticky header needs the viewport top's file.
Two functions, two genuinely different questions.

`jump_to_collapsed` keeps its existing exemption from `anchor_selected` — that exemption is what
preserves `selected_dir`, and losing it makes `z` fold instead of unfold.

### The remaining motion decisions, written down rather than left implicit

- **`[` / `]` search from the cursor**, not from `st.scroll` (`stream.rs:118,128`). Left
  scroll-relative, `]` can find a hunk *behind* the cursor and move it backwards.
  `prev_hunk_steps_back_and_clamps_at_the_top` seeds `scroll: 30` with a default cursor and must be
  reseeded — the test is about hunk stepping, not about which field it reads.
- **`jump_to_collapsed` sets the cursor**, like every other jump. It moves the viewport on `fold_dir`,
  `fold_all`, `toggle_viewed`'s auto-collapse, and every step onto a placeholder; without this, `z`
  lands the viewport and leaves the cursor stale off-screen, and the next `j` snaps the view back.
- **`stream::clamp` clamps `cursor_row`.** It is the every-frame net (`ui/frame.rs:153`) and the
  post-fold cleanup (`nav.rs:50,129,145,186`), and it is the cheap place that catches anything the
  rest of this misses.
- **`G` puts the cursor on the last row.** That row is a `Spacer` — every file's plan ends with one
  (`rows.rs:223`) — which is the honest meaning of "the end". Not worth a rule to skip it.
- **`App::top` does not change signature.** `App::bottom` (`nav.rs:212`) already reads
  `self.viewport_h` internally; `top` can too. Only the `stream::` functions take the height.

### The cursor is a marker in the free left column, not a background

Both `draw_stack` and `draw_split` inset their content by one column — `inner.x = area.x + 1`
(`ui/stream.rs:179-183`, `230-234`) — so column `area.x` is already blank in both layouts. The cursor
is drawn there, by the two `draw_*` functions directly. The row keeps whatever background it had.

- *Rejected — replacing the row background with `sel_bg`, the sidebar's idiom:* it fails four ways,
  each verified.
  - **Emphasis punches holes in it.** `emphasize` (`ui/frame.rs:47-88`) sets a background on the
    word-diff span, and in ratatui a span's own style patches over the line's. A cursor row with
    intra-line emphasis would show `add_emph` where the highlight should be.
  - **Split has no line-level background at all.** `split_row_line`'s `Pair` arm never calls
    `.style()`; `cell_spans` (`ui/stream.rs:398`) bakes the background onto every span individually.
    The two layouts would need two different mechanisms.
  - **The empty half and the divider have no background path.** `cell_spans` early-returns
    `Span::raw(" ".repeat(width))` for a missing cell (`:406-408`), and the `│` divider is styled
    with a foreground only — so a `Pair(Some, None)` row would highlight as a ragged half-block.
  - **It would be fainter than the row it replaces.** `add_bg`/`del_bg` are `blend(.., 0.84)`;
    `sel_bg` is `blend(.., 0.12)` (`theme.rs:200-207`). The cursor on a changed line would look *less*
    marked than an ordinary added line.

The marker also makes the change substantially smaller. `render_row`, `split_row_line`, `cell_spans`,
and `emphasize` are all untouched — no signature change — which means `ui/overlays.rs` needs no edit
either, and the peek is genuinely untouched rather than untouched-except-for-a-mechanical-parameter.

And it is testable with the harness that already exists: the marker is a glyph, so
`render_to_string` (`render_tests.rs:72-84`, which reads `.symbol()`) can assert which row carries it
and that no other row does. Asserting a *background* would have required cell-style inspection that
exists nowhere in `render_tests.rs` — its one `cell.fg` read (`:174`) is a debug ANSI dump whose
assertion compares theme values, not rendered cells.

The one care needed: the marker column is drawn per **drawn line**, and the sticky header occupies
drawn line 0 without being a plan row. The offset between drawn line and plan row is therefore `1`
when the header is pinned and `0` otherwise — the same trap as reaching for `lines.len()` inside the
row loop, which is off by one exactly when the header shows.

## Risks / Trade-offs

- **The test migration is the bulk of the work.** Measured: `keys_tests.rs` 21 `scroll` / 29
  `selected`, `events.rs` 18/6, `render_tests.rs` 13/8, plus `stream.rs`'s four motion tests. Several
  *write* `state_mut().selected`. → Its own task with an enumerated list, not fallout. There is no
  free safety net here; the guarantee has to be rebuilt.
- **`j` no longer scrolling is a user-visible change** to the most-used key. → Stated in the proposal
  rather than discovered.
- **`j` means two things while the peek is out of scope.** → Accepted deliberately; the alternative is
  a second visibility model for a renderer that drops rows.
- **`viewport_h` is 1 until the first draw.** → `usable` carries the same `.max(1)` guard
  `max_scroll_rows` already has.
- **"cursor" now means four things** in this module tree (view stack, selected file, blame line, and
  this). → Named `cursor_row`, and the name is load-bearing: `self.session.cursor` and
  `self.state().cursor_row` in one function would otherwise be a live bug. At spec level the noun is
  **the line cursor**, since the accepted `directory-collapse` spec already calls the selection "the
  cursor".
- **The scroll percentage** reports the viewport top, so at the bottom of a file it reads under 100%
  with the cursor on the last row. → Switch it to the cursor; it is what the reader is asking about.
- **`ViewState.cursor_row` is dead for the peek**, which shares the struct. → A doc comment on the
  field, and the follow-up that gives it a meaning.

## Open Questions

None.

Considered and deliberately left out: a cursor readout (`f.rs:+7`) in the status line. Useful, and
`tui-review` may well want it, but with no side to disambiguate, the highlighted row already says
everything the reader needs.

---

## What the earlier drafts got wrong

Kept so a fourth pass does not rediscover it. Two adversarial reviews against the code produced
nineteen findings across two drafts; all are resolved above, but four of these are the ones that
would otherwise be repeated.

1. **Keying a row off the number it displays.** `ui/stream.rs:611` maps a context line to the new
   side for the *gutter*. Using that as identity broke split→unified for every context row while
   leaving unified→split working, so the bug was asymmetric and read as flaky.
2. **Enumerating rebuild paths instead of finding the funnel.** Five listed, seven exist, one
   function underneath them all.
3. **Treating the peek as a smaller instance of the same problem.** It is a different one: its
   renderer drops rows, so plan index ≠ screen line and even `max_scroll` over-counts.
4. **Asserting a spec requirement the change does not implement.** "File actions act on the cursor's
   file" contradicts both this design's own non-goal and the accepted `directory-collapse`
   requirement, which makes file actions *inert* on a collapsed placeholder — enforced by
   `selected_file()` returning `None` (`view.rs:48`) and checked in `toggle_viewed` (`nav.rs:426`)
   and `open_peek_*` (`peekview.rs:42,65,217`).
5. **Promoting an implementation detail to a feature.** Two drafts argued over whether a key should
   move the cursor between a split row's two cells, and one of them reversed the answer twice. Both
   missed that the reviewed unit is the *change*, not the line: the two cells are one change, so
   there was never a choice to expose. What the two identities are actually for is matching one row
   against two across a layout toggle — machinery, which needed no key and no requirement.
