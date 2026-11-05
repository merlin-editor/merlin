# merlin

A lighter VS Code: a native Rust code editor in its own window, with VS
Code's layout and keys, that stays small and quiet. It starts from
[kestrel](../kestrel), a terminal editor, and keeps its core, plugin host and
plugins as they are; only the frontend is new. kestrel tested two ideas from
looking at [ttt](https://github.com/eugenioenko/ttt), and merlin keeps both:

1. **Most editor bugs are state bugs.** The core is a reducer with no IO:
   `Editor::update(Event) -> Vec<Effect>`. It hands the window plain data
   to draw, so tests drive it with scripts and check that data, or a text
   dump of it.
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
| `plugins/lsp` | Language servers: diagnostics, hover, go-to-definition, completion, code actions, format, rename, references. |
| `plugins/cpp` | C and C++: the languages and their colours. |
| `plugins/merlin-themes` | Eight colour themes, as data. |
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

The core owns all state and behaviour, and the window draws what it says:
`Editor::window_view` for the frame (tabs, breadcrumbs, the rail, each
side's panel nodes, the status items, the palette, notices and dialogs) and
`Editor::text_views` for each group's code, with its scrollbar marks and
minimap. The window lays the groups out in pixels and tells the core how
many rows and columns each has room for (`Event::Viewports`). Clicks on the
frame go back as `Event::Ui`, the mouse in the code as `Event::Pointer`,
and the wheel and touchpad as `Event::Scroll`, all in rows and columns and
parts of them. Built-in views and plugin panels are the same panel nodes,
so one renderer draws them all.

It uses winit for the window, wgpu for drawing (or softbuffer on the CPU,
with `MERLIN_RENDERER=cpu`, or when there's no GPU) and cosmic-text for
fonts. There's no web view. It sleeps between events.
Output from a plugin's process wakes it straight away, and a check every
half second looks for files changed on disk and rebuilt plugins. Every key
VS Code uses reaches the editor, Ctrl+Shift included.
`MERLIN_ISLANDS=off` lays the panes edge to edge instead. `MERLIN_FONT` and
`MERLIN_FONT_SIZE` set the code's font, over the settings' `font_family` and
`font_size`.

Measured on Linux (Hyprland) in a 948×1141 window, release build:

| | Memory (RSS) | Idle wakeups |
|---|---|---|
| The bundled plugins, a file open | 155 MB | 7 per 10 s |

The first start after a plugin changes compiles it, and takes about 100 MB
more until the next start.

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

The scrollbar is VS Code's: its ruler marks changes, search matches,
selections and problems down the whole file, in lanes, and its thumb fades
in while you scroll or the mouse is near and out a second after. Click the
track to jump there, or drag the thumb. Long lines get a scrollbar along the
bottom too, and a shadow under the tabs shows the code is scrolled.

View › Minimap shows the file left of the scrollbar in tiny blocks of its
colours, with selections, matches, problems and changes marked across it.
The mouse on it shows a slider over the lines in view; click or drag it to
scroll. A long file's minimap scrolls with the view. It's off at first;
toggling it saves `editor.minimap` in your settings (`MERLIN_MINIMAP=on` or
`off` overrides it), and it folds away in a narrow group.

View › Word Wrap (`Alt+Z`) wraps long lines at spaces instead of scrolling
sideways; continued rows keep the line's indent, and Up and Down move by
row. It's saved in your settings too.

View › Split Editor Right (`Ctrl+\`) opens the file again beside it, and
Split Editor Down under it, as VS Code's editor groups do: up to four, each
with its own tabs. A file open in two groups shares its text, but each keeps
its own cursor and scroll, and an edit in one moves the other's cursor along.
Files open in the focused group; `Ctrl+1`, `Ctrl+2` and `Ctrl+3` or a click
move between groups. Drag a tab along the tab row to reorder it, or onto
another group to move it there; a group whose last tab goes closes. Drop
it on a side of a group's code, which shades where it would go, to split
there, as you can a file dragged from the Explorer. A middle click closes
a tab, a double click on a strip's empty end opens a new file, and the
wheel scrolls a strip whose tabs don't fit. A tab's menu pins it: pinned
tabs stay first, show only their icon, and Close Others, Close Saved and
Close All leave them open.

Drag the edge between two groups to resize them. Closing a file with
unsaved changes, or quitting with some, asks first: Save, Don't Save or
Cancel. File › Open File… (`Ctrl+O`), Save As… and Open Folder…
(`Ctrl+K Ctrl+O`) use the desktop's own file dialogs; opening a folder
starts merlin there. In the Explorer, New File, New Folder and Rename name
the file in its row, and Delete asks before moving it to the trash.

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

## Themes and packs

merlin's look is a stack of **packs**, as Minecraft's is of resource packs.
A pack is a TOML file that sets any of colours, icons, layout and fonts, and
each layer changes only what it sets. From the bottom up:

1. merlin's own look,
2. the colour theme (Preferences: Color Theme),
3. the icon theme (Preferences: File Icon Theme),
4. the packs in `[editor] packs`, the first of them on top
   (Preferences: Theme Packs switches them on and off and puts them in
   order).

The pickers show each choice as you move through them, Enter keeps it and
Esc goes back. Packs come from plugins, as data: a pack plugin has no
component and asks for nothing. Saving a pack in use shows the change at
once, so you can write one while looking at it.

merlin comes with:

- [`plugins/merlin-themes`](plugins/merlin-themes): sixteen colour themes
  after Japanese scenes, three of them light and one with high contrast.
- [`plugins/merlin-icons`](plugins/merlin-icons): the Origami and Outline
  icon themes, and Line Icons, a pack with only the frame's icons.
- [`plugins/merlin-layouts`](plugins/merlin-layouts): Compact, Flat, Soft,
  Large Code and Mono Interface, small packs to layer over any theme.

```toml
# plugin.toml
name = "my-looks"
title = "My Looks"
api = "0.8"

[[themes]]          # shows in the colour or icon theme picker
label = "My Theme"
file = "my-theme.toml"

[[packs]]           # a layer for the Theme Packs panel
label = "My Icons"
file = "my-icons.toml"
```

```toml
# my-theme.toml: say what you care about; the rest follows from it.
[ui]
island = "#1b1d24"     # the side bar, editor and panel
text = "#d8dae0"
accent = "#8fb3ff"

[code]
keyword = "#c9a7f0"
string = "#a8d58c"

[layout]
radius = 12            # corners of the islands
gap = 6                # space between them
density = "compact"    # or "comfortable"

[fonts]
ui = "Inter"
code = "JetBrains Mono"
code-size = 14

[icons]
style = "badge"        # for files with no icon: "badge", "glyph" or "none"

[icons.files]          # by extension
"rs" = "icons/rust.svg"                          # an SVG in its own colours
"toml" = { svg = "icons/gear.svg", tint = "muted" } # as a shape in one colour
"md" = { badge = "MD", colour = "#7fb0ff" }

[icons.names]          # by file name
"Cargo.toml" = { glyph = "\ue7a8", colour = "#d77a4f" }

[icons.folders]        # "default", "open", a name, or "<name>-open"
default = "icons/folder.svg"
open = "icons/folder-open.svg"

[icons.ui]             # the frame's icons; they follow their buttons' colours
search = "icons/search.svg"
```

The frame's icons are `explorer`, `search`, `source-control`, `outline`,
`plugin`, `menu`, `layout-left`, `layout-panel`, `layout-right`, `split`
and `more`. A tint is `"current"` (the colour of the spot), `"own"`, a theme
colour's name (`text`, `muted`, `accent`, `keyword`…) or `"#rrggbb"`. All
the colour keys are in [`crates/app/src/theme.rs`](crates/app/src/theme.rs),
and the pack format in [`crates/app/src/packs.rs`](crates/app/src/packs.rs).

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
font_family = "JetBrains Mono"   # the code's, over the packs'
font_size = 13.5
line_height = 1.55    # times the font size below 8, else points
font_ligatures = true
cursor_style = "line"            # line-thin, block, block-outline, underline, underline-thin
cursor_blinking = "blink"        # smooth, phase, expand, solid
smooth_scrolling = true
mouse_wheel_scroll_sensitivity = 1.0
fast_scroll_sensitivity = 5.0    # with Alt held
scroll_beyond_last_line = true
scrollbar_size = 14
minimap_scale = 1                # up to 3
ui_font_size = 13                # the window's own text; the frame grows with it, the code doesn't
bracket_pair_colorization = true
color_decorators = true          # a bar in the colour under #e6c07b, rgb(…) and hsl(…)

[files]
trim_trailing_whitespace = false
insert_final_newline = false
auto_save = "off"                # after_delay, on_focus_change, on_window_change
auto_save_delay = 1000           # milliseconds, with after_delay

[keys]
"ctrl+k ctrl+m" = "view.minimap"   # any command id from the palette; two keys make a chord
"ctrl+d" = ""                      # unbinds a key
# Only when a condition holds, with VS Code's `when` context keys:
"ctrl+l" = { command = "edit.delete_line", when = "editorTextFocus && editorHasSelection" }

[plugins.lsp]               # a plugin's settings, by its name
```

A project's `.editorconfig` files win over these for the files they
cover: `indent_style`, `indent_size`, `tab_width`,
`trim_trailing_whitespace` and `insert_final_newline`. Markdown keeps its
trailing spaces, which break lines, unless an `.editorconfig` says
otherwise. Saving by itself leaves trailing spaces on the cursor's line,
so a space you just typed stays.

`MERLIN_ICONS` and `MERLIN_MINIMAP` override the
settings. Scripts (`--exec`) use the defaults, so their output is the same
everywhere.

Keys, as in VS Code:

| Keys | Does |
|---|---|
| `Ctrl+Shift+P` / `F1` | Commands |
| `Ctrl+P` / `Ctrl+E` | Go to a file (fuzzy; skips hidden and git-ignored files) |
| `Ctrl+O` / `Ctrl+K Ctrl+O` | Open a file / a folder, in the desktop's dialog |
| `F10`, `Alt+F`, `Alt+E`, … | Menus |
| `Ctrl+B` | Side bar |
| `Ctrl+Shift+E` / `Ctrl+Shift+F` / `Ctrl+Shift+G` | Explorer / search the project / Source Control |
| `Ctrl+Alt+B` | Plugin panels; press again to step through them |
| `Ctrl+J` | The panel under the editor |
| `Ctrl+Shift+M` / `Ctrl+Shift+U` | Problems / Output |
| `Ctrl+G` | Go to a line |
| `Alt+Z` | Word wrap |
| `Ctrl+\` / `Ctrl+1` `2` `3` | Split the editor / focus a group |
| `Ctrl+Shift+[` / `Ctrl+Shift+]` | Fold / unfold the block |
| `Ctrl+Shift+O` | Go to a symbol in the file |
| `Ctrl+/` | Comment or uncomment the lines |
| `Ctrl+S` / `Ctrl+Shift+S` / `Ctrl+W` | Save / save as / close the file |
| `Ctrl+PageUp` / `Ctrl+PageDown` | Previous / next tab |
| `Shift`+arrows, `Ctrl+A` | Select |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy, cut, paste (the whole line without a selection) |
| `Tab` / `Shift+Tab`, `Ctrl+]` / `Ctrl+[` | Indent / dedent the selected lines |
| `Ctrl+Left` / `Ctrl+Right` | Move by word |
| `Ctrl+Backspace` / `Ctrl+Delete` | Delete the word before / after |
| `Alt+Up` / `Alt+Down` | Move the lines up / down |
| `Ctrl+Shift+Alt+Up` / `Down` | Copy the lines up / down |
| `Ctrl+Shift+K` | Delete the lines |
| `Ctrl+Enter` / `Ctrl+Shift+Enter` | New line below / above |
| `Ctrl+L` | Select the line, and the next each time |
| `Ctrl+Shift+\` | Go to the matching bracket |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo |
| `Ctrl+D` / `Ctrl+Shift+L` | Add the next occurrence as a cursor / select them all |
| `Shift+Alt+Up` / `Down` (or `Ctrl+Alt`) | Add a cursor above / below |
| `Ctrl+U` | Take back the last cursor added |
| `Ctrl+K Ctrl+D` | Move the last selection on to the next occurrence |
| `Shift+Alt+Right` / `Left` | Expand / shrink the selection: word, inside brackets or quotes, the brackets, the line, the file |
| `Ctrl+K Ctrl+0` / `Ctrl+K Ctrl+J` | Fold / unfold everything |
| `Ctrl+K Ctrl+T` | Pick a colour theme |
| `Ctrl+F` / `Ctrl+H` | Find / find and replace. Lowercase queries ignore case. Enter or Down for the next match, Up for the previous; in the replace field Enter replaces one, `Alt+Enter` all |
| ``Ctrl+` `` / ``Ctrl+Shift+` `` / `Ctrl+Shift+5` | Show or hide the terminal / a new terminal / split it. In a terminal, `Ctrl+Shift+C` and `Ctrl+Shift+V` copy and paste, `Shift+PageUp` and `PageDown` scroll back, and the mouse selects |
| `Alt+C` / `Alt+W` / `Alt+R` / `Alt+L` | In find: Match Case / Whole Word / regular expressions (`$1` in the replacement) / Find in Selection, which a selection over several lines turns on |
| `Alt+Left` | Go back after go-to-definition |
| `Ctrl+Q` | Quit |

In a panel, Up/Down/Tab move between buttons and inputs, PageUp/PageDown
scroll, Esc goes back to the text; in the Explorer, Left and Right fold
folders. The mouse does everything: menus, the activity bar, tabs and their
×, the Explorer, search results, problems, panel buttons, notice buttons,
palette items, status bar items and the scrollbar. It places
the cursor, selects by dragging (Shift+click extends), and scrolls, sideways
with Shift or a sideways wheel, and five times as far with Alt. A touchpad
moves the text exactly as far as your fingers. Alt+click adds a cursor,
Shift+Alt+drag selects a box, dragging after a double or triple click
selects whole words or lines, and a middle click pastes the selection.

Brackets and quotes close themselves, are typed over, wrap a selection,
and go together on Backspace. Enter keeps the line's indent, adds a level
after an opener, and opens a line between a pair; a closing bracket alone
on its line goes back a level. Indentation follows the file (the status
bar shows it), or `tab_size` and `insert_spaces` where it has none.

With several cursors, typing, deleting, moving and selecting happen at
each, as one undo step. Copy takes every selection, a line each, and
pasting as many lines as there are cursors puts one at each. Esc goes back
to one cursor.

Source Control lists what changed since the last commit, staged changes
first, as VS Code does; opening a file there shows its changes as a diff,
side by side when the window is wide enough, and Enter jumps to that line in
the file. `+` beside a file stages it and `−` unstages it, Stage All stages
everything, and a message with Enter or Commit commits what's staged. A
file's arrow (or Right) unfolds its changes, each with its line: Stage or
Unstage one alone, or Discard it. Discarding asks nothing and the notice
has Undo; a file git doesn't track goes to the trash, after asking. A
file's right-click menu opens it or its changes, stages, discards (Ctrl+Del),
copies its path or shows it in the Explorer.

Files changed on disk, by another program or a `git checkout`, reload by themselves,
as one step you can undo. One with unsaved changes asks first: Reload from
Disk (which you can undo too), Overwrite, or Keep Editing, which Enter
picks, since the question comes up while you may be typing. In a git repository, the gutter marks lines changed since the last
commit: `▎` green for added, yellow for changed, `‾` red where lines were
deleted.

The clipboard goes through the window's own connection to the desktop,
Wayland's or X11's. Selecting text makes it the selection that a middle
click pastes, as Linux apps do. Input methods work too: what you're
composing shows underlined at the cursor.

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
`Ctrl+Space` completes, as does typing `.` or `::`. `Ctrl+.` lists code
actions, `Ctrl+Shift+I` formats, and `F2` renames: Enter renames every
place, and Shift+Enter shows them in the bottom panel's Rename tab first,
to leave some out. `Shift+F12` peeks at a name's references between the
lines; `F4` steps through them and Enter opens one.

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
- Git can't push, pull or blame yet, or stage single lines.
- Settings have no editor of their own (they're a TOML file), and the
  theme is VS Code's Dark Modern, with no others yet.
- Completions don't add imports, and renames don't create, move or delete
  files.
- A review is all or nothing per edit; you can't accept some hunks and not
  others. Markdown has no tables or nested quotes.
- The SDK isn't published, so `plugin new` points at the copy in this repo.
- No networking API.
