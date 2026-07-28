## ADDED Requirements

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

## MODIFIED Requirements

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
