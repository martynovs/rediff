//! Shared `#[cfg(test)]` helpers: the git-scratch-repo scaffolding, the blame
//! pump, and the review-store fixtures, kept in one place so an
//! environment-driven fixture fix (a git config, an `init` flag) lands once
//! rather than in every test module's private copy.

#![cfg(test)]

use std::path::Path;

use tempfile::TempDir;

use crate::model::{Changeset, DiffFile, FileStatus};
use crate::review::{now, Log, Record};

/// Run a git command in `dir`, asserting it succeeds (stderr on failure).
pub(crate) fn run_git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A fresh, empty git repository with a review-friendly identity and gpg signing
/// off — the common prelude for every scratch-repo fixture.
pub(crate) fn scratch_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    run_git(dir.path(), &["config", "user.email", "t@t.t"]);
    run_git(dir.path(), &["config", "user.name", "t"]);
    run_git(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

/// A throwaway repo with two commits, for history/range tests that must not
/// depend on the crate repo's own (squashable) commit count.
pub(crate) fn multi_commit_repo() -> TempDir {
    let dir = scratch_repo();
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    run_git(dir.path(), &["add", "-A"]);
    run_git(dir.path(), &["commit", "-qm", "first"]);
    std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    run_git(dir.path(), &["add", "-A"]);
    run_git(dir.path(), &["commit", "-qm", "second"]);
    dir
}

// ---- review-store fixtures -------------------------------------------------

/// An empty review log in a throwaway worktree.
///
/// Deliberately named apart from [`opened_review_log`]: an earlier round of these
/// tests had one `temp_log` helper that appended an `open` record in two modules
/// and not in the other two, so the same call meant different things depending on
/// the file.
pub(crate) fn review_log() -> (TempDir, Log) {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = Log::at_worktree(dir.path());
    (dir, log)
}

/// A review log with an `open` record already appended, for tests about what
/// happens *within* a review rather than about opening one.
pub(crate) fn opened_review_log() -> (TempDir, Log) {
    let (dir, log) = review_log();
    log.append(&Record::Open {
        review: "r1".into(),
        target: "worktree".into(),
        label: None,
        at: now(),
    })
    .expect("append open");
    (dir, log)
}

/// A diffed `DiffFile` carrying `new_text`, for changeset fixtures. `None` models
/// a side with no readable content (a deletion, or a binary file).
pub(crate) fn diff_file(path: &str, new_text: Option<&str>) -> DiffFile {
    let mut f = DiffFile::stub(path.into(), None, FileStatus::Modified, false, None);
    f.new_text = new_text.map(ToString::to_string);
    f.diffed = true;
    f
}

/// A working-tree changeset over the given files.
pub(crate) fn changeset(files: Vec<DiffFile>) -> Changeset {
    Changeset {
        source: "worktree".into(),
        files,
    }
}
