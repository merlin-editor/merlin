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
| 2. Language servers, finished | Built. See below |
| 3. Git | Built. See below |
| 4. Tasks and tests | Built. See below |
| 5. Hot exit | Built. See below |
| 6. Settings editor | Built. See below |
| 7. Debugging | First slice built. See below |
| 8 to 10 | Not started |

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
  - **Editing the new side:** keys that change the text (typing, and the
    commands in `compare::FILE_EDITS`, Ctrl+S among them) run in the
    file's own document at the same place (`through_file`), then the tab
    is rebuilt from the file's text (`rebuild_changes`) with the cursor
    where the edit left it. Paste goes the same way. The file opens as a
    tab beside the changes tab (`open_beside`, `opening_beside`). The last
    commit's side says it can't be edited. While the file has unsaved
    edits the row boxes hide and a box says to save first, as git sees
    only the disk.
  - Enter, `]` and `[` no longer open the file or move between changes in
    a changes tab: they type. Alt+F5 and Alt+F3 move.
  - Not yet: boxes inline.
  - Tests: the changes tab tests at the end of
    `crates/core/tests/workbench.rs`.
  - Checked in the window: Alt+I, a change's box, «, and Undo.
- **A change opened in place from the gutter, built** (Git.dc.html):
  - A click on a git sign (`mouse.rs`, `sign_line`) opens it:
    `inline_change::ChangePeek`, with the file's runs of changes
    (`diff::runs` of `git_base` and the text), worked out again after
    edits (`follow_change_peek`).
  - It opens in the rows after its last line through the same gap as the
    references peek: `peek_gap` is `references_gap` plus `change_gap`,
    and the row cache's key has both. `TextView.change` is a
    `ChangeView`; the window draws it in `Frame::change_peek`.
  - Stage stages its lines through `compare::stage_parts`, the same
    patches as the changes tab (`line_staged` says whether it's staged,
    for Unstage). Revert edits the text back with `replace_text`, so
    Ctrl+Z undoes it.
  - Alt+F5 and Alt+F3 (`go.next_change`, `go.prev_change`) move the
    cursor between changes in a file, and the open change follows. In a
    changes tab they're `diff.next` and `diff.prev`.
  - Test: `a_gutter_bar_opens_its_change_in_place`.
  - Checked in the window: the click, Stage and Unstage, and the arrows.
- **Push, pull and sync, built** (GitSync.dc.html, the first board):
  - The app reads ahead and behind from `git status --branch`'s first line
    (`sync_state`) and sends `Event::GitSync(SyncState)` after each status;
    a branch with no upstream gets the first remote, to publish to.
  - `GitAction::Sync(SyncKind)` runs push, pull (merging unless
    `pull.rebase` or `pull.ff` is set), pull then push, `push
    --set-upstream`, or a quiet fetch, in `Git::net`: `GIT_TERMINAL_PROMPT=0`,
    ssh in batch mode unless the user set how it runs, its own process
    group, and its pid in `GitNet` so `GitAction::CancelSync` (handled on
    the window's thread, not the git lane) can stop it.
  - What went wrong is read from git's words: rejected, a remote that
    wants a password, conflicts (the files from `git diff
    --diff-filter=U`), or git's first line. A rejected push fetches first,
    to say how many commits. The result is `Event::Synced`, after the
    status; git's words go to the Output panel.
  - The fetch runs every 5 minutes while the window has focus
    (`App::fetch_due`, `window_focused`).
  - Source Control: Commit and the sync button (`views::sync_button`);
    Stage All moved to a `+` on the Changes heading, which is now an
    `Entry` with `heading` set. The status bar has ↓ ↑, which syncs.
  - A reload notice no longer replaces a notice with buttons.
  - Not yet: Resolve opens the first conflicted file, since the merge
    editor isn't built.
  - Test: `the_sync_button_says_what_it_will_do`.
  - Checked in the window with a local bare remote: Push, a rejected
    push and Pull and Push, Sync with a merge, Publish Branch, and a
    conflict with Abort Pull.
- **Blame for the cursor's line, built** (Git.dc.html):
  - After each update the core asks for the current file's blame when its
    text changed since (`blame::ask_blame`, `Effect::Blame` with the
    text); the app waits 400 ms for the asking to stop (`blame_due`), then
    runs `git blame --porcelain --contents -` with that text, so unsaved
    edits show as not committed. `git::parse_blame` and `git::ago`.
  - `Event::Blame` comes with the version it's for and the time; a line
    shows its blame only while the text is still that version
    (`line_blame`), on the cursor's line, without a problem or selection.
    `LineView.blame`; the window draws it faint after the line.
  - Test: `the_cursors_line_says_who_last_changed_it`.
- **The blame column and its card, built** (GitSync.dc.html, the second
  board):
  - Git: Blame (`git.blame`) toggles it for the file (`blame_column`); Esc
    or the status bar's Blame hides it. It takes `blame::BLAME_COLS` at
    the gutter's left (`gutter()` adds `blame_cols()`); the window draws
    `LineView.blame_cell` there and moves the signs and lightbulb after
    it, and the sign click and lightbulb test follow.
  - Each cell: the run's first line has who, how long and the summary;
    the bar's colour is the commit's age among the file's (`age`), and
    lines not committed are in the accent.
  - A rest on the column (`hover_at`) opens the card (`blame_card`,
    `BlameCardView`) and asks `Effect::CommitInfo` for the message's body
    and its size (`git show`); moving into the text closes it. The
    commit's lines get a soft tint.
  - Open Changes asks `Effect::OpenCommit`; the app answers with a
    `merlin.commit` review of the file before and after the commit, which
    only Esc leaves (`blame::COMMIT`). Blame Before This adds the commit
    to `blame_ignore`, passed to `git blame --ignore-rev`. Copy Hash
    copies the whole hash.
  - Test: `git_blame_shows_who_changed_each_line_and_a_card_for_the_commit`.
  - Checked in the window: the column, the card, the tint and Open
    Changes.
- **New, deleted, moved and other files, built** (GitFiles.dc.html):
  - The app's status marks files with conflicts `!`, in neither list, and
    sends `Event::GitMoves`: staged renames from `old -> new`, and an
    unstaged deleted file paired with a new one at least half alike
    (`pair_moves`, `diff::similarity`, at most 20 of each under 512 KB).
  - Source Control: a Merge Conflicts section first (`cx:` rows open the
    file; + marks it resolved), the status bar's conflict count, a move as
    one row (`R` or `R?`, the old path's row hidden and not counted; `+`
    is `GitAction::StageMove`, opening is `Effect::OpenMove`), deleted
    names struck through (`Entry.strike`), and a chip (`Entry.chip`) for a
    mode change, a submodule's commits or `binary`, from the diff's header
    (`FileDiff::mode_change`, `submodule`, `binary`).
  - The menu has Restore for a deleted file (`restore --source=HEAD
    --staged --worktree`, no Undo) and Add to .gitignore for a new one
    (`GitAction::Ignore`).
  - The changes tab: a new or deleted file is one column (`split_rows`);
    the bar has a note (`DiffBarView.note`: new, deleted, moved and how
    alike, a binary's sizes); a binary file (a NUL near its start,
    `changes_of`) is `Event::ShowBinary`, a tab that says its sizes.
  - Not built from the mock-up: images side by side, Diff Anyway for files
    over 2 MB, a submodule's list of commits, Show Ignored, staging part of
    a new file, and the merge editor for Resolve.
  - Test: `new_deleted_moved_and_conflicted_files_show_as_what_they_are`.
  - Checked in the window: each kind of row, opening a move, a deleted, a
    binary and a new file, and Restore.
- Not in the mock-ups' order but still to do: the changes tab's boxes
  inline.
- **Merge state in Source Control** (from
  [`mockups/MergeFlow.dc.html`](mockups/MergeFlow.dc.html)):
  - The app reads what git is doing from its folder (`Git::merge_state`:
    `MERGE_HEAD` and `MERGE_MSG`, `rebase-merge/` or `rebase-apply/` and
    their `head-name`, `CHERRY_PICK_HEAD`) and counts each conflicted
    file's `<<<<<<< ` markers; `Event::GitMerge` comes with every status.
  - The core keeps it in `git.merge` (`editor/merge.rs`): a merge starting
    fills the commit box with git's message, and ending clears it unless
    it was edited.
  - Source Control's top is then a banner (`Node::Banner`) with Abort
    (`GitAction::AbortPull`, which tries merge, rebase, then cherry-pick),
    the message, and a wide Commit Merge (`Node::Wide`), faded and saying
    what's left until the conflicts are gone. A rebase or cherry-pick has
    Continue (`GitAction::Continue`, `GIT_EDITOR=true`) and no message.
  - Conflicted rows say `2 left` (`Entry.note`); the status bar says
    `Merging: 3 conflicts in 2 files` in red (`StatusView.error`).
  - Test: `a_merge_shows_what_is_coming_in_and_waits_for_its_conflicts`.
  - Checked in the window: a real merge's banner, staging both files,
    Commit Merge with git's message, and Abort Merge.
- **The merge editor** (`editor/merge_editor.rs`):
  - A conflicted row or the notice's Resolve calls `open_merge`: the file
    opens and `Effect::MergeSides` asks the app for git's base version
    (`:1:`) and the sides' names; `Event::MergeSides` starts it once the
    file is open (`pending_merge`, also tried after `Event::Opened`).
  - `Merging::new` parses the markers into Incoming's and Current's whole
    files and each `Conflict`; each base is Current's lines found in the
    base version by a line diff (git's `--diff3` splits conflicts
    differently, so it isn't used). The Result's text is the file with
    each base in place.
  - `follow` keeps each conflict's Result lines through edits; one typed
    in is `edited`, and one undone back to its base or its boxes is that
    again, so Ctrl+Z works. `apply_conflict` writes a choice as one edit.
  - The view: `TextView.merge` (`MergeView`: two `MergePaneView`s of
    `MergeRow`s, from the line level with the Result's top, and the
    Result's `MergeNote`s, one gap row under each conflict via
    `merge_gap`); the Result's lines are tinted by `merge_style`
    (`Style::MergeIncoming`, `MergeCurrent`, `MergeBase`);
    `WindowView.merge_bar`; `TabView.merging`, `GroupView.merging`.
  - Boxes and links are `UiEvent::Press("merge:…")`; the bar's are the
    `merge.*` commands, also in the palette.
  - The window: `GroupArea::split_for_merge` makes the group's text the
    Result's code, under the two panes; `Frame::merge_editor` draws the
    panes, titles and notes, and `merge_bar` the bar.
  - Tests: `the_merge_editor_shows_both_sides_over_the_result_and_takes_lines_from_each`,
    and the parser's in the module.
  - Checked in the window: a real conflict from Source Control, boxes on
    both sides, and Complete Merge saving and staging it.
- **Finishing a merge:** Complete Merge with conflicts left asks
  (`Ask::MergeLeft`: Complete Anyway, or Keep Resolving, the default).
  `finish_merge_file` saves quietly and stages once saved
  (`merge_staging`, in `after_save`, so git never stages the old file),
  then its notice offers the next conflicted file (`merge.open:<path>`)
  or, with none left, Commit Merge (`merge.commit`) with a green dot
  (`Message.done`, `ToastView.done`). Checked in the window across two
  files, to the merge commit.
- **The merge editor, narrow:** under `SIDE_BY_SIDE` (125) columns of
  Result, or by `merge_inline` (Alt+I, `MERGE_KEYS` under
  `isInMergeEditor`, or a block's Side by side), it's the Result alone
  (`GroupView.merge_inline`, `split_for_merge(.., inline)`). Each
  conflict not chosen yet opens a block of rows over its base
  (`block_line`, `block_rows`; `MergeNote.sides` with `MergeRow::Title`),
  drawn by `Frame::merge_editor`; chosen ones keep their note. The bar is
  shorter (`MergeBarView.inline`). A conflicted file opened as text shows
  Open in Merge Editor (`MergeBarView.offer`, while it has markers, so a
  file just completed doesn't flash it). Test:
  `a_narrow_merge_editor_shows_both_sides_in_place_and_alt_i_switches`.
  Checked in the window: Open in Merge Editor, the Result alone, Side by
  side and Alt+I back, Accept in place, Complete.
- **Compare Files** (`editor/compare_files.rs`): a review of kind
  `COMPARE` (`COMPARE_REVIEW`), its id what's on the left (a path,
  `clipboard:` or `saved:`), its path the right file. It shares the
  changes tab's view and bar (`in_compare` next to `in_changes_tab` for
  typing through the file, the bar, Alt+F5/F3/I and Inline), with no
  staging (`DiffBarView.compare`) and ⇄ Swap for two files
  (`DiffBarView.swap`, `compare.swap`, which replaces the tab via
  `compare_replace`). `Comparing` tracks the two-step picker (the palette's
  file pick goes to `compare_picked` first; Esc stops it), the clipboard,
  and texts read with `Effect::ReadFiles` (`compare_files_read`); open
  files' texts are used as they are. Titles over each half come from
  `GroupView.compare`, drawn by `Frame::compare_titles` above the code
  (`GroupArea::under_titles`). The Explorer's menu has
  `explorer.select_compare` and `explorer.compare_with` (labelled with the
  chosen file's name). Tab icons go by the tab name's first word.
  - Both sides edit: a click records the half (`compare_on_left`, in
    `mouse.rs`), the caret shows there, and `through_file` maps the
    cursor through `old_numbers` into the left file (opened beside), then
    rebuilds from it and the right file (or `new_text`). A removed line
    is always the left's. The clipboard and saved sides are read-only.
  - Test: `compare_files_asks_for_two_files_and_shows_them_side_by_side`.
  - Checked in the window: Compare Files…, side by side, typing on the
    right, Swap, the Explorer's menu, Compare with Saved and with
    Clipboard.
- Git's mock-ups are all built, but for what's listed as not built above.

### Tasks and tests (`Tasks.dc.html`, `TaskPlugins.dc.html`)

Built, each checked in the window on a small cargo project:

- **Run Task** (`e2c7ad8`, `crates/core/src/editor/tasks.rs`). `task.run`
  reads `Cargo.toml`, `package.json` and `.merlin/tasks.toml` with
  `Effect::ReadFiles` each time (`tasks_read`; the parsers `from_cargo`,
  `from_package`, `from_tasks_file` are small and hand-written: core has
  no TOML or JSON crate). The palette's `Source::Tasks` shows
  `PaletteView.extras`: where each is from, a plugin's chip, group
  headings, Configure tasks… dimmed. A task runs with `start_command`,
  in a terminal of its own named after it (`TermOp::Spawn.command`,
  `TermSession.task`), replacing its last run; the host runs it under
  `/bin/sh -c` after a dimmed `❯ command` line, and on exit sends
  `Event::TaskOutput` with all the terminal printed (`text_of`), read
  after alacritty drains the pty (its `Exit` event, not `ChildExit`).
  Keys: Ctrl+Shift+B the build task, Ctrl+; A the test task. The mock-up
  had Ctrl+Shift+T, which is Reopen Closed Tab. Test:
  `crates/core/tests/tasks.rs`.
- **Busy terminals** (`ff78769`). alacritty wakes the window only after
  it has read all a pty has, so output that never pauses showed in
  half-second jumps. `Terminals::poll_busy` looks at each screen's damage
  every frame while output comes (`Terminals::due`).
- **Tests** (`04d6dd3`, `crates/core/src/editor/testing.rs`). The Tests
  view (`TESTS`, a rail item) finds tests once: the file list
  (`self.tree` or `Effect::ListTree`), then every `Cargo.toml` and Rust
  file in a `src/` or `tests/` folder read, scanned by `rust_tests`
  (`#[test]` and `#[…::test]`). Roots are crates, then files, then
  tests; results are keyed `path::fn`. Runs go through `run_tests`
  (`Scope`): one terminal, "cargo test", with `-p`, `--test` and names
  as filters. `tests_output` reads cargo's output (`parse_report`:
  targets, `test … ok/FAILED`, failures captured or not, times) and
  `match_test` maps each to a file. A test task from Run Task is read the
  same way. In the editor, `doc_tests` (cached by doc, version and
  `Tests.version`) gives each test a lens row before its `fn` and a
  failure row under the line that failed; they're gap rows like the peek
  views (`test_gap` in `peek_gap`, `has_test_rows` in `row_based`, and
  `Tests.version` in the row cache's key). `Frame::test_rows` draws them
  and the gutter's ✓ ✕. Debug runs the test under rust-gdb, as cargo's
  runner, in a terminal of its own: there's no debugger UI yet. The
  status bar says how many failed; Ctrl+; E, F, C rerun failed, the file,
  the test at the cursor. `Node::Tools` is a row of small buttons.
- **From plugins** (`1127a31`). New imports `set-tasks`, `set-tests`,
  `set-test-results`, `run-task`; no new exports, API still 0.8. A test
  with a `command` runs in the test terminal and is read as cargo's; one
  without is `run-command` with `run-test:<id>`/`debug-test:<id>`, and a
  task without one is `task:<name>`. A plugin's tests in a Rust file
  replace `rust_tests` there; elsewhere they're a root named after the
  plugin. The Plugins view adds "Tests N · Tasks M" (`plugin_gives`).
- **rust-analyzer** (`80ba934`). The lsp plugin asks
  `experimental/runnables` with the rest of `describe`, sending tests
  with rust-analyzer's cargo command (less `--nocapture` and
  `--show-output`) and binaries and benches as tasks.
- **Problems from a task's output** (`crates/core/src/editor/matcher.rs`).
  When a task's output comes (`task_output`), `problems` reads rustc's
  header and `-->` line, `path:line:col: [error|warning|note:] message`,
  TypeScript's `path(line,col): error TS…:` and a Python traceback's last
  frame in the project. They go into `Editor.diagnostics` under
  `task:<name>`, replacing that task's last run, with the task as their
  source and rustc's codes linked. One a language server already shows
  on that line with that message is left out. `place_marks` widens a
  task's one-character problem to its word (`word_end`). Test:
  `a_tasks_errors_become_problems_until_it_runs_clean`.
- Not built: a python or jest plugin giving tests (the API is ready for
  one), and buttons on a plugin's notice.

### Hot exit (`HotExit.dc.html`)

Built on the crash backups (`disk::Backups`), checked in the window:

- **Quitting** (`app.quit` → `quit_asked`, `crates/core/src/editor/hot_exit.rs`).
  With `files.hot_exit` on (the default) and unsaved files, it quits
  without asking and `Editor::keeps_unsaved` says so. `main` then writes
  the backups at once (`Backups::plan`, then `Backups::keep`) with a
  `.hot-exit` file beside them; otherwise a clean exit clears them as
  before. With the setting off, `Ask::Quit` asks as it did.
- **Backups** now head each file with its path, a tab and the file's
  modified time on disk in nanoseconds, as the app last saw it
  (`App.disk`). Older backups, with the path alone, still read.
- **The next start.** `Backups::left_over` returns the files and whether
  `.hot-exit` was there. The workspace's tabs open first, then each
  backup is `Event::Recovered { moved }`, `moved` when the file's time on
  disk isn't the one kept; then `Event::RecoveredAll { crashed }` gives
  the notice: "N files with unsaved changes are back, as you left them."
  [Show], or after a crash, with an amber dot (`ToastView.warning`),
  [Discard Them] [Show]. `recovered` keeps the tab in front unless it's
  the blank scratch file; `Editor.restored` keeps each file's disk text.
- **Show** is a palette (`Source::Restored`): files first, each with
  "+N −M" lines changed; picking one opens Compare with Saved.
  **Discard Them** asks (`Ask::DiscardRestored`), then puts each file back
  as on disk and closes untitled ones.
- **Changed on disk while closed.** `GroupView.away` gives the bar over
  the code (`GroupArea::under_away`, `Frame::away_bar`), with Compare,
  Use the Disk's and Keep Mine (`UiEvent::Press("away:…:<id>")`).
- Test: `crates/core/tests/hot_exit.rs`. Not kept: undo history, and an
  untitled file's place among the tabs (it comes back at the end).

### Settings editor (`Settings.dc.html`, `SettingsPages.dc.html`)

Built and checked in the window, with a copy of the user's config
(`XDG_CONFIG_HOME` pointed at the scratchpad) so nothing touched theirs:

- **A page tab.** `settings.open` (Ctrl+,) opens `Doc::page` named
  `SETTINGS_PAGE` and sends `Effect::OpenSettingsWindow { plugin }`; a
  click in the Plugins view sends it with the plugin. `GroupView.page`
  says which group shows it; page docs have no text views. The window
  draws it with `Frame::settings_page` (`crates/app/src/gui/settings.rs`)
  from `App::settings_view` (`crates/app/src/prefs.rs`), and clicks come
  back as `Hit::Settings(SettingsAction)`. While the tab has the
  keyboard, `Gui::send_key` gives keys to `App::settings_key` first;
  Ctrl and Alt keys go on to the editor, except while catching keys.
- **The model.** `prefs.rs` had been written in a WIP commit (`313e655`)
  and never compiled; it's now `mod prefs`. `App.loaded` keeps the merged
  table and each file's on its own, refreshed by `load_settings`, so
  drawing reads no files. Every row is in `App::sections`; a row knows
  whether the file being changed sets it (Modified, ↺), whether the
  workspace's does (Workspace, and a note saying what the other file
  says). Writes go through `Files::set_in` and `remove_in`.
- **Appearance.** Theme cards drawn in each theme's palette
  (`App.previews`); the pointer on one previews it (`Gui::hover_settings`
  → `App::preview_card`), as do the arrow keys (`SettingsUi.card`), and
  a click picks it. Follow the system writes
  `editor.theme_follows_system`; a light card then goes to
  `editor.theme_light`. `App::chosen_colours` picks by
  `App.system_light`, which `check_system_theme` asks the desktop portal
  for (`busctl … org.freedesktop.appearance color-scheme`, else
  `gsettings`) when the window gets focus. Icon theme cards, and layouts
  stacked as `editor.packs` with ↑, ↓, ✕ and Add a layout.
- **Keyboard Shortcuts.** All, Changed, Plugins and Conflicts (keys two
  commands have where the settings or a plugin gave one). Double-click or
  Enter catches keys (`SettingsUi.capture`, a chord of two at most) and
  says what has them now; Add writes `[keys]`, Replace also takes the
  command's others off (`""` for merlin's own). Record Keys finds what a
  key runs. ↺ (`reset_key`) takes the settings' lines for the command and
  its default keys out.
- **Plugins.** Each with its switch and memory (its own plus its
  programs', read from `/proc` at most every 3 s, `prefs::Memory`; the
  window redraws on that while the page shows). The page has what it's
  allowed, what it costs, what it adds, its own settings (booleans
  switch; other values are shown), Reload, Output and Uninstall.
- **Search** covers every page's rows and the keys; **a broken file**
  shows a banner with the file and line (`Files::load` now says the line).
- The Theme Packs panel is gone (`looks.rs`); its palette command too.
- Not done: plugins' settings
  other than on and off can't be changed in the tab; the follow switch
  between light and dark wasn't seen happen, since the desktop wasn't
  changed.

### Debugging (`Debug.dc.html`, `DebugFlow.dc.html`)

All of it comes from plugins, so a language plugin gets debugging by
declaring a debugger, with no code in merlin for its language.

- **Plugin API.** Manifests declare `[[debuggers]]` (id, name, languages,
  command, install, launch, driver). New host imports: `set-debug-targets`
  for anyone; `debug-start`, `breakpoints`, `set-debug-state`,
  `set-threads`, `set-stack`, `set-variables`, `set-breakpoint-states`,
  `set-stop-filters`, `set-inline-values` and `debug-output` need the new
  `debugger` permission. No new exports: the toolbar and keys arrive as
  `run-command` (`debug.step-over`, `debug.expand:<ref>`,
  `debug.frame:<id>`, `debug.stop-on:<id>`), the console as `on-submit`
  `debug-console`. A `debugger` plugin may spawn only the command of the
  run the user started (`State.debug_start`).
- **Host.** `manifest_events` sends a manifest's languages and debuggers,
  for data-only plugins and on activate. A plugin waiting for its
  permission hasn't sent them yet, so its debuggers appear once allowed.
  `PluginHost::set_debug` hands the driver what to start before each
  `Effect::Debug`.
- **Core** (`editor/debug.rs`). `Debugging` holds debuggers, which
  programs are on `PATH` (`Effect::LocateDebuggers`), plugins' targets,
  breakpoints, the `Session` and the console. F5 picks a target the first
  time (a picker with plugin `merlin.debug`), runs its `build` as a task,
  then sends `debug.start` to the debugger's driver (`dap` unless the
  manifest says). The driver's reports come back as `Event::Debug`
  (`DebugUpdate`). Keys while a run is on: F5, F6, F10, F11, Shift+F11,
  Shift+F5, Ctrl+Shift+F5 (`debug_key`, before the keymap). Restart stops
  and starts again in the core, so the build runs too. Breakpoints go in
  the lightbulb's room, which shows wherever a debugger is declared for
  the language (`can_break_here`); F9 too. `LineView` has `breakpoint`,
  `paused` and `values` (the frame's locals the line names, only while
  paused). Views: Run and Debug (`DEBUG`, left) and Debug Console
  (`CONSOLE`, bottom). `WindowView.debug` is the toolbar.
- **Debugger plugin** (`plugins/dap`). Initialize, launch (or attach with a
  `pid`), breakpoints and exception filters on `initialized`, then
  `configurationDone`. A stop asks threads and the stack; the first frame
  in the project gets its scopes, and the first scope's variables. Frames
  outside the workspace are library ones and fold in Call Stack.
- **Language plugins.** Rust and C/C++ declare lldb-dap and gdb
  (`gdb --interpreter=dap`, gdb 14 and later); Python declares debugpy and
  offers the open file. The lsp plugin lists a crate's binaries from
  rust-analyzer's runnables (`debug_targets`).
- **Window.** The toolbar floats at the top of the code (`debug_bar`, with
  drawn step icons in `step_icon`); the status bar takes the warning
  colour; dots, the paused band and arrow, and value chips are in
  `code.rs`.
- Checked with gdb 17 on a small crate: build, pause on a breakpoint,
  step, continue, the console, stop.
- **The hover card while paused** starts with the variable's value as a
  code block in the file's language, with what's in it unfolded under it
  (`hover_variable`, `hover_value_lines`), then the language server's part.
- **launch.toml.** The app reads `.merlin/launch.toml` (`launch_configs`
  in main.rs) on `Effect::ReadLaunch`, before each F5 and when Run and
  Debug shows; `[[debug]]` tables with name, program, args, build,
  debugger and language, every other key passed to the debugger, and
  `attach = <pid>` as `pid`, which the Debugger plugin attaches with. A
  config with neither debugger nor language takes the open file's.
  Debug: Add Configuration opens it with an example when new.
- **Kept with the workspace:** `break` lines (with `break-if`, `break-hit`,
  `break-log`) and `debug <target>`; `Event::RestoreBreakpoints` brings
  them back when merlin opens named files instead of the last tabs.
- **Breakpoint menu and box.** A right-click in the breakpoint room
  (`break_line`) opens `break_menu`: Add Breakpoint, Add Conditional
  Breakpoint…, Add Logpoint…, or on one Edit, Disable or Enable, Remove
  (`BREAK_ITEMS`). The box (`BreakBox`, `PopupKind::Breakpoint`) sits under
  the line like the rename box: Tab goes through Condition, Hit Count and
  Log Message, Enter sets it, Esc leaves it. gdb's hit count skips that
  many hits and stops on the next.
- **Values after lines** come from every scope the debugger doesn't mark
  `expensive`, and only from the paused function's lines
  (`function_start`, which looks up for `fn`, `def` or `function`).
- The Debugger plugin maps the debugger's breakpoint ids to merlin's, so a
  `breakpoint` event updates that one; sending all again reset gdb's hit
  counts. A `pending` breakpoint counts as set.
- **Panics.** A manifest's debugger may list `stop-on` entries, each
  functions to break in (`FunctionStop`): Rust's is "Rust panics" on
  `rust_panic` and `__rustc::rust_panic`, as newer Rust names it, on by
  default. The Debugger plugin sends them with `setFunctionBreakpoints`
  and lists them with the debugger's own filters; a stop in one is
  reported as an exception, its message taken from the last
  `panicked at` the program printed (`panic_message`). Paused on an
  exception in the shown file, `PopupKind::Exception` is the card under
  the line (`exception_card`), with Continue and Copy, until a hover
  takes its place.
- **Steps skip library code.** A step's stop is held until its stack
  comes: if the top frame is a library one and project code is below,
  the plugin sends `stepOut`, and if that lands back on the line the step
  began on, `next`; up to 30 times. So each F10 or F11 lands on a project
  line.
- **Left:** attaching by picking a process, Add to Watch and Set Value,
  Debug the Last Build, Debug on a Rust test, threads' Pause, the
  toolbar dragged.

### Memory and CPU (2026-11-06)

Measured with merlin on its own repo and the bundled plugins
(`/proc/<pid>/smaps_rollup`, CPU from `/proc/<pid>/stat` over 30–60 s):

- **Fonts were 270 MB.** cosmic-text keeps every font it opens mapped, and
  for a character in no font its fallback tries for, it opens every font
  installed (2,554 files with Noto). `gui/fonts.rs` now reads the fonts
  once for both font systems, and its `Fallback` keeps cosmic-text's
  script and common lists but forbids the last resort every family but
  broad, symbol, emoji and icon fonts (`LAST_RESORT`, `is_icons`). A face
  counts as wanted by any of its names: Nerd Fonts are also "… NF".
- **A terminal kept the window awake** at 60 Hz, 6% CPU, once it had
  printed anything: alacritty's `damage()` always reports the cursor's
  cell, so `poll_busy` never saw it go quiet. It now ignores that cell
  (`is_output`).
- Now: 77 MB PSS idle after start (RSS 107), 85 MB after use with a
  terminal open, about 30 MB of it private; 0.00% CPU idle and 2 wakeups
  a second. The caret's blink costs about 3% CPU for its first 10 s.
- Without a Nerd Font the core's Nerd icons are boxes, as they were
  before; Glyph icons: Plain is for that.

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
  - `cargo test --workspace`: 401 tests passed after the memory and CPU check
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
