## ADDED Requirements

### Requirement: Committed-rev file blame
The system SHALL compute, for the selected file, a per-line attribution to the commit that last modified each line within committed history, evaluated at the current view's committed rev (HEAD for a local or staged view, the viewed commit for a commit view, the target for a range view). The blame SHALL consider committed content only; working-tree modifications SHALL NOT be attributed, so every blamed line resolves to a real commit and there is no "not committed yet" sentinel.

#### Scenario: Blame at HEAD from a local view
- **WHEN** the user blames a file while the current view is working-tree or staged changes
- **THEN** every line is attributed to the commit that last touched it as of HEAD

#### Scenario: Blame at the viewed commit
- **WHEN** the user blames a file while viewing a specific commit
- **THEN** the attribution is computed against that commit's content, not the working copy

#### Scenario: Every line is attributed
- **WHEN** blame is shown for a file
- **THEN** no line is left unattributed and no working-tree placeholder appears

### Requirement: Blame runs off the UI thread
Computing blame SHALL NOT block the event loop. The blame SHALL be computed on a background worker, reusing the streaming-load and progress-chrome pattern, so the UI stays responsive while it runs and progress chrome appears only once the work passes the established delay threshold.

#### Scenario: Input stays live during a long blame
- **WHEN** blame of a large file with deep history is in progress
- **THEN** the user can still scroll and issue keys without the UI freezing

#### Scenario: Progress chrome only for slow blame
- **WHEN** a blame computation runs past the progress-delay threshold
- **THEN** progress chrome is shown; a fast blame completes without flashing any indicator

### Requirement: Blame attribution gutter
When a file is shown in blame mode, the system SHALL render, in place of the line-number gutter, a fixed 12-column attribution gutter followed by a vertical rule and then the code. Contiguous runs of lines sharing the same commit SHALL be collapsed so that only the first line of a run prints its attribution and continuation lines render a blank gutter. Each attribution token SHALL show the author name left-justified and the relative age right-justified with at least one space between them, the name claiming the remaining columns (`12 − 1 − age_width`). The token SHALL be painted with a stable per-commit color so that one run reads as a single colored block. The commit SHA SHALL NOT appear in the gutter.

#### Scenario: Runs are collapsed
- **WHEN** several consecutive lines were last changed by the same commit
- **THEN** only the first line shows the name and age and the rest show a blank gutter

#### Scenario: Adjacent runs are visually distinct
- **WHEN** two consecutive runs belong to different commits
- **THEN** each run's token is painted in that commit's stable color so the boundary is visible

#### Scenario: Vertical rule stays aligned
- **WHEN** ages of differing widths appear on different lines
- **THEN** the age is right-justified so the vertical rule separating the gutter from the code stays in the same column

### Requirement: Compact relative age format
The relative age in the blame gutter SHALL use the unit ladder hours → days → months → years, rolling from 12 months to 1 year. Hours and days SHALL always be rendered as integers. Months and years SHALL render with one decimal place only while the integer part is a single digit (1–9) and as integers once the integer part reaches 10.

#### Scenario: Hours and days are integers
- **WHEN** a line's age is 2 hours, 23 hours, 1 day, or 29 days
- **THEN** it renders as `2h`, `23h`, `1d`, `29d` with no decimal

#### Scenario: Single-digit months and years carry one decimal
- **WHEN** a line's age is 2.5 months or 1.3 years
- **THEN** it renders as `2.5m` and `1.3y`

#### Scenario: Two-digit months and years drop the decimal
- **WHEN** a line's age is 11 months or 12 years
- **THEN** it renders as `11m` and `12y`

#### Scenario: Twelve months rolls to one year
- **WHEN** a line's age reaches 12 months
- **THEN** it renders in years (e.g. `1.0y`), not as `12m`

### Requirement: Cursor line identity in the header
Because the gutter omits the SHA and blanks continuation lines, the blame view SHALL show the full identity of the line under the cursor — at least its commit SHA and summary — in the peek header, updating as the cursor moves.

#### Scenario: Header names the cursor line's commit
- **WHEN** the cursor is on any blame line, including a collapsed continuation line
- **THEN** the header shows that line's commit SHA and summary

### Requirement: Open a blame line's commit message
In blame mode, pressing `Enter` on the cursor's line SHALL open the shared commit-message popup for that line's commit. It SHALL NOT switch the view directly; switching happens only via the popup's own confirm.

#### Scenario: Enter opens the message, not the diff
- **WHEN** the user presses `Enter` on a blame line
- **THEN** the commit-message popup for that line's commit opens over the blame view, and the view has not changed

#### Scenario: Confirm from the popup switches the view
- **WHEN** the commit-message popup is open from a blame line and the user confirms it
- **THEN** the view switches to that commit's diff
