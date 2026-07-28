## Context

rediff is a TUI git-diff reviewer built around a single normalized `Changeset` (files → hunks → lines) rendered through a `Plan` of rows. Browsing is a stack of `ViewEntry` views (`session.rs`); the modal single-file `Peek` (`peek.rs`) reuses the same plan/row/highlight machinery over a synthetic one-file changeset, today with two modes (content, diff). Transient overlays (fuzzy palette, help, theme picker) layer over a retained base in one `Mode { base, overlay }` model (`app/types.rs`), and keybindings, status hints, and the help catalog all derive from one definition (`keymap.rs`) guarded by a consistency test. Background diffs stream in via a `Loader` worker pool with progress chrome after an 80 ms delay.

The commit picker already enumerates `CommitInfo` but `commit_info()` in `git/commits.rs` keeps only `message().summary()` — the body is discarded. `gix = "0.83"` exposes a `blame` feature that is not yet enabled.

## Goals / Non-Goals

**Goals:**
- One reusable commit-message popup, summoned from both the picker (`Tab`) and a blame line (`Enter`), that fetches the body by SHA and whose confirm switches to the commit.
- A scroll-away commit-message banner at the top of a commit view's stream.
- Committed-rev file blame as a third `Peek` mode, computed off the UI thread, with a collapsed, per-commit-colored `name + age` gutter and a cursor-tracking header.
- No drift in the keymap/hints/help catalog.

**Non-Goals:**
- Working-tree / uncommitted-line blame (every blamed line resolves to a committed SHA).
- Blame of a file at an arbitrary historical rev other than the current view's rev.
- Editing, reverting, or any write operation from the popup or blame.
- A new view-history entry for the popup or the blame peek (both are ephemeral, like the existing peek).

## Decisions

### One `Overlay::CommitMessage` shared by two entry points
Add a third `Overlay` variant carrying the target SHA and the fetched body plus a scroll offset. Both `Tab`-in-picker and `Enter`-in-blame construct it the same way; confirm calls the existing `open_commit`. Rationale: the popup's contract ("show body, Enter switches, Esc returns") is identical in both contexts, and routing it through the established single-overlay model gives layering, input capture, and status/help derivation for free.
- *Alternative — a Peek-style base instead of an overlay:* rejected; the popup must sit *over* the picker (itself an overlay) and over the blame peek, which is exactly what an overlay-over-retained-base already expresses.

### Fetch the body by SHA on open, don't widen `CommitInfo`
The popup loads the full message via a small `git/commits.rs` by-SHA lookup when it opens. Rationale: a blame line has only a SHA (it was never in any picker list), so a SHA→body path is required anyway; using it for the picker too avoids carrying every body in the 200-entry `CommitInfo` list. The bodies are small and the fetch is a single object read, so lazy-on-open is cheap and uniform.
- *Alternative — eager `body` on `CommitInfo`:* rejected; doesn't serve the blame entry point and bloats enumeration.

### Blame as a third `PeekMode`, not a new base or view
Extend `PeekMode` to `{ Content, Diff, Blame }`; `Tab` cycles all three. Blame mode renders the whole-file content (like content mode) but swaps the line-number gutter for the attribution gutter. Rationale: the peek already owns modal scroll, highlight, and the one-file plan; blame is "content + a different gutter," so it reuses everything and only adds gutter rendering and a parallel attribution array.
- *Alternative — `Base::Blame`:* rejected as redundant scaffolding.

### Attribution rides alongside `Line`, not inside it
The peek holds a `Vec<BlameLine { sha, author, age_token, color_key }>` indexed by file line, parallel to the rendered lines, rather than adding blame fields to the shared `Line`/`Hunk` model. The gutter renderer reads this array only in blame mode. Rationale: keeps the core diff model untouched; blame data is peek-local.

### Run-collapsing + per-commit color computed once at build
When the blame result lands, precompute for each line whether it begins a new run (SHA differs from the previous line) and a stable color key from the SHA hash. The renderer prints the `name + age` token only on run-start lines. The cursor line's full SHA/summary goes to the header via the existing `Peek::label`-style mechanism, so the gutter never needs the SHA. Age uses the compact ladder (hours/days integer; months/years one decimal only for single-digit integer parts; 12 months → years).

### Background blame via the existing `Loader` pattern
Blame walks history and can be slow, so it runs on a worker and streams its result like a diff load: the peek shows a loading state, progress chrome appears past the 80 ms threshold, and the gutter fills when the result arrives. Rationale: matches the codebase's established async-load culture; never blocks the event loop.

### Banner as synthetic top-of-plan rows
For a `ViewKind::Commit` view, the plan builder prepends message rows (header + wrapped body) ahead of the first file. They scroll with the stream and impose no fixed chrome. The body is fetched when the commit view loads (one object read) and stored on the view entry / kind. Rationale: long messages must not eat permanent space, and reusing the row plan keeps scrolling/percentage logic unchanged.

### Module placement honors the import-only convention
New logic lands in named submodules with their tests: blame computation under `src/git/` (e.g. `git/blame.rs`), the popup overlay under `src/tui/app/` alongside the other overlay code, blame gutter rendering under `src/tui/ui/`, and the age/run-collapse helpers as small pure functions (cheap to unit-test to the ≥90% floor). `mod.rs`/`lib.rs` stay declarations + re-exports only.

## Risks / Trade-offs

- **Blame performance on large/old files** → run off-thread with progress chrome; cap or bound the walk if needed; the peek stays interactive while it computes.
- **`gix` blame API maturity at 0.83** → validate the `blame` feature against a real file early (a spike test); the git layer is isolated behind `git/blame.rs`, so swapping the implementation (or shelling to `git blame --porcelain` as a fallback) stays contained.
- **Renames across history** → first cut attributes within the file's current path; cross-rename attribution is a later refinement, not a correctness blocker for the committed-rev view.
- **Gutter width vs. code width** → fixed 12-col gutter is a deliberate trade; the SHA lives in the header/popup to keep the gutter narrow.
- **CRAP/coverage gate** → keep age-formatting, run-collapsing, and gutter-layout as pure helpers so they reach the ≥90% per-function floor without driving up the cyclomatic complexity of the render orchestrators.

## Open Questions

- Exact `gix::blame` entry point and result shape at 0.83 — confirm during the spike before wiring the gutter.
- Whether `Tab` cycling into blame should lazily trigger the background blame the first time it is entered, or only `b` pre-loads it (leaning: both paths share one "ensure blame loaded" entry).
