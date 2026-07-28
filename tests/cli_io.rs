//! Program-boundary integration tests: drive the real `rediff` binary as a
//! subprocess so the non-interactive entry points (`main`'s pipe path, the
//! `pager`/`external` filters, and `Config::load`) execute end to end.
//!
//! cargo-llvm-cov instruments subprocesses of the test binary too: the
//! `CARGO_BIN_EXE_rediff` child inherits `LLVM_PROFILE_FILE`, so the coverage of
//! these otherwise terminal-bound functions is captured here.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use common::GitFixture;

const MODIFY_DIFF: &str = "\
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

/// A temp dir holding `rediff/config.toml` so the child's `Config::load` runs
/// its read + TOML-parse path (rather than the absent-file default).
fn config_home(theme: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("config tempdir");
    let cfg_dir = dir.path().join("rediff");
    std::fs::create_dir_all(&cfg_dir).expect("mk config dir");
    std::fs::write(
        cfg_dir.join("config.toml"),
        format!("theme = \"{theme}\"\nmode = \"stack\"\n"),
    )
    .expect("write config");
    dir
}

/// Run the binary with the given args, a pinned `XDG_CONFIG_HOME`, optional
/// stdin, and a piped (non-TTY) stdout. Returns (stdout, success).
fn run(args: &[&str], xdg: &std::path::Path, stdin: Option<&str>) -> (String, bool) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rediff"));
    cmd.args(args)
        .env("XDG_CONFIG_HOME", xdg)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rediff");
    if let Some(s) = stdin {
        child
            .stdin
            .take()
            .expect("child stdin")
            .write_all(s.as_bytes())
            .expect("write stdin");
    }
    let out = child.wait_with_output().expect("wait rediff");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

#[test]
fn pager_reads_stdin_and_writes_ansi() {
    let cfg = config_home("dark");
    let (stdout, ok) = run(&["pager"], cfg.path(), Some(MODIFY_DIFF));
    assert!(ok, "pager exits cleanly");
    assert!(!stdout.is_empty(), "pager produced output");
    assert!(stdout.contains("\x1b[38;2;"), "pager emits truecolor ANSI");
    assert!(
        stdout.contains("src/lib.rs"),
        "pager rendered the file header"
    );
}

#[test]
fn pager_with_explicit_theme_flag() {
    // Exercises filter_theme's explicit-flag branch (flag wins over config).
    let cfg = config_home("dark");
    let (stdout, ok) = run(
        &["pager", "--theme", "light"],
        cfg.path(),
        Some(MODIFY_DIFF),
    );
    assert!(ok, "pager --theme exits cleanly");
    assert!(stdout.contains("\x1b[38;2;"), "themed ANSI emitted");
}

#[test]
fn diff_piped_prints_unified_text() {
    // stdout is a pipe (not a TTY) → main takes the synchronous git::load +
    // render::to_unified_string path instead of launching the TUI.
    let f = GitFixture::new();
    f.write("a.rs", "fn main() {\n    one();\n}\n");
    f.commit_all("init");
    f.write("a.rs", "fn main() {\n    one();\n    two();\n}\n");

    let cfg = config_home("dark");
    let repo = f.path().to_str().expect("utf8 repo path");
    let (stdout, ok) = run(&["diff", "-C", repo], cfg.path(), None);
    assert!(ok, "diff exits cleanly");
    assert!(stdout.contains("a.rs"), "unified diff names the file");
    assert!(
        stdout.contains("two()"),
        "unified diff shows the added line"
    );
}

#[test]
fn external_renders_two_files() {
    // GIT_EXTERNAL_DIFF per-file renderer: path old-file old-hex old-mode
    // new-file new-hex new-mode.
    let dir = tempfile::tempdir().expect("tempdir");
    let old = dir.path().join("old.rs");
    let new = dir.path().join("new.rs");
    std::fs::write(&old, "fn main() {\n    let x = 1;\n}\n").expect("write old");
    std::fs::write(&new, "fn main() {\n    let x = 2;\n}\n").expect("write new");

    let cfg = config_home("dark");
    let (stdout, ok) = run(
        &[
            "external",
            "src/lib.rs",
            old.to_str().unwrap(),
            "oldhex",
            "100644",
            new.to_str().unwrap(),
            "newhex",
            "100644",
        ],
        cfg.path(),
        None,
    );
    assert!(ok, "external exits cleanly");
    assert!(stdout.contains("src/lib.rs"), "external names the file");
    assert!(stdout.contains("\x1b[38;2;"), "external emits ANSI");
}

// ---- review loop ----------------------------------------------------------

/// Run `rediff <args>` in `repo`, returning (stdout, stderr, success).
fn rediff(repo: &std::path::Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_rediff"))
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run rediff");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// Append a record to the review log, standing in for the capture surface that
/// has not been built yet. Legitimate precisely because the log's format *is* the
/// contract between surfaces.
fn append_thread(repo: &std::path::Path, id: &str, body: &str) {
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(repo.join("rediff.jsonl"))
        .expect("open log");
    writeln!(
        f,
        r#"{{"t":"thread","id":"{id}","body":"{body}","at":"2026-07-27T00:00:00+00:00"}}"#
    )
    .expect("append");
}

#[test]
fn review_loop_request_then_feedback() {
    let repo = common::GitFixture::new();
    repo.write("a.rs", "fn main() {}\n");
    repo.commit_all("base");
    repo.write("a.rs", "fn main() { let x = 1; }\n");

    // No review yet.
    let (out, _, ok) = rediff(repo.path(), &["review-status"]);
    assert!(ok && out.contains("no review open"), "{out}");

    // Request opens one, and hints about .gitignore on stderr only.
    let (out, err, ok) = rediff(repo.path(), &["request", "--label", "agent-a"]);
    assert!(ok, "request failed: {err}");
    assert!(
        out.contains("opened review") && out.contains("round 1"),
        "{out}"
    );
    assert!(err.contains(".gitignore"), "hint goes to stderr: {err}");
    assert!(repo.path().join("rediff.jsonl").exists());

    // Status reflects it.
    let (out, _, ok) = rediff(repo.path(), &["review-status", "--json"]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).expect("status is JSON");
    assert_eq!(v["review"]["target"], "worktree");
    assert_eq!(v["review"]["label"], "agent-a");
    assert_eq!(v["review"]["round"], 1);

    // A human comments (via the log, since no capture surface exists yet).
    append_thread(repo.path(), "t1", "prefer a named constant");

    // The agent drains it.
    let (out, _, ok) = rediff(repo.path(), &["feedback"]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).expect("feedback is JSON");
    assert_eq!(v["threads"][0]["id"], "t1");
    assert_eq!(v["threads"][0]["body"], "prefer a named constant");
    assert_eq!(v["threads"][0]["delivered"], false);

    // Draining again delivers nothing.
    let (out, _, ok) = rediff(repo.path(), &["feedback"]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["threads"].as_array().unwrap().len(), 0, "drain-once");

    // But a replay still shows it, flagged.
    let (out, _, ok) = rediff(repo.path(), &["feedback", "--all"]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["threads"][0]["id"], "t1");
    assert_eq!(v["threads"][0]["delivered"], true);

    // A second request attaches rather than starting over, and re-emits no hint.
    let (out, err, ok) = rediff(repo.path(), &["request"]);
    assert!(ok, "{err}");
    assert!(out.contains("attached to review"), "{out}");
    assert!(!err.contains(".gitignore"), "hint is not repeated: {err}");
}

#[test]
fn request_refuses_path_filters_and_reports_nothing_to_review() {
    let repo = common::GitFixture::new();
    repo.write("a.rs", "fn main() {}\n");
    repo.commit_all("base");

    // A clean tree has nothing to review, and opens no log.
    let (out, _, ok) = rediff(repo.path(), &["request"]);
    assert!(ok, "an empty changeset is not an error");
    assert!(out.contains("nothing to review"), "{out}");
    assert!(!repo.path().join("rediff.jsonl").exists());

    repo.write("a.rs", "fn main() { let x = 1; }\n");
    let (_, err, ok) = rediff(repo.path(), &["request", "src/"]);
    assert!(!ok, "path filters are refused");
    assert!(err.contains("no path filters"), "{err}");
}

#[test]
fn the_review_log_never_appears_in_a_diff() {
    let repo = common::GitFixture::new();
    repo.write("a.rs", "fn main() {}\n");
    repo.commit_all("base");
    repo.write("a.rs", "fn main() { let x = 1; }\n");
    rediff(repo.path(), &["request"]);
    assert!(repo.path().join("rediff.jsonl").exists());

    // `diff` piped is the non-TUI dump path.
    let (out, _, ok) = rediff(repo.path(), &["diff"]);
    assert!(ok);
    assert!(out.contains("a.rs"), "the real change is shown");
    assert!(
        !out.contains("rediff.jsonl"),
        "the reviewer never reviews its own log:\n{out}"
    );
}

#[test]
fn request_accepts_a_single_commit_as_its_target() {
    // `show:<rev>` was a target the codec supported but no invocation could
    // produce: `request HEAD` was classified as a path filter and rejected.
    let repo = common::GitFixture::new();
    repo.write("a.rs", "fn main() {}\n");
    repo.commit_all("base");
    repo.write("a.rs", "fn main() { let x = 1; }\n");
    repo.commit_all("second");

    let (out, err, ok) = rediff(repo.path(), &["request", "HEAD"]);
    assert!(ok, "a bare rev must be a target, not a filter: {err}");
    assert!(out.contains("opened review"), "{out}");

    let (out, _, ok) = rediff(repo.path(), &["review-status", "--json"]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["review"]["target"], "show:HEAD");
}

#[test]
fn a_closed_pipe_does_not_consume_the_feedback() {
    // Marking delivered before the write lands would destroy the comments.
    let repo = common::GitFixture::new();
    repo.write("a.rs", "fn main() {}\n");
    repo.commit_all("base");
    repo.write("a.rs", "fn main() { let x = 1; }\n");
    rediff(repo.path(), &["request"]);
    append_thread(repo.path(), "t1", "still here");

    // `head -0` closes the pipe immediately.
    let ok = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "{} feedback | head -0",
            env!("CARGO_BIN_EXE_rediff")
        ))
        .current_dir(repo.path())
        .status()
        .expect("run pipeline");
    let _ = ok;

    // The comment must survive for the next real drain.
    let (out, _, ok) = rediff(repo.path(), &["feedback"]);
    assert!(ok);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["threads"][0]["id"], "t1",
        "a closed pipe must not consume the feedback:\n{out}"
    );
}
