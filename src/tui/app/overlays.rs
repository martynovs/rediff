//! Transient overlays layered over a base: the live-preview theme picker, the
//! fuzzy file/commit palette, the help screen, and the sidebar mouse click that
//! focuses or folds.

use crate::model::{Changeset, CommitInfo};
use crate::tui::fuzzy;
use crate::tui::sidebar;
use crate::tui::theme::{self, ThemeName};
use crate::tui::view::ViewKind;

use super::types::{
    App, CommitMsg, Focus, InputContext, Overlay, Palette, PaletteKind, ThemePicker, THEME_CELL_W,
};

impl App {
    // ---- theme picker overlay ----------------------------------------------

    /// Open the live-preview theme picker, snapshotting the current theme so a
    /// cancel can roll back.
    pub fn open_theme_picker(&mut self) {
        let original = self.theme.name;
        self.mode.push_overlay(Overlay::ThemePicker(ThemePicker {
            selected: original.position_in_tab(),
            dark_tab: original.is_dark(),
            original,
        }));
    }

    pub fn theme_picker(&self) -> Option<&ThemePicker> {
        match self.mode.overlay() {
            Some(Overlay::ThemePicker(p)) => Some(p),
            _ => None,
        }
    }

    pub fn theme_picker_open(&self) -> bool {
        self.theme_picker().is_some()
    }

    /// Number of themes in the active tab.
    pub fn theme_picker_count(&self) -> usize {
        self.theme_picker()
            .map_or(0, |p| theme::themes_by_brightness(p.dark_tab).len())
    }

    /// Grid columns for the picker, derived from the body width so navigation and
    /// rendering agree without threading geometry through picker state.
    pub fn theme_picker_cols(&self) -> usize {
        let w = (self.viewport_w * 9 / 10).max(THEME_CELL_W);
        (w / THEME_CELL_W).clamp(1, self.theme_picker_count().max(1))
    }

    /// Number of grid rows for the picker (a column's height). The grid is
    /// column-major, so a column holds `rows` consecutive themes.
    pub fn theme_picker_rows(&self) -> usize {
        self.theme_picker_count().div_ceil(self.theme_picker_cols())
    }

    /// Move the picker cursor and live-preview the theme now under it. The grid is
    /// column-major: vertical steps (`dy`) walk within a column and flow into the
    /// top of the next column at the bottom; horizontal steps (`dx`) jump a whole
    /// column height. Clamped to the active tab's bounds.
    #[expect(
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "theme counts/indices are tiny (well under isize::MAX) and the result is clamped to [0, count-1], so it is non-negative"
    )]
    pub fn theme_picker_move(&mut self, dx: isize, dy: isize) {
        let rows = self.theme_picker_rows() as isize;
        let count = self.theme_picker_count() as isize;
        if count == 0 {
            return;
        }
        let Some(p) = self.theme_picker() else { return };
        let next = (p.selected as isize + dy + dx * rows).clamp(0, count - 1) as usize;
        self.set_picker_selection(next);
    }

    /// Advance to the next theme within the tab (wrapping), live-previewing it.
    /// Bound to `t` so repeatedly tapping the open key cycles through the tab.
    pub fn theme_picker_next(&mut self) {
        let count = self.theme_picker_count();
        if count == 0 {
            return;
        }
        let Some(p) = self.theme_picker() else { return };
        self.set_picker_selection((p.selected + 1) % count);
    }

    /// Switch between the dark and light tabs, previewing the theme now under the
    /// cursor (clamped into the new tab's bounds).
    pub fn theme_picker_toggle_tab(&mut self) {
        let new_dark = match self.theme_picker() {
            Some(p) => !p.dark_tab,
            None => return,
        };
        let len = theme::themes_by_brightness(new_dark).len();
        if len == 0 {
            return;
        }
        if let Some(Overlay::ThemePicker(p)) = self.mode.overlay_mut() {
            p.dark_tab = new_dark;
            p.selected = p.selected.min(len - 1);
        }
        let idx = self.theme_picker().map_or(0, |p| p.selected);
        self.set_picker_selection(idx);
    }

    fn set_picker_selection(&mut self, idx: usize) {
        let Some(p) = self.theme_picker() else { return };
        let list = theme::themes_by_brightness(p.dark_tab);
        if list.is_empty() {
            return;
        }
        let idx = idx.min(list.len() - 1);
        #[expect(
            clippy::indexing_slicing,
            reason = "idx is clamped to list.len()-1 and list is non-empty (checked above)"
        )]
        let name = list[idx];
        if let Some(Overlay::ThemePicker(p)) = self.mode.overlay_mut() {
            p.selected = idx;
        }
        self.apply_theme(name);
    }

    /// Commit the highlighted theme: close the picker, keep the theme. Returns
    /// the committed theme so the caller can persist it.
    pub fn theme_picker_commit(&mut self) -> Option<ThemeName> {
        if let Some(Overlay::ThemePicker(_)) = self.mode.pop_overlay() {
            Some(self.theme.name)
        } else {
            None
        }
    }

    /// Cancel the picker: restore the theme active when it opened.
    pub fn theme_picker_cancel(&mut self) {
        if let Some(Overlay::ThemePicker(p)) = self.mode.pop_overlay() {
            self.apply_theme(p.original);
        }
    }

    /// The clicked screen row is offset by the sidebar's scroll window
    /// (`sidebar_top`), so a scrolled list maps to the right file. A click on a
    /// directory line selects nothing.
    pub fn click(&mut self, x: u16, y: u16) -> bool {
        let rows = self.sidebar_rows();
        match sidebar::row_at(
            self.sidebar_area,
            self.sidebar_top,
            self.sidebar_visible,
            x,
            y,
            &rows,
        ) {
            Some(sidebar::RowHit::File(idx)) => {
                self.set_focus(Focus::Sidebar);
                self.state_mut().select_file(idx);
                self.state_mut().reveal_selected = true;
                self.jump_to_file(idx);
                true
            }
            // A click on a directory header or a folded placeholder toggles its fold.
            Some(sidebar::RowHit::Dir(dir)) => {
                self.set_focus(Focus::Sidebar);
                self.toggle_fold_dir(&dir);
                true
            }
            None => false,
        }
    }

    // ---- palette accessors -------------------------------------------------

    /// The open palette overlay, if any.
    pub fn palette(&self) -> Option<&Palette> {
        match self.mode.overlay() {
            Some(Overlay::Palette(p)) => Some(p),
            _ => None,
        }
    }

    fn palette_mut(&mut self) -> Option<&mut Palette> {
        match self.mode.overlay_mut() {
            Some(Overlay::Palette(p)) => Some(p),
            _ => None,
        }
    }

    /// Whether a fuzzy palette overlay is open.
    pub fn palette_open(&self) -> bool {
        self.palette().is_some()
    }

    /// Whether the help overlay is open.
    pub fn help_open(&self) -> bool {
        matches!(self.mode.overlay(), Some(Overlay::Help))
    }

    /// Pop the topmost overlay when it is a palette and return it (pushing it
    /// back if it was not). Lets palette edits own the value without borrowing
    /// `self` twice; the caller pushes the modified palette back.
    fn take_palette(&mut self) -> Option<Palette> {
        match self.mode.pop_overlay() {
            Some(Overlay::Palette(p)) => Some(p),
            Some(other) => {
                self.mode.push_overlay(other);
                None
            }
            None => None,
        }
    }

    // ---- palette (file jump + commit picker) -------------------------------

    pub fn open_palette(&mut self) {
        let mut p = Palette {
            kind: PaletteKind::Files,
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
            mode_hint: "",
        };
        recompute(&mut p, self.cs().as_ref());
        self.mode.push_overlay(Overlay::Palette(p));
    }

    /// Open the commit picker over commits from HEAD, excluding the reviewed
    /// range's own commits when the current view is a range review.
    pub fn open_commit_palette(&mut self) {
        self.open_commit_picker(None);
    }

    /// Open the commit picker scoped to the selected file's history.
    pub fn open_file_history(&mut self) {
        let path = self
            .state()
            .selected_file()
            .and_then(|i| self.cs().files.get(i))
            .map(|f| f.path.clone());
        self.open_commit_picker(path);
    }

    fn open_commit_picker(&mut self, scoped_path: Option<String>) {
        let Some(dir) = self.session.repo_dir.clone() else {
            return;
        };
        let exclude = self.range_exclusion(&dir);
        let (commits, truncated) = crate::git::enumerate_commits(
            &dir,
            "HEAD",
            crate::git::COMMIT_CAP,
            scoped_path.as_deref(),
            &exclude,
        )
        .unwrap_or_default();
        let mut p = Palette {
            kind: PaletteKind::Commits {
                commits,
                scoped_path,
                truncated,
            },
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
            mode_hint: "summary",
        };
        recompute(&mut p, self.cs().as_ref());
        self.mode.push_overlay(Overlay::Palette(p));
    }

    /// The set of commit ids to hide while reviewing a range; empty otherwise.
    fn range_exclusion(&self, dir: &std::path::Path) -> std::collections::HashSet<String> {
        if let ViewKind::Range { base, target } = self.kind() {
            crate::git::range_commit_ids(dir, base, target).unwrap_or_default()
        } else {
            std::collections::HashSet::new()
        }
    }

    pub fn palette_input(&mut self, c: char) {
        if let Some(mut p) = self.take_palette() {
            p.query.push(c);
            recompute(&mut p, self.cs().as_ref());
            self.refresh_commit_mode(&mut p);
            self.mode.push_overlay(Overlay::Palette(p));
        }
    }

    pub fn palette_backspace(&mut self) {
        if let Some(mut p) = self.take_palette() {
            p.query.pop();
            recompute(&mut p, self.cs().as_ref());
            self.refresh_commit_mode(&mut p);
            self.mode.push_overlay(Overlay::Palette(p));
        }
    }

    /// Re-derive a commit palette's matches from the smart-filter interpretation:
    /// hex prefix → SHA match; an exactly-typed known path → re-scope to that
    /// file's history (clearing the query, like `F`); otherwise fuzzy summary.
    fn refresh_commit_mode(&mut self, p: &mut Palette) {
        let PaletteKind::Commits { scoped_path, .. } = &p.kind else {
            return;
        };
        let q = p.query.clone();
        // Already a file-scoped list, or empty query → fuzzy over the summaries.
        if scoped_path.is_some() || q.is_empty() {
            p.mode_hint = if scoped_path.is_some() {
                "file history"
            } else {
                "summary"
            };
            recompute(p, self.cs().as_ref());
            return;
        }
        let is_sha = q.len() >= 4 && q.chars().all(|c| c.is_ascii_hexdigit());
        if is_sha {
            p.mode_hint = "sha";
            if let PaletteKind::Commits { commits, .. } = &p.kind {
                p.matches = commits
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.id.starts_with(&q) || c.short.starts_with(&q))
                    .map(|(i, _)| i)
                    .collect();
            }
            p.selected = p.selected.min(p.matches.len().saturating_sub(1));
        } else if self.cs().files.iter().any(|f| f.path == q) {
            // Exact known path → re-scope to that file's history.
            if let Some(dir) = self.session.repo_dir.clone() {
                let exclude = self.range_exclusion(&dir);
                if let Ok((commits, truncated)) = crate::git::enumerate_commits(
                    &dir,
                    "HEAD",
                    crate::git::COMMIT_CAP,
                    Some(&q),
                    &exclude,
                ) {
                    p.query.clear();
                    p.matches = (0..commits.len()).collect();
                    p.selected = 0;
                    p.mode_hint = "file history";
                    p.kind = PaletteKind::Commits {
                        commits,
                        scoped_path: Some(q),
                        truncated,
                    };
                }
            }
        } else {
            p.mode_hint = "summary";
            recompute(p, self.cs().as_ref());
        }
    }

    #[expect(
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "match counts/indices fit in isize, and the result is clamped to [0, last] so it is non-negative"
    )]
    pub fn palette_move(&mut self, delta: isize) {
        if let Some(p) = self.palette_mut() {
            if p.matches.is_empty() {
                return;
            }
            let last = p.matches.len() as isize - 1;
            p.selected = (p.selected as isize + delta).clamp(0, last) as usize;
        }
    }

    pub fn palette_confirm(&mut self) {
        let Some(p) = self.take_palette() else { return };
        let Some(&idx) = p.matches.get(p.selected) else {
            // No match to confirm (e.g. a query that filtered everything out):
            // keep the picker open with its query rather than dropping it.
            self.mode.push_overlay(Overlay::Palette(p));
            return;
        };
        if let PaletteKind::Commits {
            commits,
            scoped_path,
            ..
        } = &p.kind
        {
            let Some(rev) = commits.get(idx).map(|ci| ci.id.clone()) else {
                return;
            };
            let land = scoped_path.clone();
            if !self.open_commit(&rev, land.as_deref(), None) {
                // A failed switch restores the picker (query, scope, selection)
                // instead of stranding the user on the base view with nothing.
                self.mode.push_overlay(Overlay::Palette(p));
            }
        } else {
            // Jump-by-path always lands: unfold the file's directory if folded.
            self.land_on_file(idx);
        }
    }

    /// Pick the n-th (0-based) filtered match by number and jump to it.
    pub fn palette_pick(&mut self, n: usize) {
        if let Some(p) = self.palette_mut() {
            if n < p.matches.len() {
                p.selected = n;
                self.palette_confirm();
            }
        }
    }

    pub fn palette_close(&mut self) {
        self.mode.pop_overlay();
    }

    /// Whether the open palette (if any) is the commit picker.
    pub fn commit_palette_open(&self) -> bool {
        matches!(
            self.palette().map(|p| &p.kind),
            Some(PaletteKind::Commits { .. })
        )
    }

    // ---- commit-message popup ----------------------------------------------

    /// Open the shared commit-message popup for `sha`, fetching its full body.
    /// Pushed onto the overlay stack, so an open commit picker beneath it is
    /// revealed again by a dismiss (pop); opened over the blame peek base there
    /// is simply nothing beneath. A repo or fetch failure is a silent no-op.
    pub fn open_commit_message(&mut self, sha: &str) {
        let Some(dir) = self.session.repo_dir.clone() else {
            return;
        };
        let Ok(msg) = crate::git::commit_message(&dir, sha) else {
            return;
        };
        self.mode
            .push_overlay(Overlay::CommitMessage(CommitMsg::new(msg)));
    }

    /// The open commit-message popup, if any.
    pub fn commit_msg(&self) -> Option<&CommitMsg> {
        match self.mode.overlay() {
            Some(Overlay::CommitMessage(m)) => Some(m),
            _ => None,
        }
    }

    pub fn commit_msg_open(&self) -> bool {
        self.commit_msg().is_some()
    }

    /// The single input-context resolver: which surface captures input right
    /// now. Both the key router (`runtime::keys::handle_key`) and the status
    /// bar's bindings ([`App::status_bindings`]) consume this one precedence,
    /// so what is dispatched and what is advertised cannot drift apart. The
    /// overlay slot (one at a time) always outranks the peek base; the peek
    /// outranks the normal panes.
    pub fn active_context(&self) -> InputContext {
        match self.mode.overlay() {
            Some(Overlay::Help) => InputContext::Help,
            Some(Overlay::CommitMessage(_)) => InputContext::CommitMsg,
            Some(Overlay::Palette(_)) => InputContext::Palette,
            Some(Overlay::ThemePicker(_)) => InputContext::ThemePicker,
            Some(Overlay::Comment(_)) => InputContext::Comment,
            None if self.peek_open() => InputContext::Peek,
            None => InputContext::Normal,
        }
    }

    /// The binding table the status bar renders for the active input context
    /// (resolved by [`App::active_context`], the same resolver the key router
    /// dispatches from).
    pub fn status_bindings(&self) -> &'static [crate::tui::keymap::Binding] {
        use crate::tui::keymap as k;
        use crate::tui::peek::PeekMode;
        match self.active_context() {
            // Help renders its own dismiss hint in the status bar, not a table.
            InputContext::Help => &[],
            InputContext::CommitMsg => k::BIND_COMMITMSG,
            InputContext::Comment => k::BIND_COMMENT,
            InputContext::ThemePicker => k::BIND_THEME,
            InputContext::Palette => {
                if self.commit_palette_open() {
                    k::BIND_PALETTE_COMMIT
                } else {
                    k::BIND_PALETTE_FILE
                }
            }
            InputContext::Peek => match self.peek().map(|p| p.mode) {
                Some(PeekMode::Diff) => k::BIND_PEEK_DIFF,
                Some(PeekMode::Blame) => k::BIND_PEEK_BLAME,
                _ => k::BIND_PEEK_CONTENT,
            },
            InputContext::Normal => match self.focus() {
                Focus::Sidebar => k::BIND_SIDEBAR,
                Focus::Stream => k::BIND_STREAM,
            },
        }
    }

    /// Open the highlighted commit's message from the commit picker (`Tab`).
    /// A no-op for the file palette or an empty result list.
    pub fn palette_open_highlighted_message(&mut self) {
        let sha = self.palette().and_then(|p| match &p.kind {
            PaletteKind::Commits { commits, .. } => p
                .matches
                .get(p.selected)
                .and_then(|&i| commits.get(i))
                .map(|c| c.id.clone()),
            PaletteKind::Files => None,
        });
        if let Some(sha) = sha {
            self.open_commit_message(&sha);
        }
    }

    /// Scroll the popup body, stopping a page short of the end so the last screen
    /// stays full (the bottom of the message reaches the bottom of the viewport,
    /// not the top).
    pub fn commit_msg_scroll(&mut self, delta: isize) {
        let vh = self.commit_msg_viewport_h;
        if let Some(Overlay::CommitMessage(m)) = self.mode.overlay_mut() {
            let max = crate::tui::stream::max_scroll_rows(m.body_lines, vh);
            let step = delta.unsigned_abs();
            m.scroll = if delta >= 0 {
                (m.scroll + step).min(max)
            } else {
                m.scroll.saturating_sub(step)
            };
        }
    }

    /// Page the popup body by its viewport height (mirroring the peek and the
    /// stream, which both page by viewport − 1).
    #[expect(
        clippy::cast_possible_wrap,
        reason = "the popup viewport height is a small terminal dimension, well under isize::MAX"
    )]
    pub fn commit_msg_page(&mut self, dir: isize) {
        let step = self.commit_msg_viewport_h.saturating_sub(1).max(1) as isize;
        self.commit_msg_scroll(dir * step);
    }

    /// Confirm the popup: switch to its commit, landing on the file the popup
    /// was reached through (the stashed file-history picker's scope, or the
    /// blamed file when opened over the peek) — the same landing the picker's
    /// own confirm performs. Only on success is context torn down (popup
    /// dropped, peek closed); a failed switch keeps everything so a transient
    /// repo error doesn't strand the user on the base view.
    pub fn commit_msg_confirm(&mut self) {
        let Some(Overlay::CommitMessage(m)) = self.mode.pop_overlay() else {
            return;
        };
        // Popping the popup reveals whatever it was summoned over: a commit
        // picker (land on its scoped file) or the blame peek (land on its file).
        let land = match self.mode.overlay() {
            Some(Overlay::Palette(Palette {
                kind: PaletteKind::Commits { scoped_path, .. },
                ..
            })) => scoped_path.clone(),
            _ => self.peek().map(|p| p.path.clone()),
        };
        // The popup already fetched the message — reuse it for the banner.
        if self.open_commit(&m.msg.sha, land.as_deref(), Some(&m.msg)) {
            // Switched views: the picker beneath (if any) and the peek are stale.
            if matches!(self.mode.overlay(), Some(Overlay::Palette(_))) {
                self.mode.pop_overlay();
            }
            if self.peek_open() {
                self.peek_close();
            }
        } else {
            self.mode.push_overlay(Overlay::CommitMessage(m));
        }
    }

    /// Dismiss the popup: pop it off the stack, revealing whatever it was
    /// summoned over (the commit picker, or the blame peek base).
    pub fn commit_msg_dismiss(&mut self) {
        self.mode.pop_overlay();
    }

    pub fn toggle_help(&mut self) {
        if self.help_open() {
            self.mode.pop_overlay();
        } else {
            self.mode.push_overlay(Overlay::Help);
        }
    }
}

/// Subsequence-filter commits by summary (and short sha), best first. An empty
/// query keeps changeset order.
fn filter_commits_text(commits: &[CommitInfo], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..commits.len()).collect();
    }
    let mut scored: Vec<(i32, usize)> = commits
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let hay = format!("{} {}", c.summary, c.short);
            fuzzy::score(query, &hay).map(|s| (s, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// Recompute palette matches for its current query.
fn recompute(p: &mut Palette, cs: &Changeset) {
    match &p.kind {
        PaletteKind::Files => {
            let mut scored: Vec<(i32, usize)> = cs
                .files
                .iter()
                .enumerate()
                .filter_map(|(i, f)| fuzzy::score(&p.query, &f.path).map(|s| (s, i)))
                .collect();
            // Best score first; stable by original order on ties.
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            p.matches = scored.into_iter().map(|(_, i)| i).collect();
            p.selected = p.selected.min(p.matches.len().saturating_sub(1));
        }
        PaletteKind::Commits { commits, .. } => {
            p.matches = filter_commits_text(commits, &p.query);
            p.selected = p.selected.min(p.matches.len().saturating_sub(1));
        }
    }
}

#[cfg(test)]
mod overlays_tests;
