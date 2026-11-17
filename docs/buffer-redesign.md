# The text buffer: a redesign for large files

Research of 2026-11-17 and 18, on the `editor-rework` branch. The goal is
to beat VS Code with the same features. Faster isn't enough: merlin should
also use less memory, and less CPU while working and while idle.

Every number below comes from code run on this machine (22 cores), on
generated files and some real ones:

| File | Size |
|---|---|
| Text | 85 MB |
| Rust, made up | 20 MB |
| JSON on one line | 19–23 MB |
| Log | 342 MB |
| Large real Rust files from the cargo registry | 2–4 MB |

The prototypes are in [`buffer-redesign/`](buffer-redesign), and each one
checks that its results match what it replaces. Run any of them with:

```
cargo run --release --manifest-path docs/buffer-redesign/Cargo.toml --bin <name>
```

The editor's own timings come from `crates/core/tests/large_probe.rs`:

```
cargo test --release -p merlin-core --test large_probe -- --ignored --nocapture
```

The comparison with VS Code comes from
[`buffer-redesign/compare`](buffer-redesign/compare). Make the files once,
then run one editor on one file at a time:

```
python3 docs/buffer-redesign/compare/make_files.py
docs/buffer-redesign/compare/run.sh vscode unique.log
docs/buffer-redesign/compare/run.sh merlin unique.log
```

Any UI this leads to needs a mock-up first.

## In short

Keep the rope. Change everything that is worked out *from* the text, and
make each thing pay for what's on screen, not for the size of the file.

The rope (`ropey` 1.6) is fine. It takes 1.14 times the file in memory.
An edit takes 1–2 µs, a snapshot 0.02 µs, and a line lookup 0.5–2 µs at
342 MB. Another rope saves 8% of memory and 30% of lookup time, and
nothing we found is slow or large because of the rope.

What goes wrong follows one pattern. A feature needs something about the
whole file and works it out for every line. It keeps the result, often
larger than the file, under the buffer's version, and works it all out
again after any edit, or on every frame. Ten such features make a large
file slow, heavy and busy even when idle.

The redesign rests on six rules, each proven below:

1. **A frame costs what the screen shows.** Nothing drawn goes through the
   whole file. A frame with nothing new to show, such as a cursor blink,
   doesn't rebuild the view.
2. **An edit costs what it touched.** Every fact kept about the text is in
   blocks of lines with totals. An edit updates its block, not the file.
3. **Keep what the screen needs, and work the rest out when asked.**
   Colours, search matches and line facts are kept for the screen and its
   surroundings, or as a few bytes a block. They aren't kept for every
   line.
4. **Whole-file work runs once, in the background, and can stop.** It
   runs on a snapshot, on a few low-priority threads. It does the screen
   first, and stops as soon as the text changes.
5. **Share, don't copy.** Plugins, saving, blame, backups and undo take
   snapshots of the rope (free) or its changes, never a String of the
   file.
6. **Hold one copy of the file.** Git's version is kept as a hash a line,
   undo as rope snapshots, and backups as a journal of edits.

## How merlin and VS Code compare today

Both were opened on each file with a throwaway profile, no extensions or
language servers, and measured for 45 seconds. Memory is PSS, which shares
pages fairly between processes, summed over every process. CPU is the
total used in those 45 seconds, and idle is the last 10 of them.

| File | VS Code memory | merlin memory | VS Code CPU / idle | merlin CPU / idle |
|---|---|---|---|---|
| 3 KB Rust | 1 009 MB | 78 MB | 19.8 s / 6.2% | 0.8 s / 0% |
| 2 MB Rust, made up | 1 106 MB | 85 MB | 31.8 s / 9.1% | 8.5 s / 0% |
| 3.9 MB Rust, real | 1 062 MB | 88 MB | 41.2 s / 8.5% | 7.2 s / 0% |
| 20 MB Rust, made up | 1 156 MB | 141 MB | 22.6 s / 9.1% | 26.9 s / 0% |
| 19 MB JSON on one line | 1 661 MB | 122 MB | 26.7 s / 6.4% | 11.4 s / 0% |
| 85 MB text, one line repeated | 988 MB\* | 175 MB | 19.4 s / 5.6% | 44.6 s / 100% |
| 342 MB log, one line repeated | 1 043 MB\* | 449 MB | 23.3 s / 6.9% | 44.5 s / 100% |
| 342 MB log, every line different | 1 351 MB | 448 MB | 23.3 s / 2.1% | 49.7 s / 100% |

\* VS Code keeps a file of repeated lines in far less than its size. With
every line different, the 342 MB log costs it about 340 MB more than an
empty window, as expected. Merlin keeps the large text files at
1.07–1.12× on top of its 80 MB. The 20 MB Rust file costs it about 3×,
which is still to explain.

VS Code colours the Rust and JSON files and merlin, above 512 KB, doesn't.
So VS Code's figures include colours that merlin's don't. On the real
3.9 MB file the gap is still 12×.

I checked that VS Code really showed the 342 MB log:
- It read the file from disk: 550 MB read in all, against 209 MB for the
  small file.
- A capture of its window showed the log's lines and minimap.
- Its large-file confirmation (`workbench.editorLargeFileConfirmation`)
  was raised in its profile for these runs. The runs before that gave the
  same figures within 8%.

Zed couldn't be measured. With a separate profile it reports "zed is
already running" though no copy runs, and I left it there.

Merlin uses an eighth to a fourteenth of VS Code's memory on the code and
JSON files, and a third of it on the 342 MB log. It uses less CPU opening
files up to 19 MB. It uses more on the made-up 20 MB Rust file, and far
more idle on the two large text files.

### Step 1: idle and per-frame costs, done

Merlin used a whole core while idle on the large text files, and
seconds of CPU opening the large code files. I found why by sampling the
app and timing the frame the window draws (`window_view`) in
`large_probe`, with the app's default settings on.

**The cause.** With the defaults on, every frame on the 85 MB file took
5.4 s. The breadcrumbs, drawn twice a frame, ran the symbol finder over
every line of the file. The window spent all its time in such frames,
which is the whole core. The probe had missed this because its sessions
ran without the app's defaults. It now starts with them.

**What changed:**
- The breadcrumbs find their symbols from the lines around the cursor
  only: back to the last line with no indent, at most 10 000 lines. The
  finder starts afresh at such a line, so the result is the same. A test
  checks every line of several files against the whole-file answer.
- Whether a symbol's block reaches the cursor is checked up to the first
  line with text at or past the cursor, not to the block's end.
- A plugin's symbols, and Markdown's, which do need the whole file, are
  worked out once per text version and plugin update, not per frame.
- The finder reads only the first 1 024 characters of a line, so a
  minified file's one line isn't copied.
- The scrollbar's git marks come from a list of the marked lines, not
  from every line's entry.
- A long line keeps checkpoints of its columns. They're built as far
  along as asked, and kept through edits before them. Finding a column
  far along a 20 MB line starts near it.
- `large_probe` times frames as the window draws them, and fails if one
  takes over 2 ms. Find, the minimap and word wrap are the later steps,
  and are noted as such.

**The effect, in the app** (same harness, 45 s):

| File | CPU before → after | Idle before → after |
|---|---|---|
| 85 MB text | 44.6 s → 1.2 s | 100% → 0% of a core |
| 342 MB log, every line different | 49.7 s → 2.3 s | 100% → 0% |
| 20 MB Rust | 26.9 s → 1.0 s, 141 → 107 MB | 0% → 0.1% |
| 3.9 MB real Rust | 7.2 s → 0.9 s | 0% |
| 19 MB JSON on one line | 11.4 s → 5.0 s | 0% |

VS Code used 19–41 s of CPU on the same files, and idled at 2–9% of a
core.

**In `large_probe`**, a frame is now 0.1–0.6 ms on every file, whatever
its size: 0.3 ms on the 342 MB log, against 3.7 ms before the git marks
and 5.4 s before the breadcrumbs.

**Still to do from it:** the outline works symbols out on the window's
thread after every edit while it shows. That waits for the background
jobs of step 7.

### Fixed on this branch so far

Committed (8181070) and covered by tests:

- The widest line is measured once and then kept up to date.
- Scroll rows aren't counted when every line is one row.
- The git gutter diffs only the lines between the common start and end.
- Blame shares the text instead of copying it.
- Long lines are built only around the screen.
- Column and word lookups read the rope in place.

The effect, in `large_probe`:

| | Before | After |
|---|---|---|
| A frame after an edit, 85 MB / 342 MB | 2.9 s / 33 s | 1.5 ms / 3.6 ms |
| A keystroke in a file git tracks, 85 MB | 150 ms | 22 ms |
| A frame on a 23 MB one-line file | 0.8–5 s | 0.1 ms |

### Still slow, or heavy

On the 85 MB / 342 MB files:

| | Today | Why |
|---|---|---|
| Open Find and type a word | 20 s / 70 s | Every line copied into a `Vec<char>`, several times a keystroke |
| A frame with Find open | 6.4 s / 23 s | The scrollbar finds every match again each frame |
| Find next; Ctrl+D | 3.2 s / 12 s; 5 s / 17 s for three | All matches found to pick one |
| A frame with the minimap on | 2.3 s / 7.7 s | Built for every line; it shows a few hundred |
| A frame with word wrap on | 2.6 s / 8.7 s | Every line wrapped again after any change |
| Idle | ~~a whole core~~ fixed in step 1 | Symbols for the breadcrumbs, each frame |
| Opening | 3.2× the file at its peak | Read as a String, made a rope, git's copy kept beside it |
| Git gutter after many edits | wrong | The diff gives up after 100 ms and marks 4.3 million lines (prototype 9) |
| Backups of unsaved files | the whole file copied twice on the window's thread, then written, after each pause | `Backups::plan` asks `text_of` for each file |
| Undo of a large delete | the removed text as a String | `Edit` keeps `removed: String` |
| Colours | none above 512 KB | Kept for every token at 24 B, placed by copying lines (prototype 3) |

## What the prototypes show

### 1. Line facts (`lines`)

Each line's display width and its rows when wrapped, in blocks of 1 024
lines, each block with its total rows and widest line. 2 000 random edits
(characters, new lines, deletions across lines, long lines that wrap) are
then compared against building from scratch, and give the same result
every time.

| | 20 MB Rust | 85 MB text | 342 MB log |
|---|---|---|---|
| Build, a line at a time | 477 ms | 892 ms | 2 990 ms |
| Build, one pass over the bytes | 82 ms | 355 ms | 1 470 ms |
| Build, on every core | 15 ms | 46 ms | 168–203 ms |
| An edit, per-line facts kept | 6.9 µs | 7.5 µs | 10.7–11.8 µs |
| Rows before a line, line at a row | 1.0 µs | 0.9–1.3 µs | 2.2–2.5 µs |
| Total rows and widest line | 1.0 µs | 1.4 µs | 3.6 µs |
| An edit, totals only (block measured again) | 360 µs | 542 µs | 590–630 µs |
| Rows before a line, totals only | 182 µs | 269 µs | 295–314 µs |

The totals-only layout re-measures a block's 1 024 lines, a line at a
time, for an edit and for a lookup in it. With the one-pass measure it
would be about a sixth of that, but this isn't measured yet.

Memory, from the counting allocator (prototype 7):

| Layout | Per line | 342 MB log |
|---|---|---|
| Width and rows, `u32` each | 8 B | 37.5 MB (0.11×) |
| `u16` width, `u8` rows | 4 B | 18.7 MB (0.055×) |
| Each block's totals only | 0.02 B | 0.1 MB |

### 2. Search (`search`, `threads`)

| | 20 MB Rust | 85 MB text | 342 MB log, common | 342 MB log, rare |
|---|---|---|---|---|
| Today's way: a `Vec<char>` a line | 704 ms | 1 993 ms | 8 085 ms | 7 052 ms |
| Exact text, `memchr` over chunks | 8 ms | 39 ms | 160 ms | 77 ms |
| Regex over whole lines, any case | 44 ms | 201 ms | 738 ms | 118 ms |
| Regex on every core | 8 ms | 32 ms | 112 ms | 24 ms |
| One screen's matches only | 0.04 ms | 0.04 ms | 0.04 ms | 0.02 ms |

All count the same matches. Turning matches into character positions
costs 0.1 ms for a screen's 1 000, but 618 ms for all 4.9 million.

How many threads to use, finding every match in the 342 MB log:

| Threads | Time | CPU used in all |
|---|---|---|
| 1 | 505 ms | 510 ms |
| 2 | 273 ms | 520 ms |
| 4 | 154 ms | 560 ms |
| 8 | 97 ms | 620 ms |
| 22 | 71 ms | 940 ms |

Four threads take 70% less time for 10% more CPU. All 22 save another
83 ms but cost 84% more CPU, so the editor should use about four.

### 3. Colours (`tokens`, `syntax`, `memory`)

Keeping colours for a 20 MB Rust file (10 million tokens):

| | Today | In blocks | As runs |
|---|---|---|---|
| Memory | 232 MB (11.6×) | 116 MB (5.8×) | 39 MB (2.0×) |
| Moving them for an edit | 51 ms | 8 µs | — |
| Placing what a plugin sends | 11 s; hours on one long line | 0.4 s from bytes | — |

Even the tightest layout costs as much as the file. Tree-sitter's tree is
far larger. Here are large real Rust files, each in its own process:

| | `Extensions/mod.rs` (3.9 MB) | `vk/definitions.rs` (2.1 MB) | `encoding_rs/data.rs` (2.5 MB) |
|---|---|---|---|
| First parse | 553 ms | 241 ms | 664 ms |
| The tree's memory | 105 MB (27×) | 42 MB (20×) | 163 MB (65×) |
| A keystroke: edit, parse again | 5.2 ms | 18.8 ms | 0.2 ms |
| Colours for one screen | 0.52 ms | 0.19 ms | 0.26 ms |
| Parse 2 000 lines around the screen | 19 ms | 7 ms | 60 ms |
| That window's tree | 4.3 MB | 1.5 MB | 2.7 MB |

Parsing from the rope's chunks (`parse_with_options`) is as fast as from a
String, or faster, so a plugin needs no String. A made-up 20 MB file built
a 1.6 GB tree. In a plugin that tree lives in wasm memory, which stops at
4 GB.

So colours for the screen cost a fraction of a millisecond to work out.
Keeping a whole file's tree and tokens costs 20–80 times the file. The
lean way is to keep colours only around the screen, and to keep a tree
only while it's small. Past that, parse a window around the screen.

### 4. Work off the window's thread (`background`)

On the 342 MB log, a worker searched a snapshot while the window's thread
typed:

- A snapshot (a rope clone) takes 0.02 µs.
- The screen's matches came after 0.48 ms, and all 4.9 million after
  735 ms.
- The window's thread typed 1.1 million characters meanwhile. Its slowest
  edit was 0.56 ms.
- When the version moved on, the worker stopped 0.35 ms later.

A highlighting plugin keeping up with a keystroke:

| | 20 MB | 342 MB |
|---|---|---|
| Today: the whole text as a String, then compare | 38 ms | 592 ms |
| Apply the change to its own rope | 0.02 ms | 0.02 ms |

### 5. Opening (`load`)

342 MB, each way in its own process:

| | Time | Peak memory |
|---|---|---|
| Today: String, then rope, git's copy kept | 910 ms | 1 080 MB (3.2×) |
| Streamed into the rope (`Rope::from_reader`) | 510 ms | 397 MB (1.16×) |
| Mapped, line starts every 1 024 lines | 122 ms | the file's own pages |

### 6. Positions and ropes (`units`, `memory`)

| 342 MB log | `ropey`, characters | `ropey`, bytes | `crop`, bytes |
|---|---|---|---|
| Memory | 1.135× | | 1.046× |
| Line of a position | 0.87 µs | 0.90 µs | 0.56 µs |
| An insert | 2.4 µs | — | 3.6 µs |

Characters cost no more than bytes, and another rope gains little.

### 7. The git gutter from hashes (`gitsigns`)

Git's version kept as an 8-byte hash a line, and the text's lines hashed
as they change, then diffed as numbers. After 30 random edits:

| | 20 MB Rust | 342 MB log |
|---|---|---|
| Git's version kept | 10 MB instead of 20 MB | 37.5 MB instead of 342 MB |
| Slowest gutter after an edit, today | 208 ms | 1 492 ms |
| Slowest gutter after an edit, from hashes | 27 ms | 74 ms |
| Lines marked: today / from hashes | 76 / 76 | 4 281 185 / 78 |

On the Rust file both mark the same lines. On the log, today's diff gives
up at its 100 ms limit and marks 4.3 million lines that didn't change.

### 8. Undo (`memory`)

| | 20 MB | 85 MB | 342 MB |
|---|---|---|---|
| Undo of deleting half, removed text kept as a String | 15 MB | 62 MB | 246 MB |
| Undo of deleting half, the rope from before kept | under 0.1 MB | under 0.1 MB | under 0.1 MB |

The rope from before shares nearly all its nodes with the text now, so
keeping it costs only the nodes the edit replaced.

## The design

### The text

`Buffer` keeps its `ropey::Rope`, its version and its list of `Change`s.
What changes around it:

- **Opening** streams the file into a rope on the disk lane, and sends the
  rope, not a String: 1.16× the file at its peak, against 3.2×.
- **Saving, blame, plugins and backups** take a `TextSnapshot` and read
  its chunks. Blame does this already.
- **Undo** keeps an edit's text while it's small. For a large one it keeps
  the rope from before, which shares its memory with the text now.
- **Git's version** is kept as a hash a line (8 B), not as the text. The
  gutter diffs hashes in the background. When a hunk needs git's text, to
  show or revert it, `git show` reads it then.
- **Backups** become a journal. Each pause in typing appends the edits
  since the last one, written from the disk lane. The journal is
  rewritten as a whole only once it outgrows the file. A pause in a
  342 MB file writes a few bytes, not 342 MB.

### What's kept about the text

Everything kept about the text is updated from the edits as they happen.
`Doc::sync` hands each kept thing the changes and the rope after them,
and nothing is keyed by version any more.

- **`LineIndex`**: each line's width and its rows when wrapped, in blocks
  of 1 024 lines with totals. It answers the scroll rows, the widest line,
  which line is at a row, and the wrap.
  - Per line it costs 4 bytes (`u16` width, saturating, and `u8` rows), or
    0.1 MB in all with the totals-only layout, at about 0.5 ms an edit (prototype 1). The best of both: totals for the file, and per-line facts kept only for the blocks near the screen.
  - It's built on a few cores when a file opens or the wrap width
    changes, at 15–200 ms. Until then each line counts as one row.
  - Bracket depth joins it later, as each line's change in depth, so the
    bracket colours need no scan before the screen.
- **`RangeMap<T>`**: ranges counted from their block's start, for
  diagnostics, inlays, folds and symbols. An edit moves only its own
  block, in 5–12 µs.
- **Colours** are kept for the screen and a few screens around it, as runs
  of 3–4 bytes, in a `RangeMap`. The plugin is asked for more as the view
  moves. The screen's colours come in well under a millisecond, and the
  last ones stay shown until new ones come, so scrolling never shows
  plain text. Colours for the whole file are never kept.
- **Search** keeps a count per block, for the total and the scrollbar's
  marks, and nothing per match.
  - The screen's matches are found when it's drawn (0.04 ms).
  - Find next and Ctrl+D search forward from the cursor, and stop at the
    first match.
  - Select All Occurrences is the one that needs every match, and makes
    them then.
- **The minimap** draws its few hundred lines from the screen's
  surroundings, as the editor does. It never builds every line.
- **Symbols** for the breadcrumbs, the outline and sticky scroll come from
  the plugin, or from the built-in finder run once per version in the
  background. They're never worked out on a frame.

### Frames

- Each part of the view is cached until what it shows changes. The text
  rows are cached by the screen's lines, their versions and the colours.
  The breadcrumbs are cached by the cursor's line and the symbols'
  version.
- A cursor blink redraws the cursor only, and a window event that
  changes nothing shown doesn't rebuild the view.
- `large_probe` gains limits: a frame on any file must take under 2 ms,
  and must not grow with the file. CI fails if one does.

### Background work

The core stays free of IO and threads. Whole-file work is an effect that
carries a snapshot, run by the app and answered with an event that says
which version it's for:

- **Threads**: a few workers at low priority (four: 70% less time for 10% more CPU than one thread), not one per
  core. A job for an old version stops within a fraction of a millisecond.
- **What runs there**: search counts, git hunks, the built-in symbol
  finder, building `LineIndex`, and backups.
- **Debounced**: the gutter and the counts wait for a pause in typing, as
  blame does now. Typing doesn't start a whole-file job per key.
- **Tests**: the script `Session` runs jobs straight away, so tests stay
  as they are.

### Plugins

API changes are fine during development. Bump the API, the WIT, the SDK
and the manifests together.

- **Syncing**: the grammar SDK syncs with `active-changes`, which exists
  already. It keeps its own rope and parses from its chunks.
- **Colours on request**: the editor asks for colours for a range of
  lines when the screen needs them. The plugin answers from its tree,
  with byte offsets that need no placing.
- **Large files**: past a size, a tree costs 20–80× the file. Above it,
  the plugin parses a window of lines around the screen (1.5–4.3 MB,
  7–60 ms) and moves the window as the screen moves. The size is to be
  chosen by measuring, perhaps 8 MB. The 512 KB limit goes.
- **Until then**: `pos_at_utf16` uses the rope's UTF-16 count instead of
  copying the line. It gives the same positions, from twice as fast to
  over 100 000 times faster on long lines.

### Files too large to load

This is optional, and needs a mock-up. Above a share of memory, the file
opens read-only over the mapped file, with line starts every 1 024 lines
(122 ms for 342 MB). Search runs over the mapping. Editing means loading
it, with a notice saying so.

## Order

Each step ships on its own. Each keeps the 459 tests in
`crates/core/tests` passing, and is checked with `large_probe` and the
comparison with VS Code.

1. **Fix the idle and per-frame costs.** Done; see "Step 1" above.
   - The breadcrumbs' symbols come from the lines around the cursor.
   - The whole-file lists are kept once per version.
   - Long lines keep column checkpoints.
   - The scrollbar's git marks are a short list.
   - Limits added to `large_probe`.

   Stopping a blink from rebuilding the view wasn't needed: a whole frame
   now costs under a millisecond.
2. **Search without lists.**
   - Search the rope's bytes.
   - Counts per block.
   - The screen's matches on demand.
   - Find next and Ctrl+D search forward.

   This fixes Find, Find next and Ctrl+D (seconds to milliseconds) with no
   UI change.
3. **`LineIndex`.** Rows, widest line and wrap kept by edits. This fixes
   word wrap (2.6–8.7 s a frame) and replaces three caches.
4. **The minimap from its window only.** This fixes the minimap (2.3–7.7 s
   a frame).
5. **One copy of the file.**
   - Stream into the rope.
   - Send ropes, and save from a snapshot.
   - Git's version as hashes.
   - Undo keeps large edits as rope snapshots.
   - Backups as a journal.

   This takes memory from 3.2× the file to about 1.2× and makes the gutter
   correct.
6. **`RangeMap`, and colours on request.**
   - Diagnostics, inlays and folds move to `RangeMap`.
   - `pos_at_utf16` uses the rope's count.
   - The grammar SDK syncs by changes, answers ranges in bytes, and parses
     windows of large files (API bump).
   - The 512 KB limit goes.

   This brings colours to large files at a few MB.
7. **Background jobs and their UI.** Search counts, git hunks and
   `LineIndex` builds off the window's thread, with the count that fills
   in. The count needs a mock-up first.
8. **Later, if wanted:** positions in bytes; a read-only view for files
   too large to load.

## Looked at and left

- **Another rope** (`crop`), or **one of our own** as Zed has, with these
  facts in its nodes. It saves 8% of memory and 30% of lookup time, but
  nothing slow is in the rope. A structure beside it gets the speed
  without owning a rope.
- **A piece table** as VS Code has. Streaming into the rope gets to 1.16×
  already, and mapping covers files too large to load.
- **Positions in bytes first.** It touches about 600 places for little
  gain on its own.
- **Keeping colours for the whole file** in a compact layout. It still
  costs 1–2× the file, plus the tree at 20–80×, for colours nobody is
  looking at.
- **A collaborative text type (CRDT).** Nothing here needs one.
