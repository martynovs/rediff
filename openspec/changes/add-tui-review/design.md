## Context

`add-review-store` and `add-review-cli` shipped the log and the agent's side of it. Nothing writes
feedback. The TUI is where a human already reads diffs, so it is where they should be able to
respond.

The relevant existing machinery:

- **The cursor has landed.** `ViewState.cursor_row` names a row, and `rows::cursor_key(plan, row)`
  returns `(file, Side, line)` — which is already `review::capture`'s argument list, already carries
  the new-side-else-old rule, and is already tested against every row shape. The anchor work this
  change was scoped to do is done.
- **The cursor marker occupies `area.x`**, the one column `draw_stack`/`draw_split` left blank. The
  commented-line gutter this change wants has to share it or pay another column.
- **Overlays are a stack**, not a single slot: `Mode { base, overlays: Vec<Overlay> }` with
  push/pop, and `CommitMessage` is routinely pushed over `Palette`. `Palette` does text entry, of
  the simplest kind — push-char/pop-char on one line, no cursor.
- **Review sessions exist.** `Session::is_review()` is the predicate — true for working-tree,
  staged, *and* `rediff review <rev>` (a commit view) — `R` promotes a browse view, and
  `viewed-tracking` already scopes itself to it.
- **`handle_global_key` is still at CRAP 30**, the threshold exactly — re-measured after the cursor
  landed (cyclomatic 30 at 100 % coverage). Anything added to it breaches the gate. `handle_stream_key`
  is at 21 and `handle_peek_key` at 19, so both have room. Note also `ui::frame::paint` dispatches overlays as an `if`-chain, not an exhaustive match:
  a new overlay omitted there compiles clean and silently draws nothing.
- **The keymap consistency test guards the opposite direction** from what one would hope: it asserts
  documented keys appear in the binding tables, not that new bindings get documented. It will not
  catch an undocumented key.

## Goals / Non-Goals

**Goals:**
- A human can leave anchored and review-level comments without leaving the TUI.
- An agent picks them up with `rediff feedback`, whether or not it ever ran `rediff request`.
- Browsing a diff writes nothing.
- Reviewed state survives a restart, which is the obvious win from having a log at all.

**Non-Goals:**
- The web surface (`web-render`, `web-app`).
- Any server, port, or URL.
- Reading the agent's *replies* in the TUI. The loop is human → agent here; showing the agent's
  round-closing instructions back to the human is a later refinement.

## Decisions

### The human initiates; the review is opened lazily
`rediff request` is an agent asking to be reviewed. This is a human reviewing whether or not anyone
asked, so the TUI must not require an open review — it opens or attaches on the **first comment**.
- *Rejected — open on entering a review session:* every `rediff diff` would create a log, so merely
  looking at your own changes would leave a file behind and print an ignore hint.
- *Rejected — require `rediff request` first:* it inverts who is in charge. The human should not have
  to ask an agent's permission to write down what they think.
- Consequence: the first comment is the point where the target is fixed, the ignore hint fires, and
  a `Round` is opened over the current changeset.

### A view that disagrees with the open review cannot take comments
The same hazard `request` has: `open_review` never compares targets, so commenting from a
working-tree view into a review of `show:HEAD~3` would anchor against a diff the review is not
about. The TUI refuses and names both, and — as in `request` — only while the open review holds
undelivered feedback, so a delivered review is simply replaced.

### Comments are confined to review sessions — the predicate, not a list of view kinds
`Session::is_review()` already draws this line for viewed-tracking, and `R` promotes a browse view.
Reuse the predicate itself: `rediff review <rev>` resolves to `Show { rev }` and is a `ViewKind::
Commit` view that **is** a review session, so any rule phrased as "working-tree, staged, or range"
would break the flagship review command.

### Commenting waits for the load, and for a whole view
`open_round` rejects a changeset that is not fully diffed, and `capture` returns `None` for a file
with no text — both of which are the normal state while the TUI streams. So commenting reports
"still loading" rather than surfacing an `InvalidInput`. Likewise a *filtered* view
(`rediff diff src/`) must not open a review: the round would be hashed over a subset, and the next
unfiltered request would report every excluded file as added.

### Reviewed state needs a new record variant, and the format must be made to tolerate one first
The objection to a new variant is real: `Record` is internally tagged with no catch-all
(`record.rs`), so a build that does not know a variant fails to parse the line, `fold` counts it in
`unparsed` (`log.rs`), and `safe_to_replace()` is `fully_drained() && unparsed == 0` — pinning it
false forever and wedging `rediff request` on a log containing no feedback at all.

But the alternative this design used to name — "an optional field on an existing record" — has no
host. Every variant has a lifecycle effect when appended: `Open` resets the whole `ReviewState`,
`Thread` becomes undelivered feedback (which is *precisely* the `request` wedge above, and would make
every `v` press into feedback the agent must drain), `Submit` closes a round, `Round` pushes a
duplicate, `Serve`/`Close` pair a server. The only fold-inert variant is `Drained`, whose meaning is
the opposite. Reviewed state is written on every `v`, so it needs a record of its own.

**So make the format tolerant first, then add the variant.** A `#[serde(other)] Unknown` catch-all on
`Record`, ignored by `apply`, means an unrecognised variant parses successfully and folds to nothing
rather than counting as `unparsed`. Verified against serde: `#[serde(other)]` is supported on an
internally-tagged enum, and a `{"t":"viewed",...}` line deserialises to `Unknown` instead of failing.

That does not protect builds predating the catch-all itself — the first new variant is still opaque
to those. The window is bounded and unavoidable; what matters is that it closes rather than recurring
for every future addition. The catch-all is a prerequisite task, not part of the reviewed-state work.

### New keys route through a new handler, not `handle_global_key`
That function is at CRAP 30 against a threshold of 30. The review keys get their own
`handle_review_key`, dispatched from the same router — which is also the better factoring, since they
are inert outside a review session and would otherwise add a guard to every arm.

### Reviewed state persists only while a review is open
`v` is live in every review session. Recording it unconditionally would create a log on the
most-pressed key in the application — precisely the outcome lazy opening exists to avoid. So marks
are written only once a review exists, and the promise is scoped to that: a review you have commented
on remembers what you read; a diff you only browsed still writes nothing.
- Restoring is by **path**, not by index: `ViewState.viewed` is positional over `cs.files`, and the
  file order can differ between sessions.

### The comment input is an overlay, not a mode
New `Overlay` variants carrying the anchor being commented on plus the text buffer. `Enter` confirms
(a comment is one line, as `Palette`'s input already is); escape discards. Because overlays *stack*,
editing a thread from the thread list pushes the input over it and pops back correctly for free.

Both variants must be added to `ui::frame::paint`'s dispatch chain — the one place a missing overlay
fails silently rather than at compile time.

### Keys, chosen from what is actually free
`c C t s v` are all taken (`handle_global_key`, and the stream context). Free and mnemonic enough:
`a` add comment, `A` review-level comment, `e` edit, `x` retract, `o` resolve, `n` thread list,
`y` submit. They route through a new `handle_review_key`, and each must be added to the keymap
catalog **by hand** — the consistency test will not catch an omission.

### Where the integration lives, given `tui::review` is taken
`src/tui/review.rs` already exists — viewed-tracking over `ViewState` — so `crate::tui::review` and
`crate::review` are already easy to confuse, and this change adds code that talks to both. The
TUI↔log integration goes in a distinctly named module (`tui/reviewlog.rs` or `tui/app/reviewlog.rs`),
not by extending `tui/review.rs`, and the log crate is referred to as `crate::review` at every use
site rather than imported bare.

## Risks / Trade-offs

- **Anchoring has no side to choose.** `add-line-cursor` shipped `cursor_key` as new-side-else-old,
  and `line-cursor`'s accepted spec forbids offering a side as a user-visible choice. The old worry
  here — that picking `new` unconditionally breaks a deleted file — was unfounded: a deleted file's
  rows carry only `Old` by construction. → Nothing to decide; the remaining work is the bridge from a
  file *index* to the `&DiffFile` that `capture` actually takes.
- **A comment on a context line.** Legal and useful ("this nearby code assumes X"), and the anchor
  captures fine. → No special case; just do not assume every anchor is on a changed line.
- **The keymap test runs the other way from what an earlier draft said.**
  `bindings_only_reference_documented_keys` (`keymap.rs`) asserts that every key in a *binding table*
  appears in the *help catalog* — bindings → help. So a key added to `BIND_STREAM` without a `HELP_*`
  row **does** fail the test. What escapes it is a key wired only into the router and never listed in
  a binding table. → Add the keys to a table and the catalog; note `ALL_TABLES` is a list of tables,
  so it changes only if this change introduces a `BIND_REVIEW`.
- **`handle_global_key` at exactly 30.** → New routing goes in its own function; verify the gate
  after wiring, not at the end.

## Open Questions

- Should submitting a round also mark every file reviewed, or are those independent? Leaning
  independent — a human may submit having deliberately skipped files.

Resolved during review: the TUI shows **all** threads, not only the current round's — not by
preference but by necessity, since `Thread` carries no round and `RoundInfo` no ordinal, so the fold
cannot attribute a thread to a round at all.
