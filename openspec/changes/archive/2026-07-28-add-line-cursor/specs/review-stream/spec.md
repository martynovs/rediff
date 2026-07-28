## MODIFIED Requirements

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
