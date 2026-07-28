//! Off-thread highlight service: a worker thread builds the engine once and
//! highlights files on demand; the UI thread renders plain text until results
//! arrive in the cache, so input is never blocked on highlighting.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::highlight::{Engine, FileHighlight, Highlight};
use crate::model::DiffFile;
use crate::tui::theme::ThemeName;

struct Job {
    idx: usize,
    epoch: u32,
    old: Option<String>,
    new: Option<String>,
    lang: Option<String>,
    theme: two_face::theme::EmbeddedThemeName,
}

pub struct HlService {
    job_tx: Sender<Job>,
    res_rx: Receiver<(usize, u32, FileHighlight)>,
    cache: HashMap<usize, FileHighlight>,
    requested: HashSet<usize>,
    theme: ThemeName,
    /// Bumped on every view switch / appearance change; results tagged with a
    /// stale epoch are discarded so a late result can't paint the new view.
    epoch: u32,
    _worker: JoinHandle<()>,
}

impl HlService {
    pub fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (res_tx, res_rx) = mpsc::channel::<(usize, u32, FileHighlight)>();

        let worker = thread::spawn(move || {
            // Building the engine (grammars + syntaxes) happens here, off the UI thread.
            let engine = Engine::new();
            while let Ok(job) = job_rx.recv() {
                let lang = job.lang.as_deref();
                let dep = engine.theme_dependent(lang);
                let old = job
                    .old
                    .map(|t| engine.highlight(&t, lang, job.theme))
                    .unwrap_or_default();
                let new = job
                    .new
                    .map(|t| engine.highlight(&t, lang, job.theme))
                    .unwrap_or_default();
                let fh = FileHighlight {
                    old,
                    new,
                    theme_dependent: dep,
                };
                if res_tx.send((job.idx, job.epoch, fh)).is_err() {
                    break;
                }
            }
        });

        HlService {
            job_tx,
            res_rx,
            cache: HashMap::new(),
            requested: HashSet::new(),
            theme: ThemeName::default(),
            epoch: 0,
            _worker: worker,
        }
    }

    /// Switch the active theme. Tree-sitter and plain results are theme-
    /// independent (their spans carry capture indices resolved at render time),
    /// so they survive untouched — only theme-baked syntect results are dropped
    /// and re-highlighted, keeping live theme preview responsive.
    pub fn set_theme(&mut self, theme: ThemeName) {
        if theme == self.theme {
            return;
        }
        self.theme = theme;
        let drop: Vec<usize> = self
            .cache
            .iter()
            .filter(|(_, fh)| fh.theme_dependent)
            .map(|(&i, _)| i)
            .collect();
        for i in drop {
            self.cache.remove(&i);
        }
        // Bump the epoch so any in-flight job (built under the old theme) is
        // discarded on arrival, then re-request anything no longer cached.
        self.epoch = self.epoch.wrapping_add(1);
        self.requested.retain(|idx| self.cache.contains_key(idx));
    }

    /// Drop all cached/in-flight highlights and bump the epoch. Called on a view
    /// switch so the new view re-highlights and stale results are ignored.
    pub fn reset(&mut self, theme: ThemeName) {
        self.theme = theme;
        self.invalidate();
    }

    fn invalidate(&mut self) {
        self.cache.clear();
        self.requested.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Drop one index's cache + request flag so it re-highlights (used for the
    /// peek's reserved slot when its content changes).
    pub fn forget(&mut self, idx: usize) {
        self.cache.remove(&idx);
        self.requested.remove(&idx);
    }

    /// Whether `idx` still needs a highlight request.
    pub fn needs(&self, idx: usize) -> bool {
        !self.requested.contains(&idx) && !self.cache.contains_key(&idx)
    }

    /// Request highlighting for a file once. No-op if already requested/binary.
    pub fn request(&mut self, idx: usize, file: &DiffFile) {
        if file.is_binary || self.requested.contains(&idx) {
            return;
        }
        if file.old_text.is_none() && file.new_text.is_none() {
            return;
        }
        self.requested.insert(idx);
        #[expect(
            clippy::let_underscore_must_use,
            reason = "send error means the worker pool is gone; nothing to do"
        )]
        let _ = self.job_tx.send(Job {
            idx,
            epoch: self.epoch,
            old: file.old_text.clone(),
            new: file.new_text.clone(),
            lang: file.language.clone(),
            theme: self.theme.embedded_name(),
        });
    }

    /// Drain finished results into the cache, discarding stale-epoch results.
    /// Returns true if anything for the current epoch arrived.
    pub fn drain(&mut self) -> bool {
        let mut got = false;
        while let Ok((idx, epoch, fh)) = self.res_rx.try_recv() {
            if epoch != self.epoch {
                continue;
            }
            self.cache.insert(idx, fh);
            got = true;
        }
        got
    }

    pub fn get(&self, idx: usize) -> Option<&FileHighlight> {
        self.cache.get(&idx)
    }
}

impl Default for HlService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileStatus, Stats};

    fn file() -> DiffFile {
        DiffFile {
            path: "a.rs".into(),
            previous_path: None,
            status: FileStatus::Modified,
            staged: false,
            hunks: Vec::new(),
            stats: Stats::default(),
            language: Some("rust".into()),
            is_binary: false,
            old_text: Some("fn a() {}\n".into()),
            new_text: Some("fn b() {}\n".into()),
            content_digest: None,
            diffed: true,
        }
    }

    /// Seed `idx` directly into the cache. The cache-management methods
    /// (`set_theme`, `reset`) operate on cached entries; seeding keeps their
    /// tests deterministic instead of spinning up the worker thread and building
    /// a real `Engine` per test — which is slow and flaky under parallel CI load.
    /// The worker → `drain` → cache path is exercised by the PTY event-loop test.
    fn seed(hl: &mut HlService, idx: usize, theme_dependent: bool) {
        hl.cache.insert(
            idx,
            FileHighlight {
                theme_dependent,
                ..Default::default()
            },
        );
        hl.requested.insert(idx);
    }

    #[test]
    fn default_constructs_and_needs_tracks_requests() {
        let mut hl = HlService::default();
        assert!(hl.needs(0), "nothing requested or cached yet");
        let f = file();
        hl.request(0, &f);
        assert!(!hl.needs(0), "a requested index no longer needs a request");
        assert!(hl.needs(1), "an untouched index still needs one");
    }

    #[test]
    fn set_theme_same_theme_is_a_noop() {
        let mut hl = HlService::default();
        let before = hl.epoch;
        hl.set_theme(ThemeName::default());
        assert_eq!(
            hl.epoch, before,
            "re-setting the active theme changes nothing"
        );
    }

    #[test]
    fn set_theme_drops_theme_dependent_highlights_and_bumps_epoch() {
        let mut hl = HlService::default();
        // A theme-dependent (syntect) entry plus a theme-independent (tree-sitter)
        // one: only the former is dropped on a theme change.
        seed(&mut hl, 0, true);
        seed(&mut hl, 1, false);
        assert!(hl.get(0).is_some() && hl.get(1).is_some(), "both seeded");

        let before = hl.epoch;
        hl.set_theme(ThemeName::Light); // a different theme
        assert!(
            hl.get(0).is_none(),
            "theme-dependent (syntect) highlight dropped on theme change"
        );
        assert!(
            hl.get(1).is_some(),
            "theme-independent highlight survives the theme change"
        );
        assert_ne!(
            hl.epoch, before,
            "epoch bumped so in-flight jobs are discarded"
        );
    }

    #[test]
    fn reset_clears_cache_and_bumps_epoch() {
        let mut hl = HlService::new();
        seed(&mut hl, 0, false);
        assert!(hl.get(0).is_some(), "seeded highlight present");
        let before = hl.epoch;
        hl.reset(ThemeName::default());
        assert!(hl.get(0).is_none(), "cache dropped on view switch");
        assert_ne!(
            hl.epoch, before,
            "epoch bumped so stale results are discarded"
        );
    }
}
