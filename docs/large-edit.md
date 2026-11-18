# Editing files too large to load, in place

A plan, 2026-11-18; all seven steps built the same day, as the Order
section says. It follows step 8 of `docs/buffer-redesign.md`, where a
file too large to load opens read-only: the editor holds about 4 MB of it
at a time, as a rope, and reads the next part from disk as the view moves.
Here, such a file can be edited where it is, without loading it, at the
memory its edits take. The mock-up is `docs/mockups/LargeFileEdit.html`, approved 2026-11-18.

## In short

- **The text becomes pieces.** Each piece is either a range of the file on
  disk or text in memory. The rope the editor holds stays a window of
  about 4 MB, read through the pieces.
- **Editing happens in the window**, with everything that works in any
  file: typing, multiple cursors, undo. When the view moves on, the edited
  window is kept in memory with its undo, and the next is read.
- **Large edits never load the text.** Deleting a million lines, or
  everything above the cursor, cuts the list of pieces.
- **Saving streams the pieces** to a new file next to the old one and
  swaps it in, as merlin saves now. On this machine that writes 1 GB a
  second: about 15 s for 14 GB, with progress in the bar.
- **Memory stays small:** the window, the edited windows up to a budget,
  and the text typed. Past the budget, the oldest edited windows are
  folded into pieces, which keeps their changes and drops their undo.
- **Still off:** colours, the git gutter, blame, word wrap, language
  servers and plugins. They need the whole text.

## How others do it

- **VS Code, Sublime, Zed, Helix and Vim** load the whole file. VS Code
  turns colours and other features off for large files.
- **EmEditor and UltraEdit** are known for opening files of many GB,
  keeping much of the file on disk, with temporary files.
- **Hex editors** (HxD, 010 Editor) keep the file on disk and the changes
  as a piece table. Saving rewrites the file, or writes in place when no
  length changed.

What's planned here is the hex editors' way under a text editor's view.

## The design

### Pieces

```text
Large {
    file:    the file, kept open while edits refer to it
    pieces:  [Piece]                         in order, end to end
    window:  bytes start..end of the text    the rope the editor holds
    parked:  [Parked]                        edited windows kept whole
}
Piece = File { from, to } | Text(Rope)       each with its bytes and lines
```

- **Places become bytes of the text**, not of the file: from the start of
  the first piece. Before any edit they're the same, so step 8's code is
  the case of one `File` piece. Lines are counted per piece, so a window's
  first line is the sum of the lines in the pieces before it.
- **The window is read through the pieces**, on a background thread as
  now. `Effect::ReadWindow` carries the pieces, which are cheap to copy,
  since the ropes in them are shared.
- **The counts by piece** that step 8 keeps for the file stay as they are.
  A `File` piece's lines are the counts it covers, and its ends are counted
  when it's cut. Only the text typed is counted again.

### Editing in the window

The window's rope is a normal `Buffer`, so every edit works as it does
in any file, with undo.

- **Leaving an edited window** parks it: its `Buffer`, undo and all, takes
  the place of the pieces it covered, as one `Text` piece. Coming back to
  it reads it back with its undo.
- **Undo past the window** goes to the last edited window, reads it back
  and undoes there. Undo goes in the order of the edits, across the file.
- **The memory budget**, about 256 MB by default, covers the parked
  windows. Past it, the oldest parked window is folded. What it changed is
  worked out from its buffer's history and kept as `Text` pieces. The rest
  is cut back to `File` pieces. Its undo goes, and the notice says so the
  first time.
- **An edit across the window's edge**, such as a selection reaching past
  it or Delete at its end, first reads a window that covers both sides.

### Large edits without the text

These work on the list of pieces, at any size:

- Deleting a selection that reaches past the window: Shift+Ctrl+End then
  Delete, or lines selected from Go to Line.
- Cut and copy of such a selection. Past 64 MB they ask first, since the
  clipboard holds it in memory.
- Select All, then Delete or typing over it.

Replace All across the file is planned for later, as a change applied to
the pieces as they're read.

### Saving

1. **Disk space**: saving needs room for the whole new file. If there
   isn't room, it says so before starting, and nothing is written.
2. **Streamed to a new file** beside the old one, read through the pieces
   a few MB at a time on the disk thread, flushed to disk, then renamed
   over the old file. This is how saves work now, so the file is whole
   whatever happens.
3. **Progress shows** in the bar: "Saving 41%… 5.8 of 14.2 GB". The text
   is read-only until the save is done: edits made while it's written
   would land on text that's moving. Cancel stops the save, deletes the
   new file, and leaves the old one as it was.
4. **After saving**, the pieces become the new file's. Undo still works
   across the save:
   - On Linux and macOS, the old file stays readable through the handle
     merlin keeps open, even after the rename.
   - On Windows, the old file is opened so it can be renamed while open.
   - Undo to before the save gives back the edits; the file on disk is
     as saved.

### Quitting with changes

- **Hot exit** keeps a journal, not the text: the `Text` pieces, where
  each `File` piece points, and the file's size, change time and inode.
  That is a few KB for most edits.
- **The next start** checks that the file is the same and brings back the
  edits. If it isn't, it says so and opens the file as it is.

### The file changing on disk while edited

- **Replaced**, as most programs and git save: the edits are safe, since
  the handle still reads the old file. merlin asks: Keep My Changes, which
  overwrites on save, or Reload, which drops them.
- **Grown at the end**, as a log grows: the edits are safe. The new lines
  show past the last piece.
- **Cut short or rewritten in place**: the `File` pieces may now read other
  bytes. merlin stops editing and asks to Save As a new file from what
  it still holds, which is all of the edits.
  - On Windows, opening the file without allowing writes stops this
    happening.
  - On Linux and macOS, nothing can stop it. merlin notices from the size
    and change time when the window is read next.

### What changes on screen

From the mock-up:

- **The bar**: "14.2 GB, edited from disk. Colours, git and plugins are
  off; saving writes the whole file (about 15 s)." There's no Load to
  Edit…, which stays in the menu for files that fit.
- **Unsaved edits** put the dot on the tab, as for any file. The status
  bar says "Edited from disk".
- **Saving** shows progress in the bar with a progress line. If it fails,
  the bar says why, with Retry and Save As….
- **Notices** for: no room on disk, undo dropped from old edits, the file
  replaced, the file cut short, and copying past 64 MB.
- **The read-only view stays** as an option: Settings › Files › Large
  files gets "Edit in place" (the default) or "Read-only".

## Order

Each step ships on its own, with tests, and keeps the step 8 tests
passing.

1. **Pieces under the read-only view.** Done. Nothing changes on screen.
   `bigfile::FileText` is the file, opened once and kept, with its list
   of pieces. Windows, line counts, match counts and Find's search all
   read through it, using positional reads that threads can share. Places
   are bytes of the text. Read-only, the text is one piece, the whole
   file. A test edits the list by hand, with text between two ranges of
   the file and a line split across two pieces. Windows, line numbers,
   counts and search all read it as one text.
2. **Editing in the window**, parking and undo across windows. Done.
   - **Every edit goes into the text.** After each event that changed
     the window, its rope takes its place in the pieces, so reading,
     counting and searching see the edits. Copies of the rope share its
     nodes, so this costs little.
   - **Undo across windows.** A window left with undo or redo is kept,
     with its buffer. Undo past the window brings back the one edited
     before, and undoes there. Redo goes the other way.
   - **Line numbers** come from each piece's lines. A range of the file
     counts from the background count of the file as opened, plus the
     breaks from a block's start to each place a window started or ended.
     Those places are the only ones the file's ranges are cut at.
   - **On screen**: the FROM DISK bar in the accent colour, the tab's dot,
     and "Edited from disk" in the status bar. Settings › Files gains
     `large_file_mode`, "edit" (the default) or "read_only".
   - **Jumps** read again closer, up to three times, when the window read
     doesn't hold the line: a line's place in the file is known only to a
     block.
   - **In the app**, on a copy of the 360 MB log: typing 1 200 characters
     added 0.5 MB, at 0% CPU when idle. Undo went from the top window to
     the end of the file in turn.

   **Left for later steps:**
   - Saving: built in step 3. Hot exit keeps the edits: step 6.
   - The Find count starts again after each edit: fixed in step 4.
   - A file changing on disk while edited: handled in step 6.
   - A window left with things to redo goes on the undo stack. Redo then
     takes the windows undone past first.
3. **Saving**: streamed to a new file, the disk space check, progress,
   undo kept after saving. Done.
   - **How it saves**: `bigfile::save` reads the text a block at a time,
     through its pieces. It writes each block to a new file beside the
     old one, counting its lines as it goes, then flushes and renames it
     over the old one. It never writes in place, as the text reads the
     old file.
   - **Room is checked first** (statvfs on Unix). If there isn't enough,
     the bar says "NO ROOM" with Save As…, and nothing is written.
   - **In the app**, it runs on a thread of its own, so other saves
     aren't held up. It wakes the window every 64 MB, and the bar shows
     "Saving… 460 MB of 2.5 GB" with a line along its foot and Cancel.
     The status bar shows "Saving 18%". Edits and undo wait until it's
     done.
   - **After saving**, the text is one piece, the new file, with the
     lines counted while writing, so nothing is counted again. The
     window, the cursor and undo stay as they are, since the text is the
     same. Undo goes on past the save, and leaves the file unsaved
     again.
   - **Cancel or a failure** removes the new file and leaves the old one
     as it was, with the edits still here. A failure shows "NOT SAVED"
     with Retry and Save As…. Quitting or closing waits for the save, and
     stops if it fails.
   - **Save As** writes the text to the new place, edited or not, and
     reads from there afterwards.
   - **In the app**, a 2.5 GB log edited in three places saved in about
     4.8 s, all three edits in the file. Memory stayed at 146 MB. Cancel
     part-way left the file as it was, with nothing beside it.
4. **Counts after edits**: lines and Find over the pieces, counting only
   the `Text` pieces again. Done.
   - **Lines** have come from each piece's own count since step 2.
   - **Find counts in three kinds of part** (`bigfile::Part`):
     - blocks of the file as opened, which edits never move;
     - the matches before each place a range of the file is cut, in its
       block. Cuts are always at a line's start, so a match never spans
       one.
     - pieces of text in memory.

     A range of the file has the matches before its end less those
     before its start.
   - **After an edit**, the pieces kept whole keep their counts
     (`FileText::splice_keeping`). Only the window's new text, and any new
     cut, are counted again, in the background. Jobs for more of the same
     search add to the one under way rather than stopping it.
   - **Searching past the window** skips blocks of the text that lie in
     the file where its blocks, and those either side, have no match.
   - **In the app**, on the 360 MB log: "72 found", the same as `grep`.
     Typing another match with Find open read "73 found" within 0.15 s,
     with the CPU idle.
   - **Left**: after a save the count starts again, over the new file. A
     line an edit joins across a window's edge could be counted in two
     parts.
5. **Large edits on the pieces**: cut, copy and delete past the window.
   Done.
   - **A selection past the window** keeps its far end as a byte of the
     text (`Large::anchor`). In the window it shows from the window's
     edge. Shift+Ctrl+End, Shift+Ctrl+Home and Select All make one. The
     status bar gives its size: "(2.4 GB selected)".
   - **Deleting it, or typing or pasting over it,** splices the pieces
     without reading what's between. The window's edits are kept with
     their undo first. The cut goes on the undo stack as one step, with
     the text before and after (`Edited::Pieces`), and the window is read
     again where it was. Undo and redo swap the text back and forth. A
     save marks those steps saved, as it does windows.
   - **Lines stay numbered:** each new place a range of the file is cut
     has its breaks counted at once, from at most a block of the file.
   - **Cut and copy** read the range on the disk thread, asking first past
     64 MB. A cut takes the text out only if nothing changed while it was
     read.
   - **In the app**, on a 2.5 GB log: from line 1 000 001 to the end
     selected as 2.4 GB, deleted at once, leaving 74.3 MB. Saving wrote
     its 1 000 000 lines. Memory stayed at 152 MB.
   - **Left**: typing a bracket over such a selection puts the bracket in
     its place rather than around it. The Find count can be off by
     matches on the two lines a cut in the middle of a line joins, until
     saved. The copy question is a notice, not a dialog.
6. **Hot exit** as a journal, and the file changing on disk. Done.
   - **A hot exit** no longer asks about large files. Each one with edits
     is kept as a `bigfile::Journal`: its pieces, the text in memory
     written out, and which file the rest reads (its size, change time
     and inode). Journals go in `.large/` among the backups.
   - **At the next start** the file opens again, even when the workspace
     opened it already, and the edits go back in if the file is the same.
     If it changed meanwhile, a notice says the changes couldn't be put
     back.
   - **The journal holds whole edited windows**: 4.2 MB for one line
     edited in a 360 MB log, not a few KB. Step 7's folding would bring
     it down to the changes. It's written at a hot exit only, not
     after a crash.
   - **Saved over by another program** (a new inode at its path): the
     edits are safe, as the handle kept reads the old file. A notice asks
     once: Reload, which drops them and reads the new file from the same
     place, or Keep My Changes, which overwrites it on save.
   - **Grown in place:** what's edited stays right, and nothing is asked.
   - **Cut short in place:** the bar says "CHANGED" with Save As…. Editing,
     undo and saving over the file are refused; Save As… writes what's
     still there to a new file.
   - **A reopen never drops edits:** opening a file that's already open
     with edits leaves it as it is.
   - **In the app**: an edit at line 3 000 000 of a copy of the 360 MB
     log, then closing the window, then starting again. The edit came
     back, with "Put back the unsaved changes to edit.log".
   - **Left**: on Windows, opening the file so others can't write it, and
     renaming over it while it's open, aren't done yet.
7. **The memory budget**: folding the oldest parked windows. Done.
   - **Folding** (`bigfile::fold`, `FileText::folded`): ranges of the
     file only ever go forward. So each run of text in memory between two
     of them took the place of the file between them, and is compared
     with it line by line (Myers, on hashes of lines). Lines that are the
     same become ranges of the file again, and only the lines changed stay
     in memory. The text reads the same, so the window and undo stay
     right, and nothing has to be tracked as windows overlap. Runs, or
     the file between, of more than 64 MB aren't folded.
   - **The budget**, 256 MB by default (`Editor::set_large_budget`), is
     what the windows kept for undo and the text in memory hold. Past it,
     the oldest windows let their undo go, along with any older steps,
     which can't be undone past them, until the rest is under half. Then
     the text is folded. A notice says so once. What they changed is all
     kept.
   - **Journals** are folded too, so a hot exit keeps the lines changed:
     8 KB for 70 edits across a 2.5 GB log, against 4.2 MB for one edit
     before.
   - **Every place a range of the file starts or ends** has its breaks
     counted whenever the pieces change (`Large::count_cuts`), as does the
     window's start. The window run caught this: folding turned typed text
     back into ranges of the file, a later edit cut one at a place never
     counted, and Go to Line then waited for a count that never came.
   - **Freed memory is given back** to the system after folding
     (`malloc_trim`, on Linux with glibc).
   - **In the app**: 70 edits 480 000 lines apart in a 2.5 GB log.
     Memory rose to about 340 MB by the 30th and stayed at 330–380 MB to
     the 70th, where keeping every window would pass 480 MB. All 70 were
     in the saved file, each on its own line.
   - **Left**: the budget isn't in Settings yet. A run of typed text
     standing for more than 64 MB of the file, as after typing over a
     huge selection, stays whole.

Later: Replace All across the file, and in-place writes for edits that
keep every length, which would save a 14 GB file in milliseconds.

## Risks

- **A file rewritten in place by another program** while it has edits
  here: on Linux and macOS nothing prevents it, so merlin can only notice
  and stop. This is the one case where edits could read the wrong bytes.
  Saving them is refused.
- **Undo across many windows** holds those windows in memory until the
  budget folds them. Typing a character in each of 1 000 places 4 MB
  apart parks 1 000 windows. That costs about 256 MB and then folds,
  keeping the edits.
- **Saving takes as long as writing the file.** On a slow or network
  disk, 14 GB may take minutes, during which the text can't be edited.
- **Windows**: renaming over a file merlin has open, and the share modes,
  need checking on a real Windows machine.
