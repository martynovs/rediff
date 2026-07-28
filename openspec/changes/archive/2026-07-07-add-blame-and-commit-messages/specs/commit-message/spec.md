## ADDED Requirements

### Requirement: Shared commit-message popup
The system SHALL provide a single commit-message popup overlay that displays a commit's SHA, author, date, and full message body. The same popup SHALL be reachable from the commit picker and from a blame line. Confirming the popup (`Enter`) SHALL switch the current view to that commit's diff; dismissing it (`Esc`) SHALL return to whatever context was beneath it, unchanged.

#### Scenario: Popup shows the full body
- **WHEN** the commit-message popup opens for a commit
- **THEN** it shows the commit's SHA, author, date, and the full multi-line message body, not just the summary line

#### Scenario: Confirm switches to the commit
- **WHEN** the popup is open and the user presses `Enter`
- **THEN** the current view switches to that commit's diff

#### Scenario: Dismiss returns to the base
- **WHEN** the popup is open and the user presses `Esc`
- **THEN** the popup closes and the underlying context (the picker or the blame view) is shown exactly as before

### Requirement: Message body fetched by SHA
The popup SHALL obtain the full message body by commit SHA when it opens, so that it works identically whether the SHA came from the commit picker's enumerated list or from a blame line, without the enumerated commit list having to carry every body.

#### Scenario: Body available from a blame line
- **WHEN** the popup is opened from a blame line whose commit was never listed in a picker
- **THEN** that commit's full body is fetched by its SHA and shown

#### Scenario: Long body scrolls
- **WHEN** the commit message is longer than the popup's height
- **THEN** the body can be scrolled within the popup

### Requirement: Commit-message banner before a commit's diff
When the current view is a single commit, the system SHALL render that commit's message before its diff as scroll-away content at the top of the stream (not fixed chrome), so the message scrolls out of view as the user reads into the diff and arbitrarily long messages do not consume permanent space.

#### Scenario: Banner precedes the first file
- **WHEN** the user views a single commit's diff
- **THEN** the commit's message is shown above the first changed file

#### Scenario: Banner scrolls away
- **WHEN** the user scrolls down into the diff of a commit view
- **THEN** the message banner scrolls out of view rather than remaining pinned

#### Scenario: No banner outside commit views
- **WHEN** the current view is working-tree, staged, or range changes
- **THEN** no commit-message banner is shown
