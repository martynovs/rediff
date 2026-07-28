## ADDED Requirements

### Requirement: Reviewed state persists with an open review
The system SHALL record which files the user has marked reviewed **once a review is open**, and SHALL
restore that state when the same review is opened again in a later session. The system SHALL NOT
create a review in order to record reviewed state, so marking files reviewed while merely browsing
a diff writes nothing. Restoration SHALL match files by path rather than by position, since a
changeset's file order may differ between sessions.

#### Scenario: Reviewed files survive a restart
- **WHEN** the user marks files reviewed in a review that is open, leaves, and opens that review again
- **THEN** those files are still marked reviewed

#### Scenario: Marking reviewed does not open a review
- **WHEN** the user marks files reviewed with no review open
- **THEN** no review log is created

#### Scenario: Restoration follows paths
- **WHEN** a review is reopened and its changeset lists the files in a different order
- **THEN** the same files are marked reviewed, not the same positions
