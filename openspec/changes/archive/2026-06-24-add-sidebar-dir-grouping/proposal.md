## Why

The sidebar is a flat list: one row per file, in whatever order git enumeration
yields. For a changeset that touches many files across several directories that
order is hard to scan — and git's order is not even reliably grouped by
directory (a directory's own files interleave with its subdirectories'
unpredictably). Grouping the list by directory — a dim directory line followed
by its files — gives the reader orientation ("everything under `src/tui` is
here"), and a hotkey lets them flip back to the flat view when they want a plain
list.

The same gap shows up structurally: the sidebar has no row model at all — the
visible row index *is* the file index, and `selected`, the click hit-test, the
`1–9` jump digits, and the window math all assume one file per row. Inserting
directory headers needs the sidebar to grow the small row abstraction the diff
body already has (its `Plan`/`Row`).

## What Changes

- **Stable path ordering at load (a global reorder).** Enumeration sorts the
  changed files once, by `(parent directory, file name)`, before the streaming
  diff begins (we already collect every stub up front, so this is index-aligned
  with the load and resume machinery). This sets the canonical file order, so it
  reorders the **diff body** and the **non-TUI text dump** too — not just the
  sidebar — in both flat and grouped modes: files appear in path order instead of
  git's status/enumeration order everywhere. This is a deliberate, accepted
  change (a stable path order is more predictable, and it makes grouping a pure
  header-insert). Root files (empty parent) sort first, so `./` is at the top.
- **A sidebar row model.** The sidebar builds `SidebarRow = Dir(path) | File(idx)`
  from the files and the active grouping mode — `Flat` yields the plain list,
  `ByDir` inserts a directory line whenever the parent directory changes. This
  mirrors the diff body's `Plan::build(cs, viewed, layout)`.
- **Grouped rendering.** A directory line shows the file's combined parent path,
  shortened to fit (reusing the existing path-abbreviation), styled dim/muted
  ("dark line"); top-level files sit under a `./` line. In grouped mode each
  file row shows just its basename (shortened if long), since the directory is
  in the header; flat mode keeps the shortened full path.
- **Directories are informative only.** Selection, `j`/`k`, and the `1–9` jump
  digits operate on files; directory lines are never selected, carry no digit,
  and are skipped by navigation. `selected` stays a file index.
- **A toggle hotkey.** `D` cycles the sidebar grouping (flat ⇄ grouped),
  app-global like the layout toggle (`g` was taken by go-to-top), defined in the keymap and documented in the
  `?` help.

This touches only file ordering and the sidebar; the diff algorithm, the diff
body rendering, reviewed-tracking, and the load/resume machinery are unchanged
(the streaming loader installs by index either way).

## Capabilities

### New Capabilities
- `sidebar-grouping`: a toggleable directory-grouped view of the sidebar file
  list — dim directory lines (combined, shortened paths; `./` for root) above
  their files, with selection and jump navigation operating on files only.

### Modified Capabilities
- `changeset-loading`: the changeset's files are ordered by `(parent directory,
  file name)`, so files in a directory are contiguous and the order is stable
  and predictable rather than git's enumeration order.
- `navigation`: the sidebar file list is shown in path order and can be grouped
  by directory; file selection and the jump digits address files, not rows.

## Impact

- **`src/git.rs`** — `enumerate` orders the stubs by `(parent_dir, name)` (one
  named "order the changeset" step) before returning, so every load path (working
  tree, staged, commit, range) is ordered once at the source. This also reorders
  the non-TUI text dump (`render::to_unified_string`), which loads through the
  same path — an accepted CLI-output change.
- **`src/tui/sidebar.rs`** — add the `SidebarRow` model + a builder over the
  grouping mode; rewire `window`/reveal (map the selected file to its row),
  `file_at_row` (a directory row selects nothing), and the digit spread (over
  file rows only).
- **`src/tui/ui.rs`** — `draw_sidebar` iterates `SidebarRow`s: a dim directory
  line vs a file row (basename in grouped mode); reuse `shorten_path`.
- **`src/tui/app.rs`** — a `grouping` field and its toggle; the sidebar window
  reconcile feeds the row model.
- **`src/tui/keymap.rs` / `mod.rs`** — `D` binding + a help entry (the hints↔help
  consistency test will require it documented).
- **Risk** — sorting changes file indices, so *enumerate-based* tests that assume
  a specific index (e.g. "README is index 1" in the palette tests) must be updated
  — hand-built `Changeset` tests bypass the sort and are unaffected. The
  window/digit logic also gains a row-vs-file distinction (paging counts rows,
  jump digits count files) that the flat list did not need.
