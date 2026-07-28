//! Delivery: which feedback a consumer has not seen yet, and marking what it has.
//!
//! Drain-once is the default because it is the shape a consumer wants — poll, get
//! an inbox, act. The full replay exists because drain-once has one failure mode: a
//! consumer that dies between receiving and acting never sees those records again.
//! Both are filters over the same fold, so there is one set of semantics and no
//! cursor for a caller to keep.

use std::collections::HashMap;
use std::io;

use super::anchor::{find_file, resolve_in, side_lines, Resolution};
use super::log::{Log, ReviewState};
use super::record::{Anchor, Record, Side, Submit, Thread};
use crate::model::Changeset;

/// One thread as delivered, with its anchor resolved against the current diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    /// The thread's folded state.
    pub thread: Thread,
    /// Where its anchor landed, or `None` for a review-level thread.
    pub resolution: Option<Resolution>,
    /// Whether a previous drain already delivered this record.
    pub already_delivered: bool,
}

impl Delivered {
    /// Whether this thread is anchored to code that could not be found.
    #[must_use]
    pub fn is_detached(&self) -> bool {
        self.resolution.is_some_and(Resolution::is_detached)
    }
}

/// One round-closing instruction as delivered.
///
/// Carries the same delivery flag threads do. A submit is an *instruction* — "ship
/// it", "revert the caching change" — so a consumer recovering via [`all`] must be
/// able to tell a stale round's instruction from the current one, or it will act on
/// the wrong one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredSubmit {
    /// The instruction.
    pub submit: Submit,
    /// Whether a previous drain already delivered it.
    pub already_delivered: bool,
}

/// What a delivery call returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delivery {
    /// Threads, in first-appearance order.
    pub threads: Vec<Delivered>,
    /// Round-closing instructions, in the order they were recorded.
    pub submits: Vec<DeliveredSubmit>,
}

impl Delivery {
    /// Whether there is nothing to deliver.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty() && self.submits.is_empty()
    }
}

/// Collect threads and submits, optionally restricted to undelivered records.
///
/// Retracted threads are excluded from delivery in both modes — the record stays in
/// the log, but a consumer should not act on something the author took back.
fn collect(st: &ReviewState, cs: &Changeset, only_pending: bool) -> Delivery {
    // Split each referenced file's text once and reuse it for every anchor in that
    // file. Resolving anchor-by-anchor re-splits the whole file per thread, which a
    // consumer polling `all()` pays on every call.
    let mut lines_cache: HashMap<(&str, Side), Option<Vec<&str>>> = HashMap::new();

    let threads = st
        .threads
        .values()
        .filter(|t| !t.thread.deleted)
        .filter(|t| !only_pending || t.ordinal > st.drained_upto)
        .map(|t| Delivered {
            resolution: t
                .thread
                .anchor
                .as_ref()
                .map(|a| resolve_cached(a, cs, &mut lines_cache)),
            already_delivered: t.ordinal <= st.drained_upto,
            thread: t.thread.clone(),
        })
        .collect();
    let submits = st
        .submits
        .iter()
        .filter(|(ord, _)| !only_pending || *ord > st.drained_upto)
        .map(|(ord, s)| DeliveredSubmit {
            submit: s.clone(),
            already_delivered: *ord <= st.drained_upto,
        })
        .collect();
    Delivery { threads, submits }
}

/// Resolve one anchor, reusing a per-`(path, side)` split of the file's text.
///
/// Mirrors [`resolve`] exactly, including reporting an undiffed file as
/// [`Resolution::Unresolved`] rather than as deleted code.
fn resolve_cached<'c>(
    anchor: &Anchor,
    cs: &'c Changeset,
    cache: &mut HashMap<(&'c str, Side), Option<Vec<&'c str>>>,
) -> Resolution {
    let Some(file) = find_file(anchor, cs) else {
        return Resolution::Detached;
    };
    if !file.diffed {
        return Resolution::Unresolved;
    }
    let key = (file.path.as_str(), anchor.side);
    let lines = cache
        .entry(key)
        .or_insert_with(|| side_lines(file, anchor.side));
    match lines {
        None => Resolution::Detached,
        Some(lines) => resolve_in(anchor, lines),
    }
}

/// Feedback recorded since the last drain, with anchors resolved against `cs`.
#[must_use]
pub fn undelivered(st: &ReviewState, cs: &Changeset) -> Delivery {
    collect(st, cs, true)
}

/// The full folded review, with each thread flagged as already delivered or not.
///
/// Appends nothing — this is the recovery and context path, safe to call at will.
#[must_use]
pub fn all(st: &ReviewState, cs: &Changeset) -> Delivery {
    collect(st, cs, false)
}

/// Deliver everything pending and record that it was delivered.
///
/// The marker records the log's last ordinal rather than the last *feedback*
/// ordinal, so records appended after this point are pending regardless of type.
pub fn drain(log: &Log, st: &ReviewState, cs: &Changeset) -> io::Result<Delivery> {
    let out = undelivered(st, cs);
    if !out.is_empty() {
        log.append(&Record::Drained {
            upto: st.last_ordinal,
        })?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Changeset;
    use crate::review::anchor::capture;
    use crate::review::record::{now, Side};
    use crate::testutil::{changeset, diff_file, opened_review_log};

    const SRC: &str = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n";

    fn cs_with(text: &str) -> Changeset {
        changeset(vec![diff_file("a.rs", Some(text))])
    }

    fn empty_cs() -> Changeset {
        changeset(vec![])
    }

    fn plain(id: &str, body: &str) -> Record {
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

    fn anchored(id: &str) -> Record {
        let a = capture(&diff_file("a.rs", Some(SRC)), Side::New, 3).unwrap();
        Record::Thread(Thread {
            id: id.into(),
            anchor: Some(a),
            body: "on let b".into(),
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

    #[test]
    fn drain_delivers_once() {
        let (_d, log) = opened_review_log();
        log.append(&plain("t1", "hello")).unwrap();
        log.append(&submit(1)).unwrap();

        let first = drain(&log, &log.state().unwrap(), &empty_cs()).unwrap();
        assert_eq!(first.threads.len(), 1);
        assert_eq!(first.submits.len(), 1);

        let second = drain(&log, &log.state().unwrap(), &empty_cs()).unwrap();
        assert!(second.is_empty(), "nothing new: {second:?}");
    }

    #[test]
    fn drain_records_the_boundary_and_writes_nothing_when_empty() {
        let (_d, log) = opened_review_log();
        log.append(&plain("t1", "hello")).unwrap();
        let st = log.state().unwrap();
        let last = st.last_ordinal;
        drain(&log, &st, &empty_cs()).unwrap();
        assert_eq!(log.state().unwrap().drained_upto, last);

        let lines_before = std::fs::read_to_string(log.path()).unwrap().lines().count();
        let out = drain(&log, &log.state().unwrap(), &empty_cs()).unwrap();
        assert!(out.is_empty());
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap().lines().count(),
            lines_before,
            "an empty drain appends no marker"
        );
    }

    #[test]
    fn an_edit_after_a_drain_is_pending_again() {
        let (_d, log) = opened_review_log();
        log.append(&plain("t1", "first")).unwrap();
        drain(&log, &log.state().unwrap(), &empty_cs()).unwrap();

        log.append(&plain("t1", "second")).unwrap();
        let out = drain(&log, &log.state().unwrap(), &empty_cs()).unwrap();
        assert_eq!(out.threads.len(), 1);
        assert_eq!(out.threads[0].thread.body, "second");
    }

    #[test]
    fn all_appends_nothing_and_flags_delivery() {
        let (_d, log) = opened_review_log();
        log.append(&plain("t1", "old")).unwrap();
        drain(&log, &log.state().unwrap(), &empty_cs()).unwrap();
        log.append(&plain("t2", "new")).unwrap();

        let before = std::fs::read_to_string(log.path()).unwrap();
        let everything = all(&log.state().unwrap(), &empty_cs());
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap(),
            before,
            "all() is read-only"
        );

        assert_eq!(everything.threads.len(), 2);
        assert!(everything.threads[0].already_delivered, "t1 was drained");
        assert!(!everything.threads[1].already_delivered, "t2 was not");
    }

    #[test]
    fn a_retracted_thread_is_not_delivered_but_stays_in_the_log() {
        let (_d, log) = opened_review_log();
        log.append(&plain("t1", "oops")).unwrap();
        log.append(&Record::Thread(Thread {
            id: "t1".into(),
            anchor: None,
            body: "oops".into(),
            replace: None,
            resolved: false,
            deleted: true,
            at: now(),
        }))
        .unwrap();

        let st = log.state().unwrap();
        assert!(undelivered(&st, &empty_cs()).threads.is_empty());
        assert!(all(&st, &empty_cs()).threads.is_empty());
        assert!(st.threads.contains_key("t1"), "still folded, still on disk");
    }

    #[test]
    fn a_resolved_thread_is_still_delivered() {
        let (_d, log) = opened_review_log();
        log.append(&Record::Thread(Thread {
            id: "t1".into(),
            anchor: None,
            body: "done with this".into(),
            replace: None,
            resolved: true,
            deleted: false,
            at: now(),
        }))
        .unwrap();
        let out = undelivered(&log.state().unwrap(), &empty_cs());
        assert_eq!(out.threads.len(), 1);
        assert!(out.threads[0].thread.resolved);
    }

    #[test]
    fn anchors_are_resolved_against_the_supplied_changeset() {
        let (_d, log) = opened_review_log();
        log.append(&anchored("t1")).unwrap();
        let st = log.state().unwrap();

        let out = undelivered(&st, &cs_with(SRC));
        assert_eq!(
            out.threads[0].resolution,
            Some(Resolution::Attached { line: 3 })
        );
        assert!(!out.threads[0].is_detached());

        let moved = format!("// added\n{SRC}");
        let out = undelivered(&st, &cs_with(&moved));
        assert_eq!(
            out.threads[0].resolution,
            Some(Resolution::Shifted { from: 3, to: 4 })
        );
    }

    #[test]
    fn a_detached_thread_is_delivered_with_its_evidence() {
        let (_d, log) = opened_review_log();
        log.append(&anchored("t1")).unwrap();

        let out = undelivered(&log.state().unwrap(), &empty_cs());
        assert_eq!(out.threads.len(), 1, "never dropped");
        assert!(out.threads[0].is_detached());
        let anchor = out.threads[0].thread.anchor.as_ref().unwrap();
        assert_eq!(anchor.quote, "    let b = 2;");
        assert_eq!(anchor.before, vec!["fn main() {", "    let a = 1;"]);
    }

    #[test]
    fn all_flags_stale_submits_so_a_recovering_consumer_can_tell() {
        // Regression: submits carried no delivery flag, so a consumer recovering via
        // all() could not distinguish round 1's "revert that" from round 3's "ship
        // it" — and a submit is an instruction, so acting on the stale one is worse
        // than acting on a stale comment.
        let (_d, log) = opened_review_log();
        log.append(&submit(1)).unwrap();
        drain(&log, &log.state().unwrap(), &empty_cs()).unwrap();
        log.append(&submit(2)).unwrap();

        let everything = all(&log.state().unwrap(), &empty_cs());
        assert_eq!(everything.submits.len(), 2);
        assert!(everything.submits[0].already_delivered, "round 1 is stale");
        assert_eq!(everything.submits[0].submit.round, 1);
        assert!(!everything.submits[1].already_delivered, "round 2 is new");

        let pending = undelivered(&log.state().unwrap(), &empty_cs());
        assert_eq!(pending.submits.len(), 1);
        assert_eq!(pending.submits[0].submit.round, 2);
    }

    #[test]
    fn the_cached_resolver_agrees_with_resolve_on_every_outcome() {
        // `collect` splits each file once for speed; that shortcut must not change
        // any answer, least of all by reporting a still-loading file as deleted.
        let (_d, log) = opened_review_log();
        log.append(&anchored("t1")).unwrap();
        let st = log.state().unwrap();

        let mut pending = diff_file("a.rs", None);
        pending.diffed = false;
        let loading = changeset(vec![pending]);
        assert_eq!(
            undelivered(&st, &loading).threads[0].resolution,
            Some(Resolution::Unresolved),
            "still loading, not gone"
        );
        assert!(!undelivered(&st, &loading).threads[0].is_detached());

        // Present and diffed, but that side carries no text (binary, or deleted).
        let no_text = changeset(vec![diff_file("a.rs", None)]);
        assert_eq!(
            undelivered(&st, &no_text).threads[0].resolution,
            Some(Resolution::Detached)
        );
    }

    #[test]
    fn many_anchors_in_one_file_all_resolve() {
        // Exercises the cache's hit path: the second and later anchors on a file
        // reuse the split rather than recomputing it.
        let (_d, log) = opened_review_log();
        for (i, line) in [2u32, 3, 4].iter().enumerate() {
            let a = capture(&diff_file("a.rs", Some(SRC)), Side::New, *line).unwrap();
            log.append(&Record::Thread(Thread {
                id: format!("t{i}"),
                anchor: Some(a),
                body: "c".into(),
                replace: None,
                resolved: false,
                deleted: false,
                at: now(),
            }))
            .unwrap();
        }
        let out = undelivered(&log.state().unwrap(), &cs_with(SRC));
        assert_eq!(out.threads.len(), 3);
        for (t, want) in out.threads.iter().zip([2u32, 3, 4]) {
            assert_eq!(t.resolution, Some(Resolution::Attached { line: want }));
        }
    }

    #[test]
    fn a_review_level_thread_has_no_resolution() {
        let (_d, log) = opened_review_log();
        log.append(&plain("t1", "overall, good")).unwrap();
        let out = undelivered(&log.state().unwrap(), &empty_cs());
        assert_eq!(out.threads[0].resolution, None);
        assert!(!out.threads[0].is_detached());
    }

    #[test]
    fn delivery_is_the_same_whichever_writer_recorded_it() {
        // Two "surfaces" appending to one log; delivery cannot tell them apart.
        let (_d, log) = opened_review_log();
        log.append(&plain("from-tui", "a")).unwrap();
        log.append(&anchored("from-web")).unwrap();

        let out = undelivered(&log.state().unwrap(), &cs_with(SRC));
        let ids: Vec<_> = out
            .threads
            .iter()
            .map(|t| t.thread.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["from-tui", "from-web"]);
    }
}
