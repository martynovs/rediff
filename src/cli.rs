//! CLI surface and its resolution into a repo directory, a `LoadRequest`, and
//! optional path filters.
//!
//! A positional argument is interpreted by what it is:
//! - contains `..` — a commit range (diff only)
//! - a repo-root directory — the repository to open (like `git -C`)
//! - anything else — a path filter (pathspec)
//!
//! `-C/--repo` always sets the repository directory explicitly.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::git::LoadRequest;

#[derive(Parser, Debug)]
#[command(name = "rediff", version, about = "A fast Rust TUI git-diff viewer")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Review working-tree changes (or a range with `a..b`).
    Diff {
        /// Review staged changes only (HEAD vs index).
        #[arg(long)]
        staged: bool,
        /// Exclude untracked files from a working-tree review.
        #[arg(long)]
        exclude_untracked: bool,
        /// Compare the working tree against this ref (e.g. a branch) instead of
        /// HEAD — shows your local state relative to it.
        #[arg(long = "from", value_name = "REF")]
        from: Option<String>,
        /// Repository directory to open (defaults to the current directory).
        #[arg(short = 'C', long = "repo", value_name = "DIR")]
        repo: Option<PathBuf>,
        /// Layout mode: auto, split, or stack.
        #[arg(long, value_name = "MODE")]
        mode: Option<String>,
        /// Theme: dark or light.
        #[arg(long, value_name = "THEME")]
        theme: Option<String>,
        /// A range (`old..new`), a repo directory, and/or path filters.
        #[arg(value_name = "RANGE|PATH")]
        targets: Vec<String>,
    },
    /// Review a commit or a branch range with viewed-tracking (default HEAD).
    Review {
        /// Start the range at this base (sha or branch); reviews `base..target`
        /// as one combined net diff. Omit to review a single commit.
        #[arg(long = "from", value_name = "BASE")]
        from: Option<String>,
        /// Repository directory to open (defaults to the current directory).
        #[arg(short = 'C', long = "repo", value_name = "DIR")]
        repo: Option<PathBuf>,
        /// Layout mode: auto, split, or stack.
        #[arg(long, value_name = "MODE")]
        mode: Option<String>,
        /// Theme: dark or light.
        #[arg(long, value_name = "THEME")]
        theme: Option<String>,
        /// The target commit/ref (defaults to HEAD) and/or path filters.
        #[arg(value_name = "SHA|PATH")]
        targets: Vec<String>,
    },
    /// Render a unified diff read from stdin to themed, highlighted ANSI — a
    /// non-interactive git/lazygit pager (`git.pagers[].pager: rediff pager`).
    /// Post-processes git's diff, so line/hunk staging keeps working.
    Pager {
        /// Theme: `dark`, `light`, or an exact theme name.
        #[arg(long, value_name = "THEME")]
        theme: Option<String>,
    },
    /// `GIT_EXTERNAL_DIFF` per-file renderer: git calls this with
    /// `path old-file old-hex old-mode new-file new-hex new-mode`. Diffs the two
    /// whole files and writes themed, highlighted ANSI. For use as a lazygit
    /// `externalDiffCommand` — shows untracked files in the combined view, with
    /// full-file context (better highlighting than `pager`).
    External {
        /// Theme: `dark`, `light`, or an exact theme name.
        #[arg(long, value_name = "THEME")]
        theme: Option<String>,
        /// git's positional args: path old-file old-hex old-mode new-file new-hex
        /// new-mode (plus an 8th dest path for `--no-index` creates).
        #[arg(
            value_name = "ARG",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<String>,
    },
    /// Open a review over the current changes and exit — the non-interactive
    /// request an agent makes for a human review. Writes `rediff.jsonl` at the
    /// worktree root; read the result back with `rediff feedback`.
    Request {
        /// Review staged changes only (HEAD vs index).
        #[arg(long)]
        staged: bool,
        /// Exclude untracked files from a working-tree review.
        #[arg(long)]
        exclude_untracked: bool,
        /// Review everything since this ref — commits *and* uncommitted work — as
        /// one changeset. (Note this is `diff --from`, not `review --from`: it
        /// ends at the working tree, not at another ref.)
        #[arg(long = "from", value_name = "REF")]
        from: Option<String>,
        /// Repository directory to open (defaults to the current directory).
        #[arg(short = 'C', long = "repo", value_name = "DIR")]
        repo: Option<PathBuf>,
        /// Human-facing label, e.g. which agent opened the review.
        #[arg(long, value_name = "TEXT")]
        label: Option<String>,
        /// A range (`old..new`) and/or a repo directory. Path filters are not
        /// accepted: a review covers everything that changed under its target.
        #[arg(value_name = "RANGE|DIR")]
        targets: Vec<String>,
    },
    /// Report the worktree's review state (open review, round, pending feedback).
    ReviewStatus {
        /// Repository directory to open (defaults to the current directory).
        #[arg(short = 'C', long = "repo", value_name = "DIR")]
        repo: Option<PathBuf>,
        /// Emit JSON instead of a human-readable summary.
        #[arg(long)]
        json: bool,
    },
    /// Drain the review's feedback as JSON — the agent's read side.
    Feedback {
        /// Repository directory to open (defaults to the current directory).
        #[arg(short = 'C', long = "repo", value_name = "DIR")]
        repo: Option<PathBuf>,
        /// Replay every item with its delivery status instead of draining the
        /// undelivered ones. Records nothing.
        #[arg(long)]
        all: bool,
    },
    /// Review the changes introduced by a commit (default HEAD).
    Show {
        /// Repository directory to open (defaults to the current directory).
        #[arg(short = 'C', long = "repo", value_name = "DIR")]
        repo: Option<PathBuf>,
        /// Layout mode: auto, split, or stack.
        #[arg(long, value_name = "MODE")]
        mode: Option<String>,
        /// Theme: dark or light.
        #[arg(long, value_name = "THEME")]
        theme: Option<String>,
        /// A commit/ref and/or path filters (e.g. `HEAD~1 src/`).
        #[arg(value_name = "REV|PATH")]
        targets: Vec<String>,
    },
}

/// A fully resolved invocation. `mode`/`theme` are the CLI-flag overrides (None
/// means "fall back to config/default").
#[derive(Debug)]
pub struct Resolved {
    pub repo_dir: PathBuf,
    pub req: LoadRequest,
    pub filters: Vec<String>,
    pub mode: Option<String>,
    pub theme: Option<String>,
    /// Whether the launch view is a review session (viewed-tracking on). True for
    /// working-tree, staged, range, and `review`; false for `show`.
    pub review: bool,
}

/// The flags every repo-opening subcommand shares, borrowed for resolution.
///
/// Grouping them keeps each `resolve_*` helper under the argument-count lint and,
/// more usefully, lets the two target-scanning loops be shared rather than
/// re-derived per subcommand.
struct Common<'a> {
    repo: &'a Option<PathBuf>,
    mode: &'a Option<String>,
    theme: &'a Option<String>,
    targets: &'a [String],
}

impl Common<'_> {
    /// The repository directory before positional arguments are considered.
    fn seed_repo_dir(&self, cwd: &Path) -> PathBuf {
        self.repo.clone().unwrap_or_else(|| cwd.to_path_buf())
    }
}

/// Scan positionals for `review`/`show`: a repo root, then the first argument
/// that is not an existing path (a ref/sha), then path filters.
fn scan_rev_targets(c: &Common, cwd: &Path) -> (PathBuf, Option<String>, Vec<String>) {
    let mut repo_dir = c.seed_repo_dir(cwd);
    let mut rev: Option<String> = None;
    let mut filters = Vec::new();
    for t in c.targets {
        if c.repo.is_none() && is_repo_root(t) {
            repo_dir = PathBuf::from(t);
        } else if rev.is_none() && !Path::new(t).exists() {
            // A ref/sha doesn't exist as a filesystem path.
            rev = Some(t.clone());
        } else {
            filters.push(t.clone());
        }
    }
    (repo_dir, rev, filters)
}

/// Scan positionals for `diff`: a repo root, an `a..b` range, then path filters.
fn scan_diff_targets(c: &Common, cwd: &Path) -> (PathBuf, Option<String>, Vec<String>) {
    let mut repo_dir = c.seed_repo_dir(cwd);
    let mut range: Option<String> = None;
    let mut filters = Vec::new();
    for t in c.targets {
        if t.contains("..") {
            range = Some(t.clone());
        } else if c.repo.is_none() && is_repo_root(t) {
            repo_dir = PathBuf::from(t);
        } else {
            filters.push(t.clone());
        }
    }
    (repo_dir, range, filters)
}

/// `rediff` with no subcommand: the working tree, untracked included.
fn resolve_default(cwd: &Path) -> Resolved {
    Resolved {
        repo_dir: cwd.to_path_buf(),
        req: LoadRequest::WorkingTree {
            include_untracked: true,
            base: None,
        },
        filters: Vec::new(),
        mode: None,
        theme: None,
        review: true,
    }
}

fn resolve_diff(
    cwd: &Path,
    c: &Common,
    staged: bool,
    exclude_untracked: bool,
    from: Option<&str>,
) -> Resolved {
    let (repo_dir, range, filters) = scan_diff_targets(c, cwd);
    let req = match (range, staged) {
        (Some(r), _) => {
            let (old, new) = split_range(&r);
            LoadRequest::Range { old, new }
        }
        (None, true) => LoadRequest::Staged,
        (None, false) => LoadRequest::WorkingTree {
            include_untracked: !exclude_untracked,
            base: from.map(ToString::to_string),
        },
    };
    Resolved {
        repo_dir,
        req,
        filters,
        mode: c.mode.clone(),
        theme: c.theme.clone(),
        review: true,
    }
}

fn resolve_review(cwd: &Path, c: &Common, from: Option<&str>) -> Resolved {
    let (repo_dir, target, filters) = scan_rev_targets(c, cwd);
    let target = target.unwrap_or_else(|| "HEAD".to_string());
    let req = match from {
        Some(base) => LoadRequest::ReviewRange {
            base: base.to_string(),
            target,
        },
        None => LoadRequest::Show { rev: target },
    };
    Resolved {
        repo_dir,
        req,
        filters,
        mode: c.mode.clone(),
        theme: c.theme.clone(),
        review: true,
    }
}

/// `request` accepts every target the viewer does, and **no** path filters — which
/// is what makes its scan unambiguous: a positional that is neither a range nor a
/// repo root can only be a revision. Reusing `resolve_diff` here classified
/// `rediff request HEAD~1` as a filter and rejected it, so `show:<rev>` was a
/// target the codec supported but no invocation could produce.
fn resolve_request(
    cwd: &Path,
    c: &Common,
    staged: bool,
    exclude_untracked: bool,
    from: Option<&str>,
) -> Resolved {
    let mut repo_dir = c.seed_repo_dir(cwd);
    let mut range: Option<String> = None;
    let mut rev: Option<String> = None;
    let mut filters = Vec::new();
    for t in c.targets {
        if t.contains("..") {
            range = Some(t.clone());
        } else if c.repo.is_none() && is_repo_root(t) {
            repo_dir = PathBuf::from(t);
        } else if rev.is_none() && !t.ends_with('/') && !Path::new(t).exists() {
            // A ref/sha doesn't exist as a filesystem path — the same heuristic
            // `show`/`review` use — and cannot end in `/`, which git forbids in a
            // ref name. Anything else is a path the caller meant as a filter, and
            // is collected so the command can reject it with a message about
            // filters rather than an opaque git revision error.
            rev = Some(t.clone());
        } else {
            filters.push(t.clone());
        }
    }
    let req = match (range, rev, staged) {
        (Some(r), _, _) => {
            let (old, new) = split_range(&r);
            LoadRequest::Range { old, new }
        }
        (None, Some(rev), _) => LoadRequest::Show { rev },
        (None, None, true) => LoadRequest::Staged,
        (None, None, false) => LoadRequest::WorkingTree {
            include_untracked: !exclude_untracked,
            base: from.map(ToString::to_string),
        },
    };
    Resolved {
        repo_dir,
        req,
        filters,
        mode: None,
        theme: None,
        review: true,
    }
}

fn resolve_show(cwd: &Path, c: &Common) -> Resolved {
    let (repo_dir, rev, filters) = scan_rev_targets(c, cwd);
    Resolved {
        repo_dir,
        req: LoadRequest::Show {
            rev: rev.unwrap_or_else(|| "HEAD".to_string()),
        },
        filters,
        mode: c.mode.clone(),
        theme: c.theme.clone(),
        review: false,
    }
}

impl Cli {
    /// Resolve the parsed CLI against the current working directory.
    ///
    /// A dispatcher only: each subcommand's rules live in its own `resolve_*`
    /// helper. Keeping this flat would be more readable in isolation, but the
    /// combined branch count runs into the CRAP gate as subcommands are added.
    pub fn resolve(&self, cwd: &Path) -> Resolved {
        match &self.command {
            None => resolve_default(cwd),
            // `pager` is a non-interactive stdin→stdout filter handled in `main`
            // before `resolve` is ever called; it has no repo/view to resolve.
            #[expect(
                clippy::unreachable,
                reason = "pager is dispatched in main before resolve()"
            )]
            Some(Command::Pager { .. }) => unreachable!("pager is handled before resolve()"),
            #[expect(
                clippy::unreachable,
                reason = "external is dispatched in main before resolve()"
            )]
            Some(Command::External { .. }) => unreachable!("external is handled before resolve()"),
            Some(Command::Diff {
                staged,
                exclude_untracked,
                from,
                repo,
                mode,
                theme,
                targets,
            }) => resolve_diff(
                cwd,
                &Common {
                    repo,
                    mode,
                    theme,
                    targets,
                },
                *staged,
                *exclude_untracked,
                from.as_deref(),
            ),
            Some(Command::Review {
                from,
                repo,
                mode,
                theme,
                targets,
            }) => resolve_review(
                cwd,
                &Common {
                    repo,
                    mode,
                    theme,
                    targets,
                },
                from.as_deref(),
            ),
            // `--from` here means `WorkingTree { base }` (as on `diff`), not the
            // `ReviewRange` that `review --from` builds.
            Some(Command::Request {
                staged,
                exclude_untracked,
                from,
                repo,
                targets,
                ..
            }) => {
                let none = None;
                resolve_request(
                    cwd,
                    &Common {
                        repo,
                        mode: &none,
                        theme: &none,
                        targets,
                    },
                    *staged,
                    *exclude_untracked,
                    from.as_deref(),
                )
            }
            // Neither takes a target: both are dispatched in `main` before
            // `resolve` runs, so unlike `pager`/`external` there is nothing here
            // that could be reached.
            #[expect(
                clippy::unreachable,
                reason = "review-status is dispatched in main before resolve()"
            )]
            Some(Command::ReviewStatus { .. }) => {
                unreachable!("review-status is handled before resolve()")
            }
            #[expect(
                clippy::unreachable,
                reason = "feedback is dispatched in main before resolve()"
            )]
            Some(Command::Feedback { .. }) => unreachable!("feedback is handled before resolve()"),
            Some(Command::Show {
                repo,
                mode,
                theme,
                targets,
            }) => resolve_show(
                cwd,
                &Common {
                    repo,
                    mode,
                    theme,
                    targets,
                },
            ),
        }
    }
}

/// True when `p` is a directory that is a git repository root (has a `.git`).
fn is_repo_root(p: &str) -> bool {
    let path = Path::new(p);
    path.is_dir() && path.join(".git").exists()
}

/// Split an `old..new` range, defaulting either side to HEAD when omitted.
fn split_range(range: &str) -> (String, String) {
    match range.split_once("..") {
        Some((a, b)) => {
            let old = if a.is_empty() {
                "HEAD".to_string()
            } else {
                a.to_string()
            };
            let new = if b.is_empty() {
                "HEAD".to_string()
            } else {
                b.to_string()
            };
            (old, new)
        }
        None => (format!("{range}^"), range.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This crate's own directory: a real git repo root, used to exercise the
    /// `is_repo_root` positional-as-repo branch.
    fn repo_root() -> &'static str {
        env!("CARGO_MANIFEST_DIR")
    }

    fn diff(targets: &[&str]) -> Command {
        Command::Diff {
            staged: false,
            exclude_untracked: false,
            from: None,
            repo: None,
            mode: None,
            theme: None,
            targets: targets.iter().map(ToString::to_string).collect(),
        }
    }

    // --- split_range ----------------------------------------------------

    #[test]
    fn request_resolves_every_target_it_accepts() {
        let cwd = std::path::Path::new(".");
        let r = |targets: &[&str], staged, from: Option<&str>| {
            let owned: Vec<String> = targets.iter().map(|s| (*s).to_string()).collect();
            let no_repo: Option<PathBuf> = None;
            let no_str: Option<String> = None;
            resolve_request(
                cwd,
                &Common {
                    repo: &no_repo,
                    mode: &no_str,
                    theme: &no_str,
                    targets: &owned,
                },
                staged,
                false,
                from,
            )
        };

        // A bare rev is a target, not a filter — the bug this guards.
        assert_eq!(
            r(&["HEAD~1"], false, None).req,
            LoadRequest::Show {
                rev: "HEAD~1".into()
            }
        );
        assert_eq!(
            r(&["main..HEAD"], false, None).req,
            LoadRequest::Range {
                old: "main".into(),
                new: "HEAD".into()
            }
        );
        assert_eq!(r(&[], true, None).req, LoadRequest::Staged);
        assert_eq!(
            r(&[], false, Some("main")).req,
            LoadRequest::WorkingTree {
                include_untracked: true,
                base: Some("main".into())
            }
        );
        assert_eq!(
            r(&[], false, None).req,
            LoadRequest::WorkingTree {
                include_untracked: true,
                base: None
            }
        );

        // A trailing slash can never be a ref, and an existing path is obviously
        // not one either: both are collected so the command can reject them with a
        // message about filters rather than an opaque git error.
        assert_eq!(r(&["src/"], false, None).filters, vec!["src/"]);
        assert_eq!(r(&["Cargo.toml"], false, None).filters, vec!["Cargo.toml"]);

        // A second rev-looking positional is a filter too, not a silent override.
        let two = r(&["HEAD", "HEAD~1"], false, None);
        assert_eq!(two.req, LoadRequest::Show { rev: "HEAD".into() });
        assert_eq!(two.filters, vec!["HEAD~1"]);
    }

    #[test]
    fn split_range_old_and_new() {
        assert_eq!(
            split_range("old..new"),
            ("old".to_string(), "new".to_string())
        );
    }

    #[test]
    fn split_range_defaults_empty_sides_to_head() {
        assert_eq!(
            split_range("..new"),
            ("HEAD".to_string(), "new".to_string())
        );
        assert_eq!(
            split_range("old.."),
            ("old".to_string(), "HEAD".to_string())
        );
        assert_eq!(split_range(".."), ("HEAD".to_string(), "HEAD".to_string()));
    }

    #[test]
    fn split_range_without_dots_uses_parent() {
        assert_eq!(
            split_range("abc123"),
            ("abc123^".to_string(), "abc123".to_string())
        );
    }

    // --- is_repo_root ---------------------------------------------------

    #[test]
    fn is_repo_root_true_for_this_repo() {
        assert!(is_repo_root(repo_root()));
    }

    #[test]
    fn is_repo_root_false_for_non_repo() {
        assert!(!is_repo_root("/no/such/directory/anywhere"));
        // An existing dir without a `.git` is not a repo root.
        assert!(!is_repo_root("/"));
    }

    // --- resolve: no subcommand ----------------------------------------

    #[test]
    fn resolve_none_is_default_working_tree_review() {
        let cli = Cli { command: None };
        let r = cli.resolve(Path::new("/work/dir"));
        assert_eq!(r.repo_dir, PathBuf::from("/work/dir"));
        assert!(matches!(
            r.req,
            LoadRequest::WorkingTree {
                include_untracked: true,
                base: None
            }
        ));
        assert!(r.filters.is_empty());
        assert_eq!(r.mode, None);
        assert_eq!(r.theme, None);
        assert!(r.review);
    }

    // --- resolve: Diff --------------------------------------------------

    #[test]
    fn resolve_diff_range_target() {
        let cli = Cli {
            command: Some(diff(&["a..b"])),
        };
        let r = cli.resolve(Path::new("."));
        assert!(matches!(
            r.req,
            LoadRequest::Range { old, new } if old == "a" && new == "b"
        ));
        assert!(r.filters.is_empty());
        assert!(r.review);
    }

    #[test]
    fn resolve_diff_staged() {
        let cli = Cli {
            command: Some(Command::Diff {
                staged: true,
                exclude_untracked: false,
                from: None,
                repo: None,
                mode: None,
                theme: None,
                targets: Vec::new(),
            }),
        };
        let r = cli.resolve(Path::new("."));
        assert!(matches!(r.req, LoadRequest::Staged));
    }

    #[test]
    fn resolve_diff_plain_working_tree() {
        let cli = Cli {
            command: Some(Command::Diff {
                staged: false,
                exclude_untracked: true,
                from: Some("main".to_string()),
                repo: None,
                mode: Some("split".to_string()),
                theme: Some("dark".to_string()),
                targets: Vec::new(),
            }),
        };
        let r = cli.resolve(Path::new("/cwd"));
        assert_eq!(r.repo_dir, PathBuf::from("/cwd"));
        assert!(matches!(
            r.req,
            LoadRequest::WorkingTree {
                include_untracked: false,
                base
            } if base.as_deref() == Some("main")
        ));
        assert_eq!(r.mode.as_deref(), Some("split"));
        assert_eq!(r.theme.as_deref(), Some("dark"));
    }

    #[test]
    fn resolve_diff_path_filter_target() {
        let cli = Cli {
            command: Some(diff(&["some_pathspec_name"])),
        };
        let r = cli.resolve(Path::new("."));
        assert_eq!(r.filters, vec!["some_pathspec_name".to_string()]);
        assert!(matches!(r.req, LoadRequest::WorkingTree { .. }));
    }

    #[test]
    fn resolve_diff_repo_dir_positional() {
        let cli = Cli {
            command: Some(diff(&[repo_root()])),
        };
        let r = cli.resolve(Path::new("."));
        assert_eq!(r.repo_dir, PathBuf::from(repo_root()));
        assert!(r.filters.is_empty());
    }

    // --- resolve: Review ------------------------------------------------

    #[test]
    fn resolve_review_from_is_review_range() {
        let cli = Cli {
            command: Some(Command::Review {
                from: Some("main".to_string()),
                repo: None,
                mode: None,
                theme: None,
                targets: vec!["feature_branch_xyz".to_string()],
            }),
        };
        let r = cli.resolve(Path::new("."));
        assert!(matches!(
            r.req,
            LoadRequest::ReviewRange { base, target }
                if base == "main" && target == "feature_branch_xyz"
        ));
        assert!(r.review);
    }

    #[test]
    fn resolve_review_no_from_defaults_head_show() {
        let cli = Cli {
            command: Some(Command::Review {
                from: None,
                repo: None,
                mode: None,
                theme: None,
                targets: Vec::new(),
            }),
        };
        let r = cli.resolve(Path::new("."));
        assert!(matches!(r.req, LoadRequest::Show { rev } if rev == "HEAD"));
    }

    #[test]
    fn resolve_review_explicit_target() {
        let cli = Cli {
            command: Some(Command::Review {
                from: None,
                repo: None,
                mode: None,
                theme: None,
                targets: vec!["HEAD~2".to_string()],
            }),
        };
        let r = cli.resolve(Path::new("."));
        assert!(matches!(r.req, LoadRequest::Show { rev } if rev == "HEAD~2"));
    }

    #[test]
    fn resolve_review_existing_path_is_filter() {
        // `src` exists on disk (tests run from the crate root), so it is treated
        // as a path filter, not a target — target falls back to HEAD.
        let cli = Cli {
            command: Some(Command::Review {
                from: None,
                repo: None,
                mode: None,
                theme: None,
                targets: vec!["src".to_string()],
            }),
        };
        let r = cli.resolve(Path::new("."));
        assert_eq!(r.filters, vec!["src".to_string()]);
        assert!(matches!(r.req, LoadRequest::Show { rev } if rev == "HEAD"));
    }

    #[test]
    fn resolve_review_repo_dir_positional() {
        let cli = Cli {
            command: Some(Command::Review {
                from: None,
                repo: None,
                mode: None,
                theme: None,
                targets: vec![repo_root().to_string()],
            }),
        };
        let r = cli.resolve(Path::new("."));
        assert_eq!(r.repo_dir, PathBuf::from(repo_root()));
        assert!(matches!(r.req, LoadRequest::Show { rev } if rev == "HEAD"));
    }

    // --- resolve: Show --------------------------------------------------

    #[test]
    fn resolve_show_default_head_not_review() {
        let cli = Cli {
            command: Some(Command::Show {
                repo: None,
                mode: None,
                theme: None,
                targets: Vec::new(),
            }),
        };
        let r = cli.resolve(Path::new("/cwd"));
        assert_eq!(r.repo_dir, PathBuf::from("/cwd"));
        assert!(matches!(r.req, LoadRequest::Show { rev } if rev == "HEAD"));
        assert!(!r.review);
    }

    #[test]
    fn resolve_show_explicit_rev_and_path_filter() {
        let cli = Cli {
            command: Some(Command::Show {
                repo: None,
                mode: None,
                theme: None,
                targets: vec!["HEAD~1".to_string(), "src".to_string()],
            }),
        };
        let r = cli.resolve(Path::new("."));
        assert!(matches!(r.req, LoadRequest::Show { rev } if rev == "HEAD~1"));
        assert_eq!(r.filters, vec!["src".to_string()]);
    }

    #[test]
    fn resolve_show_repo_dir_positional() {
        let cli = Cli {
            command: Some(Command::Show {
                repo: None,
                mode: None,
                theme: None,
                targets: vec![repo_root().to_string()],
            }),
        };
        let r = cli.resolve(Path::new("."));
        assert_eq!(r.repo_dir, PathBuf::from(repo_root()));
    }

    #[test]
    fn resolve_explicit_repo_flag_wins_over_positional() {
        // With `-C` set, a positional repo-root is not re-interpreted as the
        // repo; it falls through to the filter/target rules.
        let cli = Cli {
            command: Some(Command::Diff {
                staged: false,
                exclude_untracked: false,
                from: None,
                repo: Some(PathBuf::from("/explicit/repo")),
                mode: None,
                theme: None,
                targets: vec![repo_root().to_string()],
            }),
        };
        let r = cli.resolve(Path::new("."));
        assert_eq!(r.repo_dir, PathBuf::from("/explicit/repo"));
        // The positional repo-root path exists on disk, so for Diff it is a
        // filter (Diff only checks `..`/repo-root, everything else is a filter).
        assert_eq!(r.filters, vec![repo_root().to_string()]);
    }
}
