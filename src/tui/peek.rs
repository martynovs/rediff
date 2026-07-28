//! The single-file "peek" overlay: a modal, scrollable view of one file in
//! either content mode (the whole file) or diff mode (a unified diff with an
//! adjustable context level). Reuses the row plan + renderer over a synthetic
//! one-file changeset.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::diff::{compute_hunks_with_context, whole_file_hunks};
use crate::lang;
use crate::model::{BlameLine, Changeset, DiffFile, FileStatus, LayoutMode, LineKind, Stats};
use crate::tui::rows::{Plan, Row};
use crate::tui::view::ViewState;

/// Compact diff context (a tight hunk view).
const COMPACT_CONTEXT: u32 = 3;
/// "Full" diff context — large enough to show the whole file with changes inline.
const FULL_CONTEXT: u32 = 1_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PeekMode {
    Content,
    Diff,
    /// Whole-file content (at the committed rev) with a per-line blame gutter.
    Blame,
}

/// A single-file overlay. Holds the source texts and rebuilds its one-file
/// changeset/plan when the mode or context changes.
pub struct Peek {
    pub path: String,
    /// Blue accent when opened from a local/staged view, else the commit accent.
    pub origin_local: bool,
    pub mode: PeekMode,
    /// Diff context: full (whole file inline) when true, compact hunks when false.
    pub full: bool,
    /// Render the diff side-by-side (split) rather than unified.
    pub split_view: bool,
    /// The texts backing content and diff modes. `None` until fetched — a
    /// blame-opened peek defers them until the user first Tabs out of blame,
    /// so `b` doesn't pay git reads for panes that are never shown.
    sides: Option<Sides>,
    pub cs: Changeset,
    /// The single row plan, rebuilt for the active layout (unified vs split) when
    /// the mode/context/split toggles — the same shape the main stream now uses.
    pub plan: Plan,
    /// Row index where each contiguous run of changed lines begins, for `[`/`]`
    /// navigation — works even in full-context mode (one merged hunk). Recomputed
    /// per rebuild for the active layout.
    pub change_starts: Vec<usize>,
    /// Scroll/viewport state, so the shared `stream::` navigation operates on the
    /// peek exactly as it does the main view.
    pub state: ViewState,
    /// File content at the committed rev, shown in blame mode (so blame lines
    /// align). Filled by the background blame worker together with `blame`.
    blame_text: String,
    /// Per-line blame attribution, index-aligned to `blame_text`'s lines.
    /// `None` until a fetch settles; `Some` (possibly empty — an untracked
    /// file has no blame at the rev) once one has, so "settled empty" is
    /// distinguishable from "never started" and a failed blame isn't
    /// respawned on every mode entry.
    blame: Option<Vec<BlameLine>>,
    /// In-flight blame computation (committed-rev content + attribution);
    /// `Some` while the worker runs, `None` once the result is installed (or
    /// before blame was ever requested).
    pub blame_rx: Option<Receiver<(String, Vec<BlameLine>)>>,
    /// Cooperative cancel signal for the in-flight blame worker; set when this
    /// peek is dropped (closed or replaced), so an abandoned fetch stops at its
    /// next checkpoint instead of computing a result nobody will receive.
    pub blame_cancel: Option<Arc<AtomicBool>>,
}

impl Drop for Peek {
    fn drop(&mut self) {
        if let Some(cancel) = &self.blame_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// The content/diff-side texts (see `Peek::sides`). The content pane shows the
/// view's new-side text, which is always one of the two diff sides — so it is
/// borrowed from there, not stored a third time.
struct Sides {
    /// Old/new sides for diff mode.
    diff_old: String,
    diff_new: String,
    /// Which diff side the content pane shows: `diff_new` for a review/blame
    /// peek (old-vs-new), `diff_old` for a preview peek (new-vs-top).
    content_from_new: bool,
}

impl Sides {
    /// The content-mode text (the view's new side).
    fn content(&self) -> &str {
        if self.content_from_new {
            &self.diff_new
        } else {
            &self.diff_old
        }
    }
}

impl Peek {
    pub fn new(
        path: String,
        origin_local: bool,
        mode: PeekMode,
        diff_old: String,
        diff_new: String,
        content_from_new: bool,
    ) -> Self {
        Self::build(
            path,
            origin_local,
            mode,
            Some(Sides {
                diff_old,
                diff_new,
                content_from_new,
            }),
        )
    }

    /// A peek opened directly in blame mode (`b`): blame renders its own
    /// committed-rev text, so the content/diff sides start unfetched and are
    /// installed lazily via [`Peek::set_sides`] on the first Tab out of blame.
    pub fn new_blame(path: String, origin_local: bool) -> Self {
        Self::build(path, origin_local, PeekMode::Blame, None)
    }

    fn build(path: String, origin_local: bool, mode: PeekMode, sides: Option<Sides>) -> Self {
        let mut p = Peek {
            path,
            origin_local,
            mode,
            full: true, // diff mode opens with full context (expanded)
            split_view: false,
            sides,
            cs: Changeset {
                source: String::new(),
                files: Vec::new(),
            },
            plan: Plan {
                rows: Vec::new(),
                file_starts: Vec::new(),
                visible_files: Vec::new(),
                hunk_starts: Vec::new(),
                content_w: 0,
                layout: LayoutMode::Stack,
            },
            change_starts: Vec::new(),
            state: ViewState::default(),
            blame_text: String::new(),
            blame: None,
            blame_rx: None,
            blame_cancel: None,
        };
        p.rebuild();
        p
    }

    /// Whether blame should be kicked off: in blame mode, never settled, and
    /// no computation already in flight. Settled-but-empty (`Some(vec![])`)
    /// does not re-trigger, so a file with no blame at the rev doesn't respawn
    /// a doomed worker on every mode entry.
    pub fn needs_blame(&self) -> bool {
        self.mode == PeekMode::Blame && self.blame.is_none() && self.blame_rx.is_none()
    }

    /// Whether the content/diff sides are still unfetched (a blame-opened peek
    /// before the first Tab out of blame).
    pub fn sides_unfilled(&self) -> bool {
        self.sides.is_none()
    }

    /// Store the content/diff sides WITHOUT rebuilding — for a caller that
    /// triggers a single rebuild itself right after (the Tab out of blame, whose
    /// mode toggle rebuilds; filling then rebuilding here would build, and
    /// immediately discard, an empty-sided plan).
    pub fn install_sides(&mut self, diff_old: String, diff_new: String, content_from_new: bool) {
        self.sides = Some(Sides {
            diff_old,
            diff_new,
            content_from_new,
        });
    }

    /// Install the content/diff sides and rebuild the plan for the active mode.
    /// Production fills sides via [`Self::install_sides`] + the mode toggle's
    /// rebuild; this is the standalone install-and-rebuild used by tests.
    #[cfg(test)]
    pub fn set_sides(&mut self, diff_old: String, diff_new: String, content_from_new: bool) {
        self.install_sides(diff_old, diff_new, content_from_new);
        self.rebuild();
    }

    /// The settled per-line attribution (empty while unfetched, or for a file
    /// with no blame at the rev).
    pub fn blame(&self) -> &[BlameLine] {
        self.blame.as_deref().unwrap_or_default()
    }

    /// Install a completed blame fetch: the committed-rev content (so the body
    /// and gutter align to that revision) plus the per-line attribution. Marks
    /// the fetch settled and rebuilds the plan.
    pub fn install_blame(&mut self, text: String, lines: Vec<BlameLine>) {
        self.blame_text = text;
        self.blame = Some(lines);
        self.blame_rx = None;
        self.blame_cancel = None;
        self.rebuild();
    }

    /// True while a blame computation is in flight (for the loading state).
    pub fn blame_loading(&self) -> bool {
        self.blame_rx.is_some()
    }

    /// The blame attribution for the content line at the top of the viewport,
    /// for the header. Finds the first `Line` row at or after the scroll position
    /// and maps its new line number to a blame index.
    pub fn blame_cursor(&self) -> Option<&BlameLine> {
        self.plan
            .rows
            .iter()
            .skip(self.state.scroll)
            .find_map(|r| match r {
                Row::Line { new: Some(n), .. } => Some(*n),
                _ => None,
            })
            .and_then(|n| self.blame().get((n as usize).saturating_sub(1)))
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            PeekMode::Content => PeekMode::Diff,
            PeekMode::Diff => PeekMode::Blame,
            PeekMode::Blame => PeekMode::Content,
        };
        self.state.scroll = 0;
        self.rebuild();
    }

    /// Flip the diff context between full and compact. Inert in content mode —
    /// `=`/`-` only adjust a diff (use `Tab` to switch to diff first) — and a
    /// no-op when the level is unchanged, so a repeat keypress doesn't rebuild.
    pub fn set_full(&mut self, full: bool) {
        if self.mode != PeekMode::Diff || full == self.full {
            return;
        }
        self.full = full;
        self.rebuild();
    }

    /// Switch between unified and side-by-side; rebuilds the plan for the new
    /// layout (split only takes effect in diff mode). A no-op when unchanged, so
    /// opening a peek in the main view's current layout doesn't rebuild twice.
    pub fn set_split(&mut self, split: bool) {
        if split == self.split_view {
            return;
        }
        self.split_view = split;
        self.rebuild();
    }

    /// True when there is no renderable content — including a diff with no
    /// actual changes (only a header/spacer), so it shows a message not a blank.
    /// The plan may be unified (`Line` rows) or split (`Pair` rows).
    pub fn is_empty(&self) -> bool {
        !self
            .plan
            .rows
            .iter()
            .any(|r| matches!(r, Row::Line { .. } | Row::Pair(..)))
    }

    /// Row count of the (single, active-layout) plan being rendered.
    pub fn active_rows(&self) -> usize {
        self.plan.rows.len()
    }

    pub fn is_split(&self) -> bool {
        self.mode == PeekMode::Diff && self.split_view
    }

    /// Header label: path · mode. In diff mode it shows the file's real change
    /// stat (+added −removed) rather than the misleading whole-file hunk span.
    pub fn label(&self) -> String {
        match self.mode {
            PeekMode::Content => format!("{} · preview", self.path),
            PeekMode::Blame => {
                // The cursor line's full identity rides in the header, since the
                // gutter omits the sha and blanks continuation lines.
                match self.blame_cursor() {
                    Some(b) => {
                        let sha = &b.commit.sha;
                        let short = sha.get(..7).unwrap_or(sha);
                        format!("{} · blame · {short} {}", self.path, b.commit.summary)
                    }
                    None if self.blame_loading() => format!("{} · blame · …", self.path),
                    None => format!("{} · blame", self.path),
                }
            }
            PeekMode::Diff => {
                let (a, d) = self
                    .cs
                    .files
                    .first()
                    .map_or((0, 0), |f| (f.stats.additions, f.stats.deletions));
                format!(
                    "{}  +{a} -{d} · {} · {}",
                    self.path,
                    if self.full { "full" } else { "compact" },
                    if self.split_view { "split" } else { "unified" },
                )
            }
        }
    }

    fn rebuild(&mut self) {
        let context = if self.full {
            FULL_CONTEXT
        } else {
            COMPACT_CONTEXT
        };
        let file = match self.mode {
            PeekMode::Content => {
                content_file(&self.path, self.sides.as_ref().map_or("", Sides::content))
            }
            PeekMode::Diff => {
                let (old, new) = self
                    .sides
                    .as_ref()
                    .map_or(("", ""), |s| (s.diff_old.as_str(), s.diff_new.as_str()));
                diff_file(&self.path, old, new, context)
            }
            // Blame shows the file at the committed rev; the gutter is drawn from
            // `self.blame` at render time.
            PeekMode::Blame => content_file(&self.path, &self.blame_text),
        };
        self.cs = Changeset {
            source: self.path.clone(),
            files: vec![file],
        };
        let layout = if self.is_split() {
            LayoutMode::Split
        } else {
            LayoutMode::Stack
        };
        self.plan = Plan::build(
            &self.cs,
            &[false],
            layout,
            &std::collections::BTreeSet::new(),
        );
        self.change_starts = change_starts(&self.plan.rows);
        let max = self.plan.rows.len().saturating_sub(1);
        self.state.scroll = self.state.scroll.min(max);
    }
}

/// Row indices where a contiguous run of changed lines begins, for either layout:
/// a unified `Line` that is added/removed, or a split `Pair` with a non-context
/// side.
fn change_starts(rows: &[Row]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut prev_changed = false;
    for (i, row) in rows.iter().enumerate() {
        let changed = match row {
            Row::Line {
                kind: LineKind::Added | LineKind::Removed,
                ..
            } => true,
            Row::Pair(l, r) => {
                l.as_ref().is_some_and(|c| c.kind != LineKind::Context)
                    || r.as_ref().is_some_and(|c| c.kind != LineKind::Context)
            }
            _ => false,
        };
        if changed && !prev_changed {
            starts.push(i);
        }
        prev_changed = changed;
    }
    starts
}

fn content_file(path: &str, content: &str) -> DiffFile {
    DiffFile {
        path: path.to_string(),
        previous_path: None,
        status: FileStatus::Modified,
        staged: false,
        hunks: whole_file_hunks(content),
        stats: Stats::default(),
        language: lang::detect(path),
        is_binary: false,
        old_text: None,
        new_text: Some(content.to_string()),
        content_digest: None,
        diffed: true,
    }
}

fn diff_file(path: &str, old: &str, new: &str, context: u32) -> DiffFile {
    let (hunks, additions, deletions) = compute_hunks_with_context(old, new, context);
    DiffFile {
        path: path.to_string(),
        previous_path: None,
        status: FileStatus::Modified,
        staged: false,
        hunks,
        stats: Stats {
            additions,
            deletions,
        },
        language: lang::detect(path),
        is_binary: false,
        old_text: Some(old.to_string()),
        new_text: Some(new.to_string()),
        content_digest: None,
        diffed: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peek() -> Peek {
        Peek::new(
            "a.rs".into(),
            true,
            PeekMode::Content,
            "fn a() {}\n".into(),
            "fn b() {}\n".into(),
            true,
        )
    }

    #[test]
    fn toggle_mode_cycles_content_diff_blame_resetting_scroll() {
        let mut p = peek();
        assert_eq!(p.mode, PeekMode::Content);
        p.state.scroll = 7;

        p.toggle_mode();
        assert_eq!(p.mode, PeekMode::Diff, "content → diff");
        assert_eq!(p.state.scroll, 0, "scroll reset on the mode switch");

        p.state.scroll = 4;
        p.toggle_mode();
        assert_eq!(p.mode, PeekMode::Blame, "diff → blame");
        assert_eq!(p.state.scroll, 0, "scroll reset again");

        p.state.scroll = 2;
        p.toggle_mode();
        assert_eq!(p.mode, PeekMode::Content, "blame → content wraps");
        assert_eq!(p.state.scroll, 0, "scroll reset again");
    }

    #[test]
    fn dropping_a_peek_signals_its_blame_worker() {
        let mut p = Peek::new_blame("a.rs".into(), true);
        let cancel = Arc::new(AtomicBool::new(false));
        p.blame_cancel = Some(Arc::clone(&cancel));
        drop(p);
        assert!(
            cancel.load(Ordering::Relaxed),
            "closing/replacing the peek cancels its in-flight fetch"
        );
    }

    #[test]
    fn blame_opened_peek_fills_sides_lazily() {
        let mut p = Peek::new_blame("a.rs".into(), true);
        assert_eq!(p.mode, PeekMode::Blame);
        assert!(p.sides_unfilled(), "`b` defers the content/diff sides");
        p.toggle_mode(); // → content, still unfilled → renders empty
        assert!(p.is_empty(), "unfilled content is empty, not stale");
        p.set_sides("fn a() {}\n".into(), "fn b() {}\n".into(), true);
        assert!(!p.sides_unfilled());
        assert!(!p.is_empty(), "sides installed and the plan rebuilt");
        p.toggle_mode(); // → diff, sides present
        assert!(!p.is_empty(), "diff mode sees the installed sides");
    }

    #[test]
    fn blame_label_covers_unloaded_loading_and_loaded() {
        let mut p = Peek::new(
            "a.rs".into(),
            true,
            PeekMode::Blame,
            String::new(),
            String::new(),
            true,
        );
        // Blame mode, nothing loaded and not in flight → the bare blame label.
        let bare = p.label();
        assert!(bare.contains("blame") && !bare.contains('…'), "{bare}");

        // A pending computation → the loading ellipsis.
        let (_tx, rx) = std::sync::mpsc::channel();
        p.blame_rx = Some(rx);
        assert!(p.label().contains('…'), "loading shows an ellipsis");
        p.blame_rx = None;

        // Loaded with content + attribution → the cursor line's commit identity.
        p.install_blame(
            "fn x() {}\n".into(),
            vec![BlameLine {
                commit: std::sync::Arc::new(crate::model::BlameCommit {
                    sha: "abcdef0123".into(),
                    author: "me".into(),
                    summary: "do the thing".into(),
                    time_secs: 1,
                    color_key: 0,
                }),
            }],
        );
        let loaded = p.label();
        assert!(loaded.contains("abcdef0"), "short sha in header: {loaded}");
        assert!(
            loaded.contains("do the thing"),
            "summary in header: {loaded}"
        );
    }

    #[test]
    fn set_full_and_set_split_are_inert_when_unchanged() {
        let mut p = peek(); // content mode, opens full
                            // Content mode: set_full is inert (the mode guard).
        p.set_full(false);
        assert!(p.full, "set_full ignored outside diff mode");
        p.toggle_mode(); // → diff
        assert_eq!(p.mode, PeekMode::Diff);
        // Same value → the unchanged-guard early return.
        p.set_full(true);
        assert!(p.full, "no-op when already full");
        // Changed value applies.
        p.set_full(false);
        assert!(!p.full, "compact context applied");
        // set_split: same value is a no-op, a change applies.
        let split = p.split_view;
        p.set_split(split);
        assert_eq!(p.split_view, split, "no-op when unchanged");
        p.set_split(!split);
        assert_ne!(p.split_view, split, "a change applies");
    }
}
