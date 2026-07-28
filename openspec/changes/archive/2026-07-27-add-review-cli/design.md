## Context

`add-review-store` shipped `src/review/`: an append-only `rediff.jsonl` per worktree, with
`open_review`, `open_round`, `drain`, `all`, and `last_serve` as its surface. Nothing calls it.

The CLI it must attach to already has opinions. `src/cli.rs` parses five subcommands into a
`Resolved { repo_dir, req: LoadRequest, filters, mode, theme, review }`, and `main.rs` either
launches the TUI (stdout is a terminal) or dumps unified text (it isn't). Two facts constrain this
change:

- **`rediff review` is taken.** It means "open the TUI on a range with viewed-tracking". A second
  meaning would be a breaking change to a documented command.
- **`src/review/` does not depend on `src/git/`.** It knows `model::Changeset` and nothing about
  `LoadRequest`. That separation is worth keeping: the store records a review, it does not know what
  a git target is.

## Goals / Non-Goals

**Goals:**
- An agent can request a review, then drain it, without a terminal.
- A review can cover the working tree, earlier commits, or **both at once** — the case that matches
  how an agent actually works (some changes committed, some not).
- A drained anchor is resolved against the changeset it was written against, not against whatever
  happens to be in the working tree at drain time — which means the target must round-trip.
- A human can find out what state a worktree's review is in.
- Every command is exercisable from a shell, so the store gets end-to-end coverage it could not have
  had as a library.

**Non-Goals:**
- `--web` and the server behind it (`web-render`).
- Any way for a human to *write* feedback (`tui-review`, and the web surface). After this change the
  log can be opened and drained but only written to by hand — deliberately, so the agent-facing
  contract settles before a UI is built against it.
- Persisting viewed-tracking to the store. It is an obvious follow-on, but it belongs with the TUI
  change that owns that state.
- Verdict presets in `config.toml`. They are read by whatever offers a submit action; nothing here
  submits.

## Decisions

### `rediff request`, not a second meaning for `rediff review`
The agent-facing verb is `rediff request` — "request a human review". It opens or attaches to the
worktree's review, opens a round over the current changeset, prints the review id, round, and log
path, and exits.
- *Rejected — overloading `rediff review`:* it already launches the TUI on a range. Making it
  sometimes non-interactive, depending on a flag, is the kind of overload that makes a CLI hard to
  remember and would break existing invocations.
- *Rejected — a `--record` flag on `rediff diff`/`rediff review`:* a reasonable future addition for
  the *human* path, and the right home for it is `tui-review`, where capture lives. An agent needs a
  command that does not open a TUI at all.
- *Rejected — implicit opening by whatever surface looks first:* an agent must be able to initiate,
  which is the workflow this whole stack exists for.

### `Cli::resolve` must be split before a sixth arm is added
`Cli::resolve` sits at cyclomatic **26** against a CRAP threshold of **30**, with an empty baseline —
so the gate's predicate is "new and crap > 30". A `Request` arm shaped like the `Diff` arm (a
positional loop with `contains("..")` and `is_repo_root`, then a three-arm match) is worth roughly
seven on its own, and two further variants add two more. That breaches the gate before a single
command works, and the usual remedy does not apply: the bloat lands in `cli.rs`, not in a new command
body, and `resolve` already carries an `#[expect(clippy::too_many_lines)]` arguing against splitting
it *further* in place.

So: extract each subcommand's resolution into its own function and leave `Cli::resolve` a dispatcher.
This is a prerequisite task, not cleanup afterwards.

### Two dispatch points, because two of the commands have no target
`review-status` and `feedback` take no target — `feedback` derives its `LoadRequest` from the log.
There is nothing for `Cli::resolve` to return for them, and the `unreachable!()` trick the `pager` and
`external` arms use (`src/cli.rs:152`, `:157`) works only because `main` dispatches those *before*
calling `resolve` at all.

Therefore: `review-status` and `feedback` dispatch **before** `resolve`, needing only `-C`; `request`
dispatches **after** it, needing the resolved `LoadRequest`. Neither group belongs in
`run_filter_command`, whose contract is "never opens a repo" — all three do.
- *Rejected — one dispatch point after `resolve`:* would force `resolve` to fabricate a `Resolved`
  for two commands that never read it, or to `unreachable!()` on a path that is actually reached,
  which panics at runtime.

### The target is a canonical string, parsed in the CLI layer
`Record::Open.target` stays a `String`. This change defines a grammar for it and parses it back into
a `LoadRequest`:

```
worktree                    include_untracked = true,  base = None
worktree:<base>             include_untracked = true,  base = Some
worktree-tracked            include_untracked = false, base = None
worktree-tracked:<base>     include_untracked = false, base = Some
staged
show:<rev>
range:<old>..<new>
review:<base>..<target>
```

The kind is the text before the **first** `:`; everything after it is the revspec, verbatim. Splitting
at the first colon is what makes this safe — a revspec may itself contain colons (`HEAD:path`,
`:/message`, `:0:file` are all legal), so any encoding that split on the *last* colon, or on `=`/`;`,
would corrupt them.

The separator that genuinely is ambiguous is `..`, in the two-field forms. `range` and `review` split
at the **first** `..`, matching `split_range` in `src/cli.rs`. That leaves one unrepresentable case:
a `ReviewRange` whose `base` itself contains `..` (`rediff review --from 'a..b' feature`) would encode
to `review:a..b..feature` and parse back wrong. Rather than corrupt it silently, `encode` returns an
error for a base or target containing `..`. No command in this change can produce that shape —
`request` never constructs a `ReviewRange` — so it is a guard for a future caller, not a live path.

- *Rejected — a structured target in the record:* it would put a `git::LoadRequest` inside a
  `review::Record`, inverting the dependency and teaching the store what git is.
- *Rejected — reusing `Changeset.source`:* that field is a **display label** (`"working tree"`,
  `"worktree vs main"`, `"review a..b"`) — it contains spaces and, critically, drops
  `include_untracked`, so it cannot round-trip. Encoding and labelling are different jobs and
  conflating them would silently lose a flag.
- The round-trip is the tested property: every `LoadRequest` variant encodes and parses back equal.

### `request --from <ref>` means `WorkingTree { base }`, not `ReviewRange`
`--from` is overloaded in the existing CLI: on `diff` it means `WorkingTree { base }`
(`src/cli.rs:36`), on `review` it means `ReviewRange { base }` (`:55`). For `request` it means the
former — the combined base-through-worktree diff. `request` therefore never constructs a
`ReviewRange`; the codec still round-trips one because the log may record a review some future
surface opened.

### The combined target is the important one, not an edge case
`WorkingTree { base: Some(ref) }` — surfaced today as `rediff diff --from <ref>` — is the net diff
from a base ref through to the working tree, spanning committed *and* uncommitted work. That is the
default shape of agent output: three commits and a dirty tree. The encoding therefore treats
`worktree:<base>` as a first-class form rather than a modifier, and it is the target most worth
covering in tests.

### No path filters on `request`
The other subcommands take pathspecs; this one does not.
- A changeset is already scoped to what changed, and an agent wants the human to see everything it
  touched — a narrower review is a way to miss something.
- *And it could not be made correct cheaply:* `git::load` does not apply filters (`main.rs` applies
  them as a separate step afterwards), and the `open` record has no field for them. A filtered
  `request` would hash a round over eight files and a later `feedback` would rebuild all forty,
  reporting every unfiltered file as added and resolving anchors against files that were never under
  review. Supporting them properly means persisting them, which is a change to the store's record —
  worth doing if a real need appears, not to satisfy symmetry.

### A different target is refused only while feedback is pending
`open_review` attaches whenever the log is not safe to replace, and it never compares targets. So a
`request` for `show HEAD~3` against an open working-tree review would attach and report a round
number — success, for something the caller did not ask for.

But refusing *unconditionally* is worse than the bug it fixes: after a normal round trip (request,
comment, drain) the log is fully delivered and the store would gladly start a fresh review, yet a
blanket refusal would reject every subsequent `--from main`. With no `--force` and no documented
escape, changing target would mean deleting the log by hand. Every target change would be a one-way
door.

So the comparison is gated on `ReviewState::safe_to_replace()`: refuse only when attaching would
mis-report *undelivered* feedback; otherwise let the store start fresh. That is the same condition
the store already uses to decide attach-versus-replace.

### `frozen` is dropped: no target is reliably immutable
An earlier draft passed `frozen = true` for `show`/`range`/`review`, so those would keep a single
round. That is wrong. `ReviewRange { base: "main", target: "HEAD" }` moves every time the agent
commits; so does `Range { old: "main", new: "HEAD" }`, and `Show { rev: "HEAD" }` under an amend.
Freezing them makes `open_round` return the first round forever (`round.rs`), so the agent's core
loop — request, get comments, commit fixes, request again — would report "round 1" indefinitely while
the recorded hashes went stale. That contradicts this change's own "a later request opens the next
round" requirement.

Immutability is a property of a *resolved object id*, not of a request kind, and `changed_since`
already suppresses a round when nothing moved — which gives a genuinely frozen target exactly the
behaviour `frozen` was for. So `open_round` is always called with `frozen = false`.

**Do not reach for `Resolved.review` here.** It is a different predicate — true for working-tree,
staged, range, and `review`; false only for `show` — and in a change called `review-cli` the name is
a trap. It is not `!frozen`, and `frozen` no longer exists.

### A review is *continued* while its target is unchanged
`open_review` is called only when there is genuinely a new review to open — no
review at all, or one over a different target that is safe to replace. A request
naming the target already open attaches to it directly.

This was found by a failing test, and the reason matters: `open_review` replaces a
fully-delivered log. Since a normal round trip (request, comment, drain) leaves the
review fully delivered, calling it unconditionally would truncate the log and
restart the round counter at 1 on the very next request — exactly when the loop is
still going. Rounds are the iteration counter *within* a review, so the store's
"finished reviews may be replaced" rule must not be applied to a loop that has not
finished.

### `feedback` rebuilds the changeset from the recorded target
Resolving an anchor needs a changeset. `feedback` reads the target from the `open` record, parses it,
and calls `git::load` — the synchronous full load, which returns a fully-diffed changeset. That
matters: `open_round` rejects a half-loaded changeset and `resolve` reports one as `Unresolved`, so
the streaming loader would be the wrong tool here.
- Note this resolves anchors against the target **as it is now**, which for a working-tree review is
  the live tree. That is intended — the point of re-anchoring is to tell the agent where its comments
  landed *after* its own edits.
- **When the target no longer resolves, feedback is still delivered.** A review over
  `review:main..feature` whose `feature` branch is gone would otherwise deadlock: `feedback` errors,
  `--all` errors too (it needs a changeset), and `request` for anything else is refused — leaving the
  human's comments unreachable short of deleting the log. Instead the changeset falls back to empty,
  every anchor resolves `Detached`, and a warning goes to stderr. The anchors carry their own quote
  and context, so a detached item is still meaningful; a deadlock is not.

### JSON is `feedback`'s contract; `review-status` is for a human
`feedback` emits JSON unconditionally: its consumer is a program. `review-status` prints a
human-readable summary and takes `--json` for tooling. Asymmetric on purpose — each command's default serves its
actual reader.

Each delivered thread carries its anchor, its resolution (`attached` / `shifted` / `detached` /
`unresolved`, with line numbers), the body, any suggested replacement, and the delivery flag. A
detached thread carries the quote and context it was recorded against, since that is the only
evidence left of what it referred to.

### Non-interactive dispatch, but these commands open a repository
`main.rs` dispatches `pager`/`external` before touching a repo. The new commands are also
non-interactive and must bypass the TUI/text-dump branch, but unlike the filters they do need a
repository. So they get their own dispatch step after repo resolution and before the terminal check,
rather than being folded into `run_filter_command` — whose contract is precisely "never opens a
repo".

### Command bodies live in `src/reviewcli/`, split render from I/O
`src/reviewcli/` holds the three command bodies and the target codec, matching the capability name.
`mod.rs` and `lib.rs` stay declarations and re-exports only, per the module convention.

Each command is split into a **pure function producing the output** and a thin shell that writes it.
That is not only tidiness: the 90% per-function coverage floor is unreachable for a body that formats
and prints in one step, and the alternative — driving every case through a spawned binary — is slow
and gives worse failure messages. The pure half is unit-tested exhaustively; the shell gets one
end-to-end test each.

### Small things, written down because they land in the log
- **Review id:** derived from the process id and the current timestamp, rendered as a short hex
  string. It only needs to be distinguishable within one worktree's history, so this is sufficient —
  not because randomness is unavailable (`RandomState` is OS-seeded), but because it needs none.
- **Label:** absent unless `--label` is given. There is no default; a guessed one (a branch name,
  "agent") would be noise in the common case where one review is open.
- **`keep`:** `open_review` is always called with `keep = false`. A new review replaces a delivered
  one; retaining history is a flag nothing in this change exposes.
- **Target comparison is string equality on encodings.** `show:HEAD` and `show:abc1234` may name the
  same commit yet compare unequal, and `worktree` differs from `worktree-tracked`. That coarseness is
  acceptable *only* because the refusal is now gated on pending feedback — a spurious mismatch on a
  delivered review just starts a fresh one.
- **JSON shape:** `Thread` serializes with `skip_serializing_if`, so `anchor`, `replace`, `resolved`
  and `deleted` are **absent keys**, not nulls. That is the agent-facing contract and is asserted.

### The ignore hint fires on a new review, not "once per worktree"
"Once per worktree" is not derivable from the store's API. `Log::append` reports whether it created
the file, but `open_review` discards that, and `Opened::Fresh` deletes and recreates the log — so a
second review would re-fire the hint anyway. Pre-checking `exists()` is explicitly documented in the
store as the wrong approach (two writers would both claim creation).

`open_review` does return `Opened`, so the implementable and honest rule is: hint on
`Opened::Fresh`. It goes to **stderr**, so `feedback`'s stdout stays parseable — though note
`feedback` can never actually start a review, so in practice only `request` emits it.

## Risks / Trade-offs

- **A review can be requested but not answered.** After this change nothing writes threads, so
  `feedback` will return empty for any real use. → Accepted and explicit: the value here is the
  settled agent contract plus end-to-end coverage. Tests write threads to the log directly, which is
  legitimate precisely because the store's format is the contract.
- **`feedback` does a synchronous full load**, which on a very large changeset is slower than the
  TUI's streaming path. → Correct rather than fast: a partial changeset would make every anchor
  `Unresolved`. If it becomes a problem, load only the files that threads actually anchor to.
- **The target grammar is a new parser**, and parsers rot. → Bounded to five variants with a
  round-trip test over each, and the ambiguity analysis above (`:` cannot occur in a ref) is what
  keeps it decidable.
- **A stale target.** A review opened over `review:main..feature` still parses months later even if
  `feature` is gone; `git::load` then fails. → Report it as a clear error naming the target rather
  than a bare git error, so the human knows the review outlived its refs.

## Open Questions

- Resolved during review: `request` refuses an empty changeset, takes no path filters, and refuses a
  target that differs from the open review's.
- Does `review-status` report only the current worktree, or discover sibling worktrees' logs? Current
  worktree only — cross-worktree discovery has no caller until something aggregates.
