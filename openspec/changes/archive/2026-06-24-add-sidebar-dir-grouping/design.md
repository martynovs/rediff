## Context

The sidebar (`draw_sidebar` in `ui.rs`) renders `cs.files[top..end]`, one row per
file: the visible row index *is* the file index. Everything keys off that
identity — `selected` is a file index, `sidebar::file_at_row` maps a click row to
`top + (y - area.y)`, `sidebar::window` windows over `n_files`, and the `1–9`
badges spread via `offset_to_digit(i - top, visible)`. The diff body, by
contrast, already has a row model (`Plan { rows: Vec<Row> }`, where `Row` is
chrome + content). Directory grouping is the same move for the sidebar.

Two facts decided the shape:

1. **Git order is not directory-contiguous.** Observed on a real cross-directory
   commit, a directory's own files interleave with its subdirectories' files in
   git's enumeration order (e.g. `…/design.md`, `…/.openspec.yaml`, `…/specs/…`,
   `…/tasks.md`, `…/proposal.md`, `…/specs/review-stream/…`). So clean grouping
   ("a directory line, then its files, then the next directory") requires an
   explicit sort, not just header insertion.
2. **We have every file before the diff load starts.** `enumerate` returns all
   stubs synchronously; the streaming diff then fills them in by index. So a
   one-time sort at enumeration is free and stays index-aligned with the loader
   and the resume logic (indices are assigned post-sort).

## Goals / Non-Goals

**Goals:**
- A toggleable directory-grouped sidebar: dim directory lines (combined,
  shortened paths; `./` for root) above their files; basenames as file rows.
- Files contiguous by directory, via a stable `(parent_dir, name)` sort done
  once at enumeration, index-aligned with the existing load/resume machinery.
- A `SidebarRow` model + builder mirroring `Plan::build`, with flat mode as the
  degenerate case.
- Navigation/selection/jump-digits operate on files only; directory lines are
  informative.

**Non-Goals:**
- Collapsible / foldable directories (directory lines are display-only here; a
  collapse layer could come later and would reuse the row model and the
  `file_at_row` directory hit).
- A nested, indented tree with per-depth indentation — directory lines are flat,
  combined full paths, not an indented hierarchy.
- Grouping the diff *body*'s file headers by directory (the body keeps its
  per-file headers in the now-sorted order; only the sidebar gains directory
  lines).
- Configurable default mode or per-view grouping — `grouping` is one app-global
  toggle; the directory-grouped view is the default.

## Decisions

### Decision 1: Sort once at enumeration, by `(parent_dir, name)`
`enumerate` sorts the stub list by parent directory then file name before
returning `Enumeration`, covering every load path (working tree, staged, commit,
range) from one place. The sort key is **not** the full path string —
full-path lexicographic order *splits* a directory's group when it has both
files and subdirectories:

```
  full-path sort (WRONG — splits "src"):   (parent_dir, name) sort (RIGHT):
    src/aaa.rs       parent src              ./           root files
    src/tui.rs       parent src              lib/…
    src/tui/app.rs   parent src/tui          src/aaa.rs ┐ "src" files
    src/zzz.rs       parent src   ← split    src/tui.rs ┘ contiguous
                                             src/zzz.rs ┘
                                             src/tui/app.rs   subdir group
```

Grouping by parent directory means the sort must group by that same key. The
resulting header order is parent directories in lexicographic order (`""` <
`lib` < `src` < `src/tui` < `src/tui/widgets`), which reads as a natural flat
tree (parents before their children). The empty parent (`""`) is the root group,
shown as `./`.

This is index-aligned with the load/resume machinery: stub indices are assigned
*after* the sort, the loader installs diffs by index, and resume re-diffs
undiffed indices — all post-sort, so nothing downstream needs to change.

**This is a global ordering change, deliberately — not a sidebar-only one.**
Because the sort sets the canonical `cs.files` order, it reorders *the diff body*
(the primary reading surface, whose per-file headers follow `file_starts`) and
the *non-TUI text dump* (`render::to_unified_string`, used when stdout is not a
TTY), in **both** flat and grouped modes, for users who never press `D`. The
files appear in path order instead of git's status/enumeration order everywhere.
We accept this: a stable path order is a more predictable reading order than
git's, and it is what makes grouping a pure header-insert. The alternative —
sorting only the sidebar's *display* while leaving the body in git order —
decouples sidebar position from the file index (and forces a "does `j`/`k` follow
git order or display order?" decision); rejected here for that complexity. The
body is *reordered* but not *grouped* (no directory headers in the body — that
stays a Non-Goal); only its order changes.

**Root files (empty parent) sort first**, so the `./` group is at the top of the
sidebar and the body. This falls out of `"" < any directory`, and is the chosen
behavior (root files like `README`/`Cargo.toml` at the top); root-last is not
done.

The sort is expressed as one explicit "order the changeset" step at the single
point every load funnels through (the end of `enumerate`), named as such rather
than an anonymous inline `sort_by` — both so the concern is legible and so it is
the natural seam if ordering ever becomes configurable.

### Decision 2: A `SidebarRow` model, built per grouping mode
```
enum SidebarRow { Dir(String), File(usize) }   // String = the group's parent path

fn rows(files: &[DiffFile], grouping: Grouping) -> Vec<SidebarRow>
//   Flat  → [File(0), File(1), … File(n-1)]            (today's behavior)
//   ByDir → walk files; when parent_dir changes, push Dir(parent); push File(i)
```
This mirrors `Plan::build(cs, viewed, layout)`: one builder, a mode parameter,
chrome rows (`Dir`) interleaved with content rows (`File`). Because files are
already `(parent_dir, name)`-sorted (Decision 1), `ByDir` is a pure
header-insert — a directory line appears exactly once per directory, at the
boundary where the parent changes.

### Decision 3: Directory rows are informative; files own all the state
`selected` stays a file index; `viewed` stays per file; `j`/`k`
(`next_file`/`prev_file`) move file→file. Directory rows are never selected,
carry no jump digit, and are skipped by navigation. So the *navigation* model is
unchanged — only *rendering* and the *row-indexed* helpers (window, hit-test,
digits) learn about directory rows. This keeps the change small and avoids
touching the file-index invariants the rest of the app relies on.

### Decision 4: Rendering — combined shortened directory lines, basename files
- **Directory line:** the group's combined parent path (e.g. `src/tui/widgets`),
  shortened to the sidebar width with the existing `shorten_path`/`abbrev_dir`;
  root group shown as `./`. Styled dim/muted (`t.muted` + `DIM`) — the "dark
  line".
- **File row (grouped):** the file's basename, shortened if too long — the
  directory is already in the header, so the full path is redundant.
- **File row (flat):** unchanged — the shortened full path.
The selection marker, status glyph, stats, and reviewed styling on file rows are
unchanged.

### Decision 5: The four row-indexed helpers learn to skip `Dir` rows
The subtlety here is that there are now **two counts**, where the flat list
conflated them into one (`sidebar_visible`):
- *visible rows* — `Dir` + `File` rows on screen, the unit for paging/windowing;
- *visible files* — `File` rows only, the unit for the jump digits.
Keep them distinct; conflating them is the likeliest off-by-one.

- **`window` + reveal:** windows over `SidebarRow`s (row count). Reveal still
  targets the selected *file*, so it maps that file to its row index (the row of
  `File(sel)`) and scrolls to keep that row visible — leaving room for any
  directory line(s) above it. In flat mode the row of `File(i)` is `i`, so this
  reduces to today's behavior.
- **`file_at_row` (click):** maps a click row to its `SidebarRow`; a `Dir` row
  selects nothing (the natural seam where a future collapse action would hook);
  a `File(idx)` selects `idx`.
- **`1–9` digits:** spread over the *visible files* (skipping directory rows),
  so `1` is the first visible file and `9` the last; a file's badge is computed
  from its index *among the visible files*, not its row.

### Decision 6: `grouping` is one app-global toggle, key `D`
`App` holds a `grouping: Grouping { Flat, ByDir }` (default `ByDir`), flipped by
`D` exactly like `m` flips the layout. (`g` — the originally-proposed key — is
already go-to-top in the stream/sidebar/peek; `D` for "Directory" is free and
matches the capital-action convention `C`/`F`/`R`.) The binding has a help entry
(`D  group by dir`); the toggle sets `reveal_selected` so the cursor stays
visible after the row layout changes. The grouped/flat choice is app-global, not
per-view.

## Risks / Trade-offs

- **File indices change** when the sort lands, so behaviors assuming a specific
  index break — notably the palette test's `// README is index 1`. The fallout is
  *narrow and locatable*: only tests that load through real `enumerate` reorder;
  the many tests that hand-build a `Changeset` and call `App::new(&cs)` (e.g. the
  synthetic `f0..f11` sidebar test) bypass the sort entirely and are unaffected.
  Audit only the enumerate-based tests.
- **Flat order changes** from git's order to path order. Accepted as a more
  predictable default; document it. (If preserving git's order for flat mode is
  ever wanted, the sort would have to move into the grouped *display* and the
  sidebar would decouple from the file index — explicitly rejected here for the
  complexity.)
- **The file→row mapping** for reveal/window is the one piece of genuinely new
  logic; cover it with a test (reveal a file whose row sits below a directory
  line and assert the window includes both).
- **Empty / single-directory changesets** must render sensibly — all files under
  one header, or all under `./`; assert the degenerate cases.

## Open Questions

- Should a directory line show an aggregate (e.g. reviewed `2/3`, or summed
  `+/-`)? Out of scope here ("informative" = the path), but a cheap future add.
- Should `D` be a two-state toggle (flat ⇄ grouped) or leave room for a third
  mode later (e.g. grouped-collapsed)? Modeled as an enum so a third variant is
  additive; ships with two.
