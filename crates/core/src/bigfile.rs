//! Files too large to load: shown read-only, a window of a few MB at a
//! time read from disk, while the whole file's lines and a search's matches
//! are counted on the background threads (`docs/buffer-redesign.md`,
//! step 8, and `docs/mockups/LargeFileView.html`).
//!
//! The text the editor holds is the window, as a rope, so everything that
//! draws and moves in a file works on it as it is. What's kept besides is
//! where the window is in the text and the counts by block, a few bytes a
//! 4 MB block: a 14 GB file costs a few MB, whatever its size.
//!
//! The text is a list of [`Piece`]s: ranges of the file, and text in
//! memory. Read-only, it's one piece, the whole file; editing in place
//! (`docs/large-edit.md`) adds the others. Places are bytes of the text,
//! which are the file's until it's edited.
//!
//! The file is opened once and read here, by whichever thread the host
//! runs the work on; the core's state changes only as events bring what
//! was read.

use std::fs::File;
use std::io;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ropey::Rope;

use crate::search::Matcher;

/// Bytes a block of the background counts reads.
pub const BLOCK: u64 = 4 << 20;
/// About how much of the text a window holds.
pub const WINDOW: u64 = 4 << 20;
/// How far an edge looks for a line's end before it cuts the line: a line
/// longer than this is shown in parts.
const LONG_LINE: u64 = 16 << 20;

/// Line breaks in `bytes`, as the rope counts them: `\n`, `\r\n` once, a
/// lone `\r`, VT, FF, NEL, LS and PS. `next` is the byte after, which says
/// whether a `\r` at the end is half of a `\r\n`.
pub fn breaks(bytes: &[u8], next: Option<u8>) -> u64 {
    let newlines = memchr::memchr_iter(b'\n', bytes).count() as u64;
    // Most files have nothing else.
    let rare = memchr::memchr3(b'\r', 0x0b, 0x0c, bytes).is_some()
        || memchr::memchr2(0xc2, 0xe2, bytes).is_some();
    if !rare {
        return newlines;
    }
    let mut count = newlines;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                let after = bytes.get(i + 1).copied().or(next);
                if after != Some(b'\n') {
                    count += 1;
                }
            }
            0x0b | 0x0c => count += 1,
            0xc2 if bytes.get(i + 1) == Some(&0x85) => count += 1,
            0xe2 if bytes.get(i + 1) == Some(&0x80)
                && matches!(bytes.get(i + 2), Some(0xa8 | 0xa9)) =>
            {
                count += 1
            }
            _ => {}
        }
        i += 1;
    }
    count
}

/// One piece of a large file's text: bytes of the file on disk, or text in
/// memory, as typed or as a window kept with its edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Bytes `from..to` of the file.
    File {
        from: u64,
        to: u64,
    },
    Text(Rope),
}

impl Piece {
    /// Its bytes.
    pub fn len(&self) -> u64 {
        match self {
            Piece::File { from, to } => to - from,
            Piece::Text(rope) => rope.len_bytes() as u64,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A large file's text: its pieces end to end, over the file, which is
/// kept open so they read the file they were made from. Copies share the
/// pieces and the file.
#[derive(Clone, Debug)]
pub struct FileText {
    pub path: PathBuf,
    file: Arc<File>,
    pieces: Arc<Vec<Piece>>,
    len: u64,
}

/// The same text: the same file, and pieces that are the same list.
impl PartialEq for FileText {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && Arc::ptr_eq(&self.file, &other.file)
            && Arc::ptr_eq(&self.pieces, &other.pieces)
    }
}

impl Eq for FileText {}

impl FileText {
    /// The whole of the file at `path`, as one piece.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        Ok(Self::of(
            path,
            Arc::new(file),
            vec![Piece::File { from: 0, to: size }],
        ))
    }

    /// `pieces` over the open `file`.
    pub fn of(path: &Path, file: Arc<File>, pieces: Vec<Piece>) -> Self {
        let pieces: Vec<Piece> = pieces.into_iter().filter(|p| !p.is_empty()).collect();
        let len = pieces.iter().map(Piece::len).sum();
        Self {
            path: path.to_path_buf(),
            file,
            pieces: Arc::new(pieces),
            len,
        }
    }

    /// The same file with other pieces.
    pub fn with_pieces(&self, pieces: Vec<Piece>) -> Self {
        Self::of(&self.path, self.file.clone(), pieces)
    }

    /// Its bytes in all.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// The byte of the file that byte `at` of the text is, if it's in a
    /// range of the file: at a piece's end, the next piece's start.
    pub fn file_pos(&self, at: u64) -> Option<u64> {
        let mut start = 0;
        let mut last = None;
        for piece in self.pieces.iter() {
            let stop = start + piece.len();
            if at < stop {
                return match piece {
                    Piece::File { from, .. } => Some(from + (at - start)),
                    Piece::Text(_) => None,
                };
            }
            last = Some((piece, stop));
            start = stop;
        }
        // The text's end: the last piece's end, if it's the file's.
        match last {
            Some((Piece::File { to, .. }, stop)) if stop == at => Some(*to),
            None if at == 0 => Some(0),
            _ => None,
        }
    }

    /// The line breaks in the file from the start of the block `at` is in
    /// up to `at`: what a cut of the file's ranges there needs to count its
    /// lines.
    pub fn file_breaks(&self, at: u64) -> io::Result<u64> {
        let from = at / BLOCK * BLOCK;
        let mut bytes = Vec::new();
        read_file_at(&self.file, from, at - from + 1, &mut bytes)?;
        let len = ((at - from) as usize).min(bytes.len());
        Ok(breaks(&bytes[..len], bytes.get(len).copied()))
    }

    /// The text with bytes `from..to` replaced by `with`: the pieces
    /// either side are cut there.
    pub fn splice(&self, from: u64, to: u64, with: Rope) -> Self {
        self.splice_keeping(from, to, with).0
    }

    /// As [`FileText::splice`], and for each of the new pieces, the old
    /// piece it is if it's kept whole: what was counted of it still holds.
    pub fn splice_keeping(&self, from: u64, to: u64, with: Rope) -> (Self, Vec<Option<usize>>) {
        let mut out: Vec<(Piece, Option<usize>)> = Vec::new();
        let mut start = 0;
        let mut placed = false;
        for (i, piece) in self.pieces.iter().enumerate() {
            let stop = start + piece.len();
            // What of it is before `from`, and after `to`.
            if start < from {
                let b = (from - start).min(piece.len());
                let whole = b == piece.len();
                out.push((cut(piece, 0, b), whole.then_some(i)));
            }
            if !placed && stop >= from {
                out.push((Piece::Text(with.clone()), None));
                placed = true;
            }
            if stop > to {
                let a = to.max(start) - start;
                out.push((cut(piece, a, piece.len()), (a == 0).then_some(i)));
            }
            start = stop;
        }
        if !placed {
            out.push((Piece::Text(with), None));
        }
        out.retain(|(p, _)| !p.is_empty());
        let kept = out.iter().map(|(_, k)| *k).collect();
        let pieces = out.into_iter().map(|(p, _)| p).collect();
        (self.with_pieces(pieces), kept)
    }

    /// The pieces bytes `a..b` of the text are, cut at both ends.
    pub fn pieces_in(&self, a: u64, b: u64) -> Vec<Piece> {
        let mut out = Vec::new();
        let mut start = 0;
        for piece in self.pieces.iter() {
            let stop = start + piece.len();
            if stop > a && start < b {
                out.push(cut(piece, a.max(start) - start, b.min(stop) - start));
            }
            start = stop;
        }
        out
    }

    /// Bytes `at..at + len` of the text, or fewer at its end, read from
    /// the pieces they're in.
    pub fn read(&self, at: u64, len: u64) -> io::Result<Vec<u8>> {
        let end = (at + len).min(self.len);
        let mut out = Vec::with_capacity(end.saturating_sub(at) as usize);
        let mut start = 0;
        for piece in self.pieces.iter() {
            let stop = start + piece.len();
            if stop > at && start < end {
                let (a, b) = (at.max(start) - start, end.min(stop) - start);
                match piece {
                    Piece::File { from, .. } => {
                        read_file_at(&self.file, from + a, b - a, &mut out)?
                    }
                    Piece::Text(rope) => {
                        let slice = rope.byte_slice(a as usize..b as usize);
                        for chunk in slice.chunks() {
                            out.extend_from_slice(chunk.as_bytes());
                        }
                    }
                }
            }
            if stop >= end {
                break;
            }
            start = stop;
        }
        Ok(out)
    }
}

/// Bytes `a..b` of a piece, as a piece.
fn cut(piece: &Piece, a: u64, b: u64) -> Piece {
    match piece {
        Piece::File { from, .. } => Piece::File {
            from: from + a,
            to: from + b,
        },
        Piece::Text(rope) => Piece::Text(Rope::from(rope.byte_slice(a as usize..b as usize))),
    }
}

/// Appends `len` bytes of `file` from `at`, without moving where the file
/// is read from, so threads can share it.
fn read_file_at(file: &File, at: u64, len: u64, out: &mut Vec<u8>) -> io::Result<()> {
    let start = out.len();
    out.resize(start + len as usize, 0);
    let mut done = 0;
    while done < len as usize {
        #[cfg(unix)]
        let n =
            std::os::unix::fs::FileExt::read_at(file, &mut out[start + done..], at + done as u64)?;
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(
            file,
            &mut out[start + done..],
            at + done as u64,
        )?;
        if n == 0 {
            break;
        }
        done += n;
    }
    out.truncate(start + done);
    Ok(())
}

/// The first line start at or after `at`: just after a `\n`, or the text's
/// start or end. A line longer than [`LONG_LINE`] is cut where a character
/// starts. The same `at` always gives the same place, so blocks cut by it
/// fit end to end.
fn line_start(text: &FileText, at: u64) -> io::Result<u64> {
    let size = text.len();
    if at == 0 || at >= size {
        return Ok(at.min(size));
    }
    let mut pos = at - 1;
    while pos < size && pos < at + LONG_LINE {
        let buf = text.read(pos, 64 << 10)?;
        if buf.is_empty() {
            break;
        }
        if let Some(i) = memchr::memchr(b'\n', &buf) {
            return Ok(pos + i as u64 + 1);
        }
        pos += buf.len() as u64;
    }
    if pos >= size {
        return Ok(size);
    }
    // No line end near: the first character's start.
    let head = text.read(at, 4)?;
    let skip = head.iter().take_while(|&&b| b & 0xc0 == 0x80).count();
    Ok(at + skip as u64)
}

/// Text read as UTF-8, any bytes that aren't standing in for a character
/// each.
fn text_of(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
    }
}

/// Part of the text, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// Its bytes in the text, from a line start.
    pub start: u64,
    pub end: u64,
    /// The text's size when read.
    pub size: u64,
    /// Which version of the text it was read from.
    pub generation: u64,
    /// The file's breaks before its ends, where they're in the file: see
    /// [`FileLines::edges`].
    pub edges: Vec<(u64, u64)>,
    pub text: Rope,
}

/// What a window is read from: about where, and which version of the text
/// it was asked of, which comes back with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct From {
    /// The window's middle is wanted about here.
    pub around: u64,
    pub generation: u64,
}

/// Where a window around `around` starts, before it's moved on to a
/// line's start: half a window before, but a whole window from the end.
fn window_start(around: u64, size: u64) -> u64 {
    around
        .min(size)
        .saturating_sub(WINDOW / 2)
        .min(size.saturating_sub(WINDOW))
}

/// The line breaks in bytes `from..to` of the text, read a block at a time.
fn breaks_in(text: &FileText, from: u64, to: u64) -> io::Result<u64> {
    let mut count = 0;
    let mut pos = from;
    while pos < to {
        let len = (to - pos).min(BLOCK);
        let bytes = text.read(pos, len + 1)?;
        let len = (len as usize).min(bytes.len());
        count += breaks(&bytes[..len], bytes.get(len).copied());
        if len == 0 {
            break;
        }
        pos += len as u64;
    }
    Ok(count)
}

/// Reads the window of `text` around `from.around`: about [`WINDOW`]
/// bytes of whole lines.
pub fn read_window(text: &FileText, from: From) -> io::Result<Window> {
    let size = text.len();
    let want = window_start(from.around, size);
    let start = line_start(text, want)?;
    let end = match start + WINDOW {
        e if e >= size => size,
        e => line_start(text, e)?.max(start + 1).min(size),
    };
    let bytes = text.read(start, end - start)?;
    let end = start + bytes.len() as u64;
    // Its ends where they're in the file, for the lines either side once
    // it's edited.
    let mut edges = Vec::new();
    for at in [start, end] {
        if let Some(f) = text.file_pos(at)
            && f % BLOCK != 0
        {
            edges.push((f, text.file_breaks(f)?));
        }
    }
    Ok(Window {
        start,
        end,
        size,
        generation: from.generation,
        edges,
        text: Rope::from_str(&text_of(bytes)),
    })
}

/// How many blocks a text of `size` bytes is counted in.
pub fn blocks(size: u64) -> usize {
    size.div_ceil(BLOCK).max(1) as usize
}

/// The line breaks in block `block` of `text`: the bytes from
/// `block * BLOCK`, up to the next block.
pub fn count_lines(text: &FileText, block: usize) -> io::Result<u64> {
    let at = block as u64 * BLOCK;
    breaks_in(text, at, (at + BLOCK).min(text.len()))
}

/// The lines that start in block `block`, as text, and the byte the first
/// starts at.
fn block_lines(text: &FileText, block: usize) -> io::Result<(u64, String)> {
    let at = block as u64 * BLOCK;
    let start = line_start(text, at)?;
    let end = line_start(text, at + BLOCK)?;
    if end <= start {
        return Ok((start, String::new()));
    }
    Ok((start, text_of(text.read(start, end - start)?)))
}

/// Calls `f` with each match's start in `text`, as a byte from its start:
/// many lines at once for a plain query, a line at a time otherwise.
fn each_match(
    matcher: &Matcher,
    text: &str,
    mut f: impl FnMut(usize) -> ControlFlow<()>,
) -> ControlFlow<()> {
    if matcher.is_plain() {
        return matcher.each(text, |r| f(r.start));
    }
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let bare = line
            .strip_suffix('\n')
            .map_or(line, |l| l.strip_suffix('\r').unwrap_or(l));
        matcher.each(bare, |r| f(at + r.start))?;
        at += line.len();
    }
    ControlFlow::Continue(())
}

/// The matches in block `block` of `text`.
pub fn count_matches(text: &FileText, matcher: &Matcher, block: usize) -> io::Result<u32> {
    let (_, lines) = block_lines(text, block)?;
    let mut count = 0u32;
    let _ = each_match(matcher, &lines, |_| {
        count += 1;
        ControlFlow::Continue(())
    });
    Ok(count)
}

/// The matches in the lines of the file that start in the block `cut` is
/// in, before `cut`: what the file's ranges cut there leave of the block's
/// count. `cut` is a line's start.
pub fn count_before(file: &FileText, matcher: &Matcher, cut: u64) -> io::Result<u32> {
    let start = line_start(file, cut / BLOCK * BLOCK)?;
    if cut <= start {
        return Ok(0);
    }
    let lines = text_of(file.read(start, cut - start)?);
    let mut count = 0u32;
    let _ = each_match(matcher, &lines, |_| {
        count += 1;
        ControlFlow::Continue(())
    });
    Ok(count)
}

/// The matches in text in memory.
pub fn count_rope(matcher: &Matcher, rope: &Rope) -> u32 {
    let text = rope.to_string();
    let mut count = 0u32;
    let _ = each_match(matcher, &text, |_| {
        count += 1;
        ControlFlow::Continue(())
    });
    count
}

/// What a count of a search in a large file is done by: a block of the
/// file as opened, what's before a place its ranges are cut, or a piece
/// of text in memory, for the text's version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Block(usize),
    Cut(u64),
    Text { piece: usize, generation: u64 },
}

/// Counts one part of a search in a large file.
pub fn count_part(
    file: &FileText,
    text: &FileText,
    matcher: &Matcher,
    part: Part,
) -> io::Result<u32> {
    match part {
        Part::Block(block) => count_matches(file, matcher, block),
        Part::Cut(at) => count_before(file, matcher, at),
        Part::Text { piece, .. } => Ok(match text.pieces().get(piece) {
            Some(Piece::Text(rope)) => count_rope(matcher, rope),
            _ => 0,
        }),
    }
}

/// The nearest match to `from` in `text`, as the byte it starts at: the
/// first at or after it going forward, the last before it going back.
/// Blocks `counts` says have none are skipped without reading.
pub fn seek(
    text: &FileText,
    matcher: &Matcher,
    from: u64,
    forward: bool,
    counts: &[Option<u32>],
) -> io::Result<Option<u64>> {
    let n = blocks(text.len());
    let first = ((from / BLOCK) as usize).min(n - 1);
    let order: Box<dyn Iterator<Item = usize>> = if forward {
        Box::new(first..n)
    } else {
        Box::new((0..=first).rev())
    };
    for block in order {
        if counts.get(block).copied().flatten() == Some(0) {
            continue;
        }
        let (start, lines) = block_lines(text, block)?;
        let mut hit = None;
        let _ = each_match(matcher, &lines, |at| {
            let at = start + at as u64;
            match forward {
                true if at >= from => {
                    hit = Some(at);
                    ControlFlow::Break(())
                }
                false if at < from => {
                    hit = Some(at);
                    ControlFlow::Continue(())
                }
                false => ControlFlow::Break(()),
                true => ControlFlow::Continue(()),
            }
        });
        if hit.is_some() {
            return Ok(hit);
        }
    }
    Ok(None)
}

/// As [`seek`], going round from the other end of the text when there's
/// none that way: none only when it has no match at all.
pub fn seek_round(
    text: &FileText,
    matcher: &Matcher,
    from: u64,
    forward: bool,
    counts: &[Option<u32>],
) -> io::Result<Option<u64>> {
    if let Some(at) = seek(text, matcher, from, forward, counts)? {
        return Ok(Some(at));
    }
    let other = if forward { 0 } else { u64::MAX };
    seek(text, matcher, other, forward, counts)
}

/// Which file a large file's pieces read: its size, when it last changed
/// and, on Unix, its inode, which another program saving over it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub size: u64,
    pub modified: u128,
    pub inode: u64,
}

impl Identity {
    fn from_meta(meta: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(meta);
        #[cfg(not(unix))]
        let inode = 0;
        Self {
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos()),
            inode,
        }
    }

    /// The file at `path` now.
    pub fn of(path: &Path) -> Option<Self> {
        std::fs::metadata(path).ok().map(|m| Self::from_meta(&m))
    }
}

impl FileText {
    /// The file the pieces read, through the handle kept: as opened, even
    /// if another file has since taken its name.
    pub fn identity(&self) -> Option<Identity> {
        self.file.metadata().ok().map(|m| Identity::from_meta(&m))
    }

    /// Whether the file at its path is still the one it reads, not one
    /// saved over it.
    pub fn same_file(&self) -> bool {
        match (self.identity(), Identity::of(&self.path)) {
            (Some(a), Some(b)) => a.inode == b.inode,
            _ => false,
        }
    }
}

/// A large file's unsaved edits, kept by a hot exit for the next start:
/// its pieces, the text in memory written out, and which file the rest
/// reads, so they're put back only on the same file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    /// The file, as the document names it.
    pub path: PathBuf,
    pub identity: Identity,
    pub pieces: Vec<Piece>,
}

const JOURNAL: &str = "merlin large-file journal 1";

impl Journal {
    /// Writes it, a few lines and then the text in memory as it is.
    pub fn write(&self, to: &Path) -> io::Result<()> {
        use std::io::Write;
        let mut out = io::BufWriter::new(File::create(to)?);
        let id = &self.identity;
        writeln!(out, "{JOURNAL}")?;
        writeln!(out, "{}", self.path.display())?;
        writeln!(
            out,
            "{} {} {} {}",
            id.size,
            id.modified,
            id.inode,
            self.pieces.len()
        )?;
        for piece in &self.pieces {
            match piece {
                Piece::File { from, to } => writeln!(out, "F {from} {to}")?,
                Piece::Text(rope) => {
                    writeln!(out, "T {}", rope.len_bytes())?;
                    for chunk in rope.chunks() {
                        out.write_all(chunk.as_bytes())?;
                    }
                }
            }
        }
        out.flush()
    }

    /// Reads one written by [`Journal::write`].
    pub fn read(from: &Path) -> io::Result<Self> {
        use std::io::BufRead;
        let bad = || io::Error::new(io::ErrorKind::InvalidData, "not a large-file journal");
        let mut input = io::BufReader::new(File::open(from)?);
        let mut line = String::new();
        let mut next = |input: &mut io::BufReader<File>| -> io::Result<String> {
            line.clear();
            input.read_line(&mut line)?;
            Ok(line.trim_end_matches('\n').to_string())
        };
        if next(&mut input)? != JOURNAL {
            return Err(bad());
        }
        let path = PathBuf::from(next(&mut input)?);
        let head = next(&mut input)?;
        let nums: Vec<u128> = head.split(' ').filter_map(|n| n.parse().ok()).collect();
        let [size, modified, inode, count] = nums[..] else {
            return Err(bad());
        };
        let mut pieces = Vec::new();
        for _ in 0..count {
            let piece = next(&mut input)?;
            let mut parts = piece.split(' ');
            let kind = parts.next();
            let nums: Vec<u64> = parts.filter_map(|n| n.parse().ok()).collect();
            match (kind, &nums[..]) {
                (Some("F"), [from, to]) => pieces.push(Piece::File {
                    from: *from,
                    to: *to,
                }),
                (Some("T"), [len]) => {
                    let mut bytes = vec![0; *len as usize];
                    io::Read::read_exact(&mut input, &mut bytes)?;
                    pieces.push(Piece::Text(Rope::from_str(&text_of(bytes))));
                }
                _ => return Err(bad()),
            }
        }
        Ok(Self {
            path,
            identity: Identity {
                size: size as u64,
                modified,
                inode: inode as u64,
            },
            pieces,
        })
    }
}

/// Why a large file's save didn't happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    /// Stopped by Cancel: the file is as it was.
    Cancelled,
    /// The disk hasn't room for the whole new file: nothing was written.
    NoRoom { need: u64, free: u64 },
    /// Writing failed part way: the file is as it was.
    Failed(String),
}

/// What a save of a large file wrote: the line breaks in each block of
/// the new file, so its lines are known at once.
pub type Saved = Result<Vec<u64>, SaveError>;

/// Writes `text` to `path` a block at a time: to a new file beside it,
/// flushed to disk, then renamed over it, so the file is whole whatever
/// happens. Never in place, as the text reads the old file. `written` is
/// told how far it's got after each block, and `stop` set cancels it.
/// `free` is the room on the disk, if known.
pub fn save(
    text: &FileText,
    path: &Path,
    free: Option<u64>,
    written: &dyn Fn(u64),
    stop: &std::sync::atomic::AtomicBool,
) -> Saved {
    use std::io::Write;
    use std::sync::atomic::Ordering;
    let need = text.len();
    // What's already there is freed only once the new file takes its
    // place, so the whole new file needs room.
    if let Some(free) = free
        && free < need
    {
        return Err(SaveError::NoRoom { need, free });
    }
    let failed = |err: io::Error| SaveError::Failed(err.to_string());
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(SaveError::Failed("no folder to save in".into()));
    };
    let temp = dir.join(format!(".{}.merlin-save", name.to_string_lossy()));
    let result = (|| {
        let mut file = File::create(&temp).map_err(failed)?;
        let mut counts = Vec::with_capacity(blocks(need));
        let mut at = 0;
        while at < need {
            if stop.load(Ordering::Relaxed) {
                return Err(SaveError::Cancelled);
            }
            let bytes = text.read(at, BLOCK + 1).map_err(failed)?;
            let len = (BLOCK as usize).min(bytes.len());
            if len == 0 {
                return Err(SaveError::Failed("the text ended early".into()));
            }
            counts.push(breaks(&bytes[..len], bytes.get(len).copied()));
            file.write_all(&bytes[..len]).map_err(failed)?;
            at += len as u64;
            written(at);
        }
        if counts.is_empty() {
            counts.push(0);
        }
        file.sync_all().map_err(failed)?;
        if let Ok(meta) = std::fs::metadata(path) {
            std::fs::set_permissions(&temp, meta.permissions()).map_err(failed)?;
        }
        std::fs::rename(&temp, path).map_err(failed)?;
        Ok(counts)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// A window's text as pieces of what it was read from, `origin`, and only
/// the lines it changed as text in memory: what a window kept whole for
/// its undo is folded to, when its undo goes, or for a journal.
pub fn fold(origin: &FileText, text: &Rope) -> io::Result<Vec<Piece>> {
    let old = text_of(origin.read(0, origin.len())?);
    let new = text.to_string();
    let starts = |s: &str| -> Vec<usize> {
        let mut at = vec![0];
        at.extend(s.match_indices('\n').map(|(i, _)| i + 1));
        if *at.last().expect("one") != s.len() {
            at.push(s.len());
        }
        at
    };
    let (old_at, new_at) = (starts(&old), starts(&new));
    let lines = |s: &str, at: &[usize]| -> Vec<u64> {
        // Each line by a hash, to compare as numbers.
        at.windows(2)
            .map(|w| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                s[w[0]..w[1]].hash(&mut h);
                h.finish()
            })
            .collect()
    };
    let (a, b) = (lines(&old, &old_at), lines(&new, &new_at));
    let mut out: Vec<Piece> = Vec::new();
    let push_text = |out: &mut Vec<Piece>, s: &str| {
        if s.is_empty() {
            return;
        }
        if let Some(Piece::Text(last)) = out.last_mut() {
            last.append(Rope::from_str(s));
        } else {
            out.push(Piece::Text(Rope::from_str(s)));
        }
    };
    for op in similar::capture_diff_slices(similar::Algorithm::Myers, &a, &b) {
        match op {
            similar::DiffOp::Equal { old_index, len, .. } => {
                let (x, y) = (old_at[old_index] as u64, old_at[old_index + len] as u64);
                out.extend(origin.pieces_in(x, y));
            }
            similar::DiffOp::Delete { .. } => {}
            similar::DiffOp::Insert {
                new_index, new_len, ..
            }
            | similar::DiffOp::Replace {
                new_index, new_len, ..
            } => push_text(
                &mut out,
                &new[new_at[new_index]..new_at[new_index + new_len]],
            ),
        }
    }
    // Ranges of the file next to each other, one again.
    let mut merged: Vec<Piece> = Vec::new();
    for piece in out {
        match (merged.last_mut(), &piece) {
            (Some(Piece::File { to, .. }), Piece::File { from, to: end }) if *to == *from => {
                *to = *end
            }
            _ => merged.push(piece),
        }
    }
    Ok(merged)
}

/// Runs of text in memory longer than this, or standing for more of the
/// file than this, aren't folded: comparing them would read too much.
const FOLD_MOST: u64 = 64 << 20;

impl FileText {
    /// The same text with each run of text in memory folded to the lines
    /// it changed. Ranges of the file only ever go forward, so a run
    /// between two of them took the place of the file between them, which
    /// is what it's compared with. The text reads the same.
    pub fn folded(&self) -> Self {
        let pieces = self.pieces();
        let mut out = Vec::new();
        let mut i = 0;
        while i < pieces.len() {
            let Piece::Text(_) = pieces[i] else {
                out.push(pieces[i].clone());
                i += 1;
                continue;
            };
            let run_end = (i..pieces.len())
                .find(|&j| matches!(pieces[j], Piece::File { .. }))
                .unwrap_or(pieces.len());
            let run = &pieces[i..run_end];
            let from = match out.last() {
                Some(Piece::File { to, .. }) => *to,
                _ if i == 0 => 0,
                _ => u64::MAX,
            };
            let to = match pieces.get(run_end) {
                Some(Piece::File { from, .. }) => *from,
                _ => self.file_len(),
            };
            let len: u64 = run.iter().map(Piece::len).sum();
            let folded = (from <= to && to - from <= FOLD_MOST && len <= FOLD_MOST)
                .then(|| {
                    let mut rope = Rope::new();
                    for piece in run {
                        if let Piece::Text(r) = piece {
                            rope.append(r.clone());
                        }
                    }
                    let origin = self.with_pieces(vec![Piece::File { from, to }]);
                    fold(&origin, &rope).ok()
                })
                .flatten();
            match folded {
                Some(f) => out.extend(f),
                None => out.extend(run.iter().cloned()),
            }
            i = run_end;
        }
        // Ranges of the file next to each other, one again.
        let mut merged: Vec<Piece> = Vec::new();
        for piece in out {
            match (merged.last_mut(), &piece) {
                (Some(Piece::File { to, .. }), Piece::File { from, to: end }) if *to == *from => {
                    *to = *end
                }
                (Some(Piece::Text(last)), Piece::Text(r)) => last.append(r.clone()),
                _ => merged.push(piece),
            }
        }
        self.with_pieces(merged)
    }

    /// The size of the file the text reads, through the handle kept.
    fn file_len(&self) -> u64 {
        self.file.metadata().map_or(0, |m| m.len())
    }

    /// Bytes of the text that are in memory.
    pub fn in_memory(&self) -> u64 {
        self.pieces
            .iter()
            .filter(|p| matches!(p, Piece::Text(_)))
            .map(Piece::len)
            .sum()
    }
}

/// Which files open read-only from disk rather than loaded: Settings ›
/// Files, `large_file_read_only`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LargeFiles {
    /// Larger than a quarter of the memory free when it opens.
    #[default]
    Auto,
    /// Always loaded.
    Never,
    /// Larger than this many MB.
    Above(u32),
}

/// Files opened with Auto when the memory free isn't known: larger than
/// this.
const AUTO_UNKNOWN: u64 = 2 << 30;

impl LargeFiles {
    /// As the setting is written: "auto", "never", or a size such as
    /// "2 GB" or "500 MB".
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        match text.as_str() {
            "auto" => return Some(Self::Auto),
            "never" => return Some(Self::Never),
            _ => {}
        }
        let digits = text.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == ' ');
        let unit = text[digits.len()..].trim();
        let n: f64 = digits.trim().parse().ok()?;
        let scale = match unit {
            "" | "b" => 1.0,
            "kb" | "k" => 1024.0,
            "mb" | "m" => 1024.0 * 1024.0,
            "gb" | "g" => 1024.0 * 1024.0 * 1024.0,
            _ => return None,
        };
        Some(Self::Above(((n * scale) / (1024.0 * 1024.0)).round() as u32))
    }

    /// Whether a file of `size` bytes opens read-only, with `free` bytes
    /// of memory free if known.
    pub fn read_only(self, size: u64, free: Option<u64>) -> bool {
        match self {
            Self::Never => false,
            Self::Above(mb) => size > (mb as u64) << 20,
            Self::Auto => size > free.map_or(AUTO_UNKNOWN, |f| f / 4),
        }
    }
}

/// What the view does once the window asked for comes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Then {
    /// Stays on the same text: the view moved near the window's edge.
    Stay,
    /// Goes to a line of the file.
    Line(u64, usize),
    /// Goes to a byte of the file, as a scrollbar drag past the counted
    /// lines does.
    Byte(u64),
    /// Puts the top of the view at a byte of the file.
    Top(u64),
    /// Puts the top of the view at a line of the file.
    TopLine(u64),
    /// Selects the match that starts at a byte.
    Match(u64),
}

/// The file's lines, as the background count finds them by block, and the
/// breaks before each place the file's ranges are cut, so a range's lines
/// are known wherever it starts and ends.
#[derive(Clone, Debug, Default)]
pub struct FileLines {
    /// The file's size: its end needs no edge.
    pub size: u64,
    pub blocks: Vec<Option<u64>>,
    /// The line breaks from the start of a block up to a byte of the file,
    /// by that byte: where windows started and ended.
    pub edges: std::collections::BTreeMap<u64, u64>,
}

impl FileLines {
    pub fn new(size: u64) -> Self {
        Self {
            size,
            blocks: vec![None; blocks(size)],
            edges: Default::default(),
        }
    }

    /// Blocks counted from the top without a gap.
    pub fn counted(&self) -> usize {
        self.blocks.iter().take_while(|c| c.is_some()).count()
    }

    pub fn done(&self) -> bool {
        self.counted() == self.blocks.len()
    }

    pub fn progress(&self) -> f32 {
        let done = self.blocks.iter().filter(|c| c.is_some()).count();
        done as f32 / self.blocks.len().max(1) as f32
    }

    /// The line breaks in the file before byte `at`, once counted.
    pub fn before(&self, at: u64) -> Option<u64> {
        if at >= self.size {
            return self.done().then(|| self.blocks.iter().flatten().sum());
        }
        let block = (at / BLOCK) as usize;
        if block > self.counted() {
            return None;
        }
        let whole: u64 = self.blocks[..block.min(self.blocks.len())]
            .iter()
            .flatten()
            .sum();
        match at % BLOCK {
            0 => Some(whole),
            _ => Some(whole + self.edges.get(&at)?),
        }
    }

    /// The breaks in bytes `from..to` of the file.
    fn between(&self, from: u64, to: u64) -> Option<u64> {
        Some(self.before(to)? - self.before(from)?)
    }

    /// Where the file's break number `breaks` is, give or take a block:
    /// the start of the block it's in, or `from` if that's later.
    fn place(&self, from: u64, breaks: u64) -> Option<u64> {
        if self.before(from)? >= breaks {
            return Some(from);
        }
        let mut block = (from / BLOCK) as usize;
        let mut before: u64 = self.blocks[..block].iter().flatten().sum();
        loop {
            let count = (*self.blocks.get(block)?)?;
            if before + count >= breaks {
                return Some((block as u64 * BLOCK).max(from));
            }
            before += count;
            block += 1;
        }
    }
}

/// The line breaks in a piece, once the file's are counted.
fn piece_breaks(piece: &Piece, lines: &FileLines) -> Option<u64> {
    match piece {
        Piece::File { from, to } => lines.between(*from, *to),
        Piece::Text(rope) => Some(rope.len_lines() as u64 - 1),
    }
}

impl FileText {
    /// The line byte `at` of the text is on, once counted that far.
    pub fn line_of(&self, at: u64, lines: &FileLines) -> Option<u64> {
        let mut start = 0;
        let mut line = 0;
        for piece in self.pieces.iter() {
            let stop = start + piece.len();
            if at < stop {
                return Some(
                    line + match piece {
                        Piece::File { from, .. } => lines.between(*from, from + (at - start))?,
                        Piece::Text(rope) => rope.byte_to_line((at - start) as usize) as u64,
                    },
                );
            }
            line += piece_breaks(piece, lines)?;
            start = stop;
        }
        Some(line)
    }

    /// About where line `line` of the text is, to read a window around:
    /// exactly in text in memory, a block or so before in the file.
    pub fn place_of_line(&self, line: u64, lines: &FileLines) -> Option<u64> {
        let mut start = 0;
        let mut seen = 0;
        for piece in self.pieces.iter() {
            let breaks = piece_breaks(piece, lines)?;
            if seen + breaks >= line {
                return Some(match piece {
                    Piece::Text(rope) => start + rope.line_to_byte((line - seen) as usize) as u64,
                    Piece::File { from, .. } => {
                        let file = lines.place(*from, lines.before(*from)? + (line - seen))?;
                        start + (file - from)
                    }
                });
            }
            seen += breaks;
            start += piece.len();
        }
        None
    }
}

/// A window left with its undo, to come back to.
pub enum Edited {
    /// A window left with its undo: its first byte in the text, as it was
    /// when left.
    Window {
        start: u64,
        buffer: crate::buffer::Buffer,
    },
    /// An edit made on the pieces, such as deleting a selection that
    /// reaches past the window: the text before and after, where it was,
    /// and whether what's saved has it.
    Pieces {
        at: u64,
        before: FileText,
        after: FileText,
        saved: bool,
    },
}

impl Edited {
    /// Whether the text differs from what's saved because of it, where it
    /// is: on the undo stack, or with `undone`, the redo stack.
    pub fn unsaved(&self, undone: bool) -> bool {
        match self {
            Edited::Window { buffer, .. } => buffer.is_dirty(),
            Edited::Pieces { saved, .. } => *saved == undone,
        }
    }
}

/// A file too large to load, as an open document keeps it.
pub struct Large {
    /// The file as opened, which the line count reads.
    pub file: FileText,
    /// The text: pieces of the file, and of text in memory, with the
    /// window's edits in it.
    pub text: FileText,
    /// Its bytes in all.
    pub size: u64,
    /// Bumped with each change to the text, so a window read from an older
    /// one is read again.
    pub generation: u64,
    /// The window held, in bytes of the text, and its first line if known.
    pub start: u64,
    pub end: u64,
    pub first_line: Option<u64>,
    /// The window's version last put in the text.
    pub synced: u64,
    /// The file's lines, counted.
    pub lines: FileLines,
    /// The count's job, while it's under way.
    pub lines_job: Option<u64>,
    /// A window asked for and not yet come, and what to do when it does.
    pub asked: Option<(u64, Then)>,
    /// The view's top byte the window was last read for, so a line longer
    /// than a window isn't asked for again and again.
    pub settled: Option<u64>,
    /// Where to go once the window there is read, or for a line, once
    /// the lines up to it are counted.
    pub pending: Option<Then>,
    /// A search past the window to ask for: from which byte, and forward.
    pub seek: Option<(u64, bool)>,
    /// A search past the window is under way.
    pub seeking: bool,
    /// Windows read for the place pending, so far.
    pub tries: u8,
    /// The far end of a selection past the window, as a byte of the text.
    pub anchor: Option<u64>,
    /// Undo for the oldest edits was let go, past the memory budget: said
    /// once.
    pub folded: bool,
    /// The window held is gone, after an edit on the pieces: one is read
    /// around this byte, and until then nothing is edited.
    pub stale: Option<u64>,
    /// Another program saved over the file while it had edits here: asked
    /// once whether to keep them.
    pub replaced: bool,
    /// The file was cut short on disk while it had edits here: the parts
    /// not edited may read wrong, so it's not edited or saved over.
    pub broken: bool,
    /// Memory free when it opened, for what loading it would take.
    pub free: Option<u64>,
    /// Edits go into the text; otherwise it's read-only.
    pub editable: bool,
    /// Windows left with undo to give, the last left last; and those
    /// undone past, to redo, the next to redo last.
    pub undos: Vec<Edited>,
    pub redos: Vec<Edited>,
    /// Edits whose undo was let go: the text is changed, whatever undo
    /// says.
    pub changed: bool,
    /// The window's last change was an undo or redo, which leaves what can
    /// be redone.
    pub undoing: bool,
    /// The bar over it was closed.
    pub bar_hidden: bool,
    /// Loading it to edit was asked for.
    pub loading: bool,
    /// A save under way, and how many bytes it's written.
    pub saving: Option<u64>,
    /// The last save failed: why, for the bar.
    pub save_error: Option<SaveError>,
    /// Where it was opened or last saved, as the document names it: a
    /// Save As elsewhere saves even with nothing changed.
    pub saved_as: Option<PathBuf>,
    /// Bytes the file grew by on disk while open, until seen.
    pub grew: Option<u64>,
    /// The search counted: its matches in each block, once counted.
    pub found: Option<FileFound>,
}

/// A search's matches in a large file: by block of the file as opened,
/// which edits never move, by what's before each place its ranges are cut,
/// and by piece of text in memory. After an edit only the new text is
/// counted again.
#[derive(Clone, Debug)]
pub struct FileFound {
    pub matcher: Matcher,
    pub blocks: Vec<Option<u32>>,
    pub blocks_asked: bool,
    /// By the place in the file: asked for, and counted.
    pub cuts: std::collections::BTreeMap<u64, Option<u32>>,
    /// By piece of the text: `None` not asked for yet, `Some(None)` asked.
    pub texts: Vec<Option<Option<u32>>>,
    /// The jobs counting it.
    pub jobs: Vec<u64>,
}

impl FileFound {
    pub fn new(matcher: Matcher, file: &FileText, text: &FileText) -> Self {
        Self {
            matcher,
            blocks: vec![None; blocks(file.len())],
            blocks_asked: false,
            cuts: Default::default(),
            texts: vec![None; text.pieces().len()],
            jobs: Vec::new(),
        }
    }

    /// The parts not yet asked for: the file's blocks, the places the
    /// text cuts the file's ranges, and the text in memory.
    pub fn wanted(&mut self, file: &FileText, text: &FileText, generation: u64) -> Vec<Part> {
        let mut parts = Vec::new();
        if !self.blocks_asked {
            self.blocks_asked = true;
            parts.extend((0..self.blocks.len()).map(Part::Block));
        }
        for (i, piece) in text.pieces().iter().enumerate() {
            match piece {
                Piece::File { from, to } => {
                    for at in [*from, *to] {
                        if at % BLOCK != 0 && at < file.len() && !self.cuts.contains_key(&at) {
                            self.cuts.insert(at, None);
                            parts.push(Part::Cut(at));
                        }
                    }
                }
                Piece::Text(_) => {
                    if self.texts.get(i).copied().flatten().is_none() {
                        if let Some(slot) = self.texts.get_mut(i) {
                            *slot = Some(None);
                        }
                        parts.push(Part::Text {
                            piece: i,
                            generation,
                        });
                    }
                }
            }
        }
        parts
    }

    /// A part counted.
    pub fn fill(&mut self, part: Part, count: u32, generation: u64) {
        match part {
            Part::Block(b) => {
                if let Some(slot) = self.blocks.get_mut(b) {
                    *slot = Some(count);
                }
            }
            Part::Cut(at) => {
                self.cuts.insert(at, Some(count));
            }
            Part::Text {
                piece,
                generation: g,
            } if g == generation => {
                if let Some(slot) = self.texts.get_mut(piece) {
                    *slot = Some(Some(count));
                }
            }
            Part::Text { .. } => {}
        }
    }

    /// The text was spliced: the pieces kept whole keep their counts.
    pub fn spliced(&mut self, kept: &[Option<usize>]) {
        self.texts = kept
            .iter()
            .map(|k| k.and_then(|i| self.texts.get(i).copied().flatten()))
            .collect();
    }

    /// The matches in the file before byte `at`, in lines that start
    /// before it, once counted.
    fn before(&self, at: u64, size: u64) -> Option<u64> {
        let sum = |n: usize| -> Option<u64> {
            self.blocks[..n.min(self.blocks.len())]
                .iter()
                .map(|c| c.map(u64::from))
                .sum()
        };
        if at == 0 {
            return Some(0);
        }
        if at >= size {
            return sum(self.blocks.len());
        }
        let block = (at / BLOCK) as usize;
        match at % BLOCK {
            0 => sum(block),
            _ => Some(sum(block)? + u64::from((*self.cuts.get(&at)?)?)),
        }
    }

    /// The matches in all of `text`, and whether that's all of them: until
    /// it is, those counted so far.
    pub fn total(&self, file: &FileText, text: &FileText) -> (u64, bool) {
        let mut exact = Some(0u64);
        for (i, piece) in text.pieces().iter().enumerate() {
            let count = match piece {
                Piece::File { from, to } => {
                    (|| Some(self.before(*to, file.len())? - self.before(*from, file.len())?))()
                }
                Piece::Text(_) => self
                    .texts
                    .get(i)
                    .copied()
                    .flatten()
                    .flatten()
                    .map(u64::from),
            };
            exact = exact.zip(count).map(|(a, b)| a + b);
        }
        match exact {
            Some(total) => (total, true),
            None => {
                let blocks: u64 = self.blocks.iter().flatten().map(|&c| c as u64).sum();
                let texts: u64 = self
                    .texts
                    .iter()
                    .flatten()
                    .flatten()
                    .map(|&c| c as u64)
                    .sum();
                (blocks + texts, false)
            }
        }
    }

    /// How far the count has got, from 0 to 1.
    pub fn progress(&self) -> f32 {
        let all = self.blocks.len() + self.cuts.len() + self.texts.len();
        let done = self.blocks.iter().filter(|c| c.is_some()).count()
            + self.cuts.values().filter(|c| c.is_some()).count()
            + self
                .texts
                .iter()
                .filter(|c| matches!(c, Some(Some(_))))
                .count();
        done as f32 / all.max(1) as f32
    }

    /// For each block of `text`, `Some(0)` when it's known to have no
    /// match: in the file, in blocks counted none. A search past the
    /// window skips those without reading them.
    pub fn empty_blocks(&self, file: &FileText, text: &FileText) -> Vec<Option<u32>> {
        let n = blocks(text.len());
        let mut out = vec![None; n];
        let mut start = 0;
        for piece in text.pieces() {
            let stop = start + piece.len();
            if let Piece::File { from, .. } = piece {
                // Text blocks wholly in this range, with no match in the
                // file's blocks under them, or either side.
                let first = start.div_ceil(BLOCK);
                let mut k = first;
                while (k + 1) * BLOCK <= stop {
                    let (a, b) = (from + (k * BLOCK - start), from + ((k + 1) * BLOCK - start));
                    let (fa, fb) = ((a / BLOCK).saturating_sub(1), (b / BLOCK + 1) as usize);
                    let none =
                        (fa as usize..fb.min(self.blocks.len())).all(|j| self.blocks[j] == Some(0));
                    if none && let Some(slot) = out.get_mut(k as usize) {
                        *slot = Some(0);
                    }
                    k += 1;
                }
                let _ = file;
            }
            start = stop;
        }
        out
    }
}

impl Large {
    pub fn new(file: FileText, free: Option<u64>, editable: bool) -> Self {
        let size = file.len();
        Self {
            text: file.clone(),
            lines: FileLines::new(size),
            file,
            size,
            generation: 0,
            start: 0,
            end: 0,
            first_line: None,
            synced: 0,
            lines_job: None,
            asked: None,
            settled: None,
            pending: None,
            seek: None,
            seeking: false,
            tries: 0,
            anchor: None,
            folded: false,
            stale: None,
            replaced: false,
            broken: false,
            free,
            editable,
            undos: Vec::new(),
            redos: Vec::new(),
            changed: false,
            undoing: false,
            bar_hidden: false,
            loading: false,
            saving: None,
            save_error: None,
            saved_as: None,
            grew: None,
            found: None,
        }
    }

    /// Whether a window has come yet.
    pub fn has_window(&self) -> bool {
        self.end > self.start || self.size == 0
    }

    /// Whether the text is the file as opened: nothing in it edited.
    pub fn untouched(&self) -> bool {
        matches!(self.text.pieces(), [Piece::File { from: 0, to }] if *to == self.file.len())
            || self.text.pieces().is_empty()
    }

    /// Whether every line is counted.
    pub fn lines_done(&self) -> bool {
        self.lines.done()
    }

    /// How far the line count has got.
    pub fn lines_progress(&self) -> f32 {
        self.lines.progress()
    }

    /// The text's lines, once counted.
    pub fn total_lines(&self) -> Option<u64> {
        Some(self.text.line_of(self.size, &self.lines)? + 1)
    }

    /// The window's first line, worked out again: after it's read, after
    /// an edit before it, and as the count comes in.
    pub fn renumber(&mut self) {
        // Its start in a range of the file needs that place counted too.
        if let Some(at) = self.text.file_pos(self.start)
            && at % BLOCK != 0
            && at < self.file.len()
            && !self.lines.edges.contains_key(&at)
            && let Ok(breaks) = self.file.file_breaks(at)
        {
            self.lines.edges.insert(at, breaks);
        }
        self.first_line = self.text.line_of(self.start, &self.lines);
    }

    /// Counts the line breaks before each place the text cuts the file's
    /// ranges that isn't counted yet, from at most a block of the file
    /// each: what numbering the lines past it needs. Most changes make
    /// none new.
    pub fn count_cuts(&mut self) {
        let mut cuts = Vec::new();
        for piece in self.text.pieces() {
            if let Piece::File { from, to } = piece {
                cuts.extend([*from, *to]);
            }
        }
        for cut in cuts {
            if cut % BLOCK != 0
                && cut < self.file.len()
                && !self.lines.edges.contains_key(&cut)
                && let Ok(breaks) = self.file.file_breaks(cut)
            {
                self.lines.edges.insert(cut, breaks);
            }
        }
    }

    /// Where to read a window from to show the text around `at`.
    pub fn from(&self, at: u64) -> From {
        From {
            around: at,
            generation: self.generation,
        }
    }

    /// About where line `line` of the text is, once the count has got
    /// that far.
    pub fn place_of_line(&self, line: u64) -> Option<u64> {
        self.text.place_of_line(line, &self.lines)
    }

    /// About how many bytes a line takes, from what's been counted or
    /// read.
    pub fn bytes_per_line(&self, window_lines: usize) -> f64 {
        let counted = self.lines.counted();
        let lines: u64 = self.lines.blocks[..counted].iter().flatten().sum();
        if counted > 0 && lines > 0 {
            return (counted as u64 * BLOCK).min(self.file.len()) as f64 / lines as f64;
        }
        if window_lines > 1 && self.end > self.start {
            return (self.end - self.start) as f64 / window_lines as f64;
        }
        80.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> (tempfile::TempDir, FileText) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        std::fs::write(&path, text).unwrap();
        let text = FileText::open(&path).unwrap();
        (dir, text)
    }

    #[test]
    fn the_setting_reads_sizes() {
        assert_eq!(LargeFiles::parse("auto"), Some(LargeFiles::Auto));
        assert_eq!(LargeFiles::parse("Never"), Some(LargeFiles::Never));
        assert_eq!(LargeFiles::parse("2 GB"), Some(LargeFiles::Above(2048)));
        assert_eq!(LargeFiles::parse("500mb"), Some(LargeFiles::Above(500)));
        assert_eq!(LargeFiles::parse("lots"), None);
        assert!(LargeFiles::Above(100).read_only(200 << 20, None));
        assert!(!LargeFiles::Never.read_only(u64::MAX, Some(0)));
        // Auto: a quarter of what's free.
        assert!(LargeFiles::Auto.read_only(5 << 30, Some(16 << 30)));
        assert!(!LargeFiles::Auto.read_only(3 << 30, Some(16 << 30)));
    }

    #[test]
    fn breaks_are_the_ropes() {
        for text in [
            "a\nb\r\nc\rd\u{b}e\u{c}f\u{85}g\u{2028}h\u{2029}i",
            "\r\r\n\n\r",
            "no breaks",
            "ünïcode\nlines\n",
        ] {
            let rope = Rope::from_str(text);
            assert_eq!(
                breaks(text.as_bytes(), None),
                rope.len_lines() as u64 - 1,
                "{text:?}"
            );
        }
        // A `\r` at the end of a piece, with the `\n` in the next.
        assert_eq!(breaks(b"a\r", Some(b'\n')), 0);
        assert_eq!(breaks(b"\nb", None), 1);
    }

    fn numbered(lines: usize) -> String {
        (0..lines).map(|n| format!("line {n}\n")).collect()
    }

    /// The file's lines, all counted.
    fn counted(file: &FileText) -> FileLines {
        let mut lines = FileLines::new(file.len());
        for b in 0..lines.blocks.len() {
            lines.blocks[b] = Some(count_lines(file, b).unwrap());
        }
        lines
    }

    fn at(around: u64) -> From {
        From {
            around,
            generation: 0,
        }
    }

    #[test]
    fn a_window_is_whole_lines_with_its_first_line() {
        let text = numbered(1_000_000);
        let (_dir, file) = file(&text);
        let w = read_window(&file, at(text.len() as u64 / 2)).unwrap();
        assert!(w.start > 0 && w.end < text.len() as u64);
        assert_eq!(&text[w.start as usize - 1..w.start as usize], "\n");
        assert_eq!(w.text.to_string(), text[w.start as usize..w.end as usize]);
        // Its first line, from the count and the breaks before its start.
        let mut lines = FileLines::new(file.len());
        assert_eq!(file.line_of(w.start, &lines), None, "not counted yet");
        lines = FileLines {
            edges: lines.edges,
            ..counted(&file)
        };
        lines.edges.extend(w.edges.iter().copied());
        let first = file.line_of(w.start, &lines).unwrap();
        let line = w.text.line(0).to_string();
        assert_eq!(line, format!("line {first}\n"));
    }

    #[test]
    fn the_counts_add_up_to_the_ropes_lines() {
        let text = numbered(1_200_000);
        let (_dir, file) = file(&text);
        let size = text.len() as u64;
        let total: u64 = (0..blocks(size))
            .map(|p| count_lines(&file, p).unwrap())
            .sum();
        assert_eq!(total, 1_200_000);
        let matcher = Matcher::exact("line 9");
        let count: u32 = (0..blocks(size))
            .map(|p| count_matches(&file, &matcher, p).unwrap())
            .sum();
        let expected = text.matches("line 9").count() as u32;
        assert_eq!(count, expected);
    }

    #[test]
    fn seeking_finds_the_next_and_last_match() {
        let mut text = numbered(1_000_000);
        text.push_str("needle here\n");
        let (_dir, file) = file(&text);
        let at = text.find("needle").unwrap() as u64;
        let matcher = Matcher::exact("needle");
        assert_eq!(seek(&file, &matcher, 0, true, &[]).unwrap(), Some(at));
        assert_eq!(seek(&file, &matcher, at + 1, true, &[]).unwrap(), None);
        let size = text.len() as u64;
        assert_eq!(seek(&file, &matcher, size, false, &[]).unwrap(), Some(at));
        assert_eq!(seek(&file, &matcher, at, false, &[]).unwrap(), None);
        // A regular expression goes a line at a time.
        let re = Matcher::regex(regex::Regex::new("^line 99999[0-9]$").unwrap());
        let hit = seek(&file, &re, 0, true, &[]).unwrap().unwrap();
        assert_eq!(&text[hit as usize..hit as usize + 11], "line 999990");
    }

    #[test]
    fn a_line_finds_its_place() {
        let text = numbered(2_000_000);
        let (_dir, file) = file(&text);
        let lines = counted(&file);
        assert_eq!(file.line_of(file.len(), &lines), Some(2_000_000));
        let place = file.place_of_line(1_500_000, &lines).unwrap();
        let line_at = text.find("line 1500000\n").unwrap() as u64;
        assert!(
            place <= line_at && line_at - place < BLOCK,
            "{place} {line_at}"
        );
    }

    #[test]
    fn text_between_two_parts_of_the_file_reads_as_one() {
        let text = numbered(1_000_000);
        let (_dir, file) = file(&text);
        // "line 10\n" goes, and two lines come in its place; the cut at
        // 500 000 splits a line in two pieces.
        let ten = text.find("line 10\n").unwrap() as u64;
        let mid = text.find("line 500000\n").unwrap() as u64 + 3;
        let typed = "typed one\ntyped two\n";
        let edited = file.with_pieces(vec![
            Piece::File { from: 0, to: ten },
            Piece::Text(Rope::from_str(typed)),
            Piece::File {
                from: ten + 8,
                to: mid,
            },
            Piece::File {
                from: mid,
                to: text.len() as u64,
            },
        ]);
        let mut expected = text.clone();
        expected.replace_range(ten as usize..ten as usize + 8, typed);
        assert_eq!(edited.len(), expected.len() as u64);
        let all = edited.read(0, edited.len()).unwrap();
        assert!(all == expected.as_bytes(), "the pieces read as the text");
        // Lines count as the text's, block by block.
        let total: u64 = (0..blocks(edited.len()))
            .map(|b| count_lines(&edited, b).unwrap())
            .sum();
        assert_eq!(total, 1_000_001);
        // A window around the typed text starts on a line, numbered.
        let w = read_window(&edited, at(ten)).unwrap();
        let mut lines = counted(&file);
        lines.edges.extend(w.edges.iter().copied());
        // The cuts the pieces make, as windows' ends would give.
        for cut in [ten, ten + 8, mid] {
            let breaks = file.file_breaks(cut).unwrap();
            lines.edges.insert(cut, breaks);
        }
        let first = edited.line_of(w.start, &lines).unwrap() as usize;
        assert_eq!(edited.line_of(edited.len(), &lines), Some(1_000_001));
        let typed_line = (first..)
            .zip(w.text.lines())
            .find(|(_, l)| *l == "typed two\n");
        assert_eq!(typed_line.map(|(n, _)| n), Some(11));
        // Search finds the typed text, and a line cut across two pieces.
        let found = seek(&edited, &Matcher::exact("typed two"), 0, true, &[]).unwrap();
        assert_eq!(found, Some(ten + 10));
        let found = seek(&edited, &Matcher::exact("line 500000"), 0, true, &[]).unwrap();
        assert_eq!(found, Some(expected.find("line 500000").unwrap() as u64));
        let count: u32 = (0..blocks(edited.len()))
            .map(|b| count_matches(&edited, &Matcher::exact("typed"), b).unwrap())
            .sum();
        assert_eq!(count, 2);
    }

    #[test]
    fn a_save_with_no_room_or_cancelled_leaves_the_file() {
        use std::sync::atomic::AtomicBool;
        let text = numbered(1_000_000);
        let (dir, file) = file(&text);
        let path = dir.path().join("big.log");
        let edited = file.splice(0, 0, Rope::from_str("new\n"));
        let go = AtomicBool::new(false);
        let room = save(&edited, &path, Some(1000), &|_| {}, &go);
        assert_eq!(
            room,
            Err(SaveError::NoRoom {
                need: edited.len(),
                free: 1000
            })
        );
        let stop = AtomicBool::new(true);
        assert_eq!(
            save(&edited, &path, None, &|_| {}, &stop),
            Err(SaveError::Cancelled)
        );
        assert!(std::fs::read_to_string(&path).unwrap() == text);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "nothing left over"
        );
        // Saved: the counts are the new file's.
        let counts = save(&edited, &path, None, &|_| {}, &go).unwrap();
        assert_eq!(counts.iter().sum::<u64>(), 1_000_001);
        assert!(std::fs::read_to_string(&path).unwrap() == format!("new\n{text}"));
    }

    #[test]
    fn a_journal_reads_back_as_written() {
        let text = numbered(1000);
        let (dir, file) = file(&text);
        let edited = file.splice(10, 20, Rope::from_str("typed\ntext with a newline\n"));
        let journal = Journal {
            path: "logs/big.log".into(),
            identity: edited.identity().unwrap(),
            pieces: edited.pieces().to_vec(),
        };
        let at = dir.path().join("journal");
        journal.write(&at).unwrap();
        assert_eq!(Journal::read(&at).unwrap(), journal);
        assert!(edited.same_file());
    }

    #[test]
    fn a_window_folds_to_its_changes() {
        let text = numbered(100_000);
        let (_dir, file) = file(&text);
        let (from, to) = (1000u64, 1_000_000u64);
        let origin = file.with_pieces(file.pieces_in(from, to));
        let mut window = Rope::from_str(&text[from as usize..to as usize]);
        // Two lines changed, far apart.
        let first = window.line_to_char(5);
        window.insert(first, "typed ");
        let later = window.line_to_char(40_000);
        window.remove(later..window.line_to_char(40_001));
        let folded = fold(&origin, &window).unwrap();
        let texts: u64 = folded
            .iter()
            .filter(|p| matches!(p, Piece::Text(_)))
            .map(Piece::len)
            .sum();
        assert!(texts < 100, "only the changed line kept: {texts} bytes");
        let back = file.with_pieces(folded);
        assert!(back.read(0, back.len()).unwrap() == window.to_string().as_bytes());
    }

    #[test]
    fn the_text_folds_to_its_changes_and_reads_the_same() {
        let text = numbered(1_000_000);
        let (_dir, file) = file(&text);
        // Two windows edited, as whole windows put in the pieces.
        let mut edited = file.clone();
        for (at, typed) in [(100_000u64, "one "), (5_000_000, "two ")] {
            let end = at + 2_000_000;
            let mut window = Rope::from_str(&text[at as usize..end as usize]);
            window.insert(window.line_to_char(3), typed);
            edited = edited.splice(at, end, window);
        }
        assert!(edited.in_memory() > 3_000_000);
        let folded = edited.folded();
        assert!(folded.in_memory() < 100, "{}", folded.in_memory());
        assert!(folded.read(0, folded.len()).unwrap() == edited.read(0, edited.len()).unwrap());
    }
}
