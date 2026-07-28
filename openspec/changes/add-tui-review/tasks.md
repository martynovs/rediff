# Tasks

Capturing review points in the TUI the human already uses, so an agent can grab them with
`rediff feedback`. Human-initiated: a review is opened by the first comment, never by browsing.

> **`add-line-cursor` has landed.** `ViewState.cursor_row` exists, and §1 is now mostly *deletion*:
> the anchor tuple this change was going to compute is already computed.

## 1. Cursor → anchor

- [x] 1.1 There is no `anchor_at` to write, but the bridge is **not** a one-liner. `rows::cursor_key`
  gives `(usize, Side, u32)` where the `usize` is a `Changeset::files` **index**
  (`rows.rs`), while `review::capture(file: &DiffFile, side: Side, line: u32)`
  (`review/anchor.rs:92`) wants the file itself. So the bridge is
  `cursor_key(..)` → `cs.files.get(fi)` → `capture(..)`, with **two** failure points: no key on this
  row, and `capture` returning `None`. An earlier revision of this task claimed the two signatures
  matched; they do not.
- [x] 1.2 The side rule is already decided and implemented — the new side when the row carries one,
  the old side otherwise — so there is no "prefer the side that has text" logic left to write. Note
  the old justification for that logic was wrong anyway: a deleted file's rows carry only `Old` by
  construction (`rows.rs`), so `cursor_key` cannot pick a textless new side.
  `rows::row_keys` already has tests for every row shape.
- [x] 1.3 Test the bridge, and cover `capture`'s `None` arm **deliberately**: from a real cursor it is
  close to unreachable (a binary file's row carries no key, so the key is inert before `capture` is
  reached; an undiffed file shows `Row::Pending`, also keyless). That makes it a §9.3 coverage-floor
  trap — reach it from a direct unit test, not through the TUI.
- [x] 1.4 `cursor_key` normalises a two-sided row to the new side, so commenting on the *old* half of
  a side-by-side pair is not expressible. Deliberate, per `line-cursor`'s accepted "names a change,
  not a side of one"; `m` (unified layout) is the escape hatch, where the halves are separate rows.
  **The `tui-review` delta spec must stop saying the anchor "names the side the cursor is on"** —
  in split there is no such side.

## 2. Opening the review lazily

- [x] 2.1 `ensure_review(log, cs, target, state, frozen) -> Result<...>` as a **pure body** taking `log`/`cs` as parameters, with I/O in a thin shell — the `reviewcli` precedent, and what makes its error arms coverable. It needs `frozen`: `open_round` takes it (`review/round.rs`) and `request` deliberately passes `false` (`reviewcli/request.rs`).
- [x] 2.1a Refuse while the view is still loading (`open_round` rejects a partially-diffed changeset and `capture` has no text for an undiffed file), with a message rather than an `InvalidInput`.
- [x] 2.1b **Refusing a *filtered* view needs plumbing that does not exist.** `LoadRequest` (`git/types.rs`) carries no filters; they live in `cli::Resolved.filters`, are passed to `tui::run`, consumed by `git::apply_stub_filter`, and then dropped — nothing on `App`/`Session`/`ViewEntry` records that the view was narrowed. So `ViewEntry.req` alone cannot answer "is this filtered". Either carry the filtered-ness onto the view alongside `req`, or drop the requirement and accept that a filtered view opens a review hashed over a subset — which makes the next unfiltered `request` report every excluded file as added. Decide before §2; do not discover it mid-build.
- [x] 2.2 Refuse when a review is open over a different target **and** `!safe_to_replace()`, naming both — the same rule `request` uses, and for the same reason (an anchor would otherwise be captured against a diff the review is not about).
- [x] 2.3 Encode the target with `reviewcli::target::encode` from `ViewEntry.req`, which retains the exact `LoadRequest`. **Remove its `#[expect(dead_code)]`** in the same commit or the unfulfilled expectation fails the zero-warning gate. No `Session` accessor is forced — `App.session` and `ViewEntry.req` are both `pub`. `req` is `Some` in every production path; only `push_test_view` passes `None`, so that is the case to define behaviour for.
- [x] 2.3a Resolve the log path once at launch from a discovered repository's worktree root, not from `Session.repo_dir` (the invocation directory) — `rediff` run inside `src/` would otherwise write `src/rediff.jsonl`, which the loader does not filter out. Nothing needs exporting: `Log::at_worktree`, `log_path` and `log_path_in` are already public, and `reviewcli::run::open_log`'s body is three lines of them.
- [x] 2.4 Emit the ignore hint once, on `Opened::Fresh`, via `App.flash` — and set it **after** the comment overlay pops, since `draw_status` suppresses the flash while any overlay is shown.
- [x] 2.5 Tests: browsing writes no log; first comment opens review + round; second attaches; agent-opened review over the same target is joined; mismatched target refused while pending and fresh once delivered; still-loading refused; filtered view refused.

## 3. Comment input overlay

- [x] 3.1 `Overlay::Comment` carrying the anchor (or `None` for review-level), the text buffer, and the thread id when editing. Follow `Palette`'s text entry (push-char/pop-char, one line, `Enter` confirms).
- [x] 3.2 Confirm appends a `Thread`; escape discards and records nothing.
- [x] 3.3 Render in `ui/overlays.rs`, **and add both new overlays to `ui::frame::paint`'s dispatch chain** — it is an `if`-chain, not an exhaustive match, so an omission compiles clean and silently draws nothing.
- [x] 3.4 Tests: confirm records; dismiss records nothing and restores the base exactly; input is captured and does not leak to the diff beneath.

## 4. Seeing what has been said

- [x] 4.1 Fold the log into a per-`(path, side, line)` index of live threads, resolved against the current changeset. Rebuild **on our own appends** — nothing watches the file, and replaying plus re-resolving on the 100 ms poll tick would re-read the log and re-split every referenced file at 10 Hz.
- [x] 4.2 Gutter marker on commented lines in `ui/stream.rs`, for both layouts. `area.x` is taken —
  `add-line-cursor` put the cursor marker there. But the line-number gutter has a spare column:
  both `render_row` and `cell_spans` emit `format!("{num:>4} ")`, and that trailing space is a
  separator nothing uses, sits in the fixed non-panning prefix, and exists in **both** layouts. Use
  it. (A line number of five or more digits eats it — decide what wins there.)
- [x] 4.3 `Overlay::Threads`: the review's threads with body, anchor, and state; `Enter` jumps to the
  anchor — and **must move `cursor_row`, not just `scroll`**. `stream::clamp` runs every frame and
  scrolls the viewport back to the cursor, so a scroll-only jump snaps straight back; this is the
  hazard `jump_to_collapsed` documents. `rows::find_key` already resolves an anchor key to a row, so
  it is `find_key` + `scroll_to` + set `cursor_row`.
- [x] 4.4 Distinguish `Resolution::Detached` from `Resolution::Unresolved` in the list: during the streaming window every thread in an undiffed file is `Unresolved`, and calling those "detached" is the alarming lie that variant exists to prevent.
- [x] 4.5 Tests: marker appears on the right line in both layouts; jump moves the cursor; detached threads listed.

## 5. Edit, retract, resolve, submit

- [x] 5.1 Edit reopens the input pre-filled and appends a superseding record with the same id.
- [x] 5.2 Retract appends `deleted`; resolve appends `resolved`. Neither rewrites anything.
- [x] 5.3 Submit: pick a verdict preset, edit the text, append a `Submit` closing the round.
- [x] 5.4 **Introduce** `[[verdict]]` presets (name + text) in `config.toml` with sensible defaults; they do not exist yet. `Config::load` discards the whole file on a parse error, so a malformed preset must not cost the user their theme and layout — parse presets tolerantly or separately. Add a `theming-and-config` delta, since the config file gains a top-level array.
- [x] 5.5 Tests: edit supersedes and both records remain; retracted thread is not delivered but stays on disk; resolved thread is still delivered, flagged; submitting closes the round and records the *edited* text with the preset name; submitting works with only a review-level comment.

## 6. Keys, help, and routing

- [x] 6.1 Route the new keys through a **new** `handle_review_key`, not by extending `handle_global_key` — that function is at CRAP **30** against a threshold of 30, and the review keys are inert outside a review session anyway.
- [x] 6.2 Add every new binding to the `keymap.rs` catalog **and its `ALL_TABLES` list** — the consistency test asserts documented keys appear in tables, not the reverse, so nothing fails if a new key goes undocumented. Pick from what is free (`c C t s v` are all taken); the design proposes `a A e x o n y`.
- [x] 6.2a Check the help overlay still fits: `help_column` pads the left column's keys to **13** and the right's to 8 (`ui/overlays.rs`), and the box height is clamped to the body. The right column is 19 rows today, so seven more clip on an 80×24 terminal.
- [x] 6.3 Keys are inert outside a review session — gate on `Session::is_review()` itself, **not** on a list of view kinds: `rediff review <rev>` is a `ViewKind::Commit` view that *is* a review session.
- [x] 6.4 Tests: catalog consistency passes; each key is inert in a browse view and live after `R`.

## 7. Persisting reviewed state

- [ ] 7.0 **Prerequisite: make the log format tolerate unknown variants.** Add `#[serde(other)] Unknown` to `Record` and ignore it in `apply`. Verified that serde supports this on an internally-tagged enum. Without it, any new variant is counted `unparsed` by another build, pinning `safe_to_replace()` false and wedging `rediff request` on a log with no feedback in it. Ship this before 7.1, and test that a synthetic `{"t":"viewed",...}` line folds to nothing rather than incrementing `unparsed`.
- [ ] 7.1 Reviewed files then get their **own** `Record` variant. There is no "optional field on an existing record" to use: every existing variant has a lifecycle effect — `Open` resets the state, `Thread` becomes undelivered feedback (which would make every `v` press something the agent must drain, and is exactly the `request` wedge), `Submit` closes a round, `Round` duplicates one, `Serve`/`Close` pair a server. Only `Drained` is fold-inert and its meaning is the opposite.
- [ ] 7.2 Record only while a review is **already** open — `v` is the most-pressed key in the app, and writing on it would create a log exactly as lazily opening exists to avoid.
- [ ] 7.3 Restore by **path**, not index: `ViewState.viewed` is positional over `cs.files` and is seeded at construction, before any log is read, so restoration is a later step keyed on path.
- [ ] 7.4 Tests: marks survive a reopen; marking with no review open writes nothing; a reordered changeset restores the same files, not the same positions.

## 8. End-to-end

- [ ] 8.1 A PTY test (`tests/tui_pty.rs` has the harness): open the TUI on a dirty worktree, comment on a line, quit — then assert `rediff feedback` returns that comment with the right anchor. This is the whole point of the change and the only test that proves the two halves meet.
- [x] 8.2 A falsifiable regression check, not a suite re-run: assert `rediff diff` on a worktree with a review log open still lists exactly the files it did before, and that the log is absent from it. `git::enumerate` already drops `rediff.jsonl`, so this is a guard against regression, not new work.

## 9. Gates (per CLAUDE.md, in order)

- [x] 9.1 `cargo clippy --workspace --all-targets` — zero warnings.
- [x] 9.2 `just crap-ci` — check `handle_global_key` did not move; it has no headroom.
- [x] 9.3 `just coverage` — every new function ≥90%. The hard ones are `ensure_review`'s error arms: `repo_dir`/`req` absent, `TargetError::AmbiguousRange` from `encode`, `open_round`'s not-fully-diffed refusal, and an append failure.Construct each deliberately; the pure-body/thin-shell split from 2.1 is what makes them reachable.
- [x] 9.4 `cargo fmt --all` last.
