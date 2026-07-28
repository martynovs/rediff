## ADDED Requirements

### Requirement: Read a commit's message from the picker
The commit picker SHALL let the user read the full message of the highlighted commit before picking it: pressing `Tab` SHALL open the shared commit-message popup for the highlighted commit. The picker's existing `Enter` and number-shortcut selection SHALL continue to switch to the commit directly without first opening the popup.

#### Scenario: Tab opens the highlighted commit's message
- **WHEN** the commit picker is open and the user presses `Tab` with a commit highlighted
- **THEN** the commit-message popup opens for that commit, over the picker

#### Scenario: Confirm from the popup picks the commit
- **WHEN** the commit-message popup is open from the picker and the user confirms it
- **THEN** the view switches to that commit's diff

#### Scenario: Dismiss returns to the picker
- **WHEN** the commit-message popup is open from the picker and the user dismisses it
- **THEN** the picker is shown again with the same query, results, and highlight

#### Scenario: Enter still picks directly
- **WHEN** the picker is open and the user presses `Enter` (or a number shortcut)
- **THEN** the view switches to that commit directly, without opening the message popup
