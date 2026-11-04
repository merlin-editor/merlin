//! What plugins tell the editor about a document's language, as VS Code's
//! language extensions do: colours (a base layer in place of the built-in
//! grammar, as a TextMate grammar is, and a semantic layer over it, as
//! semantic tokens are), the symbols in it, and where its blocks fold.
//!
//! Everything is kept by character position, so it moves with edits until
//! the plugin sends something new.

use std::ops::Range;

use merlin_syntax::Syntax;

use crate::buffer::Change;

/// A coloured run of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: Syntax,
}

/// Which colours a plugin sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Every colour in the file, in place of the built-in grammar's.
    Base,
    /// Colours over the base, from knowing what names mean.
    Semantic,
}

/// A symbol a plugin found, such as a function, with where it starts and
/// ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocSymbol {
    pub name: String,
    /// What it is, such as `function` or `class`.
    pub kind: String,
    /// Shown dimmed after the name, such as a signature.
    pub detail: String,
    pub start: usize,
    pub end: usize,
    /// How many symbols it's inside.
    pub depth: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Layers {
    /// A plugin's base colours, once it has sent any.
    pub base: Option<Vec<Token>>,
    pub semantic: Vec<Token>,
    pub symbols: Option<Vec<DocSymbol>>,
    /// Blocks that fold: where their first and last lines start.
    pub folds: Option<Vec<(usize, usize)>>,
    /// Goes up each time the colours change, for what's kept of them.
    pub colours: u64,
}

impl Layers {
    pub fn map(&mut self, change: &Change) {
        let tokens = self.base.iter_mut().flatten().chain(&mut self.semantic);
        for t in tokens {
            t.start = change.map(t.start);
            t.end = change.map(t.end).max(t.start);
        }
        for s in self.symbols.iter_mut().flatten() {
            s.start = change.map(s.start);
            s.end = change.map(s.end).max(s.start);
        }
        for f in self.folds.iter_mut().flatten() {
            *f = (change.map(f.0), change.map(f.1));
        }
    }

    /// Drops what edits emptied.
    pub fn tidy(&mut self) {
        for tokens in self.base.iter_mut().chain([&mut self.semantic]) {
            tokens.retain(|t| t.start < t.end);
        }
    }

    /// Replaces a layer's tokens within `within`, or all of them.
    pub fn set(&mut self, layer: Layer, within: Option<Range<usize>>, mut tokens: Vec<Token>) {
        self.colours += 1;
        let list = match layer {
            Layer::Base => self.base.get_or_insert_with(Vec::new),
            Layer::Semantic => &mut self.semantic,
        };
        match within {
            Some(r) => {
                list.retain(|t| t.end <= r.start || t.start >= r.end);
                list.extend(tokens);
            }
            None => {
                *list = std::mem::take(&mut tokens);
            }
        }
        list.sort_by_key(|t| t.start);
    }

    /// The tokens of a layer that touch `range`, in order.
    fn touching(list: &[Token], range: &Range<usize>) -> impl Iterator<Item = Token> {
        let from = list.partition_point(|t| t.start < range.start.saturating_sub(4096));
        list[from..]
            .iter()
            .take_while(move |t| t.start < range.end)
            .filter(move |t| t.end > range.start)
            .copied()
    }

    /// Colours over `range` from plugins: the base layer, then the semantic
    /// one, which wins where they overlap.
    pub fn spans(&self, range: &Range<usize>) -> Vec<(usize, usize, Syntax)> {
        let base = self.base.iter().flat_map(|b| Self::touching(b, range));
        base.chain(Self::touching(&self.semantic, range))
            .map(|t| (t.start, t.end, t.kind))
            .collect()
    }
}
