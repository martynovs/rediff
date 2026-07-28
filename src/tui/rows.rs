//! The single row-planning layer: flatten a changeset into a flat list of rows.
//! Windowing, scrolling, and navigation all derive from this one structure.
//!
//! One parametric [`Plan`] serves both layouts: the chrome rows (file/hunk
//! headers, collapsed/pending placeholders, spacers) are identical, and only the
//! body differs — a stacked [`Row::Line`] in unified layout, or a side-by-side
//! [`Row::Pair`] in split layout. `Plan::build` branches only in the hunk-body
//! emission; the row count and order genuinely differ between layouts, so a plan
//! is built per layout (the active one).

use std::collections::BTreeSet;

use crate::model::{parent_dir, Changeset, LayoutMode, LineKind};
use crate::review::Side;

/// One renderable row in the review stream. The body variant present depends on
/// the plan's `layout`: `Line` in stacked layout, `Pair` in split layout.
pub enum Row {
    /// A file header; carries the index into `Changeset::files`.
    FileHeader(usize),
    /// A collapsed (reviewed) file body placeholder; carries hidden-hunk count.
    Collapsed(usize),
    /// A folded directory's body placeholder: stands in for all its (hidden)
    /// files. `reviewed` is how many of the `n` files are reviewed (so the body
    /// can show `Y/N` and a `✓` once the whole directory is done).
    CollapsedDir {
        dir: String,
        n: usize,
        reviewed: usize,
    },
    /// An undiffed file's body placeholder, shown while its diff streams in.
    Pending,
    /// A commit-message banner line, prepended above the diff for a commit view.
    /// Part of the scrollable plan, so it scrolls away as the user reads down.
    Banner(String),
    /// A hunk boundary, rendered as a dim `⋯` gap in the interactive view (the
    /// `@@ … @@` ranges live only in the lazygit-compatible renderers). Carries the
    /// previous hunk's last new-side line number — the smaller-digit surrounding
    /// number — used only to left-align the `⋯` under the gutter numbers.
    HunkHeader(u32),
    /// A stacked (unified) diff content line. `file` is the index into
    /// `Changeset::files` (for highlight-cache lookup).
    Line {
        file: usize,
        kind: LineKind,
        old: Option<u32>,
        new: Option<u32>,
        text: String,
        emphasis: Option<(u32, u32)>,
    },
    /// A split (side-by-side) row: deletions on the left paired with insertions
    /// on the right (either side may be blank).
    Pair(Option<SplitCell>, Option<SplitCell>),
    /// Blank separator between files.
    Spacer,
}

/// One side of a split (side-by-side) row, or blank.
pub struct SplitCell {
    pub file: usize,
    pub side_new: bool,
    pub lineno: Option<u32>,
    pub kind: LineKind,
    pub text: String,
    pub emphasis: Option<(u32, u32)>,
}

/// The flattened plan plus the indices needed for navigation. `layout` records
/// which layout these rows were built for.
pub struct Plan {
    pub rows: Vec<Row>,
    /// Row index where each *visible* file's header sits, parallel to
    /// `visible_files` (NOT to `Changeset::files`): `file_starts[k]` is the row of
    /// the k-th visible file. With nothing folded this is the dense per-file index.
    pub file_starts: Vec<usize>,
    /// Original `Changeset::files` index of each visible file, in stream order.
    /// Parallel to `file_starts`. Identity (`0..n`) when nothing is folded.
    pub visible_files: Vec<usize>,
    /// Row indices of every hunk header, in stream order.
    pub hunk_starts: Vec<usize>,
    /// Widest rendered row (columns), for clamping horizontal scroll. In split
    /// layout this is the widest single cell (one column).
    pub content_w: usize,
    /// The layout these rows were built for.
    pub layout: LayoutMode,
}

/// Columns the line-number gutter + sign prefix occupy before the body text.
const GUTTER_W: usize = 6;

impl Plan {
    /// Build the plan for `layout` from a changeset. Files marked viewed are
    /// collapsed to a single placeholder row (their hunks are hidden). Files whose
    /// parent directory is in `collapsed` are folded out entirely — no header, no
    /// hunks — replaced by one [`Row::CollapsedDir`] placeholder per directory. The
    /// chrome is identical across layouts; only the hunk body differs (a unified
    /// line vs a paired split row).
    pub fn build(
        cs: &Changeset,
        viewed: &[bool],
        layout: LayoutMode,
        collapsed: &BTreeSet<String>,
    ) -> Plan {
        Self::build_with_banner(cs, viewed, layout, collapsed, &[])
    }

    /// Like [`Plan::build`], but prepends `banner` (one [`Row::Banner`] per line,
    /// then a spacer) ahead of the first file — the commit-message banner. Because
    /// the navigation indices are computed from `rows.len()` as rows are pushed,
    /// the banner offset flows into `file_starts`/`hunk_starts` automatically and
    /// all navigation stays correct.
    pub fn build_with_banner(
        cs: &Changeset,
        viewed: &[bool],
        layout: LayoutMode,
        collapsed: &BTreeSet<String>,
        banner: &[String],
    ) -> Plan {
        let split = matches!(layout, LayoutMode::Split);
        let mut rows = Vec::new();
        let mut file_starts = Vec::new();
        let mut visible_files = Vec::new();
        let mut hunk_starts = Vec::new();
        prepend_banner(&mut rows, banner);
        let mut content_w = 0;

        let mut prev_dir: Option<&str> = None;
        for (fi, f) in cs.files.iter().enumerate() {
            let dir = parent_dir(&f.path);
            let first_of_dir = prev_dir != Some(dir);
            prev_dir = Some(dir);

            // A folded directory: emit one placeholder on its first file (files of
            // a directory are contiguous, since they are sorted by parent), then
            // skip every file in it.
            if collapsed.contains(dir) {
                if first_of_dir {
                    #[expect(
                        clippy::indexing_slicing,
                        reason = "fi is an enumerate index into cs.files"
                    )]
                    let n = cs.files[fi..]
                        .iter()
                        .take_while(|g| parent_dir(&g.path) == dir)
                        .count();
                    let reviewed = (fi..fi + n)
                        .filter(|&k| viewed.get(k).copied().unwrap_or(false))
                        .count();
                    rows.push(Row::CollapsedDir {
                        dir: dir.to_string(),
                        n,
                        reviewed,
                    });
                    rows.push(Row::Spacer);
                }
                continue;
            }

            file_starts.push(rows.len());
            visible_files.push(fi);
            rows.push(Row::FileHeader(fi));

            if viewed.get(fi).copied().unwrap_or(false) {
                rows.push(Row::Collapsed(f.hunks.len()));
                rows.push(Row::Spacer);
                continue;
            }

            // Not yet diffed: a single placeholder row stands in for the body
            // until the background diff lands.
            if !f.diffed {
                rows.push(Row::Pending);
                rows.push(Row::Spacer);
                continue;
            }

            if f.is_binary {
                // Split layout has no sensible two-column body for a binary file,
                // so it shows nothing; the stacked layout shows a note line.
                if split {
                    rows.push(Row::Spacer);
                    continue;
                }
                rows.push(Row::Line {
                    file: fi,
                    kind: LineKind::Context,
                    old: None,
                    new: None,
                    text: "Binary file — no preview".to_string(),
                    emphasis: None,
                });
            }

            // The gap marker before each hunk is aligned to the previous hunk's
            // last new-side line (the smaller-digit surrounding number, which
            // precedes — so is ≤ — this hunk's numbers). Tracked across iterations
            // so the first hunk (no predecessor) simply gets no marker.
            let mut prev_hunk_end: Option<u32> = None;
            for h in &f.hunks {
                hunk_starts.push(rows.len());
                if let Some(above) = prev_hunk_end {
                    rows.push(Row::HunkHeader(above));
                }
                if split {
                    for l in &h.lines {
                        content_w = content_w.max(GUTTER_W + l.text.chars().count());
                    }
                    emit_split_body(&mut rows, fi, &h.lines);
                } else {
                    for l in &h.lines {
                        content_w = content_w.max(GUTTER_W + l.text.chars().count());
                        rows.push(Row::Line {
                            file: fi,
                            kind: l.kind,
                            old: l.old_lineno,
                            new: l.new_lineno,
                            text: l.text.clone(),
                            emphasis: l.emphasis,
                        });
                    }
                }
                prev_hunk_end = Some(h.new_start + h.new_len.saturating_sub(1));
            }
            rows.push(Row::Spacer);
        }

        Plan {
            rows,
            file_starts,
            visible_files,
            hunk_starts,
            content_w,
            layout,
        }
    }

    /// The visible ordinal (index into `file_starts`/`visible_files`) of the file
    /// at `Changeset::files` index `fi`, or `None` when it is folded away.
    pub fn visible_ordinal(&self, fi: usize) -> Option<usize> {
        self.visible_files.iter().position(|&i| i == fi)
    }

    /// Row index of the folded-directory placeholder for `dir`, if present.
    pub fn collapsed_row(&self, dir: &str) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| matches!(r, Row::CollapsedDir { dir: d, .. } if d == dir))
    }
}

/// Prepend the commit-message banner (one [`Row::Banner`] per line, then a
/// spacer) to `rows`. Banner rows never pan with `h_scroll` (the renderer draws
/// them fixed), so they deliberately do not contribute to `content_w` — a long
/// message line must not widen the horizontal scroll range of the diff body.
/// Empty `banner` pushes nothing.
fn prepend_banner(rows: &mut Vec<Row>, banner: &[String]) {
    for line in banner {
        rows.push(Row::Banner(line.clone()));
    }
    if !banner.is_empty() {
        rows.push(Row::Spacer);
    }
}

/// Emit a hunk's lines as split `Pair` rows: removals are paired with insertions
/// row-for-row, context lines align on both sides.
fn emit_split_body(rows: &mut Vec<Row>, fi: usize, lines: &[crate::model::Line]) {
    let mut rem: Vec<&crate::model::Line> = Vec::new();
    let mut add: Vec<&crate::model::Line> = Vec::new();
    let flush = |rows: &mut Vec<Row>,
                 rem: &mut Vec<&crate::model::Line>,
                 add: &mut Vec<&crate::model::Line>| {
        let n = rem.len().max(add.len());
        for i in 0..n {
            let left = rem.get(i).map(|l| SplitCell {
                file: fi,
                side_new: false,
                lineno: l.old_lineno,
                kind: LineKind::Removed,
                text: l.text.clone(),
                emphasis: l.emphasis,
            });
            let right = add.get(i).map(|l| SplitCell {
                file: fi,
                side_new: true,
                lineno: l.new_lineno,
                kind: LineKind::Added,
                text: l.text.clone(),
                emphasis: l.emphasis,
            });
            rows.push(Row::Pair(left, right));
        }
        rem.clear();
        add.clear();
    };

    for l in lines {
        match l.kind {
            LineKind::Context => {
                flush(rows, &mut rem, &mut add);
                let left = SplitCell {
                    file: fi,
                    side_new: false,
                    lineno: l.old_lineno,
                    kind: LineKind::Context,
                    text: l.text.clone(),
                    emphasis: None,
                };
                let right = SplitCell {
                    file: fi,
                    side_new: true,
                    lineno: l.new_lineno,
                    kind: LineKind::Context,
                    text: l.text.clone(),
                    emphasis: None,
                };
                rows.push(Row::Pair(Some(left), Some(right)));
            }
            LineKind::Removed => rem.push(l),
            LineKind::Added => add.push(l),
        }
    }
    flush(rows, &mut rem, &mut add);
}

/// A row's content identity: the file, which side of the diff, and the line
/// number on that side. `(file, side, line)` rather than `(file, line)` because a
/// removed and an added line can share a file and a number.
pub type Key = (usize, Side, u32);

/// One side of a split row as a key, using the cell's own recorded side rather
/// than its position, so the two can never disagree.
fn cell_key(cell: Option<&SplitCell>) -> Option<Key> {
    let c = cell?;
    let side = if c.side_new { Side::New } else { Side::Old };
    Some((c.file, side, c.lineno?))
}

/// Every identity a row carries, old side first.
///
/// A row can carry **two**: a unified context line exists on both sides, and a
/// split pair holds one cell per side. That is what lets a layout toggle match
/// one row against two, in both directions — it is machinery for re-anchoring,
/// not a choice offered to the user.
///
/// A binary-file note (`old: None, new: None`) and every chrome row carry none,
/// by construction rather than by special case.
pub fn row_keys(row: &Row) -> (Option<Key>, Option<Key>) {
    match row {
        Row::Line { file, old, new, .. } => (
            old.map(|n| (*file, Side::Old, n)),
            new.map(|n| (*file, Side::New, n)),
        ),
        Row::Pair(l, r) => (cell_key(l.as_ref()), cell_key(r.as_ref())),
        _ => (None, None),
    }
}

/// The single key to re-anchor `row` by: the new side when the row carries one,
/// the old side otherwise. Nothing is remembered between rebuilds — a stored
/// preference is row-scoped memory that one field cannot hold, and it goes stale
/// as soon as the cursor leaves the row that set it.
pub fn cursor_key(plan: &Plan, row: usize) -> Option<Key> {
    let (old, new) = row_keys(plan.rows.get(row)?);
    new.or(old)
}

/// The row carrying `key`, if any. A key occurs at most once in a plan: hunks
/// advance monotonically through a file, so a given `(file, side, line)` is
/// emitted once.
pub fn find_key(plan: &Plan, key: Key) -> Option<usize> {
    plan.rows.iter().position(|r| {
        let (old, new) = row_keys(r);
        old == Some(key) || new == Some(key)
    })
}

/// What the line cursor was pointing at, captured before a plan is rebuilt.
///
/// Capture **classifies**; restore is per class. A single fallback ladder cannot
/// work, because only the *old* plan knows that a row was a folded directory's
/// placeholder and which directory it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorAnchor {
    /// A body row, identified by its content.
    Line(Key),
    /// A folded directory's placeholder (or the spacer that follows it).
    Dir(String),
    /// Any other row belonging to a file: its header, a hunk gap, a `Pending`
    /// placeholder, a spacer — held as an offset from the file's own start so
    /// another file changing size cannot move it.
    InFile { file: usize, offset: usize },
    /// A row above the first file: the commit-message banner region, whose rows
    /// never move.
    Above(usize),
}

/// The row a folded-directory placeholder belongs to, if `row` is one — or the
/// spacer immediately after one.
pub(crate) fn dir_at(plan: &Plan, row: usize) -> Option<&str> {
    let named = |r: usize| match plan.rows.get(r) {
        Some(Row::CollapsedDir { dir, .. }) => Some(dir.as_str()),
        _ => None,
    };
    named(row).or_else(|| {
        matches!(plan.rows.get(row), Some(Row::Spacer))
            .then(|| named(row.checked_sub(1)?))
            .flatten()
    })
}

/// Classify the cursor's row against the plan it currently sits in.
pub fn capture_cursor(plan: &Plan, row: usize) -> CursorAnchor {
    // Folded directories first: `file_starts` gets an entry only for a real file,
    // so `file_at` on a placeholder silently reports whichever file *precedes*
    // it. Treating that as the cursor's file fabricates an anchor, and the
    // fabrication survives into the fallbacks.
    if let Some(dir) = dir_at(plan, row) {
        return CursorAnchor::Dir(dir.to_string());
    }
    if let Some(key) = cursor_key(plan, row) {
        return CursorAnchor::Line(key);
    }
    match plan.file_starts.first() {
        Some(&first) if row >= first => {
            let ord = file_at(&plan.file_starts, row);
            let start = plan.file_starts.get(ord).copied().unwrap_or(0);
            CursorAnchor::InFile {
                file: plan.visible_files.get(ord).copied().unwrap_or(0),
                offset: row - start,
            }
        }
        _ => CursorAnchor::Above(row),
    }
}

/// Row where a file's header sits in this plan, or `None` when it is folded away.
fn file_start_of(plan: &Plan, file: usize) -> Option<usize> {
    plan.visible_ordinal(file)
        .and_then(|o| plan.file_starts.get(o).copied())
}

/// Where a file's rows went: its header, or the placeholder of the directory
/// that swallowed it.
fn file_or_placeholder(plan: &Plan, cs: &Changeset, file: usize) -> Option<usize> {
    file_start_of(plan, file).or_else(|| {
        let path = &cs.files.get(file)?.path;
        plan.collapsed_row(parent_dir(path))
    })
}

/// Last row belonging to the file whose header is at `start`.
///
/// A file's rows always end with a `Spacer` and never contain one — every
/// `Plan::build_with_banner` branch closes a file with exactly one. So the first
/// spacer at or after the header bounds the file, which is what stops an offset
/// captured near the bottom of a file from spilling into the *next* file when
/// this one shrinks (marking it reviewed collapses its body to a placeholder).
fn file_end(plan: &Plan, start: usize) -> usize {
    plan.rows
        .iter()
        .skip(start)
        .position(|r| matches!(r, Row::Spacer))
        .map_or_else(|| plan.rows.len().saturating_sub(1), |off| start + off)
}

/// Place the cursor in a freshly built plan.
pub fn restore_cursor(plan: &Plan, anchor: &CursorAnchor, cs: &Changeset) -> usize {
    let last = plan.rows.len().saturating_sub(1);
    let row = match anchor {
        CursorAnchor::Line(key) => find_key(plan, *key)
            .or_else(|| file_or_placeholder(plan, cs, key.0))
            .unwrap_or(last),
        CursorAnchor::Dir(dir) => plan
            .collapsed_row(dir)
            .or_else(|| {
                // Unfolded since capture: land on the directory's first file.
                let i = cs.files.iter().position(|f| parent_dir(&f.path) == dir)?;
                file_start_of(plan, i)
            })
            .unwrap_or(last),
        CursorAnchor::InFile { file, offset } => file_or_placeholder(plan, cs, *file)
            .map_or(last, |start| (start + offset).min(file_end(plan, start))),
        CursorAnchor::Above(row) => *row,
    };
    row.min(last)
}

/// Index of the file whose region contains `row` (the last start <= row).
pub fn file_at(starts: &[usize], row: usize) -> usize {
    match starts.binary_search(&row) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) => i - 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffFile, FileStatus, Hunk, Line, Stats};

    fn nofold() -> BTreeSet<String> {
        BTreeSet::new()
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

    /// One file with a hunk: 1 context, 2 removed, 1 added.
    fn fixture() -> Changeset {
        let hunk = Hunk {
            old_start: 1,
            old_len: 3,
            new_start: 1,
            new_len: 2,
            lines: vec![
                line(LineKind::Context, Some(1), Some(1), "ctx"),
                line(LineKind::Removed, Some(2), None, "a"),
                line(LineKind::Removed, Some(3), None, "b"),
                line(LineKind::Added, None, Some(2), "c"),
            ],
        };
        let f = DiffFile {
            path: "f.rs".into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks: vec![hunk],
            stats: Stats {
                additions: 1,
                deletions: 2,
            },
            language: None,
            is_binary: false,
            old_text: None,
            new_text: None,
            content_digest: None,
            diffed: true,
        };
        Changeset {
            source: "t".into(),
            files: vec![f],
        }
    }

    fn kinds(rows: &[Row]) -> Vec<&'static str> {
        rows.iter()
            .map(|r| match r {
                Row::FileHeader(_) => "fh",
                Row::Collapsed(_) => "col",
                Row::CollapsedDir { .. } => "cdir",
                Row::Pending => "pend",
                Row::Banner(_) => "ban",
                Row::HunkHeader(_) => "hh",
                Row::Line { .. } => "line",
                Row::Pair(..) => "pair",
                Row::Spacer => "sp",
            })
            .collect()
    }

    #[test]
    fn stack_sequence_interleaves_lines() {
        let cs = fixture();
        let p = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        // header, 4 body lines (ctx, rem, rem, add), spacer. The first (only) hunk
        // has no `⋯` gap marker — it follows the file header directly.
        assert_eq!(kinds(&p.rows), ["fh", "line", "line", "line", "line", "sp"]);
        assert_eq!(p.file_starts, vec![0]);
        assert_eq!(p.hunk_starts, vec![1]);
    }

    #[test]
    fn split_sequence_pairs_removed_with_added() {
        let cs = fixture();
        let p = Plan::build(&cs, &[false], LayoutMode::Split, &nofold());
        // header, then: ctx pair, then 2 removed paired with 1 added (max(2,1) = 2
        // pair rows), spacer. No `⋯` before the first hunk.
        assert_eq!(kinds(&p.rows), ["fh", "pair", "pair", "pair", "sp"]);
        assert_eq!(p.hunk_starts, vec![1]);
    }

    #[test]
    fn change_starts_are_consistent_per_layout() {
        let cs = fixture();
        // The first changed row in stack is the first Removed line (row index 2,
        // after fh + ctx-line; no leading hunk marker). In split it is the first
        // non-context pair.
        let stack = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        let first_change = stack.rows.iter().position(|r| {
            matches!(
                r,
                Row::Line {
                    kind: LineKind::Removed,
                    ..
                }
            )
        });
        assert_eq!(first_change, Some(2));
        let split = Plan::build(&cs, &[false], LayoutMode::Split, &nofold());
        let first_pair_change = split
            .rows
            .iter()
            .position(|r| matches!(r, Row::Pair(Some(c), _) if c.kind != LineKind::Context));
        assert_eq!(first_pair_change, Some(2));
    }

    #[test]
    fn first_hunk_has_no_gap_marker_but_later_hunks_do() {
        // Two hunks: the first (new lines 10–12) has no `⋯`; the second (starting
        // at new line 120) does. The digit count changes across the boundary, so
        // the marker aligns to the smaller — the first hunk's last line, 12.
        let hunk = |new_start: u32, new_len: u32| Hunk {
            old_start: new_start,
            old_len: new_len,
            new_start,
            new_len,
            lines: (0..new_len)
                .map(|i| line(LineKind::Added, None, Some(new_start + i), "x"))
                .collect(),
        };
        let f = DiffFile {
            path: "f.rs".into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks: vec![hunk(10, 3), hunk(120, 1)],
            stats: Stats::default(),
            language: None,
            is_binary: false,
            old_text: None,
            new_text: None,
            content_digest: None,
            diffed: true,
        };
        let cs = Changeset {
            source: "t".into(),
            files: vec![f],
        };
        let p = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        // fh, 3 lines (hunk 0, no marker), hh (⋯ before hunk 1), line, sp.
        assert_eq!(
            kinds(&p.rows),
            ["fh", "line", "line", "line", "hh", "line", "sp"]
        );
        // One gap marker, carrying the first hunk's last new-side line (12) — the
        // smaller-digit surrounding number — not the second hunk's start (120).
        let markers: Vec<u32> = p
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::HunkHeader(n) => Some(*n),
                _ => None,
            })
            .collect();
        assert_eq!(markers, vec![12]);
    }

    /// Two files in `src/`, one at root. Folding `src` drops both `src` files'
    /// rows, leaving one placeholder; the root file still renders in full.
    fn two_dir_fixture() -> Changeset {
        let mk = |path: &str| DiffFile {
            path: path.into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks: vec![Hunk {
                old_start: 1,
                old_len: 1,
                new_start: 1,
                new_len: 1,
                lines: vec![line(LineKind::Added, None, Some(1), "x")],
            }],
            stats: Stats {
                additions: 1,
                deletions: 0,
            },
            language: None,
            is_binary: false,
            old_text: None,
            new_text: None,
            content_digest: None,
            diffed: true,
        };
        // Sorted by (parent_dir, name): root file first, then the two src files.
        Changeset {
            source: "t".into(),
            files: vec![mk("a.rs"), mk("src/b.rs"), mk("src/c.rs")],
        }
    }

    #[test]
    fn folded_directory_yields_one_placeholder_and_no_file_rows() {
        let cs = two_dir_fixture();
        let mut collapsed = BTreeSet::new();
        collapsed.insert("src".to_string());
        let p = Plan::build(&cs, &[false, false, false], LayoutMode::Stack, &collapsed);

        // The two src files leave no FileHeader rows; exactly one CollapsedDir.
        let headers: Vec<usize> = p
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::FileHeader(i) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(
            headers,
            vec![0],
            "only the root file has a header; src is folded"
        );
        let cdirs = p
            .rows
            .iter()
            .filter(|r| matches!(r, Row::CollapsedDir { .. }))
            .count();
        assert_eq!(cdirs, 1, "one placeholder for the folded directory");
        // file_starts / visible_files re-index over the visible file only.
        assert_eq!(p.visible_files, vec![0], "only the root file is visible");
        assert_eq!(p.visible_ordinal(0), Some(0));
        assert_eq!(
            p.visible_ordinal(1),
            None,
            "folded file has no visible ordinal"
        );
        assert!(
            p.collapsed_row("src").is_some(),
            "the placeholder row is locatable"
        );
    }

    #[test]
    fn banner_rows_precede_files_and_offset_indices() {
        let cs = fixture();
        let banner = vec![
            "abc123 · me · 2026-06-30".to_string(),
            String::new(),
            "the message".to_string(),
        ];
        let p = Plan::build_with_banner(&cs, &[false], LayoutMode::Stack, &nofold(), &banner);
        // Three banner rows then a spacer precede the file header.
        assert_eq!(&kinds(&p.rows)[..4], ["ban", "ban", "ban", "sp"]);
        assert_eq!(
            p.file_starts,
            vec![4],
            "the file header is offset past the banner+spacer"
        );
        assert_eq!(
            p.hunk_starts,
            vec![5],
            "hunk index carries the banner offset"
        );
        // No banner → the plan is unchanged (file at row 0).
        let p0 = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        assert_eq!(p0.file_starts, vec![0]);
    }

    #[test]
    fn viewed_file_collapses_to_one_placeholder() {
        let cs = fixture();
        let p = Plan::build(&cs, &[true], LayoutMode::Stack, &nofold());
        // A reviewed file's body is hidden behind a single Collapsed placeholder.
        assert_eq!(kinds(&p.rows), ["fh", "col", "sp"]);
    }

    #[test]
    fn binary_file_body_differs_by_layout() {
        let mut cs = fixture();
        cs.files[0].is_binary = true;
        cs.files[0].hunks.clear();
        // Stacked layout shows a one-line "no preview" note.
        let stack = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        assert_eq!(kinds(&stack.rows), ["fh", "line", "sp"]);
        // Split layout has no two-column body for a binary, so it shows nothing.
        let split = Plan::build(&cs, &[false], LayoutMode::Split, &nofold());
        assert_eq!(kinds(&split.rows), ["fh", "sp"]);
    }

    /// Every key carried by any row of a plan, in row order.
    fn all_keys(p: &Plan) -> Vec<Key> {
        p.rows
            .iter()
            .flat_map(|r| {
                let (o, n) = row_keys(r);
                [o, n]
            })
            .flatten()
            .collect()
    }

    #[test]
    fn a_unified_context_row_carries_both_sides() {
        // The whole point: one row, two identities, so a layout toggle can match
        // it from either side. Keying off the *displayed* number (always the new
        // side) is what broke split→unified for context rows in an earlier draft.
        let row = Row::Line {
            file: 3,
            kind: LineKind::Context,
            old: Some(5),
            new: Some(7),
            text: "ctx".into(),
            emphasis: None,
        };
        assert_eq!(
            row_keys(&row),
            (Some((3, Side::Old, 5)), Some((3, Side::New, 7)))
        );
    }

    #[test]
    fn one_sided_unified_rows_carry_one_key_and_binary_notes_carry_none() {
        let mk = |old, new| Row::Line {
            file: 0,
            kind: LineKind::Context,
            old,
            new,
            text: String::new(),
            emphasis: None,
        };
        assert_eq!(
            row_keys(&mk(Some(2), None)),
            (Some((0, Side::Old, 2)), None)
        );
        assert_eq!(
            row_keys(&mk(None, Some(2))),
            (None, Some((0, Side::New, 2)))
        );
        // The binary-file note is a body row with no line number on either side,
        // so it falls to the index path by construction, not by special case.
        assert_eq!(row_keys(&mk(None, None)), (None, None));
    }

    #[test]
    fn split_cells_key_by_their_own_recorded_side() {
        let cell = |side_new, lineno| SplitCell {
            file: 1,
            side_new,
            lineno,
            kind: LineKind::Context,
            text: String::new(),
            emphasis: None,
        };
        let both = Row::Pair(Some(cell(false, Some(5))), Some(cell(true, Some(7))));
        assert_eq!(
            row_keys(&both),
            (Some((1, Side::Old, 5)), Some((1, Side::New, 7)))
        );
        // A surplus row from an unbalanced hunk has one populated cell.
        assert_eq!(
            row_keys(&Row::Pair(Some(cell(false, Some(9))), None)),
            (Some((1, Side::Old, 9)), None)
        );
        assert_eq!(
            row_keys(&Row::Pair(None, Some(cell(true, Some(9))))),
            (None, Some((1, Side::New, 9)))
        );
        // A cell with no line number carries nothing.
        assert_eq!(
            row_keys(&Row::Pair(Some(cell(false, None)), None)),
            (None, None)
        );
    }

    #[test]
    fn every_chrome_row_carries_no_key() {
        let chrome = [
            Row::FileHeader(0),
            Row::Collapsed(3),
            Row::CollapsedDir {
                dir: "src".into(),
                n: 2,
                reviewed: 1,
            },
            Row::Pending,
            Row::Banner("msg".into()),
            Row::HunkHeader(12),
            Row::Spacer,
        ];
        for row in &chrome {
            assert_eq!(row_keys(row), (None, None), "chrome carries no identity");
        }
    }

    #[test]
    fn a_context_row_is_found_from_either_of_its_keys() {
        let cs = fixture();
        let p = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        // fixture's context line is old 1 / new 1 — the same number on both
        // sides, so the sides must be distinguished by more than the number.
        let by_old = find_key(&p, (0, Side::Old, 1));
        let by_new = find_key(&p, (0, Side::New, 1));
        assert_eq!(by_old, by_new, "both keys resolve to the one context row");
        assert!(by_old.is_some());
    }

    #[test]
    fn a_removed_and_an_added_line_at_the_same_number_are_distinct_rows() {
        // fixture has removed old-2 and added new-2. Keyed on (file, line) alone
        // these would collide; the side is what separates them.
        let cs = fixture();
        let p = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        let rem = find_key(&p, (0, Side::Old, 2)).expect("removed line 2");
        let add = find_key(&p, (0, Side::New, 2)).expect("added line 2");
        assert_ne!(rem, add, "same number, different sides, different rows");
    }

    #[test]
    fn keys_are_unique_within_a_plan_in_both_layouts() {
        // The restore path assumes `find_key` is unambiguous.
        // `fixture()`, not `two_dir_fixture()`: the latter is all `Added` lines,
        // so no split row ever carries an old-side cell and collapsing both
        // sides onto `New` would go unnoticed. `fixture()` has a context line at
        // old 1 / new 1, which duplicates the moment the side stops mattering.
        let cs = fixture();
        for layout in [LayoutMode::Stack, LayoutMode::Split] {
            let p = Plan::build(&cs, &[false], layout, &nofold());
            let keys = all_keys(&p);
            // A set, not a sort: `Side` is a serialized on-disk type and derives
            // `Hash` but deliberately not `Ord`. A test is not a reason to widen
            // the wire format's derives.
            let uniq: std::collections::HashSet<Key> = keys.iter().copied().collect();
            assert_eq!(keys.len(), uniq.len(), "duplicate key in {layout:?} plan");
            assert!(!keys.is_empty(), "the fixture does carry keys");
        }
    }

    #[test]
    fn cursor_key_prefers_the_new_side_and_falls_back_to_old() {
        let cs = fixture();
        let p = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        // Row 1 is the context line (both sides) -> new wins.
        assert_eq!(cursor_key(&p, 1), Some((0, Side::New, 1)));
        // Row 2 is a removed line (old only) -> old is all there is.
        assert_eq!(cursor_key(&p, 2), Some((0, Side::Old, 2)));
        // Row 0 is the file header, and past the end is nothing.
        assert_eq!(cursor_key(&p, 0), None);
        assert_eq!(cursor_key(&p, 999), None);
    }

    /// Rebuild `cs` under `layout`/`collapsed` and carry the cursor across.
    fn rebuild(
        cs: &Changeset,
        from: &Plan,
        row: usize,
        viewed: &[bool],
        layout: LayoutMode,
        collapsed: &BTreeSet<String>,
    ) -> (Plan, usize) {
        let anchor = capture_cursor(from, row);
        let plan = Plan::build(cs, viewed, layout, collapsed);
        let row = restore_cursor(&plan, &anchor, cs);
        (plan, row)
    }

    #[test]
    fn a_layout_toggle_keeps_the_cursor_on_the_same_change_both_ways() {
        let cs = fixture();
        let stack = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        // The context row: it carries both sides, which is what lets one row
        // match against two.
        let ctx = find_key(&stack, (0, Side::New, 1)).unwrap();
        let (split, moved) = rebuild(&cs, &stack, ctx, &[false], LayoutMode::Split, &nofold());
        assert!(
            matches!(&split.rows[moved], Row::Pair(..)),
            "landed on a split row"
        );
        assert_eq!(
            row_keys(&split.rows[moved]).1,
            Some((0, Side::New, 1)),
            "the same context line"
        );
        // ...and back again.
        let (_, back) = rebuild(&cs, &split, moved, &[false], LayoutMode::Stack, &nofold());
        assert_eq!(back, ctx, "round-tripped to the same row");
    }

    #[test]
    fn a_split_context_rows_old_cell_survives_the_toggle_to_unified() {
        // The bug that killed an earlier draft: keying a row off the number it
        // *displays* (always the new side) left the left-hand cell of every
        // context row with nothing to match in a unified plan.
        let cs = fixture();
        let split = Plan::build(&cs, &[false], LayoutMode::Split, &nofold());
        let ctx = find_key(&split, (0, Side::Old, 1)).unwrap();
        let (stack, moved) = rebuild(&cs, &split, ctx, &[false], LayoutMode::Stack, &nofold());
        let (old, _) = row_keys(&stack.rows[moved]);
        assert_eq!(old, Some((0, Side::Old, 1)), "found from the old side");
    }

    #[test]
    fn a_placeholder_stays_with_its_own_file_when_another_file_grows() {
        // The ordinary streaming case: every file starts as three keyless rows,
        // so a cursor parked on one while an *earlier* file's diff lands must
        // ride with its own file. A bare index clamp leaves it inside whichever
        // file expanded underneath it.
        let mut cs = two_dir_fixture();
        // Give the first file a body worth streaming in: undiffed it is three
        // rows like every other file, diffed it is many.
        cs.files[0].hunks[0].lines = (1..=20)
            .map(|i| line(LineKind::Added, None, Some(i), "x"))
            .collect();
        for f in &mut cs.files {
            f.diffed = false;
        }
        let viewed = [false, false, false];
        let before = Plan::build(&cs, &viewed, LayoutMode::Stack, &nofold());
        // The last file's `Pending` row.
        let last_start = *before.file_starts.last().unwrap();
        let pending = last_start + 1;
        assert!(matches!(before.rows[pending], Row::Pending));

        // File 0's diff lands and its body grows from one placeholder to many.
        cs.files[0].diffed = true;
        let (after, moved) = rebuild(&cs, &before, pending, &viewed, LayoutMode::Stack, &nofold());
        assert!(after.rows.len() > before.rows.len(), "the plan did grow");
        assert!(
            matches!(after.rows[moved], Row::Pending),
            "still on a Pending placeholder, not adrift in the grown file"
        );
        assert_eq!(
            file_at(&after.file_starts, moved),
            2,
            "and it is still the third file's placeholder"
        );
    }

    #[test]
    fn folding_the_cursors_directory_lands_on_that_directorys_placeholder() {
        let cs = two_dir_fixture();
        let viewed = [false, false, false];
        let before = Plan::build(&cs, &viewed, LayoutMode::Stack, &nofold());
        // A body row inside src/b.rs.
        let row = find_key(&before, (1, Side::New, 1)).unwrap();

        let mut collapsed = BTreeSet::new();
        collapsed.insert("src".to_string());
        let (after, moved) = rebuild(&cs, &before, row, &viewed, LayoutMode::Stack, &collapsed);
        assert_eq!(
            after.collapsed_row("src"),
            Some(moved),
            "landed on the placeholder that replaced it, not an unrelated file"
        );
    }

    #[test]
    fn a_cursor_on_a_folded_placeholder_is_not_mistaken_for_the_file_before_it() {
        // `file_starts` has no entry for a folded directory, so `file_at` on the
        // placeholder reports whichever real file precedes it. Classifying it as
        // that file fabricates an anchor which then survives into the fallbacks.
        let mut cs = two_dir_fixture();
        cs.files[0].diffed = false; // a.rs (root) will grow later
        let viewed = [false, false, false];
        let mut collapsed = BTreeSet::new();
        collapsed.insert("src".to_string());
        let before = Plan::build(&cs, &viewed, LayoutMode::Stack, &collapsed);
        let ph = before.collapsed_row("src").unwrap();
        assert_eq!(
            capture_cursor(&before, ph),
            CursorAnchor::Dir("src".into()),
            "classified as a directory, not as the preceding file"
        );
        // The spacer right after it belongs to the directory too.
        assert_eq!(
            capture_cursor(&before, ph + 1),
            CursorAnchor::Dir("src".into())
        );

        cs.files[0].diffed = true; // the preceding file grows
        let (after, moved) = rebuild(&cs, &before, ph, &viewed, LayoutMode::Stack, &collapsed);
        assert_eq!(
            after.collapsed_row("src"),
            Some(moved),
            "still on src's placeholder"
        );
    }

    #[test]
    fn a_banner_row_keeps_its_index_and_a_body_row_is_classified_by_content() {
        let cs = fixture();
        let banner = ["abc123 · me".to_string(), "the message".to_string()];
        let p = Plan::build_with_banner(&cs, &[false], LayoutMode::Stack, &nofold(), &banner);
        assert_eq!(capture_cursor(&p, 1), CursorAnchor::Above(1));
        // The file header carries no key but does belong to a file.
        let header = p.file_starts[0];
        assert_eq!(
            capture_cursor(&p, header),
            CursorAnchor::InFile { file: 0, offset: 0 }
        );
        assert!(matches!(
            capture_cursor(&p, header + 1),
            CursorAnchor::Line(_)
        ));
    }

    #[test]
    fn capture_then_restore_against_the_same_plan_holds_every_row() {
        // The property that actually pins the restore logic down. Asserting only
        // that the result is in range is vacuous: `restore_cursor` ends with
        // `.min(last)`, so a body of `usize::MAX` would satisfy it.
        //
        // One row is deliberately not the identity: the spacer trailing a folded
        // directory's placeholder belongs to that directory, so it normalises
        // onto the placeholder — a blank line moving to the row that means
        // something. Asserted explicitly rather than excused.
        let cs = two_dir_fixture();
        let viewed = [false, true, false];
        let mut collapsed = BTreeSet::new();
        collapsed.insert("src".to_string());
        let banner = ["abc123 · me".to_string(), "the message".to_string()];
        for fold in [&nofold(), &collapsed] {
            for layout in [LayoutMode::Stack, LayoutMode::Split] {
                // With a banner too, so the `Above` arm is exercised — without
                // it no fixture ever produces that anchor, and neither it nor
                // the trailing range clamp is reachable from this property.
                for b in [&[][..], &banner[..]] {
                    let p = Plan::build_with_banner(&cs, &viewed, layout, fold, b);
                    for row in 0..p.rows.len() {
                        let back = restore_cursor(&p, &capture_cursor(&p, row), &cs);
                        let trailing_dir_spacer = matches!(p.rows.get(row), Some(Row::Spacer))
                            && row > 0
                            && matches!(p.rows.get(row - 1), Some(Row::CollapsedDir { .. }));
                        if trailing_dir_spacer {
                            assert_eq!(back, row - 1, "normalises onto its placeholder");
                        } else {
                            assert_eq!(back, row, "row {row} of a {layout:?} plan round-tripped");
                        }
                        // Either way it must be a fixed point: repeated rebuilds may
                        // not walk the cursor further each time.
                        let again = restore_cursor(&p, &capture_cursor(&p, back), &cs);
                        assert_eq!(again, back, "restore is stable");
                    }
                }
            }
        }
    }

    #[test]
    fn a_keyless_row_does_not_bleed_into_the_next_file_when_its_own_file_shrinks() {
        // Park the cursor at the bottom of a file and mark that file reviewed:
        // its body collapses to a placeholder, so an offset held from the old
        // header would land inside whatever file now occupies those rows.
        let mut cs = two_dir_fixture();
        cs.files.truncate(2);
        for f in &mut cs.files {
            f.hunks[0].lines = (1..=5)
                .map(|i| line(LineKind::Added, None, Some(i), "x"))
                .collect();
        }
        let before = Plan::build(&cs, &[false, false], LayoutMode::Stack, &nofold());
        let sp = before
            .rows
            .iter()
            .position(|r| matches!(r, Row::Spacer))
            .expect("the first file ends with a spacer");
        assert_eq!(
            capture_cursor(&before, sp),
            CursorAnchor::InFile {
                file: 0,
                offset: sp
            }
        );

        let after = Plan::build(&cs, &[true, false], LayoutMode::Stack, &nofold());
        let moved = restore_cursor(&after, &capture_cursor(&before, sp), &cs);
        assert!(
            matches!(after.rows[moved], Row::Spacer),
            "stayed on a spacer, not adrift in the next file"
        );
        assert_eq!(
            file_at(&after.file_starts, moved),
            0,
            "and still inside its own file's region"
        );
    }

    #[test]
    fn restore_is_always_in_range_even_for_an_empty_plan() {
        let cs = fixture();
        let p = Plan::build(&cs, &[false], LayoutMode::Stack, &nofold());
        let empty = Changeset {
            source: "t".into(),
            files: Vec::new(),
        };
        let blank = Plan::build(&empty, &[], LayoutMode::Stack, &nofold());
        for row in 0..p.rows.len() {
            let r = restore_cursor(&blank, &capture_cursor(&p, row), &empty);
            assert_eq!(r, 0, "a plan with no rows leaves nowhere else to be");
        }
    }

    #[test]
    fn file_at_locates_the_region_containing_a_row() {
        let starts = vec![0, 5, 12];
        assert_eq!(file_at(&starts, 0), 0, "exact match on a start");
        assert_eq!(file_at(&starts, 5), 1, "exact match on a later start");
        assert_eq!(file_at(&starts, 3), 0, "between starts → previous region");
        assert_eq!(file_at(&starts, 20), 2, "past the last start → last region");
        // Err(0): a row before the first start maps to region 0.
        let starts2 = vec![3, 8];
        assert_eq!(file_at(&starts2, 1), 0, "before the first start → 0");
    }

    #[test]
    fn collapsed_and_pending_chrome_match_across_layouts() {
        let mut cs = fixture();
        cs.files[0].diffed = false;
        for layout in [LayoutMode::Stack, LayoutMode::Split] {
            let p = Plan::build(&cs, &[false], layout, &nofold());
            assert_eq!(
                kinds(&p.rows),
                ["fh", "pend", "sp"],
                "undiffed chrome is layout-independent"
            );
        }
    }
}
