//! Resolve a stub's two content sides to bytes and compute its diff, plus the
//! single-file blob/worktree text reads used by the peek.

use std::path::Path;

use super::enumerate::rev_tree;
use super::types::{FileStub, Side};
use crate::diff::compute_hunks;
use crate::model::{DiffFile, Stats};

/// Compute one enumerated file's diff: resolve its two sides to bytes, run the
/// diff, and produce a fully populated `DiffFile`. Safe to call off the UI
/// thread (each worker opens its own repository).
pub fn diff_file(repo: &gix::Repository, stub: &FileStub) -> DiffFile {
    let old = resolve_side(repo, &stub.old);
    let new = resolve_side(repo, &stub.new);
    build_diffed(stub, old.as_deref(), new.as_deref())
}

fn resolve_side(repo: &gix::Repository, side: &Side) -> Option<Vec<u8>> {
    match side {
        Side::Blob(id) => repo.find_object(*id).ok().map(|o| o.data.clone()),
        Side::Worktree(path) => read_worktree(repo.workdir(), path),
        Side::Absent => None,
    }
}

fn blob_at_path(tree: &gix::Tree, path: &str) -> Option<Vec<u8>> {
    tree.clone()
        .lookup_entry_by_path(path)
        .ok()
        .flatten()
        .and_then(|e| e.object().ok())
        .map(|o| o.data.clone())
}

fn read_worktree(workdir: Option<&Path>, path: &str) -> Option<Vec<u8>> {
    let root = workdir?;
    std::fs::read(root.join(path)).ok()
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

fn to_text(bytes: Option<&[u8]>) -> String {
    bytes
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default()
}

/// Build a fully-diffed `DiffFile` from a stub and its two resolved content
/// sides. The status is taken from the stub (decided during enumeration).
fn build_diffed(stub: &FileStub, old: Option<&[u8]>, new: Option<&[u8]>) -> DiffFile {
    let binary = old.is_some_and(is_binary) || new.is_some_and(is_binary);
    let old_full = to_text(old);
    let new_full = to_text(new);
    let (hunks, additions, deletions) = if binary {
        (Vec::new(), 0, 0)
    } else {
        compute_hunks(&old_full, &new_full)
    };

    DiffFile {
        path: stub.path.clone(),
        previous_path: stub.previous_path.clone(),
        status: stub.status,
        staged: stub.staged,
        hunks,
        stats: Stats {
            additions,
            deletions,
        },
        language: stub.language.clone(),
        is_binary: binary,
        old_text: if binary || old.is_none() {
            None
        } else {
            Some(old_full)
        },
        new_text: if binary || new.is_none() {
            None
        } else {
            Some(new_full)
        },
        // From the raw bytes, so a binary file — which carries no text — still has
        // a fingerprint the review store's rounds can compare across passes.
        content_digest: new.map(crate::review::content_hash),
        diffed: true,
    }
}

// ---- single-file loads (for the peek) --------------------------------------

/// Full text of a file at a revision (None when absent).
pub fn file_text_at(repo_dir: &Path, rev: &str, path: &str) -> Option<String> {
    let repo = gix::discover(repo_dir).ok()?;
    file_text_at_in(&repo, rev, path)
}

/// Like [`file_text_at`], over an already-open repository — a worker doing
/// several reads pays for one discover.
pub fn file_text_at_in(repo: &gix::Repository, rev: &str, path: &str) -> Option<String> {
    let tree = rev_tree(repo, rev).ok()?;
    let bytes = blob_at_path(&tree, path)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Like [`file_text_at_in`] but for an already-resolved commit id — so a caller
/// that also blames the same commit resolves the rev once and reads both the
/// text and the attribution from that id, not re-parsing the spec twice.
pub fn file_text_at_commit(
    repo: &gix::Repository,
    commit: gix::ObjectId,
    path: &str,
) -> Option<String> {
    let tree = repo
        .find_object(commit)
        .ok()?
        .try_into_commit()
        .ok()?
        .tree()
        .ok()?;
    let bytes = blob_at_path(&tree, path)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Full text of a file in the working tree (None when absent).
pub fn worktree_text(repo_dir: &Path, path: &str) -> Option<String> {
    let repo = gix::discover(repo_dir).ok()?;
    worktree_text_in(&repo, path)
}

/// Like [`worktree_text`], over an already-open repository — so a caller reading
/// both sides of a stub shares one discover.
pub fn worktree_text_in(repo: &gix::Repository, path: &str) -> Option<String> {
    let bytes = read_worktree(repo.workdir(), path)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crate's own repo as a live fixture for HEAD-only checks.
    fn fixture() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn file_text_loads_at_rev_and_worktree() {
        let at_head = file_text_at(fixture(), "HEAD", "Cargo.toml").unwrap();
        assert!(
            at_head.contains("[package]"),
            "loads a file's blob at a rev"
        );
        let wt = worktree_text(fixture(), "Cargo.toml").unwrap();
        assert!(wt.contains("[package]"), "reads the working-tree file");
        assert!(file_text_at(fixture(), "HEAD", "does/not/exist.rs").is_none());
    }
}
