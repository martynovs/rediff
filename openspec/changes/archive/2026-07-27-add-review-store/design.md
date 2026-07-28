## Context

rediff normalizes every source (working tree, staged, commit, range) into one `Changeset` of
`DiffFile → Hunk → Line`, where a `Line` carries `old_lineno`/`new_lineno` and its text, and a
`DiffFile` carries full `old_text`/`new_text`. Views are a stack of `ViewEntry`; `ViewState.viewed`
already models per-file reviewed state but only in memory, for the life of the process. Preferences
persist to `~/.config/rediff/config.toml` (`src/config.rs`), establishing XDG as the config
convention — but review feedback is *per-worktree working state*, not a preference, so it does not
belong there.

The consumer on the other end of this store is an agent: it edits the working tree, asks a human to
look, and needs the result back as structured data it can act on without re-reading the diff. The
human may look through the TUI or through a local web page; the store is what makes those
interchangeable.

## Goals / Non-Goals

**Goals:**
- A single, deletable, human-readable file per worktree that fully represents a review.
- Anchors that survive the agent editing the file underneath them, with an honest three-state
  outcome and no silent loss.
- "What changed since my last pass" without storing file content.
- Delivery semantics that are safe to re-run and require the caller to remember nothing.
- Enough recorded serve lifecycle that a caller which owns server processes can decide, on its own
  terms, whether one is already up.

**Non-Goals:**
- CLI subcommands, an HTTP server, TUI capture UI, HTML rendering — all separate changes.
- Applying a suggestion to the working tree. rediff stays read-only against the user's files; the
  agent applies.
- Reachability beyond `127.0.0.1`. Tunnelling (`ssh -L`, `tailscale serve`) is the user's concern
  and works against a loopback listener; there is consequently no token, no auth, and no secret to
  store.
- Multi-review concurrency as a design centre. The workflow is one agent per worktree, one review at
  a time; a second concurrent review is out of scope for this change.
- Line-level "since round N". Rounds answer *which files* moved, not *which lines* (see below).

## Decisions

### One file at the worktree root, not a directory and not `.git`
The log is `<worktree>/rediff.jsonl` — the workdir root reported by gix.
- *Alternative — `.git/rediff/`:* rejected, and it is not merely impolite: in a **linked worktree
  `.git` is a file**, not a directory (`gitdir: …/.git/worktrees/<name>`), so there is nothing to
  create a directory under. Verified: `git rev-parse --git-dir` there resolves outside the worktree.
- *Alternative — a global XDG state dir keyed by a path hash:* rejected; it puts a worktree's working
  state somewhere the user cannot see or delete alongside the thing it describes, and it needs
  keying and pruning logic that a plain file in the worktree does not.
- *Alternative — a `.rediff/` directory:* rejected once the token disappeared. With no secret and no
  snapshots there is exactly one file, and a directory for one file is a directory too many.

Consequence: the file is visible to git. rediff prints a one-time hint to add it to `.gitignore` when
it creates the file, and never writes to the user's ignore configuration itself. Writing
`info/exclude` was considered and rejected: git resolves `info/exclude` to the **common** dir, so a
write from one worktree silently changes ignore behaviour in every worktree of the repository.

### The line number is the sequence
Delivery needs a monotonic ordinal. The file provides one: the 1-based line index, which append-only
writing makes monotonic for free. `{"t":"drained","upto":12}` means every record on lines 1–12 has
been delivered. Counting is over **raw lines**, including any that fail to parse, so a malformed line
can never shift the sequence.
- *Alternative — an explicit `seq` field:* rejected as a second source of truth that can disagree
  with the file.

### An unanchored thread *is* the verdict
There is one feedback record. `anchor` present → a line comment; `anchor` absent → a review-level
comment. What the agent should do next is prose in a `submit` record's `body`, pre-filled from a
named config preset and editable by the human before sending, so the instruction can differ per
round. The preset's `name` rides along as an optional field purely so a script can branch without
parsing prose; the `body` is what the agent reads.
- *Alternative — a closed `verdict` enum:* rejected. A fixed `{approve, rework, revert}` cannot say
  "fix the second one, leave the rest, and don't start anything new," which is the common case.
- *Rationale for keeping `submit` distinct from `thread`:* sending the verdict is what closes a
  round. It is a lifecycle event, not another comment, and rounds key off it.

### Anchors carry their own context; re-anchoring is a windowed match
A thread records `{path, side, line, quote, before[], after[]}` — a few hundred bytes, entirely
self-describing. Resolution against a later changeset:

```
  1. file absent from the changeset                    → detached
  2. text at (side, line) == quote                     → attached
  3. else scan a bounded window around line for quote,
     score candidates by matching before/after lines,
     break ties by distance from the original line     → shifted (report both line numbers)
  4. no candidate above threshold                      → detached
```

- *Alternative — a content-addressed blob store and mapping the line through
  `imara_diff(old_blob, new_text)`:* genuinely more precise, and rejected anyway. It requires a blob
  directory to write, track, and garbage-collect, which is exactly the state this design is trying
  not to own. The windowed match costs a scan of one file and needs no storage at all.
- A detached thread is **still delivered**, carrying its `quote` and context, so the agent sees the
  code the comment was written against even when that code is gone.

### Rounds are per-file content hashes, not snapshots
Opening a round appends `{"t":"round","n":N,"files":{path: hash}}`. Comparing two rounds — or a round
against the current changeset — yields the set of paths whose hash differs, plus paths added or
removed. That is "3 of 12 files changed since your last pass": enough to direct attention, at roughly
50 bytes per file per round.
- *Alternative — storing each round's full unified patch inline:* would give true line-level "since
  round N", at patch-size × rounds in a file meant to stay readable and deletable. Deferred, not
  foreclosed — it is an added optional field, not a format break.
- The hash is **XXH3-64** (`xxhash-rust`, `xxh3` feature), and the format names the variant.
  64 bits is ample: this detects change, and a collision costs a missed highlight, not correctness —
  at ~100 files the collision probability is around 3×10⁻¹⁶.
  - *Rejected — `DefaultHasher` (as `commit_color_key` uses):* its output is explicitly unspecified
    across Rust releases. Fine for an ephemeral palette index, disqualifying for a persisted value,
    which would silently invalidate every recorded round on a toolchain bump. `rustc-hash`/FxHash is
    the same trap wearing a faster costume — also documented as unstable across versions.
  - *Rejected — FNV-1a 64 defined in-tree:* it satisfies the stability requirement and needs no
    dependency, and was the first choice for that reason. But the minimalism argument is weak next to
    a tree that already carries gix, syntect, two-face, ratatui and four tree-sitter grammars, and
    `xxhash-rust` is `no_std` with zero transitive dependencies. Speed is *not* the deciding factor —
    hashing happens once per human-paced round, where a 5 MB changeset costs about 5 ms under FNV and
    is invisible either way.
  - *Rejected — blake3:* cryptographic strength buys nothing here. Nothing in the threat model
    involves an adversary crafting a colliding edit, and a much larger dependency is the price.
- A frozen target (a commit or a range) cannot change, so it has exactly one round.

### Threads are identified, superseded, and never rewritten
Each thread carries an `id`. A later `thread` record with the same `id` supersedes it (an edit); one
with `"deleted": true` retracts it. Replay folds by `id`, last write wins. A `resolved` flag lets the
human mark their own thread done without deleting it — which is what makes a second round readable,
since round 1's threads otherwise persist forever.
- *Alternative — identity by anchor, as `agent-stage` does:* rejected. Two threads on one line is
  ordinary (one per round, or two separate thoughts), and anchor-identity cannot express it.
- An edit made after a drain is itself undelivered, so the agent receives the update. That is the
  correct behaviour and falls out of the line-number sequence.

### Drain-once by default; full replay is the same fold
`undelivered()` returns records after the last `drained.upto` and appends a new marker.
`all()` returns the full folded state with delivery flags and appends nothing. Both are filters over
one replay, so there is one code path and one set of semantics.
- *Alternative — a caller-held cursor:* rejected. It pushes state onto an agent that has nowhere
  durable to keep it between turns, in exchange for a guarantee `--all` already provides.

### The store records the serve lifecycle; it does not interpret it
`serve` records `{pid, port, url}`; `close` records `{pid, reason}`. The fold reports the most recent
serve and whether a close for that pid followed it. It stops there — the store never asks whether the
recorded process is running.
- *Forcing function:* the workspace denies `unsafe_code` (`Cargo.toml`), so `libc::kill(pid, 0)` —
  named in the first draft of this design — cannot compile here at all.
- *Rejected — a safe pid probe via `rustix` (already in `Cargo.lock`, so free to promote):* it would
  have satisfied the original requirement, but a pid is a poor liveness signal for this purpose. Pids
  are recycled, so a stale record can match an unrelated live process; and `kill(pid, 0)` on another
  user's process returns `EPERM`, which a caller must read as "alive". Both failure modes hand out a
  URL pointing at nothing.
- *Rejected — a lock the server holds for its lifetime (std `File::lock`, released by the OS on any
  death including `SIGKILL`):* the most correct answer, and it needs a second file in the worktree,
  which cuts against the one-file decision above.
- *Rationale for deferring rather than choosing:* the question "is a server already up, and should I
  rebind?" belongs next to the code that owns server processes, where the port is in hand and a
  health check is one request away. That code lands in `web-render`. Recording the facts is this
  change's job; acting on them is not.

Note that std's `File::lock`/`try_lock`/`unlock` are stable on the toolchain in use, so the
append lock below needs no dependency either — `libc` drops out of this change entirely.

### Module placement honours the import-only convention
`src/review/mod.rs` is declarations and re-exports only. Logic and tests live in named siblings:
`record.rs` (schema + serde), `log.rs` (path, append, replay, review lifecycle), `anchor.rs`
(resolution), `round.rs` (hashing + comparison), `drain.rs` (delivery selection), `serve.rs` (the
lifecycle records). Splitting this way also keeps per-function cyclomatic complexity low enough that
the CRAP gate does not need an orchestrator carve-out.

### Timestamps reuse `gix::date`
gix is already a dependency and formats commit times; reusing it avoids adding `chrono`/`jiff`/`time`
for a handful of RFC 3339 strings.

## Record schema

```jsonc
{"t":"open",   "review":"r7k2","target":"worktree","label":"agent · src/tui","at":"…"}
{"t":"serve",  "pid":4411,"port":53411,"url":"http://127.0.0.1:53411/"}
{"t":"round",  "n":1,"files":{"src/tui/rows.rs":"3f9a…","src/git/diff.rs":"81cc…"}}
{"t":"thread", "id":"t1","anchor":{"path":"src/git/diff.rs","side":"new","line":47,
                 "quote":"    let mut sink = Sink::new();",
                 "before":["fn diff(…) {"],"after":["    imara_diff::diff(…);"]},
               "body":"reuse the pooled sink here","at":"…"}
{"t":"thread", "id":"t2","body":"overall this is close","at":"…"}          // no anchor = review-level
{"t":"thread", "id":"t1","resolved":true,"body":"…","at":"…"}              // supersedes t1
{"t":"submit", "round":1,"preset":"rework","body":"fix t1, leave the rest, then show me again"}
{"t":"drained","upto":7}
{"t":"round",  "n":2,"files":{"src/tui/rows.rs":"3f9a…","src/git/diff.rs":"b204…"}}
{"t":"close",  "pid":4411,"reason":"drained"}
```

`replace` (an optional string on `thread`) carries suggestion text for the anchored line range. It is
in the schema from the start because adding it later is a migration; no surface is required to offer
it in its first version.

## Risks / Trade-offs

- **The log is visible in the worktree.** Mitigated by unconditional self-exclusion from rediff's own
  changesets plus a one-time hint. The alternative — hiding it in `.git` or a global cache — was
  rejected above for worse reasons.
- **A windowed match can re-anchor to the wrong line** in a file with many identical lines (`}`,
  blank lines). Mitigated by scoring on surrounding context and preferring the nearest candidate, and
  bounded by the fact that a wrong anchor is visible to the human on the next pass. Requiring a
  minimum context match rather than accepting a bare `quote` hit is the lever if this proves noisy.
- **Rounds cannot say which lines moved**, only which files. Accepted for v1; the upgrade path
  (storing the round's patch) is additive.
- **Two writers, one file.** The TUI and a server can both append. Appends are single-line and opened
  `O_APPEND`, which is atomic for writes under `PIPE_BUF` on the platforms rediff targets; a record
  larger than that (a long suggestion) needs the whole-line write to stay atomic, so the writer holds
  an advisory lock for the append rather than relying on size.
- **Adding `serde_json`** widens the dependency set of a crate that has kept it deliberately narrow.
  It is the format's cost of being human-readable and machine-consumable; TOML is a poor fit for an
  append-only log and a bespoke encoder is worse than the dependency.
