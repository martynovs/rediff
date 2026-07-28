## Why

`add-review-store` landed the review log but nothing that can reach it. There is no way to open a
review, no way to see whether one is open, and — the point of the whole exercise — no way for an
agent to read back what a human concluded. The store is a library with no callers.

This change adds the **non-interactive** commands: enough for an agent to request a review and drain
its result, and for a human to see what state a worktree's review is in. It deliberately stops
short of any capture surface. A human still cannot *write* feedback after this change; that arrives
with `tui-review` and the web surface. What lands here is the half an agent talks to, which is also
the half that is fully testable from a shell.

## What Changes

- **`rediff request`** — open or attach to the worktree's review, open a round over the current
  changeset, and exit. Non-interactive: no TUI, no server. This is how an agent says "I have
  finished; please look." Prints the review id, the round number, and the log path.
- **Every target the viewer supports, including the combined one.** A review can cover the working
  tree, staged changes, a commit, a range — or, most importantly for the agent workflow,
  **everything since a base ref whether committed or not** (`--from <ref>`, the existing
  `WorkingTree { base }` request). An agent that made three commits and still has uncommitted work
  needs the human to see all of it as one changeset.
- **`rediff review-status`** — report the worktree's review state: whether one is open, its label and
  target, the current round, how many threads are pending delivery, and whether a server recorded
  itself as serving it. Human-readable by default, `--json` for tooling.
- **`rediff feedback [--all]`** — drain the review, resolving every anchor against the current
  changeset, and emit JSON. Default drains undelivered records once and marks them delivered;
  `--all` replays the full folded review, flags what was already delivered, and appends nothing.
- **A target encoding.** The log's `open` record stores its target as a string; that string must
  round-trip back to a `LoadRequest` so `feedback` can rebuild the changeset an anchor resolves
  against. A canonical, parseable form (`worktree`, `worktree:<base>`, `worktree-tracked`,
  `worktree-tracked:<base>`, `staged`, `show:<rev>`, `range:<a>..<b>`, `review:<base>..<target>`) is defined and tested both ways.
- **A request that names a different target is refused only while feedback is pending.** Attaching
  silently to a review of a different target would report success for something the caller did not ask
  for; refusing *unconditionally* would make every target change a one-way door, since the store is
  otherwise happy to start a fresh review.
- **The ignore hint** is emitted when a *new* review is started, naming `rediff.jsonl` and suggesting
  `.gitignore`. Not "once per worktree": the store gives no such signal, and a fresh review legitimately
  recreates the file. Nothing writes to any git ignore configuration.
- **Not in this change:** `--web` and everything behind it (that is `web-render`), and any means for
  a human to record a comment (that is `tui-review` and the web surface). `rediff request` accepts
  no `--web` flag rather than accepting one that does nothing.
- **No path filters on `request`.** The other subcommands take pathspecs, but a review is already
  scoped to what changed, and an agent wants the human to see everything it touched. Accepting them
  would also mean recording them — `git::load` does not filter, so `feedback` would otherwise rebuild
  a different file set than the round was hashed over.

## Capabilities

### New Capabilities

- `review-cli`: the non-interactive commands over the review store — requesting a review, reporting
  its state, and draining its feedback — together with the canonical target encoding that lets a
  drained anchor be resolved against the changeset it was written against.

### Modified Capabilities

None. The store's requirements are unchanged; this change only calls it. `rediff review` and
`rediff diff` keep their current meaning and gain no flags here.

## Impact

- **`src/cli.rs`**: three new `Command` variants. The existing `review`, `diff`, `show`, `pager`,
  and `external` subcommands are untouched — note `rediff review` already exists and means "open the
  TUI on a range with viewed-tracking", which is why the new verb is `request` rather than a second
  meaning for `review`, and why the status command is `review-status` rather than a near-homograph
  `reviews`.
- **`src/main.rs`**: two dispatch points, not one. `review-status` and `feedback` need a repository
  but no target, so they run *before* `Cli::resolve`; `request` needs a resolved `LoadRequest`, so it
  runs after. Neither belongs in `run_filter_command`, whose contract is "never opens a repo".
- **`src/cli.rs` refactor, required first**: `Cli::resolve` is at cyclomatic 26 against a CRAP
  threshold of 30. Adding a `Request` arm in its current shape would breach the gate, so each
  subcommand's resolution is extracted into its own function and `resolve` becomes a dispatcher.
- **New module** for the command bodies and the target encoding, so `main.rs` stays a dispatcher and
  the logic is unit-testable (`mod.rs`/`lib.rs` stay import-only per the module convention).
- **`src/review/`**: no requirement changes. `open_review`, `open_round`, `drain`, `all`,
  `last_serve`, `safe_to_replace`, and `Log` are consumed as they are. Note `open_round`'s `frozen`
  argument is always passed `false` — see the design.
- **Testing**: `tests/cli_io.rs` already exercises subcommands end-to-end through the built binary;
  the new commands extend it. `feedback`'s JSON is the agent's contract, so its shape is asserted
  rather than smoke-tested.
- **No new dependencies.** `serde_json` and everything else needed is already present.
- **Gates**: clippy pedantic clean, the CRAP gate with the ≥90% per-function coverage floor, then
  `cargo fmt --all`.
