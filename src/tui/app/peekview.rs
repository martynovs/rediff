//! The modal single-file peek: opening it (content or diff mode), sourcing its
//! text from the cache or directly from git, scrolling/paging, hunk jumps, and
//! its reserved highlight slot.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use crate::model::BlameLine;
use crate::tui::peek::{Peek, PeekMode};
use crate::tui::view::ViewKind;

use super::types::{App, Base, Focus, PEEK_HL};

impl App {
    /// The open peek, if the base is a peek.
    pub fn peek(&self) -> Option<&Peek> {
        match &self.mode.base {
            Base::Peek(p) => Some(p.as_ref()),
            Base::Normal { .. } => None,
        }
    }

    fn peek_mut(&mut self) -> Option<&mut Peek> {
        match &mut self.mode.base {
            Base::Peek(p) => Some(p.as_mut()),
            Base::Normal { .. } => None,
        }
    }

    /// Whether the single-file peek is the active base.
    pub fn peek_open(&self) -> bool {
        matches!(self.mode.base, Base::Peek(_))
    }

    // ---- single-file peek --------------------------------------------------

    /// Open the peek in content mode (`p`): the selected file at the view's new
    /// side, with its diff (when toggled) comparing that version against TOP.
    pub fn open_peek_preview(&mut self) {
        let Some(f) = self
            .state()
            .selected_file()
            .and_then(|i| self.cs().files.get(i))
        else {
            return;
        };
        if f.is_binary {
            return;
        }
        let path = f.path.clone();
        let content = self.peek_new_text(f);
        let top = self.home_top_text(&path);
        let origin_local = self.source_is_local();
        // Preview diffs the new-side content against home TOP (content as old);
        // the content pane shows that new side (diff_old).
        let peek = Peek::new(path, origin_local, PeekMode::Content, content, top, false);
        self.set_peek(peek);
    }

    /// Open the peek in diff mode (`=`): the selected file's own change for the
    /// current view, with an expanded, adjustable context level.
    pub fn open_peek_review(&mut self) {
        let Some(f) = self
            .state()
            .selected_file()
            .and_then(|i| self.cs().files.get(i))
        else {
            return;
        };
        if f.is_binary {
            return;
        }
        let path = f.path.clone();
        let (old, new) = self.diff_sides(f);
        let origin_local = self.source_is_local();
        // Review diffs old-vs-new; the content pane shows the new side (diff_new).
        let peek = Peek::new(path, origin_local, PeekMode::Diff, old, new, true);
        self.set_peek(peek);
    }

    /// The view's new-side text for `path`, over an already-open repo: the new
    /// rev's blob (commit / range target) or the working copy (local/staged).
    /// Shared by the single-side preview read and the both-sides diff read.
    fn new_side_text_in(&self, repo: &gix::Repository, path: &str) -> String {
        match self.kind() {
            ViewKind::Commit(rev) => crate::git::file_text_at_in(repo, rev, path),
            ViewKind::Range { target, .. } => crate::git::file_text_at_in(repo, target, path),
            ViewKind::Local | ViewKind::Staged => crate::git::worktree_text_in(repo, path),
        }
        .unwrap_or_default()
    }

    /// The new-side text for the peeked file: the cached diff text when the file
    /// is already diffed, else sourced directly from git by the current view's
    /// new side — so the peek works on a not-yet-diffed (stub) file.
    fn peek_new_text(&self, f: &crate::model::DiffFile) -> String {
        if f.diffed {
            return f.new_text.clone().unwrap_or_default();
        }
        let Some(dir) = self.session.repo_dir.as_deref() else {
            return String::new();
        };
        let Ok(repo) = gix::discover(dir) else {
            return String::new();
        };
        self.new_side_text_in(&repo, &f.path)
    }

    /// The peeked file's `(old, new)` diff-side texts. The cached texts when the
    /// file is already diffed; otherwise both sides are read over ONE repository
    /// handle (a stub peeked before its diff streamed in) instead of two
    /// discovers. The old side is the view's old-side rev ([`Self::blame_revs`],
    /// shared with the blame fallback so they can't drift).
    fn diff_sides(&self, f: &crate::model::DiffFile) -> (String, String) {
        if f.diffed {
            return (
                f.old_text.clone().unwrap_or_default(),
                f.new_text.clone().unwrap_or_default(),
            );
        }
        let Some(dir) = self.session.repo_dir.as_deref() else {
            return (String::new(), String::new());
        };
        let Ok(repo) = gix::discover(dir) else {
            return (String::new(), String::new());
        };
        let new = self.new_side_text_in(&repo, &f.path);
        let old_rev = self.blame_revs().1;
        let old = crate::git::file_text_at_in(&repo, &old_rev, &f.path).unwrap_or_default();
        (old, new)
    }

    fn set_peek(&mut self, mut peek: Peek) {
        // Open consistent with the main view's current layout; `m` toggles it.
        peek.set_split(self.is_split());
        self.hl.forget(PEEK_HL);
        self.mode.base = Base::Peek(Box::new(peek));
    }

    /// Toggle the peek's diff layout between unified and side-by-side. `m` is a
    /// diff-mode binding: in content/blame mode it must not silently arm a
    /// layout change that would only surface on a later Tab into diff mode.
    pub fn peek_toggle_split(&mut self) {
        let vh = self.peek_viewport_h.max(1);
        if let Some(p) = self.peek_mut() {
            if p.mode != PeekMode::Diff {
                return;
            }
            p.set_split(!p.split_view);
            p.state.scroll = p.state.scroll.min(p.active_rows().saturating_sub(vh));
        }
    }

    /// Text of a file at TOP — the newest side of the launch (home) review:
    /// the working copy for a local/staged review, the target for a range, the
    /// commit for a single-commit/`show` launch.
    fn home_top_text(&self, path: &str) -> String {
        let Some(dir) = self.session.repo_dir.clone() else {
            return String::new();
        };
        match self.session.views.first().map(|v| &v.kind) {
            Some(ViewKind::Commit(rev)) => {
                crate::git::file_text_at(&dir, rev, path).unwrap_or_default()
            }
            Some(ViewKind::Range { target, .. }) => {
                crate::git::file_text_at(&dir, target, path).unwrap_or_default()
            }
            _ => crate::git::worktree_text(&dir, path).unwrap_or_default(),
        }
    }

    pub fn peek_toggle_mode(&mut self) {
        // A `b`-opened peek defers its content/diff sides; fill them BEFORE the
        // toggle so the single post-toggle rebuild uses them. Filling afterwards
        // would rebuild an empty-sided plan first and immediately discard it.
        self.fill_peek_sides();
        if let Some(p) = self.peek_mut() {
            p.toggle_mode();
        }
        self.hl.forget(PEEK_HL);
        // Cycling into blame fetches the committed content and starts the worker.
        self.prepare_blame();
    }

    /// Fill a blame-opened peek's content/diff sides just before it first leaves
    /// blame — `b` defers them (blame renders its own committed-rev text), so the
    /// first Tab out pays the read instead of every `b` keypress. Installs them
    /// quietly; the mode toggle that follows rebuilds the plan once.
    fn fill_peek_sides(&mut self) {
        let path = match self.peek() {
            // Still in blame (about to leave) with sides unfilled — the only state
            // that defers them; Content/Diff always open with their sides.
            Some(p) if p.mode == PeekMode::Blame && p.sides_unfilled() => p.path.clone(),
            _ => return,
        };
        let (old, new) = {
            let Some(f) = self.cs().files.iter().find(|f| f.path == path) else {
                return;
            };
            self.diff_sides(f)
        };
        if let Some(p) = self.peek_mut() {
            // Review-style sides (old-vs-new); content pane shows the new side.
            p.install_sides(old, new, true);
        }
    }

    // ---- blame mode --------------------------------------------------------

    /// Open the peek directly in blame mode (`b`) for the selected file and
    /// kick off the background blame. The content/diff sides are deferred until
    /// the first Tab out of blame (see `fill_peek_sides`), so `b` pays for the
    /// blame fetch only.
    pub fn open_peek_blame(&mut self) {
        let Some(f) = self
            .state()
            .selected_file()
            .and_then(|i| self.cs().files.get(i))
        else {
            return;
        };
        if f.is_binary {
            return;
        }
        let path = f.path.clone();
        let origin_local = self.source_is_local();
        let peek = Peek::new_blame(path, origin_local);
        self.set_peek(peek);
        self.prepare_blame();
    }

    /// A local/staged view's old-side base rev: its explicit `--from` ref, else
    /// HEAD. Feeds [`Self::blame_revs`] (which both the diff old side via
    /// [`Self::diff_sides`] and the blame fallback read) so they share one base
    /// — they drifted before: blame hardcoded HEAD while the diff honored
    /// `--from`.
    fn local_base_rev(&self) -> String {
        self.session
            .views
            .get(self.session.cursor)
            .and_then(|v| v.base.clone())
            .unwrap_or_else(|| "HEAD".to_string())
    }

    /// The `(blame rev, old-side fallback rev)` pair for the current view: where
    /// blame is computed (a committed rev — HEAD for a local/staged view, the
    /// viewed commit, or a range's target) and, for a file the new side deleted,
    /// the old side where its history still lives (the commit's parent, the range
    /// base, or the local view's `--from` base). One match, so a new `ViewKind`
    /// variant is handled in a single place and the two revs can't drift.
    fn blame_revs(&self) -> (String, String) {
        match self.kind() {
            ViewKind::Commit(rev) => (rev.clone(), format!("{rev}^")),
            ViewKind::Range { target, base } => (target.clone(), base.clone()),
            ViewKind::Local | ViewKind::Staged => ("HEAD".to_string(), self.local_base_rev()),
        }
    }

    /// Ensure a blame peek has its background fetch (committed-rev content +
    /// attribution) running. A no-op outside blame mode, while a fetch is in
    /// flight, or once one has settled (even empty — [`Peek::needs_blame`] is
    /// only true in blame mode with nothing settled or in flight).
    fn prepare_blame(&mut self) {
        let Some(path) = self
            .peek()
            .filter(|p| p.needs_blame())
            .map(|p| p.path.clone())
        else {
            return;
        };
        let Some(dir) = self.session.repo_dir.clone() else {
            return;
        };
        // Fallbacks for a path with no history at the blame rev: a (staged)
        // rename blames its previous path, and a file the viewed commit
        // deleted blames the old side — where its content still exists.
        let (prev_path, deleted) = {
            let f = self.cs().files.iter().find(|f| f.path == path);
            (
                f.and_then(|f| f.previous_path.clone()),
                f.is_some_and(|f| f.status == crate::model::FileStatus::Deleted),
            )
        };
        let (rev, old_side) = self.blame_revs();
        let fallback_rev = deleted.then_some(old_side);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            // All git work lives on the worker, so a repo walk + large blob
            // decode can't stall the UI thread between keystrokes.
            if let Some(result) = blame_fetch(
                &dir,
                &rev,
                &path,
                prev_path.as_deref(),
                fallback_rev.as_deref(),
                &worker_cancel,
            ) {
                // Receiver gone (peek closed) → discard the send error. The empty
                // `if` is deliberate: the workspace lints both `let _ = …` and
                // `.ok()`, so a matched-and-ignored Result is the only clean form.
                if tx.send(result).is_err() {}
            }
        });
        if let Some(p) = self.peek_mut() {
            p.blame_rx = Some(rx);
            p.blame_cancel = Some(cancel);
        }
    }

    /// Install a completed blame result into the open peek; returns whether one
    /// landed (so the caller can redraw).
    pub fn drain_blame(&mut self) -> bool {
        use std::sync::mpsc::TryRecvError;
        let Some(p) = self.peek_mut() else {
            return false;
        };
        let (text, lines) = match p.blame_rx.as_ref().map(mpsc::Receiver::try_recv) {
            Some(Ok(result)) => result,
            // The worker died without sending (a panic inside gix): settle on
            // "no blame" so the loading header — and the 16ms poll cadence it
            // drives — can't stick for the rest of the session.
            Some(Err(TryRecvError::Disconnected)) => (String::new(), Vec::new()),
            Some(Err(TryRecvError::Empty)) | None => return false,
        };
        p.install_blame(text, lines);
        // The event loop has already requested a highlight for the empty
        // placeholder text; invalidate the slot so the freshly installed
        // committed text gets highlighted rather than inheriting that miss.
        self.hl.forget(PEEK_HL);
        true
    }

    /// Whether the open peek is computing blame (drives the brief poll cadence).
    pub fn peek_blame_loading(&self) -> bool {
        self.peek().is_some_and(Peek::blame_loading)
    }

    /// Open the commit-message popup for the blame line under the cursor (`Enter`
    /// in blame mode). A no-op in other modes — their plans index a different
    /// text (worktree vs committed rev), so the blame vector would misattribute —
    /// and before blame has loaded.
    pub fn peek_blame_open_message(&mut self) {
        let sha = self
            .peek()
            .filter(|p| p.mode == PeekMode::Blame)
            .and_then(Peek::blame_cursor)
            .map(|b| b.commit.sha.clone());
        if let Some(sha) = sha {
            self.open_commit_message(&sha);
        }
    }

    /// Flip the peek's diff context between full (`=`) and compact (`-`).
    /// The text is unchanged (only the context level), so the line-keyed
    /// highlight cache stays valid — no `forget`, hence no re-highlight flash.
    pub fn peek_set_full(&mut self, full: bool) {
        let vh = self.peek_viewport_h.max(1);
        if let Some(p) = self.peek_mut() {
            p.set_full(full);
            // The compact plan is shorter; keep the last page full so content
            // can't scroll off the top.
            p.state.scroll = p.state.scroll.min(p.active_rows().saturating_sub(vh));
        }
    }

    pub fn peek_scroll(&mut self, delta: isize) {
        // The shared stream nav stops one viewport short of the end (max_scroll =
        // rows - viewport), so the last page stays full — the peek's prior clamp.
        let vh = self.peek_viewport_h.max(1);
        if let Some(p) = self.peek_mut() {
            crate::tui::stream::scroll_by(&mut p.state, &p.plan, vh, delta);
        }
    }

    #[expect(
        clippy::cast_possible_wrap,
        reason = "viewport height is a small terminal dimension, well under isize::MAX"
    )]
    pub fn peek_half_page(&mut self, dir: isize) {
        let step = (self.peek_viewport_h / 2).max(1) as isize;
        self.peek_scroll(dir * step);
    }

    #[expect(
        clippy::cast_possible_wrap,
        reason = "viewport height is a small terminal dimension, well under isize::MAX"
    )]
    pub fn peek_page(&mut self, dir: isize) {
        let step = self.peek_viewport_h.saturating_sub(1).max(1) as isize;
        self.peek_scroll(dir * step);
    }

    pub fn peek_top(&mut self) {
        self.peek_scroll(isize::MIN / 2);
    }

    pub fn peek_bottom(&mut self) {
        self.peek_scroll(isize::MAX / 2);
    }

    /// Jump the peek to the previous/next change region (works in full-context
    /// mode too, and uses the active plan's row positions so it lands correctly
    /// in both unified and split layouts).
    pub fn peek_hunk(&mut self, dir: isize) {
        if let Some(p) = self.peek_mut() {
            let cur = p.state.scroll;
            let target = if dir > 0 {
                p.change_starts.iter().copied().find(|&r| r > cur)
            } else {
                p.change_starts.iter().copied().rev().find(|&r| r < cur)
            };
            if let Some(row) = target {
                let max = p.active_rows().saturating_sub(1);
                p.state.scroll = row.min(max);
            }
        }
    }

    pub fn peek_close(&mut self) {
        // Closing the peek returns to the normal stream base.
        self.mode.base = Base::Normal {
            focus: Focus::Stream,
        };
        self.hl.forget(PEEK_HL);
    }

    /// Request highlighting for the peeked file under its reserved slot.
    pub fn request_peek_highlight(&mut self) {
        // Only suppress while blame mode itself is still showing its empty
        // placeholder: highlighting it wastes a job and can race the committed
        // text's request through the shared slot (a late placeholder result could
        // re-satisfy the slot after drain_blame's forget, before the real request
        // goes out). Once the user has Tabbed to content/diff, its sides are
        // installed and on screen — gating on a *still-in-flight* blame worker
        // there (the worker isn't cancelled until the peek closes) would leave the
        // code unhighlighted until that irrelevant fetch settles.
        let blame_placeholder = self
            .peek()
            .is_some_and(|p| p.mode == PeekMode::Blame && p.blame_loading());
        if blame_placeholder || !self.hl.needs(PEEK_HL) {
            return;
        }
        let Some(file) = self.peek().and_then(|p| p.cs.files.first().cloned()) else {
            return;
        };
        self.hl.request(PEEK_HL, &file);
    }
}

/// The blame worker body: pin the (possibly symbolic, e.g. "HEAD") rev to one
/// commit, then read the file's committed text and per-line attribution over a
/// single repository handle — so text and attribution can't straddle a
/// concurrent ref update, and one `b` press pays for one repo open. A path
/// with no history at the rev falls back to `prev_path` (a staged rename),
/// then to `fallback_rev` (the old side, for a file the viewed commit
/// deleted). Returns `None` when `cancel` was signalled (the peek is gone);
/// a fetch that finds nothing settles as `Some(empty)`.
fn blame_fetch(
    dir: &std::path::Path,
    rev: &str,
    path: &str,
    prev_path: Option<&str>,
    fallback_rev: Option<&str>,
    cancel: &AtomicBool,
) -> Option<(String, Vec<BlameLine>)> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let Ok(repo) = gix::discover(dir) else {
        return Some((String::new(), Vec::new()));
    };
    let rev = repo
        .rev_parse_single(rev)
        .map_or_else(|_| rev.to_string(), |id| id.to_string());
    // Each read checks the cancel flag first, so an abandoned peek stops before
    // the next whole-file blame rather than running the full fallback chain.
    let read = |rev: &str, path: &str| {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        // Resolve the rev to a commit ONCE, then read the text and the
        // attribution from that id — instead of file_text_at_in and blame_file_in
        // each re-parsing the spec.
        let commit = repo
            .rev_parse_single(rev)
            .ok()?
            .object()
            .ok()?
            .peel_to_commit()
            .ok()?
            .id;
        let content = crate::git::file_text_at_commit(&repo, commit, path)?;
        let lines = crate::git::blame_commit_in(&repo, commit, path).ok()?;
        Some((content, lines))
    };
    let result = read(&rev, path)
        .or_else(|| prev_path.and_then(|p| read(&rev, p)))
        .or_else(|| fallback_rev.and_then(|r| read(r, path)));
    // A cancel that fired mid-chain leaves the peek gone; don't settle it.
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    Some(result.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::super::appcore::stub_changeset;
    use super::*;
    use crate::git::LoadRequest;
    use crate::model::{Changeset, DiffFile, FileStatus, LayoutMode, Stats};
    use crate::tui::theme::ThemeName;
    use std::path::PathBuf;

    fn crate_repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// An app with an empty changeset seeded with a specific home view kind,
    /// repo dir, and explicit old-side base — enough to exercise the git-sourced
    /// text helpers directly with hand-built files.
    fn app_kind(kind: ViewKind, repo: Option<PathBuf>, base: Option<String>) -> App {
        let cs = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            repo,
            kind,
            false,
            base,
            None,
        )
    }

    /// A not-yet-diffed stub file (forces the git-sourced branches).
    fn stub(path: &str) -> DiffFile {
        DiffFile::stub(path.into(), None, FileStatus::Modified, false, None)
    }

    /// An already-diffed file carrying cached old/new text.
    fn diffed(path: &str, old: Option<&str>, new: Option<&str>) -> DiffFile {
        DiffFile {
            path: path.into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks: Vec::new(),
            stats: Stats::default(),
            language: None,
            is_binary: false,
            old_text: old.map(Into::into),
            new_text: new.map(Into::into),
            content_digest: None,
            diffed: true,
        }
    }

    #[test]
    fn peek_new_text_covers_every_source() {
        // Diffed → cached new_text (and the None default).
        let app = app_kind(ViewKind::Local, None, None);
        assert_eq!(app.peek_new_text(&diffed("a", None, Some("hi"))), "hi");
        assert_eq!(app.peek_new_text(&diffed("a", None, None)), "");
        // Undiffed stub with no repo dir → empty.
        assert_eq!(app.peek_new_text(&stub("Cargo.toml")), "");

        let dir = crate_repo();
        // Commit view sources the new side from the commit.
        let c = app_kind(ViewKind::Commit("HEAD".into()), Some(dir.clone()), None);
        assert!(!c.peek_new_text(&stub("Cargo.toml")).is_empty());
        // Range view sources the new side from the target.
        let r = app_kind(
            ViewKind::Range {
                base: "HEAD".into(),
                target: "HEAD".into(),
            },
            Some(dir.clone()),
            None,
        );
        assert!(!r.peek_new_text(&stub("Cargo.toml")).is_empty());
        // Local view sources the new side from the worktree.
        let l = app_kind(ViewKind::Local, Some(dir), None);
        assert!(!l.peek_new_text(&stub("Cargo.toml")).is_empty());
    }

    #[test]
    fn diff_sides_cover_every_source() {
        // Diffed → cached (old, new) texts (and the None defaults).
        let app = app_kind(ViewKind::Local, None, None);
        assert_eq!(
            app.diff_sides(&diffed("a", Some("old"), Some("new"))),
            ("old".to_string(), "new".to_string())
        );
        assert_eq!(
            app.diff_sides(&diffed("a", None, None)),
            (String::new(), String::new())
        );
        // Undiffed stub with no repo dir → both empty.
        assert_eq!(
            app.diff_sides(&stub("Cargo.toml")),
            (String::new(), String::new())
        );

        let dir = crate_repo();
        // Commit view: new side from the commit, old side from its parent (`rev^`).
        let c = app_kind(ViewKind::Commit("HEAD".into()), Some(dir.clone()), None);
        let (_old, new) = c.diff_sides(&stub("Cargo.toml"));
        assert!(!new.is_empty(), "commit new side sourced from the commit");
        // Range view: new from target, old from base — both HEAD here.
        let r = app_kind(
            ViewKind::Range {
                base: "HEAD".into(),
                target: "HEAD".into(),
            },
            Some(dir.clone()),
            None,
        );
        let (old, new) = r.diff_sides(&stub("Cargo.toml"));
        assert!(
            !old.is_empty() && !new.is_empty(),
            "range sources both sides"
        );
        // Local view with an explicit base ref: old from the base, new from the
        // worktree.
        let l = app_kind(ViewKind::Local, Some(dir.clone()), Some("HEAD".into()));
        let (old, new) = l.diff_sides(&stub("Cargo.toml"));
        assert!(!old.is_empty(), "local old side from the explicit base");
        assert!(!new.is_empty(), "local new side from the worktree");
        // Local view with no base → old side defaults to HEAD.
        let l2 = app_kind(ViewKind::Local, Some(dir), None);
        let (old, _new) = l2.diff_sides(&stub("Cargo.toml"));
        assert!(!old.is_empty(), "local old side defaults to HEAD");
    }

    #[test]
    fn home_top_text_covers_every_source() {
        // No repo dir → empty.
        let app = app_kind(ViewKind::Local, None, None);
        assert_eq!(app.home_top_text("Cargo.toml"), "");

        let dir = crate_repo();
        // Commit home view → file text at the commit.
        let c = app_kind(ViewKind::Commit("HEAD".into()), Some(dir.clone()), None);
        assert!(!c.home_top_text("Cargo.toml").is_empty());
        // Range home view → file text at the target.
        let r = app_kind(
            ViewKind::Range {
                base: "HEAD".into(),
                target: "HEAD".into(),
            },
            Some(dir.clone()),
            None,
        );
        assert!(!r.home_top_text("Cargo.toml").is_empty());
        // Local home view → worktree text.
        let l = app_kind(ViewKind::Local, Some(dir), None);
        assert!(!l.home_top_text("Cargo.toml").is_empty());
    }

    #[test]
    fn peek_mut_is_some_only_when_a_peek_is_open() {
        let mut app = app_kind(ViewKind::Local, None, None);
        assert!(app.peek_mut().is_none(), "normal base has no peek");
        let peek = Peek::new(
            "a".into(),
            true,
            PeekMode::Content,
            "x".into(),
            "x".into(),
            true,
        );
        app.set_peek(peek);
        assert!(app.peek_mut().is_some(), "peek base yields a mutable peek");
    }

    #[test]
    fn open_peek_review_skips_empty_selection_and_binary() {
        // Empty changeset → nothing selectable → no peek.
        let cs = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::new(&cs);
        app.open_peek_review();
        assert!(!app.peek_open(), "nothing to peek with an empty changeset");

        // A binary file is skipped.
        let mut bin = diffed("img.png", None, None);
        bin.is_binary = true;
        let cs = Changeset {
            source: String::new(),
            files: vec![bin],
        };
        let mut app = App::new(&cs);
        app.open_peek_review();
        assert!(!app.peek_open(), "binary files are not peeked");

        // A normal diffed file opens the review peek via the cached-text path.
        let cs = Changeset {
            source: String::new(),
            files: vec![diffed("a.rs", Some("old\n"), Some("new\n"))],
        };
        let mut app = App::new(&cs);
        app.open_peek_review();
        assert!(app.peek_open(), "review peek opens on a normal file");
    }

    #[test]
    fn request_peek_highlight_enqueues_then_is_idempotent() {
        let cs = Changeset {
            source: String::new(),
            files: vec![diffed("a.rs", Some("old\n"), Some("new fn\n"))],
        };
        let mut app = App::new(&cs);
        app.open_peek_review();
        assert!(app.hl.needs(PEEK_HL), "fresh peek slot needs a highlight");
        app.request_peek_highlight();
        assert!(
            !app.hl.needs(PEEK_HL),
            "the request was enqueued under the peek slot"
        );
        // Second call short-circuits on `needs()` == false.
        app.request_peek_highlight();
        assert!(!app.hl.needs(PEEK_HL));
    }

    #[test]
    fn request_peek_highlight_without_a_peek_is_noop() {
        let cs = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::new(&cs);
        // The slot needs a highlight, but there is no peek → no request.
        assert!(app.hl.needs(PEEK_HL));
        app.request_peek_highlight();
        assert!(
            app.hl.needs(PEEK_HL),
            "no open peek means nothing is requested"
        );
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

    /// An app over the crate's own repo with one selectable real-path file, so
    /// blame (committed-rev) has a live repository to read.
    fn blame_app() -> App {
        let cs = Changeset {
            source: String::new(),
            files: vec![DiffFile::stub(
                "Cargo.toml".into(),
                None,
                FileStatus::Modified,
                false,
                None,
            )],
        };
        App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(crate_repo()),
            ViewKind::Local,
            false,
            None,
            None,
        )
    }

    use crate::tui::testutil::drive_blame;

    #[test]
    fn no_highlight_requested_while_blame_loads() {
        // The blame peek opens over an empty placeholder; requesting a highlight
        // for it wastes a job and can race the committed text's request. The
        // request is skipped until blame settles, then goes out for real text.
        let mut app = blame_app();
        app.state_mut().selected = 0;
        app.open_peek_blame();
        assert!(app.peek_blame_loading(), "blame is in flight");
        app.request_peek_highlight();
        assert!(
            app.hl.needs(PEEK_HL),
            "no highlight requested for the placeholder while loading"
        );
        drive_blame(&mut app);
        assert!(!app.peek_blame_loading(), "blame settled");
        app.request_peek_highlight();
        assert!(
            !app.hl.needs(PEEK_HL),
            "the committed text is requested once blame settles"
        );
    }

    #[test]
    fn open_peek_blame_loads_committed_attribution() {
        let mut app = blame_app();
        app.state_mut().selected = 0;
        app.open_peek_blame();
        assert!(app.peek_open(), "blame peek opened");
        assert_eq!(app.peek().unwrap().mode, PeekMode::Blame);
        drive_blame(&mut app);
        let p = app.peek().unwrap();
        assert!(!p.blame().is_empty(), "blame attribution loaded");
        assert!(p.label().contains("blame"), "header is the blame label");
        // The cursor line's commit identity is reachable for the header.
        assert!(p.blame_cursor().is_some(), "cursor line has a blame entry");
    }

    #[test]
    fn open_peek_blame_inert_on_a_placeholder() {
        let mut app = blame_app();
        app.state_mut().select_dir("x".into(), 0); // cursor on a folded placeholder
        app.open_peek_blame();
        assert!(
            !app.peek_open(),
            "blame is inert on a collapsed placeholder"
        );
    }

    #[test]
    fn blame_peek_has_content_in_the_other_modes() {
        // Opening blame via `b` must still populate the content/diff sides, so
        // cycling Tab to content shows the file rather than "No content."
        let mut app = blame_app();
        app.state_mut().selected = 0;
        app.open_peek_blame();
        drive_blame(&mut app); // committed content arrives with the blame
        assert!(!app.peek().unwrap().is_empty(), "blame mode has content");
        app.peek_toggle_mode(); // blame → content
        assert_eq!(app.peek().unwrap().mode, PeekMode::Content);
        assert!(
            !app.peek().unwrap().is_empty(),
            "content mode has content after opening via blame"
        );
    }

    #[test]
    fn peek_toggle_cycles_content_diff_blame() {
        let mut app = blame_app();
        app.state_mut().selected = 0;
        app.open_peek_preview();
        assert_eq!(app.peek().unwrap().mode, PeekMode::Content);
        app.peek_toggle_mode();
        assert_eq!(app.peek().unwrap().mode, PeekMode::Diff);
        app.peek_toggle_mode();
        assert_eq!(
            app.peek().unwrap().mode,
            PeekMode::Blame,
            "Tab cycles to blame"
        );
        app.peek_toggle_mode();
        assert_eq!(
            app.peek().unwrap().mode,
            PeekMode::Content,
            "and wraps back"
        );
    }

    #[test]
    fn peek_blame_enter_opens_the_commit_message() {
        let mut app = blame_app();
        app.state_mut().selected = 0;
        app.open_peek_blame();
        drive_blame(&mut app);
        app.peek_blame_open_message();
        assert!(
            app.commit_msg_open(),
            "Enter on a blame line opens the commit-message popup"
        );
    }

    #[test]
    fn peek_blame_open_message_is_noop_without_blame() {
        // A content peek has no blame cursor → opening a message is a no-op.
        let cs = Changeset {
            source: String::new(),
            files: vec![diffed("a.rs", Some("old\n"), Some("new\n"))],
        };
        let mut app = App::new(&cs);
        app.open_peek_preview();
        app.peek_blame_open_message();
        assert!(!app.commit_msg_open(), "no blame cursor → nothing opens");
    }

    #[test]
    fn drain_blame_without_a_peek_is_false() {
        let mut app = blame_app();
        assert!(!app.drain_blame(), "no peek → nothing to drain");
    }

    #[test]
    fn drain_blame_settles_when_the_worker_died_without_sending() {
        let mut app = blame_app();
        app.state_mut().selected = 0;
        app.open_peek_blame();
        // Replace the live channel with one whose sender is already gone — the
        // shape a panicked worker leaves behind.
        let (tx, rx) = mpsc::channel();
        drop(tx);
        app.peek_mut().unwrap().blame_rx = Some(rx);
        assert!(app.drain_blame(), "a dead channel settles (and redraws)");
        assert!(
            !app.peek_blame_loading(),
            "loading cleared — the 16ms poll cadence can't stick"
        );
        assert!(
            !app.peek().unwrap().needs_blame(),
            "settled as attempted, not respawned"
        );
        // With the receiver cleared, further drains are quiet.
        assert!(!app.drain_blame(), "nothing left to drain");
    }

    #[test]
    fn blame_settles_empty_without_respawning_for_a_file_missing_at_head() {
        // A file with no blame at the rev (untracked): the fetch settles on an
        // empty result instead of reading as "never started", so re-entering
        // blame mode doesn't re-pay a git read + doomed worker every time.
        let cs = Changeset {
            source: String::new(),
            files: vec![DiffFile::stub(
                "no_such_file_in_repo.xyz".into(),
                None,
                FileStatus::Untracked,
                false,
                None,
            )],
        };
        let mut app = App::with_launch(
            &cs,
            LayoutMode::Stack,
            ThemeName::Dark,
            Some(crate_repo()),
            ViewKind::Local,
            false,
            None,
            None,
        );
        app.state_mut().selected = 0;
        app.open_peek_blame();
        drive_blame(&mut app);
        assert!(
            app.peek().unwrap().blame().is_empty(),
            "no blame for an untracked file"
        );
        assert!(!app.peek().unwrap().needs_blame(), "the fetch settled");
        // Cycle out of blame and back in: no worker respawns.
        app.peek_toggle_mode(); // blame → content
        app.peek_toggle_mode(); // content → diff
        app.peek_toggle_mode(); // diff → blame
        assert!(!app.peek_blame_loading(), "no doomed respawn on re-entry");
        assert!(!app.peek().unwrap().needs_blame());
    }

    #[test]
    fn fill_peek_sides_is_inert_when_the_file_left_the_view() {
        // A blame peek whose path has no source file in the current view (e.g.
        // the view switched beneath it): the lazy fill finds nothing and the
        // sides stay unfilled instead of installing garbage.
        let cs = Changeset {
            source: String::new(),
            files: Vec::new(),
        };
        let mut app = App::new(&cs);
        app.set_peek(Peek::new_blame("ghost.rs".into(), true));
        app.peek_toggle_mode(); // → content; no matching file → nothing to fill
        assert!(
            app.peek().unwrap().sides_unfilled(),
            "no source file → still unfilled"
        );
    }

    #[test]
    fn split_toggle_is_inert_outside_diff_mode() {
        let mut app = blame_app();
        app.state_mut().selected = 0;
        app.open_peek_blame();
        app.peek_toggle_split();
        assert!(
            !app.peek().unwrap().split_view,
            "m must not silently arm a layout change in blame mode"
        );
        app.peek_close();
        app.open_peek_review(); // diff mode
        app.peek_toggle_split();
        assert!(app.peek().unwrap().split_view, "m toggles in diff mode");
    }

    #[test]
    fn enter_outside_blame_mode_does_not_open_a_message() {
        // Content/diff plans index a different text than the blame vector, so
        // Enter must be inert there even after blame has loaded.
        let mut app = blame_app();
        app.state_mut().selected = 0;
        app.open_peek_blame();
        drive_blame(&mut app);
        assert!(
            app.peek().unwrap().blame_cursor().is_some(),
            "blame is loaded"
        );
        app.peek_toggle_mode(); // blame → content, blame vector persists
        app.peek_blame_open_message();
        assert!(
            !app.commit_msg_open(),
            "Enter is inert outside blame mode even with blame loaded"
        );
    }

    /// A throwaway repo committing `old.rs`, then running `extra` git commands.
    fn git_fixture(extra: &[&[&str]]) -> tempfile::TempDir {
        use crate::testutil::run_git;
        let dir = crate::testutil::scratch_repo();
        std::fs::write(dir.path().join("old.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        run_git(dir.path(), &["add", "-A"]);
        run_git(dir.path(), &["commit", "-qm", "add old.rs"]);
        for c in extra {
            run_git(dir.path(), c);
        }
        dir
    }

    #[test]
    fn blame_fetch_falls_back_to_the_previous_path_for_a_staged_rename() {
        let repo = git_fixture(&[&["mv", "old.rs", "new.rs"]]);
        let cancel = AtomicBool::new(false);
        // new.rs has no history at HEAD; the previous path does.
        let (text, lines) =
            blame_fetch(repo.path(), "HEAD", "new.rs", Some("old.rs"), None, &cancel).unwrap();
        assert!(!lines.is_empty(), "history found under the previous path");
        assert!(text.contains("fn a()"), "committed content loaded");
    }

    #[test]
    fn blame_fetch_falls_back_to_the_old_side_for_a_deleted_file() {
        let repo = git_fixture(&[&["rm", "-q", "old.rs"], &["commit", "-qm", "delete old.rs"]]);
        let cancel = AtomicBool::new(false);
        // The deleting commit's tree has no old.rs; its parent does.
        let (text, lines) =
            blame_fetch(repo.path(), "HEAD", "old.rs", None, Some("HEAD^"), &cancel).unwrap();
        assert!(!lines.is_empty(), "the old side still has the file");
        assert!(text.contains("fn a()"), "old-side content loaded");
    }

    #[test]
    fn blame_fetch_honors_cancellation_and_settles_outside_a_repo() {
        // A pre-cancelled fetch does no git work at all, even with fallbacks
        // present — each read short-circuits on the cancel flag rather than
        // running the full prev-path/old-side chain for a peek that is gone.
        let cancelled = AtomicBool::new(true);
        assert!(
            blame_fetch(
                &crate_repo(),
                "HEAD",
                "no_such_file.xyz",
                Some("also_missing.xyz"),
                Some("HEAD^"),
                &cancelled,
            )
            .is_none(),
            "cancelled → no result, no fallback work"
        );
        // Not a repository → settles empty instead of erroring or retrying.
        let cancel = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let (text, lines) = blame_fetch(dir.path(), "HEAD", "x.rs", None, None, &cancel).unwrap();
        assert!(text.is_empty() && lines.is_empty(), "settled empty");
    }

    #[test]
    fn blame_revs_resolve_per_view_kind() {
        let dir = crate_repo();
        // (blame rev, old-side fallback rev) for each view kind.
        assert_eq!(
            app_kind(ViewKind::Local, Some(dir.clone()), None).blame_revs(),
            ("HEAD".to_string(), "HEAD".to_string())
        );
        assert_eq!(
            app_kind(ViewKind::Staged, Some(dir.clone()), None).blame_revs(),
            ("HEAD".to_string(), "HEAD".to_string())
        );
        assert_eq!(
            app_kind(ViewKind::Commit("abc123".into()), Some(dir.clone()), None).blame_revs(),
            ("abc123".to_string(), "abc123^".to_string())
        );
        assert_eq!(
            app_kind(
                ViewKind::Range {
                    base: "x".into(),
                    target: "y".into()
                },
                Some(dir.clone()),
                None
            )
            .blame_revs(),
            ("y".to_string(), "x".to_string())
        );
        // A local view launched with `--from <base>`: blame still reads HEAD, but
        // the deleted-file fallback now honors the explicit base (matching the
        // diff old side) where it previously hardcoded HEAD.
        assert_eq!(
            app_kind(ViewKind::Local, Some(dir), Some("origin/main".into())).blame_revs(),
            ("HEAD".to_string(), "origin/main".to_string())
        );
    }

    #[test]
    fn peek_blame_confirm_switches_and_closes_the_peek() {
        let mut app = blame_app();
        app.state_mut().selected = 0;
        app.open_peek_blame();
        drive_blame(&mut app);
        app.peek_blame_open_message();
        assert!(
            app.commit_msg_open(),
            "the popup is open over the blame peek"
        );
        app.commit_msg_confirm();
        assert!(!app.commit_msg_open(), "confirm closes the popup");
        assert!(!app.peek_open(), "confirm closes the blame peek");
        assert_eq!(app.session.cursor, 1, "and switches to the commit view");
    }

    #[test]
    fn peek_sources_a_stub_file_from_git() {
        // The peek must work on a not-yet-diffed file by loading its content
        // directly from git.
        let (mut app, _stubs) = stub_app("HEAD");
        if app.cs().files.is_empty() {
            return;
        }
        // Land on the first non-binary stub.
        let Some(i) = app.cs().files.iter().position(|f| !f.is_binary) else {
            return;
        };
        app.state_mut().selected = i;
        // Content preview.
        app.open_peek_preview();
        assert!(app.peek_open(), "preview opened on a stub");
        assert!(
            !app.peek().unwrap().is_empty(),
            "stub content loaded from git"
        );
        app.peek_close();
        // On-demand single-file diff.
        app.open_peek_review();
        assert!(app.peek_open(), "diff opened on a stub");
    }
}
