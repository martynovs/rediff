## Why

rediff can show *what* a commit changed but never *why*: the commit picker keeps only a commit's summary line (the body is discarded at enumeration), commit diff views show no message at all, and there is no way to see which commit last touched a given line. Reviewers routinely need the full message and per-line attribution to understand a change, and today they must drop out to `git` to get either.

## What Changes

- **Full commit messages become viewable.** A single shared "commit message" popup shows a commit's SHA, author, date, and full body. It is reachable from the commit picker (press `Tab` on the highlighted commit) and from a blame line. Its `Enter` switches the current view to that commit; `Esc` returns to whatever was beneath it.
- **Commit diff views gain a message banner.** When viewing a single commit, its message is rendered before the diff as scroll-away content at the top of the stream (not fixed chrome), so long bodies behave naturally.
- **File blame.** A new `b` key blames the selected file at the current view's committed rev (HEAD for a local view, the commit for a commit view); committed content only. Blame is a third peek mode alongside content and diff (`Tab` cycles all three), reusing the peek's modal/scroll/highlight scaffolding. Its gutter shows a per-line `name + relative-age` token (collapsed across runs of the same commit, colored per-commit); the cursor line's full SHA and summary appear in the peek header. `Enter` on a blame line opens the shared commit-message popup for that line's commit rather than jumping directly.
- **Keymap, hints, and help** are extended for the new keys (`b`, `Tab` in the picker, blame-mode bindings), keeping the single-source keymap catalog and its consistency test honest.

## Capabilities

### New Capabilities
- `file-blame`: computing committed-rev blame for a file off the UI thread and presenting it as a peek mode with a collapsed, per-commit-colored attribution gutter, a cursor-tracking header, and `Enter`-to-message-popup.
- `commit-message`: the shared commit-message popup (SHA, author, date, full body; `Enter` switches to the commit, `Esc` returns) and the scroll-away commit-message banner shown before a commit's diff.

### Modified Capabilities
- `commit-navigation`: the commit picker adds `Tab` on the highlighted commit to open the commit-message popup (read-before-pick); `Enter` still picks directly.
- `file-peek`: the peek gains a third mode (blame) that `Tab` cycles through, a `b` open key, and a mode-dependent gutter and header.
- `mode-routing`: the commit-message popup becomes a transient overlay in the single-overlay routing model (alongside the palette and help), layered over its base and returning to it on dismiss.

## Impact

- **Dependencies**: enable the `blame` feature on the existing `gix = "0.83"`.
- **Git layer** (`src/git/`): add committed-rev blame; extend commit enumeration / a by-SHA lookup so the popup can fetch a full message body.
- **Model** (`src/model.rs`): blame-line attribution type; commit-message body carried for the banner.
- **TUI**: new `Overlay::CommitMessage` (`src/tui/app/`), a `PeekMode::Blame` and blame gutter (`src/tui/peek.rs`, `src/tui/ui/`), background blame load reusing the `Loader`/progress pattern, banner rows in the plan builder, and key routing + `keymap.rs` catalog/hints updates.
- **Gates**: clippy pedantic clean, the CRAP gate with the ≥90% per-function coverage floor, then `cargo fmt --all`.
