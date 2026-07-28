## Why

rediff is a read-only viewer: review state (viewed flags, everything the reader concluded) lives
in memory and dies with the process, and there is no way for anything outside the TUI to learn what
a human thought of a change. That closes the door on the workflow this project is increasingly used
for — an **agent** edits the working tree, a **human** reviews the result, and the agent needs the
feedback back in a form it can act on.

The return channel is the expensive decision, not the surfaces that write to it. Its on-disk format
is persisted, appended to by more than one writer (a TUI review session, a local web page), and read
by a machine; getting the anchor and delivery semantics wrong is a migration, not a patch. So this
change lands **only the headless store** — the file, the records, the anchoring rules, and the
delivery semantics — with no CLI subcommand, no server, and no UI. The surfaces that use it are
separate changes on top.

## What Changes

- **One log file per worktree: `<worktree>/rediff.jsonl`.** Append-only JSON Lines. No directories,
  no sidecar state, nothing written inside `.git`, nothing in a global cache. Cleanup is `rm`.
- **One record type for feedback.** A `thread` carries an optional anchor, a body, and an optional
  replacement. **An anchored thread is a line comment; an unanchored thread is a review-level
  comment** — there is no separate "verdict" type. A `submit` record closes a round and carries the
  human's parting instruction (a config preset's text, editable before sending).
- **Anchors are self-contained.** A thread records the anchored line's text plus a few lines of
  surrounding context, inline. Re-anchoring against a later changeset is a windowed match — no blob
  store, no snapshots, nothing to track or garbage-collect.
- **Re-anchoring never drops.** An anchor resolves as `attached`, `shifted`, `detached`, or
  `unresolved` (the file is present but not yet diffed — distinct from detached, so a streaming load
  never claims the commented code is gone). A detached thread is still delivered, carrying the code
  it was written against.
- **Rounds are per-file content hashes.** Opening a round records `{path: hash}` for the changeset,
  so a later pass can answer "which files changed since my last look" without storing any content.
- **Delivery is drain-once, with a full replay available.** Undelivered records are everything after
  the last `drained` marker; draining appends a new one. A full replay is a filter over the same log,
  not a second mechanism. No cursor is handed to the caller.
- **The serve lifecycle is recorded, not interpreted.** Start and stop records carry the process id,
  bound port, and URL, and the fold reports whether a start is still unpaired. Deciding whether that
  process is *running* — and whether to rebind — belongs to the code that owns server processes, not
  to the store.
- **rediff excludes `rediff.jsonl` from its own output**, unconditionally — from every changeset the
  loader produces, and from the pager and external-diff renderers, which git can hand the log
  directly. Otherwise the diff reviewer reviews its own review log.

## Capabilities

### New Capabilities

- `review-store`: the append-only per-worktree review log — its path, record schema, review and
  round lifecycle, self-contained anchors and their re-anchoring rules, drain-once delivery with
  full replay, and the recorded serve lifecycle.

### Modified Capabilities

- `changeset-loading`: the review log is never itself review material — the loader omits
  `rediff.jsonl` from every changeset it produces.
- `diff-pager`: the same exclusion for the two non-interactive renderers, which post-process git's
  own output and never open a repository, so git can hand them the log directly.

## Impact

- **Dependencies**: add `serde_json` (the workspace has `serde` with derive but no JSON codec) and
  `xxhash-rust` (`xxh3` feature — `no_std`, no transitive dependencies) for the round content hash.
  Nothing else: the append lock uses std `File::lock`, and timestamps reuse `gix::date` rather than
  adding a date crate. Notably **no `libc`** — the workspace denies `unsafe_code`.
- **New module** `src/review/` (`mod.rs` declarations + re-exports only, per the module convention):
  record schema, log path/append/replay, anchor resolution, round hashing, drain selection, and the
  serve lifecycle records.
- **`src/git/`**: enumeration filters the log path out of the changeset, at the one funnel every
  source and both callers pass through. `DiffFile` gains `content_digest`, taken from the raw
  new-side bytes at diff time, so a binary file still has a fingerprint rounds can compare.
- **`src/pager.rs`**: the same exclusion for `pager` and `external`.
- **Effectively no user-visible surface.** This change is a library layer and its tests; nothing in
  the TUI, the CLI, or a browser changes. The one exception is the pager exclusion above, which
  suppresses a file the user would not want rendered anyway. Keeping the store headless was
  deliberate — it let the format be designed against tests rather than a UI deadline.
- **Follow-on changes** (each consuming this store, none blocking each other once it lands):
  `review-cli` (`rediff review`, `rediff feedback [--all]`), `tui-review` (capture and reply in the
  TUI), `web-render` (spans → HTML, local server), `web-app` (the browser review surface).
- **Gates**: clippy pedantic clean, the CRAP gate with the ≥90% per-function coverage floor, then
  `cargo fmt --all`.
