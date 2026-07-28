//! rediff — a fast Rust TUI git-diff viewer.
//!
//! Launches the interactive TUI when stdout is a terminal; otherwise prints
//! the changeset as unified diff text (so pipes and redirects still work).

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use clap::Parser;

use rediff::cli::{Cli, Command};
use rediff::config::{self, Config};
use rediff::model::LayoutMode;
use rediff::tui::{ThemeName, ViewKind};
use rediff::{git, pager, render, reviewcli, tui};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // pager/external are non-interactive stdin→stdout filters (git/lazygit
    // pagers): they never open a repo or the TUI, so handle them first.
    if let Some(result) = run_filter_command(&cli) {
        return result;
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // `review-status` and `feedback` need a repository but no target — `feedback`
    // derives its own from the log — so there is nothing for `resolve` to return
    // for them. They dispatch here, before it runs.
    if let Some(result) = run_stateless_review_command(&cli, &cwd) {
        return result;
    }

    let resolved = cli.resolve(&cwd);

    // `request` does need a resolved target, so it dispatches after `resolve` and
    // before the terminal check — it is non-interactive either way.
    if let Some(Command::Request { label, .. }) = &cli.command {
        return reviewcli::run_request(
            &resolved.repo_dir,
            &resolved.req,
            &resolved.filters,
            label.as_deref(),
        );
    }
    let cfg = Config::load();

    // Precedence: CLI flag > config file > pick by terminal width at startup
    // (`None` defers that choice to `tui::run`).
    let mode: Option<LayoutMode> = resolved
        .mode
        .as_deref()
        .and_then(config::parse_mode)
        .or_else(|| cfg.layout_mode());
    let theme = resolved
        .theme
        .as_deref()
        .or(cfg.theme.as_deref())
        .map(ThemeName::parse)
        .unwrap_or_default();

    if std::io::stdout().is_terminal() {
        // The TUI enumerates the file list instantly and streams the diffs in;
        // no synchronous full load up front.
        let (kind, base) = ViewKind::launch_for(&resolved.req);
        tui::run(
            &resolved.req,
            &resolved.filters,
            mode,
            theme,
            resolved.repo_dir.clone(),
            kind,
            resolved.review,
            base,
        )?;
    } else {
        // Pipes/redirects get the full diff synchronously.
        let mut changeset = git::load(&resolved.repo_dir, &resolved.req)?;
        git::apply_path_filter(&mut changeset, &resolved.filters);
        // `write_all`, not `print!`: the latter panics on a broken pipe, so
        // `rediff diff | head` would abort with exit 101 instead of ending.
        std::io::Write::write_all(
            &mut std::io::stdout().lock(),
            render::to_unified_string(&changeset).as_bytes(),
        )?;
    }
    Ok(())
}

/// Dispatch the review commands that need a repository but no resolved target.
///
/// Kept out of [`run_filter_command`], whose contract is "never opens a repo", and
/// run before `Cli::resolve` because neither command has a target for it to
/// produce.
fn run_stateless_review_command(cli: &Cli, cwd: &Path) -> Option<anyhow::Result<()>> {
    let dir = |repo: &Option<PathBuf>| repo.clone().unwrap_or_else(|| cwd.to_path_buf());
    match &cli.command {
        Some(Command::ReviewStatus { repo, json }) => {
            Some(reviewcli::run_status(&dir(repo), *json))
        }
        Some(Command::Feedback { repo, all }) => Some(reviewcli::run_feedback(&dir(repo), *all)),
        _ => None,
    }
}

/// Dispatch the non-interactive filter subcommands (`pager`, `external`).
/// Returns `Some(result)` when `cli` selected one — so `main` returns it
/// immediately — or `None` for the normal repo/TUI path.
fn run_filter_command(cli: &Cli) -> Option<anyhow::Result<()>> {
    match &cli.command {
        // `pager` is a stdin→stdout filter (a git/lazygit pager).
        Some(Command::Pager { theme }) => Some(pager::run(filter_theme(theme.as_deref()))),
        // `external` is the GIT_EXTERNAL_DIFF per-file renderer.
        Some(Command::External { theme, args }) => {
            Some(pager::external(args, filter_theme(theme.as_deref())))
        }
        _ => None,
    }
}

/// Resolve a filter command's theme: explicit `--theme` flag, else the config
/// file, else the default.
fn filter_theme(flag: Option<&str>) -> ThemeName {
    let cfg = Config::load();
    flag.or(cfg.theme.as_deref())
        .map(ThemeName::parse)
        .unwrap_or_default()
}
