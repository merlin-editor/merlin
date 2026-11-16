//! C and C++, as a language plugin: the languages, their comments, base
//! colours from a fast lexer, and `clangd` language server integration
//! tuned for low CPU and memory usage.

pub mod lex;

use merlin_lsp::{Spec, WithLsp};
use merlin_plugin::prelude::*;
use merlin_plugin::shared::{CompileCommands, Shared};
use merlin_plugin::text::utf16_column;

use lex::Kind;

/// clangd configured for low memory and CPU overhead: precompiled headers on
/// disk instead of in RAM, low-priority background indexing, and glibc heap
/// trimming after requests.
const CPP_SERVER: Spec = Spec {
    program: "clangd",
    args: &[
        "--pch-storage=disk",
        "--background-index-priority=low",
        "--malloc-trim",
        "-j=2",
    ],
    files: &[
        ("c", "c"),
        ("h", "cpp"),
        ("cc", "cpp"),
        ("cpp", "cpp"),
        ("cxx", "cpp"),
        ("c++", "cpp"),
        ("hpp", "cpp"),
        ("hh", "cpp"),
        ("hxx", "cpp"),
        ("h++", "cpp"),
        ("ipp", "cpp"),
        ("inl", "cpp"),
        ("tpp", "cpp"),
    ],
    settings: None,
    options: &[],
    flags: &[
        ("query-driver", "--query-driver="),
        ("fallback-style", "--fallback-style="),
    ],
    snippets: false,
    // The manifest's [programs] says how to install it, per system.
    install: None,
    restarts: &["compile-commands", "include-folders", "defines"],
    shared: &[CompileCommands::KEY],
    adjust: Some(adjust),
};

/// clangd's start-up options from the plugin's settings and what build
/// plugins share.
fn adjust(options: &mut serde_json::Value) {
    if !options.is_object() {
        *options = serde_json::Value::Object(Default::default());
    }
    let root = host::workspace_root();
    if let Some(dir) = compile_commands_dir() {
        let dir = std::path::Path::new(&root).join(dir.trim());
        options["compilationDatabasePath"] = dir.to_string_lossy().into_owned().into();
    }
    let flags = fallback_flags(&root, &list("include-folders"), &list("defines"));
    if !flags.is_empty() {
        options["fallbackFlags"] = flags.into();
    }
}

/// Where `compile_commands.json` is: the setting, else what a build
/// plugin shares, else nothing, and clangd looks for itself.
fn compile_commands_dir() -> Option<String> {
    settings::text("compile-commands")
        .filter(|d| !d.trim().is_empty())
        .or_else(|| shared::get::<CompileCommands>().map(|c| c.dir))
}

/// A list setting's items, trimmed, without empty ones.
fn list(key: &str) -> Vec<String> {
    let Some(merlin_plugin::Setting::Other(text)) = host::get_setting(key) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&text)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Flags for files no compile command covers: include folders, relative
/// to the workspace or absolute, and defines, as `NAME` or `NAME=value`.
fn fallback_flags(root: &str, folders: &[String], defines: &[String]) -> Vec<String> {
    let folders = folders.iter().map(|f| {
        let path = std::path::Path::new(root).join(f);
        format!("-I{}", path.display())
    });
    let defines = defines
        .iter()
        .map(|d| format!("-D{}", d.trim_start_matches("-D")));
    folders.chain(defines).collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_flags_hold_include_folders_and_defines() {
        let flags = fallback_flags(
            "/work",
            &["include".into(), "/opt/sdk/include".into()],
            &["DEBUG".into(), "-DLEVEL=2".into()],
        );
        assert_eq!(
            flags,
            [
                "-I/work/include",
                "-I/opt/sdk/include",
                "-DDEBUG",
                "-DLEVEL=2"
            ]
        );
    }
}
