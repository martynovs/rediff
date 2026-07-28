## MODIFIED Requirements

### Requirement: Sidebar file list
The system SHALL show a sidebar listing the changed files with per-file
added/removed line stats, in path order (by parent directory then file name).
The list MAY be shown flat or grouped by directory; in either case file
selection and the jump digits address files, not list rows.

#### Scenario: Files and stats listed
- **WHEN** a changeset is opened
- **THEN** the sidebar lists each changed file with its addition and deletion counts, in path order

#### Scenario: Grouped view available
- **WHEN** the user switches the sidebar to the directory-grouped view
- **THEN** the same files are shown beneath directory lines, and selecting or jumping to a file still addresses files (directory lines are skipped)
