//! `rediff review-status` — what state is this worktree's review in?
//!
//! Named apart from `review` (which opens the TUI) so neither reads as the other,
//! and singular because there is one review per worktree.
//!
//! The two renderers share one collected struct, so the human-readable and JSON
//! forms cannot drift apart.

use serde::Serialize;

use crate::review::{last_serve, Log, ReviewState, ServeState};

/// The review's state, as both renderers see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The log's path, whether or not it exists.
    pub log: String,
    /// Absent when no review has been opened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewSummary>,
}

/// An open review's summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewSummary {
    /// Short identifier.
    pub id: String,
    /// What is under review, in its canonical encoded form.
    pub target: String,
    /// Human-facing label, when one was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The current round number, or 0 when none has been opened.
    pub round: u32,
    /// Feedback items not yet delivered — threads **and** round-closing
    /// instructions, since a consumer must act on both.
    pub pending: usize,
    /// The URL a server recorded, when its start has no matching stop.
    ///
    /// Reported without any claim that the process is still running: the store
    /// records the serve lifecycle and deliberately does not probe it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serving: Option<String>,
}

/// Collect the review's state from a folded log.
#[must_use]
pub fn collect(st: &ReviewState, log: &Log) -> Status {
    Status {
        log: log.path().display().to_string(),
        review: st.open.as_ref().map(|open| ReviewSummary {
            id: open.review.clone(),
            target: open.target.clone(),
            label: open.label.clone(),
            round: st.rounds.last().map_or(0, |r| r.n),
            pending: st.pending_ordinals().count(),
            serving: last_serve(st)
                .filter(ServeState::is_unpaired)
                .map(|s| s.url),
        }),
    }
}

/// Render for a human.
#[must_use]
pub fn render_human(status: &Status) -> String {
    let Some(r) = status.review.as_ref() else {
        return format!("no review open · {}\n", status.log);
    };
    let mut lines = vec![match r.label.as_deref() {
        Some(label) => format!("review {} · {} · {label}", r.id, r.target),
        None => format!("review {} · {}", r.id, r.target),
    }];
    lines.push(format!("round {} · {} pending", r.round, r.pending));
    if let Some(url) = r.serving.as_deref() {
        lines.push(format!("last served at {url}"));
    }
    lines.push(status.log.clone());
    lines.join("\n") + "\n"
}

/// Render as JSON.
///
/// # Errors
/// Returns the serializer's error; the shape is plain data, so this is
/// effectively infallible.
pub fn render_json(status: &Status) -> Result<String, serde_json::Error> {
    let mut s = serde_json::to_string_pretty(status)?;
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::{record_close, record_serve, Record, Submit, Thread};
    use crate::testutil::{opened_review_log, review_log};

    fn thread(id: &str) -> Record {
        Record::Thread(Thread {
            id: id.into(),
            anchor: None,
            body: "b".into(),
            replace: None,
            resolved: false,
            deleted: false,
            at: crate::review::now(),
        })
    }

    #[test]
    fn no_log_at_all_reports_no_review() {
        let (_d, log) = review_log();
        let st = log.state().unwrap();
        let status = collect(&st, &log);
        assert_eq!(status.review, None);
        assert!(render_human(&status).starts_with("no review open"));
        assert!(render_human(&status).contains("rediff.jsonl"));
    }

    #[test]
    fn a_log_with_no_review_reports_no_review() {
        let (_d, log) = review_log();
        log.append(&Record::Drained { upto: 1 }).unwrap();
        let status = collect(&log.state().unwrap(), &log);
        assert_eq!(status.review, None);
    }

    #[test]
    fn an_open_review_is_summarized() {
        let (_d, log) = opened_review_log();
        log.append(&thread("t1")).unwrap();
        log.append(&Record::Submit(Submit {
            round: 1,
            preset: None,
            body: "again".into(),
            at: crate::review::now(),
        }))
        .unwrap();

        let r = collect(&log.state().unwrap(), &log).review.unwrap();
        assert_eq!(r.id, "r1");
        assert_eq!(r.target, "worktree");
        assert_eq!(r.round, 0, "no round opened yet");
        assert_eq!(
            r.pending, 2,
            "the thread and the submit both await delivery"
        );
        assert_eq!(r.serving, None);
    }

    #[test]
    fn the_round_number_comes_from_the_last_round() {
        let (_d, log) = opened_review_log();
        for n in 1..=3 {
            log.append(&Record::Round {
                n,
                files: std::collections::BTreeMap::new(),
            })
            .unwrap();
        }
        assert_eq!(
            collect(&log.state().unwrap(), &log).review.unwrap().round,
            3
        );
    }

    #[test]
    fn an_unpaired_serve_reports_its_url() {
        let (_d, log) = opened_review_log();
        record_serve(&log, 4411, 53411, "http://127.0.0.1:53411/").unwrap();
        let r = collect(&log.state().unwrap(), &log).review.unwrap();
        assert_eq!(r.serving.as_deref(), Some("http://127.0.0.1:53411/"));
        assert!(render_human(&collect(&log.state().unwrap(), &log)).contains("last served at"));

        // A stop clears it — and note we never claimed the process was alive.
        record_close(&log, Some(4411), "drained").unwrap();
        let r = collect(&log.state().unwrap(), &log).review.unwrap();
        assert_eq!(r.serving, None);
    }

    #[test]
    fn human_output_includes_the_label_when_present() {
        let (_d, log) = review_log();
        log.append(&Record::Open {
            review: "rzz".into(),
            target: "worktree:main".into(),
            label: Some("agent-a".into()),
            at: crate::review::now(),
        })
        .unwrap();
        let text = render_human(&collect(&log.state().unwrap(), &log));
        assert!(
            text.contains("review rzz · worktree:main · agent-a"),
            "{text}"
        );
        assert!(text.contains("round 0 · 0 pending"), "{text}");
    }

    #[test]
    fn json_is_parseable_and_omits_absent_fields() {
        let (_d, log) = opened_review_log();
        let json = render_json(&collect(&log.state().unwrap(), &log)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).expect("parses");
        assert_eq!(v["review"]["target"], "worktree");
        assert_eq!(v["review"]["pending"], 0);
        assert!(v["review"].get("label").is_none(), "absent, not null");
        assert!(v["review"].get("serving").is_none(), "absent, not null");
    }

    #[test]
    fn json_omits_the_review_entirely_when_there_is_none() {
        let (_d, log) = review_log();
        let json = render_json(&collect(&log.state().unwrap(), &log)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("review").is_none());
        assert!(v["log"].as_str().unwrap().ends_with("rediff.jsonl"));
    }
}
