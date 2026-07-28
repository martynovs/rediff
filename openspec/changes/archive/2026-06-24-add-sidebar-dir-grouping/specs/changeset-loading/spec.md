## ADDED Requirements

### Requirement: Stable path ordering
The system SHALL order a changeset's files by their parent directory and then by
file name, applied once at enumeration so it holds for every load path
(working-tree, staged, commit, and range). Files in the same directory SHALL be
contiguous, and the order SHALL be stable rather than git's enumeration order.
The ordering SHALL be applied before the streaming diff load begins, so it does
not change which file each streamed diff is installed into.

#### Scenario: Files in a directory are contiguous
- **WHEN** a changeset touches several files in one directory and files in its subdirectories
- **THEN** the directory's own files appear consecutively, ordered by name, with subdirectory files grouped under their own directories

#### Scenario: Ordering is stable across load kinds
- **WHEN** the same set of changed files is loaded via a working-tree diff and via a commit
- **THEN** the files appear in the same parent-directory-then-name order in both

#### Scenario: Streaming load stays aligned
- **WHEN** the changeset is ordered at enumeration and the per-file diffs stream in
- **THEN** each diff is installed into its file's position and the file list order does not shift as diffs arrive
