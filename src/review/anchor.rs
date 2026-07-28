//! Capturing an anchor, and resolving one against a later changeset.
//!
//! An anchor is self-contained: the line's own text plus a little surrounding
//! context. Resolving it needs nothing else — no snapshot of the file as it was,
//! no blob store, nothing to keep or clean up. The cost is that resolution is a
//! search rather than a lookup, and a search can be wrong; the mitigations are
//! below, and the outcome is always honest about which of the three cases it hit.
//!
//! The one rule that matters: **resolution never drops a thread**. A `Detached`
//! result still carries its recorded quote and context, so a consumer sees the code
//! a comment was written against even when that code is gone.

use super::record::{Anchor, Side, CONTEXT_LINES};
use crate::model::{Changeset, DiffFile};

/// How far from the recorded line a moved line is looked for, in lines.
///
/// Bounds the work on a large file and, more importantly, bounds how wrong a match
/// can be: a line that "moved" two thousand lines is far more likely to be a
/// different line that happens to read the same.
pub const SEARCH_WINDOW: usize = 200;

/// Context lines a *disambiguating* candidate must match.
///
/// This only applies when the quote occurs more than once in the window. A quote
/// that occurs exactly once is unambiguous and is accepted on its own; when several
/// candidates compete, at least this many surrounding lines must agree, or the
/// anchor detaches rather than guess. Raise it if wrong re-anchors show up in
/// practice; lower it if too much detaches.
pub const MIN_CONTEXT_MATCH: usize = 1;

/// Where a recorded anchor landed in the current changeset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Found at exactly the line it was recorded at.
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
    /// Not found: the file is gone, the line is gone, or the match was too
    /// ambiguous to claim. The thread is still delivered.
    Detached,
    /// Cannot be decided yet: the file is present but its diff has not been
    /// computed, so it carries no content to search.
    ///
    /// Distinct from [`Detached`](Self::Detached) on purpose — during a streaming
    /// load every anchor would otherwise resolve to "the code is gone", which is a
    /// far more alarming and entirely wrong thing to tell a consumer.
    Unresolved,
}

impl Resolution {
    /// The current line, when the anchor resolved to one.
    #[must_use]
    pub fn line(self) -> Option<u32> {
        match self {
            Resolution::Attached { line } => Some(line),
            Resolution::Shifted { to, .. } => Some(to),
            Resolution::Detached | Resolution::Unresolved => None,
        }
    }

    /// Whether the anchored code could not be found.
    ///
    /// False for [`Unresolved`](Self::Unresolved): not knowing yet is not the same
    /// as knowing it is gone.
    #[must_use]
    pub fn is_detached(self) -> bool {
        matches!(self, Resolution::Detached)
    }
}

/// The full text of one side of a file, when the loader carries it.
fn side_text(file: &DiffFile, side: Side) -> Option<&str> {
    match side {
        Side::Old => file.old_text.as_deref(),
        Side::New => file.new_text.as_deref(),
    }
}

/// Capture an anchor for a 1-based line on one side of a file.
///
/// Returns `None` when that side has no text, or the line is out of range.
#[must_use]
pub fn capture(file: &DiffFile, side: Side, line: u32) -> Option<Anchor> {
    let text = side_text(file, side)?;
    let lines: Vec<&str> = text.lines().collect();
    let idx = usize::try_from(line).ok()?.checked_sub(1)?;
    let quote = (*lines.get(idx)?).to_string();

    let before_start = idx.saturating_sub(CONTEXT_LINES);
    let before = lines.get(before_start..idx).unwrap_or_default();
    let after_end = idx.saturating_add(1);
    let after = lines
        .get(after_end..lines.len().min(after_end + CONTEXT_LINES))
        .unwrap_or_default();

    Some(Anchor {
        path: file.path.clone(),
        side,
        line,
        quote,
        before: before.iter().map(|s| (*s).to_string()).collect(),
        after: after.iter().map(|s| (*s).to_string()).collect(),
    })
}

/// Resolve a recorded anchor against the current changeset.
///
/// Four steps, in order: the file must still be there; an exact hit at the recorded
/// line attaches; otherwise the window is searched and scored; otherwise it
/// detaches.
///
/// A file still being diffed yields [`Resolution::Unresolved`] rather than
/// `Detached`, so a streaming load never claims the commented code is gone.
#[must_use]
pub fn resolve(anchor: &Anchor, cs: &Changeset) -> Resolution {
    match find_file(anchor, cs) {
        None => Resolution::Detached,
        Some(f) if !f.diffed => Resolution::Unresolved,
        Some(f) => match side_text(f, anchor.side) {
            None => Resolution::Detached,
            Some(text) => resolve_in(anchor, &text.lines().collect::<Vec<_>>()),
        },
    }
}

/// The changeset entry an anchor names.
///
/// Matches the rename source too: an agent that moves a file while addressing a
/// comment would otherwise detach every thread in it, reporting the code as gone
/// when it moved wholesale and every quote would still match. `apply_path_filter`
/// matches on `previous_path` for the same reason.
#[must_use]
pub fn find_file<'c>(anchor: &Anchor, cs: &'c Changeset) -> Option<&'c DiffFile> {
    cs.files
        .iter()
        .find(|f| f.path == anchor.path || f.previous_path.as_deref() == Some(anchor.path.as_str()))
}

/// The text of the side an anchor points at, split into lines.
///
/// Exposed so a caller resolving many anchors against one file can split it once
/// and hand the same slice to every [`resolve_in`], rather than re-splitting the
/// whole file per anchor.
#[must_use]
pub fn side_lines(file: &DiffFile, side: Side) -> Option<Vec<&str>> {
    side_text(file, side).map(|t| t.lines().collect())
}

/// Resolve an anchor against one file's already-split lines.
#[must_use]
pub fn resolve_in(anchor: &Anchor, lines: &[&str]) -> Resolution {
    let Some(recorded) = usize::try_from(anchor.line)
        .ok()
        .and_then(|l| l.checked_sub(1))
    else {
        return Resolution::Detached;
    };

    if lines.get(recorded).is_some_and(|l| *l == anchor.quote) {
        return Resolution::Attached { line: anchor.line };
    }

    match best_candidate(lines, recorded, anchor) {
        Some(idx) => {
            let to = u32::try_from(idx.saturating_add(1)).unwrap_or(u32::MAX);
            Resolution::Shifted {
                from: anchor.line,
                to,
            }
        }
        None => Resolution::Detached,
    }
}

/// Indices within the search window whose text equals the recorded quote.
fn candidates(lines: &[&str], recorded: usize, quote: &str) -> Vec<usize> {
    let lo = recorded.saturating_sub(SEARCH_WINDOW);
    let hi = recorded.saturating_add(SEARCH_WINDOW).min(lines.len());
    (lo..hi)
        .filter(|i| lines.get(*i).is_some_and(|l| *l == quote))
        .collect()
}

/// How many of the anchor's recorded context lines also match around `idx`.
///
/// `before` is nearest-last and `after` nearest-first, so both are walked outward
/// from the candidate.
fn context_score(lines: &[&str], idx: usize, anchor: &Anchor) -> usize {
    let before = anchor
        .before
        .iter()
        .rev()
        .enumerate()
        .filter(|(back, want)| {
            idx.checked_sub(back.saturating_add(1))
                .and_then(|i| lines.get(i))
                .is_some_and(|got| got == want)
        })
        .count();
    let after = anchor
        .after
        .iter()
        .enumerate()
        .filter(|(fwd, want)| {
            lines
                .get(idx.saturating_add(fwd.saturating_add(1)))
                .is_some_and(|got| got == want)
        })
        .count();
    before + after
}

/// Pick the best candidate, or `None` when there is nothing acceptable.
///
/// A single candidate is unambiguous and is taken as-is. Several competing
/// candidates are scored on context: the best wins, ties break by proximity to the
/// recorded line, and if the winner cannot muster [`MIN_CONTEXT_MATCH`] matching
/// context lines the anchor detaches rather than guess — unless the anchor recorded
/// no context at all, in which case proximity is the only evidence there is.
fn best_candidate(lines: &[&str], recorded: usize, anchor: &Anchor) -> Option<usize> {
    let found = candidates(lines, recorded, &anchor.quote);
    match found.as_slice() {
        [] => return None,
        [only] => return Some(*only),
        _ => {}
    }

    let distance = |i: &usize| i.abs_diff(recorded);
    let scored = found
        .iter()
        .map(|i| (context_score(lines, *i, anchor), *i))
        .max_by(|(sa, ia), (sb, ib)| sa.cmp(sb).then_with(|| distance(ib).cmp(&distance(ia))));

    let (score, idx) = scored?;
    let has_context = !anchor.before.is_empty() || !anchor.after.is_empty();
    if has_context && score < MIN_CONTEXT_MATCH {
        return None;
    }
    Some(idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffFile, FileStatus};

    fn file(path: &str, text: &str) -> DiffFile {
        let mut f = DiffFile::stub(path.into(), None, FileStatus::Modified, false, None);
        f.new_text = Some(text.to_string());
        f.diffed = true;
        f
    }

    fn cs(files: Vec<DiffFile>) -> Changeset {
        Changeset {
            source: "worktree".into(),
            files,
        }
    }

    const SRC: &str = "fn main() {\n    let a = 1;\n    let b = 2;\n    print(a + b);\n}\n";

    #[test]
    fn capture_records_the_line_and_bounded_context() {
        let f = file("a.rs", SRC);
        let a = capture(&f, Side::New, 3).unwrap();
        assert_eq!(a.path, "a.rs");
        assert_eq!(a.line, 3);
        assert_eq!(a.quote, "    let b = 2;");
        assert_eq!(a.before, vec!["fn main() {", "    let a = 1;"]);
        assert_eq!(a.after, vec!["    print(a + b);", "}"]);
        assert!(a.before.len() <= CONTEXT_LINES && a.after.len() <= CONTEXT_LINES);
    }

    #[test]
    fn capture_clamps_context_at_the_file_edges() {
        let f = file("a.rs", SRC);
        let first = capture(&f, Side::New, 1).unwrap();
        assert!(first.before.is_empty());
        let last = capture(&f, Side::New, 5).unwrap();
        assert!(last.after.is_empty());
    }

    #[test]
    fn capture_rejects_a_line_out_of_range_or_a_missing_side() {
        let f = file("a.rs", SRC);
        assert!(capture(&f, Side::New, 99).is_none());
        assert!(capture(&f, Side::New, 0).is_none(), "lines are 1-based");
        assert!(capture(&f, Side::Old, 1).is_none(), "no old text carried");
    }

    #[test]
    fn unchanged_line_attaches() {
        let f = file("a.rs", SRC);
        let a = capture(&f, Side::New, 3).unwrap();
        let r = resolve(&a, &cs(vec![f]));
        assert_eq!(r, Resolution::Attached { line: 3 });
        assert_eq!(r.line(), Some(3));
        assert!(!r.is_detached());
    }

    #[test]
    fn inserted_lines_above_shift_the_anchor() {
        let f = file("a.rs", SRC);
        let a = capture(&f, Side::New, 3).unwrap();

        let grown = file("a.rs", &format!("// header\n// more\n{SRC}"));
        let r = resolve(&a, &cs(vec![grown]));
        assert_eq!(r, Resolution::Shifted { from: 3, to: 5 });
        assert_eq!(r.line(), Some(5));
        assert!(!r.is_detached());
    }

    #[test]
    fn a_deleted_line_detaches() {
        let f = file("a.rs", SRC);
        let a = capture(&f, Side::New, 3).unwrap();
        let shrunk = file("a.rs", "fn main() {\n    let a = 1;\n    print(a);\n}\n");
        let r = resolve(&a, &cs(vec![shrunk]));
        assert_eq!(r, Resolution::Detached);
        assert_eq!(r.line(), None);
        assert!(r.is_detached());
        assert_eq!(a.quote, "    let b = 2;", "the thread keeps its evidence");
    }

    #[test]
    fn a_renamed_file_keeps_its_anchors() {
        // Regression: lookup was exact-path only, so `git mv` while addressing a
        // comment detached every thread in the file — reporting the code as gone
        // when it had merely moved.
        let f = file("src/git/diff.rs", SRC);
        let a = capture(&f, Side::New, 3).unwrap();

        let mut renamed = file("src/git/difftext.rs", SRC);
        renamed.previous_path = Some("src/git/diff.rs".into());
        renamed.status = FileStatus::Renamed;

        assert_eq!(
            resolve(&a, &cs(vec![renamed])),
            Resolution::Attached { line: 3 },
            "the anchor follows the rename"
        );
    }

    #[test]
    fn a_file_still_being_diffed_is_unresolved_not_detached() {
        // During a streaming load the file is listed but carries no content. Saying
        // "detached" there would tell a consumer the commented code is gone, which
        // is both wrong and alarming; "not yet" is the honest answer.
        let f = file("a.rs", SRC);
        let a = capture(&f, Side::New, 3).unwrap();

        let mut pending = file("a.rs", SRC);
        pending.diffed = false;
        pending.new_text = None;

        let r = resolve(&a, &cs(vec![pending]));
        assert_eq!(r, Resolution::Unresolved);
        assert!(!r.is_detached(), "not knowing is not knowing it is gone");
        assert_eq!(r.line(), None);
    }

    #[test]
    fn resolve_in_matches_resolve_on_presplit_lines() {
        // The cached delivery path splits each file once and calls `resolve_in`;
        // it must agree with the convenience wrapper exactly.
        let f = file("a.rs", SRC);
        let a = capture(&f, Side::New, 3).unwrap();
        let lines: Vec<&str> = SRC.lines().collect();
        assert_eq!(resolve_in(&a, &lines), resolve(&a, &cs(vec![f])));
    }

    #[test]
    fn a_missing_file_detaches() {
        let f = file("a.rs", SRC);
        let a = capture(&f, Side::New, 3).unwrap();
        assert_eq!(resolve(&a, &cs(vec![])), Resolution::Detached);
        assert_eq!(
            resolve(&a, &cs(vec![file("other.rs", SRC)])),
            Resolution::Detached
        );
    }

    #[test]
    fn a_missing_side_detaches() {
        let f = file("a.rs", SRC);
        let mut a = capture(&f, Side::New, 3).unwrap();
        a.side = Side::Old; // the changeset carries no old text
        assert_eq!(resolve(&a, &cs(vec![f])), Resolution::Detached);
    }

    #[test]
    fn context_disambiguates_identical_lines() {
        // `    x();` three times; the anchor's context names the middle one.
        let orig = "fn a() {\n    x();\n}\nfn b() {\n    x();\n}\nfn c() {\n    x();\n}\n";
        let f = file("a.rs", orig);
        let a = capture(&f, Side::New, 5).unwrap();
        assert_eq!(a.quote, "    x();");
        assert_eq!(a.before.last().unwrap(), "fn b() {");

        // Insert two lines at the top so every occurrence shifts by 2.
        let moved = file("a.rs", &format!("// one\n// two\n{orig}"));
        assert_eq!(
            resolve(&a, &cs(vec![moved])),
            Resolution::Shifted { from: 5, to: 7 },
            "context must pick fn b's x(), not fn a's or fn c's"
        );
    }

    #[test]
    fn equal_scores_break_toward_the_nearest_line() {
        // Identical lines with identical (empty) surroundings, so every candidate
        // scores the same; the one nearest the recorded position must win.
        let text = "q\n\n\n\nq\n\n\n\nq\n";
        let f = file("a.rs", text);
        let anchor = Anchor {
            path: "a.rs".into(),
            side: Side::New,
            line: 4, // between the first and second `q`
            quote: "q".into(),
            before: vec![],
            after: vec![],
        };
        // Candidates are lines 1, 5, 9; line 5 is nearest to 4.
        assert_eq!(
            resolve(&anchor, &cs(vec![f])),
            Resolution::Shifted { from: 4, to: 5 }
        );
    }

    #[test]
    fn an_ambiguous_candidate_below_the_threshold_detaches() {
        // Two identical lines, and the recorded context matches neither — better to
        // detach and say so than to attach to the wrong one.
        let f = file("a.rs", "aaa\n    dup\nbbb\nccc\n    dup\nddd\n");
        let anchor = Anchor {
            path: "a.rs".into(),
            side: Side::New,
            line: 3,
            quote: "    dup".into(),
            before: vec!["nothing like this".into()],
            after: vec!["nor this".into()],
        };
        assert_eq!(resolve(&anchor, &cs(vec![f])), Resolution::Detached);
    }

    #[test]
    fn a_lone_candidate_is_accepted_without_context() {
        // Unambiguous quote, context entirely rewritten around it: still a match.
        let f = file("a.rs", "totally\ndifferent\n    let b = 2;\nagain\n");
        let anchor = Anchor {
            path: "a.rs".into(),
            side: Side::New,
            line: 1,
            quote: "    let b = 2;".into(),
            before: vec!["gone".into()],
            after: vec!["also gone".into()],
        };
        assert_eq!(
            resolve(&anchor, &cs(vec![f])),
            Resolution::Shifted { from: 1, to: 3 }
        );
    }

    #[test]
    fn a_move_beyond_the_search_window_detaches() {
        let mut text = String::from("    needle\n");
        for _ in 0..(SEARCH_WINDOW + 50) {
            text.push_str("filler\n");
        }
        let f = file("a.rs", &text);
        let anchor = Anchor {
            path: "a.rs".into(),
            side: Side::New,
            // Recorded far below where the needle actually is now.
            line: u32::try_from(SEARCH_WINDOW + 40).unwrap(),
            quote: "    needle".into(),
            before: vec![],
            after: vec![],
        };
        assert_eq!(resolve(&anchor, &cs(vec![f])), Resolution::Detached);
    }

    #[test]
    fn context_score_counts_both_directions() {
        let lines = vec!["a", "b", "c", "d", "e"];
        let anchor = Anchor {
            path: "x".into(),
            side: Side::New,
            line: 3,
            quote: "c".into(),
            before: vec!["a".into(), "b".into()],
            after: vec!["d".into(), "e".into()],
        };
        assert_eq!(context_score(&lines, 2, &anchor), 4);
        assert_eq!(context_score(&lines, 0, &anchor), 0);
    }
}
