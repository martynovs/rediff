//! Core app state types: the `App` struct, the input `Mode`/`Base`/`Overlay`
//! model, the palette and theme-picker overlay state, and shared consts.

use std::time::Duration;

use ratatui::layout::Rect;

use crate::highlight::Rgb;
use crate::model::{CommitInfo, CommitMessage, LayoutMode};
use crate::tui::highlight::HlService;
use crate::tui::peek::Peek;
use crate::tui::session::Session;
use crate::tui::sidebar;
use crate::tui::theme::{Theme, ThemeName};

/// Reserved highlight-cache index for the single-file peek (never a real file).
pub const PEEK_HL: usize = usize::MAX;

/// How long a load must run before any progress chrome appears, so small,
/// fast loads never flash a loading indicator.
pub const LOAD_PROGRESS_DELAY: Duration = Duration::from_millis(80);

/// Which pane currently receives navigation keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Stream,
    Sidebar,
}

/// The active input mode: a base context (the stream, or the single-file peek)
/// with a stack of transient overlays layered on top. The topmost overlay
/// captures input and is the one drawn; dismissing it pops back to whatever was
/// beneath (another overlay, or the base). A stack — rather than a single slot
/// plus per-overlay stash fields — makes restore uniform and means no opener can
/// silently drop a stashed overlay. This is the single source of truth for input
/// routing and overlay selection.
pub struct Mode {
    pub base: Base,
    overlays: Vec<Overlay>,
}

impl Mode {
    /// The active (topmost) overlay, if any.
    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlays.last()
    }

    pub(crate) fn overlay_mut(&mut self) -> Option<&mut Overlay> {
        self.overlays.last_mut()
    }

    /// The whole stack, for the rare case that needs an overlay which is not the
    /// topmost — the thread list under an edit input being the one.
    pub(crate) fn overlays_mut(&mut self) -> &mut [Overlay] {
        &mut self.overlays
    }

    /// Push a transient overlay onto the stack (the new topmost / active one).
    pub(crate) fn push_overlay(&mut self, overlay: Overlay) {
        self.overlays.push(overlay);
    }

    /// Pop the topmost overlay, revealing whatever was beneath it.
    pub(crate) fn pop_overlay(&mut self) -> Option<Overlay> {
        self.overlays.pop()
    }
}

/// What the body is showing — the context the user lives in.
pub enum Base {
    /// The normal diff stream, with keyboard focus on the stream or the sidebar.
    Normal { focus: Focus },
    /// The modal single-file peek (boxed — `Peek` is large, and most bases are
    /// `Normal`, so keeping it out-of-line keeps `Mode`/`App` lean).
    Peek(Box<Peek>),
}

/// A transient layer summoned over a base; it captures input until dismissed.
pub enum Overlay {
    Palette(Palette),
    Help,
    ThemePicker(ThemePicker),
    /// The shared commit-message popup (from the picker's `Tab` or a blame line).
    CommitMessage(CommitMsg),
    /// Composing a review comment. Pushed over whatever summoned it, so editing
    /// a thread from the thread list returns to the list.
    Comment(CommentInput),
    /// The review's threads, with a selection.
    Threads(ThreadList),
    /// Closing the round: pick a verdict preset, then edit its text.
    Submit(SubmitDraft),
}

/// Which surface captures input right now, resolved by [`App::active_context`]
/// — the single precedence that both the key router and the status bar's
/// advertised bindings consume, so dispatch and display cannot drift apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputContext {
    Help,
    CommitMsg,
    Palette,
    ThemePicker,
    Comment,
    Threads,
    Submit,
    Peek,
    Normal,
}

/// Closing a review round: a two-stage overlay.
///
/// Stage one picks a preset, stage two edits its text and sends. Two stages
/// rather than one editable line with a cycle key, because cycling would have to
/// either discard what the user typed or keep a stale preset name — and the one
/// thing this record has to get right is that the body delivered is the body the
/// human actually wrote.
pub struct SubmitDraft {
    /// The offered presets, from config or the built-ins.
    pub presets: Vec<crate::config::VerdictPreset>,
    /// Cursor in `presets` while picking; the chosen one once editing.
    pub selected: usize,
    /// `None` while picking a preset, `Some` while editing its text.
    pub buffer: Option<String>,
    /// Why the last send was refused, shown inside the box (the status bar hides
    /// the flash while an overlay is up).
    pub refusal: Option<String>,
}

impl SubmitDraft {
    pub fn new(presets: Vec<crate::config::VerdictPreset>) -> Self {
        Self {
            presets,
            selected: 0,
            buffer: None,
            refusal: None,
        }
    }

    /// The preset under the cursor.
    pub fn current(&self) -> Option<&crate::config::VerdictPreset> {
        self.presets.get(self.selected)
    }

    /// Move the preset cursor, clamped — a preset list does not wrap.
    pub fn move_by(&mut self, delta: isize) {
        let last = self.presets.len().saturating_sub(1);
        #[expect(
            clippy::cast_possible_wrap,
            clippy::cast_sign_loss,
            reason = "a config's preset count is far below isize::MAX; clamped to >= 0 before the cast back"
        )]
        let next = (self.selected as isize + delta).clamp(0, last as isize) as usize;
        self.selected = next;
    }

    /// Enter the editing stage, pre-filled from the selected preset.
    pub fn begin_edit(&mut self) {
        if let Some(p) = self.current() {
            self.buffer = Some(p.text.clone());
        }
    }

    /// The title, which also tells the user which stage they are in.
    pub fn title(&self) -> String {
        match (&self.buffer, self.current()) {
            (Some(_), Some(p)) => format!("submit · {}", p.name),
            (Some(_), None) => "submit".to_string(),
            (None, _) => "close the round — pick a verdict".to_string(),
        }
    }
}

/// The review's live threads, with a cursor over them.
pub struct ThreadList {
    pub threads: Vec<crate::tui::reviewlog::LiveThread>,
    pub selected: usize,
}

impl ThreadList {
    pub fn new(threads: Vec<crate::tui::reviewlog::LiveThread>) -> Self {
        Self {
            threads,
            selected: 0,
        }
    }

    pub fn current(&self) -> Option<&crate::tui::reviewlog::LiveThread> {
        self.threads.get(self.selected)
    }

    /// Move the selection, clamped — a list this short does not wrap.
    pub fn move_by(&mut self, delta: isize) {
        let last = self.threads.len().saturating_sub(1);
        #[expect(
            clippy::cast_possible_wrap,
            clippy::cast_sign_loss,
            reason = "a review's thread count is far below isize::MAX; clamped to >= 0 before the cast back"
        )]
        let next = (self.selected as isize + delta).clamp(0, last as isize) as usize;
        self.selected = next;
    }
}

/// Composing a review comment.
///
/// One line, like the palette's query — a review point is a sentence, and
/// anything longer belongs in the agent's reply. `anchor` is `None` for the
/// review-level comment the store models as an unanchored thread.
pub struct CommentInput {
    /// The line this comment is about, or `None` for a review-level comment.
    pub anchor: Option<crate::review::Anchor>,
    /// What the user has typed so far.
    pub buffer: String,
    /// Set when editing an existing thread: the id whose record this supersedes.
    pub replacing: Option<String>,
    /// Why the last save attempt was refused, shown inside the box.
    ///
    /// Not `App::flash`: `draw_status` suppresses the flash whenever an overlay
    /// is active, so a refusal reported that way is invisible at exactly the
    /// moment the input stays open to report it.
    pub refusal: Option<String>,
}

impl CommentInput {
    /// A fresh comment on `anchor` (or on the review, when `None`).
    pub fn new(anchor: Option<crate::review::Anchor>) -> Self {
        Self {
            anchor,
            buffer: String::new(),
            replacing: None,
            refusal: None,
        }
    }

    /// The one-line summary shown in the overlay's title.
    pub fn title(&self) -> String {
        match (&self.anchor, &self.replacing) {
            (Some(a), None) => format!("comment on {}:{}", a.path, a.line),
            (Some(a), Some(_)) => format!("edit comment on {}:{}", a.path, a.line),
            (None, None) => "comment on this review".to_string(),
            (None, Some(_)) => "edit review comment".to_string(),
        }
    }
}

/// The commit-message popup: a commit's full message, scrolled independently.
/// `Enter` switches the view to the commit; `Esc` pops it off the overlay stack,
/// revealing whatever it was summoned over (the commit picker, or the blame peek
/// base).
pub struct CommitMsg {
    /// The fetched commit identity and full body.
    pub msg: CommitMessage,
    /// Body scroll offset (top line).
    pub scroll: usize,
    /// `msg.body`'s line count, cached at construction so the popup's height,
    /// scroll clamp, and paint don't each re-scan a (possibly huge) body every
    /// frame.
    pub body_lines: usize,
}

impl CommitMsg {
    /// Open a popup for `msg`, caching its body line count.
    pub fn new(msg: CommitMessage) -> Self {
        let body_lines = msg.body.lines().count();
        CommitMsg {
            msg,
            scroll: 0,
            body_lines,
        }
    }
}

/// The live-preview theme picker: a grid of themes navigated with arrows/`hjkl`.
/// Moving the cursor applies the theme to the whole UI immediately; `Enter`
/// commits (and persists), `Esc` restores `original`.
pub struct ThemePicker {
    /// Cursor position within the active tab's theme list.
    pub selected: usize,
    /// Which tab is shown: dark themes (`true`) or light themes.
    pub dark_tab: bool,
    /// The theme active when the picker opened, restored on cancel.
    pub original: ThemeName,
}

/// Picker grid cell width (theme name + swatch), in columns.
pub const THEME_CELL_W: usize = 24;

impl Mode {
    pub(crate) fn normal() -> Self {
        Mode {
            base: Base::Normal {
                focus: Focus::Stream,
            },
            overlays: Vec::new(),
        }
    }
}

/// What a palette overlay selects.
pub enum PaletteKind {
    /// Fuzzy file jump within the current view; matches index into `cs.files`.
    Files,
    /// Commit picker; matches index into `commits`. `scoped_path` is set for a
    /// file-scoped (`F`) list.
    Commits {
        commits: Vec<CommitInfo>,
        scoped_path: Option<String>,
        truncated: bool,
    },
}

/// A filtered popup overlay (file jump or commit pick).
pub struct Palette {
    pub kind: PaletteKind,
    pub query: String,
    /// Indices into the palette's backing list, best first.
    pub matches: Vec<usize>,
    pub selected: usize,
    /// Active interpretation of the query, for the commit picker.
    pub mode_hint: &'static str,
}

pub struct App {
    /// The browsing session: the view stack + the background load machine.
    pub session: Session,
    /// Configured layout (split/stack). App-global; the per-view plan is built
    /// for it and rebuilt when it toggles.
    pub layout: LayoutMode,
    /// Sidebar file-list grouping (flat list or grouped by directory). App-global.
    pub grouping: sidebar::Grouping,
    /// Last known stream viewport height (rows). Updated each draw.
    pub viewport_h: usize,
    /// Last known stream viewport width (columns). Updated each draw.
    pub viewport_w: usize,
    pub sidebar_w: u16,
    /// Whether the sidebar (file panel) is hidden to give the diff full width.
    pub sidebar_hidden: bool,
    /// First sidebar file row currently visible (sidebar windowing).
    pub sidebar_top: usize,
    /// Number of sidebar file rows currently visible.
    pub sidebar_visible: usize,
    /// Sidebar viewport height from the last draw.
    pub sidebar_height: usize,
    /// The active input mode (base + at most one overlay): the single source of
    /// truth for keyboard/mouse routing and overlay selection.
    pub mode: Mode,
    /// Peek viewport height (rows) from the last draw, for page/half-page.
    pub peek_viewport_h: usize,
    /// Commit-message popup body height (rows) from the last draw, so its scroll
    /// stops a page short of the end (the last screen stays full).
    pub commit_msg_viewport_h: usize,
    /// Help-overlay scroll offset (top row). The key catalog outgrew an 80x24
    /// terminal, so the box scrolls instead of clipping its last section.
    pub help_scroll: usize,
    /// Help-overlay visible content rows from the last draw, bounding the scroll.
    pub help_viewport_h: usize,
    pub theme: Theme,
    /// The active theme's per-capture color table, indexed by
    /// `highlight::Paint::Capture`. Rebuilt when the theme changes; read at
    /// render time so a theme switch recolors cached content with no re-highlight.
    pub syntax: Vec<Rgb>,
    /// Sidebar geometry from the last draw, for mouse hit-testing.
    pub sidebar_area: Rect,
    pub hl: HlService,
    /// A transient one-line status note (e.g. next-unviewed's "N hidden in folded
    /// dirs" cue), shown until the next key clears it.
    pub flash: Option<String>,
    /// The worktree's review log, resolved once at launch. `None` when the app
    /// was not launched from a worktree (or in tests that do not need one).
    pub review_log: Option<crate::review::Log>,
    /// Lines carrying a live review comment, for the gutter marker.
    ///
    /// Rebuilt only when *we* append to the log: nothing watches the file, and
    /// replaying plus re-resolving on the 100 ms poll tick would re-read the log
    /// and re-split every referenced file at 10 Hz.
    pub thread_marks: std::collections::HashMap<crate::tui::rows::Key, usize>,
    /// Verdict presets offered when closing a round. Seeded with the built-ins
    /// so every constructor has a usable set, and replaced from `config.toml` in
    /// `tui::run`.
    pub verdicts: Vec<crate::config::VerdictPreset>,
    /// Whether the launch view was narrowed by path filters. Only the launch
    /// view can be: every pushed view loads a whole target. A round hashed over
    /// a subset would make the next unfiltered `rediff request` report every
    /// excluded file as added, so a filtered view refuses to open a review.
    pub launch_filtered: bool,
    pub should_quit: bool,
}
