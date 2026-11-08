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
   [`sdk/rust/wit/merlin.wit`](sdk/rust/wit/merlin.wit), each call is checked against the
   manifest's permissions, a call that loops forever runs out of fuel instead
   of hanging the editor, and a rebuilt plugin reloads without a restart.

## Layout

| Path | What it is |
|---|---|
| `crates/core` | Buffer (rope), undo history, editor state, rendering, scripts. No IO. |
| `crates/syntax` | What a piece of code is, for its colour. The editor has no grammars. |
| `crates/plugin-host` | Loads plugins with wasmtime and turns their requests into events. |
| `crates/app` | The `merlin` binary: the window (`gui.rs`) and headless `--exec` mode. |
| `sdk/rust/wit/merlin.wit` | The plugin API. |
| `sdk/rust` | `merlin-plugin`, the Rust SDK for writing plugins. |
| `sdk/grammar` | `merlin-grammar`: colour a language in a plugin with a tree-sitter grammar. |
| `sdk/lsp` | `merlin-lsp`: a language server client a language plugin embeds with its own servers. |
| `plugins/rust`, `python`, `javascript`, `go`, `cpp` | Those languages: colours, language servers, and their tests, tasks and debuggers. |
| `plugins/json`, `toml` | Those languages and their tree-sitter colours. |
| `plugins/catch2` | Catch2 tests for C++ projects, beside the `cpp` plugin. |
| `plugins/word-count` | Small example plugin: commands, a key, a panel and a button. |
| `plugins/dap` | The Debugger: speaks the Debug Adapter Protocol to the debuggers language plugins declare. |
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

## Installation & Updates

### One-line installer / updater (Linux & macOS)

To install or update Merlin to the latest release binary into `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/merlin-editor/merlin/main/scripts/install.sh | sh
```

Running this command again checks for new releases and updates your installation automatically.

Prebuilt binaries for Linux, macOS (Apple Silicon & Intel), and Windows are also available directly from [GitHub Releases](https://github.com/merlin-editor/merlin/releases).

---

## Build and run from source

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
box that opens the palette, and buttons that show or hide the side bar, the
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
unsaved changes asks first: Save, Don't Save or Cancel. Quitting doesn't:
unsaved files are kept, and the next start brings back the same tabs with
their changes, untitled files too, under a notice whose Show lists what
changed. After a crash the same notice offers Discard Them. A file that
changed on disk while merlin was closed says so over its text, with
Compare, Use the Disk's and Keep Mine. `[files] hot_exit = false` asks on
quit instead. File › Open File… (`Ctrl+O`), Save As… and Open Folder…
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

## The palette

`Ctrl+Shift+P` opens one search over commands, symbols, files and the
project's text. With **Everything** on, each has a section with its best
few (four commands, three symbols, two files, a line of text) and a link
to the rest. The chips under the box narrow it to one, as does a sign
typed first: `>` commands, `@` symbols, `/` text. Tab and Shift+Tab move
through them, and Backspace on an empty box widens it again. `Ctrl+P`
opens it on Files. Names with the letters you typed together, as Toggle
Word Wrap has "wrap", come before ones with them scattered.

Each row says what it is: a command's keys and what it does, a symbol's
kind, what it's in and its line, a file's folder, a line of text with its
match tinted. Symbols come from the open files at once and from the
whole project through the language server, a moment after typing stops,
as text does from the project search. Empty, the box lists the files
opened and commands run last, kept with the workspace.

Moving through the results shows each in the editor behind, in the
preview tab if its file isn't open; Esc goes back to where you were and
Enter keeps it. `Ctrl+Enter` opens a result to the side, and `Alt+Enter`
moves a text search to the Search view. A command with a lit `›` offers
more: → or a click opens it, as Fold's levels (Fold Level 1 to 5), your
saved macros, or the files you closed.

## Vim mode, which-key and macros

`[editor] keymap = "vim"`, or Keymap in the status bar, gives the text
Vim's keys: Normal, Insert, Visual (also by line and block) and Replace
mode, with motions, operators, text objects, counts, registers, marks,
the dot, and one undo step for each change. The status bar shows the
mode and the keys typed so far, the cursor's shape follows the mode, and
`[editor] line_numbers = "relative"` counts lines from the cursor. merlin's
own typing does Insert mode, so completion, closing brackets and several
cursors work there too. The Ctrl keys in `[vim] keep` stay merlin's.

`:` opens Vim's command line under the text, with its commands and ranges
(`:w`, `:q`, `:s`, `:g`, `:sort`, `:norm`, `:set`, …), merlin's commands by
name, and suggestions as you type. `:s` shows its changes as you type
them. `/` and `?` search, with the matches tinted and "2 of 4" in the
status bar.

**Which-key:** a pause on a prefix (`g`, `z`, `[`, `]`, `Ctrl+W`, an
operator, `"`, `'`, `@`, or VS Code's `Ctrl+K`) shows a card of what the
next keys do. The **leader** (Space, `[vim] leader`) has merlin's commands
in groups, and plugins add their own with `[[keys]]` in their manifest.
`[vim] which_key_delay` sets the pause, in milliseconds.

**Macros:** `qq` records into `q`, `q` stops, `@q` plays, `20@q` plays it 20
times and `@@` plays the last again. A run stops at the first key that
fails and says where; one `u` undoes it; with several cursors it plays at
each. A macro is a register, so `"qp` pastes it to edit and `"qy$` puts it
back. With either keymap, Record a Macro (`Ctrl+Alt+R`) records what you
type and the commands you run from the palette, Play the Last Macro
(`Ctrl+Alt+P`) plays it, and Save the Last Macro As… names it. Saved ones go
in your settings as `[[macros]]`, with a name and the keys as Vim writes
them, and show in the palette and Keyboard Shortcuts.

## Themes and packs

merlin's look is a stack of **packs**, as Minecraft's is of resource packs.
A pack is a TOML file that sets any of colours, icons, layout and fonts, and
each layer changes only what it sets. From the bottom up:

1. merlin's own look,
2. the colour theme (Preferences: Color Theme),
3. the icon theme (Preferences: File Icon Theme),
4. the packs in `[editor] packs`, the first of them on top
   (Settings › Appearance › Layouts adds them, takes them off and puts
   them in order).

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
api = "0.9"

[[themes]]          # shows in the colour or icon theme picker
label = "My Theme"
file = "my-theme.toml"

[[packs]]           # a layer for Settings › Appearance › Layouts
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

Preferences: Open Settings (Ctrl+,) opens the Settings tab instead: its
pages are Appearance (colour themes as cards that preview on the window
while the pointer is on them, Follow the system for a dark and a light
theme, icon themes and layouts), Text Editor, Files, Keyboard Shortcuts
(press keys to give a command them, Record Keys to find what keys run,
and Conflicts) and Plugins (each plugin's switch, what it may do, its
memory and CPU, and its own settings). Search covers every page, and the
Workspace tab writes to the folder's file.

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

[plugins.rust]              # a plugin's settings, by its name
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
| `Ctrl+Shift+P` / `F1` | The palette: commands, symbols, files and text at once |
| `Ctrl+P` / `Ctrl+E` | The palette on files (fuzzy; skips hidden and git-ignored files) |
| `Ctrl+Alt+R` / `Ctrl+Alt+P` | Record a macro, again to stop / play the last one |
| `Ctrl+click` / `Ctrl+Alt+click` | Go to the definition / open it to the side |
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
The pointer says what a click does: a hand on links, the text bar on code
and on every box you type in, an open hand on a tab you can drag, a "not
allowed" sign where a dragged tab can't go, and a busy arrow while a save
waits for formatting or the debugger starts. Buttons keep the arrow.

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
first, as VS Code does; opening a file there shows its changes since the
last commit as a diff, side by side when the window is wide enough (Alt+I
switches). The new side can be edited: typing there edits the file, which
opens as a tab beside it, and Ctrl+S saves it. Side by side, each
change has a box in the gap between the halves that stages it (Alt+S) and
a « that reverts it to the last commit (Alt+R), and each changed row has
its own box; Alt+F5 and Alt+F3 go to the next and previous change, and the
status bar says how many are staged. `+` beside a file stages it and `−` unstages it, the `+` and `−`
on the Changes and Staged Changes headings stage or unstage all of them,
and a message with Enter or Commit commits what's staged. Beside Commit,
the sync button says what it will do: ↑1 Push, ↓2 Pull, ↓2 ↑1 Sync (pull,
then push) or Publish Branch for a branch with no upstream; while it runs
a click stops it. The counts are in the status bar too, from a quiet fetch
every 5 minutes while the window has focus. Git never waits on a password:
a remote that wants one says to set up an SSH key or a credential helper.
A rejected push offers Pull and Push, and a pull that stops on conflicts
offers Resolve and Abort Pull. A pull merges unless git is set to rebase. Files with conflicts come
first, under Merge Conflicts, each with how many it has left. While a
merge, rebase or cherry-pick is stopped, a banner says what's coming in
(Merging themes into main · 3 conflicts in 2 files) with Abort Merge, and
the status bar says the same in red. Commit Merge, with git's message
ready, waits until no conflicts are left; a rebase or a cherry-pick has
Continue Rebase in its place. A conflicted file opens in the merge
editor: Incoming and Current side by side over the Result, which is the
file itself and can be typed in. Each line of a conflict has a box, to
take some of a side (Incoming's ticked lines come first); Accept takes a
whole side, Accept Both takes both, and Reset goes back to the base, which
shows in the Result until a side is chosen. Ctrl+Z undoes any choice. The
bar counts the conflicts left, ↑ ↓ go between them, and Complete Merge
saves the file and stages it, asking first while conflicts are left. Its
notice offers the next file with conflicts, and after the last one,
Commit Merge. In a narrow group the merge editor is the Result alone,
each conflict not chosen yet showing both sides above its base with its
Accept links; Side by side or Alt+I switches, and Alt+F5 and Alt+F3 go
between conflicts. A file with markers opened another way offers Open in
Merge Editor in its bar.

File: Compare Files… asks for two files with the file picker and shows
them side by side in the changes tab's view, with its bar: Change 1 of 2,
↑ ↓, Inline. Both sides are the real files and can be typed in: the
side clicked is the one edited. ⇄ Swap changes sides. The Explorer's menu has Select for Compare,
then Compare with it on another file. Compare with Clipboard and Compare
with Saved (a file's unsaved edits against the disk) work the same way.

Tasks: Run Task… lists what `Cargo.toml`, `package.json` and
`.merlin/tasks.toml` offer, and what plugins add, grouped as Test, Build
and run, and Other. Each runs in a terminal tab named after it, and shows
in the status bar while it runs. Ctrl+Shift+B runs the build task and
Ctrl+; A the test task; Configure tasks… opens `.merlin/tasks.toml`:

```toml
[tasks]
lint = "cargo clippy --all-targets"

[tasks.watch]
command = "cargo watch -x check"
group = "build"
```

When a task finishes, the errors and warnings it printed go to Problems
and under the code they point at, from rustc and cargo, most compilers
and linters (`path:line:col: message`), TypeScript, and Python's
tracebacks. Its next run replaces them, and one the language server
already shows isn't listed twice.

The Tests view lists the project's tests by crate and file, with Run
All, Run Failed and On Save. In a Rust file each test has ▶ Run Test and
Debug over it, a ✓ or ✕ in the gutter, and a failure's message under the
line that failed. With rust-analyzer, its tests and binaries are used.

A deleted
file is struck through, opens as what it had, and its menu restores it. A
new file opens as one column, and its menu adds it to `.gitignore`. A
deleted file and a new one at least half alike show as one move, `app.rs
← main.rs`, which opens as the diff between them and stages both paths.
A mode change or a submodule's new commit shows as a chip, and a binary
file's tab says its sizes. A
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
deleted. A click on a mark opens the change in place under its lines, with
what they were and are, Stage (or Unstage), Revert (an edit Ctrl+Z
undoes), and arrows to the next and previous change; Esc closes it. Alt+F5
and Alt+F3 go to the next and previous change. The cursor's line says who last changed it, when
and why, faint after its end, from git blame of the text as it is (lines
not yet committed say nothing). Git: Blame in the palette shows a column
beside the numbers of who last changed each line, one entry for each run
of a commit's lines, with a bar that's brighter the newer it is. Resting on
an entry opens its commit's card: who, when, the hash, the whole message
and its size, with Open Changes (what it changed in this file), Blame
Before This (leaving it out, for lines only moved or reformatted) and Copy
Hash. Esc, or Git: Blame again, hides it.

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

Languages work as in VS Code, where extensions add them. merlin itself
knows none but plain text and Markdown: every language is a plugin, and you
install the ones you use. A language's plugin does everything for it: adds
the language (its file extensions, name, comment markers and indent rules),
colours it, runs its language server, reads its compiler's problems in
task output, and finds its tests, tasks and programs to debug.

- **`rust`, `python`, `javascript`, `go`** colour their languages with
  tree-sitter grammars, through `sdk/grammar`, and run their language
  servers (rust-analyzer, pyright, typescript-language-server and gopls)
  through `sdk/lsp`. Each change recolours only the lines whose meaning
  changed: about 3 ms a key in a 2,600-line file. Besides errors, hover,
  definitions and completion, the server gives semantic colours (over the
  base colours, as VS Code does), symbols (for the Outline, breadcrumbs and
  Go to Symbol), and folding ranges. `rust` also finds cargo's tests and
  tasks, and `javascript` npm's scripts.
- **`cpp`** adds C and C++ (`.c`, `.h`, `.cpp`, `.cc`, `.cxx`, `.hpp`,
  `.hh` and the rest), colours them with a lexer, as a VS Code grammar
  does, and runs clangd. C++ has no test system of its own, so tests come
  from a plugin for the one you use: **`catch2`** finds Catch2's tests and
  runs them through ctest.
- **`json`, `toml`** colour those languages.

```sh
(cd plugins/cpp && cargo build --release)
merlin --plugin plugins/cpp src/main.cpp
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

Hold `Ctrl` over a name and it becomes a link: underlined, with a hand
for the pointer. Rest there and a card shows its definition, with its
doc comment. `Ctrl+click` goes there, and `Ctrl+Alt+click` opens it to
the side. A name with several definitions opens them in the peek view,
and `Ctrl+click` on a name where it's defined finds its references.

rust-analyzer reports some errors as you type, such as unresolved modules,
and the rest, such as type errors, from `cargo check` when you save. Its
type checks while typing are experimental and off by default.

## Tests

```sh
cargo test
```

The plugin-host tests need the word-count, testbed, rust and cpp plugins
built first (`cargo build --release` in each folder). The LSP tests run the
rust plugin against a fake server (`plugins/rust/tests/fake/`), so they
need no language server. Several core
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
load at startup. word-count is a short example; rust a full one.

The manifest says what the plugin may do:

```toml
name = "word-count"
title = "Word Count"
# Its tile in the plugin lists: two letters and a colour.
badge = "WC"
hue = "green"
api = "0.9"
component = "target/wasm32-wasip2/release/word_count.wasm"

[permissions]
editor-read = true
editor-write = true
# Programs it may start. They run with your rights, outside the sandbox.
processes = ["wc"]
# Also: workspace-read = true, to read the project's files, and
# terminal = true, to run command lines in a terminal tab.
```

Plugins are managed in Settings › Plugins (Plugins: Manage in the palette):
each row's switch turns one off everywhere, and its ⋯ menu can turn it off
in this workspace only, reload it, open its Output or folder, or uninstall
it.

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
  `name`, `extensions`, `line-comment` and `block-comment`;
  `indent-after`, what a line ends with for Enter to indent the next
  (Python's `[":"]`); and `exception`, what the language calls one, for
  the debugger's card (Rust's `"panic"`).
  `merlin-lsp`'s `WithLsp::new(plugin, "Name", &[SERVER])` adds a language
  server client to a plugin, with a `Spec` for each of its servers: the
  program, its arguments, the files it serves and how to install it.
  `host::set_tokens(path, layer, lines, tokens)` colours a file: the
  `Base` layer replaces the editor's own grammar, and `Semantic` goes over
  it. `host::set_symbols` and `host::set_folds` give its symbols and fold
  ranges. All of it moves with edits until the plugin sends more.
  `colour_text` colours a piece of text in the language, for diffs and
  Markdown code blocks. The `cpp` plugin is a short example; with
  `merlin-grammar` a tree-sitter grammar takes a dozen lines (see
  `plugins/rust`), building against `sdk/grammar/wasm-include` instead of a
  system C library.
- **Tests and tasks.** `host::set_tests(path, group, label, tests)` lists a
  file's tests in the Tests view, under a group such as a crate, with ▶ Run
  Test over each. Run All, Run Failed, a group's and a file's come to the
  plugin as `run-tests:all`, `run-tests:failed`, `run-tests:group:<group>`
  and `run-tests:file:<path>`, and a test's as `run-test:<id>` or
  `debug-test:<id>`. `host::run_in_terminal` runs a command line in a
  terminal tab, and `terminal` gets all it printed when it ends, to read
  into `host::set_test_results` and `set_test_run`. `host::set_tasks(set,
  tasks)` adds to Run Task, which asks with `find-tasks` as it opens; the
  Tests view asks with `find-tests`. `host::start_debugging` debugs
  something, with a command line to run instead when there's no debugger.
  The `rust` plugin does all of it for cargo.
- **Problems in task output.** `[[problem-matchers]]` read a compiler's
  problems out of what a task prints, as VS Code's problem matchers do:
  `lines`, one after another, each a `regex` and the groups with the
  problem's `file`, `line`, `column`, `severity`, `code` and `message` (0
  for the whole line). `between` is a pattern for lines that may come
  between them, and `link` a page about a problem's code, with `{code}`
  where it goes. The `rust` plugin's reads rustc's two-line problems, and
  `python`'s a traceback. Lines no plugin's matcher takes are read as
  `path:line:col: severity: message`, which most tools print.
- **Settings.** Defaults go in the manifest's `[settings]`; the user's and
  the workspace's `[plugins.<name>]` override them. Read them with
  `settings::text("server")`, `settings::flag(...)`, `settings::integer(...)`
  (dotted keys reach into tables), when needed, as they can change, and
  `setting_changed` hears when one does. Describe a setting and its page in
  Settings › Plugins shows it as a row, for the user or the workspace:

  ```toml
  [settings.check-command]
  type = "choice"        # flag, string, path, integer, float, choice, list, secret
  title = "Check on save"
  default = "check"
  choices = [{ value = "check", label = "cargo check" }, "clippy"]
  description = "What rust-analyzer runs when you save."
  scope = "user-only"    # for a program's path: never set by a workspace
  ```

  A `secret`, such as an API key, is typed as dots and kept in the
  keyring, never in a settings file; read it with `host::get_secret`.
  Plain values are shown too, typed by their defaults. Every plugin also
  gets **Notices** (all, errors only, none) and **Log level**. A language
  server's `Spec::options` hands settings to the server, and restarts it
  when one changes.
- **Views.** `[[views]]` in the manifest declares each view the plugin
  draws, and where it goes: `left` puts it in the activity rail after a
  line, `right` gives it a tab in the right side bar, `bottom` a tab in the
  panel beside Problems and Output, and `tab` opens it in an editor tab.

  ```toml
  [[views]]
  id = "chat"
  title = "Chat"
  icon = "icons/chat.svg"   # an SVG in the folder, a name such as plus, or a glyph
  where = "right"
  empty = "Ask about this file."

  [[views.toolbar]]          # buttons in its header, for the plugin's commands
  command = "new-chat"
  icon = "plus"
  ```

  `view(id)` draws one, as `panel` draws the old single panel (it calls
  `panel` unless the plugin says otherwise), and only while it shows.
  `host::set_view_badge(id, count)` puts a count on its icon and tab, and
  `host::focus_view(id)` shows it. Where it goes is a default: drag its
  icon or tab elsewhere, or use its ⋯ or a right-click to move it, hide it
  or put it back; the workspace remembers,
  and View: Show finds any of them in the palette. A plugin without
  `[[views]]` keeps `panel`'s on the right, as before.
- **Buffer events**: `buffer` hears when the active file switches, changes or
  is saved.
- **The selection**: `host::active_selection` gives its text and position.
- **Reviews.** `host::show_diff(id, path, old, new)` shows a proposed change
  as a diff in its own tab; the user's answer arrives at `review`.
- **Richer panels**: `Node::markdown` for formatted text, and
  `Node::item` for clickable rows drawn as text, such as a list of problems.

Other languages work too: anything that builds a WASI 0.2 component for
[`sdk/rust/wit/merlin.wit`](sdk/rust/wit/merlin.wit).

## Not done yet

- A wrapped line taller than the view can't scroll part way. Without a
  language server, folds go by indentation and symbols are found by
  keyword. Only one plugin's symbols or folds count for a file at a time.
- Git: images don't show side by side, very large files compare anyway,
  a submodule's commits aren't listed, and there's no Show Ignored. The
  changes tab has its boxes only side by side, not inline, and stages
  only what's saved.
- The Settings tab changes plugins' own settings only when they're on or
  off; other values are set in the file.
- Completions don't add imports, and renames don't create, move or delete
  files.
- A review is all or nothing per edit; you can't accept some hunks and not
  others. Markdown has no tables or nested quotes.
- The SDK isn't published, so `plugin new` points at the copy in this repo.
- No Helix keymap yet. In Vim mode, `Ctrl+W h`, `j`, `k` and `l` cycle
  through the groups rather than going by direction.
- No networking API.
