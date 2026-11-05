# What users want from an editor, and where merlin stands

Research done on 2026-11-05 to decide what merlin builds next. It's here so
anyone working on merlin (person or agent) can pick from it without redoing
the research. When a feature on this list is built, mark it **Built** in the
tables below and add the commit.

## How this was gathered

- **Votes.** The GitHub issue trackers of VS Code (`microsoft/vscode`, label
  `feature-request`) and Zed (`zed-industries/zed`), sorted by 👍 reactions,
  taken with the GitHub search API on 2026-11-05. Votes are the best signal
  available of what many people want, not just the loudest.
- **Surveys.** The Stack Overflow Developer Survey 2025, IDE section.
- **What people love.** Blog posts, forum threads and reviews on why people
  stay with Sublime Text, Neovim and Zed. These are anecdotes, so they're used
  for themes, not rankings.
- **merlin's side.** Checked against the code on `editor-rework` at
  `0e890a1` (commands in `crates/core/src/editor/commands.rs`, the README's
  "Not done yet", and a grep for each feature).

To refresh the vote counts:

```bash
gh api -X GET search/issues -f q='repo:microsoft/vscode label:feature-request is:issue' -f sort=reactions-+1 -f order=desc -f per_page=40 --jq '.items[] | "\(.reactions["+1"])\t\(.state)\t\(.title)"'
```

```bash
gh api -X GET search/issues -f q='repo:zed-industries/zed is:issue' -f sort=reactions-+1 -f order=desc -f per_page=40 --jq '.items[] | "\(.reactions["+1"])\t\(.state)\t\(.title)"'
```

## What users love about their editors

- **Sublime Text: speed, low memory and hot exit.**
  - People who stay say it's responsive on old hardware and light on RAM.
  - They like that you add only what you want.
  - They like having one settings file.
  - They name Goto Anything and multiple cursors as its core strengths.
  - Hot exit means unsaved files survive closing the editor, with no
    prompt. People who leave Sublime miss it.
- **VS Code: everything in one place.**
  - the extension marketplace
  - an integrated terminal
  - git
  - debugging
  - language servers that work out of the box

  It's the most used IDE for five years running (75.9% in 2025), and
  48.9% of developers want to keep working in it. The common complaint is
  that it's heavy (Electron).
- **Neovim: keyboard-driven editing.** It's the most admired editor in the
  2025 survey: 74.4% of its users want to keep using it. It's modal, with
  motions and text objects, and has a which-key style hint for the next key.
- **Zed: native speed and a modern feel.**
  - GPU rendering
  - low latency
  - collaboration
  - AI built in

  Reviewers' caveats are a thin extension ecosystem, incomplete Vim mode
  and uneven language tooling. Its most voted issues are about running and
  debugging code.
- **The theme across all of them:** don't make me fight the editor. It
  should be fast and stable, the keyboard should reach everything, and
  common workflows shouldn't need plugins.

merlin's pitch matches the Sublime and Zed half: a native window, about
155 MB with the bundled plugins, and 7 wakeups per 10 s when idle. The gaps
are mostly in the VS Code half.

## Most voted requests, against merlin

### VS Code (feature-request, by 👍)

| Votes | VS Code | Request | merlin |
|---:|---|---|---|
| 3798 | open | Change the font size and font of the workbench | **Missing.** Only `MERLIN_UI_FONT` (family, env var) and zoom. Quick win. |
| 2876 | built | Floating windows | Missing. Needs a design: big. |
| 2618 | built | Several project folders in one window | Missing. |
| 2411 | built | Tabs for the integrated terminal | **No terminal at all.** Big. |
| 2285 | built | Git status in the Explorer | Has it. |
| 1933 | open | Customise mouse shortcuts | Missing. Niche. |
| 1510 | open | Problems for the whole project, not just open files | Depends on the language server. |
| 1464 | built | 3-way merge for git conflicts | Missing. |
| 1370 | open | Vim mode | Missing. |
| 1370 | open | Settings files that extend others | Missing. |
| 1228 | open | Macro recording | Missing. |
| 1193 | built | Copy left/right in compare views | Reviews only; no general diff view. |
| 1152 | open | Settings per platform | Missing. |
| 1062 | built | Symbols tree view | Has it (Outline). |
| 1056 | built | Minimap | Has it. |
| 1052 | open | Markdown in diagnostics | Panels render Markdown; hovers too. |
| 1048 | built | Use VS Code as merge editor | Missing. |
| 1044 | open | Local workspace settings | Has workspace settings. |
| 1018 | built | Multi-select in the file tree | Missing. |
| 1008 | built | Pinned tabs | Has it (`030bec8`). |
| 1000 | built | Grid layout for editors | Has it (splits in four directions). |
| 945 | open | Several workspaces in one window | Missing. |
| 856 | open | Grouped tabs, as in Chrome | Missing. |
| 810 | open | Pull the terminal out into its own window | No terminal. |
| 747 | built | Indent guides in the Explorer | Check. |
| 739 | open | Tree-sitter highlighting | Has it, through plugins. |
| 722 | open | Moving status bar items | Missing. Niche. |
| 670 | built | File nesting | Missing. |
| 661 | open | Vertical tabs | Missing. |
| 645 | built | Outline in other places | Outline is a side view. |

### Zed (all issues, by 👍)

| Votes | Request | merlin |
|---:|---|---|
| 2137 | Build and debug support | **Missing.** Tasks first, then DAP. |
| 1123 | Rainbow brackets | **Missing.** Quick win. |
| 898 | Smooth scrolling | Has it. |
| 893 | Test runner | Missing. |
| 889 | Telescope-style search box | Partly: the palette and Go to File. |
| 795 | Diff between two files | Missing. |
| 771 | Dev containers | Missing. Out of scope for now. |
| 733 | Sticky scroll | Has it. |
| 692 | Helix keymap | Missing. |
| 686 | Which-key menu | Missing. |
| 650 | Remote development | Missing. |
| 599 | Plugin interface | Has it (wasm). |
| 577 | jj version control | Missing. |
| 564 | Staged and unstaged diffs | Partly: whole files only. |
| 511 | EditorConfig | **Missing.** Quick win. |
| 494 | Colour swatches for hex colours | **Missing.** Quick win. |
| 473 | Easymotion-style jumps | Missing. |
| 467 | Second windows for more monitors | Missing. |
| 400 | Flash.nvim-style search jumps | Missing. |
| 398 | Source control panel | Has it (Changes). |
| 383 | Semantic tokens | Check what lsp does. |
| 382 | Errors shown inline, as Error Lens does | Missing. |
| 377 | Hide the title and status bar | Missing (zen mode). |
| 375 | File icons | Has it. |
| 370 | direnv environment | Missing. |
| 364 | Mermaid in Markdown preview | No Markdown preview. |
| 352 | Emmet | Missing. |
| 345 | Minimap | Has it. |

## What merlin has, and what it lacks

**Has:**
- the command palette and Go to File, Go to Symbol and Go to Line
- multiple cursors and box select
- split groups in four directions, with tabs dragged to split
- pinned tabs
- the minimap, sticky scroll, breadcrumbs, indent guides and folding
- the Explorer, with git status and inline new, rename and delete
- Outline, Problems, Search, Changes (staging and committing whole files)
  and Output
- reviews of proposed edits
- language servers through the `lsp` plugin: diagnostics, hover,
  go-to-definition and completion
- smart-case find and replace
- restoring the workspace
- system file dialogs, the clipboard, the selection clipboard and IME
- dialogs for unsaved changes and changes on disk
- wasm plugins with settings, panels, processes and secrets

**Lacks:** these are found in the code and in the README's "Not done yet".
- **Find:** only smart case. There are no case, whole-word, regex or
  find-in-selection toggles.
- **Editing hygiene:** no EditorConfig, no trimming of trailing whitespace,
  no final newline on save, no autosave and no format on save.
- **Colour:** no rainbow brackets and no colour swatches.
- **UI font size:** no setting. The README notes one font list shared by
  the frame and the code is still to do.
- **Hot exit:** unsaved changes aren't kept across restarts. Killing merlin
  counts as a crash.
- **Language servers:** no rename, format, code actions, inlay hints,
  signature help, find references, or errors shown inline. Completions
  don't add imports.
- **Git:** no staging or discarding of parts of a file, no blame, no push,
  no diff of two files, and no merge editor.
- **Running code:** no terminal, no tasks, no test runner and no debugger.
- **Keys:** no Vim or Helix mode, no which-key, no macros.
- **Windows:** one folder per window. There are no floating or second
  windows.
- **Settings:** no settings editor (they're TOML), and only one theme.
  The `merlin-themes` plugin has eight as data.
- **Markdown:** no preview, and panels have no tables or nested quotes.

## What to build, in order

The user's decision (2026-11-05): build the quick wins straight away. For
everything after them that changes or adds UI, make a **mock-up to review
first**, and only build once it's approved. The mock-ups for list 2 below
were approved the same day. Anything new beyond them still needs a
mock-up first.

### 1. Quick wins (approved; built)

All **Built** on `editor-rework`. How each turned out:

- Rainbow brackets were already built, but only in files a language plugin
  colours. This adds the setting to turn them off.
- Colour swatches are a bar in the colour under the literal, not VS Code's
  square before it. A square would shift the text, which the core's
  column maths for clicks and carets doesn't allow for. Colours close to
  the background get an edge.
- Match Case off still means smart case, as merlin had: a query with
  capitals matches case.
- Regular expressions match within a line.
- EditorConfig's `end_of_line` and `charset` aren't applied yet.
- The UI font size scales the whole frame (`ui_font_size / 13`). The code
  is drawn at the display's own scale, so it keeps its size.

| Feature | Why | Notes |
|---|---|---|
| Rainbow brackets | Zed 1123 | Colour `()[]{}` by depth from the theme's bracket colours. Setting `editor.bracket_pair_colorization`, on by default as in VS Code. Brackets in strings and comments stay as they are. |
| Find toggles | Common to every editor | Match Case, Whole Word and Regex as buttons in the find bar, with Alt+C, Alt+W and Alt+R. Find in Selection with Alt+L. Uses the `regex` crate. |
| EditorConfig | Zed 511 | `indent_style`, `indent_size`, `tab_width`, `end_of_line`, `trim_trailing_whitespace`, `insert_final_newline` and `charset` (utf-8 only), from `.editorconfig` files up to `root = true`. |
| Trim whitespace and final newline | VS Code settings everyone turns on | `files.trim_trailing_whitespace` and `files.insert_final_newline`, applied on save, with EditorConfig taking precedence. |
| Autosave | Users of VS Code and JetBrains expect it | `files.auto_save`: off, after a delay, on focus change, or on window change. `files.auto_save_delay` defaults to 1000 ms. |
| Colour swatches | Zed 494 | A small square before `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()` and `hsl()` in the code. Setting `editor.color_decorators`. |
| UI font size | VS Code's top request (3798) | `editor.ui_font_size` scales the frame apart from the code. |

### 2. Needs a mock-up first

**Approved on 2026-11-05: all of them** ("I love all of it"). There's one
artboard each on the
[merlin feature mock-ups](https://claude.ai/artifact/QzFdCBA1P32LckaUDc419F)
canvas (private to the user), and copies in [`mockups/`](mockups/README.md),
which also says what each shows. Build to them, in the order below, and
mark each **Built** here with its commit.

These are ordered by votes against effort.

1. **Integrated terminal** (**Built**, see `crates/app/src/terminal.rs` and
   `crates/core/src/editor/terminal.rs`; not yet: mouse reporting to
   programs, shell profiles, restoring terminals with the workspace): a
   bottom panel tab, with tabs for each shell
   and splits. Use the `alacritty_terminal` crate for the VT state and a
   pty, drawn by merlin's own renderer.
2. **Language servers, finished**: rename (inline box), format and format
   on save, code actions (a lightbulb and a menu), inlay hints, signature
   help, find references (a peek view or the side panel), and errors
   inline.
3. **Git**:
   - stage, unstage and discard hunks from the gutter and Changes
   - blame, inline and in a gutter
   - push and pull
   - a diff of any two files
   - a 3-way merge editor
4. **Tasks and tests**: run a task from `tasks.toml` or `Cargo.toml`, with
   output in the terminal, problems from it, and a test runner with
   gutter buttons.
5. **Hot exit**: keep unsaved buffers across quitting and crashes. This
   is mostly not UI; it decides only whether to ask on quit.
6. **Settings editor**: the Settings window already planned in
   `ROADMAP.md`, with themes chosen there.
7. **Debugging (DAP)**, as a plugin like `lsp`: breakpoints, a call stack
   and variables, and the debug toolbar.
8. **Vim or Helix mode, which-key and macros.**
9. **Several folders in one window, and floating or second windows.**
10. **Markdown preview**, with Mermaid.

## Sources

- [VS Code feature requests, by votes](https://github.com/microsoft/vscode/issues?q=is%3Aissue+label%3Afeature-request+sort%3Areactions-%2B1-desc)
- [Zed issues, by votes](https://github.com/zed-industries/zed/issues?q=is%3Aissue+sort%3Areactions-%2B1-desc)
- [Stack Overflow Developer Survey 2025, technology](https://survey.stackoverflow.co/2025/technology/)
- [Visual Studio Magazine on the 2025 survey](https://visualstudiomagazine.com/articles/2025/08/01/stack-overflow-dev-survey-visual-studio-vs-code-hold-of-ai-ides-to-remain-on-top.aspx)
- [Why I switched from VS Code to Sublime Text (DEV)](https://dev.to/restoreddev/why-i-switched-from-visual-studio-code-to-sublime-text-28k0)
- [Sublime Text versus VS Code (Sublime forum)](https://forum.sublimetext.com/t/sublime-text-versus-visual-studio-code-in-2019/41375)
- [Zed on Product Hunt](https://www.producthunt.com/posts/962092)
