//! App construction, current-view accessors, the view-history stack, the
//! streaming-diff load controls, theme application, and stream-highlight
//! requests.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use ratatui::layout::Rect;

use crate::model::{Changeset, LayoutMode};
use crate::tui::highlight::HlService;
use crate::tui::rows::{self, Plan};
use crate::tui::session::Session;
use crate::tui::sidebar;
use crate::tui::stream;
use crate::tui::theme::{Theme, ThemeName};
use crate::tui::view::{ViewEntry, ViewKind, ViewState};

use super::types::{App, Focus, Mode};

impl App {
    #[cfg(test)]
    pub fn new(cs: &Changeset) -> Self {
        Self::with_options(cs, LayoutMode::Stack, ThemeName::Dark)
    }

    #[cfg(test)]
    pub fn with_mode(cs: &Changeset, mode: LayoutMode) -> Self {
        Self::with_options(cs, mode, ThemeName::Dark)
    }

    #[cfg(test)]
    pub fn with_options(cs: &Changeset, mode: LayoutMode, theme: ThemeName) -> Self {
        Self::with_launch(cs, mode, theme, None, ViewKind::Local, true, None, None)
    }

    /// Construct the app seeding a single home view. `review` makes the home view
    /// a review session (viewed-tracking on). `base` is the explicit old-side ref
    /// (set for `diff --from <ref>`); `req` is how to reload the view.
    #[expect(
        clippy::too_many_arguments,
        reason = "with_launch threads every view-construction input through the single seeding path; a builder would obscure that"
    )]
    pub fn with_launch(
        cs: &Changeset,
        mode: LayoutMode,
        theme_name: ThemeName,
        repo_dir: Option<PathBuf>,
        kind: ViewKind,
        review: bool,
        base: Option<String>,
        req: Option<crate::git::LoadRequest>,
    ) -> Self {
        let rc: Rc<Changeset> = Rc::new(cs.clone());
        let viewed = vec![false; rc.files.len()];
        let theme = Theme::new(theme_name);
        let syntax = theme.name.syntax_table();
        let mut hl = HlService::new();
        hl.set_theme(theme.name);
        let banner = crate::tui::session::commit_banner(repo_dir.as_deref(), &kind);
        let plan = Plan::build_with_banner(
            rc.as_ref(),
            &viewed,
            mode,
            &std::collections::BTreeSet::new(),
            &banner,
        );
        let home = ViewEntry {
            kind,
            cs: rc,
            base,
            req,
            stubs: Arc::new(Vec::new()),
            state: ViewState {
                viewed,
                reveal_selected: true,
                ..Default::default()
            },
            plan,
            review,
            banner,
        };
        App {
            session: Session {
                views: vec![home],
                cursor: 0,
                repo_dir,
                loader: None,
                load_started: None,
                load_kind: crate::tui::session::LoadKind::Launch,
            },
            layout: mode,
            grouping: sidebar::Grouping::ByDir,
            viewport_h: 1,
            viewport_w: 1,
            sidebar_w: 34,
            sidebar_hidden: false,
            sidebar_top: 0,
            sidebar_visible: 1,
            sidebar_height: 1,
            mode: Mode::normal(),
            peek_viewport_h: 1,
            commit_msg_viewport_h: 1,
            theme,
            syntax,
            sidebar_area: Rect::default(),
            hl,
            flash: None,
            review_log: None,
            thread_marks: std::collections::HashMap::new(),
            launch_filtered: false,
            should_quit: false,
        }
    }

    // ---- current-view accessors (delegate to the session) ------------------
    // The view entry (owned by the session) holds the live `cs`/`state`/`plan`,
    // so there is exactly one `cs` handle (uniquely owned — the loader's
    // `make_mut` installs in place without cloning).

    #[cfg(test)]
    #[inline]
    pub(crate) fn cur_mut(&mut self) -> &mut ViewEntry {
        self.session.cur_mut()
    }
    /// The current view's changeset (the single live `Rc<Changeset>` handle).
    #[inline]
    pub fn cs(&self) -> &Rc<Changeset> {
        self.session.cs()
    }
    /// The current view's row plan.
    #[inline]
    pub fn plan(&self) -> &Plan {
        self.session.plan()
    }
    /// The current view's live navigation state.
    #[inline]
    pub fn state(&self) -> &ViewState {
        self.session.state()
    }
    #[inline]
    pub(crate) fn state_mut(&mut self) -> &mut ViewState {
        self.session.state_mut()
    }

    // ---- view stack --------------------------------------------------------

    pub fn is_review(&self) -> bool {
        self.session.is_review()
    }

    pub fn kind(&self) -> &ViewKind {
        self.session.kind()
    }

    pub fn source_is_local(&self) -> bool {
        self.session.source_is_local()
    }

    /// Make the entry at the cursor current. The entry already owns the live
    /// `cs`/`state`/`plan` (a cursor move *is* the view switch), so this only runs
    /// the switch side effects: abandon the previous load + re-anchor (`Session`),
    /// reset highlighting (the `App`-owned service), and re-clamp the viewport.
    /// Completed diffs are retained automatically — the loader installs directly
    /// into the entry's `cs`.
    fn load_current(&mut self) {
        self.session.clear_load();
        self.session.resize_and_reveal();
        self.session.build_plan(self.layout, self.grouped());
        self.hl.reset(self.theme.name);
        self.clamp();
    }

    /// Push a new view and make it current. `banner` overrides the derived
    /// commit-message banner when the caller already holds the message.
    pub(crate) fn push_view(
        &mut self,
        kind: ViewKind,
        cs: Rc<Changeset>,
        base: Option<String>,
        req: Option<crate::git::LoadRequest>,
        review: bool,
        banner: Option<Vec<String>>,
    ) {
        self.session
            .push_entry(kind, cs, base, req, review, self.layout, banner);
        self.load_current();
        self.set_focus(Focus::Stream);
    }

    /// Step back in the view history.
    pub fn view_back(&mut self) {
        if self.session.cursor > 0 {
            self.session.cursor -= 1;
            self.load_current();
            self.session.resume_load_if_stale();
        }
    }

    /// Step forward in the view history.
    pub fn view_forward(&mut self) {
        if self.session.cursor + 1 < self.session.views.len() {
            self.session.cursor += 1;
            self.load_current();
            self.session.resume_load_if_stale();
        }
    }

    /// Return to the home (launch) view, when it supports it.
    pub fn view_home(&mut self) {
        if self.session.cursor != 0 && self.session.home_reviewable() {
            self.session.cursor = 0;
            self.load_current();
            self.session.resume_load_if_stale();
        }
    }

    /// Promote the current browse view into a review session.
    pub fn promote_review(&mut self) {
        self.session.promote_review();
    }

    /// Switch to a commit's diff (browse), optionally landing on `land_path`.
    /// The file list appears immediately; the diffs stream in behind it.
    /// `msg` is the commit's message when the caller already fetched it (the
    /// popup) — its banner is reused instead of re-reading the repo.
    /// Returns whether the switch happened (`false`: no repo, or enumeration
    /// failed), so callers don't tear down their own state for nothing.
    pub fn open_commit(
        &mut self,
        rev: &str,
        land_path: Option<&str>,
        msg: Option<&crate::model::CommitMessage>,
    ) -> bool {
        let Some(dir) = self.session.repo_dir.clone() else {
            return false;
        };
        let req = crate::git::LoadRequest::Show {
            rev: rev.to_string(),
        };
        // One repository handle for the whole switch: the file enumeration and the
        // message banner both read from it, instead of discovering twice (the old
        // path re-discovered inside Session::push_entry -> commit_banner).
        let Ok(repo) = gix::discover(&dir) else {
            return false;
        };
        let Ok(en) = crate::git::enumerate_in(&repo, &req) else {
            return false;
        };
        let cs = stub_changeset(&en);
        // Reuse the caller's already-fetched message; else read it over the same
        // handle. Passing the banner prebuilt keeps push_entry from re-discovering.
        let banner = match msg {
            Some(m) => crate::tui::session::banner_lines(m),
            None => crate::git::commit_message_in(&repo, rev)
                .map(|m| crate::tui::session::banner_lines(&m))
                .unwrap_or_default(),
        };
        self.push_view(
            ViewKind::Commit(rev.to_string()),
            Rc::new(cs),
            None,
            Some(req),
            false,
            Some(banner),
        );
        self.session.begin_load(en.stubs, true);
        if let Some(path) = land_path {
            if let Some(i) = self.cs().files.iter().position(|f| f.path == path) {
                self.land_on_file(i);
            }
        }
        true
    }

    // ---- streaming diff load (the session owns the machine) -----------------

    /// Begin the initial background diff load for the current view.
    pub fn begin_load(&mut self, stubs: Vec<crate::git::FileStub>, is_switch: bool) {
        self.session.begin_load(stubs, is_switch);
    }

    /// Drain ready diff results into the view-owned changeset; returns whether
    /// anything changed (so the caller can redraw).
    pub fn drain_loader(&mut self) -> bool {
        self.session.drain_loader(
            self.layout,
            self.grouped(),
            self.viewport_h,
            self.viewport_w,
        )
    }

    /// Install everything the background workers finished — streaming diffs, a
    /// completed blame, and finished syntax highlights — in one call; returns
    /// whether anything landed (needs a repaint). The event loop calls this
    /// instead of draining each worker by name.
    pub fn drain_background(&mut self) -> bool {
        let mut dirty = self.drain_loader();
        dirty |= self.drain_blame();
        dirty |= self.hl.drain();
        dirty
    }

    /// Whether a background job that drives the brief poll cadence is in flight
    /// (a diff load or a blame). Highlighting is excluded — it repaints when it
    /// lands but doesn't need the fast cadence (plain text shows meanwhile).
    pub fn background_active(&self) -> bool {
        self.loading() || self.peek_blame_loading()
    }

    /// Cancel an in-progress load. The launch load quits; a freshly-pushed
    /// commit view is abandoned (popped) and resumes the previous view's load;
    /// a resumed load on an already-visited view is merely stopped (stay put,
    /// no restart) so revisiting-then-Esc can't destroy the view or the app.
    pub fn cancel_load(&mut self) {
        use crate::tui::session::LoadCancel;
        if !self.session.loading() {
            return;
        }
        match self.session.cancel_load() {
            LoadCancel::PoppedView => {
                self.load_current();
                self.session.resume_load_if_stale();
            }
            LoadCancel::Stopped => self.load_current(),
            LoadCancel::Quit => self.should_quit = true,
        }
    }

    /// Whether a background diff load is in progress.
    pub fn loading(&self) -> bool {
        self.session.loading()
    }

    /// Whether to show progress chrome: a load is active and past the threshold.
    pub fn show_progress(&self) -> bool {
        self.session.show_progress()
    }

    /// Progress as `(done, total)` for the active load (zero/zero when idle).
    pub fn load_progress(&self) -> (usize, usize) {
        self.session.load_progress()
    }

    /// Whether the side-by-side (split) layout is active.
    pub fn is_split(&self) -> bool {
        matches!(self.plan().layout, LayoutMode::Split)
    }

    pub fn h_scroll_by(&mut self, delta: isize) {
        let vw = self.viewport_w;
        let e = self.session.cur_mut();
        stream::h_scroll_by(&mut e.state, &e.plan, vw, delta);
    }

    pub fn toggle_wrap(&mut self) {
        stream::toggle_wrap(self.state_mut());
    }

    /// Make `name` the active theme: rebuild the chrome palette and the syntax
    /// color table, and tell the highlighter so theme-dependent (syntect) files
    /// re-highlight. Tree-sitter/plain content recolors instantly via the table.
    pub fn apply_theme(&mut self, name: ThemeName) {
        self.theme = Theme::new(name);
        self.syntax = self.theme.name.syntax_table();
        self.hl.set_theme(self.theme.name);
    }

    /// Attach the worktree's review log and record whether the launch view was
    /// narrowed by path filters. Called once, from `tui::run`.
    pub fn attach_review_log(&mut self, log: Option<crate::review::Log>, filtered: bool) {
        self.review_log = log;
        self.launch_filtered = filtered;
    }

    pub fn clamp(&mut self) {
        let (vh, vw) = (self.viewport_h, self.viewport_w);
        let e = self.session.cur_mut();
        // `usable`, not `vh`: this runs on every draw, so clamping against the
        // full viewport here would silently undo every bound computed against
        // the drawn height — one row back, every frame.
        let usable = stream::usable(&e.plan, vh);
        stream::clamp(&mut e.state, &e.plan, usable, vw);
    }

    // ---- highlighting ------------------------------------------------------

    /// Request highlighting for the files currently visible in the stream.
    pub fn request_visible(&mut self) {
        if self.session.total_rows() == 0 {
            return;
        }
        let starts = self.file_starts();
        // `file_at` returns visible ordinals; map them to cs indices via
        // `visible_files` so highlighting requests the on-screen files even when
        // some directories are folded.
        let start = rows::file_at(starts, self.state().scroll);
        let end_row = (self.state().scroll + self.viewport_h).min(self.session.total_rows() - 1);
        let end = rows::file_at(starts, end_row);
        let cs = self.cs().clone();
        let visible = self.plan().visible_files.clone();
        for ord in start..=end {
            if let Some(&idx) = visible.get(ord) {
                if let Some(file) = cs.files.get(idx) {
                    self.hl.request(idx, file);
                }
            }
        }
    }
}

/// Build a changeset of undiffed stubs from an enumeration, for instant display
/// while the diffs stream in.
pub fn stub_changeset(en: &crate::git::Enumeration) -> Changeset {
    Changeset {
        source: en.source.clone(),
        files: en
            .stubs
            .iter()
            .map(crate::git::FileStub::as_stub_file)
            .collect(),
    }
}

#[cfg(test)]
impl App {
    /// Push a synthetic view (no repo load) so tests can exercise the view stack.
    pub fn push_test_view(&mut self, cs: &Changeset, kind: ViewKind, review: bool) {
        let rc: Rc<Changeset> = Rc::new(cs.clone());
        self.push_view(kind, rc, None, None, review, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::LoadRequest;
    use crate::model::{DiffFile, FileStatus, Hunk, Line, LineKind, Stats};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn crate_repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// A one-file diffed changeset with text, so highlighting can be requested.
    fn sample_diffed() -> Changeset {
        let hunk = Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 1,
            lines: vec![
                Line {
                    kind: LineKind::Removed,
                    old_lineno: Some(1),
                    new_lineno: None,
                    text: "old".into(),
                    emphasis: None,
                },
                Line {
                    kind: LineKind::Added,
                    old_lineno: None,
                    new_lineno: Some(1),
                    text: "new".into(),
                    emphasis: None,
                },
            ],
        };
        let f = DiffFile {
            path: "a.rs".into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks: vec![hunk],
            stats: Stats {
                additions: 1,
                deletions: 1,
            },
            language: Some("rust".into()),
            is_binary: false,
            old_text: Some("old\n".into()),
            new_text: Some("new\n".into()),
            content_digest: None,
            diffed: true,
        };
        Changeset {
            source: "t".into(),
            files: vec![f],
        }
    }

    /// Build an app showing `rev`'s diff as undiffed stubs, with the loader not
    /// yet started. Returns the app and the stubs to feed [`App::begin_load`].
    fn stub_app(rev: &str) -> (App, Vec<crate::git::FileStub>) {
        let dir = crate_repo();
        let req = LoadRequest::Show { rev: rev.into() };
        let en = crate::git::enumerate(&dir, &req).unwrap();
        let cs = stub_changeset(&en);
        let app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir),
            ViewKind::Commit(rev.into()),
            false,
            None,
            Some(req),
        );
        (app, en.stubs)
    }

    /// Drive a streaming load to completion (bounded).
    fn drive(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.loading() && Instant::now() < deadline {
            app.drain_loader();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn streaming_installs_diffs_and_finishes() {
        let (mut app, stubs) = stub_app("HEAD");
        if stubs.is_empty() {
            return;
        }
        let n = stubs.len();
        // Files start as undiffed stubs.
        assert!(
            app.cs().files.iter().all(|f| !f.diffed),
            "stubs before load"
        );
        app.begin_load(stubs, false);
        assert!(app.loading());
        // No progress chrome immediately — under the threshold.
        assert!(
            !app.show_progress(),
            "fast load shows no chrome before threshold"
        );
        assert_eq!(app.load_progress().1, n, "total known up front");

        drive(&mut app);
        assert!(!app.loading(), "load finished");
        assert!(app.cs().files.iter().all(|f| f.diffed), "all files diffed");
        // The completed changeset is synced back into the view entry.
        assert!(
            app.session.views[app.session.cursor]
                .cs
                .files
                .iter()
                .all(|f| f.diffed),
            "entry synced"
        );
        // The plan no longer carries any Pending placeholder rows.
        assert!(
            !app.plan()
                .rows
                .iter()
                .any(|r| matches!(r, rows::Row::Pending)),
            "no pending rows after completion"
        );
    }

    #[test]
    fn abandoned_load_retains_progress_and_resumes_only_remainder() {
        let (mut app, stubs) = stub_app("HEAD");
        let n = stubs.len();
        if n < 2 {
            return; // need at least one completed + one unfinished file
        }
        // Record the view's stubs (as begin_load would) without spawning threads.
        app.session.views[0].stubs = Arc::new(stubs);
        let other = (*app.cs()).clone();
        // Simulate a load that completed every file but the last before we left.
        {
            let cs = Rc::make_mut(&mut app.cur_mut().cs);
            for f in cs.files.iter_mut().take(n - 1) {
                f.diffed = true;
            }
        }
        // Switch away: progress is already on the entry (installed into its cs).
        app.push_test_view(&other, ViewKind::Commit("x".into()), false);
        let kept = &app.session.views[0].cs;
        assert!(
            kept.files.iter().take(n - 1).all(|f| f.diffed),
            "completed diffs are retained on the abandoned view's entry"
        );
        assert!(
            !kept.files[n - 1].diffed,
            "the unfinished file stays undiffed"
        );

        // Return: the resumed load re-dispatches only the still-undiffed file, at
        // its original index — no re-enumeration, no from-scratch redo.
        app.view_back();
        assert_eq!(app.session.cursor, 0);
        assert_eq!(
            app.cs().files.len(),
            n,
            "file set length is fixed for the view's lifetime"
        );
        assert!(
            app.cs().files.iter().take(n - 1).all(|f| f.diffed),
            "previously completed diffs survive the return"
        );
        assert_eq!(
            app.load_progress().1,
            1,
            "resume diffs only the remainder, not all {n} files"
        );
    }

    #[test]
    fn file_set_is_stable_across_switch_away_and_return() {
        let (mut app, _stubs) = stub_app("HEAD");
        if app.cs().files.is_empty() {
            return;
        }
        let paths: Vec<String> = app.cs().files.iter().map(|f| f.path.clone()).collect();
        let other = (*app.cs()).clone();
        app.push_test_view(&other, ViewKind::Commit("x".into()), false);
        app.view_back();
        let after: Vec<String> = app.cs().files.iter().map(|f| f.path.clone()).collect();
        assert_eq!(
            after, paths,
            "paths and order are identical on return; no re-enumeration"
        );
    }

    #[test]
    fn transient_cs_clone_does_not_corrupt_install() {
        // A render-measure clone (as `request_visible` takes) may be alive when a
        // drain installs a completed diff. The install must apply to the live
        // changeset via copy-on-write without disturbing the transient clone.
        let (mut app, stubs) = stub_app("HEAD");
        if stubs.is_empty() {
            return;
        }
        let snapshot = app.cs().clone();
        {
            let cs = Rc::make_mut(&mut app.cur_mut().cs);
            cs.files[0].diffed = true;
        }
        assert!(
            app.cs().files[0].diffed,
            "install applied to the live changeset"
        );
        assert!(
            !snapshot.files[0].diffed,
            "the transient render clone is unaffected"
        );
        drop(snapshot);
    }

    #[test]
    fn cancel_at_startup_quits() {
        let (mut app, stubs) = stub_app("HEAD");
        if stubs.is_empty() {
            return;
        }
        app.begin_load(stubs, false); // launch load (not a switch)
        app.cancel_load();
        assert!(app.should_quit, "cancelling the launch load quits");
        assert!(!app.loading());
    }

    #[test]
    fn cancel_resumed_load_stops_without_quitting_or_popping() {
        // A resumed load on an already-visited view: Esc just stops it — it must
        // not quit the app (as the launch load does) nor pop the view (as a
        // fresh push does).
        let cs = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::new(&cs);
        app.session.loader = Some(crate::tui::loader::Loader::start(
            std::path::PathBuf::new(),
            Vec::new(),
        ));
        app.session.load_kind = crate::tui::session::LoadKind::Resume;
        let views = app.session.views.len();
        app.cancel_load();
        assert!(!app.should_quit, "a resumed-load cancel does not quit");
        assert_eq!(app.session.views.len(), views, "and keeps the view");
        assert!(!app.loading(), "the loader was dropped");
    }

    #[test]
    fn cancel_switch_returns_to_previous_view() {
        let dir = crate_repo();
        let req = LoadRequest::WorkingTree {
            include_untracked: true,
            base: None,
        };
        let en = crate::git::enumerate(&dir, &req).unwrap();
        let cs = stub_changeset(&en);
        let mut app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir),
            ViewKind::Local,
            true,
            None,
            Some(req),
        );
        app.begin_load(en.stubs, false);
        drive(&mut app); // let home finish so it survives the round trip

        // Switch to a commit (pushes a stub view + a switch loader).
        app.open_commit("HEAD", None, None);
        assert_eq!(app.session.cursor, 1, "pushed a commit view");
        assert!(app.loading(), "switch load streaming");

        app.cancel_load();
        assert_eq!(app.session.cursor, 0, "cancel returns to the previous view");
        assert!(!app.should_quit, "a switch cancel does not quit");
        assert!(
            app.cs().files.iter().all(|f| f.diffed),
            "previous (home) view intact"
        );
    }

    #[test]
    fn request_visible_is_noop_for_an_empty_changeset() {
        let cs = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::new(&cs);
        // total_rows() == 0 → early return; nothing is requested.
        app.request_visible();
        assert!(
            app.hl.needs(0),
            "no highlight requested for empty changeset"
        );
    }

    #[test]
    fn request_visible_requests_on_screen_files() {
        let cs = sample_diffed();
        let mut app = App::new(&cs);
        assert!(app.hl.needs(0), "not yet requested");
        app.request_visible();
        assert!(
            !app.hl.needs(0),
            "the visible file's highlight was requested"
        );
    }

    #[test]
    fn commit_view_has_a_message_banner_but_local_does_not() {
        let dir = crate_repo();
        let empty = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let has_banner = |app: &App| {
            app.plan()
                .rows
                .iter()
                .any(|r| matches!(r, rows::Row::Banner(_)))
        };

        // Launched on a commit (with a repo) → the message banner is present.
        let commit = App::with_launch(
            &empty,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir.clone()),
            ViewKind::Commit("HEAD".into()),
            false,
            None,
            None,
        );
        assert!(has_banner(&commit), "a commit view shows a message banner");

        // Launched local → no banner.
        let local = App::with_launch(
            &empty,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir),
            ViewKind::Local,
            false,
            None,
            None,
        );
        assert!(!has_banner(&local), "a local view has no banner");

        // A commit kind with no repo → no banner (the early-return branch).
        let no_repo = App::with_launch(
            &empty,
            LayoutMode::Stack,
            ThemeName::Dark,
            None,
            ViewKind::Commit("HEAD".into()),
            false,
            None,
            None,
        );
        assert!(!has_banner(&no_repo), "no repo → no banner");

        // A bad rev (fetch error) → no banner (the `Err` arm of commit_banner).
        let bad_rev = App::with_launch(
            &empty,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(crate_repo()),
            ViewKind::Commit("zzz-no-such-rev".into()),
            false,
            None,
            None,
        );
        assert!(
            !has_banner(&bad_rev),
            "an unresolvable rev yields no banner"
        );
    }

    /// The banner survives a streaming rebuild — *while the cursor is in it*.
    /// `clamp` moves the viewport to keep the cursor visible, so a cursor parked
    /// on a file whose rows move takes the viewport along; see `stream::reanchored`.
    #[test]
    fn streaming_rebuild_keeps_the_banner_in_view() {
        let dir = crate_repo();
        let empty = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::with_launch(
            &empty,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir),
            ViewKind::Local,
            true,
            None,
            None,
        );
        app.open_commit("HEAD", None, None);
        let first_file = app.plan().file_starts.first().copied().unwrap_or(0);
        assert!(
            first_file > 0,
            "the commit view has a banner above the first file"
        );
        // Park the viewport inside the banner, then rebuild as a streaming drain
        // would: the viewport must stay in the banner, not snap to the first file.
        app.state_mut().scroll = 0;
        let (layout, grouped) = (app.layout, app.grouped());
        app.session.rebuild_plan(layout, grouped, 20, 80);
        assert_eq!(
            app.state().scroll,
            0,
            "the banner stays in view across a plan rebuild"
        );
    }

    #[test]
    fn open_commit_attaches_a_banner_to_the_pushed_view() {
        let dir = crate_repo();
        let empty = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::with_launch(
            &empty,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir),
            ViewKind::Local,
            true,
            None,
            None,
        );
        app.open_commit("HEAD", None, None);
        assert_eq!(app.session.cursor, 1, "a commit view was pushed");
        assert!(
            !app.session.cur_mut().banner.is_empty(),
            "the pushed commit view carries a message banner"
        );
        assert!(
            app.plan()
                .rows
                .iter()
                .any(|r| matches!(r, rows::Row::Banner(_))),
            "the plan includes the banner rows"
        );
    }

    #[test]
    fn open_commit_without_repo_dir_is_noop() {
        let cs = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::new(&cs); // repo_dir is None
        app.open_commit("HEAD", None, None);
        assert_eq!(app.session.cursor, 0, "no view pushed without a repo dir");
        assert_eq!(app.session.views.len(), 1);
    }

    #[test]
    fn open_commit_with_a_bad_rev_is_noop() {
        let dir = crate_repo();
        let cs = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir),
            ViewKind::Local,
            true,
            None,
            None,
        );
        assert!(
            !app.open_commit("zzz-no-such-rev-zzz", None, None),
            "a failed switch reports false"
        );
        assert_eq!(app.session.cursor, 0, "enumerate failure pushes no view");
        assert_eq!(app.session.views.len(), 1);
    }

    #[test]
    fn open_commit_lands_on_the_requested_file() {
        let dir = crate_repo();
        let req = LoadRequest::Show { rev: "HEAD".into() };
        let en = crate::git::enumerate(&dir, &req).unwrap();
        let Some(path) = en.stubs.first().map(|s| s.path.clone()) else {
            return; // commit touched no files
        };
        let cs = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(dir),
            ViewKind::Local,
            true,
            None,
            None,
        );
        assert!(
            app.open_commit("HEAD", Some(&path), None),
            "the switch succeeded"
        );
        assert_eq!(app.session.cursor, 1, "the commit view was pushed");
        let i = app
            .cs()
            .files
            .iter()
            .position(|f| f.path == path)
            .expect("landed file present");
        assert_eq!(app.state().selected, i, "landed on the requested file");
    }

    #[test]
    fn wrap_and_h_scroll_do_not_leak_across_views() {
        let (mut app, _) = stub_app("HEAD");
        if app.cs().files.is_empty() {
            return;
        }
        app.state_mut().h_scroll = 5;
        app.state_mut().wrap = true;
        let other = (*app.cs()).clone();
        app.push_test_view(&other, ViewKind::Commit("x".into()), false);
        // The new view does not inherit the home view's horizontal scroll / wrap.
        assert_eq!(
            app.state().h_scroll,
            0,
            "h_scroll does not leak into the new view"
        );
        assert!(!app.state().wrap, "wrap does not leak into the new view");
        // Returning restores the home view's own wrap (h_scroll may be clamped to
        // the view's content width, so we assert the unclamped flag).
        app.view_back();
        assert!(
            app.state().wrap,
            "the home view's wrap is restored on return"
        );
    }
}
