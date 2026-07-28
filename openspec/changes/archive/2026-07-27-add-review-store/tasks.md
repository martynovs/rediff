# Tasks

The headless review store: one append-only log per worktree, self-contained anchors, rounds as
content hashes, drain-once delivery. No CLI, no server, no UI — those are follow-on changes.

## 1. Dependencies + module scaffold

- [x] 1.1 Add `serde_json` and `xxhash-rust = { version = "0.8", features = ["xxh3"] }` to `Cargo.toml`. **No `libc`** — the workspace denies `unsafe_code`, and the store no longer probes process liveness; the append lock uses std `File::lock`. Confirm the workspace still builds and no default-feature kitchen sink comes in (`xxhash-rust` should pull nothing transitively). <!-- serde_json was already in Cargo.lock transitively; xxhash-rust 0.8.18 added and pulls nothing (`cargo tree -p xxhash-rust` = one line). No libc. -->
- [x] 1.2 Create `src/review/mod.rs` as **declarations + re-exports only** (per the module convention) with siblings `record.rs`, `log.rs`, `anchor.rs`, `round.rs`, `drain.rs`, `serve.rs`; declare `mod review;` from `lib.rs`. <!-- `src/review/` with record/log/anchor/round/drain/serve; mod.rs is declarations + re-exports only. -->
- [x] 1.3 Confirm `gix::date` can format an RFC 3339 timestamp from `SystemTime`, so no date crate is needed; record the API used. <!-- `gix::date::Time::now_utc().format_or_unix(gix::date::time::format::ISO8601_STRICT)` — infallible, so no unwrap against the lint policy. -->

## 2. Record schema (`src/review/record.rs`)

- [x] 2.1 Define the record enum with a `t` discriminator: `Open`, `Serve`, `Round`, `Thread`, `Submit`, `Drained`, `Close`. Derive `Serialize`/`Deserialize`; keep every optional field `#[serde(default, skip_serializing_if)]` so lines stay narrow. <!-- Internally-tagged `#[serde(tag="t")]` enum; Thread/Submit are newtype variants wrapping structs. -->
- [x] 2.2 Define `Anchor { path, side, line, quote, before, after }` with `Side` as `new`/`old`, and cap `before`/`after` at a small fixed number of lines. <!-- `Anchor { path, side, line, quote, before, after }`, context capped at CONTEXT_LINES = 3. -->
- [x] 2.3 Define `Thread { id, anchor: Option<Anchor>, body, replace: Option<String>, resolved: bool, deleted: bool, at }` — an absent anchor is a review-level comment, per the design. <!-- Absent `anchor` is the review-level (verdict) case — no separate type. -->
- [x] 2.4 Define `Submit { round, preset: Option<String>, body, at }`. <!-- `Submit { round, preset, body, at }`; body delivered as written, preset for scripts. -->
- [x] 2.5 Round-trip tests: every variant serializes to one line and parses back equal; an unknown `t` is skipped without error; optional fields absent from older lines read as their defaults. <!-- 7 tests: every variant one-line round-trip, absent optionals default, unknown tag rejected so replay can skip it. -->

## 3. Log I/O + review lifecycle (`src/review/log.rs`)

- [x] 3.1 Resolve the log path from a repository: `<workdir>/rediff.jsonl` via gix's workdir. Test that a **linked worktree** resolves to its own root (`.git` is a file there — assert the path is inside the worktree, not the common dir). <!-- **Verified against a real linked worktree**: `.git` is a file there, and gix `workdir()` still resolves to the worktree's own root, not the common dir. Bare repo returns None. -->
- [x] 3.2 `append(record)`: open `O_APPEND`, take an advisory lock for the write, emit exactly one line. Never rewrite existing bytes. <!-- std `File::lock`/`unlock` (stable on 1.96) — no libc, no unsafe. Unlock is explicit so a failed write still releases. -->
- [x] 3.3 `replay()`: read the file once, yielding `(ordinal, Option<Record>)` where ordinal is the 1-based **raw** line number and a malformed line yields `None` while still consuming its ordinal. <!-- Raw line numbers; a malformed line yields None and still consumes its ordinal. -->
- [x] 3.4 `state()`: fold a replay into the review's current state — open review, rounds, threads folded by `id` (last wins, `deleted` retracts), submits, last drain ordinal, serve/close pairs. <!-- `fold()` split out as a pure function over records, so every folding rule is testable without a filesystem. -->
- [x] 3.5 `open_review(repo, target, label, keep)`: attach when the existing review is neither closed nor drained; otherwise truncate to a fresh log, or append when `keep`. <!-- **Spec tightened during implementation** — see note below. -->
- [x] 3.6 Emit the one-time "add `rediff.jsonl` to .gitignore" hint on file creation only; never touch git ignore configuration. <!-- `append` returns whether it created the file; the caller emits the hint. Nothing writes to git ignore configuration. -->
- [x] 3.7 Tests: attach / fresh / keep paths; fold-by-id (edit supersedes, delete retracts, resolved retained, two ids on one anchor both survive); malformed-line ordinal invariance. <!-- Covers attach/fresh/keep, supersede/retract/resolved, two ids on one anchor, first-appearance order, malformed-line ordinals. -->
- [x] 3.8 Concurrency test: two threads appending records simultaneously produce a log in which every line parses and no record is lost or split. <!-- 8 threads x 25 records x 4 KiB bodies: every line parses whole, no record lost. -->

## 4. Rounds + content hashing (`src/review/round.rs`)

- [x] 4.1 Wrap XXH3-64 (`xxhash_rust::xxh3::xxh3_64`) in one named function, and name the variant in a doc comment as part of the on-disk format. Explicitly **not** `DefaultHasher` or `rustc-hash` — both are documented as unstable across versions and this value is persisted. Test against a published XXH3-64 vector and that the same input hashes equal across calls. <!-- XXH3-64 wrapped in `content_hash`; asserted against the published empty-input vector 0x2d06800538d394c2 so a variant change fails loudly. -->
- [x] 4.2 `hash_changeset(&Changeset) -> BTreeMap<String, u64>` over each file's reviewed content; define the value used for a deleted side and for a binary file. <!-- Hashes the new side; `NO_CONTENT` sentinel distinguishes a deleted file from an emptied one. -->
- [x] 4.3 `open_round(state, changeset)`: next round number + the hash map, appended as a `Round` record. A frozen target (commit/range) opens exactly one round. <!-- `frozen` is an explicit parameter — a frozen target returns the existing round and writes nothing. -->
- [x] 4.4 `changed_since(round, &Changeset) -> Changed { modified, added, removed }` by comparing hash maps. <!-- `Changed { modified, added, removed }`. -->
- [x] 4.5 Tests: unchanged file absent from the result; modified file reported; a file that appeared and one that disappeared reported in the right bucket; frozen target opens one round only. <!-- Includes a test that no file content ever reaches the log. -->

## 5. Anchors + resolution (`src/review/anchor.rs`)

- [x] 5.1 `capture(file, side, line) -> Anchor` from a `DiffFile`, recording the line text plus the bounded context on the same side. <!-- Captures from the side's full text, clamped at both file edges. -->
- [x] 5.2 `resolve(&Anchor, &Changeset) -> Resolution` returning `Attached { line }`, `Shifted { from, to }`, or `Detached`, following the design's four-step rule: absent file → detached; exact hit at the recorded line → attached; else a bounded-window scan scored by matching `before`/`after` with ties broken by proximity; else detached. <!-- Four steps as specified; `Attached`/`Shifted{from,to}`/`Detached`. -->
- [x] 5.3 Define and document the acceptance threshold (how much context must match for a windowed candidate to be accepted) as a named constant, so tuning it later is one edit. <!-- `SEARCH_WINDOW = 200`, `MIN_CONTEXT_MATCH = 1`. The threshold applies **only when the quote is ambiguous** — a unique quote needs no context, so rewriting the surroundings does not detach it. -->
- [x] 5.4 Tests: unchanged → attached; lines inserted above → shifted with both numbers; line deleted → detached; file gone → detached; **identical lines in the window → context picks the right one, and equal scores pick the nearest**; a candidate below the threshold → detached rather than a wrong attach. <!-- 14 tests incl. identical-lines-picked-by-context, equal-scores-pick-nearest, below-threshold detaches, and a move beyond the window. -->

## 6. Delivery (`src/review/drain.rs`)

- [x] 6.1 `undelivered(state)`: threads and submits with ordinal greater than the last drain ordinal, with each thread's anchor resolved against a supplied changeset and flagged attached/shifted/detached. A detached thread is included, never dropped. <!-- Anchors resolved per thread; retracted threads excluded; detached delivered with quote + context intact. -->
- [x] 6.2 `drain(...)`: return `undelivered(...)` and append a `Drained { upto }` recording the ordinal delivered through. <!-- Marker records `last_ordinal`, so anything appended later is pending. An empty drain writes no marker. -->
- [x] 6.3 `all(state)`: the full folded state with per-thread delivery status; appends nothing. Both derive from the one replay in 3.4 — no second code path. <!-- `all()` and `undelivered()` share one `collect()` — one code path, as specified. -->
- [x] 6.4 Tests: drain twice with nothing between → second is empty; drain marker records the right ordinal; an edit after a drain is undelivered; `all()` appends nothing; a detached thread appears in the delivered set carrying its recorded quote and context. <!-- Includes an assertion that `all()` leaves the file byte-identical. -->

## 7. Serve lifecycle (`src/review/serve.rs`)

- [x] 7.1 `record_serve(pid, port, url)` / `record_close(pid, reason)` append the two lifecycle records. The store **does not** probe whether a pid is running — that belongs to the caller that owns server processes (`web-render`), where the port is in hand. <!-- **Scope cut** — the store records, it does not probe. See note below. -->
- [x] 7.2 `last_serve(state) -> Option<ServeState { pid, port, url, closed: Option<String> }>`: the most recent serve, with the reason from a following close for the same pid, or `None` for that field when unpaired. <!-- `last_serve` pairs a close to a serve by pid; an unpaired start makes no claim about the process. -->
- [x] 7.3 Tests: start records pid/port/url; an unpaired start reports `closed: None`; a stop for the same pid pairs and carries its reason; serve → close → serve reports the second start, unpaired. <!-- Covers unpaired, paired, mismatched pid, pidless close, and serve/close/serve. -->

## 8. Changeset self-exclusion (`src/git/`)

- [x] 8.1 Filter the worktree-root `rediff.jsonl` out of the enumeration for every source (working-tree, staged, commit, range), unconditionally and not configurably. <!-- Applied in `enumerate_repo`, the one funnel both `load` and `enumerate_in` pass through — so all five request kinds are covered by one filter. -->
- [x] 8.2 Tests: an untracked log is absent from a working-tree changeset while other untracked files remain; a commit that touched the log yields a changeset without it; a `rediff.jsonl` in a **subdirectory**, and a file whose name merely contains that string, both still appear. <!-- Unit test on the predicate plus real-repo tests for untracked and committed logs; `sub/rediff.jsonl`, `my-rediff.jsonl` and `rediff.jsonl.golden` all survive. -->

## 9. Gates (per CLAUDE.md, in order)

- [x] 9.1 `cargo clippy --workspace --all-targets` — zero warnings. Fix real correctness flags; `#[allow(...)]` only intentional pedantic hits, each with a justification comment. <!-- Zero warnings, confirmed by touching the new files to force re-analysis. -->
- [x] 9.2 `just crap-ci` — no baselined entry regressed and no function newly above 30. If a resolver or fold grows too branchy, split it into single-purpose helpers rather than baselining the debt. <!-- `CRAP gate PASSED: no regressions, no new over-threshold functions`. No baseline refresh needed. -->
- [x] 9.3 `just coverage` — every new function at or above the 90% per-function line-coverage floor. Expect the gaps to be the malformed-line arm in `replay`, the `Detached` arms of `resolve`, and the supersede/retract arms of the fold — cover them directly rather than lowering the bar. <!-- Two violations found and fixed by adding tests (`log_path` 0% -> the linked-worktree test; `Resolution::line` 83% -> the Attached arm). Final: 0 functions below 90%; review/* between 99.06 and 100. -->
- [x] 9.4 `cargo fmt --all` as the last step before staging; if it touches files this change did not edit, commit those separately. <!-- Touched only files in this change; no drift from elsewhere. -->

## Artifact updates made during implementation

Two things in the spec did not survive contact with the code. Both were changed
deliberately, before the code that depends on them:

- **7.1 — the liveness probe was cut, not ported.** The design named
  `libc::kill(pid, 0)`, but `Cargo.toml` sets `unsafe_code = "deny"` at the
  workspace level, so it cannot compile here at all. Rather than route around the
  policy (`rustix` was available and already in `Cargo.lock`), the store stopped
  claiming to answer the question: it records `serve`/`close` with pid, port, and
  URL, and reports whether a start is unpaired. Whether that process is *running*
  is now `web-render`'s call, where the port is in hand and a health check is one
  request away — and where pid reuse and `EPERM` can be handled honestly instead
  of silently returning "alive".

- **3.5 — "closed" was undefined, and dangerous as written.** The requirement said
  a fresh log starts when the previous review is "closed **or** fully drained", but
  nothing defined *closed* once `close` became the serve-stop record. Taken
  literally it also had a data-loss bug: a review closed with feedback nobody had
  polled would have been truncated. Delivery is now the sole criterion — an
  unfinished review is always attached to, never replaced — and two scenarios were
  added to pin the safety property and the vacuous-empty case.

## Deferred, with a reason

- **Line-level "since round N".** Rounds record per-file hashes only, so they
  answer *which files* moved and not *which lines*. Storing each round's patch
  would buy that; it is an additive optional field, not a format break.
- **`Delivered::already_delivered` is only meaningful in `all()`.** In
  `undelivered()` it is always false by construction. Kept on the one shared type
  rather than splitting the return shape in two.

## Post-implementation review pass

A `/code-review` over the change raised 15 findings. Seven were real correctness
bugs, all in code this change introduced:

- **`open_review` could delete a log holding unread feedback.** The attach test was
  `open.is_some() && !fully_drained()`, so a log whose `open` line was truncated (or
  written by a newer build) read as "no review" and was removed — with a human's
  comments in it, even when `keep` was requested. Replaced with
  `ReviewState::safe_to_replace()`: never truncate a log that has undelivered
  feedback **or** lines this build could not parse.
- **A retracted thread wedged the review permanently.** `pending_ordinals` counted
  `deleted` threads but delivery skipped them, so `drain` had nothing to deliver,
  never wrote a marker, and `fully_drained` stayed false forever. Writing a comment
  and then deleting it made the worktree's store unusable.
- **`replay` read without a lock.** `append` takes an exclusive lock precisely
  because a large record's write can be split; `replay` took none, so it could see a
  torn line, count its ordinal, and let the next drain mark the completed record
  delivered. Now takes a shared lock and decodes lossily, so a torn multi-byte
  character costs one line rather than the whole replay.
- **A pidless close un-paired a genuine one.** The fold kept only the newest `close`,
  so a TUI-only close after the server's own made a stopped server look unpaired.
  Now only a close naming the current serve overwrites.
- **A rename detached every anchor in the file.** `resolve` matched on `path` only;
  it now also matches `previous_path`, as `apply_path_filter` already did.
- **Submits carried no delivery flag**, so a consumer recovering via `all()` could
  not tell round 1's instruction from round 3's — and a submit is an *instruction*.
  Added `DeliveredSubmit`.
- **`append`'s `created` flag was a pre-open `stat`**, so two writers racing on a
  fresh log both claimed creation and the one-time hint would print twice. Derived
  from `create_new` instead.

Three test-quality findings were also fixed: `crate::testutil::run_git` and
`scratch_repo()` had been re-implemented rather than used, and a `temp_log()` helper
meant two different things in different modules (appending an `open` record in two,
not in the other two). Review fixtures now live in `testutil` as `review_log` /
`opened_review_log` / `diff_file` / `changeset`.

Two findings were **documented rather than fixed**, because fixing them means
changing the diff layer:

- `hash_changeset` and `resolve` require a **fully-diffed** changeset. An undiffed
  streaming stub carries no text, which would hash every file to `NO_CONTENT` and
  detach every anchor. Both now carry a `# Preconditions` section.
- A **binary file always hashes to `NO_CONTENT`**, so `changed_since` cannot report
  a changed binary. Fixing it needs the raw side bytes, which `src/git/diff.rs` does
  not surface. Documented on `hash_changeset`.

Two were not acted on: `open_round` appends unconditionally (that is the caller's
contract — a round is "a review pass began", and deduping it would prevent a
deliberate second look at unchanged code), and `collect` re-splits a file's text per
thread (real, but tens of threads over one file is microseconds; group by path if it
ever shows up).

## Second review pass — remaining findings fixed

The five findings left open after the first pass were all addressed rather than
documented:

- **The pager and external renderer bypassed the self-exclusion.** They
  post-process git's own output and never open a repository, so with the documented
  lazygit `externalDiffCommand: rediff external` setup an untracked `rediff.jsonl`
  rendered in the combined view — the exact failure the exclusion exists to prevent.
  Both now share an `is_review_log` check, and the `changeset-loading` delta was
  widened to require it.
- **Undiffed changesets are now impossible to misread.** `Changeset::fully_diffed()`
  was added; `open_round` rejects a half-loaded changeset with `InvalidInput`
  instead of fingerprinting every file as contentless, `changed_since` reports a
  still-loading file in no bucket rather than as removed, and anchor resolution
  gained a fourth outcome, `Resolution::Unresolved`. That last one matters most: a
  streaming load previously reported *every* thread as detached, telling a consumer
  all the commented code was gone.
- **Binary files are fingerprinted again.** `DiffFile` gained `content_digest`,
  taken from the raw new-side bytes at diff time, so a replaced image is reported as
  modified. Rounds prefer it and fall back to hashing text.
- **A no-op round no longer appends a record.** `open_round` returns the existing
  round when nothing moved, so a surface that re-enumerates on refresh cannot grow
  the log without bound. (The earlier "that is the caller's contract" position was
  wrong: the frozen path already had this shape, and the dedup is what makes it
  consistent.)
- **Delivery splits each file's text once**, via `resolve_in`/`side_lines` and a
  per-`(path, side)` cache in `collect`, instead of re-splitting the whole file for
  every anchor on the polled `all()` path.

Adding `content_digest` to `DiffFile` touched 21 existing struct literals across the
TUI, renderer, and their tests — mechanical, and the compiler found every one.
