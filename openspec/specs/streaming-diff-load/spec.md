# streaming diff load

## Purpose
Loading a diff without making the user wait for it: the file list appears instantly and the per-file
diffs stream in behind it, with progress shown only when a load is slow enough to warrant it, and
cancellable throughout.

## Requirements
### Requirement: Instant file list
The system SHALL show the list of changed files (the sidebar) before any file's diff has been computed, by enumerating paths and statuses without reading blob contents.

#### Scenario: Sidebar appears immediately for a large change
- **WHEN** rediff opens a change with many files
- **THEN** the sidebar lists every changed file with its status before the diffs are computed

#### Scenario: Navigable while loading
- **WHEN** the file list is shown and diffs are still computing
- **THEN** the user can move the selection through the file list

### Requirement: Background streaming diff
The system SHALL compute each file's diff off the UI thread and populate that file's stats and body as it completes, without blocking input.

#### Scenario: Files fill in progressively
- **WHEN** diffs are being computed in the background
- **THEN** each file's `+/−` stats and diff body become available as that file finishes, while the UI stays responsive

#### Scenario: Stable order
- **WHEN** files are diffed concurrently and complete out of order
- **THEN** the displayed file order is unchanged from the enumeration order

### Requirement: Progress in the diff pane
While diffs are still computing the system SHALL show load progress in the diff pane (not a modal popup), and SHALL render a file's diff normally once that file is done.

#### Scenario: Progress shown for undiffed content
- **WHEN** the user is viewing a file (or region) whose diff has not yet been computed
- **THEN** the diff pane shows progress (e.g. a count of files diffed) rather than blank or a popup

#### Scenario: Completed file renders normally
- **WHEN** a file's diff has finished while others are still computing
- **THEN** that file's diff renders normally

### Requirement: Cancel during load
The system SHALL let the user cancel an in-progress load with Esc or q. At startup this SHALL quit; for an in-session view switch it SHALL return to the previous view.

#### Scenario: Cancel at startup quits
- **WHEN** the initial load is in progress and the user presses Esc or q
- **THEN** rediff stops loading, restores the terminal, and exits

#### Scenario: Cancel a switch returns to the previous view
- **WHEN** a load triggered by switching to another commit/branch is in progress and the user cancels
- **THEN** the load is abandoned and the previous view is shown

### Requirement: No progress chrome for fast loads
The system SHALL NOT show progress chrome for loads that complete quickly (below a small threshold), so small changes appear instantly without a loading indicator.

#### Scenario: Small change shows no progress
- **WHEN** a change is small enough to load below the threshold
- **THEN** no progress indicator is shown and the full diff appears at once

### Requirement: Single async loader for all load sites
Startup, the commit picker, and the working-tree-vs-ref (`--from`) load SHALL all use the same async loader, so none of them blocks the UI thread.

#### Scenario: Picking a large commit does not freeze
- **WHEN** the user picks a large commit in the commit picker
- **THEN** the file list appears and the diff streams in just as at startup, without freezing
