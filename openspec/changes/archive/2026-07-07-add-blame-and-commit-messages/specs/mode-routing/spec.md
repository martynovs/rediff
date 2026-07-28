## ADDED Requirements

### Requirement: Commit-message popup is a routed overlay
The commit-message popup SHALL be a transient overlay in the overlay stack, layered over a retained base. While it is open it SHALL be the active context: keyboard input SHALL route to it and mouse events SHALL NOT leak through to the context beneath it. It SHALL be opened over a retained base (the commit picker overlay or the blame peek) and dismissing it SHALL return to that exact base. Only the topmost overlay is drawn, so opening the popup covers — never visually stacks on — the picker it was summoned from.

#### Scenario: Keyboard routes to the popup
- **WHEN** the commit-message popup is open and the user presses a key
- **THEN** the key is interpreted by the popup (scroll, confirm, dismiss), not by the base beneath it

#### Scenario: Mouse does not leak through the popup
- **WHEN** the commit-message popup is open and the user scrolls the wheel or clicks
- **THEN** the event is handled by or absorbed for the popup and does not scroll or select in the context behind it

#### Scenario: Dismiss returns to the summoning base
- **WHEN** the popup was opened over the commit picker and the user dismisses it
- **THEN** the picker is the active context again, in the same state; **and WHEN** it was opened over the blame peek, the blame peek is restored instead

#### Scenario: Popup replaces rather than stacks
- **WHEN** the popup opens over the commit picker
- **THEN** only the popup is the active, input-receiving overlay and the picker does not also receive input

### Requirement: Status line reflects the commit-message popup
While the commit-message popup is open, the status line SHALL show the popup's own bindings (confirm to switch, dismiss to return, and scroll if applicable) rather than the bindings of the base beneath it.

#### Scenario: Popup shows its own hints
- **WHEN** the commit-message popup is open
- **THEN** the status line advertises the popup's keys, not the picker's or the blame peek's
