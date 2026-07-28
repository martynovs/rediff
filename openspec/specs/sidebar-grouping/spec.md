# sidebar-grouping Specification

## Purpose
Grouping the sidebar's file list by directory — the grouped view, its toggle, and the rule that a
directory line is chrome rather than a target — so a changeset spread across a tree reads as a tree
rather than as a flat list of long paths.

## Requirements
### Requirement: Directory-grouped sidebar view
The system SHALL provide a sidebar view mode that groups the file list by
directory: each group is introduced by a directory line showing that group's
combined parent path, followed by the files in that directory. The directory
line SHALL be visually de-emphasized (dim/muted). Files whose parent is the
repository root SHALL be grouped under a `./` directory line. The combined
directory path SHALL be shortened to fit the sidebar width.

#### Scenario: Files grouped under directory lines
- **WHEN** the sidebar is in grouped mode and the changeset has files in several directories
- **THEN** each directory's files appear together beneath a dim directory line showing that directory's path, and a subdirectory appears under its own directory line

#### Scenario: Root files under "./"
- **WHEN** the sidebar is in grouped mode and a file sits at the repository root
- **THEN** it appears beneath a `./` directory line

#### Scenario: File rows show basenames when grouped
- **WHEN** the sidebar is in grouped mode
- **THEN** each file row shows the file's name (not its full path, which is in the directory line), shortened if too long

### Requirement: Toggle sidebar grouping
The system SHALL let the user toggle the sidebar between the flat list and the
directory-grouped view with the `D` key. The choice SHALL apply to the whole
session (it is not per file or per view), and the directory-grouped view SHALL be the default (press `D` for the flat list).

#### Scenario: Toggle to grouped and back
- **WHEN** the user presses `D` in the flat list
- **THEN** the sidebar switches to the directory-grouped view; **and WHEN** the user presses `D` again, it returns to the flat list

#### Scenario: Toggle is documented
- **WHEN** the user opens the help overlay
- **THEN** the `D` directory-grouping toggle is listed

### Requirement: Directory lines are informative only
In the grouped view, a directory line SHALL NOT be selectable and SHALL NOT carry
a jump digit; selection, file-step navigation, and the `1–9` jump digits SHALL
skip them. The active-file selection and its position SHALL be unaffected by
whether the sidebar is grouped.

(A folded directory's *placeholder* is a distinct, selectable row — see
`directory-collapse`. This requirement is about the directory header line.)

#### Scenario: Navigation skips directory lines
- **WHEN** the user steps the selection with `j`/`k` (or `{`/`}`) in the grouped view
- **THEN** the selection moves between selectable rows; directory header lines are never selected

#### Scenario: Jump digits skip directory lines
- **WHEN** the grouped sidebar shows directory lines interleaved with files
- **THEN** the `1–9` jump digits are spread across the selectable rows only, and pressing a digit jumps to that row

#### Scenario: Grouping does not move the cursor
- **WHEN** the user toggles grouping while a file is selected
- **THEN** the same file stays selected and the diff stream position is unchanged

### Requirement: Clicking a directory line selects no file
In the grouped view, a mouse click on a directory line SHALL NOT change the
selected file; a click on a file row SHALL select that file.

#### Scenario: Click a directory line
- **WHEN** the user clicks a directory line
- **THEN** the selected file does not change

#### Scenario: Click a file row
- **WHEN** the user clicks a file row in the grouped view
- **THEN** that file becomes selected
