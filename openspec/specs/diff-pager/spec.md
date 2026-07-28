# diff-pager Specification

## Purpose
TBD - created by archiving change add-pager-mode. Update Purpose after archive.
## Requirements
### Requirement: Non-interactive pager subcommand
The system SHALL provide a `pager` subcommand that reads a unified diff from standard input, renders it, writes the result to standard output, and exits without launching the interactive TUI.

#### Scenario: Reads stdin, writes stdout, no TUI
- **WHEN** `rediff pager` is invoked with a unified diff piped to stdin
- **THEN** the rendered diff is written to stdout and the process exits without entering the alternate screen or reading keyboard input

#### Scenario: Empty input
- **WHEN** `rediff pager` receives empty stdin (no diff)
- **THEN** it writes nothing (or only a trailing newline) to stdout and exits successfully without error

### Requirement: Forced color when output is piped
The system SHALL emit ANSI color when running as a pager even though stdout is not a terminal, because git and lazygit capture pager output through a pipe.

#### Scenario: Piped stdout still colored
- **WHEN** `rediff pager` runs with its stdout connected to a pipe rather than a TTY
- **THEN** the output contains ANSI color escape sequences rather than plain uncolored text

### Requirement: Themed, syntax-highlighted rendering
The system SHALL render diff content using the same theme resolution and syntax-highlighting engines as the interactive viewer, so the pager output matches the TUI's visual language. Added, removed, and context lines SHALL be visually distinguished, and code SHALL be syntax-highlighted from the active theme's colors.

#### Scenario: Colors come from the active theme
- **WHEN** a diff is rendered under a given theme
- **THEN** added/removed/context styling and code token colors are sourced from that theme, consistent with how the TUI renders the same content

#### Scenario: Theme selection honored
- **WHEN** a theme is selected via the `--theme` flag or configuration
- **THEN** the pager renders in that theme

### Requirement: Git-aware diff parsing
The system SHALL parse the unified diff as produced by `git diff`, tolerating git extended headers (`diff --git`, `index`, mode lines) and handling multiple files in a single input.

#### Scenario: Multi-file diff
- **WHEN** the input contains diffs for several files
- **THEN** each file's diff is rendered in turn

#### Scenario: Extended headers do not break parsing
- **WHEN** the input includes git extended headers such as `diff --git` and `index` lines
- **THEN** parsing succeeds and the hunks are rendered without error

### Requirement: File operation and binary handling
The system SHALL recognize file-level operations the diff encodes — create, delete, modify, rename, and copy — and SHALL detect binary files and present them as a notice instead of attempting to render binary content as text.

#### Scenario: Renamed file
- **WHEN** a file's diff carries git rename headers
- **THEN** the render indicates the rename (old path to new path) rather than treating it as an unrelated create/delete

#### Scenario: Binary file
- **WHEN** the input marks a file as binary (e.g. `Binary files ... differ`)
- **THEN** the render shows a binary-file notice and does not emit garbled text or fail

### Requirement: Staging-preserving pager integration
The pager SHALL operate purely as a post-processor of git's diff output, computing no diff of its own, so that the underlying unified patch remains intact for line and hunk staging in tools such as lazygit.

#### Scenario: Underlying patch unchanged
- **WHEN** `rediff pager` is used as a git/lazygit pager
- **THEN** it only transforms the displayed text and does not replace git's diff, leaving line/hunk staging functional

### Requirement: The review log is never rendered
The pager and the per-file external-diff renderer SHALL produce no output for the per-worktree review
log (`rediff.jsonl` at a repository root). Unlike the loader, these renderers post-process git's own
output and never open a repository, so git may hand them the log directly — which the documented
`GIT_EXTERNAL_DIFF` integration does for untracked files. The omission SHALL NOT be configurable, and
SHALL apply only to the root-level file, so a similarly named file elsewhere renders normally.

#### Scenario: The pager drops the log's section
- **WHEN** a unified diff containing a section for the worktree-root `rediff.jsonl` is rendered by the pager
- **THEN** that section produces no output

#### Scenario: Other files in the same diff are unaffected
- **WHEN** a unified diff contains both the review log's section and another file's
- **THEN** the other file renders normally and the log does not appear

#### Scenario: The external renderer skips the log
- **WHEN** git invokes the per-file external-diff renderer for the worktree-root `rediff.jsonl`
- **THEN** nothing is rendered for it

#### Scenario: Similarly named files still render
- **WHEN** the path is `rediff.jsonl` in a subdirectory, or a file whose name merely contains that string
- **THEN** it renders as an ordinary file
