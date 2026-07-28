## ADDED Requirements

### Requirement: Request a review
The system SHALL provide a `request` subcommand that opens or attaches to the worktree's review,
opens a round over the current changeset, and exits without launching the interactive viewer or any
server. It SHALL report the review identifier, the round number, and the log's path. The subcommand SHALL
accept every target the viewer supports — working tree by default, staged changes, a single commit, a
range, and the combined net diff from a base ref through the working tree — plus an optional label.
It SHALL NOT accept path filters: a review covers everything that changed under its target.

When a review is already open over a *different* target **and holds feedback that has not been
delivered**, the subcommand SHALL refuse and name both targets, rather than attaching and reporting
success for a target the caller did not ask for. When the open review has been fully delivered, a
different target SHALL start a fresh review instead — refusing there would make every change of
target a one-way door.

The subcommand SHALL resolve the log's location from the discovered repository's worktree root, not
from the directory it was invoked in, so that running it from a subdirectory does not write a second
log that the loader will not filter out.

#### Scenario: A log that cannot be read is not added to
- **WHEN** `request` runs and the log holds lines this build cannot parse, with no readable open review
- **THEN** it fails saying so, and appends nothing

#### Scenario: A label given while attaching is reported
- **WHEN** `request` runs with a label while attaching to a review that already has one
- **THEN** the existing label is kept and the caller is told the given one was ignored

#### Scenario: A single commit is an accepted target
- **WHEN** `request` names a revision rather than a range
- **THEN** that commit is the review's target

#### Scenario: Requesting a review opens one
- **WHEN** `request` runs in a worktree with changes and no review open
- **THEN** a review is opened, a first round is recorded, and the review identifier, round number, and log path are reported

#### Scenario: Requesting again attaches rather than duplicating
- **WHEN** `request` runs while the log holds a review with feedback that has not been delivered
- **THEN** the existing review is reported rather than a new one being started, and no second review is opened

#### Scenario: A later request opens the next round
- **WHEN** `request` runs again after the previous round was delivered and the changeset has since moved
- **THEN** a new round is recorded over the current changeset and its number is reported

#### Scenario: Rounds continue across a delivery
- **WHEN** a round is opened, its feedback delivered, the changeset changed, and `request` run again over the same target
- **THEN** the next round of the *same* review is opened, rather than a new review starting over at the first round

#### Scenario: A moving target still opens later rounds
- **WHEN** a review's target names a revision that advances, such as a range ending at `HEAD`, and the changeset moves between requests
- **THEN** a later request still opens a new round rather than reporting the first one forever

#### Scenario: Nothing to review
- **WHEN** `request` runs and the target's changeset contains no files
- **THEN** the command reports that there is nothing to review, and opens neither a review nor a round

#### Scenario: Nothing to review, but a review is already open
- **WHEN** `request` runs against an empty changeset while a review is open
- **THEN** the report names that open review, so the caller is not told merely that there is nothing to do

#### Scenario: Committed and uncommitted work reviewed together
- **WHEN** `request` runs against a base ref while the worktree has both commits since that ref and uncommitted changes
- **THEN** the round covers both, as one changeset

#### Scenario: A different target is refused while feedback is pending
- **WHEN** `request` runs naming one target while a review over a different one holds undelivered feedback
- **THEN** it fails, naming both the requested target and the open review's, and no round is opened

#### Scenario: A different target starts fresh once delivered
- **WHEN** `request` runs naming a different target than a review that has been fully delivered
- **THEN** a new review is started over the requested target

#### Scenario: Run from a subdirectory
- **WHEN** `request` runs from a subdirectory of the worktree
- **THEN** the log is written at the worktree root, not in that subdirectory

#### Scenario: The same target attaches normally
- **WHEN** `request` runs naming the target the open review already covers
- **THEN** it attaches as usual rather than failing

#### Scenario: No interactive surface is launched
- **WHEN** `request` runs with a terminal attached
- **THEN** no alternate screen is entered, no keyboard input is read, and the process exits on its own

### Requirement: Report review state
The system SHALL provide a `review-status` subcommand that reports the worktree's review state: whether a
review is open, its label and target, the current round number, how many feedback items are pending
delivery, and whether a serving process recorded itself against it. Output SHALL be human-readable by
default and machine-readable when JSON is requested.

#### Scenario: No review open
- **WHEN** `review-status` runs in a worktree whose log does not exist or holds no review
- **THEN** it reports that no review is open and exits successfully

#### Scenario: Open review is summarized
- **WHEN** `review-status` runs while a review is open with pending feedback
- **THEN** it reports the review's label, target, current round, and the count of pending items

#### Scenario: Serving state is reported
- **WHEN** `review-status` runs for a review whose log records a server start with no matching stop
- **THEN** it reports the recorded URL, without asserting that the process is still running

#### Scenario: Machine-readable output
- **WHEN** `review-status` runs with JSON output requested
- **THEN** the same facts are emitted as JSON and nothing else is written to standard output

### Requirement: Drain feedback as JSON
The system SHALL provide a `feedback` subcommand that emits the review's feedback as JSON on standard
output. By default it SHALL deliver only items not previously delivered and SHALL record that they
were delivered. When a full replay is requested it SHALL emit every item with its delivery status and
SHALL record nothing.

Every emitted item SHALL carry its body, its anchor when it has one, and the anchor's resolution
against the current changeset — including, for a moved line, both the recorded and the current line
number. An item whose anchor could not be resolved SHALL still be emitted, carrying the recorded line
text and surrounding context. A suggested replacement, when present, SHALL be emitted verbatim.

#### Scenario: Delivery is recorded only after the output is written
- **WHEN** `feedback` cannot write its document, such as when the consumer closes the pipe early
- **THEN** nothing is marked delivered, so a later run still returns the same items

#### Scenario: Undelivered items are delivered once
- **WHEN** `feedback` runs twice with nothing recorded in between
- **THEN** the first run emits the pending items and the second emits none

#### Scenario: Full replay changes nothing
- **WHEN** `feedback` runs with a full replay requested
- **THEN** every item is emitted with its delivery status and the log is byte-identical afterwards

#### Scenario: Anchors are resolved against the recorded target
- **WHEN** `feedback` runs for a review whose target is a commit rather than the working tree
- **THEN** anchors are resolved against that commit's changeset, not against the working tree

#### Scenario: A moved line reports both positions
- **WHEN** an anchored line has moved since the comment was recorded
- **THEN** the emitted item reports the anchor as shifted, with the recorded line number and the current one

#### Scenario: An unresolvable anchor is still emitted
- **WHEN** an anchored line no longer exists
- **THEN** the item is still emitted, marked as detached, carrying the line text and context it was recorded against

#### Scenario: Output is JSON only
- **WHEN** `feedback` runs
- **THEN** standard output contains only the JSON document, so a consumer can parse it without stripping other text

#### Scenario: No review to drain
- **WHEN** `feedback` runs in a worktree with no review open
- **THEN** it reports that there is no review and exits without error

### Requirement: The review target round-trips
The system SHALL encode a review's target into the log in a canonical form that parses back to the
same target, for every target the viewer supports: working-tree changes with or without untracked
files and with or without an explicit base, staged changes, a single commit, a two-dot range, and a
branch review range. Encoding SHALL be lossless — in particular, whether untracked files are included
SHALL survive the round trip.

#### Scenario: The combined base-through-worktree target round-trips
- **WHEN** a target covering everything from a base ref through the working tree is encoded and parsed back
- **THEN** the parsed target names the same base and the same untracked-inclusion setting

#### Scenario: Every target encodes and parses back
- **WHEN** each supported target is encoded and the result parsed
- **THEN** the parsed target equals the original in every field

#### Scenario: Untracked inclusion survives
- **WHEN** a working-tree target that excludes untracked files is encoded and parsed back
- **THEN** the parsed target also excludes untracked files

#### Scenario: Revision expressions survive verbatim
- **WHEN** a target naming a revision expression containing punctuation, such as `HEAD~2`, is encoded and parsed back
- **THEN** the revision expression is unchanged

#### Scenario: An unparseable target is an error, not a guess
- **WHEN** a log records a target this build cannot parse
- **THEN** the command fails with a message naming the recorded target rather than silently substituting a default

#### Scenario: Feedback survives a target whose refs are gone
- **WHEN** `feedback` runs for a review whose recorded target no longer resolves
- **THEN** the recorded feedback is still emitted, with every anchor reported as detached and carrying its recorded quote and context, and a warning is reported separately from the emitted document


### Requirement: Ignore hint on a new review
When a command starts a *new* review — as opposed to attaching to one already open — the system SHALL
report that `rediff.jsonl` can be added to `.gitignore`. The system SHALL NOT modify any git ignore
configuration itself. The hint SHALL be written separately from any machine-readable output, so a
consumer parsing that output is unaffected.

#### Scenario: Hint when a review is started
- **WHEN** a command starts a new review in a worktree
- **THEN** the ignore hint is reported

#### Scenario: No hint when attaching
- **WHEN** a command attaches to a review that is already open
- **THEN** no ignore hint is reported

#### Scenario: The hint does not pollute machine-readable output
- **WHEN** a command that emits JSON reports the hint
- **THEN** the JSON document on standard output is still parseable on its own

#### Scenario: Git configuration is untouched
- **WHEN** the hint is reported
- **THEN** no git ignore configuration file has been created or modified
