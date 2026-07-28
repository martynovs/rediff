//! End-to-end TUI test driven through a real pseudo-terminal.
//!
//! Spawning the binary on a PTY makes `stdout().is_terminal()` true in the
//! child, so `main` launches the interactive viewer. That exercises the
//! terminal lifecycle and event loop (`run`, `setup_terminal`, `event_loop`,
//! `read_event`, `redraw_if_dirty`, `restore_terminal`) plus the theme-picker
//! commit path (`persist_theme` → `Config::save_theme`) — none of which run
//! under the headless `TestBackend`. cargo-llvm-cov instruments the child, so
//! its coverage is captured here.
#![expect(
    clippy::panic,
    clippy::indexing_slicing,
    clippy::let_underscore_must_use,
    reason = "PTY integration harness: helpers panic on timeout, slice known-shaped pty output, and discard best-effort writes/kills to the master"
)]

mod common;

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::GitFixture;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// Launch the viewer on a PTY with `XDG_CONFIG_HOME` set to `xdg`, drive the
/// theme picker to a commit (`t`, navigate, Enter), then quit (`q`). Returns
/// whether the child exited cleanly. Robust against hangs: the child is killed
/// and the test fails if it does not exit.
fn drive_theme_commit(xdg: &Path) -> bool {
    // A repo with an uncommitted change so the viewer has a file to render.
    let f = GitFixture::new();
    f.write("a.rs", "fn main() {\n    one();\n}\n");
    f.commit_all("init");
    f.write("a.rs", "fn main() {\n    one();\n    two();\n}\n");

    // t = open theme picker; j/l = navigate (live-preview); Enter = commit the
    // theme (persists it); q = quit the now-normal view.
    drive(f.path(), Some(xdg), &[b"t", b"j", b"l", b"\r", b"q"])
}

/// Launch the viewer on a PTY over `repo`, send `keys` with a pause between
/// each, and wait for a clean exit. Returns whether the child exited cleanly;
/// panics (after killing the child) if it hangs, so the suite never stalls.
fn drive(repo: &Path, xdg: Option<&Path>, keys: &[&[u8]]) -> bool {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_rediff"));
    // Forward the parent environment (notably LLVM_PROFILE_FILE, so the child's
    // coverage is captured), then pin the config dir.
    for (k, v) in std::env::vars() {
        cmd.env(k, v);
    }
    if let Some(xdg) = xdg {
        cmd.env("XDG_CONFIG_HOME", xdg);
    }
    cmd.arg("diff");
    cmd.arg("-C");
    cmd.arg(repo);

    let mut child = pair.slave.spawn_command(cmd).expect("spawn on pty");
    // Drop the slave so the master sees EOF once the child exits.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let mut writer = pair.master.take_writer().expect("take writer");

    // Drain the master in a background thread so the child never blocks on a
    // full output buffer; collect bytes for the first-frame check.
    let buf = Arc::new(Mutex::new(Vec::new()));
    let buf_thread = Arc::clone(&buf);
    let reader_thread = std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf_thread
                    .lock()
                    .expect("buf lock")
                    .extend_from_slice(&chunk[..n]),
            }
        }
    });

    // Wait for the first rendered frame (any output) before sending keys.
    wait_until(Duration::from_secs(10), || {
        !buf.lock().expect("buf lock").is_empty()
    })
    .expect("the TUI rendered an initial frame");

    for key in keys {
        std::thread::sleep(Duration::from_millis(250));
        writer.write_all(key).expect("write key");
        writer.flush().expect("flush key");
    }

    // Wait for the child to exit; kill it if it hangs so the suite never stalls.
    let exited = wait_until(Duration::from_secs(15), || {
        matches!(child.try_wait(), Ok(Some(_)))
    });
    if exited.is_err() {
        let _ = child.kill();
        panic!("the TUI did not exit after the quit key");
    }
    let success = matches!(child.try_wait(), Ok(Some(s)) if s.success());

    // Releasing the writer/master lets the reader thread observe EOF and finish.
    drop(writer);
    drop(pair.master);
    let _ = reader_thread.join();
    success
}

/// Poll the master until the first frame, commit a theme, and verify it was
/// written to the config file under the pinned `XDG_CONFIG_HOME`.
#[test]
fn tui_commits_theme_and_persists_it() {
    let xdg = tempfile::tempdir().expect("config tempdir");
    let ok = drive_theme_commit(xdg.path());
    assert!(ok, "the TUI exited cleanly");

    let cfg = xdg.path().join("rediff").join("config.toml");
    let contents = std::fs::read_to_string(&cfg)
        .expect("the theme commit wrote the config file under XDG_CONFIG_HOME");
    assert!(
        contents.contains("theme"),
        "the persisted config records a theme: {contents:?}"
    );
}

/// When the config can't be written, the theme commit must be non-fatal: the
/// in-session theme already applied, so `persist_theme` only flashes a warning
/// and the app keeps running (exercising its save-failure arm). We force the
/// failure by pointing `XDG_CONFIG_HOME` at a regular file, so creating the
/// `rediff/` config directory underneath it fails.
#[test]
fn tui_theme_commit_survives_a_save_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let not_a_dir = dir.path().join("xdg-is-a-file");
    std::fs::write(&not_a_dir, b"this is a file, not a directory\n").expect("write file");

    let ok = drive_theme_commit(&not_a_dir);
    assert!(
        ok,
        "a save failure is non-fatal; the TUI still exits cleanly"
    );
    // The config "directory" is really a file, so nothing was persisted.
    assert!(
        not_a_dir.is_file(),
        "the bogus XDG path stays a file (no config was written)"
    );
}

/// The whole point of the review loop: a comment typed in the TUI comes back
/// out of `rediff feedback`, anchored to the line it was left on.
///
/// This is the only test that proves the two halves meet. Every other test
/// covers one side of the log — this one writes through the real key router,
/// terminal and all, and reads through the real agent-facing command.
#[test]
fn a_comment_left_in_the_tui_is_delivered_by_feedback() {
    let f = GitFixture::new();
    f.write("a.rs", "fn main() {\n    one();\n}\n");
    f.commit_all("init");
    f.write("a.rs", "fn main() {\n    one();\n    two();\n}\n");

    // ] lands on the first hunk, j steps onto a diff line, a opens the comment
    // input; then the text, Enter to record, q to quit.
    let keys: [&[u8]; 6] = [b"]", b"j", b"a", TEXT_STR.as_bytes(), b"\r", b"q"];
    assert!(drive(f.path(), None, &keys), "the TUI exited cleanly");

    let log = f.path().join("rediff.jsonl");
    assert!(
        log.is_file(),
        "the comment opened a review log at the worktree root"
    );

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rediff"))
        .args(["feedback"])
        .current_dir(f.path())
        .output()
        .expect("run rediff feedback");
    assert!(out.status.success(), "feedback exited cleanly");
    let json = String::from_utf8_lossy(&out.stdout);

    let doc: serde_json::Value = serde_json::from_str(&json).expect("feedback emits JSON");
    let threads = doc["threads"].as_array().expect("a threads array");
    assert_eq!(threads.len(), 1, "exactly the comment we left: {json}");
    let t = &threads[0];
    assert_eq!(
        t["body"].as_str(),
        Some(TEXT_STR),
        "the body arrives verbatim: {json}"
    );
    let anchor = &t["anchor"];
    assert_eq!(anchor["path"].as_str(), Some("a.rs"), "{json}");
    assert!(
        anchor["quote"].as_str().is_some_and(|q| !q.is_empty()),
        "the anchor quotes the line it was left on: {json}"
    );
    assert!(
        anchor["line"].as_u64().is_some(),
        "and names its line number: {json}"
    );
}

/// The comment typed into the TUI. Written in one go: crossterm turns the bytes
/// into one key event per character, which is what the input sees either way.
const TEXT_STR: &str = "needs a test";

/// Poll `cond` until it is true or `timeout` elapses.
fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> Result<(), ()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if cond() {
        Ok(())
    } else {
        Err(())
    }
}
