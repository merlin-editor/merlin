# Plugins: what merlin is missing

Research of 2026-11-08 on merlin's plugin API (0.9) against VS Code, Zed,
JetBrains and Neovim, and what their users ask for. Nothing here is built
yet; any UI in it needs a mock-up first.

## Where merlin stands

| Area | Today | Where |
|---|---|---|
| Views | One panel per plugin, from `render-panel`, always in the **right** side bar, always with the generic plugin glyph. No way to say where it goes, what icon it has, or to have two. | `sdk/rust/wit/merlin.wit` (`render-panel`, `panel`), `crates/core/src/editor/window.rs` `rail_items` (`panel_names(Side::Right)`, `Icon::Plugin`) |
| What a panel can draw | Headings, text, buttons, items, Markdown, inputs, dividers, a footer. No trees, icons, badges, toolbars, context menus or empty states. | `node` in `sdk/rust/wit/merlin.wit` |
| Messages | `notify` is the only way to tell the user anything, and it's a toast. The lsp SDK alone calls it from 11 places, several for things nobody asked about (a server's error reply, "Nothing to show here"). Reloads toast too. | `sdk/lsp/src/handlers.rs`, `sdk/lsp/src/lib.rs`, `crates/plugin-host/src/lib.rs` |
| Logs | One Output view for all of merlin. A plugin has no log of its own; the plugin page's Output button opens the shared one. | `views.rs` `output_nodes`, `prefs/plugins.rs` `PageAction::Output` |
| Settings | `[settings]` in the manifest is a table of defaults with no type, description, choices or range. The page shows booleans as switches and everything else read-only, and saves only to the user's file, though `settings.rs` has `set_in(workspace, …)`. Plugin settings don't show in Settings search. | `manifest.rs`, `gui/settings/plugins.rs` ("Its settings") |
| Managing | The same ten plugins are listed twice: the Plugins side view and Settings › Plugins. Clicking a row in the side view opens the settings page. | `views.rs` `plugins_nodes`, `gui/settings/plugins.rs` |

## What other editors do, and what their users want

**VS Code** is the model most people know.
- Views are declared in `package.json` (`viewsContainers`, `views`), each
  with an icon, and go in the activity bar, the panel, or (since 1.64) the
  secondary side bar. Users drag any view to any of the three, and VS Code
  remembers. Copilot Chat sits in, and the Claude Code and Continue extensions can sit in,
  the secondary side bar, and can also open as an editor tab.
- Webviews let a view or an editor tab draw anything. That's how chat
  extensions look the way they do, and also why some VS Code extensions feel
  foreign and slow.
- `contributes.configuration` is a JSON schema per setting: type, default,
  `enum` with descriptions, min and max, Markdown description, and a
  **scope** (application, machine, window, resource). The Settings UI draws
  the right control for each and shows them under Extensions › name, found by
  search, with User and Workspace tabs.
- Every extension can have its own **Output channel** (`createOutputChannel`,
  with log levels for `LogOutputChannel`) chosen in the Output view's
  dropdown. Notifications are for things the user should act on.
- Long-standing complaints: notification spam (VS Code added Do Not
  Disturb and per-source muting in 1.83 for that reason); extension views
  that land where the user didn't want them.

**JetBrains** tool windows declare an `anchor` (left, right, bottom) and an
icon; the user can move them, and each gets a stripe button. Plugin
settings are pages in Settings, generated or hand-built, with project and
application levels. Notifications have groups the user can set to balloon,
sticky, or log-only (the Event Log / Notifications tool window).

**Zed** extensions can't add panels yet, and extension UI is a frequent request on
its tracker; its AI features are built in for that reason. Settings
are JSON, with schemas for completion. Extensions show in an Extensions tab,
not twice.

**Neovim** plugins can draw anything, and users love that, but every one
draws its own, so they don't match. `vim.notify` gets replaced by
plugins like noice.nvim and fidget.nvim largely because LSP progress and
messages are noisy by default.

## What merlin should add

### 1. Views where they belong, with their own icon

**Built** (step 3): `[[views]]` with `id`, `title`, `icon`, `where`,
`empty` and a toolbar; `render-view(id)` in the `views` interface the SDK
exports, so 0.9 plugins without it still load; `set-view-badge` and
`focus-view`; the right side bar's own tabs, with + for hidden views; the
plugin's name after a view's title; Move, Hide and Reset Location from ⋯
or a right-click, and dragging an icon or tab to another place, remembered
by the workspace. The richer nodes are step 5.

The manifest declares each view the plugin has:

```toml
[[views]]
id = "chat"
title = "OpenCode"
icon = "icons/chat.svg"    # or a Nerd Font glyph name; else the badge
where = "right"            # left | right | bottom | tab
```

- A plugin can have more than one view, and `render-view(id)` replaces
  `render-panel`. A plugin with no `[[views]]` has none, so a language
  plugin doesn't take a rail slot.
- `left` puts it in the activity rail with the built-in views, with its
  icon. `right` puts it in the secondary side bar, which gets its own row
  of tabs along its top, as VS Code's and JetBrains' do, rather than
  sharing the left rail. `bottom` is a tab beside Problems, Output and
  Terminal. `tab` opens it in the editor area, like the diff and Settings
  tabs, for chat and previews that want the room.
- Where it lives is a default: the user can move it (drag the icon, or
  Move View to… in its menu) and merlin remembers.
- A badge count (`set-view-badge`), a title-bar toolbar of the plugin's
  commands, and a text for when it's empty.
- Richer nodes: a tree with icons and expanding rows, rows with a
  right-click menu of the plugin's commands, and a progress bar. Not
  webviews: merlin's drawing stays one style and cheap, which is its
  advantage over VS Code here. A chat view needs a streaming Markdown block,
  a multi-line input with Ctrl+Enter, and file chips, and those are what to
  add.

### 2. A log per plugin, and notices only when needed

- `log(level, message)` with trace, debug, info, warn and error, into the
  plugin's own channel. The Output view gets a channel picker (merlin, then
  each plugin), and the page's Output button opens that plugin's channel.
- Processes' stderr goes to the channel too; for a language server that's
  most of what's useful when it breaks.
- `notify` stays for what the user did or must act on, and gains a level
  and buttons. The lsp SDK moves server error replies, `showMessage` below
  error, "Nothing to show here" and similar into the log, and the status
  bar's language card shows a red dot instead.
- "Reloaded rust" goes in the log; a failed reload stays a notice, with
  Open Output.
- A setting per plugin, Notices: All, Errors only, or None (log only), on
  its settings page, and the notice gets a "Don't show from Rust" link.

### 3. Settings with a schema, in both scopes

```toml
[settings.check-command]
type = "string"
default = "check"
choices = [ { value = "check", label = "cargo check" },
            { value = "clippy", label = "cargo clippy" } ]
description = "What rust-analyzer runs on save."
scope = "workspace"     # user-only | workspace (default)
```

- Types: flag, string, integer and float with min and max, a choice from a
  list, a list of strings, and a secret (stored with `set-secret`, shown as
  dots). The page draws the same rows as the built-in settings
  (`gui/settings/rows.rs`), with descriptions, Modified and ↺.
- User and Workspace tabs apply to plugin settings as to the rest, through
  the existing `set_in(workspace, …)`. A `user-only` setting (a path to a
  program, an API key) can't be set per workspace.
- Settings search finds them, grouped under the plugin.
- `on-setting-changed(key)` so a plugin needn't poll `get-setting`, and so
  lsp can restart its server when its command changes.
- The old untyped `[settings]` keeps working, shown as today.

### 4. One home for managing plugins

Two lists of the same plugins make it unclear which one is the real one. The
rule should be: **the activity rail is for things you work in; Settings is
for setting things up.** Plugins are set up, not worked in.

- The Plugins side view and its rail icon go. Settings › Plugins is the one
  place, reached from the palette (Plugins: Manage), the gear menu, and
  the status bar's language card ("Rust plugin…").
- That frees the rail for plugins' own views from section 1, so what's in
  the rail is always somewhere you work.
- When merlin has a registry, Browse goes on the same page as a second tab
  (Installed · Browse), as JetBrains does, not in another side view.
- Enable and disable per workspace, from the switch's menu, as VS Code
  does; it's a frequent request there.

### 5. Smaller gaps

- Lazy start: `activate-on` (a language, a command, a file in the
  workspace, a view opened), so ten plugins don't all start at launch.
- Context menu contributions (`[[menus]]`: editor, Explorer, tab) for
  commands like "Ask OpenCode about this".
- Progress: `start-progress(title, cancellable)` drawn in the status bar
  and the view, rather than toasts.
- Plugins that need another (`depends = ["lsp"]`), and updates.

### 6. Drawing more than nodes, without a browser

A plugin can only show what the node list describes, so it can't show a
PDF, a plot or a git graph. VS Code answers that with webviews, a browser
process per view, which would cost more than all of merlin does today. The
window already draws everything from a few primitives (`Quad`, `Line`,
`Glyph`, `Image` in `crates/app/src/gui/scene.rs`) with pictures kept as
bitmaps, so plugins should hand it pictures and drawing, and merlin keeps
the drawing, scrolling and caching.

**First, plugin calls off the main thread.** Today every call into a
plugin runs on the window's thread; only process output has threads of its
own (`crates/plugin-host/src/process.rs`). A page of a PDF can take 100 ms
or more to render, and typing would stall for it. Each plugin gets a
worker: the window sends it requests and keeps drawing what it has until
the answer comes, and a newer request (you scrolled on) cancels an older
one. A slow plugin then can't freeze the window, whatever it does.

In four steps, each only as strong as it needs to be:

1. **Images.** `set-image(id, width, height, bytes)` (PNG or RGBA) once,
   and an `image(id)` node shows it. merlin keeps the bitmap, so redrawing
   costs the plugin nothing. Plots, previews, thumbnails.
2. **Tiled surfaces for documents.** A plugin opens a file type:

   ```toml
   [[editors]]
   id = "pdf"
   title = "PDF"
   extensions = ["pdf"]
   ```

   The file opens in a tab that is a surface. merlin owns scrolling and
   zoom and asks for tiles as they come into view,
   `render-tile(surface, page, x, y, w, h, scale) -> rgba`, and redraws the
   tiles it has between answers, so scrolling is smooth and an idle
   document costs no CPU. The plugin says where its text is on each page
   (`set-text-boxes`), so merlin draws selection, Find and copy as in any
   other tab; clicks and keys go to it with where they landed. A plugin
   can render in wasm (a Rust PDF renderer), or with the `processes`
   permission ask `mutool draw` or `pdftoppm` for the image. The same
   surface serves SVG, large images, notebook output and hex.
3. **Drawing commands for custom views.** A list of rectangles, lines,
   curves, text and image references, with colours named from the theme
   (`accent`, `muted`) and text in merlin's fonts, so a git graph or a
   flame chart fits the theme and stays sharp at any zoom. Items have ids,
   for clicks and for redrawing only what changed. The scene needs curves.
4. **Rules that keep it cheap.**
   - Nothing animates on its own: `request-frame` asks for a frame, at
     most 30 a second, and stops while the view is hidden.
   - Each plugin has a picture budget (64 MB, say) with the least recently
     used tiles dropped first, and all of them when the tab closes.
   - Its picture memory shows on its page beside its wasm memory.
   - Tiles are rendered at the screen's resolution, at 1× while zooming,
     then sharpened when you stop.

The PDF viewer is the test: it's `PluginDocument.dc.html` in
`docs/mockups`.

## Order

1. Logs and quieter notices: no UI to agree on beyond the Output picker, and
   it's the most visible annoyance. **Built.**
2. Settings with a schema and scopes: the page and rows exist. **Built**,
   with secret settings kept in the keyring.
   `on-setting-changed` is in a `plugin-sdk` world that the SDK builds, and
   the host calls it only when a plugin exports it, so plugins built for
   0.9 before it still load.
3. Views from the manifest, left / right / bottom / tab, with icons, and
   the right side bar's own tabs. **Built.** `render-view` is in a `views`
   interface the SDK exports, found the same way as `on-setting-changed`.
4. One home for plugins (drop the side view), once 3 gives the rail
   something better to hold. **Built**: Settings › Plugins has Installed
   and Browse, a filter, and a menu on each row that turns a plugin off
   everywhere or in this workspace only (`editor.disabled_plugins` in
   either settings file, read at startup, which it wasn't before).
5. Rich nodes for chat views, then the smaller gaps. **Rich nodes built**,
   with API 0.10: rows with chips and a right-click menu, messages, edit
   cards, chips under a multi-line box, progress, empty states, and real
   Markdown with coloured code. `render-panel` is gone; `render-view` and
   `on-setting-changed` are plain exports. **Lazy start and depends built**:
   `activate-on` (language, file; any use starts it too, and its commands
   are cached for the palette) and `depends`. **`[[menus]]` built**, with
   `menu-target` for the file the menu was on. **Progress built**: `start-progress`,
   `report-progress`, `end-progress` and `on-cancel`, in the status bar.
   Updates wait for a registry, as Browse does.
6. Plugin calls off the main thread, then images, then surfaces with
   `[[editors]]` (proved by a PDF viewer), then drawing commands.
   **Calls off the main thread built**: each plugin's instance has a
   thread; calls from the window don't wait (scripts and tests still do),
   a view or a file change asked for again replaces the one not yet
   started, and programs are started from one long-lived thread, since on
   Linux a program is told to end with the thread that started it.
   **Images built**, with API 0.11: `set-image(id, picture)` takes a PNG
   or RGBA, decoded on the plugin's thread into its 64 MB budget, and an
   `image` node fits it to the view, its corners rounded, scaled once per
   size; until it's there, a hatched box with its words.
   **Document surfaces built**, with API 0.12: `[[editors]]`, and
   `open-document`, `render-tile`, `page-text` and `close-document`
   exports. The bundled `pdf` plugin draws with hayro in wasm. The tab
   has the mock-up's toolbar, thumbnails, tiles with the thumbnail
   standing in (hatched until that comes), and the status bar's page and
   zoom; the words are the tab's text, so Find, selection and Copy are the
   editor's own. Its right-click menu has Copy, Search the Project for
   This, plugins' items, and Copy Page as Image (a PNG through `wl-copy`
   or `xclip`, as the clipboard library takes only text); a plugin's page
   in Settings shows its pictures' memory beside its own.
