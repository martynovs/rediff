//! The browsing session: the stack of visited views (`ViewEntry`), the active
//! cursor, and the background load machine (the worker pool + its progress
//! flags). This is the cohesive "browse-and-load" state — every field serves
//! managing the view stack and the diff load filling its current entry.
//!
//! The view entry owns the live `cs`/`state`/`plan` (a cursor move *is* the view
//! switch), so the session reads/writes through `self.views[self.cursor]` and the
//! single `cs` handle stays uniquely owned — the loader's `make_mut` installs in
//! place without cloning. Operations that also need app-global context (the
//! configured `layout`, the viewport size) take it as parameters; the
//! highlighter reset on a view switch stays with the `App` coordinator.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use crate::git::{FileStub, LoadRequest};
use crate::model::{Changeset, LayoutMode};
use crate::tui::app::LOAD_PROGRESS_DELAY;
use crate::tui::loader::Loader;
use crate::tui::rows::{self, Plan};
use crate::tui::stream;
use crate::tui::view::{ViewEntry, ViewKind, ViewState};

pub struct Session {
    /// Browser-style stack of visited views.
    pub views: Vec<ViewEntry>,
    /// The active view in the stack.
    pub cursor: usize,
    /// Repository directory, for loading commits/ranges at runtime.
    pub repo_dir: Option<PathBuf>,
    /// Background diff load for the current view, if one is streaming.
    pub loader: Option<Loader>,
    /// When the active load started, for the progress-chrome threshold.
    pub(crate) load_started: Option<Instant>,
    /// What kind of load is active — decides what cancelling it does.
    pub(crate) load_kind: LoadKind,
}

/// How the active diff load came to be, which decides what Esc/cancel does:
/// the launch load quits the app, a freshly-pushed commit view is abandoned
/// (popped), and a resumed load on an already-visited view is merely stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum LoadKind {
    /// The initial load at startup.
    #[default]
    Launch,
    /// A commit/range view just pushed onto the stack.
    Push,
    /// An existing view's leftover diffs, restarted after switching back to it.
    Resume,
}

/// The outcome of cancelling a load — what the app should do next.
pub(crate) enum LoadCancel {
    /// The launch load was cancelled: quit the app.
    Quit,
    /// A freshly-pushed view was abandoned: the cursor moved back, rebuild it.
    PoppedView,
    /// A resumed load was stopped: stay on the current view, don't restart it.
    Stopped,
}

impl Session {
    // ---- current-view accessors --------------------------------------------

    #[inline]
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub(crate) fn cur_mut(&mut self) -> &mut ViewEntry {
        &mut self.views[self.cursor]
    }
    /// The current view's changeset (the single live `Rc<Changeset>` handle).
    #[inline]
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub fn cs(&self) -> &Rc<Changeset> {
        &self.views[self.cursor].cs
    }
    /// The current view's row plan.
    #[inline]
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub fn plan(&self) -> &Plan {
        &self.views[self.cursor].plan
    }
    /// The current view's live navigation state.
    #[inline]
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub fn state(&self) -> &ViewState {
        &self.views[self.cursor].state
    }
    #[inline]
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub(crate) fn state_mut(&mut self) -> &mut ViewState {
        &mut self.views[self.cursor].state
    }

    // ---- view-stack queries ------------------------------------------------

    /// Whether the current view is a review session (viewed-tracking active).
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub fn is_review(&self) -> bool {
        self.views[self.cursor].review
    }

    /// The current view kind.
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub fn kind(&self) -> &ViewKind {
        &self.views[self.cursor].kind
    }

    /// True when the current view shows local/staged changes (blue source).
    pub fn source_is_local(&self) -> bool {
        self.kind().is_local()
    }

    /// Whether the home (launch) view supports `C` (it is local/staged/review).
    pub fn home_reviewable(&self) -> bool {
        self.views
            .first()
            .is_some_and(|v| v.review || v.kind.is_local())
    }

    pub(crate) fn total_rows(&self) -> usize {
        self.plan().rows.len()
    }

    /// Row of the file at `Changeset::files` index `idx`, if visible (mapped
    /// through `visible_files`, since `file_starts` is a visible-ordinal index).
    pub(crate) fn file_start(&self, idx: usize) -> Option<usize> {
        let p = self.plan();
        p.visible_ordinal(idx)
            .and_then(|o| p.file_starts.get(o).copied())
    }

    /// Index of the file currently at the top of the viewport.
    pub fn current_file(&self) -> usize {
        stream::current_file(self.state(), self.plan())
    }

    /// The file the viewed-actions apply to — always the active/selected file.
    pub(crate) fn active_file(&self) -> usize {
        self.state().selected
    }

    // ---- load queries ------------------------------------------------------

    /// Whether a background diff load is in progress.
    pub fn loading(&self) -> bool {
        self.loader.is_some()
    }

    /// Whether to show progress chrome: a load is active and has run past the
    /// threshold (so fast loads stay indicator-free).
    pub fn show_progress(&self) -> bool {
        self.loader.is_some()
            && self
                .load_started
                .is_some_and(|t| t.elapsed() >= LOAD_PROGRESS_DELAY)
    }

    /// Progress as `(done, total)` for the active load (zero/zero when idle).
    pub fn load_progress(&self) -> (usize, usize) {
        self.loader.as_ref().map_or((0, 0), |l| (l.done, l.total))
    }

    // ---- plan ---------------------------------------------------------------

    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub(crate) fn build_plan(&mut self, layout: LayoutMode, grouped: bool) {
        let e = &self.views[self.cursor];
        // Folds only apply in the grouped view; flat shows every file in both panes.
        let empty = BTreeSet::new();
        let collapsed = if grouped { &e.state.collapsed } else { &empty };
        // Classify the line cursor before the plan under it is replaced. This is
        // the only rebuild funnel — every `build_plan()` call site reaches it —
        // so restoring here needs no caller to remember anything. View
        // *construction* builds a plan outside this function, but on a fresh
        // `ViewState`, where there is nothing to restore.
        let anchor = rows::capture_cursor(&e.plan, e.state.cursor_row);
        let plan =
            Plan::build_with_banner(e.cs.as_ref(), &e.state.viewed, layout, collapsed, &e.banner);
        let cur = self.cur_mut();
        cur.plan = plan;
        // Identity only. Making the cursor *visible* again is `stream::clamp`'s
        // job: it has the viewport height, and this function does not.
        let row = rows::restore_cursor(&cur.plan, &anchor, cur.cs.as_ref());
        cur.state.cursor_row = row;
    }

    /// Rebuild the row plan from the current viewed/collapsed state, keeping the
    /// viewport anchored to the same position *within* the current file.
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub(crate) fn rebuild_plan(
        &mut self,
        layout: LayoutMode,
        grouped: bool,
        viewport_h: usize,
        viewport_w: usize,
    ) {
        let anchor = self.current_file();
        let old_start = self.file_start(anchor);
        let scroll = self.state().scroll;
        self.build_plan(layout, grouped);
        // Re-anchor only when the anchor file is present in both plans. When it
        // vanished (e.g. its directory folded away by a grouping toggle), keep
        // the scroll where it was — the clamp below bounds it — rather than
        // re-anchoring against a made-up start and yanking the viewport to the
        // top of the plan.
        if let (Some(old), Some(new)) = (old_start, self.file_start(anchor)) {
            self.state_mut().scroll = stream::reanchored(scroll, old, new);
        }
        let e = &mut self.views[self.cursor];
        // Same `usable` bound as `App::clamp`. This path never routes through
        // that one — `streaming_rebuild_keeps_the_banner_in_view` exercises it
        // directly — so a raw `viewport_h` here is the same off-by-one, one
        // layer down.
        let usable = stream::usable(&e.plan, viewport_h);
        stream::clamp(&mut e.state, &e.plan, usable, viewport_w);
    }

    // ---- stack mutation -----------------------------------------------------

    /// Push a new view, truncating any forward history (browser semantics). The
    /// commit-message banner is derived here from the view's own kind, so every
    /// pushed view carries it by construction — no caller-side patch-and-rebuild.
    /// A caller that already holds the message (the commit popup) passes its
    /// prebuilt `banner` to skip the duplicate repo read.
    /// The caller runs the view-switch side effects (`App::load_current`).
    #[expect(
        clippy::too_many_arguments,
        reason = "push_entry threads every view-construction input through the single seeding path, mirroring with_launch"
    )]
    pub(crate) fn push_entry(
        &mut self,
        kind: ViewKind,
        cs: Rc<Changeset>,
        base: Option<String>,
        req: Option<LoadRequest>,
        review: bool,
        layout: LayoutMode,
        banner: Option<Vec<String>>,
    ) {
        self.views.truncate(self.cursor + 1);
        let viewed = vec![false; cs.files.len()];
        let banner = banner.unwrap_or_else(|| commit_banner(self.repo_dir.as_deref(), &kind));
        let plan = Plan::build_with_banner(cs.as_ref(), &viewed, layout, &BTreeSet::new(), &banner);
        self.views.push(ViewEntry {
            kind,
            cs,
            base,
            req,
            stubs: Arc::new(Vec::new()),
            state: ViewState {
                viewed,
                ..Default::default()
            },
            plan,
            review,
            banner,
        });
        self.cursor = self.views.len() - 1;
    }

    /// Promote the current browse view into a review session.
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub fn promote_review(&mut self) {
        if !self.views[self.cursor].review {
            let n = self.cs().files.len();
            let e = &mut self.views[self.cursor];
            e.review = true;
            e.state.viewed = vec![false; n];
        }
    }

    /// Keep `viewed` sized to the (fixed) file set and request a sidebar reveal —
    /// the non-highlighter half of a view switch.
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub(crate) fn resize_and_reveal(&mut self) {
        let n = self.cs().files.len();
        let st = &mut self.views[self.cursor].state;
        if st.viewed.len() != n {
            st.viewed.resize(n, false);
        }
        st.reveal_selected = true;
    }

    // ---- load machine -------------------------------------------------------

    /// Begin the initial background diff load for the current view: record the
    /// enumerated `stubs` on the entry (fixing the view's file set for its
    /// lifetime) and diff every file.
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub fn begin_load(&mut self, stubs: Vec<FileStub>, is_switch: bool) {
        self.views[self.cursor].stubs = Arc::new(stubs);
        // `begin_load` is only called by the launch (is_switch=false) and by a
        // fresh commit-view push (is_switch=true); resumes go through
        // `resume_load_if_stale`.
        let kind = if is_switch {
            LoadKind::Push
        } else {
            LoadKind::Launch
        };
        self.start_load_undiffed(kind);
    }

    /// Start (or resume) the load for the current view by dispatching, at their
    /// original indices, exactly the stubs whose `cs.files[i]` is not yet diffed.
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    fn start_load_undiffed(&mut self, kind: LoadKind) {
        let Some(dir) = self.repo_dir.clone() else {
            return;
        };
        let stubs = self.views[self.cursor].stubs.clone();
        let jobs: Vec<(usize, FileStub)> = self
            .cs()
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.diffed)
            .filter_map(|(i, _)| stubs.get(i).map(|s| (i, s.clone())))
            .collect();
        if jobs.is_empty() {
            return;
        }
        self.load_kind = kind;
        self.load_started = Some(Instant::now());
        self.loader = Some(Loader::start(dir, jobs));
    }

    /// Resume a load abandoned by switching away — re-diff only the still-undiffed
    /// stubs at their original index (no re-enumeration; completed diffs kept).
    pub(crate) fn resume_load_if_stale(&mut self) {
        if self.loader.is_some() {
            return;
        }
        self.start_load_undiffed(LoadKind::Resume);
    }

    /// Drain ready diff results into the view-owned changeset in place, rebuilding
    /// the plan once per batch. Returns whether anything changed.
    #[expect(
        clippy::indexing_slicing,
        reason = "self.cursor is maintained as a valid index into self.views"
    )]
    pub(crate) fn drain_loader(
        &mut self,
        layout: LayoutMode,
        grouped: bool,
        viewport_h: usize,
        viewport_w: usize,
    ) -> bool {
        let Some(loader) = self.loader.as_mut() else {
            return false;
        };
        let batch = loader.drain();
        let finished = loader.finished();
        if batch.is_empty() {
            if finished {
                self.clear_load();
                return true;
            }
            return false;
        }
        // The view owns the single `cs` handle, so `make_mut` installs in place
        // with no clone (the only other handle is the transient `request_visible`
        // render clone, dropped within its own statement).
        let cs = Rc::make_mut(&mut self.views[self.cursor].cs);
        for (idx, file) in batch {
            if let Some(slot) = cs.files.get_mut(idx) {
                *slot = file;
            }
        }
        self.rebuild_plan(layout, grouped, viewport_h, viewport_w);
        if finished {
            self.clear_load();
        }
        true
    }

    /// Drop the loader (the completed diffs already live in the entry's `cs`).
    /// Used both when a load finishes and when one is abandoned on a view switch.
    pub(crate) fn clear_load(&mut self) {
        self.loader = None;
        self.load_started = None;
        self.load_kind = LoadKind::Launch;
    }

    /// Cancel an in-progress load by dropping the loader, and report what should
    /// happen next: a freshly-pushed commit view is popped (`PoppedView`), the
    /// launch load quits (`Quit`), and a resumed load on an already-visited view
    /// is merely stopped so revisiting-then-Esc can't destroy the view or the
    /// app.
    pub(crate) fn cancel_load(&mut self) -> LoadCancel {
        self.loader = None; // dropping cancels + joins the worker pool
        self.load_started = None;
        let kind = std::mem::take(&mut self.load_kind);
        match kind {
            LoadKind::Push if self.cursor > 0 => {
                self.views.truncate(self.cursor); // drop the abandoned pushed view
                self.cursor -= 1;
                LoadCancel::PoppedView
            }
            LoadKind::Launch => LoadCancel::Quit,
            // A resumed load (or the degenerate Push-at-root) just stops.
            LoadKind::Push | LoadKind::Resume => LoadCancel::Stopped,
        }
    }
}

/// The commit-message banner lines for a view: a header (short sha · author ·
/// date) then the full message body, for a single-commit view with a repo.
/// Empty for any other kind, no repo, or a fetch failure.
pub(crate) fn commit_banner(repo: Option<&std::path::Path>, kind: &ViewKind) -> Vec<String> {
    let (ViewKind::Commit(rev), Some(dir)) = (kind, repo) else {
        return Vec::new();
    };
    match crate::git::commit_message(dir, rev) {
        Ok(msg) => banner_lines(&msg),
        Err(_) => Vec::new(),
    }
}

/// Format a fetched commit message as banner lines: a `sha · author · date`
/// header, a blank separator, then the body split into lines.
pub(crate) fn banner_lines(msg: &crate::model::CommitMessage) -> Vec<String> {
    let mut out = vec![msg.identity()];
    if !msg.body.is_empty() {
        out.push(String::new());
        out.extend(msg.body.lines().map(str::to_string));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Changeset, DiffFile, FileStatus};

    fn cs(paths: &[&str]) -> Rc<Changeset> {
        let files = paths
            .iter()
            .map(|p| DiffFile::stub((*p).into(), None, FileStatus::Modified, false, None))
            .collect();
        Rc::new(Changeset {
            source: "test".into(),
            files,
        })
    }

    fn session_with(view: Rc<Changeset>) -> Session {
        let viewed = vec![false; view.files.len()];
        let plan = Plan::build(view.as_ref(), &viewed, LayoutMode::Stack, &BTreeSet::new());
        Session {
            views: vec![ViewEntry {
                kind: ViewKind::Local,
                cs: view,
                base: None,
                req: None,
                stubs: Arc::new(Vec::new()),
                state: ViewState {
                    viewed,
                    ..Default::default()
                },
                plan,
                review: false,
                banner: Vec::new(),
            }],
            cursor: 0,
            repo_dir: None,
            loader: None,
            load_started: None,
            load_kind: LoadKind::Launch,
        }
    }

    #[test]
    fn push_entry_advances_cursor_and_truncates_forward() {
        // A Session is exercised directly, without an App.
        let mut s = session_with(cs(&["a.rs", "b.rs"]));
        s.push_entry(
            ViewKind::Commit("x".into()),
            cs(&["c.rs"]),
            None,
            None,
            false,
            LayoutMode::Stack,
            None,
        );
        assert_eq!(s.cursor, 1);
        assert_eq!(s.views.len(), 2);
        assert_eq!(s.cs().files.len(), 1, "current view is the pushed one");

        // Step back, then push again — forward history is truncated.
        s.cursor = 0;
        s.push_entry(
            ViewKind::Commit("y".into()),
            cs(&["d.rs", "e.rs"]),
            None,
            None,
            false,
            LayoutMode::Stack,
            None,
        );
        assert_eq!(s.cursor, 1);
        assert_eq!(s.views.len(), 2, "the forward 'x' view was truncated");
    }

    #[test]
    fn cancel_load_outcome_depends_on_the_load_kind() {
        let mut s = session_with(cs(&["a.rs"]));
        s.push_entry(
            ViewKind::Commit("b".into()),
            cs(&["b.rs"]),
            None,
            None,
            false,
            LayoutMode::Stack,
            None,
        );
        assert_eq!(s.cursor, 1);
        // A resumed load on an already-visited view: cancel merely stops it,
        // keeping the view (and any forward history) intact — no truncation.
        s.load_kind = LoadKind::Resume;
        assert!(matches!(s.cancel_load(), LoadCancel::Stopped));
        assert_eq!(s.views.len(), 2, "resume-stop keeps the view");
        assert_eq!(s.cursor, 1, "and stays put");
        // A freshly-pushed view's load: cancel pops it.
        s.load_kind = LoadKind::Push;
        assert!(matches!(s.cancel_load(), LoadCancel::PoppedView));
        assert_eq!(s.views.len(), 1, "push-cancel drops the pushed view");
        assert_eq!(s.cursor, 0);
        // The launch load: cancel quits.
        s.load_kind = LoadKind::Launch;
        assert!(matches!(s.cancel_load(), LoadCancel::Quit));
    }

    #[test]
    fn promote_review_seeds_viewed() {
        let mut s = session_with(cs(&["a.rs", "b.rs", "c.rs"]));
        assert!(!s.is_review());
        s.promote_review();
        assert!(s.is_review());
        assert_eq!(s.state().viewed.len(), 3);
        assert!(s.state().viewed.iter().all(|v| !v));
    }

    #[test]
    fn resize_and_reveal_resizes_viewed_and_requests_reveal() {
        let mut s = session_with(cs(&["a.rs", "b.rs", "c.rs"]));
        // Force a size mismatch so the resize branch runs.
        s.state_mut().viewed = vec![true];
        s.state_mut().reveal_selected = false;
        s.resize_and_reveal();
        assert_eq!(
            s.state().viewed.len(),
            3,
            "viewed re-sized to the file count"
        );
        assert!(s.state().reveal_selected, "a sidebar reveal is requested");

        // Already correctly sized: the resize is skipped, the flag still set.
        s.state_mut().reveal_selected = false;
        s.resize_and_reveal();
        assert_eq!(s.state().viewed.len(), 3);
        assert!(s.state().reveal_selected);
    }

    #[test]
    fn drain_loader_without_a_loader_is_false() {
        let mut s = session_with(cs(&["a.rs"]));
        assert!(
            !s.drain_loader(LayoutMode::Stack, true, 20, 80),
            "no loader → nothing changed"
        );
    }

    #[test]
    fn resume_load_if_stale_is_a_noop_without_a_repo() {
        let mut s = session_with(cs(&["a.rs"]));
        // No repo_dir → start_load_undiffed returns early; nothing resumes.
        s.resume_load_if_stale();
        assert!(!s.loading());
    }

    /// A live session over the crate's own repo (HEAD vs its parent), with the
    /// view's changeset listed as undiffed stubs and `repo_dir` set so a real
    /// background load can run. `None` on an empty commit (nothing to load).
    fn loading_session() -> Option<(Session, Vec<FileStub>)> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let en = crate::git::enumerate(&dir, &LoadRequest::Show { rev: "HEAD".into() }).ok()?;
        if en.stubs.is_empty() {
            return None;
        }
        let files = en
            .stubs
            .iter()
            .map(crate::git::FileStub::as_stub_file)
            .collect();
        let view = Rc::new(Changeset {
            source: en.source.clone(),
            files,
        });
        let mut s = session_with(view);
        s.repo_dir = Some(dir);
        Some((s, en.stubs))
    }

    #[test]
    fn drain_loader_streams_diffs_and_clears_when_finished() {
        let Some((mut s, stubs)) = loading_session() else {
            return;
        };
        s.begin_load(stubs, false);
        assert!(s.loading(), "the background load started");

        // Pump the loader to completion (bounded), draining results into the cs.
        let mut changed = false;
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        while s.loading() && Instant::now() < deadline {
            if s.drain_loader(LayoutMode::Stack, true, 20, 80) {
                changed = true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(changed, "diffs were drained into the changeset");
        assert!(!s.loading(), "the loader is cleared once finished");
        assert!(
            s.cs().files.iter().all(|f| f.diffed),
            "every file ends up diffed"
        );
    }

    #[test]
    fn resume_load_if_stale_restarts_undiffed_work() {
        let Some((mut s, stubs)) = loading_session() else {
            return;
        };
        s.begin_load(stubs, false);
        // Abandon the in-flight load (as a view switch would) without draining,
        // so every file is still undiffed.
        s.clear_load();
        assert!(!s.loading());

        s.resume_load_if_stale();
        assert!(s.loading(), "undiffed work remains → the load resumes");
        // A second call while loading hits the early return.
        s.resume_load_if_stale();
        assert!(s.loading(), "already loading → a no-op");
    }
}
