//! Rounds: a per-file content fingerprint of the changeset at the moment a review
//! pass opened.
//!
//! A round stores *hashes*, never content. That is what lets the log stay a single
//! readable, deletable file: comparing a round against a later changeset answers
//! "which files moved since my last pass" — enough to direct attention on a
//! re-review — at roughly fifty bytes per file, with nothing to garbage-collect.
//!
//! What it deliberately cannot answer is *which lines* moved. Storing each round's
//! patch would buy that, at patch-size × rounds; it is an additive field if the
//! need shows up, not a format break.

use std::collections::BTreeMap;
use std::io;

use xxhash_rust::xxh3::xxh3_64;

use super::log::{Log, ReviewState, RoundInfo};
use super::record::Record;
use crate::model::Changeset;

/// Hash recorded for a file whose reviewed content is unavailable — a deletion, or
/// a binary file the loader carries no text for.
///
/// A fixed sentinel rather than the hash of an empty string, so a deleted file and
/// an emptied one are distinguishable. A real hash colliding with it would cost a
/// missed highlight, not correctness.
pub const NO_CONTENT: u64 = 0xffff_ffff_ffff_ffff;

/// The review store's content hash: **XXH3-64**.
///
/// The variant is part of the on-disk format — a round recorded by one build must
/// compare equal in another. Never substitute a hasher whose output is unspecified
/// across versions (`std`'s `DefaultHasher`, `rustc-hash`); doing so would silently
/// invalidate every recorded round on a toolchain bump.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> u64 {
    xxh3_64(bytes)
}

/// Fingerprint one file's reviewed (new-side) content.
///
/// Prefers `content_digest`, taken from the raw bytes at diff time, so a **binary**
/// file — which carries no text — is fingerprinted like any other and a changed one
/// is actually reported as changed. Falls back to hashing the text for fixtures and
/// any caller that builds a `DiffFile` by hand.
///
/// An **undiffed** file yields `None`: its content is not absent, merely not
/// computed yet, and the two must not be conflated.
fn file_hash(f: &crate::model::DiffFile) -> Option<u64> {
    if !f.diffed {
        return None;
    }
    Some(match (f.content_digest, f.new_text.as_deref()) {
        (Some(d), _) => d,
        (None, Some(t)) => content_hash(t.as_bytes()),
        (None, None) => NO_CONTENT,
    })
}

/// Fingerprint every file in a changeset by its reviewed (new-side) content.
///
/// Undiffed files are omitted rather than hashed — see [`file_hash`]. Callers that
/// need a complete fingerprint should check [`Changeset::fully_diffed`] first;
/// [`open_round`] does.
#[must_use]
pub fn hash_changeset(cs: &Changeset) -> BTreeMap<String, u64> {
    cs.files
        .iter()
        .filter_map(|f| file_hash(f).map(|h| (f.path.clone(), h)))
        .collect()
}

/// Files that differ between a recorded round and a later changeset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changed {
    /// Present in both, with different content.
    pub modified: Vec<String>,
    /// Present now, absent from the round.
    pub added: Vec<String>,
    /// Present in the round, absent now.
    pub removed: Vec<String>,
}

impl Changed {
    /// Whether anything moved at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modified.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }

    /// How many files moved.
    #[must_use]
    pub fn len(&self) -> usize {
        self.modified.len() + self.added.len() + self.removed.len()
    }
}

/// Compare a recorded round against the current changeset.
///
/// A file still being diffed is reported in no bucket: it is absent from the
/// current fingerprint, but calling it *removed* would be a lie about a file that
/// is merely still loading.
#[must_use]
pub fn changed_since(round: &RoundInfo, cs: &Changeset) -> Changed {
    let now = hash_changeset(cs);
    let undiffed: std::collections::BTreeSet<&str> = cs
        .files
        .iter()
        .filter(|f| !f.diffed)
        .map(|f| f.path.as_str())
        .collect();

    let mut out = Changed::default();
    for (path, hash) in &now {
        match round.files.get(path) {
            None => out.added.push(path.clone()),
            Some(before) if before != hash => out.modified.push(path.clone()),
            Some(_) => {}
        }
    }
    for path in round.files.keys() {
        if !now.contains_key(path) && !undiffed.contains(path.as_str()) {
            out.removed.push(path.clone());
        }
    }
    out
}

/// Open the next round over `cs`, appending a `round` record.
///
/// Returns the existing round, writing nothing, in two cases:
///
/// - a **frozen** target (a commit or range) that cannot change under the reviewer,
///   which therefore has exactly one round; and
/// - a changeset in which **nothing moved** since the last round, so a surface that
///   re-enumerates on every refresh does not append a near-identical record each
///   time to a file whose whole premise is that it stays readable.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] if `cs` is not fully diffed. Hashing a
/// half-loaded changeset would record a fingerprint that every later comparison
/// reports as wholly changed.
pub fn open_round(log: &Log, st: &ReviewState, cs: &Changeset, frozen: bool) -> io::Result<u32> {
    if !cs.fully_diffed() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot open a review round over a changeset that is still being diffed",
        ));
    }
    if let Some(existing) = st.rounds.last() {
        if frozen || changed_since(existing, cs).is_empty() {
            return Ok(existing.n);
        }
    }
    let n = st.rounds.last().map_or(1, |r| r.n + 1);
    log.append(&Record::Round {
        n,
        files: hash_changeset(cs),
    })?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{changeset, diff_file, opened_review_log};

    #[test]
    fn hash_is_stable_across_calls_and_matches_a_known_vector() {
        // XXH3-64 of the empty input is a published constant; if this ever changes,
        // every recorded round in the wild has been invalidated.
        assert_eq!(content_hash(b""), 0x2d06_8005_38d3_94c2);
        assert_eq!(content_hash(b"rediff"), content_hash(b"rediff"));
        assert_ne!(content_hash(b"rediff"), content_hash(b"rediff "));
    }

    #[test]
    fn absent_content_hashes_to_the_sentinel_not_to_empty() {
        let cs = changeset(vec![
            diff_file("gone.rs", None),
            diff_file("empty.rs", Some("")),
        ]);
        let h = hash_changeset(&cs);
        assert_eq!(h["gone.rs"], NO_CONTENT);
        assert_eq!(h["empty.rs"], content_hash(b""));
        assert_ne!(h["gone.rs"], h["empty.rs"]);
    }

    #[test]
    fn unchanged_files_are_not_reported() {
        let cs = changeset(vec![
            diff_file("a.rs", Some("one")),
            diff_file("b.rs", Some("two")),
        ]);
        let round = RoundInfo {
            n: 1,
            files: hash_changeset(&cs),
        };
        let again = changed_since(&round, &cs);
        assert!(again.is_empty(), "nothing moved: {again:?}");
        assert_eq!(again.len(), 0);
    }

    #[test]
    fn modified_added_and_removed_land_in_the_right_bucket() {
        let before = changeset(vec![
            diff_file("keep.rs", Some("same")),
            diff_file("gone.rs", Some("x")),
        ]);
        let round = RoundInfo {
            n: 1,
            files: hash_changeset(&before),
        };
        let after = changeset(vec![
            diff_file("keep.rs", Some("same")),
            diff_file("edited.rs", Some("new")),
        ]);

        // keep.rs is unchanged, gone.rs disappeared, edited.rs appeared.
        let ch = changed_since(&round, &after);
        assert_eq!(ch.added, vec!["edited.rs"]);
        assert_eq!(ch.removed, vec!["gone.rs"]);
        assert!(ch.modified.is_empty());
        assert_eq!(ch.len(), 2);

        // Now edit a file that exists in both.
        let after = changeset(vec![diff_file("keep.rs", Some("different"))]);
        let ch = changed_since(&round, &after);
        assert_eq!(ch.modified, vec!["keep.rs"]);
        assert_eq!(ch.removed, vec!["gone.rs"]);
        assert!(ch.added.is_empty());
    }

    #[test]
    fn rounds_increment_when_the_changeset_moves() {
        let (_d, log) = opened_review_log();
        let first = changeset(vec![diff_file("a.rs", Some("one"))]);
        let second = changeset(vec![diff_file("a.rs", Some("two"))]);

        let st = log.state().unwrap();
        assert_eq!(open_round(&log, &st, &first, false).unwrap(), 1);
        let st = log.state().unwrap();
        assert_eq!(open_round(&log, &st, &second, false).unwrap(), 2);
        assert_eq!(log.state().unwrap().rounds.len(), 2);
    }

    #[test]
    fn an_unchanged_changeset_does_not_open_a_second_round() {
        // A surface that re-enumerates on every refresh would otherwise append a
        // near-identical ~50-bytes-per-file record each time, to a file whose whole
        // premise is that it stays readable.
        let (_d, log) = opened_review_log();
        let cs = changeset(vec![diff_file("a.rs", Some("one"))]);

        let st = log.state().unwrap();
        assert_eq!(open_round(&log, &st, &cs, false).unwrap(), 1);
        for _ in 0..5 {
            let st = log.state().unwrap();
            assert_eq!(
                open_round(&log, &st, &cs, false).unwrap(),
                1,
                "still round 1"
            );
        }
        assert_eq!(log.state().unwrap().rounds.len(), 1, "one record, not six");
    }

    #[test]
    fn a_changed_binary_is_reported_as_changed() {
        // Regression: binary files carry no text, so hashing `new_text` gave every
        // one the same sentinel and a replaced image looked untouched.
        let mut before = diff_file("logo.png", None);
        before.is_binary = true;
        before.content_digest = Some(content_hash(b"\x89PNG old bytes"));
        let round = RoundInfo {
            n: 1,
            files: hash_changeset(&changeset(vec![before])),
        };

        let mut after = diff_file("logo.png", None);
        after.is_binary = true;
        after.content_digest = Some(content_hash(b"\x89PNG new bytes"));
        let ch = changed_since(&round, &changeset(vec![after.clone()]));
        assert_eq!(ch.modified, vec!["logo.png"]);

        // And an untouched binary reports as untouched — the property that would
        // catch `content_digest` becoming non-deterministic or being dropped for
        // binaries. (This previously rebuilt nothing and re-asserted the case
        // above, so it could not have failed.)
        let unchanged_round = RoundInfo {
            n: 2,
            files: hash_changeset(&changeset(vec![after.clone()])),
        };
        let same = changed_since(&unchanged_round, &changeset(vec![after]));
        assert!(
            same.is_empty(),
            "an untouched binary must not be reported: {same:?}"
        );
    }

    #[test]
    fn an_undiffed_changeset_cannot_open_a_round() {
        let (_d, log) = opened_review_log();
        let mut stub = diff_file("a.rs", Some("one"));
        stub.diffed = false;
        let cs = changeset(vec![diff_file("b.rs", Some("done")), stub]);

        let st = log.state().unwrap();
        let err = open_round(&log, &st, &cs, false).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(log.state().unwrap().rounds.is_empty(), "nothing recorded");
    }

    #[test]
    fn a_file_still_being_diffed_is_not_reported_as_removed() {
        let loaded = changeset(vec![diff_file("a.rs", Some("one"))]);
        let round = RoundInfo {
            n: 1,
            files: hash_changeset(&loaded),
        };

        let mut pending = diff_file("a.rs", None);
        pending.diffed = false;
        let ch = changed_since(&round, &changeset(vec![pending]));
        assert!(
            ch.is_empty(),
            "still loading is not the same as deleted: {ch:?}"
        );
    }

    #[test]
    fn a_frozen_target_opens_exactly_one_round() {
        let (_d, log) = opened_review_log();
        let cs = changeset(vec![diff_file("a.rs", Some("one"))]);

        let st = log.state().unwrap();
        assert_eq!(open_round(&log, &st, &cs, true).unwrap(), 1);
        let st = log.state().unwrap();
        assert_eq!(
            open_round(&log, &st, &cs, true).unwrap(),
            1,
            "still round 1"
        );
        assert_eq!(
            log.state().unwrap().rounds.len(),
            1,
            "and no second record was written"
        );
    }

    #[test]
    fn a_round_records_hashes_not_content() {
        let (_d, log) = opened_review_log();
        let cs = changeset(vec![diff_file("a.rs", Some("secret contents here"))]);
        let st = log.state().unwrap();
        open_round(&log, &st, &cs, false).unwrap();

        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(
            !text.contains("secret contents here"),
            "file content must never reach the log"
        );
        assert_eq!(log.state().unwrap().rounds[0].files.len(), 1);
    }
}
