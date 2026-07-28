//! Plain-text unified rendering of a changeset — the Phase 1 stdout dump and a
//! useful debugging view before the TUI exists.

use std::fmt::Write as _;

use crate::model::{Changeset, FileStatus, LineKind};

/// Render a whole changeset as git-style unified diff text.
#[expect(
    clippy::let_underscore_must_use,
    reason = "writeln! into a String is infallible; the fmt::Result can't error"
)]
pub fn to_unified_string(cs: &Changeset) -> String {
    let mut out = String::new();
    for f in &cs.files {
        if let (FileStatus::Renamed | FileStatus::Copied, Some(prev)) = (f.status, &f.previous_path)
        {
            let _ = writeln!(out, "diff --git a/{prev} b/{}", f.path);
            let _ = writeln!(out, "{} from {prev}", f.status.label());
            let _ = writeln!(out, "{} to {}", f.status.label(), f.path);
            let _ = writeln!(out, "--- a/{prev}\n+++ b/{}", f.path);
        } else {
            let _ = writeln!(out, "diff --git a/{} b/{}", f.path, f.path);
            match f.status {
                FileStatus::Added | FileStatus::Untracked => out.push_str("new file\n"),
                FileStatus::Deleted => out.push_str("deleted file\n"),
                _ => {}
            }
            let _ = writeln!(out, "--- a/{}\n+++ b/{}", f.path, f.path);
        }

        if f.is_binary {
            out.push_str("Binary files differ\n\n");
            continue;
        }

        for h in &f.hunks {
            out.push_str(&h.header());
            out.push('\n');
            for line in &h.lines {
                let prefix = match line.kind {
                    LineKind::Context => ' ',
                    LineKind::Added => '+',
                    LineKind::Removed => '-',
                };
                out.push(prefix);
                out.push_str(&line.text);
                out.push('\n');
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::to_unified_string;
    use crate::model::{Changeset, DiffFile, FileStatus, Hunk, Line, Stats};

    /// A hunk with one context, one removed, and one added line — exercises all
    /// three `LineKind` formatting arms.
    fn mixed_hunk() -> Hunk {
        Hunk {
            old_start: 1,
            old_len: 2,
            new_start: 1,
            new_len: 2,
            lines: vec![
                Line::context("keep".into(), 0, 0),
                Line::removed("gone".into(), 1),
                Line::added("fresh".into(), 1),
            ],
        }
    }

    fn diff_file(
        path: &str,
        previous_path: Option<&str>,
        status: FileStatus,
        hunks: Vec<Hunk>,
    ) -> DiffFile {
        DiffFile {
            path: path.into(),
            previous_path: previous_path.map(Into::into),
            status,
            staged: false,
            hunks,
            stats: Stats::default(),
            language: None,
            is_binary: false,
            old_text: None,
            new_text: None,
            content_digest: None,
            diffed: true,
        }
    }

    fn changeset(files: Vec<DiffFile>) -> Changeset {
        Changeset {
            source: "test".into(),
            files,
        }
    }

    #[test]
    fn modified_file_emits_headers_and_all_gutters() {
        let cs = changeset(vec![diff_file(
            "src/auth.rs",
            None,
            FileStatus::Modified,
            vec![mixed_hunk()],
        )]);
        let out = to_unified_string(&cs);
        assert!(out.contains("diff --git a/src/auth.rs b/src/auth.rs"));
        // A modified file gets no new/deleted marker line.
        assert!(!out.contains("new file"));
        assert!(!out.contains("deleted file"));
        assert!(out.contains("--- a/src/auth.rs"));
        assert!(out.contains("+++ b/src/auth.rs"));
        assert!(out.contains("@@ -1,2 +1,2 @@"));
        // Each LineKind gutter prefix.
        assert!(out.contains(" keep\n"));
        assert!(out.contains("-gone\n"));
        assert!(out.contains("+fresh\n"));
    }

    #[test]
    fn added_file_marks_new_file() {
        let cs = changeset(vec![diff_file(
            "NEW.md",
            None,
            FileStatus::Added,
            vec![Hunk {
                old_start: 0,
                old_len: 0,
                new_start: 1,
                new_len: 1,
                lines: vec![Line::added("hello".into(), 0)],
            }],
        )]);
        let out = to_unified_string(&cs);
        assert!(out.contains("new file\n"));
        assert!(out.contains("--- a/NEW.md"));
        assert!(out.contains("+hello\n"));
    }

    #[test]
    fn untracked_file_also_marks_new_file() {
        let cs = changeset(vec![diff_file(
            "scratch.txt",
            None,
            FileStatus::Untracked,
            vec![],
        )]);
        let out = to_unified_string(&cs);
        assert!(out.contains("new file\n"));
    }

    #[test]
    fn deleted_file_marks_deleted_file() {
        let cs = changeset(vec![diff_file(
            "old.rs",
            None,
            FileStatus::Deleted,
            vec![Hunk {
                old_start: 1,
                old_len: 1,
                new_start: 0,
                new_len: 0,
                lines: vec![Line::removed("bye".into(), 0)],
            }],
        )]);
        let out = to_unified_string(&cs);
        assert!(out.contains("deleted file\n"));
        assert!(out.contains("-bye\n"));
    }

    #[test]
    fn renamed_file_uses_old_and_new_paths() {
        let cs = changeset(vec![diff_file(
            "src/new_name.rs",
            Some("src/old_name.rs"),
            FileStatus::Renamed,
            vec![],
        )]);
        let out = to_unified_string(&cs);
        assert!(out.contains("diff --git a/src/old_name.rs b/src/new_name.rs"));
        assert!(out.contains("renamed from src/old_name.rs"));
        assert!(out.contains("renamed to src/new_name.rs"));
        assert!(out.contains("--- a/src/old_name.rs"));
        assert!(out.contains("+++ b/src/new_name.rs"));
    }

    #[test]
    fn copied_file_uses_copy_labels() {
        let cs = changeset(vec![diff_file(
            "dst.rs",
            Some("src.rs"),
            FileStatus::Copied,
            vec![],
        )]);
        let out = to_unified_string(&cs);
        assert!(out.contains("diff --git a/src.rs b/dst.rs"));
        assert!(out.contains("copied from src.rs"));
        assert!(out.contains("copied to dst.rs"));
    }

    #[test]
    fn renamed_without_previous_path_falls_back_to_plain_header() {
        // Status says renamed but there's no previous_path: takes the else arm.
        let cs = changeset(vec![diff_file(
            "moved.rs",
            None,
            FileStatus::Renamed,
            vec![],
        )]);
        let out = to_unified_string(&cs);
        assert!(out.contains("diff --git a/moved.rs b/moved.rs"));
        assert!(!out.contains("renamed from"));
    }

    #[test]
    fn binary_file_short_circuits_hunks() {
        let mut f = diff_file("logo.png", None, FileStatus::Modified, vec![mixed_hunk()]);
        f.is_binary = true;
        let cs = changeset(vec![f]);
        let out = to_unified_string(&cs);
        assert!(out.contains("Binary files differ\n"));
        // Hunk content must not be rendered for a binary file.
        assert!(!out.contains("@@"));
        assert!(!out.contains("keep"));
    }

    #[test]
    fn multiple_hunks_and_files_all_render() {
        let cs = changeset(vec![
            diff_file(
                "a.rs",
                None,
                FileStatus::Modified,
                vec![
                    mixed_hunk(),
                    Hunk {
                        old_start: 10,
                        old_len: 1,
                        new_start: 10,
                        new_len: 1,
                        lines: vec![Line::context("second".into(), 9, 9)],
                    },
                ],
            ),
            diff_file("b.rs", None, FileStatus::Added, vec![]),
        ]);
        let out = to_unified_string(&cs);
        assert_eq!(out.matches("diff --git").count(), 2);
        assert!(out.contains("@@ -1,2 +1,2 @@"));
        assert!(out.contains("@@ -10,1 +10,1 @@"));
        assert!(out.contains("diff --git a/b.rs b/b.rs"));
    }
}
