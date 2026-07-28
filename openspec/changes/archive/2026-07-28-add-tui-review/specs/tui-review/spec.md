## ADDED Requirements

### Requirement: Comment on the line under the cursor
The system SHALL let the user comment on the diff line the cursor is on, anchoring the comment to
that file, side, and line number, and recording it in the worktree's review log. The anchor SHALL
capture the line's text and surrounding context so it can be re-resolved later.

#### Scenario: An anchored comment is recorded
- **WHEN** the user comments on a diff line and confirms
- **THEN** a thread anchored to that file, side, and line is recorded, carrying the line's text and context

#### Scenario: Discarding writes nothing
- **WHEN** the user opens the comment input and dismisses it without confirming
- **THEN** nothing is recorded and the view is exactly as it was

#### Scenario: A change is anchored to its new side
- **WHEN** the user comments on a change that exists on both sides
- **THEN** the anchor names the new side, the line cursor naming a change rather than a side of one

#### Scenario: A context line may be commented on
- **WHEN** the cursor is on an unchanged line within a hunk
- **THEN** commenting is allowed and the anchor resolves to that line

#### Scenario: Rows that are not diff lines are inert
- **WHEN** the cursor is on a file header, hunk gap, placeholder, or spacer
- **THEN** the comment key does nothing rather than anchoring to an arbitrary line

### Requirement: A review is opened by the first comment
The system SHALL open or attach to the worktree's review when the first comment is recorded, and
SHALL NOT create a review log merely because a diff was opened or browsed. Opening a review this way
SHALL record the current view's target and open a round over its changeset, so that a review begun
in the TUI is indistinguishable to a consumer from one begun by an agent.

#### Scenario: Browsing writes nothing
- **WHEN** the user opens, scrolls, and closes a diff without commenting
- **THEN** no review log exists

#### Scenario: The first comment opens the review
- **WHEN** the user records the first comment in a worktree with no review
- **THEN** a review is opened over the current view's target, a round is opened, and the comment is recorded

#### Scenario: Later comments attach
- **WHEN** the user records a second comment in the same session
- **THEN** it joins the same review and no second review is opened

#### Scenario: An agent-opened review is joined, not duplicated
- **WHEN** the user comments while a review opened by an agent over the same target is present
- **THEN** the comment joins that review

### Requirement: Commenting is confined to a review session
The system SHALL allow commenting only in a review session, using the same predicate that governs
reviewed-tracking, and SHALL be inert in a browsed view unless the user promotes it. A commit opened
as a review SHALL therefore accept comments, while the same commit reached by browsing SHALL not.

#### Scenario: Inert while browsing
- **WHEN** the user presses the comment key in a browsed commit view
- **THEN** nothing happens and no review is opened

#### Scenario: A commit opened as a review accepts comments
- **WHEN** the user opens a commit as a review rather than browsing to it, and comments
- **THEN** the comment is recorded, because that view is a review session

#### Scenario: Promoting enables commenting
- **WHEN** the user promotes a browse view into a review session
- **THEN** commenting becomes available

### Requirement: A mismatched view refuses to take comments
The system SHALL refuse to record a comment when a review is open over a different target than the
current view and that review holds feedback which has not been delivered, and SHALL say which target
the open review covers. When the open review has been fully delivered, commenting SHALL instead start
a fresh review over the current view's target.

#### Scenario: Refused while feedback is pending
- **WHEN** the user comments in one view while a review of a different target holds undelivered feedback
- **THEN** the comment is refused, both targets are named, and nothing is recorded

#### Scenario: Fresh review once delivered
- **WHEN** the user comments in a different view than a fully delivered review covered
- **THEN** a new review is opened over the current view's target

### Requirement: Commenting waits for a complete, unfiltered view
The system SHALL refuse to open a review while the view's diffs are still loading, and SHALL say so
rather than surfacing a low-level error. The system SHALL likewise refuse to open a review from a
view narrowed by path filters, because a round recorded over a subset would make a later unfiltered
review report every excluded file as newly added.

#### Scenario: Still loading
- **WHEN** the user comments before the view's diffs have all arrived
- **THEN** they are told the view is still loading, and no review or round is opened

#### Scenario: A filtered view cannot open a review
- **WHEN** the user comments in a view narrowed by path filters, with no review open
- **THEN** they are told a review covers a whole target, and nothing is opened

#### Scenario: An anchor that cannot be captured says so
- **WHEN** the user comments on a row that names a line whose text cannot be read
- **THEN** they are told the line cannot be anchored, rather than the key doing nothing

### Requirement: Review-level comment
The system SHALL let the user record a comment with no anchor, which is the review-level comment the
store already models.

#### Scenario: Unanchored comment recorded
- **WHEN** the user records a review-level comment
- **THEN** a thread with no anchor is recorded

### Requirement: See and revisit what has been said
The system SHALL mark lines that carry a comment so they are visible while scrolling, and SHALL
provide a list of the review's threads from which the user can jump to a thread's anchor.

#### Scenario: A commented line is marked
- **WHEN** a line carries a comment
- **THEN** that line is visibly marked in the diff

#### Scenario: Jump to a thread
- **WHEN** the user selects a thread from the list
- **THEN** the view moves to that thread's anchor

#### Scenario: A detached thread is still listed
- **WHEN** a thread's anchor no longer resolves in the current diff
- **THEN** it still appears in the list, marked as detached, rather than vanishing

#### Scenario: A thread in a file still loading is not called detached
- **WHEN** a thread is anchored in a file whose diff has not yet arrived
- **THEN** the list reports it as still resolving rather than as detached, since telling the user their code is gone would be false

### Requirement: Edit, retract, and resolve
The system SHALL let the user change a thread's text, retract it, and mark it resolved. Each SHALL be
recorded as a new entry rather than by altering what was already written, and SHALL preserve every
part of the thread it does not change.

Retraction and resolution SHALL both be reversible from within the review, and a retracted thread
SHALL remain visible in the thread list — otherwise a single keystroke would put a review point
beyond the user's reach while leaving it in the log.

#### Scenario: Editing supersedes
- **WHEN** the user changes a thread's text
- **THEN** the thread reads as the new text and the earlier entry is retained in the log

#### Scenario: Editing preserves the rest of the thread
- **WHEN** the user changes the text of a thread that is anchored and marked resolved
- **THEN** it is still anchored to the same line and still marked resolved

#### Scenario: Retracting withdraws from delivery
- **WHEN** the user retracts a thread
- **THEN** it is not delivered to a consumer, and the log still contains both entries

#### Scenario: A retraction can be undone
- **WHEN** the user retracts a thread and then retracts it again
- **THEN** it is delivered once more, and the thread list showed it throughout so it could be reached

#### Scenario: Resolving keeps the thread
- **WHEN** the user marks a thread resolved
- **THEN** it remains present and delivered, flagged resolved

### Requirement: Submit a round
The system SHALL let the user close the round with a parting instruction, chosen from configured
presets and editable before sending. The instruction SHALL be recorded exactly as sent.

#### Scenario: Submitting closes the round
- **WHEN** the user submits with an instruction
- **THEN** the round is closed and the instruction is recorded for delivery

#### Scenario: An edited preset is recorded as edited
- **WHEN** the user picks a preset, changes its text, and submits
- **THEN** the recorded instruction is the changed text, with the preset's name alongside

#### Scenario: Submitting is available without any anchored comment
- **WHEN** the user submits having left only a review-level comment
- **THEN** the round closes normally
