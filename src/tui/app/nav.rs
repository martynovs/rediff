//! Stream/sidebar navigation: layout toggles, directory folding, scrolling,
//! file/hunk stepping, the sidebar window, focus, and viewed-tracking.

use crate::model::LayoutMode;
use crate::tui::review;
use crate::tui::sidebar;
use crate::tui::stream;

use super::types::{App, Base, Focus};

impl App {
    /// The sidebar's rows for the current files under the active grouping. Cheap
    /// to rebuild (one pass over the file list); derived, not stored.
    pub fn sidebar_rows(&self) -> Vec<sidebar::SidebarRow> {
        sidebar::rows(&self.cs().files, self.grouping, &self.state().collapsed)
    }

    // ---- active-plan accessors (stack vs split share the navigation model) --

    pub(crate) fn file_starts(&self) -> &[usize] {
        &self.plan().file_starts
    }

    /// Row of the file at `Changeset::files` index `idx`, if it is visible (not in
    /// a folded directory). `file_starts` is indexed by visible ordinal, so map
    /// the cs index through `visible_files` first.
    fn file_start(&self, idx: usize) -> Option<usize> {
        let p = self.plan();
        p.visible_ordinal(idx)
            .and_then(|o| p.file_starts.get(o).copied())
    }

    /// Apply the configured layout, rebuilding the plan and re-anchoring the
    /// viewport to the current file when it changed (only happens when `m`
    /// toggles the mode — the row count/order differs between layouts).
    pub fn set_layout(&mut self, _stream_width: u16) {
        let want_split = matches!(self.layout, LayoutMode::Split);
        if want_split != self.is_split() {
            let anchor = self.current_file();
            let old_start = self.file_start(anchor).unwrap_or(0);
            // A viewport parked in the banner region (above the first file) stays
            // there; otherwise snap to the anchor file's header in the new layout.
            let in_banner = self.state().scroll < old_start;
            self.build_plan();
            if !in_banner {
                if let Some(row) = self.file_start(anchor) {
                    self.state_mut().scroll = row;
                }
            }
            self.clamp();
        }
    }

    /// Toggle between the two layouts (split ↔ stack).
    pub fn cycle_mode(&mut self) {
        self.layout = match self.layout {
            LayoutMode::Split => LayoutMode::Stack,
            LayoutMode::Stack => LayoutMode::Split,
        };
    }

    /// Toggle the sidebar between the flat list and the directory-grouped view,
    /// keeping the selected file revealed in the new row layout. Folds apply only
    /// in the grouped view, so the body plan is rebuilt to honor (grouped) or
    /// ignore (flat) the collapsed set; a placeholder selection converts to its
    /// directory's first file on the way to flat (which has no placeholders).
    pub fn toggle_grouping(&mut self) {
        self.grouping = self.grouping.toggled();
        if !self.grouped() {
            if let Some(dir) = self.state().selected_dir.clone() {
                match self
                    .cs()
                    .files
                    .iter()
                    .position(|f| crate::model::parent_dir(&f.path) == dir)
                {
                    Some(i) => self.state_mut().select_file(i),
                    None => self.state_mut().selected_dir = None,
                }
            }
        }
        self.rebuild_plan();
        self.state_mut().reveal_selected = true;
    }

    // ---- directory folding -------------------------------------------------

    /// Toggle the fold of the cursor's context (`z`): on a file, fold its
    /// directory and land on the new placeholder; on a placeholder, unfold and
    /// land on the directory's first file. Inert in the flat view (no directories).
    pub fn toggle_fold(&mut self) {
        if !self.grouped() {
            return;
        }
        if let Some(dir) = self.state().selected_dir.clone() {
            self.unfold_dir(&dir);
        } else {
            // `.get`, not indexing: with an empty changeset `selected` is 0 and
            // `selected_file()` still answers `Some(0)`, so `rediff` on a clean
            // tree panicked on `z`.
            let Some(f) = self.cs().files.get(self.state().selected) else {
                return;
            };
            let dir = crate::model::parent_dir(&f.path).to_string();
            self.fold_dir(&dir);
        }
    }

    /// Toggle a specific directory's fold (mouse click on its header/placeholder).
    pub(crate) fn toggle_fold_dir(&mut self, dir: &str) {
        if !self.grouped() {
            return;
        }
        if self.state().collapsed.contains(dir) {
            self.unfold_dir(dir);
        } else {
            self.fold_dir(dir);
        }
    }

    /// Fold `dir`: hide its files from both panes and land the cursor on the new
    /// placeholder (so `z` again undoes it).
    fn fold_dir(&mut self, dir: &str) {
        let near = self.state().selected;
        self.state_mut().collapsed.insert(dir.to_string());
        self.build_plan();
        self.state_mut().select_dir(dir.to_string(), near);
        self.state_mut().reveal_selected = true;
        self.sync_body_to_selection();
        self.clamp();
    }

    /// Unfold `dir`: restore its files and land the cursor on its first file,
    /// keeping focus where it is (fold/unfold is file-list navigation).
    fn unfold_dir(&mut self, dir: &str) {
        self.state_mut().collapsed.remove(dir);
        self.build_plan();
        if let Some(i) = self
            .cs()
            .files
            .iter()
            .position(|f| crate::model::parent_dir(&f.path) == dir)
        {
            self.reveal_file(i);
        }
        self.clamp();
    }

    /// Collapse-all / expand-all (`Z`): collapse every directory when any is
    /// currently expanded, else expand them all. Inert in the flat view.
    pub fn fold_all(&mut self) {
        if !self.grouped() {
            return;
        }
        let all_dirs: std::collections::BTreeSet<String> = self
            .cs()
            .files
            .iter()
            .map(|f| crate::model::parent_dir(&f.path).to_string())
            .collect();
        let any_expanded = all_dirs.iter().any(|d| !self.state().collapsed.contains(d));
        if any_expanded {
            // Collapse all; land on the placeholder for the current file's directory.
            #[expect(
                clippy::indexing_slicing,
                reason = "the unwrap_or_else arm runs only when the cursor is on a file, so `selected` is a valid file index"
            )]
            let dir = self.state().selected_dir.clone().unwrap_or_else(|| {
                crate::model::parent_dir(&self.cs().files[self.state().selected].path).to_string()
            });
            let near = self.state().selected;
            self.state_mut().collapsed = all_dirs;
            self.build_plan();
            self.state_mut().select_dir(dir, near);
            self.state_mut().reveal_selected = true;
            self.sync_body_to_selection();
        } else {
            // Expand all; land on the current file (focus unchanged).
            self.state_mut().collapsed.clear();
            self.build_plan();
            let i = self
                .state()
                .selected
                .min(self.cs().files.len().saturating_sub(1));
            self.reveal_file(i);
        }
        self.clamp();
    }

    /// Move the line cursor by `delta` (`j`/`k`, the arrows), viewport following.
    pub fn move_cursor(&mut self, delta: isize) {
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = stream::usable(&e.plan, vh);
        stream::move_cursor_by(&mut e.state, &e.plan, usable, delta);
    }

    /// Scroll the viewport by `delta` (`J`/`K`, Shift+arrows, the mouse wheel),
    /// carrying the cursor so it keeps its row on screen.
    pub fn scroll_view(&mut self, delta: isize) {
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = stream::usable(&e.plan, vh);
        stream::scroll_view_by(&mut e.state, &e.plan, usable, delta);
    }

    pub fn page(&mut self, dir: isize) {
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = stream::usable(&e.plan, vh);
        stream::page(&mut e.state, &e.plan, usable, dir);
    }

    pub fn half_page(&mut self, dir: isize) {
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = stream::usable(&e.plan, vh);
        stream::half_page(&mut e.state, &e.plan, usable, dir);
    }

    pub fn top(&mut self) {
        let e = self.session.cur_mut();
        stream::top(&mut e.state, &e.plan);
    }

    pub fn bottom(&mut self) {
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = stream::usable(&e.plan, vh);
        stream::bottom(&mut e.state, &e.plan, usable);
    }

    /// Index of the file currently at the top of the viewport.
    pub fn current_file(&self) -> usize {
        stream::current_file(self.state(), self.plan())
    }

    pub fn next_hunk(&mut self) {
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = stream::usable(&e.plan, vh);
        stream::next_hunk(&mut e.state, &e.plan, usable);
    }

    pub fn prev_hunk(&mut self) {
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = stream::usable(&e.plan, vh);
        stream::prev_hunk(&mut e.state, &e.plan, usable);
    }

    /// Step the cursor through the navigable sequence (visible files + collapsed
    /// placeholders) by `delta`, syncing the diff body and — when `focus_stream` —
    /// dropping focus into the stream. The one model shared by `{`/`}`, Space,
    /// Ctrl+arrows (focus stream) and the sidebar's `j`/`k` (focus stays).
    fn step_selection(&mut self, delta: isize, focus_stream: bool) {
        let rows = self.sidebar_rows();
        let seq = sidebar::nav_sequence(&rows);
        if sidebar::step(self.state_mut(), &seq, delta).is_some() {
            self.sync_body_to_selection();
            if focus_stream {
                self.set_focus(Focus::Stream);
            }
        }
    }

    /// Scroll the diff body to the current selection — a file's header, or a
    /// folded directory's placeholder row.
    fn sync_body_to_selection(&mut self) {
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = stream::usable(&e.plan, vh);
        let sel = e.state.selected;
        match e.state.selected_dir.clone() {
            Some(dir) => stream::jump_to_collapsed(&mut e.state, &e.plan, usable, &dir),
            None => stream::jump_to_file(&mut e.state, &e.plan, usable, sel),
        }
    }

    /// Move to the next nav stop (file or placeholder) and select it.
    pub fn next_file(&mut self) {
        self.step_selection(1, true);
    }

    pub fn prev_file(&mut self) {
        self.step_selection(-1, true);
    }

    /// Jump to a specific file by index (sidebar selection / fuzzy jump).
    pub fn jump_to_file(&mut self, idx: usize) {
        let vh = self.viewport_h;
        let e = self.session.cur_mut();
        let usable = stream::usable(&e.plan, vh);
        stream::jump_to_file(&mut e.state, &e.plan, usable, idx);
    }

    /// Go to a file by index: jump the stream, select it, and focus the stream.
    pub fn goto_file(&mut self, idx: usize) {
        if idx < self.cs().files.len() {
            self.reveal_file(idx);
            self.set_focus(Focus::Stream);
        }
    }

    /// Select a file and reveal it in both panes, leaving the focused pane as-is
    /// (folding/unfolding is file-list navigation — it must not yank focus into
    /// the diff).
    fn reveal_file(&mut self, idx: usize) {
        if idx < self.cs().files.len() {
            self.state_mut().select_file(idx);
            self.state_mut().reveal_selected = true;
            self.jump_to_file(idx);
        }
    }

    /// Land on a file *by path*, unfolding its directory first if it is folded —
    /// so a jump (fuzzy palette, click, commit `land_path`) always reaches its
    /// target even when step-navigation would skip it. No-op if the path is gone.
    pub fn land_on_file(&mut self, idx: usize) {
        if idx >= self.cs().files.len() {
            return;
        }
        #[expect(
            clippy::indexing_slicing,
            reason = "idx is bounds-checked against files.len() just above"
        )]
        let dir = crate::model::parent_dir(&self.cs().files[idx].path).to_string();
        if self.state().collapsed.contains(&dir) {
            self.state_mut().collapsed.remove(&dir);
            self.build_plan();
        }
        self.goto_file(idx);
    }

    /// Jump using a sparse digit (1–9) spread across the visible sidebar files:
    /// 1 is the first visible file, 9 the last.
    pub fn goto_visible_digit(&mut self, d: usize) {
        let rows = self.sidebar_rows();
        if let Some(idx) = sidebar::digit_target(d, self.sidebar_top, self.sidebar_visible, &rows) {
            self.goto_file(idx);
        }
    }

    /// Recompute the sidebar window for a viewport of `height` rows. Reveals
    /// `selected` only when `reveal_selected` is set, so a manual sidebar scroll
    /// (which leaves the selection put) is preserved.
    pub fn update_sidebar_window(&mut self, height: usize) {
        let rows = self.sidebar_rows();
        let top = self.sidebar_top;
        let height = height.max(1);
        self.sidebar_height = height;
        let (new_top, visible) = sidebar::window(self.state_mut(), top, height, &rows);
        self.sidebar_top = new_top;
        self.sidebar_visible = visible;
    }

    /// Scroll the sidebar list (rows) without changing the selected file.
    pub fn sidebar_scroll(&mut self, delta: isize) {
        let rows = self.sidebar_rows();
        self.sidebar_top =
            sidebar::scroll(self.sidebar_top, self.sidebar_height, rows.len(), delta);
    }

    // ---- focus -------------------------------------------------------------

    pub fn toggle_focus(&mut self) {
        // `selected` already tracks the active file (top-of-viewport while
        // scrolling, or the last jump target), so entering the sidebar keeps it.
        if let Base::Normal { focus } = &mut self.mode.base {
            *focus = match focus {
                Focus::Stream => Focus::Sidebar,
                Focus::Sidebar => Focus::Stream,
            };
        }
    }

    // ---- mode accessors ----------------------------------------------------

    /// The current keyboard focus (Stream while peeking — focus only applies to
    /// the normal base).
    pub fn focus(&self) -> Focus {
        match self.mode.base {
            Base::Normal { focus } => focus,
            Base::Peek(_) => Focus::Stream,
        }
    }

    pub(crate) fn set_focus(&mut self, f: Focus) {
        if let Base::Normal { focus } = &mut self.mode.base {
            *focus = f;
        }
    }

    /// Whether the sidebar is rendered: always when not in hide-mode, and
    /// temporarily while it is focused (so `Tab` reveals a hidden sidebar, and
    /// `Tab` back to the diff hides it again).
    pub fn sidebar_shown(&self) -> bool {
        !self.sidebar_hidden || self.focus() == Focus::Sidebar
    }

    pub fn focus_stream(&mut self) {
        self.set_focus(Focus::Stream);
    }

    /// Show/hide the sidebar. Hiding it also moves focus to the diff.
    pub fn toggle_sidebar(&mut self) {
        self.sidebar_hidden = !self.sidebar_hidden;
        if self.sidebar_hidden {
            self.set_focus(Focus::Stream);
        }
    }

    pub fn sidebar_move(&mut self, delta: isize) {
        self.step_selection(delta, false);
    }

    // ---- viewed tracking ---------------------------------------------------

    /// Whether the sidebar is grouped by directory (collapse only applies here).
    pub(crate) fn grouped(&self) -> bool {
        self.grouping == sidebar::Grouping::ByDir
    }

    fn build_plan(&mut self) {
        self.session.build_plan(self.layout, self.grouped());
    }

    /// Rebuild the row plan from the current viewed state, keeping the viewport
    /// anchored to the same position *within* the current file (the session owns
    /// the anchoring logic — see [`crate::tui::session::Session::rebuild_plan`]).
    fn rebuild_plan(&mut self) {
        self.session.rebuild_plan(
            self.layout,
            self.grouped(),
            self.viewport_h,
            self.viewport_w,
        );
    }

    pub fn toggle_viewed(&mut self) {
        if !self.is_review() {
            return;
        }
        // Inert when the cursor is on a collapsed placeholder (no file to toggle).
        let Some(idx) = self.state().selected_file() else {
            return;
        };
        review::toggle(self.state_mut(), idx);
        // Auto-collapse: when this toggle completes the file's directory (its last
        // unreviewed file just became reviewed), fold it once — only in a grouped
        // review, and only on the completion edge (re-expanding by hand sticks).
        // Same empty-changeset hazard as `toggle_fold`: `selected_file()` answers
        // `Some(0)` even with no files.
        let Some(f) = self.cs().files.get(idx) else {
            return;
        };
        let dir = crate::model::parent_dir(&f.path).to_string();
        let auto = self.grouped()
            && self.state().viewed.get(idx).copied().unwrap_or(false)
            && !self.state().collapsed.contains(&dir)
            && self.dir_fully_reviewed(&dir);
        if auto {
            self.state_mut().collapsed.insert(dir.clone());
        }
        self.rebuild_plan();
        if auto {
            // The fold hid the just-reviewed file; keep the review flowing by
            // advancing to the next unviewed file, else park on the new placeholder.
            if !self.next_unviewed() {
                let near = self.state().selected;
                self.state_mut().select_dir(dir, near);
                self.state_mut().reveal_selected = true;
                self.sync_body_to_selection();
            }
        }
    }

    /// Whether every file in `dir` is reviewed.
    fn dir_fully_reviewed(&self, dir: &str) -> bool {
        self.cs()
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| crate::model::parent_dir(&f.path) == dir)
            .all(|(i, _)| self.state().viewed.get(i).copied().unwrap_or(false))
    }

    /// Per-file mask: true where the file's directory is folded (out of scope).
    fn hidden_mask(&self) -> Vec<bool> {
        let collapsed = &self.state().collapsed;
        self.cs()
            .files
            .iter()
            .map(|f| self.grouped() && collapsed.contains(crate::model::parent_dir(&f.path)))
            .collect()
    }

    /// Jump to the next unreviewed *visible* file after the current one (wrapping),
    /// skipping files in folded directories. Returns false when nothing visible is
    /// unreviewed; in that case it surfaces how many unreviewed files are hidden in
    /// folded directories (so the remainder reads as folded away, not lost).
    pub fn next_unviewed(&mut self) -> bool {
        if !self.is_review() {
            return false;
        }
        let (start, n) = (self.session.active_file(), self.cs().files.len());
        let hidden = self.hidden_mask();
        if let Some(idx) = review::next_unviewed_visible(self.state(), start, n, &hidden) {
            self.flash = None;
            self.goto_file(idx);
            true
        } else {
            let hidden_unviewed = (0..n)
                .filter(|&i| {
                    hidden.get(i).copied().unwrap_or(false)
                        && !self.state().viewed.get(i).copied().unwrap_or(false)
                })
                .count();
            self.flash = Some(if hidden_unviewed > 0 {
                format!("none in view · {hidden_unviewed} hidden in folded dirs")
            } else {
                "all reviewed".to_string()
            });
            false
        }
    }

    /// Count of reviewed files in the current view.
    pub fn viewed_count(&self) -> usize {
        review::count(self.state())
    }
}

#[cfg(test)]
mod nav_tests;
