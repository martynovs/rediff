//! A single entry in the browser-style view stack. The app owns a `Vec<ViewEntry>`
//! plus a cursor; navigation (`{`/`}`/`C`, the commit picker) moves or pushes
//! entries. Each entry remembers its own scroll/selection and, when it is a
//! review session, its per-file reviewed state.

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use crate::git::{FileStub, LoadRequest};
use crate::model::Changeset;
use crate::tui::rows::Plan;

/// The live, per-view navigation state. Held both as the view entry's saved copy
/// and as `App`'s working copy; switching views swaps the whole struct, so no
/// field can leak across views and "add per-view state" is a one-field change.
#[derive(Debug, Clone, Default)]
pub struct ViewState {
    /// Viewport top (row index into the active plan).
    pub scroll: usize,
    /// The line cursor: the row of the active plan the user is pointing at.
    ///
    /// Named `cursor_row`, not `cursor`, because two other things in this module
    /// tree are already called that: `Session.cursor` is the active view in the
    /// stack, and `selected`/`selected_dir` are the sidebar's cursor.
    ///
    /// The single-file peek shares this struct but **does not read this field** —
    /// its renderer drops rows, so a cursor there needs a row→drawn-line mapping
    /// the stream does not. It stays 0 for a peek.
    pub cursor_row: usize,
    /// Horizontal scroll offset (columns) for long lines when not wrapping.
    pub h_scroll: usize,
    /// Wrap long lines instead of horizontal-scrolling (stack layout only).
    pub wrap: bool,
    /// Selected file (the sidebar cursor / stream anchor). Always a valid file
    /// index; when `selected_dir` is set it is a nearby fallback (the cursor is
    /// really on the placeholder), so file-actions read [`Self::selected_file`].
    pub selected: usize,
    /// When set, the cursor is on this collapsed directory's placeholder rather
    /// than on a file. File-actions (toggle reviewed, peek, jump digits) are inert
    /// in this state; the placeholder's one verb is unfold.
    pub selected_dir: Option<String>,
    /// Directories folded out of scope (per-line: the exact parent path, not
    /// ancestors). A file is hidden — gone from the sidebar list and the diff
    /// body — iff its parent directory is in this set. Per-view, so it round-trips
    /// through the view history with the rest of the state.
    pub collapsed: BTreeSet<String>,
    /// When set, the next draw scrolls the sidebar to reveal the selection.
    pub reveal_selected: bool,
    /// Per-file reviewed state, sized to `cs.files.len()`. All-false for a browse
    /// view; meaningful only when the view is a review session (`review`).
    pub viewed: Vec<bool>,
}

impl ViewState {
    /// The selected file, or `None` when the cursor is on a collapsed
    /// placeholder (the common "act on a file, otherwise inert" guard).
    pub fn selected_file(&self) -> Option<usize> {
        self.selected_dir.is_none().then_some(self.selected)
    }

    /// Move the cursor onto a file (clearing any placeholder selection).
    pub fn select_file(&mut self, i: usize) {
        self.selected = i;
        self.selected_dir = None;
    }

    /// Move the cursor onto a collapsed directory's placeholder. `near` is a
    /// fallback file index (used when the view leaves the grouped mode).
    pub fn select_dir(&mut self, dir: String, near: usize) {
        self.selected = near;
        self.selected_dir = Some(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_for_maps_every_request_kind() {
        // Working tree carries through its explicit `--from` base; everything
        // else seeds no extra base (the kind drives its own old side).
        let (kind, base) = ViewKind::launch_for(&LoadRequest::WorkingTree {
            include_untracked: true,
            base: Some("main".into()),
        });
        assert!(matches!(kind, ViewKind::Local));
        assert_eq!(base, Some("main".into()));

        let (kind, base) = ViewKind::launch_for(&LoadRequest::WorkingTree {
            include_untracked: false,
            base: None,
        });
        assert!(matches!(kind, ViewKind::Local) && base.is_none());

        let (kind, base) = ViewKind::launch_for(&LoadRequest::Staged);
        assert!(matches!(kind, ViewKind::Staged) && base.is_none());

        let (kind, base) = ViewKind::launch_for(&LoadRequest::Show { rev: "abc".into() });
        assert!(matches!(kind, ViewKind::Commit(r) if r == "abc") && base.is_none());

        let (kind, base) = ViewKind::launch_for(&LoadRequest::Range {
            old: "a".into(),
            new: "b".into(),
        });
        assert!(matches!(kind, ViewKind::Range { base, target } if base == "a" && target == "b"));
        assert!(base.is_none());

        let (kind, base) = ViewKind::launch_for(&LoadRequest::ReviewRange {
            base: "x".into(),
            target: "y".into(),
        });
        assert!(matches!(kind, ViewKind::Range { base, target } if base == "x" && target == "y"));
        assert!(base.is_none());
    }
}

/// What a view is showing, which also drives its source color and whether `C`
/// (return-home) applies.
#[derive(Debug, Clone)]
pub enum ViewKind {
    /// Working-tree changes.
    Local,
    /// Staged changes.
    Staged,
    /// A single commit's diff (commit vs its parent).
    Commit(String),
    /// A branch/range net diff.
    Range { base: String, target: String },
}

impl ViewKind {
    /// Local or staged changes (blue source accent).
    pub fn is_local(&self) -> bool {
        matches!(self, ViewKind::Local | ViewKind::Staged)
    }

    /// The launch view kind for a load request, paired with the explicit old-side
    /// base ref to seed (only a working-tree `diff --from` carries one; every
    /// other kind derives its old side from the kind itself).
    pub fn launch_for(req: &LoadRequest) -> (ViewKind, Option<String>) {
        match req {
            LoadRequest::WorkingTree { base, .. } => (ViewKind::Local, base.clone()),
            LoadRequest::Staged => (ViewKind::Staged, None),
            LoadRequest::Show { rev } => (ViewKind::Commit(rev.clone()), None),
            LoadRequest::Range { old, new } => (
                ViewKind::Range {
                    base: old.clone(),
                    target: new.clone(),
                },
                None,
            ),
            LoadRequest::ReviewRange { base, target } => (
                ViewKind::Range {
                    base: base.clone(),
                    target: target.clone(),
                },
                None,
            ),
        }
    }
}

/// One view in the stack.
pub struct ViewEntry {
    pub kind: ViewKind,
    pub cs: Rc<Changeset>,
    /// Explicit base ref for this view's old side (set for `diff --from <ref>`);
    /// `None` falls back to the kind's default (HEAD / commit parent / range
    /// merge-base). Used to source the peek's old side for an undiffed file.
    pub base: Option<String>,
    /// How to (re)enumerate this view's changeset from git. Read by the review
    /// integration, which encodes it as the review's target so a TUI-opened
    /// review is indistinguishable from an agent-opened one. Under snapshot
    /// semantics a view's file set is fixed for its lifetime, so nothing
    /// re-enumerates from it; an explicit reload action would.
    pub req: Option<LoadRequest>,
    /// The view's enumerated file stubs, index-aligned with `cs.files` and fixed
    /// for the view's lifetime. A resumed load re-diffs only the stubs whose
    /// `cs.files[i]` is still undiffed, at their original index — no
    /// re-enumeration. Empty for views that never streamed (already-diffed).
    pub stubs: Arc<Vec<FileStub>>,
    /// This view's live navigation state. The view owns it directly — `App` reads
    /// and writes through `views[cursor].state`, so a cursor move *is* the view
    /// switch (no save/load copy).
    pub state: ViewState,
    /// The view's row plan, derived from `cs` + `state.viewed` + the active
    /// layout. Cached on the entry so it stays warm across switches; rebuilt when
    /// `cs`/`viewed`/layout change.
    pub plan: Plan,
    /// Whether this view is a review session (viewed-tracking active). The
    /// reviewed flags themselves live in `state.viewed`.
    pub review: bool,
    /// Commit-message banner lines, prepended to the plan above the first file.
    /// Non-empty only for a single-commit view; empty for local/staged/range.
    pub banner: Vec<String>,
}
