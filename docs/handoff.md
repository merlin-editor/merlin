# Handoff: building the approved mock-ups

This is for the next agent continuing work on the `editor-rework` branch.
Read it with [`feature-research.md`](feature-research.md), which has the
research, the build order, and what's marked **Built**, and with
[`mockups/README.md`](mockups/README.md), which describes the 11 approved
mock-ups. Nothing is pushed. The last commit when this was written was
`d4283cc`.

## The user's rules

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

Not built yet, in the order to do them:

1. **Code actions and the lightbulb.** The mock-up has a lightbulb in the
   gutter on the cursor's line, and a card with **Quick Fix** and
   **Refactor** groups, the preferred fix highlighted, and "Format the
   file Ctrl Shift I" at the bottom. Keys: Ctrl+. and a click on the bulb.
2. **Format and format on save.** Ctrl+Shift+I, and a setting
   `editor.format_on_save`.
3. **Rename.** F2 opens an inline box over the name, with "Enter to
   rename N places in M files · Shift+Enter to review first".
4. **Problems in the hover card.** The user asked (2026-11-05) whether
   hovering a squiggle gives a proper hover, as other editors do. It
   doesn't yet.
   - Today, resting the mouse on a word asks the server for its hover
     (`hover_at` in `context.rs`): its type and docs only. The problem
     under the pointer isn't shown. Its message shows only in the status
     bar when the cursor is on it (`cursor_mark` in `chrome.rs`), and now
     after the line.
   - VS Code's hover over a squiggle puts the problem first: the message
     coloured by severity, its source and code (such as `rustc E0308`,
     linked), and "View Problem" and "Quick Fix… (Ctrl+.)". A divider
     follows, then the type and docs. It also shows when the pointer
     rests on a squiggle that isn't on a word, and a problem-only card
     appears even without a language server's hover.
   - To build it:
     - have `hover_at` also look up `doc.marks` under the pointer
     - give `Popup::Hover` / `PopupKind::Hover` a problems part
     - let `Diagnostic` carry the server's `source` and `code`
     - draw them in `Frame::popup`
   - It isn't in the mock-ups. The card is the existing hover's, but it
     adds UI, so show the user a quick mock-up first. Quick Fix depends on
     code actions (item 1).
5. **Find references** has its own mock-up (`References.dc.html`), the
   next item after these.

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
  - `cargo test --workspace`: 312 tests passed at `d4283cc`
  - `cargo clippy --workspace --all-targets`
  - in `plugins/lsp`: `cargo clippy --release --target wasm32-wasip2`
  - `cargo fmt --all`

## Checking in the window

The user's desktop is Hyprland. The earlier agent drove it with these
tools: `wtype` for keys, `grim` and `hyprctl` for screenshots, and
python-uinput scripts for the mouse. They were in that session's
scratchpad, which the next agent won't have, so here is enough to
recreate them:

- **Launch:** kill any running `target/release/merlin` with an
  *anchored* `pgrep -f '^/home/marc/programming/rust/merlin/target/release/merlin'`
  (an unanchored pattern can kill your own shell). Then, from a test
  project folder, run
  `setsid target/release/merlin src/main.rs > log 2>&1 < /dev/null &`.
  Poll `hyprctl clients -j` for the window whose `class` is `""`, and
  `hyprctl dispatch focuswindow address:<addr>`.
- **Screenshot:** take that client's `at` and `size` from
  `hyprctl clients -j`, then `grim -g "x,y wxh" out.png`.
- **Keys:** `wtype -M ctrl s -m ctrl` presses Ctrl+S, and `wtype "text"`
  types text.
- **Test project:** a small Cargo project whose `src/main.rs` has a
  type error (`let count: u32 = steps.len();`), an unused variable, and
  calls with several arguments shows every language feature.
  rust-analyzer takes about 5–10 s to send hints, and cargo-check
  problems come after a save.
- Killing merlin with unsaved changes leaves a crash backup in
  `~/.local/share/merlin/backups/<project>`, which comes back at the next
  start. Remove the backup of a scratch project before rerunning.
