//! The normalized changeset model. Every input source (working tree, staged,
//! a commit, a range) is loaded into one `Changeset` so the renderer,
//! navigation, and highlighting all derive from a single source of truth.

/// How diffs are laid out in the review stream. Resolved to one of these at
/// startup (a wide terminal defaults to split, a narrow one to stack); `m`
/// toggles between them at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Side-by-side old/new.
    Split,
    /// Unified top-to-bottom.
    Stack,
}

/// The role a single diff line plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// One rendered diff line with its old/new line numbers (1-based).
#[derive(Debug, Clone)]
pub struct Line {
    pub kind: LineKind,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    pub text: String,
    /// Char range `[start, end)` within `text` that differs from the paired line
    /// on the other side (word-level intra-line emphasis). `None` when unpaired
    /// or wholly changed.
    pub emphasis: Option<(u32, u32)>,
}

impl Line {
    /// An unchanged line, present on both sides (0-based positions in).
    pub fn context(text: String, old: u32, new: u32) -> Self {
        Self {
            kind: LineKind::Context,
            old_lineno: Some(old + 1),
            new_lineno: Some(new + 1),
            text,
            emphasis: None,
        }
    }
    /// A line added on the new side.
    pub fn added(text: String, new: u32) -> Self {
        Self {
            kind: LineKind::Added,
            old_lineno: None,
            new_lineno: Some(new + 1),
            text,
            emphasis: None,
        }
    }
    /// A line removed from the old side.
    pub fn removed(text: String, old: u32) -> Self {
        Self {
            kind: LineKind::Removed,
            old_lineno: Some(old + 1),
            new_lineno: None,
            text,
            emphasis: None,
        }
    }
}

/// One contiguous hunk with a unified `@@` header range and its lines.
#[derive(Debug, Clone)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub lines: Vec<Line>,
}

impl Hunk {
    /// The git-style unified hunk header, e.g. `@@ -1,4 +1,5 @@`.
    pub fn header(&self) -> String {
        format!(
            "@@ -{},{} +{},{} @@",
            self.old_start, self.old_len, self.new_start, self.new_len
        )
    }
}

/// How a file changed relative to the comparison base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Untracked,
}

impl FileStatus {
    /// A short label for sidebar/file-header display.
    pub fn label(&self) -> &'static str {
        match self {
            FileStatus::Added => "added",
            FileStatus::Modified => "modified",
            FileStatus::Deleted => "deleted",
            FileStatus::Renamed => "renamed",
            FileStatus::Copied => "copied",
            FileStatus::Untracked => "untracked",
        }
    }
}

/// Per-file added/removed line counts.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub additions: usize,
    pub deletions: usize,
}

/// One changed file in the changeset.
#[derive(Debug, Clone)]
pub struct DiffFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: FileStatus,
    pub staged: bool,
    pub hunks: Vec<Hunk>,
    pub stats: Stats,
    pub language: Option<String>,
    pub is_binary: bool,
    /// Full old/new file text, used by the highlighter (None for binary or an
    /// absent side).
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    /// Content hash of the raw new-side bytes, when this file has been diffed and
    /// has a new side.
    ///
    /// Taken from the bytes rather than [`new_text`](Self::new_text) so that a
    /// **binary** file — which carries no text — still has a fingerprint. The review
    /// store's rounds use it to tell whether a file moved between passes; without it
    /// a changed binary looks unchanged.
    pub content_digest: Option<u64>,
    /// Whether this file's diff (hunks, stats, text, binary flag) has been
    /// computed. A streaming load lists files as undiffed stubs first and fills
    /// each one in as its background diff completes. Navigation treats an
    /// undiffed file as zero-hunk; the renderer shows a placeholder.
    pub diffed: bool,
}

impl DiffFile {
    /// A not-yet-diffed file: its path and status are known (from a cheap
    /// enumeration) but its hunks/stats/text await the background diff.
    pub fn stub(
        path: String,
        previous_path: Option<String>,
        status: FileStatus,
        staged: bool,
        language: Option<String>,
    ) -> Self {
        Self {
            path,
            previous_path,
            status,
            staged,
            hunks: Vec::new(),
            stats: Stats::default(),
            language,
            is_binary: false,
            old_text: None,
            new_text: None,
            content_digest: None,
            diffed: false,
        }
    }
}

/// The whole set of changes being reviewed, in display order.
#[derive(Debug, Clone)]
pub struct Changeset {
    pub source: String,
    pub files: Vec<DiffFile>,
}

impl Changeset {
    /// Whether every file's diff has been computed.
    ///
    /// A streaming load lists files as stubs first and fills each in as its
    /// background diff completes, so mid-load a changeset carries entries with no
    /// content at all. Consumers that read content — the review store's round
    /// hashes and its anchor resolution — must not mistake "not computed yet" for
    /// "no content", so they check this first.
    #[must_use]
    pub fn fully_diffed(&self) -> bool {
        self.files.iter().all(|f| f.diffed)
    }
}

/// One commit's metadata, for the in-TUI commit picker.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    /// Full hex object id.
    pub id: String,
    /// Abbreviated object id for display.
    pub short: String,
    /// First line of the commit message.
    pub summary: String,
    /// Author name.
    pub author: String,
    /// Authored date as `YYYY-MM-DD`.
    pub date: String,
}

/// A commit's full message and identity, fetched by sha for the commit-message
/// popup and the commit-view banner.
#[derive(Debug, Clone, Default)]
pub struct CommitMessage {
    /// Full hex object id.
    pub sha: String,
    /// Abbreviated object id for display.
    pub short: String,
    /// Author name.
    pub author: String,
    /// Authored date as `YYYY-MM-DD`.
    pub date: String,
    /// The full commit message (summary line plus body), trailing space trimmed.
    pub body: String,
}

impl CommitMessage {
    /// The one-line identity header — `short · author · date` — shown in the
    /// commit-view banner and the popup title (one place so they can't drift).
    pub fn identity(&self) -> String {
        format!("{} · {} · {}", self.short, self.author, self.date)
    }
}

/// The per-commit facts the blame gutter, header, and color key need. Shared by
/// every line a commit is blamed for, so a whole run is one allocation.
#[derive(Debug, Default)]
pub struct BlameCommit {
    /// Full hex object id of the commit.
    pub sha: String,
    /// Author name.
    pub author: String,
    /// First line of the commit message (for the cursor-line header).
    pub summary: String,
    /// The commit's time as unix seconds, for the relative-age token.
    pub time_secs: i64,
    /// Stable color key derived from `sha` ([`commit_color_key`]), computed once
    /// when the commit is resolved so the gutter is a field read per paint, not a
    /// per-line re-hash of the 40-char sha.
    pub color_key: u64,
}

/// A stable color key derived from a commit sha, so every line of one commit's
/// run paints the same color and adjacent runs differ. Deterministic within a
/// run (the renderer maps the key onto the theme's palette). Lives here, the
/// lowest layer, so the blame builder (`git`) can cache it on [`BlameCommit`]
/// without depending on the TUI.
pub fn commit_color_key(sha: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    sha.hash(&mut h);
    h.finish()
}

/// One line's blame attribution, index-aligned to a file's lines. Lines from the
/// same commit share one `Arc<BlameCommit>`, so a run of lines is a single
/// allocation and run-start detection is a pointer compare; the gutter collapses
/// runs at render time and the full message is fetched by `sha` on demand.
#[derive(Debug, Clone, Default)]
pub struct BlameLine {
    pub commit: std::sync::Arc<BlameCommit>,
}

/// The parent directory of a `/`-separated path (git paths always use `/`), or
/// `""` for a file at the repository root.
pub fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        #[expect(
            clippy::string_slice,
            reason = "i is the byte offset of an ASCII '/', a char boundary"
        )]
        Some(i) => &path[..i],
        None => "",
    }
}

/// The final path segment (file name) of a `/`-separated path.
pub fn file_name(path: &str) -> &str {
    match path.rfind('/') {
        #[expect(
            clippy::string_slice,
            reason = "i+1 is just past an ASCII '/', a char boundary"
        )]
        Some(i) => &path[i + 1..],
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_status_label_covers_every_variant() {
        assert_eq!(FileStatus::Added.label(), "added");
        assert_eq!(FileStatus::Modified.label(), "modified");
        assert_eq!(FileStatus::Deleted.label(), "deleted");
        assert_eq!(FileStatus::Renamed.label(), "renamed");
        assert_eq!(FileStatus::Copied.label(), "copied");
        assert_eq!(FileStatus::Untracked.label(), "untracked");
    }

    #[test]
    fn commit_color_key_is_stable_and_distinguishes_shas() {
        assert_eq!(commit_color_key("abc"), commit_color_key("abc"), "stable");
        assert_ne!(commit_color_key("abc"), commit_color_key("def"), "differs");
    }
}
