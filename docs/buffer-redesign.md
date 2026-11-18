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
6. **Hold one copy of the file.** Git's version shares the text's rope,
   and opening, saving and backups read the rope where it lies.

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

### Step 2: search without lists, done

Find copied every line into a `Vec<char>` and made a list of every
match, several times a keystroke and again on each frame (the count, the
scrollbar and the find box each asked). Find next and Ctrl+D made the
whole list to pick one match from it.

**What changed:**
- `search.rs` searches the rope where it lies. A query typed as it is
  can't reach past a line's end, so it's searched 64 KB of whole lines at
  a time, with `memchr` for exact text and the regex crate for case
  ignored. The gaps between matches give their lines and characters, as
  the rope counts them. A regular expression is searched a line at a
  time, read in place unless the line crosses two of the rope's chunks.
- `Found` keeps a count for each block of 1 024 lines, and which of its
  64 bands of 16 lines have a match: 16 bytes a block, 1 MB for the
  342 MB log's 4.9 million lines. Nothing is kept per match. An edit
  counts again only the blocks it touched. Each change now records its
  lines as the rope counts them, as a lone `\r` ends a line too.
- The find box's total comes from the counts. Which match is selected
  counts the blocks before it and searches its own block.
- The scrollbar marks each line with a match in a file of up to 65 536
  lines. In a larger one it marks each band with one, which is finer than
  a pixel, at most about 8 000 marks.
- Find next and previous, Ctrl+D and Ctrl+K Ctrl+D search from the cursor
  and stop at the first match, skipping blocks with none.
- Select All Occurrences, Replace All and a plugin's surface still need
  every match, and make the list then, from the same search.
- A query grown as it's typed searches only the bands of lines the last
  one matched in, as each of its matches is inside one of those. A rare
  word costs 0.3 ms a key once it's a few letters long.
- Vim's `n`, `N` and match count use the same search and counts, read in
  place. A pattern that names a line break (`\n`) is still searched over
  a copy of the whole text, as it can match across lines.

**The effect, in `large_probe`** (85 MB text / 342 MB log):

| | Before | After |
|---|---|---|
| Open Find and type a 5-letter word | 20 s / 70 s | 0.34 s / 0.56 s |
| A key typed in Find, a rare word | | 0.3 ms once a few letters long |
| A frame with Find open | 6.4 s / 23 s | 0.8 ms / 0.6 ms |
| Find next; Find previous | 3.2 s / 12 s | 0.2 ms; 0.3 ms |
| Ctrl+D three times | 5 s / 17 s | 0.1 ms |

On the 20 MB Rust file the 5-letter word takes 36 ms. The `regex` crate's
literal optimisations made no difference: they were already on through
another crate.

**Still to do from it:**
- The first letter in Find, a Backspace, or a toggle still counts the
  whole file on the window's thread: about 120 ms for exact text on the
  342 MB log, close to the 77–160 ms the prototype found for one thread.
  Counting on four threads in the background, with a count that fills in,
  is step 7.

### Step 3: wrapped rows kept by edits, done

With word wrap on, the rows before each line were a list made again
after any edit, by copying every line into characters and wrapping it.
On the 342 MB log that took 8.2 s, for every frame after an edit. Moving
to the end took 0.7 s more, scrolling a row at a time to get there.

**What changed:**
- `rows.rs` keeps each line's rows when wrapped as a byte. A line of 255
  rows or more keeps its count in a map. There are sums for each 64
  lines and the rows before each block of 1 024. That's 1 byte a line,
  4.9 MB for the log, kept for at most two widths, as two groups side by
  side may differ.
  - An edit wraps again only the lines it touched.
  - The rows before a line take a few dozen additions, and the line at a
    row a binary search of the blocks.
  - A line that can't be wider than its row is counted without wrapping:
    no wider in bytes than the row, with no tab and no inlay hint.
  - New inlay hints wrap again only the lines with hints, before and
    after. Each document counts the times its hints were set.
  - Edits not yet synced are followed as they're asked about, so the
    rows are right within the event that made the edit.
- The row index keeps the wrapped rows, and puts folds and the rows views
  open between lines (peeks, conflicts, an agent's changes, test results)
  on top, as a short list. The lines that may have such rows are asked
  for, not every line.
- Which screen row a line is on comes from the row index, not from
  wrapping each line from the top of the view. A line the kept rows say
  takes one row isn't wrapped to draw it.
- Moving the cursor far down scrolls straight there.
- The scrollbar drops a mark close to the last of its kind, closer than
  its few thousand places can show. A 1 MB paste had left 65 000 git
  marks, each looked up a frame.

**The effect, in `large_probe`** (85 MB text / 342 MB log):

| | Before | After |
|---|---|---|
| A frame wrapped, after an edit | 2.6 s / 8.2 s | 0.7 ms / 0.9 ms |
| Turning wrap on | 2.6 s / 8.2 s at its first frame | 0.15 s / 0.5 s, once |
| Ctrl+End wrapped | 0.64 s / 0.73 s | 14 ms / 15 ms |

Wrapped frames are now held to the 2 ms limit.

**Still to do from it:**
- Turning wrap on, or a new width, wraps every line on the window's
  thread, at 0.5 s for the log. Resizing the window with wrap on does it
  for each width. That moves to the background in step 7.
- A key typed takes 110–190 ms on the large files, wrapped or not. That
  isn't the wrap. The git gutter diffs on each key, which step 5 changes.
- The widest line stays the cache step 1 made, which edits already keep.

### Step 4: the minimap from its window only, done

The minimap was built for every line after any edit: a colour for each
character of the file, then each line's runs. The window draws the few
hundred that fit.

**What changed:**
- The minimap gets the lines around the view, 1 280 above and below.
  That's more than a 4K screen's height shows at two pixels a line, and
  the map shows lines from at most its height above the view to its
  height below. The lines sent start on a multiple of 512, so scrolling
  builds them again only now and then. With them come the file's line
  count, for the slider, and the first line sent.
- Colours are worked out for each line's first 120 characters only,
  which is as many as the map shows, and not at all when the file has
  none. Lines are read where they lie.
- Git marks are kept as runs of lines with the same sign. The scrollbar
  takes every *n*th line of a run, as many as it has places for, and the
  minimap takes the lines it was sent. A gutter marked all through, as
  the diff gives when it runs out of time on a large file, had been 1.7
  million marks copied every frame.

**The effect, in `large_probe`** (85 MB text / 342 MB log):

| | Before | After |
|---|---|---|
| A frame with the minimap, after an edit | 2.3 s / 7.7 s | 1.3 ms / 1.5 ms |
| A frame with the minimap, nothing new | 2.3 s / 7.7 s | 0.2 ms |
| A frame after an edit at the end of the file | 20–27 ms, from the git marks | 0.4 ms |

Every frame in `large_probe` is now held to the 2 ms limit, with no
step excused.

**Still to do from it:** a key typed at the end of the file takes 0.49 s
/ 1.7 s, wrapped or not. The git gutter diffs everything from the first
change to the last, gives up after 100 ms, and marks every line. Step 5's
line hashes fix that.

### Step 5: one copy of the file, done

Measured in the app on files git tracks (`measure.py`, 20 s), an 85 MB
file settled at 201 MB and peaked at 439 MB, 5.2× the file. The text was
held whole up to four times over:
- read as a String before the rope was made from it
- git's version read whole, copied into a String, and kept beside the
  text
- `git blame`'s answer for every line, 181 MB for this file, read whole
  and copied again
- saving and backups copying it into Strings on the window's thread

**What changed:**
- **Git's version** (`gitbase.rs`) is a clone of the text's rope with
  git's lines put back only where they differ. The two share every
  unchanged piece, so a file as committed costs nothing more.
  - Which lines are the same in both is kept as runs. An edit cuts the
    runs it touches, and only the gaps between runs are diffed again.
  - Lines with no match on the other side are left out before diffing,
    as GNU diff does, so a paste of thousands of lines costs nothing to
    diff. A line typed in is compared, not hashed.
  - The gutter can't give up and mark the whole file any more, and the
    sign for each line is gone: the signs are runs too.
  - This replaced the plan of a hash a line: that costs 8 B a line, and
    git's text would have to be read again to show or revert a change.
- **Reading git's version**: the app streams `git show` and compares it
  with the rope as it comes. It keeps only what follows the first
  difference, which is nothing for a file as committed.
- **Opening** reads the file straight into a rope (`Rope::from_reader`)
  on the disk thread, and the event carries the rope.
- **Saving** sends the rope and writes its pieces. **Backups** take each
  unsaved file's rope, with their first line put in front, which copies
  only the start.
- **Blame** of a file over 20 000 lines asks git for the lines around the
  view (`-L`), 2 000 each side, asked again when the view leaves them.
  Git's answer for every line had been 2.1× the file. Each line's commit
  is a `u32` now.

**The effect, in the app** (settled / peak):

| | Before | After |
|---|---|---|
| 85 MB prose | 201 / 439 MB | 186 / 359 MB |
| 120 MB log, every line different | 237 / 547 MB | 225 / 346 MB |

Merlin settles at its own 78 MB plus 1.14× the file. What's left of the
peak is `git blame`'s own process, 185–257 MB for under a second: git
holds the file twice however few lines are asked for.

**In `large_probe`** (85 MB / 342 MB):

| | Before | After |
|---|---|---|
| A key typed | 125 / 190 ms | 0.1 ms |
| A key typed at the end of the file | 0.49 / 1.7 s | 0.1 ms |
| Opening, with git's version | 0.3 / 0.9 s | 0.23 / 0.85 s |
| Saving | 0.66 s (342 MB) | 0.46 s |

**Looked at and left:**
- **Undo as rope snapshots.** Prototype 8 counted only what an edit
  allocates. Keeping the rope from before stops the removed text being
  freed, so what's held after a large delete is about the same as the
  String undo keeps now. What it would save is one copy of the removed
  text while the delete runs, and that isn't worth changing every `Edit`.
- **A journal for backups.** A pause in a large unsaved file still writes
  the whole file, from the disk thread, without copying it first.

**Still to do from it:**
- `git blame` costs git twice the file for under a second. Above some
  size it could be left off.
- Saving trims trailing spaces by going through every line.
- A key on the 20 MB one-line file takes 26–41 ms, from other work on
  that line.

### Step 6: colours for large files, done

Files over 512 KB had no colours: the grammar SDK read the whole text,
and a tree-sitter tree costs 20–80× the file. Small files paid too. On
every key, each grammar plugin copied the whole text into its own memory
and compared it with the last copy to find what changed. The Rust, C++
and Catch2 plugins copied it again, to look for tests, even when the file
was too large to use.

**What changed** (plugin API 0.17; `docs/plugin-registry.md` section 14):
- **New host calls**: `active-lines`, `active-size` and `active-view`,
  and a `viewed` buffer event once the view moves 256 lines or more.
- **Grammar SDK, files up to 512 KB**: they're parsed whole. The SDK keeps
  the text and follows `active-changes`, so a key typed costs the change,
  not a copy of the file.
- **Grammar SDK, larger files**: a window of about 1 000 lines either side
  of the view is parsed, from a line at its left edge. Edits in it are
  followed and an edit above it moves it. Once the view comes within 250
  lines of its edge, the lines around the view are read and parsed again.
  A new window takes about 40 ms for the Rust grammar.
- **What the editor keeps**: a large file's colours within 3 000 lines of
  the latest window. Colours further away go, so scrolling through a
  large file doesn't build up colours for all of it.
- **`pos_at_utf16`**, which places every token, diagnostic and edit a
  plugin sends, counts UTF-16 with the rope rather than copying the line.
- **Plugins** check `active-size` before reading the whole text.

**The effect, in the app.** A 20 MB Rust file is coloured wherever the
view is. With the plugins loaded it settles at 171 MB, against 87 MB for
a tiny Rust file, and uses 1.1 s of CPU over 20 s.

**Looked at and left:**
- **`RangeMap`.** Diagnostics, inlays, folds and tokens are still lists
  that each edit goes through. For a large file the tokens are now only
  those near the view, and the others are few, so each edit moves a few
  thousand at most. That's about a tenth of a millisecond.
- **Colours on request.** The editor doesn't ask for a range of lines;
  the plugin hears where the view is and colours around it. That's the
  same result with one call fewer per window.

**Still to do from it:**
- **Language servers.** The LSP SDK still sends the whole text when a
  file opens, as the protocol asks. The 20 MB Rust file made
  rust-analyzer use 6.3 GB and eight cores. A server for large files
  is better left off, perhaps above a size; VS Code turns off most
  features for large files.
- ~~**Windows from the left edge.**~~ Fixed after step 6. A window's
  parse starts at a line at the left edge after a blank line. Starting
  inside a multi-line string or comment turns what follows inside out,
  which leaves errors in the tree; tree-sitter marks the branches that
  hold one, so counting them is cheap. A parse with errors is tried
  again from further up, twice at most, keeping the one with the fewest
  errors. A clean window still takes one parse (44 ms); one that started
  in a string takes 117 ms.

### Step 7: whole-file work in the background, done

Built from `docs/mockups/BackgroundWork.html`, approved 2026-11-18.

**What changed:**
- **Jobs** (`work.rs`): the core stays free of threads. It asks for a
  `Job`, a search count or a wrap, with `Effect::Work`, carrying a
  snapshot of the rope. A job is split into pieces of about 65 000 lines,
  each a pure function, and each piece's result comes back as
  `Event::Worked`. A script's session runs every piece at once, so the
  tests see the finished result.
- **The app's workers** (`crates/app/src/work.rs`): four threads at low
  priority take a job's pieces in order, so the count fills in from the
  top, and send each one as it's done. A newer job of the same kind stops
  the one before it.
- **Find on a file over 8 MB**: it's counted in the background. Until
  then, blocks not yet counted are searched as they're met.
  - The box reads "Searching…", then "1,245,184 so far" with a spinner and
    a line along its foot for how far the count has got. "12 of 1,204+"
    shows on a match once the blocks before it are counted. "No results"
    shows only once the whole file is searched.
  - A query grown from a finished count still narrows at once.
  - Find next searches at most 65 536 lines that aren't counted yet. If it
    finds nothing there, it waits for the count and tries again as pieces
    come in. A query that matches nothing no longer searches the whole
    file on the window's thread for every key.
- **Word wrap on a file over 8 MB**: the lines within 200 of the view are
  wrapped at once, and the rest count as one row each until the
  background job fills them in. The status bar shows "Wrapping 41%" with
  a spinner. An edit while it wraps asks for a new job, since the lines
  have moved.

**In the app**, on a 360 MB log: typing "request" counted 4 823 097
matches, through "1,245,184 so far" and "3,801,088 so far", in about a
third of a second. A whole regular expression typed key by key went in
within 0.07 s, where each key had waited seconds for a full search.
Turning wrap on showed "Wrapping 0%" for a moment. In `large_probe` the
times include the work, as its session runs jobs at once, on one thread.

**Not moved:** the git gutter and the outline's symbols. Since step 5 the
gutter costs what the edits touched. The outline still finds symbols
over the whole file after each edit while it shows; it can move the same
way.

### Step 8: files too large to load, done

Built from `docs/mockups/LargeFileView.html`, approved 2026-11-18.

**The idea**: the editor works on a rope, so a file too large to load is
shown as a window of it, about 4 MB of whole lines, read from disk. As the
view nears the window's edge, the window there is read and takes its
place. The cursor, the selection and the view stay on the same text: they
go by their byte in the file. What's kept besides the window costs a few
bytes for each 4 MB piece, so the file's size doesn't matter.

**What changed:**
- **`bigfile.rs`**: what reads the file, on any thread.
  - Line breaks counted as the rope counts them, a lone `\r` and LS and
    the rest included.
  - Windows read from a line's start, with the line they start on when
    the count has got there.
  - A search's matches counted by piece, and the nearest match to a byte,
    going round the end.
- **Background work**: `Kind::Lines` counts the lines by piece and
  `Kind::FileCount` counts a search's matches, both on the workers from
  step 7.
- **Opening** (`crates/app/src/main.rs`): before reading, the size is
  compared with Settings › Files, `large_file_read_only`.
  - Auto opens read-only above a quarter of the memory free. On Linux that
    comes from `/proc/meminfo`; elsewhere the limit is 2 GB.
  - It can also be "never" or a size such as "2 GB".
  - A file too large opens with `Event::OpenedLarge`, and nothing is
    read but its first window.
- **In the editor** (`editor/large.rs`):
  - The bar over the text says why the file is read-only, with Load to
    Edit…. The tab has a lock. The status bar shows "Read-only", and
    "Counting lines 38%" while the lines are counted.
  - The gutter and the status bar number lines as the file does. Before
    the count reaches a window, the numbers show as · and the status bar
    gives the byte.
  - The scrollbar shows the whole file. Dragging it, Go to Line,
    Ctrl+Home and Ctrl+End read the window where they go. Go to Line
    waits for the count when it hasn't reached that line yet.
  - Find counts every match in the background. Next and previous search
    the window, then the file past it, skipping pieces the count says have
    none.
  - Typing, pasting or any other edit asks first. The notice gives the
    memory loading takes (1.15× the file) against what's free, and Load to
    Edit is offered only when it fits. Loading keeps the place.
  - A log that grows: with the view at the end, the new lines are read
    and followed. Elsewhere, the bar says how much it grew, with Go to
    End. A file that shrank is read again.
  - Plain text only: no colours, git gutter, blame, word wrap, language
    servers or plugins, which see no file while it's shown.
- **Never saved over**: a large file's text is only a window, so the
  core makes no save for it. The app refuses one too. Agents, formatting
  and putting back changes treat it as not open.

**In the app**, release build:

| | 360 MB log | 2.5 GB log |
|---|---|---|
| Memory, all told | 132 MB (540 MB loaded) | 142 MB |
| Lines counted | under 1.5 s | about 3 s |
| Typing a query | 0.05 s | |

The count agreed with `grep`: 72 matches of `id=0000`. Ctrl+End reached
line 33 761 680, Go to Line 2 400 000 went straight there, and Find from
line 3 000 000 went to the next match in a later window. Load to Edit
loaded the file at the same line and column.

**Left from the mock-up:**
- Loading to edit shows "Loading… to edit" without how far it has got,
  and has no Cancel.
- Selecting and copying work within the window, about 4 MB, so Select All
  selects the window only. The 64 MB question never comes up.
- The tab row doesn't show the size and the lines. The bar says the size.
- Positions in bytes, the other half of step 8, weren't needed.

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
| Open Find and type a word | ~~20 s / 70 s~~ 0.34 s / 0.56 s since step 2 | Every line copied into a `Vec<char>`, several times a keystroke |
| A frame with Find open | ~~6.4 s / 23 s~~ fixed in step 2 | The scrollbar finds every match again each frame |
| Find next; Ctrl+D | ~~3.2 s / 12 s; 5 s / 17 s for three~~ fixed in step 2 | All matches found to pick one |
| A frame with the minimap on | ~~2.3 s / 7.7 s~~ fixed in step 4 | Built for every line; it shows a few hundred |
| A frame with word wrap on | ~~2.6 s / 8.7 s~~ fixed in step 3 | Every line wrapped again after any change |
| Idle | ~~a whole core~~ fixed in step 1 | Symbols for the breadcrumbs, each frame |
| Opening | ~~3.2× the file at its peak~~ fixed in step 5 | Read as a String, made a rope, git's copy kept beside it |
| Git gutter after many edits | ~~wrong~~ fixed in step 5 | The diff gives up after 100 ms and marks 4.3 million lines (prototype 9) |
| Backups of unsaved files | ~~copied twice on the window's thread~~ shared since step 5; the whole file still written each pause | `Backups::plan` asks `text_of` for each file |
| Undo of a large delete | the removed text as a String; left, see step 5 | `Edit` keeps `removed: String` |
| Colours | ~~none above 512 KB~~ fixed in step 6 | Kept for every token at 24 B, placed by copying lines (prototype 3) |

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
keeping it costs only the nodes the edit replaced. But this counts what
the edit allocates, not what stays held: the removed nodes, kept by the
rope from before, aren't freed. Held afterwards, the two cost about the
same (see step 5).

## The design

### The text

`Buffer` keeps its `ropey::Rope`, its version and its list of `Change`s.
What changes around it:

- **Opening** streams the file into a rope on the disk lane, and sends the
  rope, not a String: 1.16× the file at its peak, against 3.2×.
- **Saving, blame, plugins and backups** take a `TextSnapshot` and read
  its chunks. Blame does this already.
- **Undo** keeps an edit's text, as it does. Keeping the rope from before
  instead was looked at; see step 5.
- **Git's version** is a clone of the text's rope with git's lines put
  back where they differ, so the two share every unchanged piece. Built
  in step 5, in place of the hash a line first planned: it costs nothing
  for a file as committed, and git's text is there to show and revert.
- **Backups** share the rope and write its pieces. A journal of edits,
  so a pause in a 342 MB file writes a few bytes, is still to do.

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

Built in step 8. Above a share of memory, the file opens read-only, read
from disk a window at a time rather than mapped: if another program cut a
mapped file short, reading it would crash the editor. The lines are
counted in pieces of 4 MB. Search runs over the file a piece at a time.
Editing means loading it, with a notice saying so.

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
2. **Search without lists.** Done; see "Step 2" above.
   - Search the rope's bytes.
   - Counts per block.
   - The screen's matches on demand.
   - Find next and Ctrl+D search forward.

   This fixes Find, Find next and Ctrl+D (seconds to milliseconds) with no
   UI change.
3. **`LineIndex`.** Done as `WrapRows`; see "Step 3" above. Rows and wrap
   kept by edits. This fixes word wrap (2.6–8.7 s a frame).
4. **The minimap from its window only.** Done; see "Step 4" above. This
   fixes the minimap (2.3–7.7 s a frame).
5. **One copy of the file.** Done; see "Step 5" above.
   - Stream into the rope.
   - Send ropes, and save from a snapshot.
   - Git's version shares the text's rope (in place of hashes).
   - Blame around the view in large files.
   - Undo snapshots and a backup journal were looked at and left.

   This takes merlin's own memory to its 78 MB plus about 1.14× the file,
   and makes the gutter correct.
6. **`RangeMap`, and colours on request.** Done as colours around the
   view; see "Step 6" above. `RangeMap` was left.
   - Diagnostics, inlays and folds move to `RangeMap`.
   - `pos_at_utf16` uses the rope's count.
   - The grammar SDK syncs by changes, answers ranges in bytes, and parses
     windows of large files (API bump).
   - The 512 KB limit goes.

   This brings colours to large files at a few MB.
7. **Background jobs and their UI.** Done; see "Step 7" above. Search
   counts and the wrap off the window's thread, with the count that fills
   in, as `docs/mockups/BackgroundWork.html` shows.
8. **Files too large to load.** Done; see "Step 8" above. Read-only, a
   window at a time from disk, as `docs/mockups/LargeFileView.html`
   shows. Positions in bytes were left.

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
