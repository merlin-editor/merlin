//! C and C++, as a language plugin: the languages, their comments, and
//! colours from a lexer, which stand in for the editor's own grammar as a
//! VS Code grammar does. The `lsp` plugin adds clangd's colours, symbols
//! and folds over them.

pub mod lex;

use merlin_plugin::prelude::*;
use merlin_plugin::text::utf16_column;

use lex::Kind;

const EXTENSIONS: &[&str] = &[
    "c", "h", "cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "inl", "tpp",
];

#[derive(Default)]
struct Cpp;

/// The open file, if it's C or C++.
fn active() -> Option<String> {
    let path = host::active_path().ok()??;
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str()).then_some(path)
}

fn colour(path: &str, text: &str) {
    host::set_tokens(path, TokenLayer::Base, None, &tokens(text));
}

/// The lexer's runs as tokens, with UTF-16 columns.
fn tokens(text: &str) -> Vec<Token> {
    let lines: Vec<&str> = text.split('\n').collect();
    let at = |line: usize, chars: usize| Position {
        line: line as u32,
        column: utf16_column(lines.get(line).copied().unwrap_or_default(), chars),
    };
    let tokens: Vec<Token> = lex::lex(text)
        .into_iter()
        .map(|r| Token {
            start: at(r.line, r.start),
            end: at(r.line, r.end),
            kind: match r.kind {
                Kind::Keyword => TokenKind::Keyword,
                Kind::String => TokenKind::String,
                Kind::Escape => TokenKind::Escape,
                Kind::Comment => TokenKind::Comment,
                Kind::Number => TokenKind::Constant,
                Kind::Macro => TokenKind::Macro,
                Kind::Type => TokenKind::Type,
                Kind::Function => TokenKind::Function,
            },
        })
        .collect();
    tokens
}

impl Plugin for Cpp {
    fn colour_text(&mut self, _language: &str, text: &str) -> Vec<Token> {
        tokens(text)
    }

    fn buffer(&mut self, event: BufferEvent) {
        if matches!(event, BufferEvent::Saved(_)) {
            return;
        }
        if let Some(path) = active()
            && let Ok(text) = host::active_text()
        {
            colour(&path, &text);
        }
    }
}

merlin_plugin::export!(Cpp);
