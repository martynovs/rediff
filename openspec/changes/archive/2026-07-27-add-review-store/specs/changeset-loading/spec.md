## ADDED Requirements

### Requirement: The review log is never review material
The system SHALL omit the per-worktree review log (`rediff.jsonl` at the worktree root) from every
changeset it produces, for every source — working-tree, staged, commit, and range — and regardless of
whether the file is ignored by git. The omission SHALL NOT be configurable.

#### Scenario: Log absent from a working-tree changeset
- **WHEN** a working-tree changeset is loaded and an untracked `rediff.jsonl` exists at the worktree root
- **THEN** no file entry for it appears in the changeset, and the other untracked files are unaffected

#### Scenario: Log absent even when committed
- **WHEN** a changeset is loaded for a commit or range in which `rediff.jsonl` was added or modified
- **THEN** no file entry for it appears in the changeset

#### Scenario: Similarly named files are unaffected
- **WHEN** a changeset is loaded and the working tree contains a changed file named `rediff.jsonl` in a subdirectory, or a file whose name merely contains that string
- **THEN** those files appear in the changeset as usual, and only the log at the worktree root is omitted
