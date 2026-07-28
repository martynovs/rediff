# mode-routing Specification

## Purpose
A single active input context — a base (normal stream/sidebar or the file peek) with at most one transient overlay (the fuzzy palette or help) — that deterministically drives keyboard routing, mouse routing, the status line's hints and context, and which overlay is shown, all from one definition so the input paths, status, and help cannot drift.

## Requirements
### Requirement: Single active input mode
The system SHALL track a single active input context that determines how keyboard and mouse input is
routed. That context is a base (the normal stream — focused on the diff or the sidebar — or the file
peek) with a **stack** of transient overlays layered on top. The topmost overlay is the active
context; with no overlays, the base is. Keyboard and mouse routing SHALL derive from this same
context, with one precedence, so the two input paths never disagree about which context is active.

Dismissing the topmost overlay SHALL reveal the one beneath it, exactly as it was, and dismissing the
last SHALL restore the base unchanged. An overlay MAY be opened from another overlay.

#### Scenario: Keyboard routes to the active overlay
- **WHEN** an overlay is open and the user presses a key
- **THEN** the key is interpreted by that overlay's bindings, not the base's

#### Scenario: Mouse does not leak through an overlay
- **WHEN** an overlay is open and the user scrolls the wheel or clicks
- **THEN** the event is handled by (or absorbed for) the active overlay and does not scroll or select within the diff behind it

#### Scenario: An overlay opened from an overlay returns to it
- **WHEN** an overlay is opened while another is shown, and then dismissed
- **THEN** the overlay beneath is shown again, exactly as it was

#### Scenario: Dismissing the last overlay restores the base
- **WHEN** the only overlay is dismissed
- **THEN** the base view is shown exactly as it was

### Requirement: Overlays layer over a retained base
An overlay (the palette or help) SHALL be opened over a base context that is retained while the overlay is active, so the overlay's mode-dependent content reflects that base and closing the overlay returns to it. The help overlay in particular SHALL present the bindings of the base it was opened over.

#### Scenario: Help reflects the base beneath it
- **WHEN** the user opens help while the file peek is the active base
- **THEN** the help lists the peek's bindings; **and WHEN** the user opens help while the normal stream is the active base, the help lists the stream's bindings

#### Scenario: Closing an overlay returns to its base
- **WHEN** the user opens an overlay over a base and then dismisses the overlay
- **THEN** the active context is the same base it was opened over, in the same state, not a default or reset context

### Requirement: Status line reflects the active mode
The status line SHALL show hints and context for the active mode. While the file peek is open it SHALL show the peek's context (the peeked file and the peek's own position) and the peek's bindings, not the underlying stream's; the same SHALL hold for other overlay modes.

#### Scenario: Peek shows peek status
- **WHEN** the file peek is open
- **THEN** the status line describes the peek (its file and scroll position) and shows keys that act in the peek, not the stream's file count, scroll position, or stream keys

#### Scenario: Returning to the stream restores stream status
- **WHEN** the user closes the peek
- **THEN** the status line again shows the stream's file count, position, and stream bindings

### Requirement: Status percentage tracks the active layout
The scroll-position percentage shown in the status line SHALL be computed against the currently displayed layout's row count.

#### Scenario: Percentage correct in split layout
- **WHEN** the user is in the side-by-side (split) layout and scrolls
- **THEN** the status percentage reflects position within the split layout's rows, not the stacked layout's

### Requirement: One keymap definition drives behavior, hints, and help
The keybindings SHALL be defined in one place, and the status-line hints and the help overlay SHALL be derived from that definition rather than maintained as independent copies, so they cannot drift from the bindings that are actually in effect.

#### Scenario: Help matches actual bindings
- **WHEN** the user opens the help overlay
- **THEN** the keys it lists are the keys the active routing actually handles

### Requirement: Exactly one overlay is shown
The **topmost** overlay SHALL be the one displayed, selected by the active context, so what is drawn
always matches what is receiving input. Overlays beneath it SHALL NOT be drawn.

#### Scenario: Opening an overlay draws it over the one below
- **WHEN** an overlay is opened while another is already active
- **THEN** only the new overlay is rendered over the body, and it is the overlay receiving input

### Requirement: Review overlays join the stack
The comment input, the thread list, and the round-closing overlay SHALL be overlays in that stack,
and editing a thread from the thread list SHALL push the input over it and return to the list on
dismissal.

#### Scenario: Editing from the list returns to the list
- **WHEN** the user edits a thread from the thread list and dismisses the input
- **THEN** the thread list is shown again

#### Scenario: A multi-stage overlay steps back before closing
- **WHEN** the user dismisses an overlay that has advanced past its first stage
- **THEN** it returns to the previous stage rather than closing, so a mistyped dismissal costs a step and not the work in progress

### Requirement: The help overlay scrolls when it does not fit
The help overlay SHALL let the user scroll its content when the terminal cannot show all of it, and
SHALL say so. Any key that is not a scroll key SHALL still dismiss it.

#### Scenario: Content beyond the box is reachable
- **WHEN** the help catalog is taller than the terminal allows
- **THEN** the user can scroll to the content below the fold, and the overlay says that scrolling is available

#### Scenario: Any other key still closes it
- **WHEN** the user presses a key that is not a scroll key while help is open
- **THEN** the overlay closes, as it does when the whole catalog fits
