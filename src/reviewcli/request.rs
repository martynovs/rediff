//! `rediff request` — open a review over the current changes and exit.
//!
//! This is how an agent says "I have finished; please look." It is deliberately
//! the *whole* command: no TUI, no server, no capture surface.
//!
//! The body takes its log and changeset as arguments rather than discovering
//! them, so every branch below is reachable from a unit test with the `testutil`
//! fixtures. The shell around it (`run`) does only repo discovery and the load —
//! the two things that genuinely need a filesystem.

use std::io;

use crate::model::Changeset;
use crate::review::{open_review, open_round, Log, Opened};

/// A review was opened or attached to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    /// The review's short identifier.
    pub review: String,
    /// The round this request opened, or the one already current.
    pub round: u32,
    /// Whether the review was started fresh, attached to, or appended after.
    pub opened: Opened,
    /// A `--label` was given but discarded, because attaching to an existing
    /// review cannot rename it. Reported so the caller can say so rather than
    /// leaving the label silently naming the wrong owner.
    pub label_ignored: bool,
}

/// What `request` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A review is open and a round is current.
    Ready(Ready),
    /// The target's changeset is empty, so nothing was opened.
    NothingToReview {
        /// The target of a review that is already open, when there is one — so
        /// the caller is not merely told "nothing to do" while feedback waits.
        open_target: Option<String>,
    },
}

/// Why a request could not be honoured.
#[derive(Debug)]
pub enum RequestError {
    /// A review over a different target holds feedback nobody has read.
    TargetMismatch {
        /// What this invocation asked for.
        requested: String,
        /// What the open review actually covers.
        open: String,
    },
    /// Path filters are not part of a review's scope.
    FiltersUnsupported,
    /// The log holds lines this build cannot read, so its state is unknown.
    UnreadableLog {
        /// How many lines failed to parse.
        lines: u64,
    },
    /// The log could not be read or appended to.
    Io(io::Error),
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::TargetMismatch { requested, open } => write!(
                f,
                "a review of `{open}` is open and has feedback you have not read; \
                 requested `{requested}`. Drain it with `rediff feedback` first, \
                 or delete rediff.jsonl to start over."
            ),
            RequestError::FiltersUnsupported => write!(
                f,
                "`rediff request` takes no path filters: a review covers everything \
                 that changed under its target"
            ),
            RequestError::UnreadableLog { lines } => write!(
                f,
                "rediff.jsonl has {lines} line(s) this build cannot read, so it is \
                 not safe to add to. It may have been written by a newer rediff, or \
                 truncated by a crash; `rediff feedback --all` may still recover \
                 what is readable."
            ),
            RequestError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RequestError {}

impl From<io::Error> for RequestError {
    fn from(e: io::Error) -> Self {
        RequestError::Io(e)
    }
}

/// A short identifier for a new review.
///
/// Distinguishable within one worktree's history is all this needs to be, so the
/// process id and the current second are plenty — no randomness required.
#[must_use]
pub fn new_review_id() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut seed = [0u8; 12];
    seed[..4].copy_from_slice(&std::process::id().to_le_bytes());
    seed[4..].copy_from_slice(&secs.to_le_bytes());
    let mixed = crate::review::content_hash(&seed);
    format!("r{:06x}", mixed & 0x00ff_ffff)
}

/// Open a review over `cs`, or report why not.
///
/// The order of checks matters. A target mismatch is only an error while the open
/// review holds *undelivered* feedback — refusing unconditionally would make every
/// change of target a one-way door, since the store is otherwise happy to start a
/// fresh review. And the empty-changeset case names any open review, so an agent
/// that committed its fixes is not merely told there is nothing to do.
///
/// # Errors
/// See [`RequestError`].
pub fn request(
    log: &Log,
    cs: &Changeset,
    target: &str,
    label: Option<&str>,
) -> Result<Outcome, RequestError> {
    let st = log.state()?;

    // A log with no readable `open` but unreadable lines is not an empty log: the
    // header may be torn, or written by a newer build. Appending a round to it
    // would report success for a review with no identity at all.
    if st.open.is_none() && st.unparsed > 0 {
        return Err(RequestError::UnreadableLog { lines: st.unparsed });
    }

    if let Some(open) = st.open.as_ref() {
        if open.target != target && !st.safe_to_replace() {
            return Err(RequestError::TargetMismatch {
                requested: target.to_string(),
                open: open.target.clone(),
            });
        }
    }

    if cs.files.is_empty() {
        return Ok(Outcome::NothingToReview {
            open_target: st.open.map(|o| o.target),
        });
    }

    // Only open a review when there is a new one to open. A review over the same
    // target is *continued*, whatever its delivery state — rounds are the iteration
    // counter within a review, so calling `open_review` here would truncate the log
    // the moment the previous round was drained and restart the count at 1, losing
    // the history of a loop that is still going.
    let same_target = st.open.as_ref().is_some_and(|o| o.target == target);
    let (opened, st) = if same_target {
        (Opened::Attached, st)
    } else {
        open_review(log, &new_review_id(), target, label, false)?
    };
    // Never frozen: no target is reliably immutable (`HEAD` moves under commit and
    // amend), and `changed_since` already suppresses a round when nothing moved.
    let round = open_round(log, &st, cs, false)?;
    Ok(Outcome::Ready(Ready {
        review: st.open.map(|o| o.review).unwrap_or_default(),
        round,
        opened,
        label_ignored: same_target && label.is_some(),
    }))
}

/// Render an outcome for a human, as `request` writes it to stdout.
#[must_use]
pub fn render(outcome: &Outcome, log: &Log) -> String {
    match outcome {
        Outcome::Ready(r) => {
            let verb = match r.opened {
                Opened::Fresh => "opened",
                Opened::Attached => "attached to",
                Opened::Kept => "continued",
            };
            format!(
                "{verb} review {} · round {} · {}\n",
                r.review,
                r.round,
                log.path().display()
            )
        }
        Outcome::NothingToReview { open_target: None } => {
            "nothing to review: no changes under this target\n".to_string()
        }
        Outcome::NothingToReview {
            open_target: Some(t),
        } => {
            format!("nothing to review: no changes under this target (a review of `{t}` is open)\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::{drain, Record, Thread};
    use crate::testutil::{changeset, diff_file, review_log};

    fn cs_one() -> Changeset {
        changeset(vec![diff_file("a.rs", Some("one\n"))])
    }

    fn cs_two() -> Changeset {
        changeset(vec![diff_file("a.rs", Some("two\n"))])
    }

    fn empty() -> Changeset {
        changeset(vec![])
    }

    fn thread(id: &str) -> Record {
        Record::Thread(Thread {
            id: id.into(),
            anchor: None,
            body: "look at this".into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: crate::review::now(),
        })
    }

    fn ready(o: Outcome) -> Ready {
        match o {
            Outcome::Ready(r) => r,
            other @ Outcome::NothingToReview { .. } => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn opens_a_review_and_a_first_round() {
        let (_d, log) = review_log();
        let r = ready(request(&log, &cs_one(), "worktree", Some("agent")).unwrap());
        assert_eq!(r.opened, Opened::Fresh);
        assert_eq!(r.round, 1);
        assert!(r.review.starts_with('r'), "id: {}", r.review);

        let st = log.state().unwrap();
        assert_eq!(st.open.as_ref().unwrap().target, "worktree");
        assert_eq!(st.open.unwrap().label.as_deref(), Some("agent"));
        assert_eq!(st.rounds.len(), 1);
    }

    #[test]
    fn a_second_request_attaches_and_keeps_the_id() {
        let (_d, log) = review_log();
        let first = ready(request(&log, &cs_one(), "worktree", None).unwrap());
        log.append(&thread("t1")).unwrap();

        let second = ready(request(&log, &cs_one(), "worktree", None).unwrap());
        assert_eq!(second.opened, Opened::Attached);
        assert_eq!(second.review, first.review, "same review, same id");
        assert_eq!(second.round, 1, "nothing moved, so no new round");
    }

    #[test]
    fn a_moving_target_still_opens_later_rounds() {
        // The regression a `frozen` flag would have caused: a range ending at HEAD
        // is not immutable, and freezing it reports round 1 forever.
        let (_d, log) = review_log();
        assert_eq!(
            ready(request(&log, &cs_one(), "review:main..HEAD", None).unwrap()).round,
            1
        );
        drain(&log, &log.state().unwrap(), &cs_one()).unwrap();

        let second = ready(request(&log, &cs_two(), "review:main..HEAD", None).unwrap());
        assert_eq!(second.round, 2, "the changeset moved, so a new round opens");
    }

    #[test]
    fn an_unchanged_changeset_does_not_open_a_new_round() {
        let (_d, log) = review_log();
        request(&log, &cs_one(), "worktree", None).unwrap();
        let again = ready(request(&log, &cs_one(), "worktree", None).unwrap());
        assert_eq!(again.round, 1);
        assert_eq!(log.state().unwrap().rounds.len(), 1);
    }

    #[test]
    fn an_empty_changeset_opens_nothing() {
        let (_d, log) = review_log();
        let out = request(&log, &empty(), "worktree", None).unwrap();
        assert_eq!(out, Outcome::NothingToReview { open_target: None });
        assert!(!log.exists(), "not even a log is created");
    }

    #[test]
    fn an_empty_changeset_names_an_open_review() {
        let (_d, log) = review_log();
        request(&log, &cs_one(), "worktree", None).unwrap();
        log.append(&thread("t1")).unwrap();

        let out = request(&log, &empty(), "worktree", None).unwrap();
        assert_eq!(
            out,
            Outcome::NothingToReview {
                open_target: Some("worktree".into())
            },
            "an agent that committed its fixes must not just be told there is nothing to do"
        );
    }

    #[test]
    fn a_different_target_is_refused_while_feedback_is_pending() {
        let (_d, log) = review_log();
        request(&log, &cs_one(), "worktree", None).unwrap();
        log.append(&thread("t1")).unwrap();

        let err = request(&log, &cs_one(), "show:HEAD~3", None).unwrap_err();
        match &err {
            RequestError::TargetMismatch { requested, open } => {
                assert_eq!(requested, "show:HEAD~3");
                assert_eq!(open, "worktree");
            }
            other => panic!("expected TargetMismatch, got {other:?}"),
        }
        assert!(
            err.to_string().contains("rediff feedback"),
            "names the way out"
        );
        assert_eq!(
            log.state().unwrap().open.unwrap().target,
            "worktree",
            "and nothing was recorded"
        );
    }

    #[test]
    fn a_different_target_starts_fresh_once_delivered() {
        // Refusing here too would make every change of target a one-way door.
        let (_d, log) = review_log();
        request(&log, &cs_one(), "worktree", None).unwrap();
        log.append(&thread("t1")).unwrap();
        drain(&log, &log.state().unwrap(), &cs_one()).unwrap();

        let out = ready(request(&log, &cs_one(), "worktree:main", None).unwrap());
        assert_eq!(out.opened, Opened::Fresh);
        assert_eq!(out.round, 1);
        assert_eq!(log.state().unwrap().open.unwrap().target, "worktree:main");
    }

    #[test]
    fn the_same_target_attaches_rather_than_failing() {
        let (_d, log) = review_log();
        request(&log, &cs_one(), "worktree:main", None).unwrap();
        log.append(&thread("t1")).unwrap();
        let out = ready(request(&log, &cs_one(), "worktree:main", None).unwrap());
        assert_eq!(out.opened, Opened::Attached);
    }

    #[test]
    fn review_ids_look_like_ids() {
        let id = new_review_id();
        assert!(id.starts_with('r') && id.len() == 7, "id: {id}");
        assert!(
            id.chars().skip(1).all(|c| c.is_ascii_hexdigit()),
            "id: {id}"
        );
    }

    #[test]
    fn render_covers_every_outcome() {
        let (_d, log) = review_log();
        let r = |opened| {
            render(
                &Outcome::Ready(Ready {
                    review: "rabc123".into(),
                    round: 2,
                    opened,
                    label_ignored: false,
                }),
                &log,
            )
        };
        assert!(r(Opened::Fresh).contains("opened review rabc123 · round 2"));
        assert!(r(Opened::Attached).contains("attached to review"));
        assert!(r(Opened::Kept).contains("continued review"));
        assert!(r(Opened::Fresh).contains("rediff.jsonl"), "names the log");

        assert!(
            render(&Outcome::NothingToReview { open_target: None }, &log)
                .contains("nothing to review")
        );
        let with_open = render(
            &Outcome::NothingToReview {
                open_target: Some("worktree".into()),
            },
            &log,
        );
        assert!(with_open.contains("a review of `worktree` is open"));
    }

    #[test]
    fn error_display_covers_every_variant() {
        assert!(RequestError::FiltersUnsupported
            .to_string()
            .contains("no path filters"));
        assert!(RequestError::UnreadableLog { lines: 3 }
            .to_string()
            .contains("3 line(s)"));
        assert!(RequestError::Io(io::Error::other("boom"))
            .to_string()
            .contains("boom"));
        let e: RequestError = io::Error::other("via From").into();
        assert!(matches!(e, RequestError::Io(_)));
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;
    use crate::testutil::{changeset, diff_file, review_log};

    fn cs() -> Changeset {
        changeset(vec![diff_file("a.rs", Some("one\n"))])
    }

    #[test]
    fn a_log_with_an_unreadable_header_is_refused_not_appended_to() {
        let (_d, log) = review_log();
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(log.path()).unwrap();
            writeln!(f, "{{\"t\":\"open\",TRUNCATED").unwrap();
        }
        let before = std::fs::read_to_string(log.path()).unwrap();

        let err = request(&log, &cs(), "worktree", None).unwrap_err();
        assert!(
            matches!(err, RequestError::UnreadableLog { lines: 1 }),
            "{err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap(),
            before,
            "nothing was appended to a log we cannot read"
        );
    }

    #[test]
    fn a_label_given_while_attaching_is_reported_not_silently_dropped() {
        let (_d, log) = review_log();
        request(&log, &cs(), "worktree", Some("agent-a")).unwrap();

        let out = request(&log, &cs(), "worktree", Some("agent-b")).unwrap();
        let Outcome::Ready(r) = out else {
            panic!("expected Ready")
        };
        assert_eq!(r.opened, Opened::Attached);
        assert!(r.label_ignored, "attaching cannot rename the review");

        // And the recorded label is still the original owner's.
        let st = log.state().unwrap();
        assert_eq!(st.open.unwrap().label.as_deref(), Some("agent-a"));
    }

    #[test]
    fn no_label_while_attaching_reports_nothing() {
        let (_d, log) = review_log();
        request(&log, &cs(), "worktree", Some("agent-a")).unwrap();
        let Outcome::Ready(r) = request(&log, &cs(), "worktree", None).unwrap() else {
            panic!("expected Ready")
        };
        assert!(!r.label_ignored);
    }
}
