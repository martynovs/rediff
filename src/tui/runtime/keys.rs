//! Key dispatch: the top-level precedence orchestrator and its per-stage
//! handlers (overlays, the single-file peek, Ctrl chords, focus-independent
//! keys, and the focused pane).

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::tui::app::{App, Focus, InputContext};
use crate::tui::theme::ThemeName;

/// Step for a Shift-modified ("fast") scroll or pan; unmodified moves by one.
pub(crate) const BIG_STEP: isize = 8;

/// Half-page the focused pane: scroll the file list (no selection change) when
/// the sidebar is focused, otherwise the diff stream.
fn half_page_focused(app: &mut App, dir: isize) {
    match app.focus() {
        Focus::Sidebar => {
            #[expect(
                clippy::cast_possible_wrap,
                reason = "half the visible sidebar height is a small terminal dimension, far below isize::MAX"
            )]
            let step = (app.sidebar_visible / 2).max(1) as isize;
            app.sidebar_scroll(dir * step);
        }
        Focus::Stream => app.half_page(dir),
    }
}

/// Persist the committed theme to the config file. A write failure is surfaced
/// as a status flash, not an error — the in-session theme already applied.
fn persist_theme(app: &mut App, name: ThemeName) {
    if let Err(e) = crate::config::Config::save_theme(name.display()) {
        app.flash = Some(format!("theme not saved: {e}"));
    }
}

/// Top-level key dispatch, routed by the same [`App::active_context`] resolver
/// the status bar renders bindings from — one precedence (overlay, then peek,
/// then the normal panes), so dispatch and advertised keys cannot drift apart.
/// Each context's key set lives in its own small handler.
pub(crate) fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    // A transient status cue lives until the next keystroke.
    app.flash = None;
    match app.active_context() {
        // The help overlay captures all input; any key dismisses it.
        InputContext::Help => app.toggle_help(),
        InputContext::CommitMsg => handle_commit_msg_key(app, code),
        InputContext::Palette => handle_palette_key(app, code),
        InputContext::ThemePicker => handle_theme_picker_key(app, code),
        InputContext::Comment => handle_comment_key(app, code, mods),
        // The single-file peek base captures all input while open.
        InputContext::Peek => handle_peek_key(app, code, mods),
        InputContext::Normal => handle_base_key(app, code, mods),
    }
}

/// Composing a comment: text entry, `Enter` to save, `Esc` to discard.
///
/// Deliberately tiny and deliberately *not* part of `handle_global_key`, which
/// is at CRAP 30 against a threshold of 30 and has no headroom at all.
fn handle_comment_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match code {
        KeyCode::Esc => app.comment_cancel(),
        KeyCode::Enter => app.comment_confirm(),
        KeyCode::Backspace => app.comment_backspace(),
        // A Ctrl chord is not text. Without this, Ctrl+C inserts a literal `c`
        // into the one buffer whose contents get persisted.
        KeyCode::Char(_) if mods.contains(KeyModifiers::CONTROL) => {}
        KeyCode::Char(c) => app.comment_input(c),
        _ => {}
    }
}

/// The normal (no overlay, no peek) key stages: a streaming-load cancel, then
/// Ctrl chords, then focus-independent keys, and finally the focused pane.
fn handle_base_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    // While a diff load streams, Esc/q cancels it: quit at launch, or return to
    // the previous view for a mid-session switch. Other keys still navigate the
    // (already-listed) files.
    if app.loading() && matches!(code, KeyCode::Esc | KeyCode::Char('q')) {
        app.cancel_load();
        return;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        handle_ctrl_key(app, code);
        return;
    }
    if handle_global_key(app, code, mods) {
        return;
    }
    if handle_review_key(app, code) {
        return;
    }
    handle_focus_key(app, code, mods);
}

/// Review-point keys, live in either pane.
///
/// Its own handler rather than an arm in `handle_global_key`, which sits at CRAP
/// 30 against a threshold of 30 — and rather than `handle_stream_key`, where
/// they were silently dead whenever the sidebar had focus. Returns whether the
/// key was consumed.
fn handle_review_key(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('a') => app.open_comment(true),
        KeyCode::Char('A') => app.open_comment(false),
        _ => return false,
    }
    true
}

/// The fuzzy palette captures all input while open. Digits pick a result; Tab
/// reads the highlighted commit's full message (commit picker only).
fn handle_palette_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => app.palette_close(),
        KeyCode::Enter => app.palette_confirm(),
        KeyCode::Tab => app.palette_open_highlighted_message(),
        KeyCode::Up => app.palette_move(-1),
        KeyCode::Down => app.palette_move(1),
        KeyCode::Backspace => app.palette_backspace(),
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            app.palette_pick((c as u8 - b'1') as usize);
        }
        KeyCode::Char(c) => app.palette_input(c),
        _ => {}
    }
}

/// The commit-message popup: scroll the body, Enter switches to the commit,
/// Esc/q restores the base (the picker or the blame peek) beneath it.
fn handle_commit_msg_key(app: &mut App, code: KeyCode) {
    match code {
        // Esc/q/Tab all dismiss — Tab mirrors the picker key that opened it, so a
        // second Tab toggles the popup back to the picker beneath it.
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Tab => app.commit_msg_dismiss(),
        KeyCode::Enter => app.commit_msg_confirm(),
        KeyCode::Down | KeyCode::Char('j') => app.commit_msg_scroll(1),
        KeyCode::Up | KeyCode::Char('k') => app.commit_msg_scroll(-1),
        // Page by the popup's viewport, like every other pane's paging.
        KeyCode::PageDown | KeyCode::Char(' ') => app.commit_msg_page(1),
        KeyCode::PageUp => app.commit_msg_page(-1),
        _ => {}
    }
}

/// The theme picker: arrows/hjkl navigate the grid and live-preview, Enter
/// commits (and persists), Esc rolls back, Tab switches dark/light tabs.
fn handle_theme_picker_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => app.theme_picker_cancel(),
        KeyCode::Enter => {
            if let Some(name) = app.theme_picker_commit() {
                persist_theme(app, name);
            }
        }
        KeyCode::Left | KeyCode::Char('h') => app.theme_picker_move(-1, 0),
        KeyCode::Right | KeyCode::Char('l') => app.theme_picker_move(1, 0),
        KeyCode::Up | KeyCode::Char('k') => app.theme_picker_move(0, -1),
        KeyCode::Down | KeyCode::Char('j') => app.theme_picker_move(0, 1),
        KeyCode::Tab | KeyCode::BackTab => app.theme_picker_toggle_tab(),
        KeyCode::Char('t') => app.theme_picker_next(),
        _ => {}
    }
}

/// Peek-overlay controls plus the full stream-navigation set (so every scroll
/// key works here too). Mode-specific keys are dispatched only in the mode that
/// advertises them (`handle_peek_mode_key`), so the router — not just a guard
/// buried in the callee — keeps dispatch and the `BIND_PEEK_*` tables in step.
fn handle_peek_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let shift = mods.contains(KeyModifiers::SHIFT);
    // A key that belongs to the current mode's table is handled there and only
    // there; everything else is shared navigation below.
    if handle_peek_mode_key(app, code) {
        return;
    }
    match code {
        // Controls
        KeyCode::Esc | KeyCode::Char('q') => app.peek_close(),
        KeyCode::Tab => app.peek_toggle_mode(),
        // Navigation (mirrors the stream)
        KeyCode::Char('f' | 'd') if ctrl => app.peek_half_page(1),
        KeyCode::Char('b' | 'u') if ctrl => app.peek_half_page(-1),
        KeyCode::Char(']') => app.peek_hunk(1),
        KeyCode::Char('[') => app.peek_hunk(-1),
        KeyCode::Char('J') => app.peek_scroll(BIG_STEP),
        KeyCode::Char('K') => app.peek_scroll(-BIG_STEP),
        KeyCode::Down if shift => app.peek_scroll(BIG_STEP),
        KeyCode::Up if shift => app.peek_scroll(-BIG_STEP),
        KeyCode::Down | KeyCode::Char('j') => app.peek_scroll(1),
        KeyCode::Up | KeyCode::Char('k') => app.peek_scroll(-1),
        KeyCode::PageDown | KeyCode::Char(' ') => app.peek_page(1),
        KeyCode::PageUp => app.peek_page(-1),
        KeyCode::Char('g') | KeyCode::Home => app.peek_top(),
        KeyCode::Char('G') | KeyCode::End => app.peek_bottom(),
        _ => {}
    }
}

/// Dispatch the keys that belong to only one peek mode's binding table, and only
/// when that mode is active — so a key never acts in a mode that doesn't
/// advertise it. Returns whether the key was consumed. Diff mode owns the
/// context (`=`/`-`) and split (`m`) keys; blame mode owns `Enter` (open the
/// cursor line's commit message).
fn handle_peek_mode_key(app: &mut App, code: KeyCode) -> bool {
    use crate::tui::peek::PeekMode;
    let Some(mode) = app.peek().map(|p| p.mode) else {
        return false;
    };
    match (mode, code) {
        (PeekMode::Diff, KeyCode::Char('=' | '+')) => app.peek_set_full(true),
        (PeekMode::Diff, KeyCode::Char('-' | '_')) => app.peek_set_full(false),
        (PeekMode::Diff, KeyCode::Char('m')) => app.peek_toggle_split(),
        (PeekMode::Blame, KeyCode::Enter) => app.peek_blame_open_message(),
        _ => return false,
    }
    true
}

/// Ctrl chords: file navigation and half-page scrolling of the focused pane.
fn handle_ctrl_key(app: &mut App, code: KeyCode) {
    match code {
        // Ctrl+↑/↓ move between files.
        KeyCode::Down => app.next_file(),
        KeyCode::Up => app.prev_file(),
        // Ctrl+f/b (and d/u) half-page the focused pane.
        KeyCode::Char('f' | 'd') => half_page_focused(app, 1),
        KeyCode::Char('b' | 'u') => half_page_focused(app, -1),
        _ => {}
    }
}

/// Keys that apply regardless of which pane is focused. Returns true if handled.
fn handle_global_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    match code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Tab | KeyCode::BackTab => app.toggle_focus(),
        KeyCode::Char('s') => app.toggle_sidebar(),
        KeyCode::Char('?') => app.toggle_help(),
        // Single-file peek: p = content preview, = / + = the file's diff with
        // expanded context.
        KeyCode::Char('p') => app.open_peek_preview(),
        KeyCode::Char('=' | '+') => app.open_peek_review(),
        // Blame the selected file (committed-rev) in a blame-mode peek.
        KeyCode::Char('b') => app.open_peek_blame(),
        // Space steps files: forward, or backward with Shift (needs a terminal
        // that reports Shift+Space — the kitty keyboard protocol).
        KeyCode::Char(' ') => {
            if mods.contains(KeyModifiers::SHIFT) {
                app.prev_file();
            } else {
                app.next_file();
            }
        }
        KeyCode::Char('/' | 'f') => app.open_palette(),
        // Commit navigation: c picker, F file-history, C home, R promote,
        // {/} view back/forward (these arrive as the shifted characters).
        KeyCode::Char('c') => app.open_commit_palette(),
        KeyCode::Char('F') => app.open_file_history(),
        KeyCode::Char('C') => app.view_home(),
        KeyCode::Char('R') => app.promote_review(),
        // Shift+brackets step files (plain printable chars — reliable in any
        // terminal, unlike Ctrl+[ which is indistinguishable from Esc).
        KeyCode::Char('{') => app.prev_file(),
        KeyCode::Char('}') => app.next_file(),
        // View history back / forward.
        KeyCode::Char('<') => app.view_back(),
        KeyCode::Char('>') => app.view_forward(),
        KeyCode::Char('v') => app.toggle_viewed(),
        KeyCode::Char('u') => {
            app.next_unviewed();
        }
        KeyCode::Char('m') => app.cycle_mode(),
        KeyCode::Char('t') => app.open_theme_picker(),
        KeyCode::Char('w') => app.toggle_wrap(),
        KeyCode::Char('D') => app.toggle_grouping(),
        // Fold the cursor's directory (z) / collapse-or-expand all (Z). Inert in
        // the flat view (no directories).
        KeyCode::Char('z') => app.toggle_fold(),
        KeyCode::Char('Z') => app.fold_all(),
        // 1–9 jump to a file spread across the visible sidebar set.
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            app.goto_visible_digit((c as u8 - b'0') as usize);
        }
        _ => return false,
    }
    true
}

/// Focus-dependent navigation: route to the sidebar file list or the diff stream.
fn handle_focus_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match app.focus() {
        Focus::Sidebar => handle_sidebar_key(app, code),
        Focus::Stream => handle_stream_key(app, code, mods),
    }
}

/// Sidebar-focused keys: move the cursor, or drop into the stream.
fn handle_sidebar_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Down | KeyCode::Char('j') => app.sidebar_move(1),
        KeyCode::Up | KeyCode::Char('k') => app.sidebar_move(-1),
        KeyCode::Char('g') | KeyCode::Home => app.sidebar_move(isize::MIN / 2),
        KeyCode::Char('G') | KeyCode::End => app.sidebar_move(isize::MAX / 2),
        // Enter / Esc / l / → drop into the stream at the selected file.
        KeyCode::Enter | KeyCode::Esc | KeyCode::Char('l') | KeyCode::Right => {
            app.focus_stream();
        }
        _ => {}
    }
}

/// Stream-focused keys: vertical/horizontal scroll (fast with Shift), paging,
/// hunk hops, and top/bottom.
fn handle_stream_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match code {
        KeyCode::Esc => app.should_quit = true,
        // Vertical: one line, or several with Shift (↑↓ or J/K).
        KeyCode::Down if mods.contains(KeyModifiers::SHIFT) => app.scroll_view(BIG_STEP),
        KeyCode::Up if mods.contains(KeyModifiers::SHIFT) => app.scroll_view(-BIG_STEP),
        KeyCode::Char('J') => app.scroll_view(BIG_STEP),
        KeyCode::Char('K') => app.scroll_view(-BIG_STEP),
        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
        KeyCode::PageDown => app.page(1),
        KeyCode::PageUp => app.page(-1),
        KeyCode::Char(']') => app.next_hunk(),
        KeyCode::Char('[') => app.prev_hunk(),
        KeyCode::Char('g') | KeyCode::Home => app.top(),
        KeyCode::Char('G') | KeyCode::End => app.bottom(),
        // Horizontal: one column, or several with Shift (←→ or H/L).
        KeyCode::Right if mods.contains(KeyModifiers::SHIFT) => app.h_scroll_by(BIG_STEP),
        KeyCode::Left if mods.contains(KeyModifiers::SHIFT) => app.h_scroll_by(-BIG_STEP),
        KeyCode::Char('L') => app.h_scroll_by(BIG_STEP),
        KeyCode::Char('H') => app.h_scroll_by(-BIG_STEP),
        KeyCode::Right | KeyCode::Char('l') => app.h_scroll_by(1),
        KeyCode::Left | KeyCode::Char('h') => app.h_scroll_by(-1),
        _ => {}
    }
}

#[cfg(test)]
mod keys_tests;
