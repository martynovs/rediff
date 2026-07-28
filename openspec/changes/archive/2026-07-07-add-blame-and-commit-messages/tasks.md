## 1. Dependency + blame spike

- [x] 1.1 Enable the `blame` feature on `gix = "0.83"` in `Cargo.toml`; confirm the workspace still builds.
- [x] 1.2 Spike `gix::blame` on a real file at HEAD in an isolated `git/blame.rs` test: confirm the entry point and result shape (per-line commit id, author, time) at 0.83, and that committed-rev blame attributes every line. Record the API shape used. — gix 0.83 → gix-blame 0.13; use `Repository::blame_file(&BStr, suspect: ObjectId, blame_file::Options)`; `Outcome.entries` are hunks (`start_in_blamed_file`, `len`, `commit_id`) expanded per-line.

## 2. Git layer

- [x] 2.1 Add `git/blame.rs`: a `blame_file(repo_dir, rev, path) -> Vec<BlameLine>` that attributes each line of the file at `rev` to its last-modifying commit (committed content only). Resolve `rev` from the view (HEAD for local/staged, the commit for a commit view, target for a range).
- [x] 2.2 Add a by-SHA full-message lookup in `git/commits.rs` (e.g. `commit_message(repo_dir, sha) -> CommitMessage { sha, author, date, body }`) returning the full body, not just the summary.
- [x] 2.3 Re-export the new functions/types from `git/mod.rs` (declarations/re-exports only).
- [x] 2.4 Unit-test 2.1 and 2.2 against the crate's own repo / a tempfile multi-commit repo (mirror the existing `commits.rs` test fixture).

## 3. Model + age/format helpers

- [x] 3.1 Add a `BlameLine` attribution type (sha, author name, commit time, plus derived run-start flag and color key) in the model/git layer; index-aligned to file lines.
- [x] 3.2 Add a pure `relative_age(now, commit_time) -> String` helper implementing the compact ladder (hours/days integer; months/years one decimal only for single-digit integer part; 12 months → years). Keep it a small pure function.
- [x] 3.3 Add a pure run-collapsing + per-commit color-key helper (mark each line as run-start when its SHA differs from the previous; stable color key from SHA hash).
- [x] 3.4 Exhaustively unit-test 3.2 (every row of the age table incl. boundaries 23h/1d, 29d, 9.x→10m, 11m, 12m→1.0y, 9.x→10y) and 3.3 to the ≥90% floor.

## 4. Commit-message popup overlay

- [x] 4.1 Add `Overlay::CommitMessage` (SHA, fetched body, scroll offset) to `app/types.rs`; wire it into the `Mode`/overlay model.
- [x] 4.2 Add open/scroll/confirm/dismiss methods in `app/overlays.rs`: open fetches the body by SHA (task 2.2); confirm calls the existing `open_commit`; dismiss restores the base.
- [x] 4.3 Render the popup (SHA · author · date · scrollable body) in `tui/ui/overlays.rs`, sized/centered like the other overlays.
- [x] 4.4 Route keys to the popup in `runtime/keys.rs` (scroll, `Enter` confirm, `Esc` dismiss); ensure mouse does not leak through (mode-routing).
- [x] 4.5 Tests: open-by-SHA populates the body; confirm switches the view; dismiss returns to the exact base.

## 5. Picker `Tab` → popup

- [x] 5.1 In the palette key handler, when the open palette is the commit picker, bind `Tab` to open the commit-message popup for the highlighted commit (over the picker); leave `Enter`/number selection picking directly.
- [x] 5.2 Ensure dismissing the popup returns to the picker with its query/results/highlight intact.
- [x] 5.3 Tests: `Tab` opens the popup over the picker; dismiss restores the picker; `Enter` still picks directly.

## 6. Commit-message banner

- [x] 6.1 Carry the commit message body for a `ViewKind::Commit` view (fetch on commit-view load; store on the view entry/kind).
- [x] 6.2 In the plan builder, prepend synthetic banner rows (header + wrapped body) ahead of the first file for commit views only; no banner for local/staged/range.
- [x] 6.3 Ensure the banner scrolls away with the stream and the scroll-percentage/row-count logic stays correct.
- [x] 6.4 Tests: banner rows present for a commit view and absent otherwise; banner precedes the first file.

## 7. Blame as a third Peek mode + gutter

- [x] 7.1 Extend `PeekMode` to `{ Content, Diff, Blame }`; make `Tab` cycle all three; hold a `Vec<BlameLine>` on the `Peek` parallel to file lines.
- [x] 7.2 Add an "ensure blame loaded" path that computes blame off the UI thread via the `Loader`/progress pattern and fills the peek's attribution array on completion; show a loading state until then.
- [x] 7.3 Render the 12-col attribution gutter in blame mode (`tui/ui/`): `name` left / `age` right, ≥1 space, name = `12 − 1 − age_width`, vertical rule; print the token only on run-start lines (blank otherwise); paint the token with the per-commit color.
- [x] 7.4 Show the cursor line's full SHA + summary in the peek header (extend the `Peek::label` mechanism), updating as the cursor moves.
- [x] 7.5 Bind `Enter` on a blame line to open the commit-message popup for that line's commit (no direct jump); confirm from the popup switches the view.
- [x] 7.6 Tests: gutter layout/run-collapse/alignment; header tracks the cursor line; `Enter` opens the popup, not the diff.

## 8. Open key + keymap/hints/help

- [x] 8.1 Bind `b` (global) to open the peek for the selected file directly in blame mode; inert on a collapsed-directory placeholder.
- [x] 8.2 Update the `keymap.rs` help catalog and status-line hints: `b` blame, `Tab` (read message) in the commit picker, blame-mode peek hints, and the commit-message popup hints. Keep the consistency test green.
- [x] 8.3 Tests: `b` opens blame from either focus and is inert on a placeholder; the keymap consistency test passes with the new keys.

## 9. Gates

- [x] 9.1 `cargo clippy --workspace --all-targets` — zero warnings (pedantic).
- [x] 9.2 `just crap-ci` green; bring every new/changed function to the ≥90% per-function coverage floor (verify with `just coverage`); refresh the CRAP baseline only for genuinely grandfathered entries, noting it in the commit message.
- [x] 9.3 `cargo fmt --all` as the final step before staging.
- [x] 9.4 `just install` and dogfood the new keys (`b`, picker `Tab`, commit banner) on this repo.
