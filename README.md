# merlin

A lighter VS Code: a native Rust code editor in its own window, with VS
Code's layout and keys, that stays small and quiet. It starts from
[kestrel](../kestrel), a terminal editor, and keeps its core, plugin host and
plugins as they are; only the frontend is new. kestrel tested two ideas from
looking at [ttt](https://github.com/eugenioenko/ttt), and merlin keeps both:

1. **Most editor bugs are state bugs.** The core is a reducer with no IO:
   `Editor::update(Event) -> Vec<Effect>`. It renders to a plain grid, so
   tests drive it with scripts and check the screen as text.
2. **Plugins should be sandboxed, typed and quick to iterate on.** Plugins are
   WebAssembly components. They can call only what's in
   [`wit/merlin.wit`](wit/merlin.wit), each call is checked against the
   manifest's permissions, a call that loops forever runs out of fuel instead
   of hanging the editor, and a rebuilt plugin reloads without a restart.

## Layout

| Path | What it is |
|---|---|
| `crates/core` | Buffer (rope), undo history, editor state, rendering, scripts. No IO. |
| `crates/syntax` | What a piece of code is, for its colour. The editor has no grammars. |
| `crates/plugin-host` | Loads plugins with wasmtime and turns their requests into events. |
| `crates/app` | The `merlin` binary: the window (`gui.rs`) and headless `--exec` mode. |
| `wit/merlin.wit` | The plugin API. |
| `sdk/rust` | `merlin-plugin`, the Rust SDK for writing plugins. |
| `sdk/grammar` | `merlin-grammar`: colour a language in a plugin with a tree-sitter grammar. |
| `plugins/rust`, `python`, `javascript`, `json`, `toml` | Those languages and their tree-sitter colours. |
| `plugins/word-count` | Small example plugin: commands, a key, a panel and a button. |
| `plugins/lsp` | Language servers: diagnostics, hover, go-to-definition, completion. |
| `plugins/cpp` | C and C++: the languages and their colours. |
| `plugins/testbed` | Exercises the plugin API for the host's tests. |

## The window

The window draws its own frame in pixels, as the
[design](https://claude.ai/artifact/DaNmFu3KfwW7wQeJb63dJh) has it. A slim
title bar holds the project, the branch, one search box and the layout
toggles, with the menus behind ☰. Beside it are the activity rail and the
side bar, editor and panel, as rounded islands on a darker ground, and a thin
status bar runs underneath. The interface uses [Inter](https://rsms.me/inter/),
which comes built into the binary (SIL Open Font License, in
[`crates/app/fonts`](crates/app/fonts); `MERLIN_UI_FONT` picks another). Code stays monospace.

The core still owns all state and behaviour. In window mode
(`Editor::set_windowed`) its screen is only the code, and the window draws
everything else from `Editor::window_view`: tabs, breadcrumbs, the rail,
each side's panel nodes, the status items, the palette and notices. Clicks on
the frame go back as `Event::Ui`; the mouse in the code goes back as cells.
Built-in views and plugin panels are the same panel nodes, so one renderer
draws them all.

It uses winit for the window, softbuffer for drawing and cosmic-text for
fonts. There's no GPU context and no web view. It sleeps between events.
Output from a plugin's process wakes it straight away, and a check every
half second looks for files changed on disk and rebuilt plugins. Every key
VS Code uses reaches the editor, Ctrl+Shift included.
`MERLIN_ISLANDS=off` lays the panes edge to edge instead. `MERLIN_FONT` and
`MERLIN_FONT_SIZE` set the code's font.

Measured on Linux (Hyprland) in a 948×1141 window, release build:

| | Memory (RSS) | Idle CPU |
|---|---|---|
| No plugins, cell-grid window | 28 MB | 0.4% |
| The 7 bundled plugins, cell-grid window | 231 MB | 0.2% |
| The 7 bundled plugins, pixel frame | 263 MB | 0.3% |

The pixel frame loads the font list twice, once for the interface and once
for the code, which accounts for most of its extra 30 MB.

Language servers run as their own processes and aren't counted here.
Plugins are where the memory goes, about 27 MB each, so they're the next
thing to make smaller.

## Build and run

```sh
scripts/install-plugins.sh
target/release/merlin src/main.rs src/lib.rs
```

The editor has no languages of its own: they come with it as plugins, as
VS Code's do. The script builds them (and the language server plugin) and
installs them with `merlin plugin install <dir>`, which links a plugin into
`~/.config/merlin/plugins/` and allows what its manifest asks for, so they
load at every start. Without them, files show uncoloured. Plugins compile
once and are cached in `~/.local/share/merlin/compiled/`, so starting with
all of them takes about a quarter of a second.

The window is laid out like VS Code, in flat panes set apart by shade: a
title bar with the menus (File, Edit, Selection, View, Go, Help), a search
box that opens Go to File, and buttons that show or hide the side bar, the
panel and the secondary side bar; the activity bar on the far left; the side
bar with the Explorer, Search and Source Control; the editor with its tabs;
plugin panels on the right; the panel underneath with Problems and Output;
and a status bar whose items act when clicked (the branch opens Source
Control, the problem counts open Problems, the position goes to a line).
Rows, tabs and buttons light up under the mouse.

Icons come from a [Nerd Font](https://www.nerdfonts.com), as VS Code's
codicons and file icons; `MERLIN_ICONS=plain` draws ordinary symbols
instead. Colours are VS Code's Dark Modern. Code gets indent guides, with the cursor's block
brighter, brackets coloured by depth, and other uses of the word under the
cursor marked.

View › Minimap shows a minimap left of the scrollbar, drawn in braille: each
cell holds 2×4 dots, so ten columns show the shape of 80 columns of code,
coloured like it. Search matches light up its cells, errors and warnings
colour their dots, and changed lines are marked beside it. Click or drag it
to scroll. It's off at first; toggling it saves `editor.minimap` in your
settings (`MERLIN_MINIMAP=on` or `off` overrides it). It folds away when
the text area is under 80 columns. The scrollbar marks
search matches too, as well as errors and changes.

View › Word Wrap (`Alt+Z`) wraps long lines at spaces instead of scrolling
sideways; continued rows keep the line's indent, and Up and Down move by
row. It's saved in your settings too.

View › Split Editor Right (`Ctrl+\`) opens the file again beside it, and
Split Editor Down under it, as VS Code's editor groups do: up to four, each
with its own tabs. A file open in two groups shares its text, but each keeps
its own cursor and scroll, and an edit in one moves the other's cursor along.
Files open in the focused group; `Ctrl+1`, `Ctrl+2` and `Ctrl+3` or a click
move between groups. Drag a tab along the tab row to reorder it, or onto
another group to move it there; a group whose last tab goes closes.

Started without files, merlin reopens the tabs, groups and cursors you
left in that folder (kept in `~/.local/share/merlin/workspaces/`).

A single click in the Explorer opens a **preview** tab, in italics, which
the next file you click replaces; editing it, double-clicking it, or opening
with Enter keeps it. Double-click selects a word, triple-click a line.

**Folding** works by indentation, as VS Code's does: `Ctrl+Shift+[` folds
the block around the cursor (again for the block outside it),
`Ctrl+Shift+]` unfolds, and Fold All and Unfold All are in the palette. An
arrow left of a block's first line shows on hover and folds it on a click;
a folded block shows `⋯` after its first line. Moving into one opens it.

**Breadcrumbs** under the tabs show the file's folders and the symbols
around the cursor; click one to go there. **Sticky scroll** pins the first
lines of the blocks in view to the top of the text, never over the cursor.
The **Outline** view in the side bar lists the file's symbols, and
`Ctrl+Shift+O` jumps to one. Symbols are found in the text: functions, types
and the like by the word that declares them, in Rust, Python, JavaScript,
TypeScript, Go and similar languages, and headings in Markdown.

## Settings

Settings are TOML, as VS Code's are JSON: yours in
`~/.config/merlin/settings.toml`, and a folder's in
`.merlin/settings.toml`, which overrides yours. Preferences: Open User
Settings and Open Workspace Settings (in the palette) open them, made from
a commented template the first time; saving one applies it at once. View
menu toggles save to your settings.

```toml
[editor]
minimap = false
word_wrap = false
breadcrumbs = true
sticky_scroll = true
preview_tabs = true
icons = "nerd"        # or "plain"
theme = "dark"

[keys]
"ctrl+k" = "view.minimap"   # any command id from the palette
"ctrl+d" = ""               # unbinds a key

[plugins.lsp]               # a plugin's settings, by its name
```

`MERLIN_ICONS` and `MERLIN_MINIMAP` override the
settings. Scripts (`--exec`) use the defaults, so their output is the same
everywhere.

Keys, as in VS Code:

| Keys | Does |
|---|---|
| `Ctrl+Shift+P` / `F1` | Commands |
| `Ctrl+P` / `Ctrl+O` | Go to a file (fuzzy; skips hidden and git-ignored files) |
| `F10`, `Alt+F`, `Alt+E`, … | Menus |
| `Ctrl+B` | Side bar |
| `Ctrl+Shift+E` (or `Ctrl+E`) / `Ctrl+Shift+F` / `Ctrl+Shift+G` | Explorer / search the project / Source Control |
| `Ctrl+Alt+B` | Plugin panels; press again to step through them |
| `Ctrl+J` | The panel under the editor |
| `Ctrl+Shift+M` / `Ctrl+Shift+U` | Problems / Output |
| `Ctrl+G` | Go to a line |
| `Alt+Z` | Word wrap |
| `Ctrl+\` / `Ctrl+1` `2` `3` | Split the editor / focus a group |
| `Ctrl+Shift+[` / `Ctrl+Shift+]`, or `Ctrl+Alt+[` / `]` | Fold / unfold the block (kitty keeps `Ctrl+Shift+[` and `]` for its windows unless you `map kitty_mod+[ no_op`) |
| `Ctrl+Shift+O` | Go to a symbol in the file |
| `Ctrl+/` | Comment or uncomment the lines |
| `Ctrl+S` / `Ctrl+W` | Save / close the file |
| `Ctrl+PageUp` / `Ctrl+PageDown` | Previous / next tab |
| `Shift`+arrows, `Ctrl+A` | Select |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy, cut, paste (the whole line without a selection) |
| `Tab` / `Shift+Tab` | Indent / dedent the selected lines |
| `Ctrl+Left` / `Ctrl+Right` | Move by word |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo |
| `Ctrl+D` / `Ctrl+Shift+L` | Add the next occurrence as a cursor / select them all |
| `Shift+Alt+Up` / `Down` (or `Ctrl+Alt`) | Add a cursor above / below |
| `Ctrl+F` / `Ctrl+R` | Find / find and replace. Lowercase queries ignore case. Enter or Down for the next match, Up for the previous; in the replace field Enter replaces one, `Alt+Enter` all |
| `Alt+Left` | Go back after go-to-definition |
| `Ctrl+Q` | Quit |

In a panel, Up/Down/Tab move between buttons and inputs, PageUp/PageDown
scroll, Esc goes back to the text; in the Explorer, Left and Right fold
folders. The mouse does everything: menus, the activity bar, tabs and their
×, the Explorer, search results, problems, panel buttons, notice buttons,
palette items, status bar items and the scrollbar. It places
the cursor, selects by dragging (Shift+click extends), and scrolls, sideways
with Shift or a sideways wheel. Hold Shift to select with the terminal
instead. Alt+click adds a cursor.

With several cursors, typing, deleting, moving and selecting happen at
each, as one undo step. Copy takes every selection, a line each, and
pasting as many lines as there are cursors puts one at each. Esc goes back
to one cursor.

Source Control lists what changed since the last commit, staged changes
first, as VS Code does; opening a file there shows its changes as a diff,
side by side when the window is wide enough, and Enter jumps to that line in
the file. `+` beside a file stages it and `−` unstages it, Stage All stages
everything, and a message with Enter or Commit commits what's staged.

Files changed on disk, by another program or a `git checkout`, reload by themselves,
as one step you can undo. One with unsaved changes isn't touched; you're told
instead. In a git repository, the gutter marks lines changed since the last
commit: `▎` green for added, yellow for changed, `‾` red where lines were
deleted.

The clipboard goes through `wl-copy`/`wl-paste`, `xclip` or `pbcopy`, and
falls back to the terminal (OSC 52) for copying when none is there.

Headless, for scripts and bug reports:

```sh
cargo run -- notes.txt --plugin plugins/word-count --size 70x8 \
  --exec 'key ctrl+b; key enter; key ctrl+s'
```

`wait-for TEXT [MS]` waits for plugin process output, and fails the run if it
never appears.

## Other plugins

The OpenCode chat, a port of ttt's, lives in its own repository,
`merlin-opencode`, as an ordinary plugin. merlin doesn't depend on it.

## Language support

Languages work as in VS Code, where extensions add them, and the ones that
come with merlin are plugins too. Plugins do everything a language needs:
add the language (its file extensions, name and comment markers), colour it,
colour pieces of it for diffs and Markdown code blocks, list its symbols and
say where it folds.

- **`rust`, `python`, `javascript`, `json`, `toml`** add those languages
  and colour them with their tree-sitter grammars, through `sdk/grammar`.
  Each change recolours only the lines whose meaning changed: about 3 ms a
  key in a 2,600-line file.

- **`lsp`** runs the language server for the open file (rust-analyzer,
  gopls, clangd, pyright or typescript-language-server, whichever you have
  on `PATH` and allow). Besides errors, hover, definitions and completion,
  it takes the server's semantic colours (over the base colours, as VS Code
  does), its symbols (for the Outline, breadcrumbs and Go to Symbol), and
  its folding ranges.
- **`cpp`** adds C and C++ (`.c`, `.h`, `.cpp`, `.cc`, `.cxx`, `.hpp`,
  `.hh` and the rest) and colours them with a lexer, as a VS Code grammar
  does. With `lsp` and clangd too, you get clangd's colours, symbols and
  folds over it.

```sh
(cd plugins/lsp && cargo build --release)
(cd plugins/cpp && cargo build --release)
merlin --plugin plugins/cpp --plugin plugins/lsp src/main.cpp
```

Copy or link the plugin folders into `~/.config/merlin/plugins/` to load
them every time. clangd needs a `compile_commands.json` (CMake writes one
with `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON`) for anything beyond one file, and
the C++ standard library's headers for the compiler it finds.

Errors are underlined, colour the line number, and show in the status bar
when the cursor is on them; the plugin's panel lists them. `Alt+K` shows
hover, `Alt+D` goes to the definition (`Alt+Left` comes back), and
`Ctrl+Space` completes, as does typing `.` or `::`.

rust-analyzer reports some errors as you type, such as unresolved modules,
and the rest, such as type errors, from `cargo check` when you save. Its
type checks while typing are experimental and off by default.

## Tests

```sh
cargo test
```

The plugin-host tests need the word-count, testbed, lsp and cpp plugins
built first (`cargo build --release` in each folder). The LSP tests run against a
fake server (`plugins/lsp/tests/fake/`), so they need no language server. Several core
tests are named after ttt bugs whose class this design rules out: undo
clearing the unsaved state, nested undo groups, paste going to the wrong
field, stale find matches, panel state reset on redraw, and UTF-16 columns.

## Writing a plugin

```sh
merlin plugin new hello
cd hello && cargo build --release
merlin --plugin hello notes.txt
```

That makes a working plugin with a command, a panel and saved state, using
the Rust SDK in `sdk/rust`. Implement the `Plugin` trait's methods you need;
the rest default to doing nothing. Rebuild while merlin runs and the plugin
reloads within a quarter of a second. Plugins in `~/.config/merlin/plugins/`
load at startup. word-count is a short example; lsp a full one.

The manifest says what the plugin may do:

```toml
name = "word-count"
title = "Word Count"
api = "0.8"
component = "target/wasm32-wasip2/release/word_count.wasm"

[permissions]
editor-read = true
editor-write = true
# Programs it may start. They run with your rights, outside the sandbox.
processes = ["wc"]
```

The first time a plugin asks for permissions, its panel shows what it wants
and it loads only if you allow it. The answer is remembered, by plugin folder,
in `~/.local/share/merlin/grants.toml`, and you're asked again if the plugin
later wants more. `--exec` scripts grant without asking and remember nothing.

What else plugins get, besides commands and panels:

- **Keys.** `Command::new("show", "Word Count: Show").key("ctrl+alt+w")`. A key
  needs Ctrl or Alt; one the editor or another plugin already uses is skipped
  with a warning. The palette shows each command's key.
- **Processes.** Output arrives at `process`, one message at a time: the host
  does the framing (lines, or LSP's `Content-Length`), so the plugin only
  parses messages.
- **State across reloads.** Return bytes from `save_state` and get them back
  in `restore_state` after a rebuild. A plugin that does also keeps its
  processes, so a language server keeps running. One that doesn't has them
  stopped.
- **Secrets**, such as API keys: `host::set_secret`, `get_secret` and
  `delete_secret`, private to each plugin. They go to the system keyring
  through `secret-tool` when it's installed, otherwise to
  `~/.local/share/merlin/secrets/<plugin>.toml`, readable only by you.
  `Node::secret_input` shows dots while typing one.
- **Languages.** `[[languages]]` in the manifest adds one, with `id`,
  `name`, `extensions`, `line-comment` and `block-comment`.
  `host::set_tokens(path, layer, lines, tokens)` colours a file: the
  `Base` layer replaces the editor's own grammar, and `Semantic` goes over
  it. `host::set_symbols` and `host::set_folds` give its symbols and fold
  ranges. All of it moves with edits until the plugin sends more.
  `colour_text` colours a piece of text in the language, for diffs and
  Markdown code blocks. The `cpp` plugin is a short example; with
  `merlin-grammar` a tree-sitter grammar takes a dozen lines (see
  `plugins/rust`), building against `sdk/grammar/wasm-include` instead of a
  system C library.
- **Settings.** Defaults go in the manifest's `[settings]`; the user's and
  the workspace's `[plugins.<name>]` override them. Read them with
  `settings::text("server")`, `settings::flag(...)`, `settings::integer(...)`
  (dotted keys reach into tables), when needed, as they can change.
- **Buffer events**: `buffer` hears when the active file switches, changes or
  is saved.
- **The selection**: `host::active_selection` gives its text and position.
- **Reviews.** `host::show_diff(id, path, old, new)` shows a proposed change
  as a diff in its own tab; the user's answer arrives at `review`.
- **Richer panels**: `Node::markdown` for formatted text, and
  `Node::item` for clickable rows drawn as text, such as a list of problems.

Other languages work too: anything that builds a WASI 0.2 component for
[`wit/merlin.wit`](wit/merlin.wit).

## Not done yet

- A wrapped line taller than the view can't scroll part way. Without a
  language server, folds go by indentation and symbols are found by
  keyword. Only one plugin's symbols or folds count for a file at a time.
- Git can stage whole files and commit, but not stage parts of a file,
  discard changes, push, or blame.
- Settings have no editor of their own (they're a TOML file), and the
  theme is VS Code's Dark Modern, with no others yet.
- Language servers can't rename, format, or apply code actions yet, and
  completions don't add imports.
- A review is all or nothing per edit; you can't accept some hunks and not
  others. Markdown has no tables or nested quotes.
- The SDK isn't published, so `plugin new` points at the copy in this repo.
- No networking API.
