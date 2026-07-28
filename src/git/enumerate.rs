//! Enumerate changed files (paths/status/sides) without reading blob contents,
//! plus the synchronous `load` that diffs every enumerated file.

use std::path::Path;

use gix::bstr::ByteSlice;
use gix::diff::index::ChangeRef;
use gix::diff::Rewrites;
use gix::status::tree_index::TrackRenames;
use indexmap::IndexMap;

use super::diff::diff_file;
use super::types::{Enumeration, FileStub, LoadRequest, Side};
use crate::lang;
use crate::model::{Changeset, FileStatus};

/// Load a changeset, computing every file's diff synchronously. Equivalent to
/// enumerating the files and then diffing each — the non-TUI text dump and the
/// tests use this; the TUI streams the diff step in the background instead.
pub fn load(repo_path: &Path, req: &LoadRequest) -> anyhow::Result<Changeset> {
    let repo = gix::discover(repo_path)?;
    let Enumeration { source, mut stubs } = enumerate_repo(&repo, req)?;
    order_changeset(&mut stubs);
    let files = stubs.iter().map(|s| diff_file(&repo, s)).collect();
    Ok(Changeset { source, files })
}

/// Order a changeset's files by parent directory, then file name — applied once
/// at load so files in a directory are contiguous and the order is stable rather
/// than git's enumeration order (which interleaves a directory's own files with
/// its subdirectories'). The sort key is `(parent_dir, name)`, NOT the full path:
/// full-path order splits a directory's group when it has both files and
/// subdirectories. See the add-sidebar-dir-grouping design.
fn order_changeset(stubs: &mut [FileStub]) {
    stubs.sort_by(|a, b| {
        let (pa, pb) = (
            crate::model::parent_dir(&a.path),
            crate::model::parent_dir(&b.path),
        );
        pa.cmp(pb)
            .then_with(|| crate::model::file_name(&a.path).cmp(crate::model::file_name(&b.path)))
    });
}

/// Enumerate a changeset's files (paths, status, rename source) without reading
/// blob contents or computing any diffs. The cheap first stage of a streaming
/// load; pair each stub with [`diff_file`] to fill it in.
pub fn enumerate(repo_path: &Path, req: &LoadRequest) -> anyhow::Result<Enumeration> {
    enumerate_in(&gix::discover(repo_path)?, req)
}

/// Like [`enumerate`], over an already-open repository — so a caller that also
/// reads (e.g.) a commit's message for the same switch shares one discover.
pub fn enumerate_in(repo: &gix::Repository, req: &LoadRequest) -> anyhow::Result<Enumeration> {
    let mut en = enumerate_repo(repo, req)?;
    order_changeset(&mut en.stubs);
    Ok(en)
}

/// The one funnel every source and both callers (`load` and `enumerate_in`) pass
/// through — so rediff's own review log is dropped exactly once, here, rather than
/// at each of the five request kinds.
fn enumerate_repo(repo: &gix::Repository, req: &LoadRequest) -> anyhow::Result<Enumeration> {
    let mut en = enumerate_source(repo, req)?;
    super::filter::drop_review_log(&mut en.stubs);
    Ok(en)
}

fn enumerate_source(repo: &gix::Repository, req: &LoadRequest) -> anyhow::Result<Enumeration> {
    match req {
        LoadRequest::WorkingTree {
            include_untracked,
            base,
        } => working_tree_stubs(repo, base.as_deref(), *include_untracked),
        LoadRequest::Staged => staged_stubs(repo),
        LoadRequest::Show { rev } => show_stubs(repo, rev),
        LoadRequest::Range { old, new } => {
            let old_tree = rev_tree(repo, old)?;
            let new_tree = rev_tree(repo, new)?;
            tree_to_tree_stubs(
                repo,
                Some(&old_tree),
                Some(&new_tree),
                format!("{old}..{new}"),
            )
        }
        LoadRequest::ReviewRange { base, target } => review_range_stubs(repo, base, target),
    }
}

/// Enumerate the range review's stubs (combined net diff between the merge-base
/// of `base`/`target` and `target`). Falls back to a literal two-dot tree diff
/// when no merge-base exists (unrelated histories).
fn review_range_stubs(
    repo: &gix::Repository,
    base: &str,
    target: &str,
) -> anyhow::Result<Enumeration> {
    let base_id = repo.rev_parse_single(base)?.detach();
    let target_id = repo.rev_parse_single(target)?.detach();
    let old_id = match repo.merge_base(base_id, target_id) {
        Ok(id) => id.detach(),
        Err(_) => base_id,
    };
    let old_tree = repo.find_object(old_id)?.peel_to_tree()?;
    let new_tree = repo.find_object(target_id)?.peel_to_tree()?;
    let short = |r: &str| {
        repo.rev_parse_single(r)
            .ok()
            .and_then(|i| i.shorten().ok().map(|p| p.to_string()))
            .unwrap_or_else(|| r.to_string())
    };
    tree_to_tree_stubs(
        repo,
        Some(&old_tree),
        Some(&new_tree),
        format!("review {}..{}", short(base), short(target)),
    )
}

pub(super) fn rev_tree<'r>(repo: &'r gix::Repository, rev: &str) -> anyhow::Result<gix::Tree<'r>> {
    let id = repo.rev_parse_single(rev)?;
    Ok(id.object()?.peel_to_tree()?)
}

// ---- working tree (HEAD vs worktree) ---------------------------------------

#[derive(Default)]
struct Entry {
    previous_path: Option<String>,
    seen_index: bool,
    seen_worktree: bool,
}

fn working_tree_stubs(
    repo: &gix::Repository,
    base: Option<&str>,
    include_untracked: bool,
) -> anyhow::Result<Enumeration> {
    // The old side: an explicit base ref's tree, or HEAD. An unborn branch (no
    // commits yet) has no HEAD tree — treat every file's old side as absent so
    // the whole working tree shows up as additions.
    let base_tree = match base {
        Some(r) => Some(rev_tree(repo, r)?),
        None => repo.head_commit().ok().and_then(|c| c.tree().ok()),
    };
    let workdir = repo.workdir().map(Path::to_path_buf);

    let mut platform = repo
        .status(gix::progress::Discard)?
        .tree_index_track_renames(TrackRenames::Given(Rewrites::default()));
    // Compare the working tree against the base ref's tree rather than HEAD.
    if base.is_some() {
        if let Some(t) = &base_tree {
            platform = platform.head_tree(t.id().detach());
        }
    }
    let iter = platform.into_iter(None)?;

    let mut entries: IndexMap<String, Entry> = IndexMap::new();
    for item in iter {
        let item = item?;
        let path = item.location().to_str_lossy().into_owned();
        let e = entries.entry(path).or_default();
        match &item {
            gix::status::Item::TreeIndex(change) => {
                e.seen_index = true;
                if let ChangeRef::Rewrite {
                    source_location, ..
                } = change
                {
                    e.previous_path = Some(source_location.to_str_lossy().into_owned());
                }
            }
            gix::status::Item::IndexWorktree(_) => {
                e.seen_worktree = true;
            }
        }
    }

    let mut stubs = Vec::new();
    for (path, e) in &entries {
        // Side existence is decided without reading blob contents: an object-id
        // lookup in the base tree, and a `stat` on the working-tree file.
        let base_lookup_path = e.previous_path.as_deref().unwrap_or(path);
        let old_id = base_tree
            .as_ref()
            .and_then(|t| tree_oid_at(t, base_lookup_path));
        let new_exists = workdir.as_ref().is_some_and(|w| w.join(path).exists());
        let untracked =
            old_id.is_none() && e.seen_worktree && !e.seen_index && e.previous_path.is_none();
        if untracked && !include_untracked {
            continue;
        }

        let status = if untracked {
            FileStatus::Untracked
        } else if e.previous_path.is_some() {
            FileStatus::Renamed
        } else if old_id.is_none() && new_exists {
            FileStatus::Added
        } else if old_id.is_some() && !new_exists {
            FileStatus::Deleted
        } else {
            FileStatus::Modified
        };

        let old = old_id.map_or(Side::Absent, Side::Blob);
        let new = if new_exists {
            Side::Worktree(path.clone())
        } else {
            Side::Absent
        };
        stubs.push(FileStub {
            path: path.clone(),
            previous_path: e.previous_path.clone(),
            status,
            staged: false,
            language: lang::detect(path),
            old,
            new,
        });
    }

    let source = match base {
        Some(r) => format!("worktree vs {r}"),
        None => "working tree".into(),
    };
    Ok(Enumeration { source, stubs })
}

// ---- staged (HEAD vs index) ------------------------------------------------

fn staged_stubs(repo: &gix::Repository) -> anyhow::Result<Enumeration> {
    let platform = repo
        .status(gix::progress::Discard)?
        .tree_index_track_renames(TrackRenames::Given(Rewrites::default()));
    let iter = platform.into_iter(None)?;

    let mut stubs = Vec::new();
    for item in iter {
        let item = item?;
        let gix::status::Item::TreeIndex(change) = &item else {
            continue;
        };
        match change {
            ChangeRef::Addition { location, id, .. } => {
                stubs.push(blob_stub(
                    &location.to_str_lossy(),
                    None,
                    FileStatus::Added,
                    true,
                    Side::Absent,
                    Side::Blob(id.clone().into_owned()),
                ));
            }
            ChangeRef::Deletion { location, id, .. } => {
                stubs.push(blob_stub(
                    &location.to_str_lossy(),
                    None,
                    FileStatus::Deleted,
                    true,
                    Side::Blob(id.clone().into_owned()),
                    Side::Absent,
                ));
            }
            ChangeRef::Modification {
                location,
                previous_id,
                id,
                ..
            } => {
                stubs.push(blob_stub(
                    &location.to_str_lossy(),
                    None,
                    FileStatus::Modified,
                    true,
                    Side::Blob(previous_id.clone().into_owned()),
                    Side::Blob(id.clone().into_owned()),
                ));
            }
            ChangeRef::Rewrite {
                source_location,
                source_id,
                location,
                id,
                ..
            } => {
                stubs.push(blob_stub(
                    &location.to_str_lossy(),
                    Some(source_location.to_str_lossy().into_owned()),
                    FileStatus::Renamed,
                    true,
                    Side::Blob(source_id.clone().into_owned()),
                    Side::Blob(id.clone().into_owned()),
                ));
            }
        }
    }

    Ok(Enumeration {
        source: "staged".into(),
        stubs,
    })
}

// ---- commit / range (tree vs tree) -----------------------------------------

fn show_stubs(repo: &gix::Repository, rev: &str) -> anyhow::Result<Enumeration> {
    let commit = repo.rev_parse_single(rev)?.object()?.peel_to_commit()?;
    let new_tree = commit.tree()?;
    let old_tree = match commit.parent_ids().next() {
        Some(pid) => Some(pid.object()?.peel_to_commit()?.tree()?),
        None => None,
    };
    let label = format!(
        "show {}",
        commit
            .id()
            .shorten()
            .map_or_else(|_| rev.to_string(), |s| s.to_string())
    );
    tree_to_tree_stubs(repo, old_tree.as_ref(), Some(&new_tree), label)
}

fn tree_to_tree_stubs(
    repo: &gix::Repository,
    old_tree: Option<&gix::Tree>,
    new_tree: Option<&gix::Tree>,
    source: String,
) -> anyhow::Result<Enumeration> {
    use gix::object::tree::diff::ChangeDetached as Change;

    let changes = repo.diff_tree_to_tree(old_tree, new_tree, None)?;
    let mut stubs = Vec::new();
    for change in changes {
        // Skip directory (tree) entries — we only diff file blobs.
        if change_entry_mode(&change).is_tree() {
            continue;
        }
        match change {
            Change::Addition { location, id, .. } => {
                stubs.push(blob_stub(
                    &location.to_str_lossy(),
                    None,
                    FileStatus::Added,
                    false,
                    Side::Absent,
                    Side::Blob(id),
                ));
            }
            Change::Deletion { location, id, .. } => {
                stubs.push(blob_stub(
                    &location.to_str_lossy(),
                    None,
                    FileStatus::Deleted,
                    false,
                    Side::Blob(id),
                    Side::Absent,
                ));
            }
            Change::Modification {
                location,
                previous_id,
                id,
                ..
            } => {
                stubs.push(blob_stub(
                    &location.to_str_lossy(),
                    None,
                    FileStatus::Modified,
                    false,
                    Side::Blob(previous_id),
                    Side::Blob(id),
                ));
            }
            Change::Rewrite {
                source_location,
                source_id,
                location,
                id,
                ..
            } => {
                stubs.push(blob_stub(
                    &location.to_str_lossy(),
                    Some(source_location.to_str_lossy().into_owned()),
                    FileStatus::Renamed,
                    false,
                    Side::Blob(source_id),
                    Side::Blob(id),
                ));
            }
        }
    }
    Ok(Enumeration { source, stubs })
}

/// Build a stub for a tree/index change whose sides are blob object ids.
fn blob_stub(
    path: &str,
    previous_path: Option<String>,
    status: FileStatus,
    staged: bool,
    old: Side,
    new: Side,
) -> FileStub {
    FileStub {
        path: path.to_string(),
        previous_path,
        status,
        staged,
        language: lang::detect(path),
        old,
        new,
    }
}

pub(super) fn tree_oid_at(tree: &gix::Tree, path: &str) -> Option<gix::ObjectId> {
    tree.clone()
        .lookup_entry_by_path(path)
        .ok()
        .flatten()
        .map(|e| e.object_id())
}

/// Read the entry mode regardless of which change variant it is.
fn change_entry_mode(
    change: &gix::object::tree::diff::ChangeDetached,
) -> gix::object::tree::EntryMode {
    use gix::object::tree::diff::ChangeDetached as Change;
    match change {
        Change::Addition { entry_mode, .. }
        | Change::Deletion { entry_mode, .. }
        | Change::Modification { entry_mode, .. }
        | Change::Rewrite { entry_mode, .. } => *entry_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The crate's own repo as a live fixture for HEAD-only checks (its commit
    /// count is not assumed — history-dependent tests build their own repo).
    fn fixture() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    /// A throwaway repo whose `HEAD~1..HEAD` range covers every file status the
    /// stub collector distinguishes: a modification, a deletion, and an addition
    /// (plus an untouched file that must NOT appear). The base commit is an
    /// ancestor of the target, so the merge-base is the base itself.
    fn review_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.t"]);
        run(&["config", "user.name", "t"]);
        run(&["config", "commit.gpgsign", "false"]);
        // Base commit: an untouched file, a file to modify, a file to delete.
        std::fs::write(dir.path().join("keep.txt"), "same\n").unwrap();
        std::fs::write(dir.path().join("mod.txt"), "one\n").unwrap();
        std::fs::write(dir.path().join("del.txt"), "bye\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "base"]);
        // Target commit: modify, delete, and add — keep.txt left alone.
        std::fs::write(dir.path().join("mod.txt"), "two\n").unwrap();
        std::fs::remove_file(dir.path().join("del.txt")).unwrap();
        std::fs::write(dir.path().join("new.txt"), "fresh\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "target"]);
        dir
    }

    fn stub(path: &str) -> FileStub {
        FileStub {
            path: path.into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            language: None,
            old: Side::Absent,
            new: Side::Absent,
        }
    }

    #[test]
    fn order_changeset_groups_by_directory() {
        // Unsorted, with a directory ("src") that has both direct files and a
        // subdirectory ("src/tui") — the case a full-path sort would split.
        let mut stubs: Vec<FileStub> = [
            "src/zzz.rs",
            "README.md",
            "src/tui/app.rs",
            "src/aaa.rs",
            "lib/b.rs",
        ]
        .iter()
        .map(|p| stub(p))
        .collect();
        order_changeset(&mut stubs);
        let order: Vec<&str> = stubs.iter().map(|s| s.path.as_str()).collect();
        // Root file first (parent ""), then dirs lexicographically; "src"'s own
        // files (aaa, zzz) stay contiguous, with the "src/tui" group after them.
        assert_eq!(
            order,
            [
                "README.md",
                "lib/b.rs",
                "src/aaa.rs",
                "src/zzz.rs",
                "src/tui/app.rs"
            ]
        );
    }

    #[test]
    fn working_tree_from_head_matches_default() {
        // `diff --from HEAD` (explicit base) must enumerate the same files as the
        // default working-tree diff (implicit HEAD base).
        let repo = gix::discover(fixture()).unwrap();
        let mut a: Vec<_> = working_tree_stubs(&repo, Some("HEAD"), true)
            .unwrap()
            .stubs
            .iter()
            .map(|s| s.path.clone())
            .collect();
        let mut b: Vec<_> = working_tree_stubs(&repo, None, true)
            .unwrap()
            .stubs
            .iter()
            .map(|s| s.path.clone())
            .collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }

    #[test]
    fn untracked_review_log_is_omitted_but_its_neighbours_are_not() {
        let dir = review_repo();
        std::fs::write(dir.path().join("rediff.jsonl"), "{\"t\":\"open\"}\n").unwrap();
        std::fs::write(dir.path().join("scratch.txt"), "untracked\n").unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/rediff.jsonl"), "not the log\n").unwrap();
        std::fs::write(dir.path().join("my-rediff.jsonl"), "nor this\n").unwrap();

        let cs = load(
            dir.path(),
            &LoadRequest::WorkingTree {
                include_untracked: true,
                base: None,
            },
        )
        .unwrap();
        let paths: Vec<&str> = cs.files.iter().map(|f| f.path.as_str()).collect();

        assert!(
            !paths.contains(&"rediff.jsonl"),
            "the review log is never review material: {paths:?}"
        );
        assert!(paths.contains(&"scratch.txt"), "other untracked files stay");
        assert!(
            paths.contains(&"sub/rediff.jsonl"),
            "subdirectory file stays"
        );
        assert!(paths.contains(&"my-rediff.jsonl"), "similar name stays");
    }

    #[test]
    fn a_committed_review_log_is_omitted_too() {
        let dir = review_repo();
        std::fs::write(dir.path().join("rediff.jsonl"), "{\"t\":\"open\"}\n").unwrap();
        std::fs::write(dir.path().join("also.txt"), "sibling\n").unwrap();
        crate::testutil::run_git(dir.path(), &["add", "-A"]);
        crate::testutil::run_git(dir.path(), &["commit", "-qm", "commit the log by accident"]);

        let cs = load(dir.path(), &LoadRequest::Show { rev: "HEAD".into() }).unwrap();
        let paths: Vec<&str> = cs.files.iter().map(|f| f.path.as_str()).collect();
        assert!(!paths.contains(&"rediff.jsonl"), "even when committed");
        assert!(paths.contains(&"also.txt"), "the rest of the commit stays");
    }

    #[test]
    fn review_range_collects_per_file_statuses() {
        let fixture = review_repo();
        let repo = gix::discover(fixture.path()).unwrap();
        let en = review_range_stubs(&repo, "HEAD~1", "HEAD").unwrap();

        // Source label reflects the reviewed range (short revs of base..target).
        assert!(
            en.source.starts_with("review "),
            "source labels the review range: {}",
            en.source
        );

        let by_path: HashMap<&str, &FileStub> =
            en.stubs.iter().map(|s| (s.path.as_str(), s)).collect();

        // The untouched file is absent; only the three changed files appear.
        assert_eq!(en.stubs.len(), 3, "stubs: {:?}", by_path.keys());
        assert!(!by_path.contains_key("keep.txt"), "unchanged file omitted");

        assert_eq!(by_path["mod.txt"].status, FileStatus::Modified);
        assert_eq!(by_path["del.txt"].status, FileStatus::Deleted);
        assert_eq!(by_path["new.txt"].status, FileStatus::Added);

        // Side resolution follows status: modified has both blob sides, the
        // deletion's new side is absent, the addition's old side is absent.
        assert!(matches!(by_path["mod.txt"].old, Side::Blob(_)));
        assert!(matches!(by_path["mod.txt"].new, Side::Blob(_)));
        assert!(matches!(by_path["del.txt"].new, Side::Absent));
        assert!(matches!(by_path["new.txt"].old, Side::Absent));
    }

    #[test]
    fn review_range_unrelated_histories_fall_back_to_base() {
        // No merge-base between two unrelated roots: review_range_stubs falls
        // back (the `Err(_) => base_id` arm) to diffing the base tree directly
        // against the target tree.
        let fixture = review_repo();
        let dir = fixture.path();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        // A second, parentless root commit sharing no history with HEAD.
        run(&["checkout", "-q", "--orphan", "other"]);
        run(&["rm", "-rfq", "--cached", "."]);
        std::fs::write(dir.join("only.txt"), "alone\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "orphan root"]);

        let repo = gix::discover(dir).unwrap();
        let en = review_range_stubs(&repo, "main", "other")
            .or_else(|_| review_range_stubs(&repo, "master", "other"))
            .unwrap();
        // With no merge-base, base ("main") is used as the old tree: every file
        // differs between the two unrelated roots, so the diff is non-empty.
        assert!(
            !en.stubs.is_empty(),
            "unrelated-history fallback still produces a diff"
        );
    }

    /// Run a sequence of `git` subcommands against `dir`, asserting each succeeds.
    fn git_runner(dir: &Path) -> impl Fn(&[&str]) + '_ {
        move |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// A repo with a base commit, then *index* changes covering every staged
    /// status the collector distinguishes — an addition, a modification, a
    /// deletion, and a rename (`git mv`, content preserved so it's detected as a
    /// rewrite) — plus one *unstaged* worktree modification (an `IndexWorktree`
    /// item `staged_stubs` must skip).
    fn staged_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = git_runner(dir.path());
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.t"]);
        run(&["config", "user.name", "t"]);
        run(&["config", "commit.gpgsign", "false"]);
        // Base commit.
        std::fs::write(dir.path().join("mod.txt"), "one\n").unwrap();
        std::fs::write(dir.path().join("del.txt"), "bye\n").unwrap();
        // A rename source with enough content for rename detection to be certain.
        let renamed_body = "line a\nline b\nline c\nline d\nline e\nline f\n";
        std::fs::write(dir.path().join("old_name.txt"), renamed_body).unwrap();
        // A file whose later edit is left unstaged.
        std::fs::write(dir.path().join("unstaged.txt"), "before\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "base"]);
        // Staged: modify, delete, add, rename.
        std::fs::write(dir.path().join("mod.txt"), "two\n").unwrap();
        run(&["add", "mod.txt"]);
        run(&["rm", "-q", "del.txt"]);
        std::fs::write(dir.path().join("new.txt"), "fresh\n").unwrap();
        run(&["add", "new.txt"]);
        run(&["mv", "old_name.txt", "new_name.txt"]);
        // Unstaged: a working-tree-only edit (no `git add`).
        std::fs::write(dir.path().join("unstaged.txt"), "after\n").unwrap();
        drop(run); // release the borrow of `dir` so it can be returned
        dir
    }

    #[test]
    fn staged_stubs_collects_every_change_kind() {
        let fixture = staged_repo();
        let repo = gix::discover(fixture.path()).unwrap();
        let en = staged_stubs(&repo).unwrap();
        assert_eq!(en.source, "staged");

        let by_path: HashMap<&str, &FileStub> =
            en.stubs.iter().map(|s| (s.path.as_str(), s)).collect();

        // Addition: old side absent, new side a staged blob.
        assert_eq!(by_path["new.txt"].status, FileStatus::Added);
        assert!(by_path["new.txt"].staged);
        assert!(matches!(by_path["new.txt"].old, Side::Absent));
        assert!(matches!(by_path["new.txt"].new, Side::Blob(_)));

        // Modification: both sides are blobs.
        assert_eq!(by_path["mod.txt"].status, FileStatus::Modified);
        assert!(matches!(by_path["mod.txt"].old, Side::Blob(_)));
        assert!(matches!(by_path["mod.txt"].new, Side::Blob(_)));

        // Deletion: new side absent.
        assert_eq!(by_path["del.txt"].status, FileStatus::Deleted);
        assert!(matches!(by_path["del.txt"].new, Side::Absent));

        // Rename (Rewrite arm): previous_path carries the source, both sides blobs.
        let renamed = &by_path["new_name.txt"];
        assert_eq!(renamed.status, FileStatus::Renamed);
        assert_eq!(renamed.previous_path.as_deref(), Some("old_name.txt"));
        assert!(matches!(renamed.old, Side::Blob(_)));
        assert!(matches!(renamed.new, Side::Blob(_)));

        // The unstaged worktree edit is an IndexWorktree item and is skipped.
        assert!(
            !by_path.contains_key("unstaged.txt"),
            "unstaged change must not appear among staged stubs"
        );
    }

    #[test]
    fn staged_stubs_empty_when_index_matches_head() {
        // Its own repo, deliberately: this used to run against the crate's
        // checkout, so it failed for anyone who happened to have something
        // staged — including this suite's own `git mv`-based fixtures and any
        // contributor running tests mid-commit. A test whose result depends on
        // the developer's index is a landmine, not a check.
        let dir = tempfile::tempdir().expect("tempdir");
        let run = git_runner(dir.path());
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.t"]);
        run(&["config", "user.name", "t"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "base"]);

        // Index matches HEAD: the loop body never runs, and an empty
        // enumeration comes back.
        let repo = gix::discover(dir.path()).unwrap();
        let en = staged_stubs(&repo).unwrap();
        assert_eq!(en.source, "staged");
        assert!(en.stubs.is_empty(), "nothing staged, nothing enumerated");
    }

    #[test]
    fn enumerate_repo_dispatches_staged() {
        let fixture = staged_repo();
        let repo = gix::discover(fixture.path()).unwrap();
        let en = enumerate_repo(&repo, &LoadRequest::Staged).unwrap();
        assert_eq!(en.source, "staged");
        let paths: Vec<&str> = en.stubs.iter().map(|s| s.path.as_str()).collect();
        assert!(paths.contains(&"new.txt"));
        assert!(!paths.contains(&"unstaged.txt"));
    }

    #[test]
    fn enumerate_repo_dispatches_working_tree() {
        // The WorkingTree arm, with and without untracked, against a repo that has
        // a tracked-but-unstaged edit and an untracked file.
        let fixture = staged_repo();
        std::fs::write(fixture.path().join("untracked.txt"), "u\n").unwrap();
        let repo = gix::discover(fixture.path()).unwrap();

        let with = enumerate_repo(
            &repo,
            &LoadRequest::WorkingTree {
                include_untracked: true,
                base: None,
            },
        )
        .unwrap();
        assert_eq!(with.source, "working tree");
        let with_paths: Vec<&str> = with.stubs.iter().map(|s| s.path.as_str()).collect();
        assert!(with_paths.contains(&"untracked.txt"), "{with_paths:?}");

        let without = enumerate_repo(
            &repo,
            &LoadRequest::WorkingTree {
                include_untracked: false,
                base: None,
            },
        )
        .unwrap();
        let without_paths: Vec<&str> = without.stubs.iter().map(|s| s.path.as_str()).collect();
        assert!(
            !without_paths.contains(&"untracked.txt"),
            "untracked excluded when include_untracked=false: {without_paths:?}"
        );
    }

    #[test]
    fn enumerate_repo_dispatches_show_range_and_review() {
        let fixture = review_repo();
        let repo = gix::discover(fixture.path()).unwrap();

        // Show: a single commit vs its parent.
        let show = enumerate_repo(&repo, &LoadRequest::Show { rev: "HEAD".into() }).unwrap();
        assert!(show.source.starts_with("show "), "{}", show.source);
        assert!(!show.stubs.is_empty());

        // Range: a two-dot tree-to-tree diff.
        let range = enumerate_repo(
            &repo,
            &LoadRequest::Range {
                old: "HEAD~1".into(),
                new: "HEAD".into(),
            },
        )
        .unwrap();
        assert_eq!(range.source, "HEAD~1..HEAD");
        assert!(!range.stubs.is_empty());

        // ReviewRange: net diff between merge-base and target.
        let review = enumerate_repo(
            &repo,
            &LoadRequest::ReviewRange {
                base: "HEAD~1".into(),
                target: "HEAD".into(),
            },
        )
        .unwrap();
        assert!(review.source.starts_with("review "), "{}", review.source);
        assert!(!review.stubs.is_empty());
    }
}
