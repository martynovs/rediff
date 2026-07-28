# changeset loading

## Requirements

### Requirement: Load working-tree changes
The system SHALL load the git working-tree changes for `rediff diff` into one normalized
changeset of files and hunks, including staged and unstaged modifications and untracked files.

#### Scenario: Mixed working-tree changes
- **WHEN** the repository has a staged modification, an unstaged modification, and an untracked file
- **THEN** the changeset contains one file entry for each, with hunks computed from the appropriate old/new content

#### Scenario: Untracked files included by default
- **WHEN** `rediff diff` runs and untracked files are present
- **THEN** those untracked files appear in the changeset (matching hunk's default, unlike `git diff`)

#### Scenario: Exclude untracked on request
- **WHEN** `rediff diff --exclude-untracked` runs
- **THEN** untracked files are omitted and only tracked changes appear

### Requirement: Load staged-only changes
The system SHALL load only staged changes (HEAD vs index) for `rediff diff --staged`.

#### Scenario: Staged diff
- **WHEN** `rediff diff --staged` runs with a staged modification and an unstaged modification to the same file
- **THEN** the changeset reflects only the staged (HEAD-to-index) difference

### Requirement: Load a commit or range
The system SHALL load the changes introduced by a commit for `rediff show [ref]` (defaulting to
HEAD) and by a range expression.

#### Scenario: Show a commit
- **WHEN** `rediff show HEAD` runs
- **THEN** the changeset contains the diff between HEAD's parent tree and HEAD's tree

#### Scenario: Show an earlier commit
- **WHEN** `rediff show <ref>` runs for a non-HEAD ref
- **THEN** the changeset reflects that commit's changes

### Requirement: Decode renames
The system SHALL detect renames and copies and render them with their source and destination
paths rather than as an unrelated add and delete.

#### Scenario: Rename with edits
- **WHEN** a file is renamed and modified
- **THEN** the file entry shows the source path, the destination path, a "renamed" indication, and the content diff between the two sides

#### Scenario: Pure rename
- **WHEN** a file is renamed with no content change
- **THEN** the file entry shows the rename with source and destination paths and an empty body

### Requirement: Produce git-faithful diffs
Hunk bodies and hunk headers produced by the system SHALL match the content that git produces
for the same change.

#### Scenario: Body matches git
- **WHEN** a file is modified and loaded into the changeset
- **THEN** the added/removed lines and hunk header ranges match `git diff` for that file

### Requirement: Two-stage loading
The system SHALL be able to enumerate a changeset's files (paths, status, rename source) without computing their diffs, and to compute a single file's diff (hunks and stats) separately. A file MAY exist in the changeset before it has been diffed.

#### Scenario: Enumerate without diffing
- **WHEN** a changeset is enumerated
- **THEN** every changed file's path and status are available with no blob contents read and no hunks computed

#### Scenario: Diff one file
- **WHEN** a single enumerated file is diffed
- **THEN** that file's hunks and `+/−` stats are produced from its two sides

### Requirement: Streaming load with progress and cancel
The loader SHALL run per-file diffs in the background, report progress (files completed of total), deliver each completed file to the caller, and stop promptly when cancellation is requested.

#### Scenario: Progress reported
- **WHEN** files are being diffed in the background
- **THEN** the caller can observe how many of the total files have completed

#### Scenario: Cancellation stops work
- **WHEN** cancellation is requested mid-load
- **THEN** the loader stops diffing further files and releases its workers

### Requirement: Enumerate commits
The system SHALL enumerate commits reachable from a tip (defaulting to HEAD) up to a fixed cap, exposing for each commit its short SHA, summary, author, and time, for use by the commit picker.

#### Scenario: Recent commits available
- **WHEN** the commit picker requests the commit list
- **THEN** commits reachable from HEAD are returned, newest first, up to the cap

#### Scenario: List is capped
- **WHEN** the repository has more commits than the cap
- **THEN** at most the cap number of commits are returned and the truncation is indicated

### Requirement: File-scoped commit history
The system SHALL determine, for a given path, which of the enumerated commits changed that path, by comparing the path's blob between each commit's tree and its parent's tree.

#### Scenario: Commits that touched a path
- **WHEN** the file-scoped history for a path is requested
- **THEN** only commits whose tree differs from their parent at that path are returned

#### Scenario: Path absent from a commit
- **WHEN** a commit neither contains nor removes the path relative to its parent
- **THEN** that commit is omitted from the file-scoped history

### Requirement: Review a commit or range
The system SHALL load a review changeset for `rediff review [sha] [--from <base>]`. With no `sha`, the target SHALL be HEAD. Without `--from`, the changeset SHALL be the single commit's diff (target vs its parent). With `--from <base>`, the changeset SHALL be the combined net diff between the merge-base of `base` and the target and the target's tree.

#### Scenario: Review the latest commit
- **WHEN** `rediff review` runs with no arguments
- **THEN** the changeset contains the diff that HEAD introduced over its parent

#### Scenario: Review a specific commit
- **WHEN** `rediff review <sha>` runs
- **THEN** the changeset contains the diff that commit introduced over its parent

#### Scenario: Review a branch range as a net diff
- **WHEN** `rediff review <sha> --from <base>` runs
- **THEN** the changeset is the combined net diff between the merge-base of `base` and the target and the target, as one flat file list

#### Scenario: Base moved ahead after branching
- **WHEN** `--from <base>` is given and `base` has commits not present in the target
- **THEN** the net diff is computed against the merge-base so only the target's own changes appear

### Requirement: Load a single file for the peek
The system SHALL load the content and diffs that the single-file peek needs: a file's full text at a given revision (for content mode), the working-copy text of a file, and a single-file unified diff computed at an arbitrary context level.

#### Scenario: File content at a revision
- **WHEN** the peek requests a file's content at a commit
- **THEN** the file's blob text at that commit is returned for full-content rendering

#### Scenario: Single-file diff at a context level
- **WHEN** the peek requests a file's diff at a given context level
- **THEN** the unified diff for that file is computed with that many surrounding context lines

#### Scenario: Missing or binary side
- **WHEN** the file is absent at the requested revision or is binary
- **THEN** the peek is told there is no content/diff to show rather than rendering garbage

### Requirement: Resolve the review top
The system SHALL resolve `TOP`, the newest side of the current review context, so the peek's history diff compares against it: the working copy for a working-tree or staged review, the target commit for a range review, and the commit itself for a single-commit view.

#### Scenario: Working-tree review top
- **WHEN** the active view is a working-tree review and the peek needs `TOP`
- **THEN** `TOP` resolves to the working copy

#### Scenario: Range review top
- **WHEN** the active view is a range review `base..target`
- **THEN** `TOP` resolves to the target commit

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
