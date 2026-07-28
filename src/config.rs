//! Persisted preferences from `~/.config/rediff/config.toml`. Missing or
//! malformed config falls back to defaults without error. CLI flags override.

use std::io;
use std::path::PathBuf;

use serde::Deserialize;

use crate::model::LayoutMode;

#[derive(Debug, Default)]
pub struct Config {
    /// "dark" | "light".
    pub theme: Option<String>,
    /// "split" | "stack" (or "auto"/unset to pick by terminal width at startup).
    pub mode: Option<String>,
    /// `[[verdict]]` presets, in file order. Empty means "none configured",
    /// which [`Config::verdicts`] reads as "use the built-ins".
    pub verdict: Vec<VerdictPreset>,
}

/// One named parting instruction offered when closing a review round.
///
/// The name is recorded alongside the body purely so a script can branch without
/// parsing prose; the body delivered is whatever the human actually sent, edits
/// and all.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VerdictPreset {
    /// Short label shown in the picker and recorded on the `submit`.
    pub name: String,
    /// The instruction, pre-filled into the editor and editable before sending.
    pub text: String,
}

/// The presets offered when `config.toml` configures none.
///
/// Three, covering the outcomes a round actually has: it is fine, it needs work,
/// or the human wants an answer before anything changes.
pub const DEFAULT_VERDICTS: &[(&str, &str)] = &[
    ("approve", "looks good — nothing blocking"),
    (
        "rework",
        "address the review points above, then show me again",
    ),
    (
        "questions",
        "answer the questions above before changing anything",
    ),
];

impl Config {
    /// Load config from the standard path, returning defaults if absent/invalid.
    pub fn load() -> Config {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    /// Read each key independently, so one malformed key costs only itself.
    ///
    /// Deserializing the whole file into one struct would mean a typo in a
    /// `[[verdict]]` entry discards the user's theme and layout too — a config
    /// that silently reverts your colors because a preset is misspelt.
    pub(crate) fn parse(text: &str) -> Config {
        let table: toml::Table = toml::from_str(text).unwrap_or_default();
        let string = |k: &str| {
            table
                .get(k)
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        };
        Config {
            theme: string("theme"),
            mode: string("mode"),
            verdict: table
                .get("verdict")
                .cloned()
                .and_then(|v| v.try_into::<Vec<VerdictPreset>>().ok())
                .unwrap_or_default(),
        }
    }

    /// The verdict presets to offer: the configured ones, or the built-ins when
    /// the file configures none.
    ///
    /// An empty list means "unset", not "no way to close a round" — a config
    /// that could remove the only key that ends a review would be a trap.
    pub fn verdicts(&self) -> Vec<VerdictPreset> {
        if self.verdict.is_empty() {
            return DEFAULT_VERDICTS
                .iter()
                .map(|(name, text)| VerdictPreset {
                    name: (*name).to_string(),
                    text: (*text).to_string(),
                })
                .collect();
        }
        self.verdict.clone()
    }

    fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("rediff").join("config.toml"))
    }

    pub fn layout_mode(&self) -> Option<LayoutMode> {
        self.mode.as_deref().and_then(parse_mode)
    }

    /// Persist the `theme` preference to the config file, preserving existing
    /// keys and comments. Creates the directory/file if absent and writes
    /// atomically (temp + rename) so a crash never truncates the config.
    pub fn save_theme(theme: &str) -> io::Result<()> {
        let path =
            Self::path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config dir"))?;
        write_theme(&path, theme)
    }
}

/// Surgically set `theme` in the TOML document at `path` (or a fresh one),
/// preserving everything else. A missing file starts a fresh document; an
/// existing-but-malformed file is an error, NOT silently overwritten — clobbering
/// a user's hand-edited config (with a typo) would lose their other keys/comments.
#[expect(
    clippy::indexing_slicing,
    reason = "toml_edit's IndexMut<&str> inserts the key if absent; it never panics"
)]
fn write_theme(path: &std::path::Path, theme: &str) -> io::Result<()> {
    let mut doc = match std::fs::read_to_string(path) {
        Ok(existing) => existing
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => toml_edit::DocumentMut::new(),
        Err(e) => return Err(e),
    };
    doc["theme"] = toml_edit::value(theme);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())?;
    std::fs::rename(&tmp, path)
}

/// Parse an explicit layout-mode string. `"auto"` (and anything unrecognized)
/// returns `None`, meaning "pick by terminal width at startup".
pub fn parse_mode(s: &str) -> Option<LayoutMode> {
    match s.to_lowercase().as_str() {
        "split" => Some(LayoutMode::Split),
        "stack" => Some(LayoutMode::Stack),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toml_config() {
        let cfg = Config::parse("theme = \"light\"\nmode = \"split\"");
        assert_eq!(cfg.theme.as_deref(), Some("light"));
        assert_eq!(cfg.layout_mode(), Some(LayoutMode::Split));
    }

    #[test]
    fn empty_and_invalid_default_safely() {
        let cfg = Config::parse("");
        assert!(cfg.theme.is_none());
        assert_eq!(cfg.layout_mode(), None);
        assert_eq!(parse_mode("nonsense"), None);
        // Syntactically broken TOML is defaults, not a panic.
        let broken = Config::parse("theme = \"unterminated\nmode =");
        assert!(broken.theme.is_none() && broken.mode.is_none());
    }

    #[test]
    fn verdict_presets_parse_in_file_order() {
        let cfg = Config::parse(
            r#"
theme = "light"
[[verdict]]
name = "ship"
text = "send it"
[[verdict]]
name = "again"
text = "another pass please"
"#,
        );
        let v = cfg.verdicts();
        assert_eq!(v.len(), 2);
        assert_eq!(
            (v[0].name.as_str(), v[0].text.as_str()),
            ("ship", "send it")
        );
        assert_eq!(v[1].name, "again", "file order is preserved");
    }

    #[test]
    fn a_malformed_preset_does_not_cost_the_user_their_theme() {
        // The whole reason presets are read separately: one struct for the file
        // would discard `theme` and `mode` over a typo in a preset.
        let cfg = Config::parse(
            r#"
theme = "Nord"
mode = "split"
[[verdict]]
name = "ship"
body = "wrong key"
"#,
        );
        assert_eq!(cfg.theme.as_deref(), Some("Nord"), "theme survives");
        assert_eq!(cfg.layout_mode(), Some(LayoutMode::Split), "and layout");
        assert_eq!(
            cfg.verdicts().len(),
            DEFAULT_VERDICTS.len(),
            "only the presets fall back"
        );
    }

    #[test]
    fn no_configured_presets_means_the_built_ins() {
        // Empty is "unset", not "no way to close a round".
        for text in ["", "verdict = []"] {
            let names: Vec<String> = Config::parse(text)
                .verdicts()
                .into_iter()
                .map(|v| v.name)
                .collect();
            assert_eq!(names.len(), DEFAULT_VERDICTS.len(), "{text:?}");
            assert!(names.contains(&"approve".to_string()), "{names:?}");
        }
    }

    #[test]
    fn write_theme_preserves_other_keys_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# my prefs\nmode = \"split\"\ntheme = \"dark\"\n").unwrap();

        super::write_theme(&path, "Dracula").unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# my prefs"), "comment preserved");
        assert!(out.contains("mode = \"split\""), "other key preserved");
        assert!(out.contains("theme = \"Dracula\""), "theme updated");
    }

    #[test]
    fn write_theme_creates_file_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        super::write_theme(&path, "Nord").unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("theme = \"Nord\""));
    }

    #[test]
    fn write_theme_propagates_a_read_error_other_than_missing() {
        // The "path" is a directory, so reading it as a string fails with an
        // error whose kind is NOT NotFound → the `Err(e) => return Err(e)` arm.
        let dir = tempfile::tempdir().unwrap();
        let err = super::write_theme(dir.path(), "Dracula").unwrap_err();
        assert_ne!(
            err.kind(),
            io::ErrorKind::NotFound,
            "a non-missing read error is propagated, not treated as a fresh file"
        );
    }

    #[test]
    fn path_prefers_xdg_then_home_then_none() {
        // These env vars are read only by Config::path, which no other test
        // exercises; save and restore them around the mutation.
        let saved_xdg = std::env::var_os("XDG_CONFIG_HOME");
        let saved_home = std::env::var_os("HOME");

        std::env::set_var("XDG_CONFIG_HOME", "/xdg/conf");
        assert_eq!(
            Config::path(),
            Some(PathBuf::from("/xdg/conf/rediff/config.toml")),
            "XDG_CONFIG_HOME wins when set"
        );

        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::set_var("HOME", "/home/u");
        assert_eq!(
            Config::path(),
            Some(PathBuf::from("/home/u/.config/rediff/config.toml")),
            "falls back to HOME/.config"
        );

        std::env::remove_var("HOME");
        assert_eq!(Config::path(), None, "neither set → no path");

        match saved_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match saved_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn write_theme_refuses_to_clobber_a_malformed_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "# prefs\nmode = \"split\"\ntheme = \"dark\nbroken";
        std::fs::write(&path, original).unwrap();

        // A malformed existing file is an error, and the file is left untouched
        // rather than overwritten with a theme-only document.
        assert!(super::write_theme(&path, "Dracula").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}
