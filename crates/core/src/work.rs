//! Whole-file work done off the window's thread: a large file's search
//! count, and its rows when wrapped. The core stays free of threads: it
//! asks for a [`Job`] with an effect, and the host runs its pieces, on a
//! few threads, in any order, and sends each piece's [`Done`] back as an
//! event. A job for text or a query since changed is answered for
//! nothing, as its id is no longer wanted.

use std::path::PathBuf;

use ropey::Rope;

use crate::search::{self, Counted, Matcher};

/// Files larger than this have their whole-file work done in the
/// background; smaller ones, at once, as it takes well under a frame.
pub const BACKGROUND_BYTES: usize = 8 << 20;

/// What a job does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Counts a search's matches.
    Count(Matcher),
    /// Wraps every line at a width.
    Wrap(usize),
    /// Counts the lines of a file too large to load, read from disk.
    Lines { path: PathBuf, size: u64 },
    /// Counts a search's matches in a file too large to load.
    FileCount {
        path: PathBuf,
        size: u64,
        matcher: Matcher,
    },
}

impl Kind {
    /// Whether a job of this kind makes `older` not needed: one count or
    /// wrap at a time, but each large file's lines.
    pub fn replaces(&self, older: &Kind) -> bool {
        match (self, older) {
            (Kind::Lines { path, .. }, Kind::Lines { path: was, .. }) => path == was,
            _ => std::mem::discriminant(self) == std::mem::discriminant(older),
        }
    }
}

/// Work on a snapshot of a file's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: u64,
    pub kind: Kind,
    /// Shared with the buffer, not copied.
    pub text: Rope,
}

/// Lines a wrap does in each piece.
const WRAP_PIECE: usize = 65_536;

/// What one piece of a job found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    /// The counts of blocks from `first`.
    Count { first: usize, counts: Vec<Counted> },
    /// Each line's rows from `first`, and those of lines that take too
    /// many for a byte.
    Wrap {
        first: usize,
        rows: Vec<u8>,
        tall: Vec<(usize, usize)>,
    },
    /// A large file's line breaks in a piece; none if it couldn't be read.
    Lines { piece: usize, breaks: Option<u64> },
    /// A search's matches in a piece of a large file.
    FileCount { piece: usize, count: Option<u32> },
}

impl Job {
    /// How many pieces it's done in.
    pub fn pieces(&self) -> usize {
        match &self.kind {
            Kind::Count(_) => search::blocks_in(&self.text).div_ceil(search::PIECE_BLOCKS),
            Kind::Wrap(_) => self.text.len_lines().div_ceil(WRAP_PIECE),
            Kind::Lines { size, .. } | Kind::FileCount { size, .. } => {
                crate::bigfile::pieces(*size)
            }
        }
        .max(1)
    }

    /// Does piece `piece`: on any thread.
    pub fn run(&self, piece: usize) -> Done {
        match &self.kind {
            Kind::Count(matcher) => {
                let blocks = search::blocks_in(&self.text);
                let first = (piece * search::PIECE_BLOCKS).min(blocks);
                let end = (first + search::PIECE_BLOCKS).min(blocks);
                Done::Count {
                    first,
                    counts: search::count_blocks(matcher, &self.text, first..end),
                }
            }
            Kind::Wrap(width) => {
                let lines = self.text.len_lines();
                let first = (piece * WRAP_PIECE).min(lines);
                let end = (first + WRAP_PIECE).min(lines);
                let (rows, tall) = crate::rows::wrap_lines(&self.text, *width, first..end);
                Done::Wrap { first, rows, tall }
            }
            Kind::Lines { path, .. } => Done::Lines {
                piece,
                breaks: crate::bigfile::count_lines(path, piece).ok(),
            },
            Kind::FileCount { path, matcher, .. } => Done::FileCount {
                piece,
                count: crate::bigfile::count_matches(path, matcher, piece).ok(),
            },
        }
    }

    /// Does every piece, in order, as a script's session does.
    pub fn run_all(&self) -> Vec<Done> {
        (0..self.pieces()).map(|p| self.run(p)).collect()
    }
}
