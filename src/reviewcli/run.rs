//! The I/O shells around the three command bodies.
//!
//! Everything that needs a filesystem lives here — repository discovery, the
//! changeset load, writing to stdout and stderr — and nothing else does. The
//! bodies these call are pure over their inputs, which is what makes their
//! branches testable without spawning a process.

use std::io::{self, Write};
use std::path::Path;

use anyhow::Context;

use super::feedback::{collect as collect_feedback, mark_delivered, render as render_feedback};
use super::request::{render as render_request, request, Outcome, RequestError};
use super::status::{collect as collect_status, render_human, render_json};
use super::target;
use crate::git::{self, LoadRequest};
use crate::model::Changeset;
use crate::review::{log_path, Log, Opened};

/// What to tell a human the first time a review log appears in their worktree.
const IGNORE_HINT: &str =
    "note: rediff writes its review log to rediff.jsonl — add it to .gitignore if you don't want it tracked";

/// Discover the repository and resolve its review log.
///
/// The log belongs to the **worktree root**, not the directory the command was
/// invoked from: writing it into a subdirectory would leave a file that
/// `drop_review_log` (root-level only) does not filter back out, so it would show
/// up as a change in every later review.
fn open_log(repo_dir: &Path) -> anyhow::Result<Log> {
    let repo = gix::discover(repo_dir)
        .with_context(|| format!("not a git repository: {}", repo_dir.display()))?;
    let path = log_path(&repo).context("this repository has no worktree to hold a review log")?;
    Ok(Log::new(path))
}

/// Write to stdout without panicking on a closed pipe.
///
/// `print!` panics on a broken pipe, which turns `rediff review-status | head`
/// into an abort with exit 101. Every command's output goes through here.
fn write_stdout(text: &str) -> io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())?;
    out.flush()
}

/// A changeset with no files, for the case where the target cannot be loaded.
fn no_changeset() -> Changeset {
    Changeset {
        source: String::new(),
        files: Vec::new(),
    }
}

/// `rediff request`.
///
/// # Errors
/// Fails when the repository cannot be opened, path filters were given, the target
/// cannot be encoded, the changeset cannot be loaded, or the request is refused.
pub fn run_request(
    repo_dir: &Path,
    req: &LoadRequest,
    filters: &[String],
    label: Option<&str>,
) -> anyhow::Result<()> {
    if !filters.is_empty() {
        return Err(RequestError::FiltersUnsupported.into());
    }
    let log = open_log(repo_dir)?;
    let target = target::encode(req)?;
    let cs = git::load(repo_dir, req)?;

    let outcome = request(&log, &cs, &target, label)?;
    // The hint rides on a *new* review, which is the only signal the store gives:
    // `open_review` discards `append`'s created flag, and a fresh review recreates
    // the file anyway. stderr, so machine-readable stdout stays clean.
    if matches!(&outcome, Outcome::Ready(r) if r.opened == Opened::Fresh) {
        eprintln!("{IGNORE_HINT}");
    }
    if matches!(&outcome, Outcome::Ready(r) if r.label_ignored) {
        eprintln!("note: --label was ignored; attaching to an existing review cannot rename it");
    }
    write_stdout(&render_request(&outcome, &log))?;
    Ok(())
}

/// `rediff review-status`.
///
/// # Errors
/// Fails when the repository cannot be opened or the log cannot be read.
pub fn run_status(repo_dir: &Path, json: bool) -> anyhow::Result<()> {
    let log = open_log(repo_dir)?;
    let status = collect_status(&log.state()?, &log);
    if json {
        write_stdout(&render_json(&status)?)?;
    } else {
        write_stdout(&render_human(&status))?;
    }
    Ok(())
}

/// `rediff feedback`.
///
/// # Errors
/// Fails when the repository cannot be opened, the log cannot be read, or the
/// recorded target cannot be **parsed**. A target that parses but no longer
/// *resolves* is not an error — see below.
pub fn run_feedback(repo_dir: &Path, replay_all: bool) -> anyhow::Result<()> {
    let log = open_log(repo_dir)?;
    let st = log.state()?;

    let (cs, unresolvable) = match st.open.as_ref() {
        None => (no_changeset(), false),
        Some(open) => {
            // A target that cannot be parsed is a corrupt record, and guessing at
            // it would resolve anchors against the wrong thing.
            let req = target::parse(&open.target)
                .with_context(|| format!("review target `{}`", open.target))?;
            match git::load(repo_dir, &req) {
                Ok(cs) => (cs, false),
                // A target that parses but no longer resolves (a deleted branch)
                // must not be fatal: `--all` needs a changeset too, and a request
                // for another target is refused while feedback is pending, so
                // failing here would leave the human's comments unreachable short
                // of deleting the log. Deliver them as detached instead.
                Err(e) => {
                    eprintln!(
                        "warning: review target `{}` no longer resolves ({e}); \
                         reporting every comment as detached",
                        open.target
                    );
                    (no_changeset(), true)
                }
            }
        }
    };

    let doc = collect_feedback(&st, &cs, replay_all, unresolvable);
    let text = render_feedback(&doc)?;

    // Write first, record delivery second. `print!` panics on a broken pipe, so
    // marking the feedback delivered before it lands would let `rediff feedback |
    // head -1` destroy a human's comments and abort — recoverable only via
    // `--all`. Writing through `write_all` also turns that pipe into an error
    // rather than a panic.
    write_stdout(&text)?;
    if !replay_all {
        mark_delivered(&log, &st, &doc)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{run_git, scratch_repo};

    #[test]
    fn open_log_resolves_to_the_worktree_root_from_a_subdirectory() {
        let repo = scratch_repo();
        std::fs::create_dir_all(repo.path().join("src")).unwrap();
        let from_root = open_log(repo.path()).unwrap();
        let from_sub = open_log(&repo.path().join("src")).unwrap();
        assert_eq!(
            from_root.path(),
            from_sub.path(),
            "the log belongs to the worktree, not the invocation directory"
        );
    }

    #[test]
    fn open_log_rejects_a_non_repository() {
        let dir = tempfile::tempdir().unwrap();
        let err = open_log(dir.path()).unwrap_err();
        assert!(err.to_string().contains("not a git repository"), "{err}");
    }

    #[test]
    fn open_log_rejects_a_bare_repository() {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q", "--bare"]);
        let err = open_log(dir.path()).unwrap_err();
        assert!(err.to_string().contains("no worktree"), "{err}");
    }

    #[test]
    fn request_refuses_path_filters() {
        let repo = scratch_repo();
        let err = run_request(
            repo.path(),
            &LoadRequest::Staged,
            &["src/".to_string()],
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no path filters"), "{err}");
    }

    #[test]
    fn status_and_feedback_work_before_any_review_exists() {
        let repo = scratch_repo();
        run_status(repo.path(), false).expect("human status");
        run_status(repo.path(), true).expect("json status");
        run_feedback(repo.path(), false).expect("drain");
        run_feedback(repo.path(), true).expect("replay");
        assert!(
            !repo.path().join("rediff.jsonl").exists(),
            "reading state must not create a log"
        );
    }

    #[test]
    fn no_changeset_is_empty() {
        assert!(no_changeset().files.is_empty());
    }

    /// Write a log holding one review over `target` plus a comment on it.
    fn log_with_target(repo: &Path, target: &str) {
        let log = Log::at_worktree(repo);
        log.append(&crate::review::Record::Open {
            review: "r1".into(),
            target: target.into(),
            label: None,
            at: crate::review::now(),
        })
        .unwrap();
        log.append(&crate::review::Record::Thread(crate::review::Thread {
            id: "t1".into(),
            anchor: None,
            body: "precious".into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: crate::review::now(),
        }))
        .unwrap();
    }

    #[test]
    fn an_unparseable_target_is_a_hard_error_naming_it() {
        // A corrupt record, not a moved ref: guessing would resolve anchors
        // against the wrong thing entirely.
        let repo = scratch_repo();
        log_with_target(repo.path(), "telepathy:HEAD");
        let err = run_feedback(repo.path(), false).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("telepathy:HEAD"), "names the target: {text}");
    }

    #[test]
    fn a_target_whose_refs_are_gone_still_delivers_its_feedback() {
        // The deadlock this avoids: `--all` needs a changeset too, and a request
        // for another target is refused while feedback is pending, so failing here
        // would strand the human's comments.
        let repo = scratch_repo();
        std::fs::write(repo.path().join("a.rs"), "fn main() {}\n").unwrap();
        run_git(repo.path(), &["add", "-A"]);
        run_git(repo.path(), &["commit", "-qm", "base"]);
        log_with_target(repo.path(), "show:no-such-branch-anywhere");

        run_feedback(repo.path(), false).expect("degrades rather than failing");

        // And the comment was actually delivered, not silently swallowed.
        let log = Log::at_worktree(repo.path());
        let st = log.state().unwrap();
        assert!(st.fully_drained(), "the drain marker was recorded");
    }
}
