use super::*;

const MODIFY: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1234567..89abcde 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 fn main() {
-    let x = 1;
+    let x = 2;
 }
";

const ADD: &str = "\
diff --git a/new.rs b/new.rs
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,2 @@
+fn added() {}
+const N: u32 = 0;
";

const DELETE: &str = "\
diff --git a/old.rs b/old.rs
deleted file mode 100644
index 1111111..0000000
--- a/old.rs
+++ /dev/null
@@ -1 +0,0 @@
-fn gone() {}
";

const RENAME: &str = "\
diff --git a/a.rs b/b.rs
similarity index 100%
rename from a.rs
rename to b.rs
";

const BINARY: &str = "\
diff --git a/img.png b/img.png
index 1111111..2222222 100644
Binary files a/img.png and b/img.png differ
";

fn dark() -> ThemeName {
    ThemeName::Dark
}

#[test]
fn empty_input_yields_empty_output() {
    assert_eq!(render("", dark()), "");
}

#[test]
fn modify_renders_header_hunk_and_gutters() {
    let out = render(MODIFY, dark());
    assert!(out.contains("src/lib.rs"), "file header path");
    assert!(out.contains("(modified)"), "status label");
    assert!(out.contains("@@ -1,3 +1,3 @@"), "hunk header");
    assert!(out.contains("- "), "removed gutter");
    assert!(out.contains("+ "), "added gutter");
}

#[test]
fn output_is_always_colored_even_though_piped() {
    // A pager's stdout is a pipe, not a TTY; color must be forced on.
    let out = render(MODIFY, dark());
    assert!(out.contains("\x1b[38;2;"), "truecolor fg escapes present");
    assert!(
        out.contains("\x1b[48;2;"),
        "add/del background escapes present"
    );
}

#[test]
fn added_and_deleted_files() {
    let add = render(ADD, dark());
    assert!(add.contains("new.rs") && add.contains("(added)"));
    assert!(add.contains("@@ -0,0 +1,2 @@"));

    let del = render(DELETE, dark());
    assert!(del.contains("old.rs") && del.contains("(deleted)"));
    assert!(del.contains("- "), "deleted content gutter");
}

#[test]
fn rename_shows_old_to_new_and_no_hunks() {
    let out = render(RENAME, dark());
    assert!(out.contains("a.rs → b.rs"), "rename arrow: {out:?}");
    assert!(out.contains("(renamed)"));
    assert!(!out.contains("@@"), "pure rename has no hunks");
}

#[test]
fn binary_file_shows_notice_not_garbage() {
    let out = render(BINARY, dark());
    assert!(out.contains("Binary file"), "binary notice: {out:?}");
    assert!(out.contains("img.png"));
    assert!(!out.contains("@@"), "no hunk header for binary");
}

#[test]
fn color_coded_input_is_parsed() {
    // lazygit pipes `git diff --color=always`; the pager must strip ANSI and
    // still render, not go blank. Wrap each line of MODIFY in a fake SGR pair.
    let mut colored = String::new();
    for l in MODIFY.lines() {
        colored.push_str("\x1b[33m");
        colored.push_str(l);
        colored.push_str("\x1b[m\n");
    }
    let out = render(&colored, dark());
    assert!(out.contains("src/lib.rs"), "header still parsed: {out:?}");
    assert!(out.contains("@@ -1,3 +1,3 @@"), "hunk still parsed");
    assert!(!out.is_empty());
}

#[test]
fn external_modify_renders_full_file_diff() {
    let old = b"fn main() {\n    let x = 1;\n}\n";
    let new = b"fn main() {\n    let x = 2;\n}\n";
    let out = render_external("src/lib.rs", old, new, dark());
    assert!(out.contains("src/lib.rs") && out.contains("(modified)"));
    assert!(out.contains("@@"), "hunk header");
    assert!(out.contains("- ") && out.contains("+ "), "both gutters");
    assert!(out.contains("\x1b[38;2;"), "colored");
}

#[test]
fn external_added_and_deleted_by_dev_null() {
    // git passes an empty (/dev/null) side for create/delete.
    let added = render_external("new.rs", b"", b"fn a() {}\n", dark());
    assert!(added.contains("(added)") && added.contains("+ "));

    let deleted = render_external("old.rs", b"fn a() {}\n", b"", dark());
    assert!(deleted.contains("(deleted)") && deleted.contains("- "));
}

#[test]
fn external_display_path_handles_dev_null() {
    // Normal modify: arg[0] is the path.
    let modify = vec_s(&["src/lib.rs", "/tmp/old", "hex", "100644", "src/lib.rs"]);
    assert_eq!(display_path(&modify), "src/lib.rs");
    // Untracked --no-index create: arg[0] is /dev/null, real name in arg[4]/arg[7].
    let untracked = vec_s(&[
        "/dev/null",
        "/dev/null",
        ".",
        ".",
        "NEWFILE.txt",
        "hash",
        "100644",
        "NEWFILE.txt",
    ]);
    assert_eq!(display_path(&untracked), "NEWFILE.txt");
}

fn vec_s(a: &[&str]) -> Vec<String> {
    a.iter().map(std::string::ToString::to_string).collect()
}

#[test]
fn external_binary_notice() {
    let out = render_external("img.png", b"\x89PNG\x00\x00", b"\x89PNG\x00\x01", dark());
    assert!(out.contains("Binary file") && out.contains("img.png"));
    assert!(!out.contains("@@"));
}

#[test]
fn multi_file_renders_every_file() {
    let input = format!("{MODIFY}{ADD}");
    let out = render(&input, dark());
    assert!(out.contains("src/lib.rs"));
    assert!(out.contains("new.rs"));
}

#[test]
fn unparseable_section_is_skipped_not_fatal() {
    // Garbage preamble before a valid diff must not abort the whole render.
    let input = format!("not a diff at all\nrandom text\n{MODIFY}");
    let out = render(&input, dark());
    assert!(out.contains("src/lib.rs"));
}

use std::borrow::Cow;

fn cow(s: &str) -> Cow<'_, str> {
    Cow::Borrowed(s)
}

#[test]
fn describe_covers_every_file_operation_variant() {
    // Create / Delete: clean path, no previous, status follows the op.
    let (p, prev, st) = describe(&FileOperation::Create(cow("a.rs")));
    assert_eq!((p.as_str(), prev, st), ("a.rs", None, FileStatus::Added));

    let (p, prev, st) = describe(&FileOperation::Delete(cow("a.rs")));
    assert_eq!((p.as_str(), prev, st), ("a.rs", None, FileStatus::Deleted));

    // Modify with differing paths → treated as a rename (prev kept).
    let (p, prev, st) = describe(&FileOperation::Modify {
        original: cow("old.rs"),
        modified: cow("new.rs"),
    });
    assert_eq!(p, "new.rs");
    assert_eq!(prev.as_deref(), Some("old.rs"));
    assert_eq!(st, FileStatus::Renamed);

    // Modify with identical paths → in-place modification, no prev.
    let (p, prev, st) = describe(&FileOperation::Modify {
        original: cow("same.rs"),
        modified: cow("same.rs"),
    });
    assert_eq!(
        (p.as_str(), prev, st),
        ("same.rs", None, FileStatus::Modified)
    );

    // Rename / Copy: dest path with the source as prev.
    let (p, prev, st) = describe(&FileOperation::Rename {
        from: cow("from.rs"),
        to: cow("to.rs"),
    });
    assert_eq!(p, "to.rs");
    assert_eq!(prev.as_deref(), Some("from.rs"));
    assert_eq!(st, FileStatus::Renamed);

    let (p, prev, st) = describe(&FileOperation::Copy {
        from: cow("src.rs"),
        to: cow("dst.rs"),
    });
    assert_eq!(p, "dst.rs");
    assert_eq!(prev.as_deref(), Some("src.rs"));
    assert_eq!(st, FileStatus::Copied);
}

#[test]
fn cr_extracts_rgb_and_falls_back_for_non_rgb() {
    assert_eq!(cr(Color::Rgb(1, 2, 3)), (1, 2, 3));
    // Any non-Rgb chrome color resolves to the neutral grey fallback.
    assert_eq!(cr(Color::Reset), (200, 200, 200));
    assert_eq!(cr(Color::Red), (200, 200, 200));
}

#[test]
fn resolve_handles_every_paint_kind() {
    let syntax = vec![(10, 20, 30), (40, 50, 60)];
    let ctx = (7, 7, 7);
    // Capture in range → indexes the syntax table.
    assert_eq!(resolve(Paint::Capture(1), &syntax, ctx), (40, 50, 60));
    // Capture out of range → context fallback.
    assert_eq!(resolve(Paint::Capture(99), &syntax, ctx), ctx);
    // Default → context color.
    assert_eq!(resolve(Paint::Default, &syntax, ctx), ctx);
    // Fixed → passes through verbatim.
    assert_eq!(resolve(Paint::Fixed((1, 2, 3)), &syntax, ctx), (1, 2, 3));
}

#[test]
fn push_line_appends_newline_only_when_missing() {
    let mut s = String::new();
    push_line(&mut s, "already\n");
    push_line(&mut s, "needs-one");
    assert_eq!(s, "already\nneeds-one\n");
}

#[test]
fn display_path_falls_back_when_all_dev_null_or_empty() {
    // Index 0 empty is skipped; the first non-empty, non-/dev/null wins.
    let with_empty = vec_s(&["", "/tmp/old", "hex", "mode", "real.rs"]);
    assert_eq!(display_path(&with_empty), "real.rs");

    // Every candidate is /dev/null → fall back to args[0] (also /dev/null).
    let all_null = vec_s(&["/dev/null", "/dev/null", "x", "x", "/dev/null"]);
    assert_eq!(display_path(&all_null), "/dev/null");

    // No args at all → empty string.
    let none: Vec<String> = Vec::new();
    assert_eq!(display_path(&none), "");
}

#[test]
fn write_hunk_header_with_and_without_function_context() {
    let theme = Theme::new(dark());

    let mut with_ctx = String::new();
    write_hunk_header(&mut with_ctx, &theme, 1, 3, 1, 4, Some("fn main() {\n"));
    assert!(with_ctx.contains("@@ -1,3 +1,4 @@"));
    // Function context is appended, with its trailing EOL trimmed.
    assert!(with_ctx.contains("@@ fn main() {"));
    assert!(!with_ctx.contains("fn main() {\n}"));
    assert!(with_ctx.ends_with(&format!("{RESET}\n")));

    let mut no_ctx = String::new();
    write_hunk_header(&mut no_ctx, &theme, 10, 2, 12, 0, None);
    assert!(no_ctx.contains("@@ -10,2 +12,0 @@"));
    // Header ends right after the closing @@ (no extra context text).
    assert!(no_ctx.contains("+12,0 @@\x1b[0m"));
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("rediff-pager-test-{}-{name}", std::process::id()));
    dir
}

#[test]
fn read_blob_reads_file_dev_null_and_missing() {
    // A real file → its bytes.
    let p = tmp_path("blob.txt");
    std::fs::write(&p, b"hello blob\n").unwrap();
    assert_eq!(read_blob(p.to_str().unwrap()), b"hello blob\n");
    std::fs::remove_file(&p).unwrap();

    // /dev/null → empty without touching the filesystem.
    assert!(read_blob("/dev/null").is_empty());

    // Unreadable / missing path → empty (unwrap_or_default).
    assert!(read_blob(p.to_str().unwrap()).is_empty());
}

#[test]
fn read_head_caps_bytes_and_handles_dev_null_and_missing() {
    let p = tmp_path("head.txt");
    std::fs::write(&p, b"0123456789").unwrap();
    // Capped read: only the first n bytes.
    assert_eq!(read_head(p.to_str().unwrap(), 4), b"0123");
    // n larger than the file → whole file.
    assert_eq!(read_head(p.to_str().unwrap(), 100), b"0123456789");
    std::fs::remove_file(&p).unwrap();

    // /dev/null short-circuits to empty.
    assert!(read_head("/dev/null", 8).is_empty());
    // Missing file → File::open Err → empty.
    assert!(read_head(p.to_str().unwrap(), 8).is_empty());
}

#[test]
fn external_runs_text_and_binary_paths_to_stdout() {
    // Text files on both sides → exercises the read_blob/render_external path.
    let old = tmp_path("ext-old.rs");
    let new = tmp_path("ext-new.rs");
    std::fs::write(&old, b"fn main() {\n    let x = 1;\n}\n").unwrap();
    std::fs::write(&new, b"fn main() {\n    let x = 2;\n}\n").unwrap();
    let args = vec_s(&[
        "src/lib.rs",
        old.to_str().unwrap(),
        "oldhex",
        "100644",
        new.to_str().unwrap(),
        "newhex",
        "100644",
    ]);
    external(&args, dark()).unwrap();

    // Binary side → exercises the read_head binary-classification branch.
    let bin = tmp_path("ext-bin");
    std::fs::write(&bin, b"\x89PNG\x00\x00\x00").unwrap();
    let bin_args = vec_s(&[
        "img.png",
        "/dev/null",
        "oldhex",
        "100644",
        bin.to_str().unwrap(),
        "newhex",
        "100644",
    ]);
    external(&bin_args, dark()).unwrap();

    std::fs::remove_file(&old).unwrap();
    std::fs::remove_file(&new).unwrap();
    std::fs::remove_file(&bin).unwrap();
}

#[test]
fn emit_covers_all_kinds_and_span_fallback() {
    let theme = Theme::new(dark());
    let syntax = theme.name.syntax_table();
    let ctx = theme.context_rgb();

    // Added line with explicit spans → has a background and walks the spans.
    let spans = vec![
        Span {
            text: "let ".to_string(),
            paint: Paint::Capture(0),
        },
        Span {
            text: "x".to_string(),
            paint: Paint::Default,
        },
    ];
    let mut added = String::new();
    emit(
        &mut added,
        &theme,
        &syntax,
        ctx,
        LineKind::Added,
        "let x\n",
        Some(&spans),
    );
    assert!(added.starts_with("\x1b[48;2;"), "add background present");
    assert!(added.contains("+ "), "added gutter");
    assert!(added.contains("let "), "span text emitted");
    assert!(added.contains('x'));

    // Removed line, no spans → background + raw-text fallback path.
    let mut removed = String::new();
    emit(
        &mut removed,
        &theme,
        &syntax,
        ctx,
        LineKind::Removed,
        "gone\n",
        None,
    );
    assert!(removed.starts_with("\x1b[48;2;"), "del background present");
    assert!(removed.contains("- "), "removed gutter");
    assert!(removed.contains("gone"));

    // Context line with an *empty* spans vec → still takes the raw fallback,
    // and emits no background sequence.
    let empty: Vec<Span> = Vec::new();
    let mut context = String::new();
    emit(
        &mut context,
        &theme,
        &syntax,
        ctx,
        LineKind::Context,
        "ctx\n",
        Some(&empty),
    );
    assert!(!context.contains("\x1b[48;2;"), "context has no background");
    assert!(context.contains("  "), "space gutter");
    assert!(context.contains("ctx"));
    assert!(context.ends_with(&format!("{RESET}\n")));
}

const REVIEW_LOG_DIFF: &str = "\
diff --git a/rediff.jsonl b/rediff.jsonl
--- a/rediff.jsonl
+++ b/rediff.jsonl
@@ -1,1 +1,2 @@
 {\"t\":\"open\",\"review\":\"r1\"}
+{\"t\":\"thread\",\"id\":\"t1\",\"body\":\"a private note\"}
";

const SUBDIR_LOG_DIFF: &str = "\
diff --git a/sub/rediff.jsonl b/sub/rediff.jsonl
--- a/sub/rediff.jsonl
+++ b/sub/rediff.jsonl
@@ -1,1 +1,2 @@
 keep
+me
";

#[test]
fn the_review_log_is_not_rendered_by_the_pager() {
    // git hands the pager whatever it diffed; rediff's own log is tool state, so
    // it must not come back out — otherwise the reviewer reads their own review.
    let out = render(REVIEW_LOG_DIFF, dark());
    assert_eq!(out, "", "the log section is dropped entirely");
    assert!(!out.contains("a private note"));
}

#[test]
fn the_pager_still_renders_other_files_alongside_the_log() {
    let combined = format!("{REVIEW_LOG_DIFF}{MODIFY}");
    let out = render(&combined, dark());
    assert!(out.contains("src/lib.rs"), "the real file survives");
    assert!(!out.contains("rediff.jsonl"), "the log does not");
}

#[test]
fn a_log_named_file_in_a_subdirectory_is_ordinary() {
    let out = render(SUBDIR_LOG_DIFF, dark());
    assert!(
        out.contains("sub/rediff.jsonl"),
        "only the worktree-root log is tool state"
    );
}

#[test]
fn external_skips_the_review_log() {
    // `rediff external` is invoked per file by git (the lazygit setup), so the
    // check has to live there too, not only in the loader.
    assert!(is_review_log("rediff.jsonl"));
    assert!(is_review_log("./rediff.jsonl"));
    assert!(!is_review_log("sub/rediff.jsonl"));
    assert!(!is_review_log("my-rediff.jsonl"));
    assert!(!is_review_log("rediff.jsonl.golden"));
}
