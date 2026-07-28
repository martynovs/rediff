//! `rediff feedback` — drain the review and emit JSON.
//!
//! **This module defines an agent-facing contract.** The JSON shape below is what
//! a consumer parses; changing a field name or its presence rule is a breaking
//! change to every agent reading it, not a refactor.
//!
//! Two contract details worth stating outright, because both are easy to get
//! wrong from the outside:
//!
//! - Optional fields are **absent keys, not nulls** (`Thread` and `Anchor`
//!   serialize with `skip_serializing_if`). A consumer must test for presence,
//!   not for null.
//! - A thread whose anchor could not be resolved is still emitted, carrying the
//!   line text and context it was recorded against. Nothing is ever dropped for
//!   being stale — that evidence is all that remains of what it referred to.

use serde::Serialize;

use crate::model::Changeset;
use crate::review::{all, undelivered, Anchor, Delivery, Log, Record, Resolution, ReviewState};

/// Where a recorded anchor landed, in wire form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ResolutionJson {
    /// Found at the line it was recorded at.
    Attached {
        /// The line, unchanged.
        line: u32,
    },
    /// Found, but the line moved.
    Shifted {
        /// Where the anchor was recorded.
        from: u32,
        /// Where it is now.
        to: u32,
    },
    /// The file or the line is gone, or the match was too ambiguous to claim.
    Detached,
    /// The file is present but has not been diffed, so this cannot be decided.
    Unresolved,
}

/// Map a resolution to its wire form.
///
/// Standalone, and unit-tested over all four variants, because [`Unresolved`] is
/// unreachable end-to-end: `git::load` always produces a fully-diffed changeset.
/// Buried inside the serializer it would be an untestable arm.
///
/// [`Unresolved`]: Resolution::Unresolved
#[must_use]
pub fn resolution_json(r: Resolution) -> ResolutionJson {
    match r {
        Resolution::Attached { line } => ResolutionJson::Attached { line },
        Resolution::Shifted { from, to } => ResolutionJson::Shifted { from, to },
        Resolution::Detached => ResolutionJson::Detached,
        Resolution::Unresolved => ResolutionJson::Unresolved,
    }
}

/// One thread as emitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadJson {
    /// Stable identity across edits.
    pub id: String,
    /// The comment text.
    pub body: String,
    /// Absent for a review-level comment (which is the verdict).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// Absent for a review-level comment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<ResolutionJson>,
    /// Replacement text to apply verbatim, when the author suggested one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,
    /// The author marked this thread done.
    pub resolved: bool,
    /// A previous drain already delivered this item.
    pub delivered: bool,
}

/// One round-closing instruction as emitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubmitJson {
    /// The round this closed.
    pub round: u32,
    /// The preset the body came from, when one was named.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// The instruction, exactly as the author sent it.
    pub body: String,
    /// A previous drain already delivered this item.
    pub delivered: bool,
}

/// The document `feedback` writes to standard output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeedbackJson {
    /// The review's identifier, absent when no review is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    /// What is under review, absent when no review is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The current round, or 0 when none has been opened.
    pub round: u32,
    /// True when the recorded target could not be resolved, so anchors were
    /// matched against nothing and every one is detached.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub target_unresolvable: bool,
    /// Anchored and review-level comments.
    pub threads: Vec<ThreadJson>,
    /// Round-closing instructions.
    pub submits: Vec<SubmitJson>,
}

/// Build the wire document from a delivery.
#[must_use]
pub fn to_json(st: &ReviewState, delivery: &Delivery, target_unresolvable: bool) -> FeedbackJson {
    FeedbackJson {
        review: st.open.as_ref().map(|o| o.review.clone()),
        target: st.open.as_ref().map(|o| o.target.clone()),
        round: st.rounds.last().map_or(0, |r| r.n),
        target_unresolvable,
        threads: delivery
            .threads
            .iter()
            .map(|d| ThreadJson {
                id: d.thread.id.clone(),
                body: d.thread.body.clone(),
                anchor: d.thread.anchor.clone(),
                resolution: d.resolution.map(resolution_json),
                replace: d.thread.replace.clone(),
                resolved: d.thread.resolved,
                delivered: d.already_delivered,
            })
            .collect(),
        submits: delivery
            .submits
            .iter()
            .map(|s| SubmitJson {
                round: s.submit.round,
                preset: s.submit.preset.clone(),
                body: s.submit.body.clone(),
                delivered: s.already_delivered,
            })
            .collect(),
    }
}

/// Build the wire document **without recording anything**.
///
/// Deliberately does not drain. Marking feedback delivered and then failing to
/// write it is silent data loss: `rediff feedback | head -1` (or any consumer that
/// exits early) would panic on the broken pipe *after* the marker had been
/// appended, and the comments would be gone from every later run. The caller
/// writes the document first and only then calls [`mark_delivered`].
///
/// `target_unresolvable` is the caller's report that the recorded target no longer
/// resolves, in which case it passes an empty changeset: every anchor lands
/// detached but the feedback is still delivered. Failing instead would deadlock
/// the worktree — `--all` needs a changeset too, and a request for another target
/// is refused while feedback is pending.
#[must_use]
pub fn collect(
    st: &ReviewState,
    cs: &Changeset,
    replay_all: bool,
    target_unresolvable: bool,
) -> FeedbackJson {
    let delivery = if replay_all {
        all(st, cs)
    } else {
        undelivered(st, cs)
    };
    to_json(st, &delivery, target_unresolvable)
}

/// Record that everything up to the log's current end has been delivered.
///
/// Call only after the document has been written successfully. A no-op when the
/// document carried nothing, so an empty drain appends no marker.
///
/// # Errors
/// Returns the log's error if the marker cannot be appended.
pub fn mark_delivered(log: &Log, st: &ReviewState, doc: &FeedbackJson) -> std::io::Result<()> {
    if doc.threads.is_empty() && doc.submits.is_empty() {
        return Ok(());
    }
    // Never mark past a line this build could not read. `Drained` is by ordinal,
    // so doing so would hide a record a newer rediff understands — permanently,
    // and without any way to notice. Stopping short costs at most a redelivery.
    let upto = st
        .first_unparsed
        .map_or(st.last_ordinal, |u| u.saturating_sub(1));
    if upto == 0 {
        return Ok(());
    }
    log.append(&Record::Drained { upto }).map(|_created| ())
}

/// Render a document as the JSON `feedback` writes.
///
/// # Errors
/// Returns the serializer's error; the shape is plain data.
pub fn render(doc: &FeedbackJson) -> Result<String, serde_json::Error> {
    let mut s = serde_json::to_string_pretty(doc)?;
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draining_never_marks_past_a_line_this_build_cannot_read() {
        // The record may be feedback a newer rediff understands. `Drained` is by
        // ordinal, so marking past it would hide it from that build forever.
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at_worktree(dir.path());
        log.append(&Record::Open {
            review: "r1".into(),
            target: "worktree".into(),
            label: None,
            at: crate::review::now(),
        })
        .unwrap();
        log.append(&Record::Thread(crate::review::Thread {
            id: "t1".into(),
            anchor: None,
            body: "readable".into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: crate::review::now(),
        }))
        .unwrap();
        // A line this build cannot read at all — torn by a crash mid-append.
        // (A *well-formed* record from a newer rediff now reads as
        // `Record::Unknown` and is deliberately not counted unparsed; damage is
        // the case that still has to stop a drain.)
        {
            use std::io::Write as _;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(log.path())
                .unwrap();
            writeln!(f, r#"{{"t":"thread","id":"rp1","body":"half a li"#).unwrap();
        }

        let st = log.state().unwrap();
        assert_eq!(st.first_unparsed, Some(3), "the third line is unreadable");
        let doc = collect(&st, &no_changeset(), false, false);
        mark_delivered(&log, &st, &doc).unwrap();

        let after = log.state().unwrap();
        assert_eq!(
            after.drained_upto, 2,
            "stopped before the unreadable line, not at the end"
        );
    }

    /// An empty changeset, for anchor-free drains.
    fn no_changeset() -> crate::model::Changeset {
        crate::model::Changeset {
            source: String::new(),
            files: Vec::new(),
        }
    }
    use crate::review::{capture, Record, Side, Submit, Thread};
    use crate::testutil::{changeset, diff_file, opened_review_log};

    const SRC: &str = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n";

    fn cs_with(text: &str) -> Changeset {
        changeset(vec![diff_file("a.rs", Some(text))])
    }

    fn empty_cs() -> Changeset {
        changeset(vec![])
    }

    fn anchored(id: &str) -> Record {
        Record::Thread(Thread {
            id: id.into(),
            anchor: Some(capture(&diff_file("a.rs", Some(SRC)), Side::New, 3).unwrap()),
            body: "on let b".into(),
            replace: Some("    let b = 3;".into()),
            resolved: false,
            deleted: false,
            at: crate::review::now(),
        })
    }

    fn plain(id: &str) -> Record {
        Record::Thread(Thread {
            id: id.into(),
            anchor: None,
            body: "ship it".into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: crate::review::now(),
        })
    }

    fn submit(round: u32) -> Record {
        Record::Submit(Submit {
            round,
            preset: Some("rework".into()),
            body: "fix t1 then show me".into(),
            at: crate::review::now(),
        })
    }

    fn value(doc: &FeedbackJson) -> serde_json::Value {
        serde_json::from_str(&render(doc).unwrap()).expect("parses")
    }

    #[test]
    fn resolution_json_covers_every_variant() {
        // Unresolved is unreachable end-to-end, so this is its only coverage.
        assert_eq!(
            resolution_json(Resolution::Attached { line: 7 }),
            ResolutionJson::Attached { line: 7 }
        );
        assert_eq!(
            resolution_json(Resolution::Shifted { from: 3, to: 9 }),
            ResolutionJson::Shifted { from: 3, to: 9 }
        );
        assert_eq!(
            resolution_json(Resolution::Detached),
            ResolutionJson::Detached
        );
        assert_eq!(
            resolution_json(Resolution::Unresolved),
            ResolutionJson::Unresolved
        );

        let v = serde_json::to_value(ResolutionJson::Shifted { from: 3, to: 9 }).unwrap();
        assert_eq!(v["state"], "shifted");
        assert_eq!(v["from"], 3);
        assert_eq!(v["to"], 9);
    }

    #[test]
    fn drain_delivers_once_then_nothing() {
        let (_d, log) = opened_review_log();
        log.append(&anchored("t1")).unwrap();
        log.append(&submit(1)).unwrap();

        let st = log.state().unwrap();
        let first = collect(&st, &cs_with(SRC), false, false);
        mark_delivered(&log, &st, &first).unwrap();
        assert_eq!(first.threads.len(), 1);
        assert_eq!(first.submits.len(), 1);

        let st = log.state().unwrap();
        let second = collect(&st, &cs_with(SRC), false, false);
        assert!(second.threads.is_empty() && second.submits.is_empty());
    }

    #[test]
    fn replay_all_leaves_the_log_byte_identical() {
        let (_d, log) = opened_review_log();
        log.append(&plain("t1")).unwrap();
        log.append(&submit(1)).unwrap();
        let st = log.state().unwrap();
        {
            let d = collect(&st, &empty_cs(), false, false);
            mark_delivered(&log, &st, &d).unwrap();
        }

        let before = std::fs::read_to_string(log.path()).unwrap();
        let st = log.state().unwrap();
        let doc = collect(&st, &empty_cs(), true, false);
        assert_eq!(std::fs::read_to_string(log.path()).unwrap(), before);

        assert_eq!(doc.threads.len(), 1);
        assert!(doc.threads[0].delivered, "flagged as already delivered");
        assert!(doc.submits[0].delivered, "so is the instruction");
    }

    #[test]
    fn a_shifted_anchor_reports_both_positions() {
        let (_d, log) = opened_review_log();
        log.append(&anchored("t1")).unwrap();
        let st = log.state().unwrap();
        let doc = collect(&st, &cs_with(&format!("// added\n{SRC}")), false, false);
        assert_eq!(
            doc.threads[0].resolution,
            Some(ResolutionJson::Shifted { from: 3, to: 4 })
        );
        let v = value(&doc);
        assert_eq!(v["threads"][0]["resolution"]["state"], "shifted");
    }

    #[test]
    fn a_detached_anchor_keeps_its_evidence() {
        let (_d, log) = opened_review_log();
        log.append(&anchored("t1")).unwrap();
        let st = log.state().unwrap();
        let doc = collect(&st, &empty_cs(), false, false);
        mark_delivered(&log, &st, &doc).unwrap();

        assert_eq!(doc.threads[0].resolution, Some(ResolutionJson::Detached));
        let anchor = doc.threads[0].anchor.as_ref().unwrap();
        assert_eq!(anchor.quote, "    let b = 2;");
        assert_eq!(anchor.before, vec!["fn main() {", "    let a = 1;"]);
        assert_eq!(doc.threads[0].replace.as_deref(), Some("    let b = 3;"));
    }

    #[test]
    fn an_unresolvable_target_still_delivers_its_feedback() {
        let (_d, log) = opened_review_log();
        log.append(&anchored("t1")).unwrap();
        let st = log.state().unwrap();
        // What the shell does when git::load fails: empty changeset, flag set.
        let doc = collect(&st, &empty_cs(), false, true);

        assert!(doc.target_unresolvable);
        assert_eq!(doc.threads.len(), 1, "never a deadlock");
        assert_eq!(doc.threads[0].resolution, Some(ResolutionJson::Detached));
        assert_eq!(value(&doc)["target_unresolvable"], true);
    }

    #[test]
    fn a_review_level_thread_has_no_anchor_or_resolution() {
        let (_d, log) = opened_review_log();
        log.append(&plain("verdict")).unwrap();
        let st = log.state().unwrap();
        let doc = collect(&st, &empty_cs(), false, false);
        mark_delivered(&log, &st, &doc).unwrap();

        assert_eq!(doc.threads[0].anchor, None);
        assert_eq!(doc.threads[0].resolution, None);
        let v = value(&doc);
        assert!(v["threads"][0].get("anchor").is_none(), "absent, not null");
        assert!(v["threads"][0].get("resolution").is_none());
        assert!(v["threads"][0].get("replace").is_none());
    }

    #[test]
    fn optional_fields_are_absent_keys_not_nulls() {
        // The agent-facing contract: consumers test presence, not null.
        let (_d, log) = opened_review_log();
        log.append(&plain("t1")).unwrap();
        let st = log.state().unwrap();
        let text = render(&collect(&st, &empty_cs(), false, false)).unwrap();
        assert!(!text.contains("null"), "no nulls in the contract:\n{text}");
        assert!(
            !text.contains("target_unresolvable"),
            "a false flag is omitted too"
        );
    }

    #[test]
    fn no_review_open_yields_an_empty_document() {
        let (_d, log) = crate::testutil::review_log();
        let st = log.state().unwrap();
        let doc = collect(&st, &empty_cs(), false, false);
        mark_delivered(&log, &st, &doc).unwrap();
        assert_eq!(doc.review, None);
        assert_eq!(doc.round, 0);
        assert!(doc.threads.is_empty() && doc.submits.is_empty());

        let v = value(&doc);
        assert!(v.get("review").is_none());
        assert_eq!(v["threads"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn the_document_carries_the_review_and_round() {
        let (_d, log) = opened_review_log();
        log.append(&Record::Round {
            n: 2,
            files: std::collections::BTreeMap::new(),
        })
        .unwrap();
        log.append(&plain("t1")).unwrap();
        let st = log.state().unwrap();
        let doc = collect(&st, &empty_cs(), false, false);
        mark_delivered(&log, &st, &doc).unwrap();
        assert_eq!(doc.review.as_deref(), Some("r1"));
        assert_eq!(doc.target.as_deref(), Some("worktree"));
        assert_eq!(doc.round, 2);
    }
}
