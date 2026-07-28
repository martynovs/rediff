# line cursor

## Purpose

A cursor naming one row of the review stream, so that actions can address a
specific line rather than a whole file. Incremental motion moves it and the
viewport follows; it survives the plan rebuilds that layout toggles, folds,
reviewed toggles and streaming diffs cause constantly.

## Requirements

### Requirement: A line cursor names a row in the review stream
The system SHALL maintain a line cursor identifying one row of the review stream, and SHALL render
that row distinctly from the rest so the user can see where they are. This SHALL hold identically in
unified and side-by-side layouts.

#### Scenario: The line cursor is visible
- **WHEN** the review stream is shown with at least one row
- **THEN** exactly one visible row is styled as current

#### Scenario: The line cursor is visible in either layout
- **WHEN** the diff is shown unified, and again when it is shown side by side
- **THEN** exactly one visible row is styled as current in both

#### Scenario: The line cursor starts at the top
- **WHEN** a view is first shown
- **THEN** the line cursor is on the first row

### Requirement: Incremental motion moves the line cursor and the viewport follows
The system SHALL move the line cursor in response to incremental motion commands, and SHALL scroll
the viewport only as far as needed to keep it visible. Commands that *jump* to a position — a file, a
hunk, a folded directory's placeholder, the top, the bottom — SHALL continue to place that position
at the top of the viewport as they do today, and SHALL additionally place the line cursor there.

Visibility SHALL be computed against the height actually available for rows rather than the full
viewport, since a sticky file header occupies one line of it. The available height SHALL NOT be
over-estimated, so that the guarantee holds whether or not the header is currently shown. Where no
row can be drawn at all, the viewport SHALL be left alone rather than moved to satisfy a guarantee
that cannot be met.

#### Scenario: Moving within the viewport does not scroll
- **WHEN** the line cursor moves to a row already on screen
- **THEN** the viewport does not move

#### Scenario: Moving past the edge scrolls just enough
- **WHEN** the line cursor moves beyond the last visible row
- **THEN** the viewport scrolls so it is visible at the edge, not recentred

#### Scenario: The line cursor stops at the ends
- **WHEN** the line cursor is on the first row and the user moves up, or on the last row and moves down
- **THEN** it stays where it is and the viewport does not move

#### Scenario: Jumping keeps top-alignment and places the line cursor
- **WHEN** the user jumps to a file or a hunk
- **THEN** that position is at the top of the viewport, as before, and the line cursor is on it

#### Scenario: Jumping to a folded directory's placeholder places the line cursor
- **WHEN** a fold lands the viewport on a collapsed directory's placeholder
- **THEN** the line cursor is on that placeholder, so the next motion continues from it rather than snapping the viewport back

#### Scenario: The line cursor stays visible under a sticky header
- **WHEN** a sticky file header is shown and the line cursor moves to the last available row
- **THEN** it is still visible, the header having been accounted for

#### Scenario: The last row of the stream is reachable
- **WHEN** the user jumps to the bottom of the stream and the stream is then redrawn
- **THEN** the line cursor is still on the last row and that row is within the drawn rows

#### Scenario: A viewport too small to draw any row moves nothing
- **WHEN** the available height leaves no room for a body row
- **THEN** the viewport is not moved

#### Scenario: Hunk stepping is relative to the line cursor
- **WHEN** the user steps to the next hunk
- **THEN** it is the next hunk after the line cursor, never one behind it

### Requirement: The selected file follows incremental motion of the line cursor
The system SHALL, when incremental motion moves the line cursor into a different file's rows, derive
the selected file from the line cursor's row rather than from whichever file appears at the top of
the viewport.

This SHALL NOT change which file the file-scoped actions act on, nor the rule that they are inert
while a collapsed directory's placeholder is selected.

#### Scenario: Sidebar agrees with the line cursor
- **WHEN** incremental motion moves the line cursor into a different file's rows
- **THEN** the sidebar highlights that file

#### Scenario: A fold's placeholder selection is preserved
- **WHEN** a directory is folded and its placeholder becomes the selection
- **THEN** the selection stays on the placeholder and the file-scoped actions remain inert

### Requirement: The line cursor survives a plan rebuild
The system SHALL keep the line cursor on the same content when the row plan is rebuilt — by a layout
change, a directory fold, a reviewed toggle, or a streamed diff arriving — rather than on a row index
whose meaning has changed.

Content SHALL be identified by file, side, and line number together, since a removed line and an
added line may share a file and a line number. A row SHALL report every identity it carries, so that
a context line — which exists on both sides — is found from either. A row that carries no such
identity, including a spacer, a hunk gap, a banner, a placeholder, and a binary-file note, SHALL be
kept at the same position **within its own file**, so that another file changing size does not move
it. When the identified content is no longer present, the line cursor SHALL move to the nearest
surviving row — for content hidden by folding a directory, that is the directory's placeholder.

After any rebuild the viewport SHALL be adjusted so the line cursor is drawn, since the line cursor
and the viewport may be repaired against different files and can otherwise drift apart.

#### Scenario: A removed and an added line at the same number are distinguished
- **WHEN** the line cursor is on a removed line whose number also exists as an added line, and the plan is rebuilt
- **THEN** it is on the removed line, not the added one

#### Scenario: A row with no identity keeps its place
- **WHEN** the line cursor is on a spacer or hunk gap and a streamed diff rebuilds the plan
- **THEN** it does not jump to an unrelated part of the diff

#### Scenario: Switching to side-by-side keeps the line cursor
- **WHEN** the user switches from unified to side-by-side layout
- **THEN** the line cursor is on the same diff line as before

#### Scenario: Switching to unified keeps the line cursor, including on a context line
- **WHEN** the line cursor is on a side-by-side context row and the user switches to unified layout
- **THEN** it is on that same context line, which carries both sides

#### Scenario: A layout round trip returns to the same change
- **WHEN** the line cursor is on a removed line in unified layout and the user switches to side-by-side and back
- **THEN** it is on that same change, which may be either of the two rows the change occupies in unified layout

#### Scenario: A placeholder stays with its own file while another file loads
- **WHEN** the line cursor is on a not-yet-loaded file's placeholder and an earlier file's diff arrives and grows
- **THEN** the line cursor is still on that file's placeholder, not on a row of the file that grew

#### Scenario: The line cursor is visible after a rebuild
- **WHEN** any rebuild moves the line cursor and the viewport by different amounts
- **THEN** the viewport is adjusted so the line cursor is drawn

#### Scenario: A streamed diff arriving keeps the line cursor
- **WHEN** a background diff lands and rebuilds the plan
- **THEN** the line cursor has not jumped

#### Scenario: Folding away the line cursor's content moves it nearby
- **WHEN** the line cursor's row is hidden, such as by folding the directory it is in
- **THEN** it moves to the nearest remaining row rather than out of range

### Requirement: The line cursor names a change, not a side of one
The system SHALL treat the reviewed unit as the change. Where a side-by-side row holds both a removed
line and the line that replaced it, the line cursor SHALL name that whole row and both of its cells
SHALL be styled as current. No key SHALL be provided to move between them, and no side SHALL be
offered as a user-visible choice in either layout.

When something acts on the line cursor, the identity used SHALL be the new side where the change has
one and the old side otherwise.

#### Scenario: A change is marked as a whole
- **WHEN** the diff is shown side by side and the line cursor is on a row holding a removed line and its replacement
- **THEN** the whole row is marked as current, the row being one change, and neither cell is marked in preference to the other

#### Scenario: Acting on a change anchors to the new side
- **WHEN** something acts on the line cursor while it names a change that has a new side
- **THEN** the identity used is the new side

#### Scenario: Acting on a deletion anchors to the old side
- **WHEN** something acts on the line cursor while it names a change that exists only on the old side
- **THEN** the identity used is the old side
