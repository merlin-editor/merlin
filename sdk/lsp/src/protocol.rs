//! Pure conversions between LSP JSON payloads and Merlin's plugin types.

use std::collections::BTreeMap;

use merlin_plugin::prelude::*;
use serde_json::Value;

/// The semantic token types the plugin colours, as LSP names them.
pub(crate) const TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "concept",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "event",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
    "label",
];

/// The colour of a semantic token type.
fn token_kind(name: &str) -> Option<TokenKind> {
    Some(match name {
        "namespace" | "type" | "class" | "enum" | "interface" | "struct" | "typeParameter"
        | "concept" | "builtinType" | "typeAlias" => TokenKind::Type,
        "parameter" | "variable" | "selfKeyword" => TokenKind::Variable,
        "property" | "label" => TokenKind::Property,
        "enumMember" | "number" | "boolean" => TokenKind::Constant,
        "function" | "method" | "event" => TokenKind::Function,
        "macro" => TokenKind::Macro,
        "keyword" | "modifier" => TokenKind::Keyword,
        "comment" => TokenKind::Comment,
        "string" | "regexp" => TokenKind::String,
        "operator" => TokenKind::Operator,
        "decorator" | "attribute" => TokenKind::Attribute,
        _ => return None,
    })
}

/// Semantic tokens from LSP's relative encoding: five numbers a token,
/// its line and start relative to the token before, its length and type.
pub(crate) fn semantic_tokens(data: &[Value], legend: &[String]) -> Vec<Token> {
    let n = |v: &Value| v.as_u64().unwrap_or(0) as u32;
    let (mut line, mut start) = (0, 0);
    let mut tokens = Vec::new();
    for chunk in data.chunks_exact(5) {
        let delta_line = n(&chunk[0]);
        if delta_line > 0 {
            line += delta_line;
            start = 0;
        }
        start += n(&chunk[1]);
        let kind = legend
            .get(n(&chunk[3]) as usize)
            .and_then(|name| token_kind(name));
        if let Some(kind) = kind {
            tokens.push(Token {
                start: Position {
                    line,
                    column: start,
                },
                end: Position {
                    line,
                    column: start + n(&chunk[2]),
                },
                kind,
            });
        }
    }
    tokens
}

/// LSP's symbol kinds, by number.
const SYMBOL_KINDS: &[&str] = &[
    "file",
    "module",
    "namespace",
    "package",
    "class",
    "method",
    "property",
    "field",
    "constructor",
    "enum",
    "interface",
    "function",
    "variable",
    "constant",
    "string",
    "number",
    "boolean",
    "array",
    "object",
    "key",
    "null",
    "enum-member",
    "struct",
    "event",
    "operator",
    "type-parameter",
];

/// Symbols as a list, nested ones after the one they're in.
pub(crate) fn flatten_symbols(list: &[Value], depth: u32, out: &mut Vec<DocSymbol>) {
    for s in list {
        let kind = s["kind"]
            .as_u64()
            .and_then(|k| SYMBOL_KINDS.get(k.saturating_sub(1) as usize))
            .unwrap_or(&"symbol");
        // A DocumentSymbol has its range; a SymbolInformation a location.
        let range = if s["range"].is_object() {
            &s["range"]
        } else {
            &s["location"]["range"]
        };
        out.push(DocSymbol {
            name: s["name"].as_str().unwrap_or_default().to_string(),
            kind: kind.to_string(),
            detail: s["detail"].as_str().unwrap_or_default().to_string(),
            start: position(&range["start"]),
            end: position(&range["end"]),
            depth,
        });
        if let Some(children) = s["children"].as_array() {
            flatten_symbols(children, depth + 1, out);
        }
    }
}

/// A `file://` URI for an absolute path.
pub(crate) fn uri(path: &str) -> String {
    let mut out = String::from("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The path of a `file://` URI.
pub(crate) fn path_of(uri: &str) -> Option<String> {
    let encoded = uri.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < encoded.len() {
        let hex = encoded
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (encoded[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(b)) => {
                bytes.push(b);
                i += 3;
            }
            (b, _) => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(bytes).ok()
}

/// An LSP signature help answer, as the editor shows it: the active
/// signature, its active parameter as characters of the label, and the
/// first paragraph of its documentation.
pub(crate) fn signature(help: &Value) -> Option<Signature> {
    let list = help["signatures"].as_array().filter(|l| !l.is_empty())?;
    let index = help["activeSignature"].as_u64().unwrap_or(0) as usize;
    let index = index.min(list.len() - 1);
    let sig = &list[index];
    let label = sig["label"].as_str()?.to_string();
    let param = sig["activeParameter"]
        .as_u64()
        .or(help["activeParameter"].as_u64())
        .and_then(|i| sig["parameters"].as_array()?.get(i as usize).cloned());
    let active = param.and_then(|p| match &p["label"] {
        // UTF-16 offsets into the label.
        Value::Array(range) => {
            let chars = |n: u64| {
                let mut seen = 0;
                label
                    .chars()
                    .take_while(|c| {
                        seen += c.len_utf16() as u64;
                        seen <= n
                    })
                    .count() as u32
            };
            Some((
                chars(range.first()?.as_u64()?),
                chars(range.get(1)?.as_u64()?),
            ))
        }
        Value::String(name) => {
            let at = label.find(name.as_str())?;
            let start = label[..at].chars().count() as u32;
            Some((start, start + name.chars().count() as u32))
        }
        _ => None,
    });
    let doc = match &sig["documentation"] {
        Value::String(s) => s.clone(),
        other => other["value"].as_str().unwrap_or_default().to_string(),
    };
    // Its first paragraph, outside any code block.
    let doc = doc
        .split("\n\n")
        .find(|p| !p.trim().is_empty() && !p.trim_start().starts_with("```"))
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some(Signature {
        label,
        active,
        doc,
        index: index as u32,
        count: list.len() as u32,
    })
}

/// An LSP inlay hint: its label, whole or in parts, at its position.
pub(crate) fn inlay_hint(hint: &Value) -> Option<InlayHint> {
    let label = match &hint["label"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["value"].as_str()).collect(),
        _ => return None,
    };
    let label = label.trim().to_string();
    let kind = match hint["kind"].as_u64() {
        Some(1) => InlayKind::Type,
        Some(2) => InlayKind::Parameter,
        _ => InlayKind::Other,
    };
    (!label.is_empty()).then(|| InlayHint {
        at: position(&hint["position"]),
        label,
        kind,
    })
}

pub(crate) fn hover_text(contents: &Value) -> String {
    let raw = match contents {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(hover_text)
            .collect::<Vec<_>>()
            .join("\n\n"),
        // An old-style code part: its language, and the code unfenced.
        Value::Object(o) if o.get("language").is_some() => format!(
            "```\n{}\n```",
            o.get("value").and_then(Value::as_str).unwrap_or_default()
        ),
        Value::Object(o) => o
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };
    // Markdown, with its code fences and rules kept for the card to draw;
    // only runs of blank lines go.
    let mut lines: Vec<&str> = Vec::new();
    for line in raw.lines() {
        if !(line.trim().is_empty() && lines.last().is_none_or(|l| l.trim().is_empty())) {
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_string()
}

/// LSP text edits as the editor's changes.
pub(crate) fn text_changes(list: &Value) -> Vec<TextChange> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|e| TextChange {
            start: position(&e["range"]["start"]),
            end: position(&e["range"]["end"]),
            text: e["newText"].as_str().unwrap_or_default().to_string(),
        })
        .collect()
}

/// A workspace edit's changes to files' text, as `changes` or as
/// `documentChanges`; creating, renaming and deleting files aren't done.
pub(crate) fn file_edits(edit: &Value) -> Vec<FileEdit> {
    let text_edits = text_changes;
    let mut files = Vec::new();
    if let Some(changes) = edit["changes"].as_object() {
        for (uri, list) in changes {
            if let Some(path) = path_of(uri) {
                files.push(FileEdit {
                    path,
                    edits: text_edits(list),
                });
            }
        }
    }
    for change in edit["documentChanges"].as_array().into_iter().flatten() {
        if let Some(path) = change["textDocument"]["uri"].as_str().and_then(path_of) {
            files.push(FileEdit {
                path,
                edits: text_edits(&change["edits"]),
            });
        }
    }
    files.retain(|f| !f.edits.is_empty());
    files
}

/// An LSP symbol kind as words, as declarations say them.
pub(crate) fn symbol_kind(kind: u64) -> &'static str {
    match kind {
        2 => "mod",
        3 => "namespace",
        5 => "class",
        6 => "method",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "fn",
        13 => "variable",
        14 => "const",
        22 => "variant",
        23 => "struct",
        26 => "type",
        _ => "symbol",
    }
}

pub(crate) fn position(value: &Value) -> Position {
    Position {
        line: value["line"].as_u64().unwrap_or(0) as u32,
        column: value["character"].as_u64().unwrap_or(0) as u32,
    }
}

/// A binary or bench to run: its name, and its command line and kind.
pub(crate) type RunTask = (String, (String, String));

/// The binaries among a crate's tasks, to debug: each built first, then
/// started from `target/debug`.
pub(crate) fn debug_targets(tasks: &BTreeMap<String, (String, String)>) -> Vec<DebugTarget> {
    tasks
        .iter()
        .filter(|(_, (command, detail))| detail == "bin" && command.starts_with("cargo run"))
        .filter_map(|(name, (command, _))| {
            let bin = name.strip_prefix("Run ")?;
            let (cargo, args) = match command.split_once(" -- ") {
                Some((c, a)) => (c, a.split_whitespace().map(String::from).collect()),
                None => (command.as_str(), Vec::new()),
            };
            Some(DebugTarget {
                name: bin.to_string(),
                debugger: String::new(),
                language: "rust".into(),
                program: format!("target/debug/{bin}"),
                args,
                build: cargo.replacen("cargo run", "cargo build", 1),
                detail: format!("--bin {bin}"),
                launch: String::new(),
            })
        })
        .collect()
}

/// rust-analyzer's runnables in a file: its binaries and benches, as tasks
/// by name with their command line and kind. Its tests are left to the
/// language's plugin, which runs them and reads their results.
pub(crate) fn runnables(list: &[Value]) -> Vec<RunTask> {
    let mut tasks = Vec::new();
    for r in list {
        let args = &r["args"];
        let strings = |v: &Value| -> Vec<String> {
            v.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let cargo = strings(&args["cargoArgs"]);
        let exec: Vec<String> = strings(&args["executableArgs"])
            .into_iter()
            .filter(|a| a != "--show-output" && a != "--nocapture")
            .collect();
        if r["kind"].as_str() != Some("cargo") || cargo.is_empty() {
            continue;
        }
        let label = r["label"].as_str().unwrap_or_default();
        let mut command = format!("cargo {}", cargo.join(" "));
        if !exec.is_empty() {
            command = format!("{command} -- {}", exec.join(" "));
        }
        if let Some(bin) = label.strip_prefix("run ") {
            tasks.push((format!("Run {bin}"), (command, "bin".to_string())));
        } else if let Some(bench) = label.strip_prefix("bench ") {
            tasks.push((format!("Bench {bench}"), (command, "bench".to_string())));
        }
    }
    tasks
}
