# review stream

## Requirements

### Requirement: Single multi-file review stream
The system SHALL render all changed files as one continuous top-to-bottom review stream in the
changeset's file order.

#### Scenario: Multiple files shown in order
- **WHEN** a changeset with several files is opened
- **THEN** every file's diff appears in one scrollable stream, in changeset order, each under its own file header

#### Scenario: Selecting a file does not collapse the stream
- **WHEN** a file is selected from the sidebar
- **THEN** the stream scrolls to that file's position and continues to show all other files (it does not reduce to a single-file view)

### Requirement: Windowed rendering
The system SHALL render only the rows visible in the viewport (plus a small overscan) so
performance does not degrade with changeset size.

#### Scenario: Large changeset stays responsive
- **WHEN** a changeset with thousands of diff rows is scrolled
- **THEN** only viewport-visible rows are rendered each frame and scrolling remains smooth

### Requirement: Scrolling
The system SHALL support moving through the review stream by keyboard and mouse wheel.

Incremental keyboard motion SHALL move the line cursor, with the viewport following only as far as
needed to keep it visible. Scroll gestures — the fast-scroll keys and the mouse wheel — SHALL move
the viewport by the corresponding amount and carry the line cursor with them, so the cursor holds its
position on screen and a scroll gesture never leaves it behind.

#### Scenario: Keyboard motion
- **WHEN** the user presses an incremental motion key
- **THEN** the line cursor moves within sub-frame latency, and the viewport moves only as far as needed to keep the cursor visible

#### Scenario: Keyboard scroll
- **WHEN** the user presses a fast-scroll key
- **THEN** the viewport moves by the corresponding amount within sub-frame latency, and the line cursor moves with it

#### Scenario: Mouse-wheel scroll
- **WHEN** the user scrolls the mouse wheel over the stream
- **THEN** the viewport moves accordingly, and the line cursor moves with it

### Requirement: Render not-yet-diffed files
The review stream and sidebar SHALL render files that have not yet been diffed — a placeholder in the stream and placeholder stats in the sidebar — and SHALL replace them with the real diff and stats when each file's computation completes.

#### Scenario: Sidebar placeholder stats
- **WHEN** a file is listed but not yet diffed
- **THEN** the sidebar shows its path and status with placeholder `+/−` stats, which become real numbers once it is diffed

#### Scenario: Stream placeholder replaced
- **WHEN** the user is positioned on a file whose diff has not yet computed
- **THEN** the diff pane shows a placeholder/progress, and the file's diff appears in place once computed

#### Scenario: Navigation tolerates undiffed files
- **WHEN** files are still streaming in
- **THEN** moving between files and the file list does not error on undiffed files
