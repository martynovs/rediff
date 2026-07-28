## MODIFIED Requirements

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

## ADDED Requirements

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
