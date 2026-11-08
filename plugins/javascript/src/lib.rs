//! JavaScript and TypeScript language plugin: tree-sitter syntax highlighting,
//! folding, `typescript-language-server` configured with `maxTsServerMemory: 512`
//! for bounded memory usage, and Run Task's npm scripts from package.json.

use merlin_grammar::Grammar;
use merlin_lsp::{Spec, WithLsp};
use merlin_plugin::{BufferEvent, Plugin, Task, TaskGroup, Token, host};

/// typescript-language-server with a bounded tsserver heap.
const JS_SERVER: Spec = Spec {
    program: "typescript-language-server",
    args: &["--stdio"],
    files: &[
        ("ts", "typescript"),
        ("tsx", "typescriptreact"),
        ("js", "javascript"),
        ("mjs", "javascript"),
        ("cjs", "javascript"),
        ("jsx", "javascriptreact"),
    ],
    settings: Some(r#"{"tsserver":{"maxTsServerMemory":512}}"#),
    options: &[],
    install: Some("npm install -g typescript-language-server typescript"),
};

merlin_plugin::export!(WithLsp<JavaScript> => WithLsp::new(JavaScript::new(), "JavaScript", &[JS_SERVER]));

pub struct JavaScript {
    grammar: Grammar,
}

impl JavaScript {
    fn new() -> Self {
        Self {
            grammar: Grammar::new(
                tree_sitter_javascript::LANGUAGE.into(),
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                &["js", "mjs", "cjs", "jsx"],
            ),
        }
    }
}

/// A package.json's scripts, as name and group, each run with npm.
fn scripts(text: &str) -> Vec<(String, TaskGroup)> {
    let Some(at) = text.find("\"scripts\"") else {
        return Vec::new();
    };
    let rest = &text[at + 9..];
    let Some(open) = rest.find('{') else {
        return Vec::new();
    };
    // The strings in the object, in pairs, up to its closing brace.
    let mut strings = Vec::new();
    let mut chars = rest[open + 1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '}' => break,
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => s.extend(chars.next()),
                        '"' => break,
                        c => s.push(c),
                    }
                }
                strings.push(s);
            }
            _ => {}
        }
    }
    strings
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| {
            let group = match pair[0].as_str() {
                "test" => TaskGroup::Test,
                "build" | "dev" | "start" => TaskGroup::Build,
                _ => TaskGroup::Other,
            };
            (pair[0].clone(), group)
        })
        .collect()
}

impl Plugin for JavaScript {
    /// Run Task asks for tasks as it opens: package.json's scripts, read
    /// again each time.
    fn run_command(&mut self, id: &str) {
        if id != "find-tasks" {
            return;
        }
        let text = host::read_file("package.json").unwrap_or_default();
        let tasks: Vec<Task> = scripts(&text)
            .into_iter()
            .map(|(name, group)| Task {
                command: Some(format!("npm run {name}")),
                name,
                group,
                detail: "package.json".into(),
            })
            .collect();
        host::set_tasks("npm", &tasks);
    }

    fn buffer(&mut self, event: BufferEvent) {
        self.grammar.buffer(event);
    }

    fn colour_text(&mut self, _language: &str, text: &str) -> Vec<Token> {
        self.grammar.colour_text(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_npm_tasks_grouped_by_name() {
        let found = scripts(r#"{"scripts":{"test":"jest","dev":"vite --port \"3000\""}}"#);
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["test", "dev"]);
        assert!(matches!(found[0].1, TaskGroup::Test));
        assert!(matches!(found[1].1, TaskGroup::Build));
    }
}
