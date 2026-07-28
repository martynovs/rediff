## ADDED Requirements

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
