# layout modes

## Purpose
How a diff is laid out: unified (stacked) or side-by-side (split), chosen automatically from the
terminal's width and overridable explicitly, so a narrow terminal is never handed a layout it cannot
render honestly.

## Requirements
### Requirement: Layout modes
The system SHALL support `auto`, `split`, and `stack` layout modes for displaying diffs.

#### Scenario: Stack layout
- **WHEN** stack mode is active
- **THEN** old and new content are shown in a unified top-to-bottom layout

#### Scenario: Split layout
- **WHEN** split mode is active
- **THEN** old and new content are shown side by side

### Requirement: Responsive auto layout
In `auto` mode the system SHALL choose split on wide terminals and stack on narrow terminals,
and SHALL re-evaluate the choice when the terminal is resized.

#### Scenario: Wide terminal chooses split
- **WHEN** `auto` mode is active and the terminal is wide
- **THEN** the split layout is used

#### Scenario: Narrow terminal chooses stack
- **WHEN** `auto` mode is active and the terminal is narrow
- **THEN** the stack layout is used

#### Scenario: Resize re-evaluates
- **WHEN** the terminal is resized across the width threshold while in `auto`
- **THEN** the layout switches accordingly

### Requirement: Explicit mode overrides auto
An explicitly selected `split` or `stack` mode SHALL override the responsive `auto` choice.

#### Scenario: Explicit split on narrow terminal
- **WHEN** the user explicitly selects split mode on a narrow terminal
- **THEN** the split layout is used regardless of width
