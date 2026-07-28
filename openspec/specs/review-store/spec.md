# review-store Specification

## Purpose
The append-only per-worktree review log — its path and record schema, the review and round lifecycle, self-contained anchors and their re-anchoring rules, drain-once delivery with full replay, and the recorded serve lifecycle — so a human's feedback on a change outlives the process that collected it and reaches the agent that made the change.

## Requirements
### Requirement: Per-worktree append-only review log
The system SHALL persist a review to a single append-only JSON Lines file at the worktree root,
`<worktree>/rediff.jsonl`, where the worktree root is the working directory git reports for the
opened repository. Each linked worktree SHALL have its own file. The system SHALL NOT create
directories for review state, SHALL NOT write review state inside the git directory, and SHALL NOT
write review state to a global or user-level location. Every record SHALL be a single line of JSON
carrying a `t` discriminator, and the system SHALL only ever append — no record is rewritten or
removed in place.

#### Scenario: Log created at the worktree root
- **WHEN** a review is opened for a repository
- **THEN** `rediff.jsonl` is created at that worktree's root and nothing is written under the git directory or in a user-level state directory

#### Scenario: Linked worktrees are independent
- **WHEN** reviews are opened in two linked worktrees of the same repository
- **THEN** each worktree has its own `rediff.jsonl` and neither review's records appear in the other

#### Scenario: Appends never rewrite
- **WHEN** any record is written after others already exist
- **THEN** the earlier lines are byte-identical to what they were before and the new record is the final line

#### Scenario: One-time ignore hint
- **WHEN** the log file is created for a worktree for the first time
- **THEN** the system reports once that `rediff.jsonl` can be added to `.gitignore`, and does not modify any git ignore configuration itself

### Requirement: Record ordinal is the line number
The system SHALL use each record's 1-based line number in the log as its ordinal, counting raw
lines including any that fail to parse, so that no separate sequence field exists and a malformed
line cannot shift the ordinals of later records.

#### Scenario: Ordinals follow file order
- **WHEN** records are appended in sequence
- **THEN** each record's ordinal equals its line number in the file

#### Scenario: A malformed line does not shift ordinals
- **WHEN** the log contains a line that is not valid JSON among valid records
- **THEN** that line is skipped for interpretation but still consumes its ordinal, and the records after it keep the ordinals their line positions give them

### Requirement: Review lifecycle
The system SHALL record the opening of a review with its target and a human-readable label.

Whether opening a review replaces the existing log SHALL be decided by delivery alone: a review is
*finished* when every thread and submit in it has been delivered, and a review with any undelivered
feedback SHALL NOT be replaced. Opening a review while the log holds an unfinished review SHALL
attach to that review and write no new `open` record. Opening a review while the log holds a
finished review SHALL start a fresh log by default, and SHALL append to the existing log when the
caller asks to keep it. A review that recorded no feedback at all is vacuously finished.

#### Scenario: Attach when feedback is pending
- **WHEN** a review is opened and the log's review holds a thread or submit that has not been delivered
- **THEN** the existing review is returned, no second `open` record is written, and the log is unchanged

#### Scenario: Fresh log after a fully delivered review
- **WHEN** a review is opened and every thread and submit in the log's review has been delivered
- **THEN** the log is replaced by a new one containing only the new review's `open` record

#### Scenario: Keep the previous review
- **WHEN** a review is opened with keep requested and the log's review is fully delivered
- **THEN** the new `open` record is appended and the previous review's records are retained

#### Scenario: A log this build cannot fully read is never replaced
- **WHEN** a review is opened and the log contains lines that do not parse, such as a line torn by a crash mid-append
- **THEN** the log is not replaced, because unreadable is not the same as empty

#### Scenario: A record type from a newer version does not pin the log
- **WHEN** a review is opened and the log's only unfamiliar content is a well-formed record whose type this build does not know
- **THEN** the log may still be replaced, because a record this build can read and knows to ignore is not damage

#### Scenario: A retracted thread does not block finishing
- **WHEN** every thread in a review has been retracted and nothing else is pending
- **THEN** the review counts as finished, since retracted threads are never delivered

#### Scenario: An empty review may be replaced
- **WHEN** a review is opened and the log's review recorded no threads or submits
- **THEN** a fresh log is started, since there is no feedback to lose

#### Scenario: Undelivered feedback is never discarded
- **WHEN** a review is opened with undelivered feedback present, by a caller that did not ask to keep the log
- **THEN** the log is still not truncated, because attaching takes precedence over starting fresh

### Requirement: Rounds record per-file content hashes
The system SHALL record the opening of a review round with an increasing round number and a map from
each changeset file path to a content hash of that file's reviewed content. The hash SHALL be
computed by a named, fixed algorithm whose output is identical for identical input across runs,
builds, and toolchain versions; a hasher whose output is unspecified between versions (such as the
standard library's default hasher, or one documented as unstable across releases) SHALL NOT be used.
The system SHALL NOT store file content in the log. The system SHALL report, for
any round compared against another round or against a current changeset, the paths whose hash
differs and the paths present in only one of the two.

#### Scenario: Round records hashes, not content
- **WHEN** a round is opened over a changeset
- **THEN** the record holds one hash per file path and no file content appears in the log

#### Scenario: Changed files since a round
- **WHEN** a later changeset is compared against an earlier round
- **THEN** the files whose content changed are reported, together with files that appeared or disappeared, and unchanged files are not reported

#### Scenario: Hash is stable across runs
- **WHEN** the same file content is hashed in two separate runs of the system
- **THEN** the two hashes are equal

#### Scenario: A frozen target has one round
- **WHEN** a review targets a commit or a range rather than the working tree
- **THEN** exactly one round is opened for it

#### Scenario: An unchanged changeset does not open a new round
- **WHEN** a round is opened over a changeset in which no file has moved since the previous round
- **THEN** the previous round is returned and no new record is written

#### Scenario: A changed binary file is reported as changed
- **WHEN** a binary file's content is replaced between two rounds
- **THEN** it is reported as modified, the same as a text file

#### Scenario: A half-loaded changeset cannot open a round
- **WHEN** a round is opened over a changeset whose files have not all been diffed
- **THEN** the attempt fails and no round record is written

#### Scenario: A file still being diffed is not reported as removed
- **WHEN** a changeset is compared against a round while one of its files is still being diffed
- **THEN** that file is reported in no bucket, since not yet computed is not the same as deleted

### Requirement: One feedback record with an optional anchor
The system SHALL represent every piece of human feedback as a single `thread` record carrying an
identifier, a body, an optional anchor, an optional replacement text, and an optional resolved flag.
A thread with an anchor SHALL be a comment on that anchored location; a thread without an anchor
SHALL be a review-level comment. There SHALL NOT be a separate record type for review-level feedback.

#### Scenario: Anchored comment
- **WHEN** a thread is recorded with an anchor
- **THEN** it is delivered as feedback on that anchored location

#### Scenario: Review-level comment
- **WHEN** a thread is recorded without an anchor
- **THEN** it is delivered as feedback on the review as a whole

#### Scenario: Replacement text is carried through
- **WHEN** a thread is recorded with replacement text for its anchored location
- **THEN** that text is delivered verbatim alongside the comment body

### Requirement: Threads are superseded, not rewritten
The system SHALL identify a thread by its identifier, and SHALL treat a later thread record bearing
the same identifier as superseding the earlier one. A record marked deleted SHALL retract the thread
from delivery while remaining in the log. Replaying the log SHALL fold records by identifier so that
the last record for each identifier determines the thread's current state.

#### Scenario: Edit supersedes
- **WHEN** a thread is recorded and later re-recorded with the same identifier and a different body
- **THEN** the folded state carries the later body and both records remain in the log

#### Scenario: Retraction removes from delivery only
- **WHEN** a thread is recorded and later re-recorded as deleted
- **THEN** it is not delivered as feedback and both records remain in the log

#### Scenario: Two threads on one location
- **WHEN** two threads with different identifiers are recorded against the same anchored location
- **THEN** both are retained and both are delivered

#### Scenario: Resolved threads are retained
- **WHEN** a thread is marked resolved
- **THEN** it is still present in the folded state, flagged resolved, and is not deleted

### Requirement: Submit closes a round
The system SHALL record the human's parting instruction for a round as a `submit` record carrying the
round number, the instruction body, and optionally the name of the configured preset the body was
derived from. A `submit` SHALL close its round. The body SHALL be delivered as written, so a caller
that edits a preset's text before sending has that edited text delivered.

#### Scenario: Submit ends the round
- **WHEN** a submit record is written for the open round
- **THEN** that round is closed and a subsequent round opens with the next number

#### Scenario: Edited preset text is preserved
- **WHEN** a submit record is written with a preset name and a body that differs from that preset's configured text
- **THEN** the body is delivered exactly as written and the preset name is delivered alongside it

### Requirement: Anchors are self-contained
The system SHALL record an anchor as the file path, the side of the diff, the line number on that
side, the exact text of that line, and a bounded number of surrounding lines of context on the same
side. An anchor SHALL NOT reference any content stored outside the log.

#### Scenario: Anchor carries its own context
- **WHEN** an anchor is recorded for a diff line
- **THEN** the record contains that line's text and its surrounding context lines, and resolving it later requires no other stored artifact

### Requirement: Anchor resolution never drops a thread
The system SHALL resolve a recorded anchor against a current changeset to one of four outcomes:
attached, when the recorded text is found at the recorded line on the recorded side; shifted, when it
is found elsewhere within a bounded window, reporting both the original and the resolved line;
detached, when the file is absent or no acceptable candidate is found; or unresolved, when the file
is present but its diff has not been computed yet. Unresolved SHALL NOT be reported as detached — a
file that is still loading is not a file whose code is gone. Candidates SHALL be scored by
how much of the recorded surrounding context also matches, with ties broken by proximity to the
recorded line. A thread whose anchor is detached SHALL still be delivered, carrying its recorded line
text and context.

#### Scenario: Unchanged line attaches
- **WHEN** an anchor is resolved against a changeset in which its line is unchanged
- **THEN** the outcome is attached at the recorded line

#### Scenario: Shifted line is found and reported
- **WHEN** lines are inserted above an anchored line so that it moves
- **THEN** the outcome is shifted, and both the recorded line number and the resolved line number are reported

#### Scenario: Deleted line detaches
- **WHEN** the anchored line no longer exists anywhere in the file
- **THEN** the outcome is detached

#### Scenario: Missing file detaches
- **WHEN** the anchored file is absent from the changeset
- **THEN** the outcome is detached

#### Scenario: A renamed file keeps its anchors
- **WHEN** the anchored file was renamed, so the changeset carries it under a new path with the recorded path as its rename source
- **THEN** the anchor resolves against that file rather than detaching

#### Scenario: A file still being diffed is unresolved
- **WHEN** the anchored file is listed in the changeset but its diff has not been computed
- **THEN** the outcome is unresolved, and it is not reported as detached

#### Scenario: A detached thread is still delivered
- **WHEN** feedback is delivered and a thread's anchor is detached
- **THEN** that thread is included, flagged detached, and carries the line text and context it was recorded against

#### Scenario: Context breaks a tie between identical lines
- **WHEN** the recorded line text occurs several times within the search window
- **THEN** the candidate whose surrounding context best matches the recorded context is chosen, and equally-scoring candidates resolve to the one nearest the recorded line

### Requirement: Drain-once delivery with full replay
The system SHALL deliver as undelivered feedback every thread and submit record whose ordinal is
greater than the ordinal recorded by the most recent drain marker, and SHALL append a new drain
marker recording the ordinal delivered through. The system SHALL also offer a full delivery of the
folded review state with delivery status included, which appends nothing. Both SHALL be derived from
one replay of the same log. The system SHALL NOT require the caller to supply or retain a cursor.

#### Scenario: Undelivered records are delivered once
- **WHEN** feedback is drained and then drained again with nothing recorded in between
- **THEN** the first drain returns the pending records and the second returns none

#### Scenario: Drain marker records the boundary
- **WHEN** feedback is drained
- **THEN** a drain marker is appended carrying the ordinal that was delivered through

#### Scenario: Full replay appends nothing
- **WHEN** the full folded state is requested
- **THEN** every thread and every round-closing instruction is returned with its delivery status, and no record is appended to the log

#### Scenario: A stale instruction is distinguishable
- **WHEN** the full folded state is requested for a review whose earlier round was closed and delivered
- **THEN** that earlier instruction is flagged as already delivered, so a recovering consumer does not act on it again

#### Scenario: An edit after a drain is undelivered
- **WHEN** a thread is drained and then superseded by a later record
- **THEN** the superseding record is undelivered and the next drain delivers it

#### Scenario: Feedback is surface-agnostic
- **WHEN** threads are appended by one writer and drained by another
- **THEN** the delivered feedback is identical regardless of which writer recorded it

### Requirement: Serve lifecycle is recorded
The system SHALL record the start of a serving process with its process id, bound port, and URL, and
SHALL record that process stopping with the same process id and a reason. Replaying the log SHALL
expose the most recent start record and whether a stop record for that same process id followed it.
The system SHALL NOT probe whether a recorded process is still running; interpreting an unpaired
start is the responsibility of the caller that owns server processes.

#### Scenario: Start is recorded with pid, port, and URL
- **WHEN** a serving process records its start
- **THEN** the log carries its process id, bound port, and URL, and the folded state reports them

#### Scenario: Unpaired start is reported as such
- **WHEN** a start record has no stop record for the same process id after it
- **THEN** the folded state reports the start as unpaired, without any claim about whether that process is running

#### Scenario: Stop pairs with its start
- **WHEN** a stop record is written carrying the process id of the most recent start
- **THEN** the folded state reports that start as paired, together with the recorded reason

#### Scenario: A later start supersedes an earlier stopped one
- **WHEN** a start, a matching stop, and then a second start are recorded
- **THEN** the folded state reports the second start as the most recent one and as unpaired

### Requirement: Concurrent appends stay intact
The system SHALL ensure that a record appended while another writer is appending is written as one
whole line, so that no two records interleave within a line and no record is truncated.

#### Scenario: Two writers append at once
- **WHEN** two writers append records to the same log concurrently
- **THEN** every line in the resulting log parses as a complete record and no record is lost or split

### Requirement: Unknown record types are tolerated, not counted as damage
The system SHALL read a well-formed record whose type it does not recognize as a known-unknown: it
SHALL be ignored when folding the log, SHALL still consume its ordinal, and SHALL NOT count toward
the lines the build could not read. A line that is not well-formed at all — torn by a crash, or
corrupt — SHALL still count as unreadable.

Because an older build silently ignores what it does not recognize, every record type added after
this SHALL be inert when folded: it SHALL NOT be feedback, and SHALL NOT be the only thing standing
between the log and replacement.

#### Scenario: A record from a newer version folds to nothing
- **WHEN** the log contains a well-formed record whose type this build does not know
- **THEN** it contributes nothing to the review's state, consumes its ordinal, and does not make the log unreadable

#### Scenario: A torn line is still damage
- **WHEN** the log contains a line that is not valid JSON, such as a record truncated mid-append
- **THEN** that line counts as unreadable and the log is not replaced

### Requirement: Reviewed files are recorded with the review
The system SHALL record which files the user has marked reviewed as a whole-set snapshot of paths,
belonging to the review that is open. It SHALL record nothing when no review is open, and nothing
when the open review is over a different target. Restoration SHALL match files by path, since a
changeset's file order may differ between sessions. The record SHALL be inert when folded: it is
not feedback, so it neither pins the review against replacement nor gives a consumer anything to
drain.

#### Scenario: The latest snapshot wins
- **WHEN** several reviewed-files records exist in one review
- **THEN** the most recent one is the review's reviewed set

#### Scenario: Reviewed state does not survive a new review
- **WHEN** a new review is opened
- **THEN** its reviewed set starts empty rather than inheriting the previous review's

#### Scenario: Recording reviewed files does not create work for a consumer
- **WHEN** a review's only records since the last drain are reviewed-files records
- **THEN** a drain has nothing to deliver and the review still counts as finished
