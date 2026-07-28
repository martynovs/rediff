//! Core git changeset types: content sides, enumerated stubs, and load requests.

use crate::model::{DiffFile, FileStatus};

/// Where one side of a file's content comes from, resolved lazily at diff time
/// so enumeration reads no blob contents.
#[derive(Debug, Clone)]
pub enum Side {
    /// A blob object in the repository (its id is known from the tree/index).
    Blob(gix::ObjectId),
    /// The file at `path` in the working directory, read when diffed.
    Worktree(String),
    /// The side does not exist (an addition's old side, a deletion's new side).
    Absent,
}

/// A changed file enumerated without diffing it: path/status are known, the two
/// content sides are resolved (and the diff computed) later by [`diff_file`].
///
/// [`diff_file`]: super::diff::diff_file
#[derive(Debug, Clone)]
pub struct FileStub {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: FileStatus,
    pub staged: bool,
    pub language: Option<String>,
    pub old: Side,
    pub new: Side,
}

impl FileStub {
    /// The undiffed placeholder `DiffFile` shown until the background diff lands.
    pub fn as_stub_file(&self) -> DiffFile {
        DiffFile::stub(
            self.path.clone(),
            self.previous_path.clone(),
            self.status,
            self.staged,
            self.language.clone(),
        )
    }
}

/// What an enumeration produced: the human-readable source label and the stubs.
pub struct Enumeration {
    pub source: String,
    pub stubs: Vec<FileStub>,
}

/// What to load.
///
/// `PartialEq` matters beyond convenience: the review store records a target as a
/// canonical string, and the property that string must have is that it parses back
/// *equal* to what was encoded (`reviewcli::target`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadRequest {
    /// Working-tree changes (`base` vs worktree, `base` defaulting to HEAD).
    /// `include_untracked` adds untracked files.
    WorkingTree {
        include_untracked: bool,
        base: Option<String>,
    },
    /// Staged changes (HEAD vs index).
    Staged,
    /// A single commit (its parent vs itself).
    Show { rev: String },
    /// A two-dot range `old..new`.
    Range { old: String, new: String },
    /// A branch/range review: the combined net diff between the merge-base of
    /// `base` and `target` and `target` itself.
    ReviewRange { base: String, target: String },
}
