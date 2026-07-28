# viewed tracking

## Requirements

### Requirement: Mark files reviewed
The system SHALL let the user mark a file as reviewed and unmark it, and SHALL show each file's
reviewed state in the sidebar.

#### Scenario: Toggle reviewed
- **WHEN** the user marks the current file as reviewed
- **THEN** the sidebar shows that file as reviewed, and unmarking restores the unreviewed state

### Requirement: Jump to next unviewed
The system SHALL provide a command to jump to the next file that has not been marked reviewed.

#### Scenario: Skip reviewed files
- **WHEN** the user invokes next-unviewed while some files are marked reviewed
- **THEN** the stream jumps to the next file that is not marked reviewed

#### Scenario: All reviewed
- **WHEN** the user invokes next-unviewed and every file is reviewed
- **THEN** the system indicates there are no unviewed files remaining

### Requirement: Collapse reviewed files
The system SHALL be able to collapse reviewed files in the sidebar so the remaining work is
easier to scan.

#### Scenario: Reviewed files collapse
- **WHEN** a file is marked reviewed and collapsing is enabled
- **THEN** that file's entry is collapsed in the sidebar while unreviewed files stay expanded

### Requirement: Per-view review sessions
Reviewed state SHALL be a property of a view rather than a single global. A view launched as working-tree, staged, or `review` changes SHALL be a review session that carries reviewed state; a view reached by browsing into a commit SHALL NOT be a review session. Reviewed-tracking commands (`v`, next-unviewed, collapse, the reviewed count) SHALL act on the current view only when it is a review session, and SHALL be inert otherwise. A review session's progress SHALL persist while the user browses other views and SHALL be restored when that view is shown again. The system SHALL let the user promote the current browse view into a review session with `R`.

#### Scenario: Reviewed commands inert while browsing
- **WHEN** the current view is a browsed commit (not a review session)
- **THEN** pressing `v` or next-unviewed has no effect and no reviewed count is shown

#### Scenario: Review progress survives browsing
- **WHEN** the user marks files reviewed in a review session, browses into a commit, and returns to the review session
- **THEN** the previously reviewed files are still marked reviewed

#### Scenario: Promote a commit to a review
- **WHEN** the user presses `R` on a browsed commit view
- **THEN** that view becomes a review session with its own reviewed state and reviewed-tracking commands take effect

### Requirement: Reviewed state persists with an open review
The system SHALL record which files the user has marked reviewed **once a review is open**, and SHALL
restore that state when the same review is opened again in a later session. The system SHALL NOT
create a review in order to record reviewed state, so marking files reviewed while merely browsing
a diff writes nothing. Restoration SHALL match files by path rather than by position, since a
changeset's file order may differ between sessions.

#### Scenario: Reviewed files survive a restart
- **WHEN** the user marks files reviewed in a review that is open, leaves, and opens that review again
- **THEN** those files are still marked reviewed

#### Scenario: Marking reviewed does not open a review
- **WHEN** the user marks files reviewed with no review open
- **THEN** no review log is created

#### Scenario: Restoration follows paths
- **WHEN** a review is reopened and its changeset lists the files in a different order
- **THEN** the same files are marked reviewed, not the same positions
