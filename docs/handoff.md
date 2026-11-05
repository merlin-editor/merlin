# Handoff: building the approved mock-ups

This is for the next agent continuing work on the `editor-rework` branch.
Read it with [`feature-research.md`](feature-research.md), which has the
research, the build order, and what's marked **Built**, and with
[`mockups/README.md`](mockups/README.md), which describes the 11 approved
mock-ups. Nothing is pushed. The last commit when this was written was
`d4283cc`.

## The user's rules

- merlin is meant to look and feel modern. It takes the best of the
  popular editors (VS Code, Zed, JetBrains, Sublime, Neovim) and what
  users ask for most, by votes on their issue trackers. Before designing
  a feature, check how those editors do it and what their users complain
  about, then fit it to merlin's current look and the mock-ups.
- Build to the mock-ups the user approved, in the order in
  `feature-research.md`. Anything new that changes or adds UI and isn't in
  a mock-up gets a mock-up first, for the user to review.
- Keep merlin's look: reuse the window's helpers (`Frame`, `Canvas`,
  `Text`, `items_card`, `icon_button`, the palette in `theme.rs`) rather
  than drawing to exact mock-up paddings.
- merlin aims at low CPU and memory use. Don't add work per frame or per
  keystroke without need.
- Commit each finished piece with a message that says what it does for
  the user. Don't push unless asked.
- Check each piece in the running window, not only in tests (see
  [Checking in the window](#checking-in-the-window)).

## Where things stand

| Step in the build order | State |
|---|---|
| Quick wins | Built (`bb07318`) |
| 1. Integrated terminal | Done (`493d5f0`, `7ee4884`) |
| 2. Language servers, finished | **Partly built.** See below |
| 3 to 10 | Not started |

### Language features (`Language.dc.html`)

Built, each checked in the window with rust-analyzer:

- **Errors inline** (`d5c2091`, `d4283cc`). A line's worst error, warning
  or info is written a few columns after its end in the interface's font
  and its severity's colour. A line with an error or warning gets a faint
  band (alpha 18) of that colour. A second line of the message joins the
  first unless it starts with `help:`, `note:` or `` `#[ ``, so rustc's
  "mismatched types: expected `u32`, found `usize`" reads as one note.
  Other distinct problems starting on the line are counted as `(+N more)`.
  The same message twice (rust-analyzer and cargo check both report type
  errors) counts once. Setting: `editor.inline_diagnostics`.
  - Core: `LineView.note`, chosen in `text_view.rs` (`note_text`).
  - Window: `code.rs`, in `draw_code`, which now takes `&mut Text` to write
    in the UI font.
- **Inlay hints** (`09b607e`). Chips in `p.raised`, the label in the
  code's font in `p.muted`, half a column of room either side. Setting:
  `editor.inlay_hints`.
  - A hint takes `str_width(label) + 1` columns of its own, and the core
    counts them wherever it turns characters into columns:
    `Doc::display_col`, `Doc::col_at_display` (a click on a hint lands
    before its character), `Doc::line_inlays`, and `wrap::wrap`, which
    takes the line's inlays. A hint at column `c` comes *after* a caret
    at `c`.
  - Hints live in `Doc.inlays` (sorted by position) and move with edits
    in `Doc::sync`, like diagnostics.
  - `LineView.inlays` (`InlayView`) carries them to the window, which
    places them in its `spots` loop in `code.rs`.
  - Tests: `crates/core/tests/inlays.rs`, and a wrap test in `wrap.rs`.
- **Signature help** (`d4283cc`). A card over the line of the call, or
  under it when there's no room above, so it keeps clear of completions.
  The active parameter is bold with an accent underline, `N of M` sits on
  the right, and the documentation's first paragraph is underneath.
  - Core: `Editor.signature` is `(doc id, anchor pos, Signature)`.
    `check_signature` in `update` closes it when the cursor leaves the
    anchor's line or goes before the anchor, when the doc changes, or when
    focus leaves the editor. Esc closes it (`popup_key`). It reaches the
    window as `WindowView.signature` (`SignatureView`).
  - Window: `Frame::signature` in `frame.rs`.
  - Tests: the two `signature_help_*` tests in
    `crates/core/tests/language.rs`.

- **Problems in the hover card** (mock-up `Hover.dc.html`, approved
  2026-11-05). Resting the mouse on a squiggle shows a card with the
  problem first:
  - the severity's badge, the message bright and its other lines muted,
    with backticked code in the code font
  - rustc's hint for the same code folded in as a `help:` note
  - the source as a chip, the code (a link when the server gives its
    `codeDescription`, opened with `xdg-open`), and "also from" for the
    other checkers that report it
  - quiet Copy and Problems links on the right, the left kept for fixes
  - a divider, then the server's hover: its code in the code font and
    coloured by the plugin that adds its language (`colour-text`, the
    same route diffs and Markdown code blocks take; see
    `hover_colours` in `context.rs`), the
    first two paragraphs of its docs up to a heading, and Go to
    definition (F12, a new core command `go.definition`)

  More of how it behaves:
  - The problem part shows at once and the server's part joins it.
  - Off a word, or with no server, it's the problem alone.
  - The same problem from two checkers shows once: the same first line,
    or the same code and severity over overlapping ranges. The inline
    note's `(+N more)` still counts both copies.
  - The card stays open while the mouse is on the word, on the
    problem's range, or over the card and the gap above it.
  - Ctrl+K Ctrl+I (`view.hover`, also the right-click menu's Show Hover)
    opens it for the cursor.

  Where it lives:
  - API: a new host call, `set-detailed-diagnostics`, takes a
    `detailed-diagnostic` record (a diagnostic plus `source`, `code`
    and `link`). The LSP plugin now asks for Markdown hovers and keeps
    their fences, for the card to draw. The LSP plugin uses it; old plugins' `set-diagnostics`
    still works, with no source or code.
  - Core: `Diagnostic` and `Mark` gained `source` and `code`.
    `problems_at`, `hover_at`, `hover_at_cursor`, `show_hover` and
    `pointer_moved` are in `context.rs`. `PopupKind::Hover` is
    `{ problems, lines }`, and the buttons send `UiEvent::Hover`.
  - Window: `Frame::hover_card`, `hover_layout` and `hover_parts` in
    `frame.rs`.
  - Tests: the hover tests at the end of `crates/core/tests/language.rs`.

- **Code actions** (`Language.dc.html`).
  - **Ctrl+.** (`edit.quick_fix`) lists the actions at the cursor in a
    card grouped as Quick Fix, Refactor, Source and More Actions. The
    server's preferred fix is chosen and marked with a bulb. The arrows
    move, Enter or a click makes it, Esc closes.
  - **The lightbulb** shows on the cursor's line when the line has a
    problem worse than a hint, and a click lists the actions. The plugin
    never hears about cursor moves, so the bulb can't wait for the
    server's answer. It sits in two cells of room the gutter gets in code
    a language plugin adds, while the LSP plugin is loaded
    (`bulb_room` in `layout.rs`). The right-aligned numbers have it as
    padding.
  - **The hover card over a problem** asks for the actions too. It
    shows the preferred quick fix (else the first) as its primary
    button, and More fixes (Ctrl .) lists them all at the problem.
  - The "Format the file" row in the mock-up's card waits for format.

  API, all new imports:
  - `show-code-actions(list<code-action>)` answers the plugin command
    `code-actions`. The core asks it from Ctrl+., the bulb and the hover,
    and keeps a queue of what each request was for
    (`actions::ActionsFor`).
  - Choosing one calls the existing `on-pick` with picker
    `code-actions`.
  - `apply-edits(list<file-edit>)` needs `editor-write`. The LSP
    plugin's manifest now asks for it, so **the user sees its
    permission card once** in the Language panel.
  - Each file's edits are one undo step. A file that isn't open is
    opened in a tab, changed and left unsaved (`opened_for_edits`).

  The LSP plugin:
  - It asks `textDocument/codeAction` for the cursor, with the line's
    diagnostics as context.
  - It keeps each action's workspace edit by id, and forgets them when
    the text changes.
  - It leaves out disabled actions and bare commands, which would need
    `workspace/executeCommand`.
  - Of a workspace edit it makes `changes` and `documentChanges`' text
    edits, not file creates, renames or deletes.
  - It doesn't advertise `resolveSupport`, so rust-analyzer sends the
    edits with the list.

  Where it lives: `crates/core/src/editor/actions.rs`;
  `Frame::actions_card`, `fix_button` and `more_button` in `frame.rs`;
  the bulb in `code.rs`.

  Tests: the code action tests at the end of
  `crates/core/tests/language.rs`, and
  `code_actions_come_with_their_edits_and_one_chosen_is_made` in
  `crates/plugin-host/tests/lsp.rs` (the fake server answers
  `textDocument/codeAction`).

  Checked in the window with rust-analyzer: rustc's "prefix it with an
  underscore" fix from the menu and from the hover card, and undoing it.
  For this project's `u32`/`usize` mismatch, rust-analyzer offers no
  quick fix at all, only refactors.

- **Format and format on save** (`Format.dc.html`, approved
  2026-11-05).
  - **Keys and commands:**
    - Ctrl+Shift+I (`edit.format`) formats the file.
    - Ctrl+K Ctrl+F (`edit.format_selection`) formats the selection.
    - Ctrl K S (`file.save_without_formatting`) saves as it is.
  - **In the code actions card:** "Format the selection" (when
    something's selected) and "Format the file" come last, under a rule,
    with their keys (`ActionKind::Format`, ids `merlin:format` and
    `merlin:format-selection`). With them the card opens even when the
    server has no actions.
  - **Format on save** is `editor.format_on_save`: `off`, `file` or
    `changed_lines`. It's in Settings › Files.
    - Only Ctrl+S formats. Auto save, Save All and saving on close
      don't.
    - The save waits for the plugin's `apply-edits` for that file, then
      tidies and saves. "Formatting…" shows in the status bar meanwhile.
    - The app times it (`format_due` in `main.rs`, `FORMAT_WAIT_MS` =
      1.5 s) and sends `Event::FormatTimedOut`. The file then saves as
      it is, with a notice and a Settings button.
    - Edits for text typed over since are dropped, and it still saves.
    - A notice that came while it waited, such as the plugin's reason,
      isn't replaced by "Saved".
  - **Changed lines:** a new host call, `changed-lines`, gives the
    plugin the active file's lines changed since git. The core works
    them out from the gutter's git signs (`Editor::changed_lines`), and
    they reach the plugin in `Snapshot.changed`. The plugin asks
    `rangeFormatting` for each run of changed lines and sends the edits
    together.
  - **The LSP plugin always answers a format with `apply-edits`,** with
    no edits when it can't, so a save never waits for nothing. Its
    commands are `format`, `format-on-save`, `format-selection` and
    `format-changed`.
  - **It says why a save didn't format once a file:**
    - "Nothing formats TOML files here"
    - "main.rs isn't in git, so it saved unformatted"
    - "rust-analyzer can't format only changed lines, so main.rs saved
      unformatted" (it needs nightly rustfmt for ranges)
  - **Differences from the mock-up:**
    - "Formatting…" has no spinner.
    - The timeout notice says "the language server", not its name.
    - Notices have one colour, not the mock-up's warning dot.
  - **Known:** the plugin sends `tabSize` 4 and spaces, since it can't
    read the editor's indentation. rustfmt ignores them.
  - Where it lives: `crates/core/src/editor/format.rs`; `format` in
    `plugins/lsp/src/lib.rs`.
  - Tests: the format tests at the end of `crates/core/tests/language.rs`;
    `formatting_sends_the_servers_edits_and_changed_lines_ask_for_their_ranges`
    and `a_file_not_in_git_saves_unformatted_and_says_so_once` in
    `crates/plugin-host/tests/lsp.rs`.
  - Checked in the window with rust-analyzer: format on save, Ctrl+.,
    Ctrl+Shift+I, and both Changed lines notices.

- **Rename, built** to the Language mock-up's box and
  [`mockups/Rename.dc.html`](mockups/Rename.dc.html):
  - **F2** (`edit.rename`) opens a box under the name, which is outlined,
    with all of it selected. The core asks the plugin's `rename` command;
    the plugin calls `prepareRename` (when the server has it) and
    `references`, and answers with the new host call `show-rename`: the
    name's range and how many places and files, or why not (the box
    closes and the reason shows).
  - The box takes every key while open and never edits the text:
    typing, Backspace, Delete, arrows, Home, End, Ctrl+A, Ctrl+C,
    Ctrl+V. Esc or a click elsewhere closes it.
  - **Enter** calls `on-submit` with input `rename`; the plugin sends
    `textDocument/rename`'s edits with `apply-edits`, and the core says
    "Renamed a to b: N places in M files".
  - **Shift+Enter** submits `rename-review`; the plugin answers with the
    new host call `show-rename-review(edits)`. Files that aren't open are
    read first (`Effect::ReadFiles`, `Event::FilesRead`).
  - **The review:** a Rename tab in the bottom panel (only while
    reviewing), with a header row (`Node::Header`, Discard Esc and Apply N
    Enter) and a row per file and per place. `Entry` gained `check`
    (a tri-state box) and `swap` (the old name struck through, the new
    after). The chosen row's file shows as a read-only diff tab, "main.rs ·
    Rename", whose changed words are tinted (`Review.words`, the styles
    `DiffAddedWord` and `DiffRemovedWord`).
  - Keys there, and in the diff tab: ↑ ↓ move, Space leaves a place (or
    a file's) in or out, Left and Right fold a file, Enter applies (each
    file one undo step), Esc discards. The status bar says "Reviewing a
    rename: Enter applies, Esc discards".
  - A file typed in since the review began is left as it is, and the
    notice says so.
  - **Differences from the mock-up:** the review tab's name has no muted
    part, and the box is as wide as its note.
  - Where it lives: `crates/core/src/editor/rename.rs`; `prepare_rename`,
    `find_references` and `submit` in `plugins/lsp/src/lib.rs`.
  - Tests: the rename tests at the end of `crates/core/tests/language.rs`,
    and the two rename tests in `crates/plugin-host/tests/lsp.rs`.
  - Checked in the window with rust-analyzer: the box and its count,
    Enter, Shift+Enter with a place left out, Esc, and a refusal.

- **Find references, built** to
  [`mockups/References.dc.html`](mockups/References.dc.html):
  - **Shift+F12** (`go.references`) runs the plugin's `references`
    command; it asks `textDocument/references` without the declaration,
    as VS Code's peek does, and answers with the new host call
    `show-references(list<location>)`. Files that aren't open are read
    (`Effect::ReadFiles`).
  - **The peek opens between the lines:** the line it's for takes 15
    more screen rows (`Editor::peek_gap`, rows with `start` at
    `usize::MAX` in `line_rows`), so the code below moves down, and the
    window draws the peek in them (`Frame::peek`).
  - Inside: a header with the name, "N references in M files", Open
    all in the Search view and Close; the chosen reference's lines on
    the left, coloured, with the file's references marked; the files and
    references on the right.
  - Keys while it's open: ↑ ↓ move, Left and Right fold a file, Enter
    opens the chosen reference, F4 and Shift+F4 step through them, Esc
    closes. Any other key closes it and does what it does. A click
    chooses a row; a double click opens it.
  - A long line's reference still shows: the list row starts a little
    before it, marked, and the preview leaves out the lines' shared
    indent and slides left to the chosen reference.
  - The right-click menu has Go to Definition (F12), Find References
    (Shift+F12) and Rename Symbol (F2) while the language server offers
    them.
  - **Differences from the mock-up:** the name in the line it's for
    isn't highlighted, the list doesn't scroll with the wheel (it follows
    the chosen row), and the mock-up's key note in the corner isn't shown.
  - Where it lives: `crates/core/src/editor/peek.rs`; the gap in
    `crates/core/src/editor/wrap.rs`.
  - Tests: the peek tests at the end of `crates/core/tests/language.rs`,
    and `references_come_for_the_peek_view` in
    `crates/plugin-host/tests/lsp.rs`.
  - Checked in the window with rust-analyzer: Shift+F12, F4, Enter, and
    the Search view button.

The language server features are all built. Next in
[`feature-research.md`](feature-research.md)'s order is **Git**, from
[`mockups/Git.dc.html`](mockups/Git.dc.html), with
[`GitSync.dc.html`](mockups/GitSync.dc.html),
[`Compare.dc.html`](mockups/Compare.dc.html) and
[`GitFiles.dc.html`](mockups/GitFiles.dc.html). It's built in slices, each
committed:

- **Staging and discarding single changes, built.**
  - After each status the app runs `git diff --relative` and
    `git diff --cached --relative` and sends `Event::GitDiffs`;
    `crates/core/src/git.rs` parses them (`FileDiff`, `Hunk`).
  - Git joins changes a few lines apart into one hunk, so Source Control
    shows each run of changed lines (`FileDiff::changes`) on its own row,
    as the gutter does. Staging one is a patch of only its lines
    (`git::patch` with chosen lines): left out, an added line is dropped and
    a removed one kept (backwards for `-R`).
  - `GitAction::Apply` runs `git apply --recount`, with
    `--directory=<prefix>` when the project is a folder inside the
    repository (git takes patch paths from the top, and skips others
    silently).
  - A file's row has an arrow (`Entry.fold`, `UiEvent::Fold`) and its
    counts (`Entry.counts`). Change rows have `Entry.links`: Stage, with
    Discard shown before it on hover so Stage never moves.
  - `GitAction::Discard` reads the file first and answers
    `Event::Discarded`; the notice has Undo (`GitAction::Undiscard` writes
    it back), and the reload notice stays quiet for it. An untracked file
    asks `Ask::Trash` instead.
  - The file menu is `scm::SCM_ITEMS`; Ctrl+Del discards from the list.
  - Where it lives: `crates/core/src/editor/scm.rs`; `Git::run` and
    `Git::diffs` in `crates/app/src/main.rs`.
  - Tests: `git::tests`, and the hunk and discard tests at the end of
    `crates/core/tests/workbench.rs`.
  - Checked in the window: unfold, Stage, Unstage, Discard and Undo, and
    the menu.
- **The changes tab side by side with boxes, built** (Compare):
  - The tab compares the last commit with the working tree, the whole
    file (`compare::WHOLE_FILE` context). Which changed lines are staged
    comes from the two diffs Source Control has (`compare::mark_staged`,
    `Review.staged`): an added line is staged unless the index's diff to
    the working tree adds it; a removed one is staged when the staged diff
    removes it.
  - Staging lines builds one patch from the index's diff, unstaging one
    from the staged diff (`set_staged`, `git::patch_lines`), with
    `FileDiff::find` and `map_line` to move between the commit's, the
    index's and the working tree's line numbers.
  - « (`revert_change`) unstages the change, then reverse-applies a patch
    made from the tab's own lines (`revert_patch`), with Undo.
  - Side by side has a 7-column gap (`text_view::GAP`) and two more gutter
    columns on the new side. The new side's `TextView.diff` has
    `DiffMarks`: each change's box and «, each changed row's box (a
    removed line and the one that replaced it go together), and rows to
    hatch; the window draws them in `Frame::diff_marks`.
  - The bar at the right of the tabs is `WindowView.diff_bar`
    (`Frame::diff_bar`); Stage File and Revert File drop out under 900 px.
  - Keys are `keys::DIFF_KEYS` with `when` `isInDiffEditor`, tried before
    the rest, so Alt+S stages there and opens the Selection menu
    elsewhere.
  - Open changes tabs ask for their diff again after each `GitDiffs`, and
    keep their place (`diff_refresh`).
  - Not yet: editing the new side, and boxes inline.
  - Tests: the changes tab tests at the end of
    `crates/core/tests/workbench.rs`.
  - Checked in the window: Alt+I, a change's box, «, and Undo.
- Still to build, in order: editing the changes tab's new side, the change peek from the gutter with Alt+F5 and Alt+F3, push,
  pull and sync with their notices (GitSync), blame, and new, deleted and
  moved files (GitFiles).

Saved for later, by the user's choice:

- **Scrolling a long hover** with the keyboard (↑ ↓ once it's
  focused), from [`mockups/Hover.dc.html`](mockups/Hover.dc.html). The
  card keeps only the docs' first paragraphs, so it's rarely long; this
  pays off only if the card one day shows the full docs.

#### A suggested design for what's left

These are suggestions; nothing of them is written yet.

- **Plugin API rule.** Plugins built before a change must still load. New
  **imports** (host calls) are fine, but a new **export** would break
  every existing plugin. That's why the last three features only added
  host calls (`set-inlay-hints`, `show-signature`). Reuse existing
  exports for answers:
  - `on-pick(picker, item)` for choosing a code action, with picker id
    `"code-actions"`.
  - `on-submit(input, text)` for the rename box, with input id `"rename"`.
  - `on-review` for Shift+Enter "review first", through `show-diff`.
- **New host calls:**
  - `show-code-actions(list<code-action>)`, with a record of `id`,
    `title`, `kind` (quick-fix, refactor, source, other) and `preferred`.
  - `apply-edits(list<file-edit>)`, needed by code actions, format and
    rename. It needs the `editor-write` permission, which the LSP plugin's
    `plugin.toml` doesn't have yet. Edits to files that aren't open will
    need the app to open, edit and save them, or show them as reviews.
  - `start-rename(start, end, note)`.
- **Keys.** Plugin command keys must use Ctrl or Alt, so F2 can't be a
  plugin key. Add core commands that forward to the LSP plugin, as
  `context.rs` (`hover_at`) and `mouse.rs` (Ctrl+click definition)
  already do with `Effect::PluginCommand { plugin: "lsp", .. }`:
  - `editor.rename` on F2
  - `editor.code_actions` on Ctrl+.
  - `editor.format` on Ctrl+Shift+I
  - and `editor.definition` on F12, which merlin lacks (only Alt+D now)
- **The lightbulb.** The plugin never hears about cursor moves, only
  `switched`, `changed` and `saved` (adding a `buffer-event` case would
  break old plugins). So show the bulb on the cursor's line when it has
  a diagnostic, and ask the server for actions when Ctrl+. or the bulb is
  pressed.
- **Format on save** can't block the save on an asynchronous plugin. One
  way: with the setting on, the core asks the plugin to format first.
  When the plugin's `apply-edits` arrives, or after a timeout of about
  1.5 s, the core saves.

### Things found along the way

- **Fixed (`14840d3`): the language server didn't start.** The language
  server is started for the first file before the window's event loop
  exists, so its first reply woke nothing. rust-analyzer then waited
  forever for `initialized`, and the Language panel said "starting…".
  `gui::run` now sends one wake as soon as the loop exists. If language
  features ever seem dead, check the Language panel (Alt+P) first.
- **Server refreshes.** The LSP plugin now advertises `refreshSupport`
  for inlay hints, semantic tokens and diagnostics, and asks again when
  rust-analyzer sends `workspace/*/refresh`. Without that, the first
  answers, which come before indexing, are empty and never replaced.
- **Inlay hint ranges.** rust-analyzer rejects a range that ends past
  the last line ("Invalid offset"). The plugin tracks each document's
  last line (`Server.last_lines`) and asks up to
  `{line: last, character: 1 << 20}`. Servers clamp a column past a
  line's end, as the protocol says.
- **Known, not fixed:**
  - Problems from cargo check (flycheck) are for the file on disk. In an
    unsaved buffer whose lines have moved they land on the wrong lines,
    until the next save.
  - Every change asks the server for inlay hints, semantic tokens,
    symbols and folds for the whole file. That's fine for rust-analyzer
    at normal file sizes, but could be debounced.
  - Sticky-scroll lines show no hints (`plain_line` gives none), and the
    minimap ignores them.
  - A tab after a hint is measured from the column that includes the
    hint.
  - Typing `(` while completions show leaves the completion list open
    beside the signature card.

## Plugin and build notes

- Plugins in `plugins/` are installed as **symlinks** to this repo
  (`~/.local/share/merlin/plugins/<name>`). Rebuilding a plugin's wasm
  changes what the user's own merlin loads; it hot-reloads.
- Build the LSP plugin with
  `cd plugins/lsp && cargo build --release --target wasm32-wasip2`.
  `scripts/install-plugins.sh` builds and installs them all.
- The WIT is `wit/merlin.wit`. Host calls are implemented in
  `crates/plugin-host/src/lib.rs` (`impl host::Host for State`) and turned
  straight into core `Event`s. The Rust SDK's re-exports are in
  `sdk/rust/src/lib.rs`; add new types to both lists there.
- Checks before each commit:
  - `cargo test --workspace`: 361 tests passed with the changes tab side by side
  - `cargo clippy --workspace --all-targets`
  - in `plugins/lsp`: `cargo clippy --release --target wasm32-wasip2`
  - `cargo fmt --all`

## Checking in the window

The user's desktop is Hyprland. The earlier agent drove it with these
tools: `wtype` for keys, `grim` and `hyprctl` for screenshots, and
python-uinput scripts for the mouse. They were in that session's
scratchpad, which the next agent won't have, so here is enough to
recreate them:

- **Launch:** kill any running `target/release/merlin` by its PID, from
  an *anchored*
  `pgrep -f '^/home/marc/programming/rust/merlin/target/release/merlin'`.
  An unanchored pattern can kill your own shell. `pkill` with that
  pattern once left the old one running, and the next launch did
  nothing. Then, from a test
  project folder, run
  `setsid target/release/merlin src/main.rs > log 2>&1 < /dev/null &`.
  Poll `hyprctl clients -j` for the window whose `class` is `""`, and
  `hyprctl dispatch focuswindow address:<addr>`.
- **Screenshot:** take that client's `at` and `size` from
  `hyprctl clients -j`, then `grim -g "x,y wxh" out.png`.
- **Keys:** `wtype -M ctrl s -m ctrl` presses Ctrl+S, and `wtype "text"`
  types text.
  Send a chord's keys as separate calls (`wtype -M ctrl -k k -m ctrl`,
  then the same for `i`). One call drops the second key.
- **Grants:** to test a plugin that asks for more permissions without
  touching the user's grants, run merlin with `XDG_DATA_HOME` set to a
  scratch folder holding a copy of `~/.local/share/merlin/grants.toml`,
  and allow the card there. Plugins still load from
  `~/.config/merlin/plugins`.
  Link `~/.local/share/fonts` into that folder as `fonts` too:
  fontconfig looks there, and without it the Nerd Font icons draw as
  boxes.
- **Focus:** windows move. Read merlin's position from `hyprctl clients
  -j` before each move, and check `hyprctl activewindow` is merlin
  before typing. Keys once went to another app.
- **Mouse:** `hyprctl dispatch movecursor` moves the cursor without
  sending the window any motion, so hovers never fire. Warp there, then
  nudge it with a virtual mouse, using python's `evdev`
  (`/dev/uinput` is writable):
  - make `UInput({EV_REL: [REL_X, REL_Y], EV_KEY: [BTN_LEFT]})`
  - wait 0.3 s for it to register
  - `movecursor` to one pixel left of the target
  - write `REL_X` +1 and −1 a few times, then +1
  - for a click, `BTN_LEFT` 1 then 0
  - give zsh one script per call: an unquoted `$var` holding a command
    with arguments isn't split.
- **Test project:** a small Cargo project whose `src/main.rs` has a
  type error (`let count: u32 = steps.len();`), an unused variable, and
  calls with several arguments shows every language feature.
  rust-analyzer takes about 5–10 s to send hints, and cargo-check
  problems come after a save.
- Killing merlin with unsaved changes leaves a crash backup in
  `~/.local/share/merlin/backups/<project>`, which comes back at the next
  start. Remove the backup of a scratch project before rerunning.
