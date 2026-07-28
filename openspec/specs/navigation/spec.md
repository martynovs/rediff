# navigation

## Requirements

### Requirement: Sidebar file list
The system SHALL show a sidebar listing the changed files with per-file added/removed line
stats, in changeset order.

#### Scenario: Files and stats listed
- **WHEN** a changeset is opened
- **THEN** the sidebar lists each changed file with its addition and deletion counts in changeset order

### Requirement: Jump to file
The system SHALL let the user select a file in the sidebar to jump to that file's position in
the review stream.

#### Scenario: Select to jump
- **WHEN** the user selects a file in the sidebar
- **THEN** the review stream scrolls so that file's header is at the top of the viewport

### Requirement: Fuzzy file jump
The system SHALL provide a fuzzy file-jump affordance that filters files by typed substring and
jumps to the chosen file.

#### Scenario: Type to jump
- **WHEN** the user opens fuzzy jump and types part of a filename
- **THEN** the matching files are shown and selecting one jumps the stream to that file

### Requirement: Hunk navigation across the stream
The system SHALL navigate to the previous and next hunk across the entire review stream with
`[` and `]`.

#### Scenario: Next hunk crosses file boundaries
- **WHEN** the user presses `]` while on the last hunk of a file
- **THEN** the viewport moves to the first hunk of the next file in the stream

#### Scenario: Previous hunk
- **WHEN** the user presses `[`
- **THEN** the viewport moves to the previous hunk in the stream

#### Scenario: Hunk navigation keys are reserved
- **WHEN** hunk navigation is used
- **THEN** only `[` and `]` move between hunks (the system does not bind `j`/`k` to hunk navigation)

### Requirement: Incremental motion is cursor-relative
The system SHALL treat the incremental motion commands as moving the cursor, with the viewport
following only as far as needed. This SHALL NOT change the jump commands, which continue to place
their target at the top of the viewport.

#### Scenario: Incremental motion within the viewport does not scroll
- **WHEN** the user presses an incremental motion key and the target row is already visible
- **THEN** the cursor moves and the viewport does not

#### Scenario: Jumps are unaffected
- **WHEN** the user jumps to a file
- **THEN** that file's header is at the top of the viewport, as before
