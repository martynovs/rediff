//! The log file itself: where it lives, how records get in, and how replaying it
//! folds into the review's current state.
//!
//! Two invariants hold everything else up:
//!
//! 1. **Append-only.** No record is ever rewritten or removed in place. An edit is
//!    a new record; a retraction is a new record. History is the file.
//! 2. **The line number is the ordinal.** Appending makes it monotonic for free, so
//!    delivery needs no separate sequence field that could disagree with the file.
//!    Ordinals count *raw* lines, so a line that fails to parse still consumes one
//!    and cannot shift the records after it.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use super::record::{Record, Submit, Thread};

/// The log's file name, at the worktree root.
pub const LOG_FILE_NAME: &str = "rediff.jsonl";

/// The log path for a worktree root.
#[must_use]
pub fn log_path_in(worktree: &Path) -> PathBuf {
    worktree.join(LOG_FILE_NAME)
}

/// The log path for an opened repository.
///
/// Resolves through the repository's *worktree* directory, so a linked worktree
/// gets its own log at its own root. Returns `None` for a bare repository, which
/// has no worktree to put one in.
#[must_use]
pub fn log_path(repo: &gix::Repository) -> Option<PathBuf> {
    repo.workdir().map(log_path_in)
}

/// A review log at a known path. Cheap to construct; the file need not exist.
#[derive(Debug, Clone)]
pub struct Log {
    path: PathBuf,
}

/// The `open` record's payload, once folded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenInfo {
    /// Short identifier for the review.
    pub review: String,
    /// What is under review.
    pub target: String,
    /// Human-facing label.
    pub label: Option<String>,
}

/// One round, once folded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundInfo {
    /// 1-based round number.
    pub n: u32,
    /// Path to content hash, as recorded.
    pub files: BTreeMap<String, u64>,
}

/// A thread after folding, with the ordinal of the record that last defined it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadState {
    /// Log line of the most recent record for this thread.
    pub ordinal: u64,
    /// The thread's current state.
    pub thread: Thread,
}

/// A review's current state, folded from one replay of the log.
///
/// Everything downstream — delivery, rounds, the serve lifecycle — reads this, so
/// there is exactly one place that interprets the log.
#[derive(Debug, Clone, Default)]
pub struct ReviewState {
    /// The most recent `open`, if the log has one.
    pub open: Option<OpenInfo>,
    /// Rounds in the order they opened.
    pub rounds: Vec<RoundInfo>,
    /// Threads folded by id, in first-appearance order.
    pub threads: IndexMap<String, ThreadState>,
    /// Submits with the ordinal each was recorded at.
    pub submits: Vec<(u64, Submit)>,
    /// Paths the user has marked reviewed, from the most recent `viewed` record.
    ///
    /// By path, not position: a changeset's file order differs between sessions,
    /// so restoring by index would mark the wrong files.
    pub viewed: Vec<String>,
    /// Ordinal delivered through, from the most recent `drained` record.
    pub drained_upto: u64,
    /// The most recent `serve`, with the ordinal it was recorded at.
    pub serve: Option<(u64, ServeRecord)>,
    /// The most recent `close` recorded after that serve.
    pub close: Option<CloseRecord>,
    /// The highest ordinal in the log, i.e. its raw line count.
    pub last_ordinal: u64,
    /// How many lines did not parse.
    ///
    /// Non-zero means this build cannot fully read the log — a line torn by a
    /// crash mid-append, or otherwise corrupt. Such a log is never replaced, since
    /// what we cannot read we cannot know to be safe to destroy. A *well-formed*
    /// record whose type this build does not know is not counted here: it folds
    /// to nothing instead (see [`Record::Unknown`]).
    pub unparsed: u64,
    /// Ordinal of the first line this build could not read, if any.
    ///
    /// A drain must not mark it delivered: the record may be feedback a newer
    /// rediff understands, and `Drained` is by ordinal, so marking past it hides
    /// it from that build forever. Counting alone is not enough — the drain
    /// needs to know *where* to stop.
    pub first_unparsed: Option<u64>,
}

/// A `serve` record's payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeRecord {
    /// Process id of the server.
    pub pid: u32,
    /// Bound loopback port.
    pub port: u16,
    /// URL the server is reachable at.
    pub url: String,
}

/// A `close` record's payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseRecord {
    /// Process id of the server that stopped, when one was serving.
    pub pid: Option<u32>,
    /// Why it stopped.
    pub reason: String,
}

impl ReviewState {
    /// Whether every thread and submit that will ever be delivered has been.
    ///
    /// Retracted threads are excluded, because delivery excludes them too: counting
    /// one as pending would mean `drain` never has anything to deliver, never writes
    /// a marker, and the review can never finish — a log wedged by the author
    /// changing their mind.
    #[must_use]
    pub fn fully_drained(&self) -> bool {
        self.pending_ordinals().next().is_none()
    }

    /// Whether this log may be truncated to start a new review.
    ///
    /// Two independent reasons to refuse: feedback nobody has consumed, and lines
    /// this build could not parse at all. The second is about *damage* — a record
    /// torn by a crash must not read as "empty, safe to delete". A readable record
    /// from a newer build does not refuse, deliberately; see [`Record::Unknown`]
    /// for the constraint that buys.
    #[must_use]
    pub fn safe_to_replace(&self) -> bool {
        self.fully_drained() && self.unparsed == 0
    }

    /// Ordinals of records that are feedback and not yet delivered.
    pub(crate) fn pending_ordinals(&self) -> impl Iterator<Item = u64> + '_ {
        let threads = self
            .threads
            .values()
            .filter(|t| !t.thread.deleted)
            .map(|t| t.ordinal);
        let submits = self.submits.iter().map(|(ord, _)| *ord);
        threads
            .chain(submits)
            .filter(move |ord| *ord > self.drained_upto)
    }
}

impl Log {
    /// A log at an explicit path.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// A log at a worktree root.
    #[must_use]
    pub fn at_worktree(worktree: &Path) -> Self {
        Self::new(log_path_in(worktree))
    }

    /// The log's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the log file exists.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Append one record as a single line.
    ///
    /// Opens `O_APPEND` and holds an exclusive advisory lock across the write, so a
    /// record written while another writer is appending is still one whole line —
    /// the TUI and a server can both append to the same log. Returns whether the
    /// file had to be created, which is how the one-time ignore hint is triggered
    /// exactly once per worktree.
    pub fn append(&self, rec: &Record) -> io::Result<bool> {
        let mut line = serde_json::to_string(rec).map_err(io::Error::other)?;
        line.push('\n');

        // Derive creation from the open itself rather than a prior `exists()`:
        // two writers racing on a fresh log would both see "absent" and both
        // report having created it, printing the one-time hint twice.
        let (mut file, created) = match OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(&self.path)
        {
            Ok(f) => (f, true),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                (OpenOptions::new().append(true).open(&self.path)?, false)
            }
            Err(e) => return Err(e),
        };
        file.lock()?;
        let write = file.write_all(line.as_bytes()).and_then(|()| file.flush());
        // Release explicitly rather than relying on the close, so a failed write
        // still unlocks before the error propagates.
        let unlock = file.unlock();
        write.and(unlock)?;
        Ok(created)
    }

    /// Every line of the log as `(ordinal, parsed)`, where a line that fails to
    /// parse yields `None` but still consumes its ordinal.
    ///
    /// Takes a **shared** lock for the read. Without one, a reader can observe a
    /// record another writer is halfway through appending; that torn line parses as
    /// `None` but still advances `last_ordinal`, so a subsequent drain would mark
    /// the completed record as delivered and it would never reach a consumer.
    ///
    /// Decodes lossily for the same reason: a read that lands mid-way through a
    /// multi-byte character must cost one unparseable line, not the whole replay.
    pub fn replay(&self) -> io::Result<Vec<(u64, Option<Record>)>> {
        let bytes = match File::open(&self.path) {
            Ok(mut f) => {
                f.lock_shared()?;
                let mut buf = Vec::new();
                let read = f.read_to_end(&mut buf);
                let unlock = f.unlock();
                read.and(unlock)?;
                buf
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        Ok(parse_lines(&bytes))
    }

    /// Truncate the log and write `rec` — but only if the log is *still* safe to
    /// replace when checked under the same exclusive lock.
    ///
    /// [`open_review`] used to read the state under a shared lock, drop it, and
    /// then truncate under its own. Anything appended in
    /// between was destroyed: an agent running `rediff request` against a log it
    /// had just seen as drained would truncate a comment a human wrote a moment
    /// later, with no error to either side. Checking inside the write lock closes
    /// that window.
    ///
    /// Returns whether the replacement happened.
    pub(crate) fn replace_if_safe(&self, rec: &Record) -> io::Result<bool> {
        let mut line = serde_json::to_string(rec).map_err(io::Error::other)?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.path)?;
        file.lock()?;
        let done = (|| -> io::Result<bool> {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            if !fold(&parse_lines(&buf)).safe_to_replace() {
                return Ok(false);
            }
            file.set_len(0)?;
            file.seek(io::SeekFrom::Start(0))?;
            file.write_all(line.as_bytes())?;
            file.flush()?;
            Ok(true)
        })();
        let unlock = file.unlock();
        done.and_then(|ok| unlock.map(|()| ok))
    }

    /// Replay the log and fold it into the review's current state.
    pub fn state(&self) -> io::Result<ReviewState> {
        Ok(fold(&self.replay()?))
    }
}

/// Split a log's bytes into `(ordinal, parsed)` pairs.
///
/// Decoded lossily on purpose: a read that lands mid-way through a multi-byte
/// character must cost one unparseable line, not the whole replay.
fn parse_lines(bytes: &[u8]) -> Vec<(u64, Option<Record>)> {
    String::from_utf8_lossy(bytes)
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let ordinal = i as u64 + 1;
            (ordinal, serde_json::from_str::<Record>(line).ok())
        })
        .collect()
}

/// Fold a replay into a [`ReviewState`].
///
/// Split from [`Log::state`] so the fold is a pure function over records — every
/// folding rule is unit-testable without touching a filesystem.
#[must_use]
pub fn fold(entries: &[(u64, Option<Record>)]) -> ReviewState {
    let mut st = ReviewState::default();
    for (ordinal, rec) in entries {
        st.last_ordinal = *ordinal;
        let Some(rec) = rec else {
            st.unparsed = st.unparsed.saturating_add(1);
            st.first_unparsed = st.first_unparsed.or(Some(*ordinal));
            continue;
        };
        apply(&mut st, *ordinal, rec);
    }
    st
}

/// Apply one record to the folding state.
fn apply(st: &mut ReviewState, ordinal: u64, rec: &Record) {
    match rec {
        Record::Open {
            review,
            target,
            label,
            ..
        } => {
            // A new review supersedes whatever preceded it in the same file.
            *st = ReviewState {
                open: Some(OpenInfo {
                    review: review.clone(),
                    target: target.clone(),
                    label: label.clone(),
                }),
                last_ordinal: ordinal,
                ..ReviewState::default()
            };
        }
        Record::Serve { pid, port, url } => {
            st.serve = Some((
                ordinal,
                ServeRecord {
                    pid: *pid,
                    port: *port,
                    url: url.clone(),
                },
            ));
            // A later serve reopens: any earlier close no longer describes it.
            st.close = None;
        }
        Record::Close { pid, reason } => {
            // Only a close naming the current server describes it. A pidless close
            // (a TUI-only review ending) or one from an unrelated process must not
            // overwrite — and thereby un-pair — a serve that genuinely stopped.
            if st.serve.as_ref().is_some_and(|(_, s)| *pid == Some(s.pid)) {
                st.close = Some(CloseRecord {
                    pid: *pid,
                    reason: reason.clone(),
                });
            }
        }
        Record::Round { n, files } => st.rounds.push(RoundInfo {
            n: *n,
            files: files.clone(),
        }),
        Record::Thread(t) => {
            st.threads.insert(
                t.id.clone(),
                ThreadState {
                    ordinal,
                    thread: t.clone(),
                },
            );
        }
        Record::Submit(s) => st.submits.push((ordinal, s.clone())),
        Record::Drained { upto } => st.drained_upto = (*upto).max(st.drained_upto),
        // A whole-set snapshot: the latest one is the answer.
        Record::Viewed { paths } => st.viewed.clone_from(paths),
        // Read, understood to be beyond this build, and deliberately ignored —
        // *not* counted unparsed. See `Record::Unknown`.
        Record::Unknown => {}
    }
}

/// What [`open_review`] did, so a caller can report it accurately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// Attached to the review already in the log; no record was written.
    Attached,
    /// Started a fresh log, discarding a fully-delivered previous review.
    Fresh,
    /// Appended to the existing log, keeping the previous review's records.
    Kept,
}

/// Open a review, attaching to the one already in the log when it still holds
/// feedback nobody has consumed.
///
/// The rule is deliberately asymmetric: attaching is the safe default because
/// starting fresh truncates the file, and truncating a log with undelivered
/// feedback would silently discard a human's work. Only a log that is both fully
/// delivered *and* fully readable may be replaced — see
/// [`ReviewState::safe_to_replace`]. Note the test is on the log's contents, not
/// on whether an `open` record parsed: a log whose header is corrupt but whose
/// comments are intact is exactly the case that must not be destroyed.
pub fn open_review(
    log: &Log,
    review: &str,
    target: &str,
    label: Option<&str>,
    keep: bool,
) -> io::Result<(Opened, ReviewState)> {
    let existing = log.state()?;
    if !existing.safe_to_replace() {
        return Ok((Opened::Attached, existing));
    }
    let outcome = if existing.open.is_some() && keep {
        Opened::Kept
    } else {
        Opened::Fresh
    };
    let rec = Record::Open {
        review: review.to_string(),
        target: target.to_string(),
        label: label.map(ToString::to_string),
        at: super::record::now(),
    };
    let outcome = match outcome {
        // Conditional: the state read above was taken under a lock that has since
        // been released, so another writer may have appended feedback in between.
        // Declining means someone did, and attaching is the safe answer.
        Opened::Fresh => {
            if log.replace_if_safe(&rec)? {
                Opened::Fresh
            } else {
                return Ok((Opened::Attached, log.state()?));
            }
        }
        Opened::Attached | Opened::Kept => {
            log.append(&rec)?;
            outcome
        }
    };
    Ok((outcome, log.state()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A comment written between an agent's "is it drained?" check and its
    /// truncate must survive. `open_review` reads state under a shared lock and
    /// releases it; only re-checking inside the write lock closes the window.
    #[test]
    fn a_comment_appended_after_the_safety_check_is_not_truncated() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Log::at_worktree(dir.path());
        // A drained review: `safe_to_replace` is true right now.
        log.append(&Record::Open {
            review: "r1".into(),
            target: "worktree".into(),
            label: None,
            at: super::super::record::now(),
        })
        .unwrap();
        let seen = log.state().unwrap();
        assert!(seen.safe_to_replace(), "the agent sees a replaceable log");

        // The human comments before the agent gets to write.
        log.append(&Record::Thread(Thread {
            id: "t1".into(),
            anchor: None,
            body: "a human's unread comment".into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: super::super::record::now(),
        }))
        .unwrap();

        let replaced = log
            .replace_if_safe(&Record::Open {
                review: "r2".into(),
                target: "staged".into(),
                label: None,
                at: super::super::record::now(),
            })
            .unwrap();
        assert!(!replaced, "the truncate is declined");

        let st = log.state().unwrap();
        assert_eq!(st.threads.len(), 1, "the comment survives");
        assert_eq!(
            st.open.unwrap().target,
            "worktree",
            "and the review it belongs to is untouched"
        );
    }

    #[test]
    fn replace_if_safe_truncates_a_genuinely_spent_log() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Log::at_worktree(dir.path());
        log.append(&Record::Open {
            review: "r1".into(),
            target: "worktree".into(),
            label: None,
            at: super::super::record::now(),
        })
        .unwrap();
        let replaced = log
            .replace_if_safe(&Record::Open {
                review: "r2".into(),
                target: "staged".into(),
                label: None,
                at: super::super::record::now(),
            })
            .unwrap();
        assert!(replaced, "nothing pending, so it may be replaced");
        let raw = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(raw.lines().count(), 1, "the old review is gone");
        assert!(raw.contains("staged"));
    }

    use crate::review::record::{now, Anchor, Side};
    use crate::testutil::{review_log, run_git};
    use tempfile::TempDir;

    fn thread(id: &str, body: &str) -> Record {
        Record::Thread(Thread {
            id: id.into(),
            anchor: None,
            body: body.into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: now(),
        })
    }

    fn anchored(id: &str, line: u32) -> Record {
        Record::Thread(Thread {
            id: id.into(),
            anchor: Some(Anchor {
                path: "a.rs".into(),
                side: Side::New,
                line,
                quote: "x".into(),
                before: vec![],
                after: vec![],
            }),
            body: "c".into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: now(),
        })
    }

    fn submit(round: u32) -> Record {
        Record::Submit(Submit {
            round,
            preset: Some("rework".into()),
            body: "again".into(),
            at: now(),
        })
    }

    fn open(review: &str) -> Record {
        Record::Open {
            review: review.into(),
            target: "worktree".into(),
            label: None,
            at: now(),
        }
    }

    #[test]
    fn log_path_is_at_the_worktree_root() {
        let p = log_path_in(Path::new("/w/tree"));
        assert_eq!(p, PathBuf::from("/w/tree/rediff.jsonl"));
    }

    #[test]
    fn log_path_resolves_to_each_worktrees_own_root() {
        // The case that makes `.git`-relative storage impossible: in a linked
        // worktree `.git` is a *file*, and its git-dir points outside the worktree.
        // The log must still land at the worktree's own root.
        let scratch = crate::testutil::scratch_repo();
        let main = scratch.path();
        std::fs::write(main.join("f.txt"), "x\n").unwrap();
        run_git(main, &["add", "-A"]);
        run_git(main, &["commit", "-qm", "base"]);
        run_git(main, &["worktree", "add", "-q", "wt", "-b", "linked"]);

        let linked = main.join("wt");
        assert!(
            linked.join(".git").is_file(),
            "a linked worktree's .git is a file, not a directory"
        );

        let root = |p: &Path| p.canonicalize().unwrap();
        for wt in [main, linked.as_path()] {
            let repo = gix::discover(wt).unwrap();
            let got = log_path(&repo).expect("a worktree has a log path");
            assert_eq!(got.file_name().unwrap(), LOG_FILE_NAME);
            assert_eq!(
                root(got.parent().unwrap()),
                root(wt),
                "the log belongs to this worktree, not the common dir"
            );
        }

        let common = root(&main.join(".git"));
        let linked_log = log_path(&gix::discover(&linked).unwrap()).unwrap();
        assert!(
            !root(linked_log.parent().unwrap()).starts_with(&common),
            "never inside the git directory"
        );
    }

    #[test]
    fn log_path_is_none_for_a_bare_repository() {
        let tmp = TempDir::new().unwrap();
        run_git(tmp.path(), &["init", "-q", "--bare"]);
        let repo = gix::discover(tmp.path()).unwrap();
        assert_eq!(log_path(&repo), None, "no worktree, nowhere to put a log");
    }

    #[test]
    fn replay_of_a_missing_file_is_empty() {
        let (_d, log) = review_log();
        assert!(!log.exists());
        assert!(log.replay().unwrap().is_empty());
        assert!(log.state().unwrap().open.is_none());
    }

    #[test]
    fn append_reports_creation_only_once() {
        let (_d, log) = review_log();
        assert!(log.append(&open("r1")).unwrap(), "first append creates");
        assert!(
            !log.append(&thread("t1", "hi")).unwrap(),
            "second append does not"
        );
    }

    #[test]
    fn append_never_rewrites_earlier_lines() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        let after_first = std::fs::read_to_string(log.path()).unwrap();
        log.append(&thread("t1", "hi")).unwrap();
        let after_second = std::fs::read_to_string(log.path()).unwrap();
        assert!(
            after_second.starts_with(&after_first),
            "earlier bytes must be untouched"
        );
        assert_eq!(after_second.lines().count(), 2);
    }

    #[test]
    fn ordinals_are_raw_line_numbers_and_survive_a_malformed_line() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        // Splice in a line that cannot parse, exactly as a corrupt writer might.
        {
            let mut f = OpenOptions::new().append(true).open(log.path()).unwrap();
            f.write_all(b"{not json at all\n").unwrap();
        }
        log.append(&thread("t1", "hi")).unwrap();

        let entries = log.replay().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].0, 1);
        assert!(entries[1].1.is_none(), "line 2 is unparseable");
        assert_eq!(entries[1].0, 2, "and still consumes ordinal 2");
        assert_eq!(entries[2].0, 3, "so the thread keeps ordinal 3");

        let st = log.state().unwrap();
        assert_eq!(st.threads["t1"].ordinal, 3);
        assert_eq!(st.last_ordinal, 3);
    }

    #[test]
    fn an_unknown_tag_folds_to_nothing_rather_than_counting_unparsed() {
        // A record from a newer rediff is read, understood to be beyond this
        // build, and ignored. Counting it unparsed instead would pin
        // `safe_to_replace()` false on a log holding no feedback at all, and
        // `rediff request` would have no way forward but deleting the file.
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        {
            let mut f = OpenOptions::new().append(true).open(log.path()).unwrap();
            f.write_all(br#"{"t":"telepathy","body":"?"}"#).unwrap();
            f.write_all(b"\n").unwrap();
        }
        let entries = log.replay().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].1, Some(Record::Unknown), "parsed, not skipped");

        let st = log.state().unwrap();
        assert_eq!(st.unparsed, 0, "it is not damage");
        assert_eq!(st.first_unparsed, None);
        assert!(st.safe_to_replace(), "and it does not wedge the log");
        assert_eq!(st.last_ordinal, 2, "it still consumes its ordinal");
    }

    #[test]
    fn viewed_snapshots_replace_and_never_become_work_for_a_consumer() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&Record::Viewed {
            paths: vec!["a.rs".into()],
        })
        .unwrap();
        log.append(&Record::Viewed {
            paths: vec!["a.rs".into(), "b.rs".into()],
        })
        .unwrap();

        let st = log.state().unwrap();
        assert_eq!(st.viewed, vec!["a.rs", "b.rs"], "the latest snapshot wins");
        // Inert: not feedback, so nothing to drain and nothing pinning the log.
        assert!(st.fully_drained(), "a `v` press is not work for the agent");
        assert!(st.safe_to_replace());

        // A new review starts with an empty reviewed set rather than inheriting
        // the previous one's — `Open` resets the fold.
        log.append(&open("r2")).unwrap();
        assert!(log.state().unwrap().viewed.is_empty());
    }

    #[test]
    fn edit_supersedes_and_delete_retracts() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&thread("t1", "first")).unwrap();
        log.append(&thread("t1", "second")).unwrap();

        let st = log.state().unwrap();
        assert_eq!(st.threads.len(), 1, "folded by id");
        assert_eq!(st.threads["t1"].thread.body, "second");
        assert_eq!(
            st.threads["t1"].ordinal, 3,
            "ordinal follows the last record"
        );
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap().lines().count(),
            3,
            "both records remain in the log"
        );

        log.append(&Record::Thread(Thread {
            id: "t1".into(),
            anchor: None,
            body: "second".into(),
            replace: None,
            resolved: false,
            deleted: true,
            at: now(),
        }))
        .unwrap();
        let st = log.state().unwrap();
        assert!(st.threads["t1"].thread.deleted, "retracted, still present");
    }

    #[test]
    fn resolved_is_retained_by_the_fold() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&thread("t1", "x")).unwrap();
        log.append(&Record::Thread(Thread {
            id: "t1".into(),
            anchor: None,
            body: "x".into(),
            replace: None,
            resolved: true,
            deleted: false,
            at: now(),
        }))
        .unwrap();
        let st = log.state().unwrap();
        assert!(st.threads["t1"].thread.resolved);
        assert!(!st.threads["t1"].thread.deleted);
    }

    #[test]
    fn two_threads_on_one_location_both_survive() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&anchored("t1", 47)).unwrap();
        log.append(&anchored("t2", 47)).unwrap();
        let st = log.state().unwrap();
        assert_eq!(st.threads.len(), 2);
        assert!(st.threads.contains_key("t1") && st.threads.contains_key("t2"));
    }

    #[test]
    fn threads_keep_first_appearance_order() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&thread("b", "1")).unwrap();
        log.append(&thread("a", "2")).unwrap();
        log.append(&thread("b", "3")).unwrap();
        let st = log.state().unwrap();
        let ids: Vec<_> = st.threads.keys().map(String::as_str).collect();
        assert_eq!(ids, vec!["b", "a"], "an edit does not reorder");
    }

    #[test]
    fn rounds_and_submits_accumulate_in_order() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&Record::Round {
            n: 1,
            files: BTreeMap::from([("a.rs".to_string(), 1_u64)]),
        })
        .unwrap();
        log.append(&submit(1)).unwrap();
        log.append(&Record::Round {
            n: 2,
            files: BTreeMap::new(),
        })
        .unwrap();
        let st = log.state().unwrap();
        assert_eq!(
            st.rounds.iter().map(|r| r.n).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(st.submits.len(), 1);
        assert_eq!(st.submits[0].1.round, 1);
    }

    #[test]
    fn drained_takes_the_high_water_mark() {
        let st = fold(&[
            (1, Some(open("r1"))),
            (2, Some(Record::Drained { upto: 9 })),
            (3, Some(Record::Drained { upto: 4 })),
        ]);
        assert_eq!(st.drained_upto, 9, "a lower marker never rewinds delivery");
    }

    #[test]
    fn a_second_open_supersedes_the_first() {
        let st = fold(&[
            (1, Some(open("r1"))),
            (2, Some(thread("t1", "old"))),
            (3, Some(open("r2"))),
            (4, Some(thread("t2", "new"))),
        ]);
        assert_eq!(st.open.unwrap().review, "r2");
        assert_eq!(st.threads.len(), 1, "the earlier review's threads are gone");
        assert!(st.threads.contains_key("t2"));
        assert_eq!(st.last_ordinal, 4);
    }

    #[test]
    fn fully_drained_tracks_pending_feedback() {
        let st = fold(&[(1, Some(open("r1"))), (2, Some(thread("t1", "x")))]);
        assert!(!st.fully_drained());

        let st = fold(&[
            (1, Some(open("r1"))),
            (2, Some(thread("t1", "x"))),
            (3, Some(Record::Drained { upto: 2 })),
        ]);
        assert!(st.fully_drained());

        let st = fold(&[
            (1, Some(open("r1"))),
            (2, Some(thread("t1", "x"))),
            (3, Some(Record::Drained { upto: 2 })),
            (4, Some(submit(1))),
        ]);
        assert!(!st.fully_drained(), "a later submit is pending again");
    }

    #[test]
    fn an_empty_review_is_vacuously_drained() {
        let st = fold(&[(1, Some(open("r1")))]);
        assert!(st.fully_drained());
    }

    #[test]
    fn open_review_attaches_when_feedback_is_pending() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&thread("t1", "unread")).unwrap();

        let (outcome, st) = open_review(&log, "r2", "worktree", None, false).unwrap();
        assert_eq!(outcome, Opened::Attached);
        assert_eq!(st.open.unwrap().review, "r1", "kept the existing review");
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap().lines().count(),
            2,
            "attaching writes nothing"
        );
    }

    #[test]
    fn open_review_starts_fresh_once_drained() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&thread("t1", "read")).unwrap();
        log.append(&Record::Drained { upto: 2 }).unwrap();

        let (outcome, st) = open_review(&log, "r2", "worktree", Some("agent"), false).unwrap();
        assert_eq!(outcome, Opened::Fresh);
        assert_eq!(st.open.as_ref().unwrap().review, "r2");
        assert_eq!(st.open.unwrap().label.as_deref(), Some("agent"));
        assert!(st.threads.is_empty(), "the old review is gone");
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap().lines().count(),
            1
        );
    }

    #[test]
    fn open_review_keeps_history_when_asked() {
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&thread("t1", "read")).unwrap();
        log.append(&Record::Drained { upto: 2 }).unwrap();

        let (outcome, st) = open_review(&log, "r2", "worktree", None, true).unwrap();
        assert_eq!(outcome, Opened::Kept);
        assert_eq!(st.open.unwrap().review, "r2");
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap().lines().count(),
            4,
            "previous records retained"
        );
    }

    #[test]
    fn open_review_on_an_empty_log_is_fresh() {
        let (_d, log) = review_log();
        let (outcome, st) = open_review(&log, "r1", "worktree", None, true).unwrap();
        assert_eq!(outcome, Opened::Fresh, "keep has nothing to keep");
        assert_eq!(st.open.unwrap().review, "r1");
    }

    #[test]
    fn a_corrupt_open_record_never_costs_the_feedback_beneath_it() {
        // Regression: the attach test used to be `open.is_some() && !drained`, so a
        // log whose header did not parse read as "no review" and was deleted —
        // taking a human's unread comments with it, even with keep requested.
        let (_d, log) = review_log();
        {
            let mut f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log.path())
                .unwrap();
            f.write_all(b"{\"t\":\"open\",TRUNCATED\n").unwrap();
        }
        log.append(&thread("t1", "unread and precious")).unwrap();

        let (outcome, st) = open_review(&log, "r2", "worktree", None, false).unwrap();
        assert_eq!(outcome, Opened::Attached, "must not replace what it holds");
        assert_eq!(st.unparsed, 1);
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(text.contains("unread and precious"), "feedback survives");
    }

    #[test]
    fn a_log_with_unreadable_records_is_never_replaced() {
        // A *damaged* line — torn by a crash mid-append, or corrupted — must not
        // be mistaken for "nothing in it". This is the case that still pins the
        // log; a well-formed record from a newer build no longer does, which is
        // the deliberate trade in `Record::Unknown`.
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        {
            let mut f = OpenOptions::new().append(true).open(log.path()).unwrap();
            f.write_all(b"{\"t\":\"thread\",\"id\":\"t1\",\"body\":\"half a li\n")
                .unwrap();
        }
        let st = log.state().unwrap();
        assert!(st.fully_drained(), "nothing this build can see is pending");
        assert!(!st.safe_to_replace(), "but it is still not safe to destroy");
        assert_eq!(st.first_unparsed, Some(2));

        let (outcome, _) = open_review(&log, "r2", "worktree", None, false).unwrap();
        assert_eq!(outcome, Opened::Attached);
        assert!(std::fs::read_to_string(log.path())
            .unwrap()
            .contains("half a li"));
    }

    #[test]
    fn a_retracted_thread_does_not_wedge_the_review() {
        // Regression: `pending_ordinals` counted deleted threads, but delivery skips
        // them — so drain had nothing to deliver, never wrote a marker, and the
        // review could never finish. Writing a comment and deleting it wedged the
        // worktree's store permanently.
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();
        log.append(&thread("t1", "on reflection, no")).unwrap();
        log.append(&Record::Thread(Thread {
            id: "t1".into(),
            anchor: None,
            body: "on reflection, no".into(),
            replace: None,
            resolved: false,
            deleted: true,
            at: now(),
        }))
        .unwrap();

        let st = log.state().unwrap();
        assert!(
            st.fully_drained(),
            "a retracted thread is not pending delivery"
        );
        let (outcome, _) = open_review(&log, "r2", "worktree", None, false).unwrap();
        assert_eq!(outcome, Opened::Fresh, "the store is not stuck");
    }

    #[test]
    fn a_pidless_close_does_not_unpair_a_genuine_one() {
        // Regression: the fold kept only the newest close, so a TUI-only close
        // arriving after the server's own close made the stopped server look live.
        let st = fold(&[
            (1, Some(open("r1"))),
            (
                2,
                Some(Record::Serve {
                    pid: 4411,
                    port: 53411,
                    url: "http://127.0.0.1:53411/".into(),
                }),
            ),
            (
                3,
                Some(Record::Close {
                    pid: Some(4411),
                    reason: "drained".into(),
                }),
            ),
            (
                4,
                Some(Record::Close {
                    pid: None,
                    reason: "review ended in the TUI".into(),
                }),
            ),
        ]);
        assert_eq!(
            st.close.as_ref().map(|c| c.reason.as_str()),
            Some("drained"),
            "the close that names the server is the one that describes it"
        );
    }

    #[test]
    fn a_review_that_recorded_no_feedback_may_be_replaced() {
        // An `open` with nothing after it is vacuously finished — there is no
        // feedback to lose, so a new review takes the file rather than attaching to
        // an abandoned one.
        let (_d, log) = review_log();
        log.append(&open("r1")).unwrap();

        let (outcome, st) = open_review(&log, "r2", "worktree", None, false).unwrap();
        assert_eq!(outcome, Opened::Fresh);
        assert_eq!(st.open.unwrap().review, "r2");
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap().lines().count(),
            1
        );
    }

    #[test]
    fn concurrent_appends_produce_whole_lines() {
        let (_d, log) = review_log();
        let writers = 8;
        let per_writer = 25;
        // A body long enough that an unlocked write could plausibly interleave.
        let filler = "x".repeat(4096);

        std::thread::scope(|scope| {
            for w in 0..writers {
                let log = log.clone();
                let filler = filler.as_str();
                scope.spawn(move || {
                    for i in 0..per_writer {
                        let rec = thread(&format!("w{w}-{i}"), filler);
                        log.append(&rec).unwrap();
                    }
                });
            }
        });

        let text = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(text.lines().count(), writers * per_writer);
        for line in text.lines() {
            assert!(
                serde_json::from_str::<Record>(line).is_ok(),
                "every line parses whole: {}…",
                line.chars().take(60).collect::<String>()
            );
        }
        let st = log.state().unwrap();
        assert_eq!(st.threads.len(), writers * per_writer, "no record lost");
    }
}
