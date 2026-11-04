# Roadmap

Each milestone ends in something you can try, with a "done when" test. The
order is deliberate: milestones 2–4 can't be judged until merlin is usable
for real editing, but milestone 2 is the biggest gain over ttt, so milestone 1
stays small and 2 follows straight after.

## 1. Usable for everyday editing

*Status: built. The "done when" test is still to do.*

- Selection (Shift+arrows, Ctrl+A) and the system clipboard (wl-copy/xclip,
  falling back to OSC 52).
- Several open files with a tab bar, and a fuzzy "open file" picker that
  reuses the palette.
- Tabs and wide characters drawn at their real width, horizontal scrolling,
  smart-case find, and find-and-replace.
- Mouse: click to place the cursor, drag to select, wheel to scroll, click
  panel buttons, tabs and palette items.
- A focused button that stands out in any terminal theme.

*Done when:* you can work on merlin's own code in merlin for an hour.

## 2. Plugin SDK and API 0.3

The answer to ttt#1223.

*Status: built. `plugin new` to a running plugin took 18 seconds, most of it
the first build. word-count went from 94 lines to 49, nine over the target;
what's left is the plugin itself, not boilerplate.*

- A `merlin-plugin` guest crate with default implementations, so a plugin
  writes only the functions it uses.
- Plugin state kept across hot reload (`save-state` / `restore-state`).
- Plugin keybindings, and events for buffer changed / saved.
- Secret storage, so the OpenCode plugin needs no plain-text tokens.
- A first-load prompt showing the manifest's permissions; the grant is
  remembered.
- A `merlin plugin new` template.

*Done when:* word-count is under 40 lines, and a new plugin works within five
minutes of `plugin new`.

## 3. Language support, built as plugins

The process API already handles LSP's `Content-Length` framing, so the LSP
client is an ordinary plugin, which proves the API is good enough.

*Status: built, and the "done when" test passes on merlin with
rust-analyzer: an unresolved module was underlined as it was typed, and
go-to-definition opened `Highlighter` in `crates/syntax`. Type errors show
only after a save (from `cargo check`), because rust-analyzer's live type
checks are experimental. Gutter "signs" are coloured line numbers, so the
gutter doesn't change width.*

- Host features: highlights and underlines on text ranges, gutter signs,
  popups for hover and completion.
- Syntax highlighting with tree-sitter, in the host for speed.
- An LSP plugin: diagnostics, hover, go-to-definition, completion, tried on
  rust-analyzer.

*Done when:* rust-analyzer errors show while editing merlin, and
go-to-definition works.

## 4. Reviewing changes, git and the file tree

What the OpenCode plugin needed, built as general plugin API. merlin is an
editor first; the OpenCode chat has since moved to its own repository,
`merlin-opencode`, as an ordinary plugin.

*Status: built. With a fake agent standing in for the model, the "done when"
test passes in kitty: a selection attached with Ctrl+Alt+A, the proposed edit
opened as a diff tab, `y` accepted it, OpenCode's write reloaded the file,
and git markers showed the changed lines. Not yet tried with a real model:
OpenCode's free tier was used up and LM Studio refused the host name. When a
model call fails, OpenCode's ACP mode reports nothing, so the chat stays on
"Working…".*

- A diff view to accept or reject each change the agent proposes.
- Attach the selection instead of the whole file.
- Markdown in panels: code blocks, lists, bold.
- A file tree, and git markers in the gutter.

*Done when:* you can review and apply an OpenCode change without leaving the
editor.

## 5. A VS Code-style interface

*Status: built, and the "done when" test passes in kitty: with the mouse,
README.md opened from the Explorer, was edited, saved from File › Save,
searched, and its changes opened from Source Control. Borders around panes,
tabs and popups follow ttt; diffs show side by side in a wide window. The
scrollbar marks errors and git changes, but not yet search matches.*

merlin works, but it looks like a prototype: a bare tab line, a reverse-video
status bar, and everything else found through the palette. ttt already
borrows VS Code's layout, and people know it, so merlin should too: someone
who has used VS Code should find things where they expect them.

- **Theme.** VS Code's Dark Modern colours in 24-bit colour, with shaded
  areas instead of box lines. Terminals without 24-bit colour keep the
  current palette look.
- **Menu bar**: File, Edit, Selection, View, Go, Help, built from the command
  table, with each command's key. Mouse, F10 or Alt+letter.
- **Side bar** on the left (Ctrl+B): Explorer (the file tree, with git
  status colours), Search across the project (Ctrl+Shift+F), and Source
  Control (Ctrl+Shift+G: changed files, each opening as a diff against the
  last commit). Plugin panels move to a secondary side bar on the right
  (Ctrl+Alt+B).
- **Editor**: tabs always shown, with × and a dot for unsaved changes;
  current-line highlight; a scrollbar marking errors, git changes and
  search matches; find as a widget at the top right.
- **Status bar** as in VS Code: branch, error and warning counts, Ln/Col,
  indentation, encoding, line endings, language. Messages show as toasts.
- **VS Code's keys** where the terminal allows them: Ctrl+P opens files,
  Ctrl+Shift+P and F1 commands, Ctrl+G goes to a line, Ctrl+/ comments.

*Done when:* with the mouse alone you can open a file from the Explorer,
edit it, save it from the File menu, search the project and look at what
changed; and every command is in a menu or the palette.

## 6. Closer to VS Code

*Status: built and checked in kitty with a Nerd Font: Explorer, hover,
panel, tabs, guides and brackets look as in the proposal. The "done when"
paths are covered by tests (a problem opens at its line from Problems, a
change opens as a diff from Source Control). Preview tabs came later, in
milestone 6½.*

Milestone 5 put things where VS Code users look for them. This one makes
them look and behave like VS Code's, following the interface proposal:
flat panes, Nerd Font icons, and the small pieces that make VS Code feel
finished.

- **Flat panes.** Areas are set apart by shade, as in VS Code, not boxes.
  Floating things (menus, palette, find, hovers, notices) keep their frames.
- **Nerd Font icons** for files, views and status items, as VS Code's icon
  theme shows them. `MERLIN_ICONS=plain` uses ordinary symbols instead.
- **Activity bar** on the far left: Explorer, Search and Source Control with
  a badge for changed files, and the plugin panels.
- **Command center** in the title bar: a search box that opens Go to File,
  and buttons that show or hide the side bar, the panel and the secondary
  side bar.
- **Explorer** with icons, indent guides, problem counts and colours, and a
  dot on folders that hold changes. Search and Source Control get the same
  rows.
- **Tabs** with icons, names coloured by problems and git, an accent under
  the active tab, × only on the active tab or the one under the mouse, and a
  button for the file's changes. (Preview tabs that a single click reuses
  are left for later.)
- **Editor**: indent guides with the current block brighter, bracket pairs
  in three colours, and the word under the cursor marked where else it
  appears.
- **Panel** under the editor (Ctrl+J) with Problems (Ctrl+Shift+M) and
  Output.
- **Minimap** in braille, as an option (View › Minimap), added after the
  milestone at the user's request, with search matches, problems and
  changes marked on it and on the scrollbar.
- **Added after the milestone**, also at the user's request: word wrap
  (Alt+Z), editor groups side by side (Ctrl+\), multiple cursors (Ctrl+D,
  Ctrl+Shift+L, Shift+Alt+Up/Down, Alt+click), and staging and committing
  from Source Control.
- **Notices** with an icon and buttons; **status bar items** that do
  something when clicked; rows, tabs and buttons that light up under the
  mouse.

*Done when:* a VS Code user dropped into merlin at 160×46 recognises every
part of the window, and with the mouse alone can go from an error in
Problems to its line, and from a changed file in Source Control to its diff.

## 6½. Editing and navigating as in VS Code

*Status: built after milestone 6, at the user's request, with tests for
each part.*

- Double- and triple-click select a word and a line; preview tabs.
- Editor groups split down as well as right; tabs drag to reorder or to
  another group; an edit in one group moves the others' cursors along.
- The tabs, groups and cursors left in a folder come back next time.
- Folding by indentation, sticky scroll, breadcrumbs, an Outline view and
  Go to Symbol, from symbols found in the text.
- Settings in TOML: the user's, a workspace's over them, and plugins'
  under `[plugins.<name>]` (plugin API 0.6). Keys can be rebound.
- Languages are plugins' to add, as in VS Code (plugin API 0.7): the
  language itself, base and semantic colours, symbols and folding ranges.
  The lsp plugin takes them from language servers, and a `cpp` plugin adds
  C and C++. Rust, Python, JavaScript, JSON and TOML moved out of the editor
  into plugins with their tree-sitter grammars (API 0.8), so the editor has
  no languages of its own; `merlin plugin install` and
  `scripts/install-plugins.sh` put them in place.
- The OpenCode chat moved to its own repository, `merlin-opencode`:
  merlin is an editor, and AI is a plugin like any other.

## 7. Robustness and release

- Random-event testing of the core: no event sequence panics, and undo always
  returns to earlier states.
- Recovery files after a crash; a performance check on 100 MB files.
- CI that compares `--html` screenshots against saved ones.
- Release builds, and more themes (settings and key bindings are done).

*Done when:* a million random events cause no panic, and a 100 MB file opens
in under a second.
