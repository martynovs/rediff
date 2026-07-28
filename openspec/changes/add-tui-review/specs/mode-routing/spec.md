## MODIFIED Requirements

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

### Requirement: Exactly one overlay is shown
The **topmost** overlay SHALL be the one displayed, selected by the active context, so what is drawn
always matches what is receiving input. Overlays beneath it SHALL NOT be drawn.

#### Scenario: Opening an overlay draws it over the one below
- **WHEN** an overlay is opened while another is already active
- **THEN** only the new overlay is rendered over the body, and it is the overlay receiving input

## ADDED Requirements

### Requirement: Review overlays join the stack
The comment input and the thread list SHALL be overlays in that stack, and editing a thread from the
thread list SHALL push the input over it and return to the list on dismissal.

#### Scenario: Editing from the list returns to the list
- **WHEN** the user edits a thread from the thread list and dismisses the input
- **THEN** the thread list is shown again
