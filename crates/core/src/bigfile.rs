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
    /// The line it starts on, when the lines before it are counted.
    pub first_line: Option<u64>,
    pub text: Rope,
}

/// What a window is read from: about where, and a line start at or
/// before it with its line, if known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct From {
    /// The window's middle is wanted about here.
    pub around: u64,
    /// A line start at or before the window's start, and its line: the
    /// lines between are counted as it's read.
    pub anchor: Option<(u64, u64)>,
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
    let first_line = match from.anchor {
        Some((at, line)) if at <= start => Some(line + breaks_in(text, at, start)?),
        _ => None,
    };
    let bytes = text.read(start, end - start)?;
    Ok(Window {
        start,
        end: start + bytes.len() as u64,
        size,
        first_line,
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

/// A file too large to load, as an open document keeps it.
#[derive(Clone, Debug)]
pub struct Large {
    /// The text: pieces of the file, and of text in memory.
    pub text: FileText,
    /// Its bytes in all.
    pub size: u64,
    /// The window held, in bytes of the text, and its first line if known.
    pub start: u64,
    pub end: u64,
    pub first_line: Option<u64>,
    /// Line breaks in each block of the text, once counted.
    pub lines: Vec<Option<u64>>,
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
    /// Memory free when it opened, for what loading it would take.
    pub free: Option<u64>,
    /// The bar over it was closed.
    pub bar_hidden: bool,
    /// Loading it to edit was asked for.
    pub loading: bool,
    /// Bytes the file grew by on disk while open, until seen.
    pub grew: Option<u64>,
    /// The search counted: its matches in each block, once counted.
    pub found: Option<FileFound>,
}

/// A search's matches in a large file, counted by block.
#[derive(Clone, Debug)]
pub struct FileFound {
    pub matcher: Matcher,
    pub counts: Vec<Option<u32>>,
    pub job: Option<u64>,
}

impl FileFound {
    pub fn total(&self) -> u64 {
        self.counts.iter().flatten().map(|&c| c as u64).sum()
    }

    pub fn complete(&self) -> bool {
        self.counts.iter().all(Option::is_some)
    }

    pub fn progress(&self) -> f32 {
        let done = self.counts.iter().filter(|c| c.is_some()).count();
        done as f32 / self.counts.len().max(1) as f32
    }
}

impl Large {
    pub fn new(text: FileText, free: Option<u64>) -> Self {
        let size = text.len();
        Self {
            text,
            size,
            start: 0,
            end: 0,
            first_line: None,
            lines: vec![None; blocks(size)],
            lines_job: None,
            asked: None,
            settled: None,
            pending: None,
            seek: None,
            seeking: false,
            free,
            bar_hidden: false,
            loading: false,
            grew: None,
            found: None,
        }
    }

    /// Whether a window has come yet.
    pub fn has_window(&self) -> bool {
        self.end > self.start || self.size == 0
    }

    /// Blocks counted from the top without a gap.
    fn counted(&self) -> usize {
        self.lines.iter().take_while(|c| c.is_some()).count()
    }

    /// Whether every line is counted.
    pub fn lines_done(&self) -> bool {
        self.counted() == self.lines.len()
    }

    /// How far the line count has got.
    pub fn lines_progress(&self) -> f32 {
        let done = self.lines.iter().filter(|c| c.is_some()).count();
        done as f32 / self.lines.len().max(1) as f32
    }

    /// The file's lines, once counted.
    pub fn total_lines(&self) -> Option<u64> {
        self.lines_done()
            .then(|| self.lines.iter().flatten().sum::<u64>() + 1)
    }

    /// The last block start at or before `at` whose line is counted, and
    /// that line.
    pub fn anchor(&self, at: u64) -> Option<(u64, u64)> {
        // Only from a counted block: counting from further back would
        // read what the count hasn't reached.
        let block = (at / BLOCK) as usize;
        (block <= self.counted()).then_some(())?;
        let line: u64 = self.lines[..block].iter().flatten().sum();
        Some((block as u64 * BLOCK, line))
    }

    /// Where to read a window from to show the file around `at`.
    pub fn from(&self, at: u64) -> From {
        From {
            around: at,
            anchor: self.anchor(window_start(at, self.size)),
        }
    }

    /// The block line `line` starts in, and the line breaks before that
    /// block, once the count has got that far.
    pub fn block_of_line(&self, line: u64) -> Option<(u64, u64)> {
        let mut before = 0;
        for (i, count) in self.lines.iter().enumerate() {
            let count = (*count)?;
            if before + count >= line {
                return Some((i as u64 * BLOCK, before));
            }
            before += count;
        }
        None
    }

    /// About how many bytes a line takes, from what's been counted or
    /// read.
    pub fn bytes_per_line(&self, window_lines: usize) -> f64 {
        let counted = self.counted();
        let lines: u64 = self.lines[..counted].iter().flatten().sum();
        if counted > 0 && lines > 0 {
            return (counted as u64 * BLOCK).min(self.size) as f64 / lines as f64;
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

    #[test]
    fn a_window_is_whole_lines_with_its_first_line() {
        let text = numbered(1_000_000);
        let (_dir, file) = file(&text);
        let w = read_window(
            &file,
            From {
                around: text.len() as u64 / 2,
                anchor: Some((0, 0)),
            },
        )
        .unwrap();
        assert!(w.start > 0 && w.end < text.len() as u64);
        assert_eq!(&text[w.start as usize - 1..w.start as usize], "\n");
        assert_eq!(w.text.to_string(), text[w.start as usize..w.end as usize]);
        let first = w.first_line.unwrap();
        let line = w.text.line(0).to_string();
        assert_eq!(line, format!("line {first}\n"));
        // Without an anchor, the line isn't known.
        let w = read_window(
            &file,
            From {
                around: 0,
                anchor: None,
            },
        )
        .unwrap();
        assert_eq!((w.start, w.first_line), (0, None));
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
    fn a_line_finds_its_block() {
        let text = numbered(2_000_000);
        let (_dir, file) = file(&text);
        let mut large = Large::new(file.clone(), None);
        for p in 0..large.lines.len() {
            large.lines[p] = Some(count_lines(&file, p).unwrap());
        }
        assert!(large.lines_done());
        assert_eq!(large.total_lines(), Some(2_000_001));
        let (at, line) = large.block_of_line(1_500_000).unwrap();
        assert!(line <= 1_500_000);
        let w = read_window(&file, large.from(at)).unwrap();
        let first = w.first_line.unwrap();
        assert!(first <= 1_500_000);
        let local = (1_500_000 - first) as usize;
        if local < w.text.len_lines() {
            assert_eq!(w.text.line(local).to_string(), "line 1500000\n");
        }
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
        let w = read_window(
            &edited,
            From {
                around: ten,
                anchor: Some((0, 0)),
            },
        )
        .unwrap();
        let first = w.first_line.unwrap() as usize;
        let typed_line = (first..)
            .zip(w.text.lines())
            .find(|(_, l)| *l == "typed two\n");
        assert_eq!(typed_line.map(|(n, _)| n), Some(11));
        // Search finds the typed text, and a line cut across two pieces.
        let at = seek(&edited, &Matcher::exact("typed two"), 0, true, &[]).unwrap();
        assert_eq!(at, Some(ten + 10));
        let at = seek(&edited, &Matcher::exact("line 500000"), 0, true, &[]).unwrap();
        assert_eq!(at, Some(expected.find("line 500000").unwrap() as u64));
        let count: u32 = (0..blocks(edited.len()))
            .map(|b| count_matches(&edited, &Matcher::exact("typed"), b).unwrap())
            .sum();
        assert_eq!(count, 2);
    }
}
