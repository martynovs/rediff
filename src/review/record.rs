//! The review log's record schema: one JSON object per line, discriminated by a
//! `t` tag.
//!
//! This is an **on-disk format**. Fields are additive-only — every optional field
//! carries `#[serde(default)]` so a log written by an older build still replays,
//! and `skip_serializing_if` keeps lines narrow enough to read by eye. Renaming or
//! repurposing an existing field is a migration, not a patch.
//!
//! The taxonomy is deliberately flat: there is exactly one feedback record
//! ([`Thread`]), and whether it carries an [`Anchor`] is what makes it a line
//! comment rather than a review-level one. There is no separate "verdict" type —
//! the human's parting instruction for a round is a [`Submit`], which is a
//! lifecycle event (it closes the round) rather than another comment.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// How many lines of context are captured on each side of an anchored line.
///
/// This bounds the anchor's size in the log and the evidence available to
/// re-anchoring: a candidate line is scored by how much of this context also
/// matches around it.
pub const CONTEXT_LINES: usize = 3;

/// Which side of a diff a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The pre-change side (a line with an `old_lineno`).
    Old,
    /// The post-change side (a line with a `new_lineno`).
    New,
}

/// A self-contained pointer to one diff line.
///
/// Self-contained is the point: the anchor carries the line's own text and its
/// surrounding context, so resolving it against a later changeset needs nothing
/// but the anchor and that changeset — no snapshot, no blob store, nothing to
/// garbage-collect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    /// Path of the file, as it appears in the changeset.
    pub path: String,
    /// Which side of the diff `line` numbers into.
    pub side: Side,
    /// 1-based line number on `side` at the time the anchor was captured.
    pub line: u32,
    /// The exact text of the anchored line when it was captured.
    pub quote: String,
    /// Up to [`CONTEXT_LINES`] lines immediately before, nearest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<String>,
    /// Up to [`CONTEXT_LINES`] lines immediately after, nearest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
}

/// One piece of human feedback.
///
/// With an [`Anchor`] it is a comment on that line; without one it is a comment on
/// the review as a whole. Identity is [`id`](Self::id): a later record with the
/// same id supersedes this one, and one with [`deleted`](Self::deleted) retracts
/// it from delivery without removing it from the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// Stable identity across edits. Two threads on one line differ by id.
    pub id: String,
    /// Absent for a review-level comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// The comment text.
    pub body: String,
    /// Replacement text for the anchored line — a suggestion the consumer can
    /// apply verbatim rather than interpret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,
    /// The author marked this thread done. Orthogonal to delivery: resolving
    /// neither deletes the thread nor stops it being delivered.
    #[serde(default, skip_serializing_if = "is_false")]
    pub resolved: bool,
    /// Retracts the thread from delivery. The record stays in the log.
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    /// RFC 3339 timestamp (see [`now`]).
    pub at: String,
}

/// The human's parting instruction for a round, which closes that round.
///
/// [`body`](Self::body) is delivered exactly as written — a caller that pre-fills
/// it from a named preset and then edits it has the *edited* text delivered, with
/// [`preset`](Self::preset) alongside purely so a script can branch without
/// parsing prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submit {
    /// The round this closes.
    pub round: u32,
    /// Name of the preset the body was derived from, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// The instruction, as written.
    pub body: String,
    /// RFC 3339 timestamp (see [`now`]).
    pub at: String,
}

/// One line of the review log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Record {
    /// A review begins. At most one is live per log.
    Open {
        /// Short identifier for the review.
        review: String,
        /// What is under review, e.g. `worktree` or a rev expression.
        target: String,
        /// Human-facing label, e.g. which agent opened it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// RFC 3339 timestamp.
        at: String,
    },
    /// A server process began serving this review.
    Serve {
        /// Process id of the server.
        pid: u32,
        /// Bound loopback port.
        port: u16,
        /// URL the server is reachable at.
        url: String,
    },
    /// A review round opened, with a content hash per changeset file.
    Round {
        /// 1-based round number.
        n: u32,
        /// Path to content hash. Sorted, so the line is stable and diffable.
        files: BTreeMap<String, u64>,
    },
    /// Human feedback.
    Thread(Thread),
    /// A round closed.
    Submit(Submit),
    /// Records that every record up to `upto` has been delivered.
    Drained {
        /// Ordinal (log line number) delivered through, inclusive.
        upto: u64,
    },
    /// A server process stopped.
    Close {
        /// Process id of the server that stopped, when one was serving.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
        /// Why it stopped.
        reason: String,
    },
}

/// The current time as an RFC 3339 string, for a record's `at` field.
///
/// Uses `gix::date`, already a dependency, rather than pulling in a date crate;
/// `format_or_unix` is infallible, so this cannot panic on an odd timezone.
#[must_use]
pub fn now() -> String {
    gix::date::Time::now_utc().format_or_unix(gix::date::time::format::ISO8601_STRICT)
}

/// `skip_serializing_if` predicate for a defaulted bool.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if always passes the field by reference"
)]
fn is_false(b: &bool) -> bool {
    !*b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> Anchor {
        Anchor {
            path: "src/git/diff.rs".into(),
            side: Side::New,
            line: 47,
            quote: "    let mut sink = Sink::new();".into(),
            before: vec!["fn diff() {".into()],
            after: vec!["    drop(sink);".into()],
        }
    }

    fn roundtrip(rec: &Record) -> Record {
        let line = serde_json::to_string(rec).unwrap();
        assert!(
            !line.contains('\n'),
            "a record must serialize to exactly one line: {line}"
        );
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn every_variant_round_trips_as_one_line() {
        let records = vec![
            Record::Open {
                review: "r7k2".into(),
                target: "worktree".into(),
                label: Some("agent".into()),
                at: now(),
            },
            Record::Serve {
                pid: 4411,
                port: 53411,
                url: "http://127.0.0.1:53411/".into(),
            },
            Record::Round {
                n: 1,
                files: BTreeMap::from([("src/a.rs".to_string(), 0x3f9a_u64)]),
            },
            Record::Thread(Thread {
                id: "t1".into(),
                anchor: Some(anchor()),
                body: "reuse the pooled sink".into(),
                replace: Some("    let mut sink = pool.get();".into()),
                resolved: true,
                deleted: false,
                at: now(),
            }),
            Record::Submit(Submit {
                round: 1,
                preset: Some("rework".into()),
                body: "fix t1, then show me again".into(),
                at: now(),
            }),
            Record::Drained { upto: 7 },
            Record::Close {
                pid: Some(4411),
                reason: "drained".into(),
            },
        ];
        for rec in &records {
            assert_eq!(&roundtrip(rec), rec);
        }
    }

    #[test]
    fn review_level_thread_has_no_anchor() {
        let rec = Record::Thread(Thread {
            id: "t2".into(),
            anchor: None,
            body: "looks good overall".into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: now(),
        });
        let line = serde_json::to_string(&rec).unwrap();
        assert!(!line.contains("anchor"), "absent anchor is not serialized");
        assert!(!line.contains("resolved"), "false flags are not serialized");
        assert_eq!(roundtrip(&rec), rec);
    }

    #[test]
    fn absent_optional_fields_read_as_defaults() {
        // The shape an older build would have written: no replace/resolved/deleted.
        let line = r#"{"t":"thread","id":"t3","body":"terse","at":"2026-07-27T00:00:00+00:00"}"#;
        let rec: Record = serde_json::from_str(line).unwrap();
        let Record::Thread(t) = rec else {
            panic!("expected a thread record")
        };
        assert_eq!(t.anchor, None);
        assert_eq!(t.replace, None);
        assert!(!t.resolved);
        assert!(!t.deleted);
    }

    #[test]
    fn anchor_context_defaults_to_empty() {
        let line = r#"{"path":"a.rs","side":"old","line":3,"quote":"x"}"#;
        let a: Anchor = serde_json::from_str(line).unwrap();
        assert_eq!(a.side, Side::Old);
        assert!(a.before.is_empty() && a.after.is_empty());
    }

    #[test]
    fn unknown_tag_does_not_parse_so_replay_can_skip_it() {
        let err = serde_json::from_str::<Record>(r#"{"t":"telepathy","body":"?"}"#);
        assert!(
            err.is_err(),
            "an unknown t is rejected, not silently coerced"
        );
    }

    #[test]
    fn now_is_rfc3339_shaped() {
        let s = now();
        assert!(s.len() >= 20, "unexpectedly short timestamp: {s}");
        assert_eq!(
            s.as_bytes().get(10),
            Some(&b'T'),
            "date/time separator: {s}"
        );
    }

    #[test]
    fn is_false_predicate() {
        assert!(is_false(&false));
        assert!(!is_false(&true));
    }
}
