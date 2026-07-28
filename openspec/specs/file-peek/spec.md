# file peek

## Purpose
The modal single-file view layered over the stream — the whole file, its diff at adjustable context,
or its blame — so a file can be read in full without losing the reader's place in the review, and
without becoming an entry in the view history.

## Requirements
### Requirement: Peek loads its own file during streaming
The single-file peek SHALL source the peeked file's content directly from git (by path and the view's base/new refs) rather than from the changeset's cached text, so that preview and diff work on any file the moment the file list appears — even before that file's bulk diff has run.

#### Scenario: Preview an undiffed file
- **WHEN** the file list is shown, a file has not yet been diffed, and the user opens the peek in content mode
- **THEN** the file's content is loaded and shown without waiting for the bulk diff

#### Scenario: Diff an undiffed file
- **WHEN** the user opens the peek in diff mode on a not-yet-diffed file
- **THEN** that one file's diff is computed on demand against the view's base and shown

### Requirement: Single-file peek overlay
The system SHALL provide a modal, full-area, scrollable, syntax-highlighted overlay showing exactly one file, opened from the selected file. The overlay SHALL be ephemeral: it captures input while open, does not create a view-history entry, and closing it SHALL restore the previous view unchanged. The overlay SHALL NOT provide viewed-tracking.

#### Scenario: Open and close
- **WHEN** the user opens the peek for a file and then presses Esc
- **THEN** the peek closes and the previous view is shown exactly as before

#### Scenario: No history entry
- **WHEN** the peek is open and the user closes it
- **THEN** the view-history back/forward state is unchanged (the peek created no entry)

#### Scenario: Scrolls and highlights
- **WHEN** the peek shows a long file
- **THEN** its content is syntax-highlighted and can be scrolled independently of the main view

### Requirement: Content and diff modes
The peek SHALL have three modes — content (the whole file, no diff markers), diff (a unified diff for the file), and blame (the whole file with a per-line commit-attribution gutter) — and Tab SHALL cycle through them in place.

#### Scenario: Cycle modes
- **WHEN** the user presses Tab in the peek
- **THEN** the peek advances through content, diff, and blame and wraps back to content

#### Scenario: Content mode shows the whole file
- **WHEN** the peek is in content mode
- **THEN** every line of the file is shown with line numbers and highlighting and no add/remove markers

#### Scenario: Blame mode shows attribution
- **WHEN** the peek is in blame mode
- **THEN** every line of the file is shown with its committed-rev attribution gutter in place of the line numbers

### Requirement: History and review open keys
The system SHALL open the peek from the selected file with two keys whose diffs share the same end point (`TOP`, the newest side of the current review context) but differ in start point:
- `p` (history) SHALL open in content mode showing the file at the commit being viewed, with its diff being that commit versus `TOP`.
- `=` (review) SHALL open in diff mode showing the view's own change for the file (its base versus `TOP`), with the context level expanded beyond the main view's.

#### Scenario: History peek from a commit
- **WHEN** the user presses `p` on a file while viewing a commit
- **THEN** the peek shows that commit's version of the file, and toggling to diff shows the change from that commit to `TOP`

#### Scenario: Review peek anchors at TOP
- **WHEN** the user presses `=` on a file
- **THEN** the peek shows the file's own change diff (base to `TOP`) with expanded context

#### Scenario: TOP follows the review context
- **WHEN** the active view is a range review `base..target`
- **THEN** the peek's diffs end at the target commit, not at the working copy

### Requirement: Adjustable diff context
In diff mode the peek SHALL expand the surrounding context with `=`/`+` and compact it with `-`/`_`, rebuilding the diff at the new context level. The level SHALL be clamped between a minimal hunk view and the whole file.

#### Scenario: Expand context
- **WHEN** the user presses `=` in diff mode
- **THEN** more unchanged lines are shown around each change

#### Scenario: Compact context
- **WHEN** the user presses `-` in diff mode
- **THEN** fewer unchanged lines are shown around each change

### Requirement: Source color
The peek SHALL inherit the origin view's source accent: blue when opened from a local or staged view, green/magenta (the commit accent) when opened from a commit or range view.

#### Scenario: Local origin is blue
- **WHEN** the peek is opened from a working-tree or staged view
- **THEN** its frame uses the local (blue) accent

#### Scenario: Commit origin is the commit accent
- **WHEN** the peek is opened from a commit or range view
- **THEN** its frame uses the commit accent

### Requirement: Open blame directly
The system SHALL open the peek for the selected file directly in blame mode with `b`, from either focus, so blame is reachable in one key without first opening the peek and cycling modes. The peek opened this way SHALL otherwise behave as the modal single-file peek (ephemeral, no view-history entry, restoring the previous view on close).

#### Scenario: b opens blame
- **WHEN** the user presses `b` with a file selected
- **THEN** the peek opens for that file in blame mode

#### Scenario: Close restores the previous view
- **WHEN** the user closes a blame peek opened with `b`
- **THEN** the previous view is shown exactly as before and the view-history state is unchanged

#### Scenario: Inert on a collapsed placeholder
- **WHEN** the cursor is on a collapsed directory placeholder rather than a file and the user presses `b`
- **THEN** nothing is opened

