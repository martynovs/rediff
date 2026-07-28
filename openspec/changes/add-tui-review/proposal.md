## Why

Everything up to here has built a return channel with nobody at the human end. The store records
feedback, the CLI drains it, and no surface can write a word of it. `rediff request` opens reviews
that cannot be answered.

This change lets a human leave review points in the TUI they already use to read diffs, and an agent
pick them up with `rediff feedback`. It is the change that makes the previous two useful.

**`add-line-cursor` has landed**, so the premise now holds: `ViewState.cursor_row` names a row and
`rows::cursor_key` turns it into the `(file, side, line)` triple `review::capture` takes. What this
change once had to build for itself — resolving a cursor to an anchor, and choosing a side — is
already built and tested.

## What Changes

- **The human opens the TUI; the review follows.** A review is *not* required to exist first.
  `rediff request` is the agent asking to be reviewed; this is the human reviewing whether or not
  anyone asked. The TUI opens or attaches to the worktree's review **lazily, on the first comment**,
  so browsing a diff still writes nothing.
- **Comment on the line under the cursor.** A key opens a small input; the comment is anchored to
  that file, side, and line using the existing `review::capture`, and appended to the log. The
  cursor and its side come from `add-line-cursor`; the row it names (`Row::Line`, `SplitCell`)
  carries the rest.
- **Comment on the review as a whole**, with no anchor — the unanchored thread the store already
  models as the verdict.
- **See what has been said.** Commented lines are marked in the gutter, and a list overlay shows
  every thread on the current review with a jump to its anchor.
- **Edit, retract, and resolve** an existing thread, all of which the store already models as new
  records rather than mutations.
- **Submit the round.** A verdict picked from configurable presets, editable before sending, which
  closes the round exactly as `Submit` does for any other surface.
- **Commenting is confined to review sessions** — whatever `Session::is_review()` already says,
  which includes `rediff review <rev>` (a *commit* view that is nonetheless a review session), with
  `R` still promoting a browse view. The rule is that predicate, not a list of view kinds.
- **A view that disagrees with the open review refuses to take comments**, naming both, rather than
  anchoring against a diff the review is not about.
- **Commenting waits for the load.** The TUI streams diffs in; a round cannot be opened over a
  half-loaded changeset and an undiffed file has no text to anchor into. Commenting says so rather
  than failing with an I/O error.
- **A filtered view and reviews interact badly**, and the mechanism to detect one does not exist yet:
  `LoadRequest` carries no filters and nothing on the view records that it was narrowed. Either the
  filtered-ness gets plumbed onto the view, or the rule is dropped and a filtered view opens a review
  hashed over a subset — which makes the next unfiltered request report every excluded file as added.
  Decided in tasks §2, not during implementation.

## Capabilities

### New Capabilities

- `tui-review`: capturing review points in the TUI — anchoring a comment to the cursor's line,
  review-level comments, the thread list and gutter markers, edit/retract/resolve, and submitting a
  round with a verdict.

### Modified Capabilities

- `mode-routing`: a comment input and a thread list join the overlay **stack**. The existing
  requirement says overlays never stack, which is already false of the code (`CommitMessage` is
  pushed over `Palette`); this change corrects it as well as adding to it.
- `viewed-tracking`: reviewed state is written to the review log **while a review is open**, so a
  review resumed after a restart remembers which files were already read. Not unconditionally: `v` is
  live in every review session, and recording it eagerly would create a log on the most-pressed key —
  the very outcome lazy opening exists to avoid.

## Impact

- **`src/tui/`**: reading `ViewEntry.req` (which does retain the exact `LoadRequest`) means removing
  its `#[expect(dead_code)]`, or the unfulfilled expectation fails the zero-warning gate; `Session`
  needs an accessor for it. The log path must come from a discovered repository's worktree root, not
  `Session.repo_dir` (the invocation directory). Plus a new comment-input overlay and thread-list overlay (`app/overlays.rs`,
  `ui/overlays.rs`); gutter markers in `ui/stream.rs`; cursor-row → anchor resolution against
  `rows.rs`; key routing and the `keymap.rs` catalog, whose consistency test must stay green.
- **`src/review/`**: consumed as-is for capture, append, and fold. Reviewed state is stored as an
  **optional field on an existing record**, not a new `Record` variant: the format is additive-only
  by field, and a new variant would read as `unparsed` in any other build, permanently pinning
  `safe_to_replace()` false and wedging `rediff request`.
- **`src/reviewcli/`**: the target encoder is reused so a TUI-opened review is indistinguishable
  from an agent-opened one — the same `rediff feedback` drains either.
- **`src/config.rs`**: verdict presets (`[[verdict]]` name + text) are **introduced** here — they do
  not exist yet, and `review-store` deliberately rejected a fixed verdict enum in their favour. Note
  `Config::load` silently discards the whole file on a parse error, so a malformed preset must not
  cost the user their theme.
- **Testing**: the TUI has an established harness (`tui/testutil.rs`, the runtime `*_tests.rs`
  modules, and `tests/tui_pty.rs` for real-terminal behaviour). Anchoring and the fold are pure and
  unit-testable; the overlays follow the existing overlay tests.
- **Gates**: clippy pedantic clean, the CRAP gate with the ≥90% per-function coverage floor, then
  `cargo fmt --all`. Note `handle_global_key` already sits **at** CRAP 30, so new keys must not be
  routed by extending it in place.
