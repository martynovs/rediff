## 1. Stable path ordering at enumeration

- [x] 1.1 In `src/git.rs`, after `enumerate_repo` produces the stubs and before returning `Enumeration`, sort the stubs by `(parent_dir, file_name)` — one place covering working-tree, staged, commit, and range loads
- [x] 1.2 Confirm index alignment is preserved: stub indices are assigned post-sort, the loader installs by index, and `resume`/`drain` still work (the streaming-diff guard tests stay green)
- [x] 1.3 Audit and fix index-literal assumptions broken by the new order in the **enumerate-based** tests only (notably the palette test's `// README is index 1`); hand-built `Changeset` tests bypass the sort. Update any "changeset order" expectations to path order
- [x] 1.4 Test: a fixture spanning multiple directories (with a directory that has both direct files and a subdirectory) enumerates with each directory's files contiguous and parents in lexicographic order

## 2. Sidebar row model + builder

- [x] 2.1 In `src/tui/sidebar.rs`, add `enum SidebarRow { Dir(String), File(usize) }` and `enum Grouping { Flat, ByDir }`
- [x] 2.2 Add `rows(files, grouping) -> Vec<SidebarRow>`: `Flat` → one `File` per file; `ByDir` → push `Dir(parent)` at each parent-directory change (root parent renders as `./`), then the `File` — relying on the path-sorted order so a directory's line appears exactly once
- [x] 2.3 Unit-test the builder without an `App`: flat is all `File`; grouped inserts a `Dir` per directory at the right boundaries; root files land under `./`; a single-directory changeset yields one header

## 3. Rewire the row-indexed helpers

- [x] 3.1 `window` + reveal: window over `SidebarRow`s, counting **visible rows** for paging (distinct from **visible files**, the digit unit — keep the two counts separate); reveal the **row of the selected file** (`File(selected)`), keeping it visible with room for directory line(s) above. In flat mode `File(i)`'s row is `i` (reduces to today)
- [x] 3.2 `file_at_row` (click): map a click row to its `SidebarRow` — `Dir` selects nothing, `File(idx)` selects `idx`
- [x] 3.3 Jump digits: spread `1–9` over the **file** rows visible in the window (skip `Dir` rows); `offset_to_digit`/`digit_target` operate on file rows
- [x] 3.4 Tests: reveal a file whose row sits below a directory line (window includes both); a click on a directory line leaves `selected` unchanged; a digit jumps to the right file in a grouped window

## 4. Grouped rendering

- [x] 4.1 `draw_sidebar` iterates `SidebarRow`s: a `Dir` line renders the combined parent path shortened via `shorten_path`, styled `t.muted` + `DIM` (root → `./`); a `File` row renders as today
- [x] 4.2 In grouped mode the file row shows the **basename** (shortened if long); flat mode keeps the shortened full path
- [x] 4.3 Visual check (TestBackend or dump): grouped frame shows dim directory lines above their files, `./` for root, basenames under headers

## 5. Toggle + keymap

- [x] 5.1 `App` holds `grouping: Grouping` (default `Flat`) and a toggle method; the sidebar window reconcile feeds the row model
- [x] 5.2 Add the `D` binding (route it in `handle_key`) and a help entry (`D  group by dir`) in `keymap.rs`; the hints↔help consistency test must stay green with `D` documented
- [x] 5.3 Test: `D` toggles `grouping` flat ⇄ grouped; toggling keeps `selected` and the stream position unchanged

## 6. Wrap-up

- [x] 6.1 `cargo clippy --all-targets` clean (pedantic); full `cargo test` green; match hand style
- [x] 6.2 Manual dogfood: a multi-directory changeset → toggle `D` (dim directory lines, `./` root, basenames), navigate with `j`/`k` and `1–9` (files only), click a directory line (no-op) and a file (selects), confirm the flat list is path-ordered
