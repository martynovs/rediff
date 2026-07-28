//! The single source of truth for how keybindings are *presented* — the `?`
//! help overlay and the status-line hints both render from here, so the two can
//! no longer drift from each other (the peek help is where they had already
//! diverged before this was centralized).
//!
//! The router itself (`mod.rs::handle_key`) stays the authoritative dispatcher:
//! its per-key precedence and modifier policy (chars match by value, arrows
//! check Shift, chords check Ctrl) don't collapse into a static table without
//! risking behavior changes. The `bindings_only_reference_documented_keys` test
//! below keeps the presented keys honest against the help catalog.

/// One thematic section of the `?` help overlay: a title and its `(keys, desc)`
/// rows. Keys are curated, human-readable labels (e.g. `"j / k  ↑↓"`), not the
/// router's raw key codes.
pub type HelpSection = (&'static str, &'static [(&'static str, &'static str)]);

/// Left column of the help overlay.
pub const HELP_LEFT: &[HelpSection] = &[
    (
        "Move",
        &[
            ("j / k  ↑↓", "move cursor · ⇧ scrolls"),
            ("PgDn / PgUp", "page down / up"),
            ("Ctrl-f / b", "half-page down / up"),
            ("[ / ]", "prev / next hunk"),
            ("{ / }", "prev / next file"),
            ("Space / ⇧Spc", "next / prev file"),
            ("g / G", "top / bottom"),
            ("h / l  ←→", "pan 1 col · ⇧ fast"),
            ("1–9", "jump to file"),
        ],
    ),
    (
        "View",
        &[
            ("Tab", "switch focus"),
            ("s", "show/hide sidebar"),
            ("m", "cycle layout"),
            ("t", "theme picker"),
            ("w", "line wrap"),
            ("D", "group by dir"),
            ("z / Z", "fold dir / all"),
            ("?", "this help"),
            ("q", "quit"),
        ],
    ),
];

/// Right column of the help overlay.
pub const HELP_RIGHT: &[HelpSection] = &[
    (
        "Commits & history",
        &[
            ("c", "pick a commit"),
            ("Tab", "read msg (in picker)"),
            ("F", "file history"),
            ("< / >", "view back / fwd"),
            ("C", "back to local"),
            ("/ or f", "fuzzy file jump"),
        ],
    ),
    (
        "Review",
        &[
            ("v", "toggle reviewed"),
            ("u", "next unreviewed"),
            ("R", "review commit"),
            ("a / A", "comment line / review"),
            ("n", "list comments"),
            ("e / x / o", "edit / retract / resolve"),
            ("y", "submit the round"),
        ],
    ),
    (
        "File peek",
        &[
            ("p", "preview file"),
            ("b", "blame file"),
            ("=", "diff (full)"),
            ("Tab =/-", "mode · full/compact"),
            ("Enter", "blame → commit msg"),
        ],
    ),
];

/// How many rendered rows a help column needs: a title and its rows per
/// section, plus a blank line between sections.
///
/// Shared with the renderer so the help box's scroll clamp is computed from the
/// same count the painter lays out.
#[must_use]
pub fn help_column_rows(sections: &[HelpSection]) -> usize {
    sections
        .iter()
        .enumerate()
        .map(|(i, (_, items))| usize::from(i > 0) + 1 + items.len())
        .sum()
}

/// One displayable key binding: a key label (e.g. `"jk"`, `"[ ]"`, `"Tab"`) and
/// what it does. Prose-only hints (e.g. "type to filter") use an empty `key`.
#[derive(Clone, Copy)]
pub struct Binding {
    pub key: &'static str,
    pub desc: &'static str,
}

/// Brief constructor for the binding tables below.
const fn b(key: &'static str, desc: &'static str) -> Binding {
    Binding { key, desc }
}

// The per-view binding registry: each input context declares the bindings it
// wants shown in the status bar, in one place. `App::status_bindings` resolves
// the active context to its table; the bottom bar renders it. The
// `bindings_only_reference_documented_keys` test keeps every advertised key
// documented in the help catalog above.

/// Diff-stream focus.
pub const BIND_STREAM: &[Binding] = &[
    b("jk", "cursor"),
    b("[ ]", "hunk"),
    b("{ }", "file"),
    b("a", "comment"),
    b("c", "commits"),
    b("F", "log"),
    b("< >", "hist"),
    b("b", "blame"),
    b("?", "help"),
    b("q", "quit"),
];

/// Sidebar (file list) focus.
pub const BIND_SIDEBAR: &[Binding] = &[
    b("↑↓", "select"),
    b("v", "viewed"),
    b("z", "fold"),
    b("c", "commits"),
    b("/", "jump"),
    b("?", "help"),
    b("q", "quit"),
];

/// File peek, diff mode (the only mode with hunks, context, and split).
pub const BIND_PEEK_DIFF: &[Binding] = &[
    b("jk", "scroll"),
    b("[ ]", "hunk"),
    b("Tab", "mode"),
    b("=/-", "context"),
    b("m", "split"),
    b("esc", "close"),
];

/// File peek, content mode (whole file, no hunks/context/split).
pub const BIND_PEEK_CONTENT: &[Binding] = &[b("jk", "scroll"), b("Tab", "mode"), b("esc", "close")];

/// File peek, blame mode: `Enter` opens the cursor line's commit message.
pub const BIND_PEEK_BLAME: &[Binding] = &[
    b("jk", "scroll"),
    b("Tab", "mode"),
    b("Enter", "commit msg"),
    b("esc", "close"),
];

/// The commit-message popup.
pub const BIND_COMMITMSG: &[Binding] = &[
    b("jk", "scroll"),
    b("Enter", "open commit"),
    b("Tab/esc", "close"),
];

/// Composing a review comment.
pub const BIND_COMMENT: &[Binding] = &[b("Enter", "save"), b("esc", "discard")];

/// The review's thread list, which is also where a thread is acted on: it is
/// the only surface with an unambiguous "this thread" to edit or retract.
pub const BIND_THREADS: &[Binding] = &[
    b("jk", "select"),
    b("Enter", "jump"),
    b("e", "edit"),
    b("x", "retract"),
    b("o", "resolve"),
    b("esc", "close"),
];

/// Closing a round, stage one: pick a verdict preset.
pub const BIND_SUBMIT_PICK: &[Binding] = &[
    b("jk", "select"),
    b("Enter", "edit text"),
    b("esc", "cancel"),
];

/// Closing a round, stage two: edit the instruction and send.
pub const BIND_SUBMIT_EDIT: &[Binding] = &[b("Enter", "submit"), b("esc", "back to presets")];

/// The fuzzy file palette.
pub const BIND_PALETTE_FILE: &[Binding] = &[
    b("", "type to filter"),
    b("↑↓", "select"),
    b("Enter", "jump"),
    b("Esc", "cancel"),
];

/// The commit picker.
pub const BIND_PALETTE_COMMIT: &[Binding] = &[
    b("", "type to filter"),
    b("↑↓", "select"),
    b("Tab", "msg"),
    b("1-9/Enter", "pick"),
    b("Esc", "cancel"),
];

/// The theme picker.
pub const BIND_THEME: &[Binding] = &[
    b("hjkl", "move"),
    b("Tab", "dark/light"),
    b("t", "next"),
    b("Enter", "apply"),
    b("q/Esc", "close"),
];

/// Join a binding table into a one-line hint string (`key desc · key desc`),
/// for the few places that render a plain string rather than styled spans.
pub fn to_hint(bindings: &[Binding]) -> String {
    bindings
        .iter()
        .map(|x| {
            if x.key.is_empty() {
                x.desc.to_string()
            } else {
                format!("{} {}", x.key, x.desc)
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn help_rows() -> Vec<(&'static str, &'static str)> {
        HELP_LEFT
            .iter()
            .chain(HELP_RIGHT)
            .flat_map(|(_, rows)| rows.iter().copied())
            .collect()
    }

    /// Every binding table the status bar can render.
    const ALL_TABLES: &[&[Binding]] = &[
        BIND_STREAM,
        BIND_SIDEBAR,
        BIND_PEEK_DIFF,
        BIND_PEEK_CONTENT,
        BIND_PEEK_BLAME,
        BIND_COMMITMSG,
        BIND_COMMENT,
        BIND_THREADS,
        BIND_SUBMIT_PICK,
        BIND_SUBMIT_EDIT,
        BIND_PALETTE_FILE,
        BIND_PALETTE_COMMIT,
        BIND_THEME,
    ];

    #[test]
    fn help_catalog_is_well_formed() {
        let rows = help_rows();
        assert!(rows.len() > 10, "the help catalog should be substantial");
        for (keys, desc) in &rows {
            assert!(!keys.trim().is_empty(), "every help row names its keys");
            assert!(
                !desc.trim().is_empty(),
                "every help row has a description: {keys}"
            );
        }
    }

    #[test]
    fn binding_constructor_sets_both_fields() {
        // `b` builds the tables in const context (so it reads as uncovered at
        // runtime); call it at runtime to exercise it directly.
        let bind = b("X", "do X");
        assert_eq!((bind.key, bind.desc), ("X", "do X"));
    }

    #[test]
    fn to_hint_renders_keys_and_prose_only_bindings() {
        // A table with both a prose-only binding ("type to filter") and keyed ones
        // exercises both arms of `to_hint`.
        let s = to_hint(BIND_PALETTE_FILE);
        assert!(s.contains("type to filter"), "prose-only binding: {s}");
        assert!(s.contains("Enter jump"), "key + desc rendered: {s}");
        assert!(s.contains(" · "), "bindings joined by a separator: {s}");
    }

    #[test]
    fn every_binding_has_a_description() {
        for table in ALL_TABLES {
            for bind in *table {
                assert!(
                    !bind.desc.trim().is_empty(),
                    "every binding has a description (key {:?})",
                    bind.key
                );
            }
        }
    }

    /// A character that can stand alone as a keybinding token — letters, digits,
    /// and the symbol keys the UI uses. `/` is included (it is the file-jump key)
    /// so it is treated as a key, not a separator.
    fn is_key_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || "[]{}<>?=/".contains(c)
    }

    /// The single-character key tokens in a key label. Separators are `·` and
    /// space; multi-char tokens ("jk", "Tab", "=/-", "1-9/Enter") are key *groups*
    /// and are intentionally not treated as single keys.
    fn single_keys(s: &str) -> Vec<char> {
        s.split(['·', ' '])
            .filter(|tok| tok.chars().count() == 1)
            .filter_map(|tok| tok.chars().next())
            .filter(|c| is_key_char(*c))
            .collect()
    }

    /// Every single-character key advertised by ANY view's bindings must be
    /// documented in the help catalog, so the bottom bar and the help overlay
    /// cannot drift apart.
    #[test]
    fn bindings_only_reference_documented_keys() {
        let documented: Vec<char> = help_rows()
            .iter()
            .flat_map(|(keys, _)| single_keys(keys))
            .collect();
        for table in ALL_TABLES {
            for bind in *table {
                for c in single_keys(bind.key) {
                    assert!(
                        documented.contains(&c),
                        "binding key '{c}' is not documented in the help catalog"
                    );
                }
            }
        }
    }
}
