//! The serve lifecycle: recording that a server started and that it stopped.
//!
//! The store records these facts and stops there. It does **not** probe whether a
//! recorded process is still running, for two reasons. The workspace denies
//! `unsafe_code`, so the obvious `kill(pid, 0)` is unavailable here; and more
//! importantly a pid is a poor liveness signal — pids are recycled, so a stale
//! record can match an unrelated live process, and probing another user's process
//! reports "alive" via `EPERM`. Both mistakes hand a caller a URL pointing at
//! nothing.
//!
//! Deciding "is a server already up, and should I rebind?" belongs next to the code
//! that owns server processes, where the port is in hand and a health check is one
//! request away.

use std::io;

use super::log::{Log, ReviewState};
use super::record::Record;

/// The most recent serve, and whether a stop for it followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeState {
    /// Process id recorded at start.
    pub pid: u32,
    /// Bound loopback port.
    pub port: u16,
    /// URL the server reported.
    pub url: String,
    /// Reason from a stop record for this same pid, or `None` while unpaired.
    ///
    /// Unpaired means only "no stop was recorded" — it is *not* a claim that the
    /// process is running.
    pub closed: Option<String>,
}

impl ServeState {
    /// Whether no stop record has been paired with this start.
    #[must_use]
    pub fn is_unpaired(&self) -> bool {
        self.closed.is_none()
    }
}

/// Record that a server started.
pub fn record_serve(log: &Log, pid: u32, port: u16, url: &str) -> io::Result<()> {
    log.append(&Record::Serve {
        pid,
        port,
        url: url.to_string(),
    })
    .map(|_created| ())
}

/// Record that a server stopped.
pub fn record_close(log: &Log, pid: Option<u32>, reason: &str) -> io::Result<()> {
    log.append(&Record::Close {
        pid,
        reason: reason.to_string(),
    })
    .map(|_created| ())
}

/// The most recent serve in the folded state, paired with its stop when one
/// followed it carrying the same pid.
#[must_use]
pub fn last_serve(st: &ReviewState) -> Option<ServeState> {
    let (_, serve) = st.serve.as_ref()?;
    let closed = st
        .close
        .as_ref()
        .filter(|c| c.pid == Some(serve.pid))
        .map(|c| c.reason.clone());
    Some(ServeState {
        pid: serve.pid,
        port: serve.port,
        url: serve.url.clone(),
        closed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::opened_review_log;

    #[test]
    fn no_serve_recorded_means_none() {
        let (_d, log) = opened_review_log();
        assert_eq!(last_serve(&log.state().unwrap()), None);
    }

    #[test]
    fn start_records_pid_port_and_url_and_reports_unpaired() {
        let (_d, log) = opened_review_log();
        record_serve(&log, 4411, 53411, "http://127.0.0.1:53411/").unwrap();

        let s = last_serve(&log.state().unwrap()).unwrap();
        assert_eq!(s.pid, 4411);
        assert_eq!(s.port, 53411);
        assert_eq!(s.url, "http://127.0.0.1:53411/");
        assert_eq!(s.closed, None);
        assert!(s.is_unpaired());
    }

    #[test]
    fn a_stop_for_the_same_pid_pairs_and_carries_its_reason() {
        let (_d, log) = opened_review_log();
        record_serve(&log, 4411, 53411, "http://127.0.0.1:53411/").unwrap();
        record_close(&log, Some(4411), "drained").unwrap();

        let s = last_serve(&log.state().unwrap()).unwrap();
        assert_eq!(s.closed.as_deref(), Some("drained"));
        assert!(!s.is_unpaired());
    }

    #[test]
    fn a_stop_for_a_different_pid_does_not_pair() {
        let (_d, log) = opened_review_log();
        record_serve(&log, 4411, 53411, "http://127.0.0.1:53411/").unwrap();
        record_close(&log, Some(9999), "some other process").unwrap();

        let s = last_serve(&log.state().unwrap()).unwrap();
        assert!(s.is_unpaired(), "pairing is by pid");
    }

    #[test]
    fn a_pidless_stop_does_not_pair_with_a_serve() {
        // A TUI-only review can close without any server having run.
        let (_d, log) = opened_review_log();
        record_serve(&log, 4411, 53411, "http://127.0.0.1:53411/").unwrap();
        record_close(&log, None, "review ended in the TUI").unwrap();
        assert!(last_serve(&log.state().unwrap()).unwrap().is_unpaired());
    }

    #[test]
    fn a_later_start_supersedes_a_stopped_one_and_is_unpaired() {
        let (_d, log) = opened_review_log();
        record_serve(&log, 4411, 53411, "http://127.0.0.1:53411/").unwrap();
        record_close(&log, Some(4411), "restart").unwrap();
        record_serve(&log, 5522, 53412, "http://127.0.0.1:53412/").unwrap();

        let s = last_serve(&log.state().unwrap()).unwrap();
        assert_eq!(s.pid, 5522);
        assert_eq!(s.port, 53412);
        assert!(s.is_unpaired(), "the earlier close does not describe it");
    }
}
