# Mock-ups for the bigger features

The user approved all of these on 2026-11-05 ("I love all of it"), so
they're the target look. The live canvas is
[merlin feature mock-ups](https://claude.ai/artifact/QzFdCBA1P32LckaUDc419F),
which is private to the user. These files are copies of its artboards, so
anyone can read them without access. They're static HTML at 1280×800 in
merlin's palette (`crates/app/src/theme.rs`, `DEFAULT`), with Inter for the
interface and JetBrains Mono for code. Open one in a browser to see it; the
`support.js` line only matters on the canvas.

Read the artboard before building its feature, and match it. Where the
mock-up and the window's existing drawing disagree on small things, such as
exact paddings, follow the window's existing helpers (`Frame`, `Canvas`,
`items_card` and the like) so the frame stays one style.

| File | Feature | What it shows |
|---|---|---|
| `Terminal.dc.html` | Integrated terminal | A Terminal tab in the bottom panel, beside Problems and Output. It has a toolbar (new, choose a shell, split, kill, maximise, close), split panes, and a list of sessions on the right with a coloured dot for each one's state. |
| `Language.dc.html` | Language features | Rename in an inline box over the name, with a note of how many places it changes. Code actions in a card from a lightbulb in the gutter, grouped as Quick Fix and Refactor. Inlay hints as quiet chips. The error at the end of its line, with the line tinted. Signature help as you type. |
| `References.dc.html` | Find references | A peek view that opens between lines: a preview on the left and the references grouped by file on the right. Keys: Shift+F12, F4 and Shift+F4. |
| `Git.dc.html` | Git | Source Control with a commit box, push and pull counts, Staged and Changes sections, and hunks unfolding under each file with Stage. The gutter bar opens the change in place, with Stage, Revert and arrows. Blame for the cursor's line, faint, at the end of the line. Sync counts in the status bar. |
| `Merge.dc.html` | Merge editor | Incoming and Current side by side over a Result pane, with Accept links per conflict, a count of conflicts left, and Complete Merge. Compare Files uses the top two panes. |
| `Tasks.dc.html` | Tasks and tests | A Tests side view as a tree of pass and fail, with Run All, Run Failed and On Save. ▶ Run Test and Debug above `#[test]` functions. A failure shown under its line. A Run Task picker from `Cargo.toml`, `package.json` and `.merlin/tasks.toml`. Output in a terminal tab. |
| `Debug.dc.html` | Debugging (DAP) | A floating toolbar (continue, step over, into, out, restart, stop). Variables, Watch, Call Stack and Breakpoints, including conditional ones. The paused line highlighted with an arrow. Values inline after lines. A Debug Console tab. The status bar in the warning colour. |
| `Settings.dc.html` | Settings window | A Settings tab with categories (Appearance, Text Editor, Files, Keyboard Shortcuts, Plugins), search, and User and Workspace scopes. Rows with a label, a description, the TOML key and a control (switch, segmented choice, number, select), and a Modified badge. The rows come from `crates/app/src/prefs.rs`. |
| `Keys.dc.html` | Vim mode, which-key and macros | NORMAL in the status bar, a block cursor and relative line numbers. A which-key card along the bottom after a pause on a prefix. "Recording @q" while a macro records. |
| `Workspace.dc.html` | Several folders, a second window | An Explorer with a root header for each folder, each with its branch, and Add Folder. Tabs name their folder. A second window for a tab dragged out. |
| `Markdown.dc.html` | Markdown preview | A preview beside the source (Ctrl+K V) that scrolls with it, with tables and Mermaid diagrams. |

Hot exit has no mock-up: it changes only whether quitting asks. Unsaved
changes are kept and come back next time, through `disk::Backups`, which
already exists for crash recovery.
