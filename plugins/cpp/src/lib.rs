//! C and C++, as a language plugin: the languages, their comments, base
//! colours from a fast lexer, and `clangd` language server integration
//! tuned for low CPU and memory usage.

pub mod lex;

use merlin_lsp::{CPP_SERVER, WithLsp};
use merlin_plugin::prelude::*;
use merlin_plugin::text::utf16_column;

use lex::Kind;

const EXTENSIONS: &[&str] = &[
    "c", "h", "cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "inl", "tpp",
];

const MAX_BYTES: usize = 512 * 1024;

#[derive(Default)]
struct Cpp {
    last: Option<(String, Option<u64>)>,
}

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
    lex::lex(text)
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
        .collect()
}

impl Plugin for Cpp {
    fn colour_text(&mut self, _language: &str, text: &str) -> Vec<Token> {
        if text.len() > MAX_BYTES {
            return Vec::new();
        }
        tokens(text)
    }

    fn buffer(&mut self, event: BufferEvent) {
        if matches!(event, BufferEvent::Saved(_)) {
            return;
        }
        let Some(path) = active() else {
            return;
        };
        let version = host::active_version().ok();
        if let Some((last_path, last_version)) = &self.last
            && *last_path == path
            && version.is_some()
            && *last_version == version
        {
            return;
        }
        let Ok(text) = host::active_text() else {
            return;
        };
        if text.len() > MAX_BYTES {
            self.last = None;
            return;
        }
        colour(&path, &text);
        self.last = Some((path, version));
    }
}

merlin_plugin::export!(WithLsp<Cpp> => WithLsp::new(
    Cpp::default(),
    "C/C++",
    &[CPP_SERVER],
));
