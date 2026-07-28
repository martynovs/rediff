# commit navigation

## Purpose
Moving between what is under review: the in-TUI commit picker and its filters, the file-scoped log,
and the browser-style view history that makes stepping into a commit and back a navigation rather
than a restart.

## Requirements
### Requirement: In-TUI commit picker
The system SHALL provide a commit picker overlay, opened with `c`, that lists recent commits (capped at a fixed limit) with number shortcuts, mirroring the fuzzy file palette. Selecting a commit SHALL switch the current view to that commit's changes (the diff between the commit and its parent).

#### Scenario: Open and pick a commit
- **WHEN** the user presses `c` and selects a commit from the list
- **THEN** the view switches to that commit's diff and the status line reflects the new source

#### Scenario: Number shortcut picks a commit
- **WHEN** the picker is open and the user presses a digit shown next to a result
- **THEN** that commit is selected and the picker closes

#### Scenario: Dismiss without switching
- **WHEN** the picker is open and the user presses Esc
- **THEN** the picker closes and the current view is unchanged

### Requirement: Smart picker filter
The commit picker SHALL interpret the typed query: a hexadecimal prefix filters by commit SHA; a query that matches a known repository path scopes the list to commits that touched that path; any other query fuzzy-matches the commit summary. The picker SHALL indicate which interpretation is active.

#### Scenario: Filter by SHA prefix
- **WHEN** the user types a hexadecimal prefix
- **THEN** only commits whose SHA starts with that prefix are listed

#### Scenario: Filter by summary text
- **WHEN** the user types a word that is not a SHA or a known path
- **THEN** commits are filtered by fuzzy subsequence match on their summaries

#### Scenario: Filter by path
- **WHEN** the user types a string that matches a known repository path
- **THEN** the list is scoped to commits that changed that path

### Requirement: File-scoped commit log
The system SHALL open the commit picker scoped to the selected file's history with `F`, from any focus, listing the commits that changed that file. Selecting a commit SHALL switch to that commit's changes with the selection landing on that file when present.

#### Scenario: Open a file's history
- **WHEN** the user presses `F` with a file selected
- **THEN** the picker lists only commits that changed that file

#### Scenario: Land on the file after picking
- **WHEN** the user picks a commit from the file-scoped list
- **THEN** the view switches to that commit and the selection is on that file if the commit changed it

### Requirement: Exclude the reviewed range's commits from pickers
While the current view is a range review, the commit selection dialogs SHALL exclude the commits that belong to the reviewed range (the `base..target` set), so the picker offers only commits outside the range under review.

#### Scenario: Range commits hidden in the picker
- **WHEN** the user opens the commit picker (`c`) while reviewing a range
- **THEN** none of the commits in that range are listed

#### Scenario: File-scoped picker excludes range commits
- **WHEN** the user opens the file-scoped picker (`F`) while reviewing a range
- **THEN** commits in the reviewed range are omitted even if they changed that file

#### Scenario: No exclusion outside a range review
- **WHEN** the picker is opened while not in a range review
- **THEN** no range-based exclusion is applied

### Requirement: Browser-style view history
The system SHALL maintain a stack of visited views with a cursor. `{` SHALL move back and `}` SHALL move forward through the stack, each view restoring its own scroll position and selection. Opening a new view while not at the end of the stack SHALL truncate the forward history.

#### Scenario: Back and forward restore position
- **WHEN** the user switches from view A to view B and then presses `{`
- **THEN** view A is shown again with the scroll position and selection it had

#### Scenario: Forward after going back
- **WHEN** the user has pressed `{` to go back and then presses `}`
- **THEN** the next view in the stack is shown again

#### Scenario: New view truncates forward history
- **WHEN** the user has gone back and then opens a different commit
- **THEN** the previously forward views are discarded and the new commit becomes the end of the stack

### Requirement: Return to the home view
The system SHALL return to the launch ("home") view with `C` when the home view is local or staged changes or a review. When rediff was launched on a commit, `C` SHALL be inert.

#### Scenario: Jump home from a browsed commit
- **WHEN** the session was launched on working-tree changes and the user has browsed into a commit
- **THEN** pressing `C` returns to the working-tree view

#### Scenario: Home key inert when launched on a commit
- **WHEN** the session was launched directly on a commit (no local home view)
- **THEN** pressing `C` does nothing

### Requirement: Source color coding
The system SHALL color-code the current view's source in the status line and the sidebar file markers: blue for local or staged changes and green for a commit or range.

#### Scenario: Local view is blue
- **WHEN** the current view is working-tree or staged changes
- **THEN** the status-line source and sidebar file markers use the local (blue) accent

#### Scenario: Commit view is green
- **WHEN** the current view is a commit or range
- **THEN** the status-line source and sidebar file markers use the commit (green) accent

### Requirement: Highlight reset on view switch
On switching the current view, the system SHALL discard cached highlight results from the previous view so that a late asynchronous result cannot paint the new view, and SHALL re-highlight the visible files of the new view without blocking the UI.

#### Scenario: Stale highlight does not leak across a switch
- **WHEN** the view is switched while a highlight request from the previous view is still in flight
- **THEN** the late result is discarded and the new view's files are highlighted instead

#### Scenario: UI stays responsive during re-highlight
- **WHEN** a view switch triggers re-highlighting
- **THEN** the new diff is shown immediately and colors arrive asynchronously

### Requirement: Read a commit's message from the picker
The commit picker SHALL let the user read the full message of the highlighted commit before picking it: pressing `Tab` SHALL open the shared commit-message popup for the highlighted commit. The picker's existing `Enter` and number-shortcut selection SHALL continue to switch to the commit directly without first opening the popup.

#### Scenario: Tab opens the highlighted commit's message
- **WHEN** the commit picker is open and the user presses `Tab` with a commit highlighted
- **THEN** the commit-message popup opens for that commit, over the picker

#### Scenario: Confirm from the popup picks the commit
- **WHEN** the commit-message popup is open from the picker and the user confirms it
- **THEN** the view switches to that commit's diff

#### Scenario: Dismiss returns to the picker
- **WHEN** the commit-message popup is open from the picker and the user dismisses it
- **THEN** the picker is shown again with the same query, results, and highlight

#### Scenario: Enter still picks directly
- **WHEN** the picker is open and the user presses `Enter` (or a number shortcut)
- **THEN** the view switches to that commit directly, without opening the message popup

