//! The bridge between the TUI's line cursor and the review log's anchors.
//!
//! Named `reviewlog`, not `review`: `crate::tui::review` already exists and is
//! viewed-tracking over `ViewState`. Everything here talks to `crate::review`,
//! the on-disk log, and the two are easy to confuse.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::model::Changeset;
use crate::review::{capture, open_review, open_round, Anchor, Log, Opened};
use crate::tui::rows::{self, Plan};

/// Why the cursor's row could not be turned into an anchor.
///
/// Two failure points, not one: the row may carry no line identity at all, and
/// the file's text for that side may be unreadable. They deserve different
/// messages — the first is "this row is not a diff line", the second is "this
/// line cannot be quoted".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorError {
    /// The row is chrome — a header, hunk gap, placeholder, spacer, banner — or
    /// a binary-file note. It carries no `(file, side, line)`.
    NotALine,
    /// The row names a line whose text cannot be read: the changeset has no such
    /// file index, or the side carries no text.
    NoText,
}

/// Resolve the line cursor's row to a review anchor.
///
/// `rows::cursor_key` yields `(file_index, side, line)`, but `review::capture`
/// takes the `&DiffFile` itself — so the changeset has to be consulted in
/// between. That lookup is the second failure point.
///
/// The side is whatever `cursor_key` chose: the new side when the row carries
/// one, the old side otherwise. There is no side to pick here, per the accepted
/// `line-cursor` requirement that the cursor names a change rather than a side
/// of one.
pub fn anchor_at(cs: &Changeset, plan: &Plan, row: usize) -> Result<Anchor, AnchorError> {
    let (fi, side, line) = rows::cursor_key(plan, row).ok_or(AnchorError::NotALine)?;
    let file = cs.files.get(fi).ok_or(AnchorError::NoText)?;
    capture(file, side, line).ok_or(AnchorError::NoText)
}

impl AnchorError {
    /// What to show the user, as a status-line flash.
    pub fn message(self) -> &'static str {
        match self {
            AnchorError::NotALine => "nothing to comment on here — move to a diff line",
            AnchorError::NoText => "this line's text can't be read, so it can't be anchored",
        }
    }
}

/// Why a review could not be opened for this view.
#[derive(Debug)]
pub enum EnsureError {
    /// The view's diffs have not all arrived. `open_round` refuses a
    /// partly-diffed changeset, and an undiffed file has no text to anchor into.
    StillLoading,
    /// The view is narrowed by path filters. A round hashed over a subset makes
    /// the next unfiltered `rediff request` report every excluded file as added.
    Filtered,
    /// A review is already open over a different target and still holds feedback
    /// nobody has drained. Replacing it would strand that feedback; anchoring
    /// into it would capture against a diff the review is not about.
    TargetMismatch {
        open: String,
        want: String,
    },
    Io(io::Error),
}

impl EnsureError {
    /// What to show the user, as a status-line flash.
    pub fn message(&self) -> String {
        match self {
            EnsureError::StillLoading => {
                "still loading — try again when the diff finishes".to_string()
            }
            EnsureError::Filtered => {
                "a review covers a whole target; this view is filtered".to_string()
            }
            EnsureError::TargetMismatch { open, want } => {
                format!("`{open}` is open with undelivered feedback; this is `{want}`")
            }
            EnsureError::Io(e) => format!("could not write the review log: {e}"),
        }
    }
}

impl From<io::Error> for EnsureError {
    fn from(e: io::Error) -> Self {
        EnsureError::Io(e)
    }
}

/// Distinct ids for threads recorded in the same process.
///
/// `reviewcli::new_review_id` seeds only on `(pid, unix seconds)`, which is
/// enough for a *review* — one per log — but not for threads: two comments
/// confirmed within the same second would share an id, and `fold` keys threads
/// by id, so the earlier one is superseded and never delivered. The bytes stay
/// on disk and every consumer reads the fold, so the loss is silent.
static THREAD_SEQ: AtomicU64 = AtomicU64::new(0);

/// A thread id unique within this process and this second.
pub fn new_thread_id() -> String {
    let n = THREAD_SEQ.fetch_add(1, Ordering::Relaxed);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut seed = [0u8; 20];
    seed[..4].copy_from_slice(&std::process::id().to_le_bytes());
    seed[4..12].copy_from_slice(&secs.to_le_bytes());
    seed[12..].copy_from_slice(&n.to_le_bytes());
    format!("t{:08x}", crate::review::content_hash(&seed) & 0xffff_ffff)
}

/// A review that is now open and has a round covering this changeset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    pub opened: Opened,
    pub round: u32,
}

/// Open or attach to the worktree's review, lazily, for `target`.
///
/// Pure over its inputs — the caller owns the `Log`, so every refusal is
/// reachable from a unit test rather than only through a running TUI.
///
/// The refusals are checked *before* anything is appended, so a refused comment
/// leaves no trace: browsing writes nothing, and neither does a comment the
/// view cannot honour.
pub fn ensure_review(
    log: &Log,
    cs: &Changeset,
    target: &str,
    filtered: bool,
) -> Result<Opening, EnsureError> {
    if filtered {
        return Err(EnsureError::Filtered);
    }
    if !cs.fully_diffed() {
        return Err(EnsureError::StillLoading);
    }

    let st = log.state()?;
    // Only open a review when one is not already ours. `open_review` appends an
    // `Open` record unconditionally, and `fold` *resets the whole state* on one
    // — so calling it per comment would restart the round counter at 1 and drop
    // the agent's label and every delivered thread from the fold. `request` had
    // exactly this bug once; the same rule fixes it here.
    let (opened, st) = match st.open.as_ref() {
        Some(open) if open.target == target => (Opened::Attached, st),
        Some(open) if !st.safe_to_replace() => {
            return Err(EnsureError::TargetMismatch {
                open: open.target.clone(),
                want: target.to_string(),
            })
        }
        // No review, or a spent one over a different target. `keep` so an
        // agent-opened review with delivered feedback is appended to rather
        // than truncated — the human is joining it, not starting over.
        _ => open_review(log, &crate::reviewcli::new_review_id(), target, None, true)?,
    };
    // `frozen: false`, as `request` passes: a round is opened only when the
    // content actually changed since the last one.
    let round = open_round(log, &st, cs, false)?;
    Ok(Opening { opened, round })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffFile, FileStatus, Hunk, LayoutMode, Line, LineKind, Stats};
    use crate::review::Side;
    use std::collections::BTreeSet;

    fn file_with(old: &str, new: &str, lines: Vec<Line>) -> DiffFile {
        DiffFile {
            path: "f.rs".into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks: vec![Hunk {
                old_start: 1,
                old_len: 2,
                new_start: 1,
                new_len: 2,
                lines,
            }],
            stats: Stats {
                additions: 1,
                deletions: 1,
            },
            language: None,
            is_binary: false,
            old_text: (!old.is_empty()).then(|| old.to_string()),
            new_text: (!new.is_empty()).then(|| new.to_string()),
            content_digest: None,
            diffed: true,
        }
    }

    fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> Line {
        Line {
            kind,
            old_lineno: old,
            new_lineno: new,
            text: text.into(),
            emphasis: None,
        }
    }

    fn sample() -> Changeset {
        Changeset {
            source: "t".into(),
            files: vec![file_with(
                "alpha\nbeta\n",
                "alpha\ngamma\n",
                vec![
                    line(LineKind::Context, Some(1), Some(1), "alpha"),
                    line(LineKind::Removed, Some(2), None, "beta"),
                    line(LineKind::Added, None, Some(2), "gamma"),
                ],
            )],
        }
    }

    fn plan_of(cs: &Changeset, layout: LayoutMode) -> Plan {
        Plan::build(cs, &[false], layout, &BTreeSet::new())
    }

    #[test]
    fn an_added_line_anchors_to_its_new_side_with_quote_and_context() {
        let cs = sample();
        let p = plan_of(&cs, LayoutMode::Stack);
        let row = rows::find_key(&p, (0, Side::New, 2)).unwrap();
        let a = anchor_at(&cs, &p, row).expect("added line anchors");
        assert_eq!((a.side, a.line), (Side::New, 2));
        assert_eq!(a.quote, "gamma");
        assert_eq!(a.before, vec!["alpha".to_string()], "context is captured");
    }

    #[test]
    fn a_removed_line_anchors_to_the_old_side() {
        // Nothing chooses this: the row carries only `Old`, so `cursor_key`
        // cannot pick a textless new side.
        let cs = sample();
        let p = plan_of(&cs, LayoutMode::Stack);
        let row = rows::find_key(&p, (0, Side::Old, 2)).unwrap();
        let a = anchor_at(&cs, &p, row).expect("removed line anchors");
        assert_eq!((a.side, a.line), (Side::Old, 2));
        assert_eq!(a.quote, "beta");
    }

    #[test]
    fn a_context_line_anchors_to_its_new_side_in_either_layout() {
        let cs = sample();
        for layout in [LayoutMode::Stack, LayoutMode::Split] {
            let p = plan_of(&cs, layout);
            let row = rows::find_key(&p, (0, Side::New, 1)).unwrap();
            let a = anchor_at(&cs, &p, row).expect("context line anchors");
            assert_eq!((a.side, a.line), (Side::New, 1), "{layout:?}");
            assert_eq!(a.quote, "alpha");
        }
    }

    #[test]
    fn every_chrome_row_is_not_a_line() {
        let cs = sample();
        let p = plan_of(&cs, LayoutMode::Stack);
        // The file header and the trailing spacer are the chrome this plan has.
        for row in [0, p.rows.len() - 1] {
            assert_eq!(
                anchor_at(&cs, &p, row),
                Err(AnchorError::NotALine),
                "row {row} is chrome"
            );
        }
        // And past the end.
        assert_eq!(anchor_at(&cs, &p, 999), Err(AnchorError::NotALine));
    }

    #[test]
    fn a_binary_note_is_not_a_line_rather_than_a_capture_failure() {
        // The distinction matters for the message: a binary file's note row
        // carries no key at all, so the key is inert *before* `capture` is
        // reached. Nothing tells the user "cannot be anchored" here.
        let mut cs = sample();
        cs.files[0].is_binary = true;
        cs.files[0].hunks.clear();
        let p = plan_of(&cs, LayoutMode::Stack);
        let note = p
            .rows
            .iter()
            .position(|r| matches!(r, crate::tui::rows::Row::Line { .. }))
            .expect("stack layout shows a note row for a binary");
        assert_eq!(anchor_at(&cs, &p, note), Err(AnchorError::NotALine));
    }

    #[test]
    fn a_row_whose_side_has_no_text_reports_no_text() {
        // Reached deliberately: from a real cursor this is near-unreachable, so
        // it is constructed rather than driven through the TUI. Here the plan
        // still names new line 2 but the file's new text is gone.
        let cs = sample();
        let p = plan_of(&cs, LayoutMode::Stack);
        let row = rows::find_key(&p, (0, Side::New, 2)).unwrap();

        let mut stripped = sample();
        stripped.files[0].new_text = None;
        assert_eq!(anchor_at(&stripped, &p, row), Err(AnchorError::NoText));

        // And a plan whose file index does not exist in the changeset at all.
        let empty = Changeset {
            source: "t".into(),
            files: Vec::new(),
        };
        assert_eq!(anchor_at(&empty, &p, row), Err(AnchorError::NoText));
    }

    #[test]
    fn a_line_past_the_end_of_the_side_text_reports_no_text() {
        let cs = sample();
        let p = plan_of(&cs, LayoutMode::Stack);
        let row = rows::find_key(&p, (0, Side::New, 2)).unwrap();
        let mut short = sample();
        short.files[0].new_text = Some("only one line\n".into());
        assert_eq!(anchor_at(&short, &p, row), Err(AnchorError::NoText));
    }

    /// A log in a fresh temp dir.
    fn scratch_log() -> (tempfile::TempDir, Log) {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at_worktree(dir.path());
        (dir, log)
    }

    #[test]
    fn the_first_comment_opens_a_review_and_a_round() {
        let (_d, log) = scratch_log();
        let cs = sample();
        let op = ensure_review(&log, &cs, "worktree", false).expect("opens");
        assert_eq!(op.round, 1, "the first round");
        assert!(log.path().exists(), "the log was created");
        let st = log.state().unwrap();
        assert_eq!(st.open.as_ref().unwrap().target, "worktree");
    }

    #[test]
    fn a_second_comment_attaches_rather_than_reopening() {
        let (_d, log) = scratch_log();
        let cs = sample();
        let first = ensure_review(&log, &cs, "worktree", false).unwrap();
        let second = ensure_review(&log, &cs, "worktree", false).unwrap();
        assert_eq!(
            second.round, first.round,
            "no new round for unchanged content"
        );
        assert_eq!(second.opened, Opened::Attached, "attached, not reopened");
        // Asserted on the raw file: `fold` resets its whole state on an `Open`,
        // so a second one still leaves `rounds.len() == 1` and hides itself. An
        // earlier version of this test checked only the fold, and so passed
        // *because* of the bug it is named for.
        let raw = std::fs::read_to_string(log.path()).unwrap();
        let opens = raw.lines().filter(|l| l.contains(r#""t":"open""#)).count();
        assert_eq!(opens, 1, "exactly one `open` record on disk");
    }

    #[test]
    fn a_still_loading_view_is_refused_and_writes_nothing() {
        let (_d, log) = scratch_log();
        let mut cs = sample();
        cs.files[0].diffed = false;
        let err = ensure_review(&log, &cs, "worktree", false).unwrap_err();
        assert!(matches!(err, EnsureError::StillLoading));
        assert!(!log.path().exists(), "a refusal leaves no log behind");
    }

    #[test]
    fn a_filtered_view_is_refused_and_writes_nothing() {
        let (_d, log) = scratch_log();
        let err = ensure_review(&log, &sample(), "worktree", true).unwrap_err();
        assert!(matches!(err, EnsureError::Filtered));
        assert!(!log.path().exists());
    }

    #[test]
    fn a_different_target_is_refused_while_feedback_is_pending() {
        let (_d, log) = scratch_log();
        let cs = sample();
        ensure_review(&log, &cs, "worktree", false).unwrap();
        // Undelivered feedback pins the review in place.
        log.append(&crate::review::Record::Thread(crate::review::Thread {
            id: "t1".into(),
            anchor: None,
            body: "hold on".into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: crate::review::now(),
        }))
        .unwrap();

        let err = ensure_review(&log, &cs, "show:HEAD~3", false).unwrap_err();
        match err {
            EnsureError::TargetMismatch { open, want } => {
                assert_eq!(open, "worktree");
                assert_eq!(want, "show:HEAD~3");
                // Both named, so the user can tell which is which.
                let m = EnsureError::TargetMismatch { open, want }.message();
                assert!(m.contains("worktree") && m.contains("show:HEAD~3"), "{m}");
            }
            other => panic!("expected a target mismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_different_target_is_allowed_once_the_feedback_is_delivered() {
        let (_d, log) = scratch_log();
        let cs = sample();
        ensure_review(&log, &cs, "worktree", false).unwrap();
        // Nothing pending: the review may be replaced.
        assert!(log.state().unwrap().safe_to_replace());
        ensure_review(&log, &cs, "show:HEAD~3", false).expect("replaces a spent review");
        assert_eq!(log.state().unwrap().open.unwrap().target, "show:HEAD~3");
    }

    #[test]
    fn replacing_a_spent_review_keeps_its_records_rather_than_truncating() {
        // `keep: true`'s whole purpose, and the only path that still reaches it
        // now that a matching target attaches without reopening. With
        // `keep: false` the log is truncated and the previous review's history
        // — an agent's label, its delivered threads — is destroyed.
        let (_d, log) = scratch_log();
        let cs = sample();
        ensure_review(&log, &cs, "staged", false).unwrap();
        let before = std::fs::read_to_string(log.path()).unwrap();
        assert!(before.contains(r#""target":"staged""#));

        // Nothing pending, so a different target is allowed.
        ensure_review(&log, &cs, "worktree", false).unwrap();
        let after = std::fs::read_to_string(log.path()).unwrap();
        assert!(
            after.contains(r#""target":"staged""#),
            "the spent review's records are kept, not truncated: {after}"
        );
        assert!(
            after.contains(r#""target":"worktree""#),
            "and the new one is open"
        );
    }

    #[test]
    fn every_refusal_has_its_own_message() {
        let msgs = [
            EnsureError::StillLoading.message(),
            EnsureError::Filtered.message(),
            EnsureError::TargetMismatch {
                open: "a".into(),
                want: "b".into(),
            }
            .message(),
            EnsureError::Io(io::Error::other("disk on fire")).message(),
        ];
        for m in &msgs {
            assert!(!m.is_empty());
        }
        let uniq: std::collections::HashSet<&String> = msgs.iter().collect();
        assert_eq!(uniq.len(), msgs.len(), "each refusal reads differently");
        assert!(
            msgs[3].contains("disk on fire"),
            "io errors carry their cause"
        );
    }

    #[test]
    fn both_messages_are_distinct_and_non_empty() {
        assert_ne!(
            AnchorError::NotALine.message(),
            AnchorError::NoText.message()
        );
        assert!(!AnchorError::NotALine.message().is_empty());
    }
}
