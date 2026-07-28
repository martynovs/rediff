//! Non-interactive pager: read a unified diff on stdin, render it with rediff's
//! themed tree-sitter/syntect highlighting, and write ANSI to stdout. Wired as a
//! git/lazygit `pager` (`git.pagers[].pager: rediff pager`) — it post-processes
//! git's diff rather than replacing it, so the underlying patch stays intact for
//! line/hunk staging.
//!
//! Unlike the interactive viewer, a pager only sees the patch's hunk fragments
//! (a few context lines), not whole files, so tree-sitter has less context than
//! the TUI; highlighting is best-effort at hunk edges.

use std::fmt::Write as _;
use std::io::{Read, Write};

use diffy::patch_set::{FileOperation, ParseOptions, PatchKind, PatchSet};
use diffy::{Hunk, Line, Patch};
use ratatui::style::Color;

use crate::highlight::{Engine, Highlight, Lines, Paint, Rgb, Span};
use crate::lang;
use crate::model::{FileStatus, LineKind};
use crate::tui::{Theme, ThemeName};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";

/// Read a unified diff from stdin and write a highlighted, themed, ANSI diff to
/// stdout. Color is emitted unconditionally — under git/lazygit stdout is a pipe,
/// not a TTY, so auto-detection would (wrongly) turn it off.
pub fn run(theme: ThemeName) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf)?;
    // A diff of text files is UTF-8; be lossy rather than fail on stray bytes.
    let input = String::from_utf8_lossy(&buf).into_owned();
    let out = render(&input, theme);
    std::io::stdout().lock().write_all(out.as_bytes())?;
    Ok(())
}

/// Render a whole `git diff` to ANSI. Forgiving: an unparseable file section is
/// skipped rather than aborting the rest of the diff.
fn render(input: &str, theme_name: ThemeName) -> String {
    let theme = Theme::new(theme_name);
    let syntax = theme_name.syntax_table();
    let ctx = theme.context_rgb();
    let engine = Engine::new();

    // lazygit (and `git diff` with `color.diff` on) pipes a *color-coded* diff to
    // the pager — `\x1b[..m diff --git ..`. diffy parses plain unified diff, so
    // strip ANSI first; otherwise nothing parses and the panel is blank.
    let input = strip_ansi_escapes::strip_str(input);

    let mut out = String::new();
    for fp in PatchSet::parse(&input, ParseOptions::gitdiff()) {
        let Ok(fp) = fp else { continue };
        let op = fp.operation().strip_prefix(1);
        let (path, prev, status) = describe(&op);
        if is_review_log(&path) {
            continue;
        }
        file_header(&mut out, &theme, &path, prev.as_deref(), status);

        match fp.patch() {
            PatchKind::Binary(_) => {
                line_plain(&mut out, &theme, &format!("Binary file {path} differs"));
            }
            PatchKind::Text(text_patch) => {
                let lang = lang::detect(&path);
                let (old_text, new_text) = side_texts(text_patch);
                let old_hl = highlight(&engine, &old_text, lang.as_deref(), theme_name);
                let new_hl = highlight(&engine, &new_text, lang.as_deref(), theme_name);

                let mut old_row = 0usize;
                let mut new_row = 0usize;
                for hunk in text_patch.hunks() {
                    hunk_header(&mut out, &theme, hunk);
                    for &line in hunk.lines() {
                        match line {
                            Line::Context(t) => {
                                emit(
                                    &mut out,
                                    &theme,
                                    &syntax,
                                    ctx,
                                    LineKind::Context,
                                    t,
                                    new_hl.get(new_row),
                                );
                                old_row += 1;
                                new_row += 1;
                            }
                            Line::Delete(t) => {
                                emit(
                                    &mut out,
                                    &theme,
                                    &syntax,
                                    ctx,
                                    LineKind::Removed,
                                    t,
                                    old_hl.get(old_row),
                                );
                                old_row += 1;
                            }
                            Line::Insert(t) => {
                                emit(
                                    &mut out,
                                    &theme,
                                    &syntax,
                                    ctx,
                                    LineKind::Added,
                                    t,
                                    new_hl.get(new_row),
                                );
                                new_row += 1;
                            }
                        }
                    }
                }
            }
        }
        out.push('\n');
    }
    out
}

/// Reconstruct the old-side and new-side text of a file from its hunk fragments,
/// so the highlighter can run over each side. Context lines belong to both.
fn side_texts(patch: &Patch<'_, str>) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    for hunk in patch.hunks() {
        for &line in hunk.lines() {
            match line {
                Line::Context(t) => {
                    push_line(&mut old, t);
                    push_line(&mut new, t);
                }
                Line::Delete(t) => push_line(&mut old, t),
                Line::Insert(t) => push_line(&mut new, t),
            }
        }
    }
    (old, new)
}

fn push_line(s: &mut String, t: &str) {
    s.push_str(t);
    if !t.ends_with('\n') {
        s.push('\n');
    }
}

fn highlight(engine: &Engine, text: &str, lang: Option<&str>, theme: ThemeName) -> Lines {
    if text.is_empty() {
        return Lines::new();
    }
    engine.highlight(text, lang, theme.embedded_name())
}

/// `GIT_EXTERNAL_DIFF` entry point. git invokes us per file as
/// `path old-file old-hex old-mode new-file new-hex new-mode`; we diff the two
/// whole files (full tree-sitter context) and write themed ANSI to stdout. Used as
/// a lazygit `externalDiffCommand`, which — unlike a pager — includes untracked
/// files in the combined view. Per-file hunk staging is unaffected (lazygit renders
/// its own diff for the staging view).
pub fn external(args: &[String], theme: ThemeName) -> anyhow::Result<()> {
    // git's positional layout: path, old-file, old-hex, old-mode, new-file, ...
    let old_path = args.get(1).map_or("/dev/null", String::as_str);
    let new_path = args.get(4).map_or("/dev/null", String::as_str);
    let path = display_path(args);

    // rediff's own review log is never review material. git invokes this per file
    // (that is the point of the lazygit `externalDiffCommand` setup), so without
    // this the untracked log shows up in the combined view and the reviewer ends up
    // reviewing rediff's record of their review.
    if is_review_log(path) {
        return Ok(());
    }

    // Classify on just the first few KB, so a large binary blob (e.g. an added
    // video) isn't slurped whole only to print a one-line notice; full-read text.
    let old_head = read_head(old_path, BINARY_SCAN);
    let new_head = read_head(new_path, BINARY_SCAN);
    let out = if is_binary(&old_head) || is_binary(&new_head) {
        render_external(path, &old_head, &new_head, theme)
    } else {
        render_external(path, &read_blob(old_path), &read_blob(new_path), theme)
    };
    std::io::stdout().lock().write_all(out.as_bytes())?;
    Ok(())
}

/// Whether a diff header's path names rediff's own review log at a repository
/// root.
///
/// The pager sees paths as git prints them — repo-relative, `/`-separated — so a
/// bare `rediff.jsonl` is the log and `sub/rediff.jsonl` is an ordinary file. The
/// loader applies the same rule in `git::filter::drop_review_log`; both exist
/// because the log is tool state, never review material.
fn is_review_log(path: &str) -> bool {
    path.trim_start_matches("./") == crate::review::LOG_FILE_NAME
}

/// Pick a human-readable path for the header. `arg[0]` is git's path, but for a
/// `--no-index` create (untracked files) it is `/dev/null`; the real name is then
/// in `arg[7]` (rename/dest) or `arg[4]` (new-file).
fn display_path(args: &[String]) -> &str {
    for i in [0usize, 7, 4, 1] {
        if let Some(p) = args.get(i) {
            if p != "/dev/null" && !p.is_empty() {
                return p;
            }
        }
    }
    args.first().map_or("", String::as_str)
}

/// Render one file's diff (whole old/new contents) to themed ANSI. Split from
/// `external` so it can be tested without touching the filesystem or stdout.
fn render_external(path: &str, old: &[u8], new: &[u8], theme_name: ThemeName) -> String {
    let theme = Theme::new(theme_name);
    let syntax = theme_name.syntax_table();
    let ctx = theme.context_rgb();

    let mut out = String::new();
    file_header(&mut out, &theme, path, None, file_status(old, new));

    if is_binary(old) || is_binary(new) {
        line_plain(&mut out, &theme, &format!("Binary file {path} differs"));
        out.push('\n');
        return out;
    }

    let old_cow = String::from_utf8_lossy(old);
    let new_cow = String::from_utf8_lossy(new);
    let old_text: &str = &old_cow;
    let new_text: &str = &new_cow;
    let (hunks, _, _) = crate::diff::compute_hunks(old_text, new_text);

    let engine = Engine::new();
    let lang = lang::detect(path);
    let old_hl = highlight(&engine, old_text, lang.as_deref(), theme_name);
    let new_hl = highlight(&engine, new_text, lang.as_deref(), theme_name);

    for h in &hunks {
        write_hunk_header(
            &mut out,
            &theme,
            h.old_start as usize,
            h.old_len as usize,
            h.new_start as usize,
            h.new_len as usize,
            None,
        );
        for line in &h.lines {
            // Full-file highlighting → real line numbers index directly into the
            // highlighted lines (no fragment bookkeeping like the pager needs).
            let spans = match line.kind {
                LineKind::Removed => line.old_lineno.and_then(|n| old_hl.get((n - 1) as usize)),
                LineKind::Added | LineKind::Context => {
                    line.new_lineno.and_then(|n| new_hl.get((n - 1) as usize))
                }
            };
            emit(&mut out, &theme, &syntax, ctx, line.kind, &line.text, spans);
        }
    }
    out.push('\n');
    out
}

/// Read a blob path git handed us; `/dev/null` (and unreadable paths) → empty.
fn read_blob(path: &str) -> Vec<u8> {
    if path == "/dev/null" {
        return Vec::new();
    }
    std::fs::read(path).unwrap_or_default()
}

/// Read at most `n` bytes from a blob path (enough to classify binary vs text
/// without loading a huge file); `/dev/null` and unreadable paths → empty.
fn read_head(path: &str, n: usize) -> Vec<u8> {
    if path == "/dev/null" {
        return Vec::new();
    }
    match std::fs::File::open(path) {
        Ok(f) => {
            let mut buf = Vec::new();
            #[expect(
                clippy::let_underscore_must_use,
                reason = "best-effort head read; on error we classify with whatever bytes buffered"
            )]
            let _ = f.take(n as u64).read_to_end(&mut buf);
            buf
        }
        Err(_) => Vec::new(),
    }
}

/// git scans the first 8000 bytes (`FIRST_FEW_BYTES`) for a NUL to decide binary.
const BINARY_SCAN: usize = 8000;

/// git's binary heuristic: a NUL byte within the first `BINARY_SCAN` bytes.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SCAN).any(|&b| b == 0)
}

/// Derive a status from which side is empty (one side is `/dev/null` for
/// create/delete; both present is a modification).
fn file_status(old: &[u8], new: &[u8]) -> FileStatus {
    match (old.is_empty(), new.is_empty()) {
        (true, false) => FileStatus::Added,
        (false, true) => FileStatus::Deleted,
        _ => FileStatus::Modified,
    }
}

/// Derive a clean path, an optional previous path, and a status from a (prefix-
/// stripped) diffy file operation.
fn describe(op: &FileOperation<'_, str>) -> (String, Option<String>, FileStatus) {
    match op {
        FileOperation::Create(p) => (p.to_string(), None, FileStatus::Added),
        FileOperation::Delete(p) => (p.to_string(), None, FileStatus::Deleted),
        FileOperation::Modify { original, modified } if original != modified => (
            modified.to_string(),
            Some(original.to_string()),
            FileStatus::Renamed,
        ),
        FileOperation::Modify { modified, .. } => {
            (modified.to_string(), None, FileStatus::Modified)
        }
        FileOperation::Rename { from, to } => {
            (to.to_string(), Some(from.to_string()), FileStatus::Renamed)
        }
        FileOperation::Copy { from, to } => {
            (to.to_string(), Some(from.to_string()), FileStatus::Copied)
        }
    }
}

fn file_header(
    out: &mut String,
    theme: &Theme,
    path: &str,
    prev: Option<&str>,
    status: FileStatus,
) {
    out.push('\n');
    fg(out, cr(theme.accent));
    out.push_str(BOLD);
    match prev {
        Some(p) if p != path => {
            out.push_str(p);
            out.push_str(" → ");
            out.push_str(path);
        }
        _ => out.push_str(path),
    }
    out.push_str(RESET);
    fg(out, cr(theme.muted));
    out.push_str("  (");
    out.push_str(status.label());
    out.push(')');
    out.push_str(RESET);
    out.push('\n');
}

fn hunk_header(out: &mut String, theme: &Theme, hunk: &Hunk<'_, str>) {
    let o = hunk.old_range();
    let n = hunk.new_range();
    write_hunk_header(
        out,
        theme,
        o.start(),
        o.len(),
        n.start(),
        n.len(),
        hunk.function_context(),
    );
}

/// Write a themed `@@ -a,b +c,d @@` hunk header (plus optional function context)
/// directly into `out` — no intermediate `String` per hunk.
fn write_hunk_header(
    out: &mut String,
    theme: &Theme,
    old_start: usize,
    old_len: usize,
    new_start: usize,
    new_len: usize,
    fctx: Option<&str>,
) {
    fg(out, cr(theme.hunk));
    #[expect(
        clippy::let_underscore_must_use,
        reason = "write! into a String is infallible"
    )]
    let _ = write!(out, "@@ -{old_start},{old_len} +{new_start},{new_len} @@");
    if let Some(c) = fctx {
        out.push(' ');
        out.push_str(trim_eol(c));
    }
    out.push_str(RESET);
    out.push('\n');
}

/// Emit one diff line: a `+`/`-`/space gutter and the line content, syntax-
/// highlighted from `spans` (falling back to the raw text in the theme's default
/// foreground), over an add/del/context background.
fn emit(
    out: &mut String,
    theme: &Theme,
    syntax: &[Rgb],
    ctx: Rgb,
    kind: LineKind,
    raw: &str,
    spans: Option<&Vec<Span>>,
) {
    let (gutter, gutter_fg, bg) = match kind {
        LineKind::Added => ('+', cr(theme.added), Some(cr(theme.add_bg))),
        LineKind::Removed => ('-', cr(theme.removed), Some(cr(theme.del_bg))),
        LineKind::Context => (' ', cr(theme.muted), None),
    };
    if let Some(bg) = bg {
        bg_seq(out, bg);
    }
    fg(out, gutter_fg);
    out.push(gutter);
    out.push(' ');
    match spans {
        Some(spans) if !spans.is_empty() => {
            for s in spans {
                fg(out, resolve(s.paint, syntax, ctx));
                out.push_str(trim_eol(&s.text));
            }
        }
        _ => {
            fg(out, ctx);
            out.push_str(trim_eol(raw));
        }
    }
    out.push_str(RESET);
    out.push('\n');
}

/// A non-diff informational line (e.g. a binary-file notice) in the muted color.
fn line_plain(out: &mut String, theme: &Theme, text: &str) {
    fg(out, cr(theme.muted));
    out.push_str(text);
    out.push_str(RESET);
    out.push('\n');
}

/// Resolve a highlight `Paint` to an RGB color — mirrors the TUI's resolution so
/// pager and viewer render the same content identically.
fn resolve(paint: Paint, syntax: &[Rgb], ctx: Rgb) -> Rgb {
    match paint {
        Paint::Capture(i) => syntax.get(i as usize).copied().unwrap_or(ctx),
        Paint::Default => ctx,
        Paint::Fixed(c) => c,
    }
}

/// Extract the RGB triple from a chrome `Color` (the theme builds them all as
/// `Color::Rgb`).
fn cr(c: Color) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (200, 200, 200),
    }
}

/// Strip a single trailing end-of-line (`\n` or `\r\n`) from line content. Other
/// trailing whitespace is preserved — it is meaningful in a diff.
fn trim_eol(s: &str) -> &str {
    let s = s.strip_suffix('\n').unwrap_or(s);
    s.strip_suffix('\r').unwrap_or(s)
}

/// Write an ANSI truecolor foreground escape into `out` (no per-call allocation).
fn fg(out: &mut String, (r, g, b): Rgb) {
    #[expect(
        clippy::let_underscore_must_use,
        reason = "write! into a String is infallible"
    )]
    let _ = write!(out, "\x1b[38;2;{r};{g};{b}m");
}

/// Write an ANSI truecolor background escape into `out`.
fn bg_seq(out: &mut String, (r, g, b): Rgb) {
    #[expect(
        clippy::let_underscore_must_use,
        reason = "write! into a String is infallible"
    )]
    let _ = write!(out, "\x1b[48;2;{r};{g};{b}m");
}

#[cfg(test)]
mod pager_tests;
