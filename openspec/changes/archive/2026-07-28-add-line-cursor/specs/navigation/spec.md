## ADDED Requirements

### Requirement: Incremental motion is cursor-relative
The system SHALL treat the incremental motion commands as moving the cursor, with the viewport
following only as far as needed. This SHALL NOT change the jump commands, which continue to place
their target at the top of the viewport.

#### Scenario: Incremental motion within the viewport does not scroll
- **WHEN** the user presses an incremental motion key and the target row is already visible
- **THEN** the cursor moves and the viewport does not

#### Scenario: Jumps are unaffected
- **WHEN** the user jumps to a file
- **THEN** that file's header is at the top of the viewport, as before
