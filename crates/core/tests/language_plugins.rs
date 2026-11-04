//! What plugins tell the editor about a language: its name and comments,
//! colours, symbols and folds.

use merlin_core::lang::{Language, PluginSymbol, PluginToken};
use merlin_core::layers::Layer;
use merlin_core::screen::Style;
use merlin_core::script::Session;
use merlin_core::{Event, Syntax};

const CODE: &str = "int main() {\n    return 0;\n}\n";

fn session() -> Session {
    let mut s = Session::new(CODE, Some("main.cpp"), 60, 12);
    s.send(Event::RegisterLanguages {
        plugin: "cpp".into(),
        languages: vec![Language {
            id: "cpp".into(),
            name: "C++".into(),
            extensions: vec!["cpp".into()],
            line_comment: Some("//".into()),
            block_comment: None,
        }],
    });
    s
}

fn token(line: u32, a: u32, b: u32, kind: Syntax) -> PluginToken {
    PluginToken {
        start: (line, a),
        end: (line, b),
        kind,
    }
}

/// The colour of the cell where `text` starts.
fn syntax_at(s: &Session, text: &str) -> Syntax {
    let screen = s.screen();
    // Below the tabs, which name the file.
    let (y, row) = screen
        .lines()
        .enumerate()
        .skip(2)
        .find(|(_, l)| l.contains(text))
        .unwrap();
    let x = row[..row.find(text).unwrap()].chars().count();
    s.editor.render().cell(x as u16, y as u16).syntax
}

#[test]
fn a_plugin_language_names_the_file_and_comments_it() {
    let mut s = session();
    assert!(s.screen().contains("C++"), "{}", s.screen());
    s.run("key ctrl+/");
    assert!(s.text().starts_with("// int main()"), "{}", s.text());
}

#[test]
fn base_colours_replace_and_semantic_colours_go_over_them() {
    let mut s = session();
    s.send(Event::SetTokens {
        path: "main.cpp".into(),
        layer: Layer::Base,
        lines: None,
        tokens: vec![
            token(0, 0, 3, Syntax::Keyword),
            token(1, 4, 10, Syntax::Keyword),
        ],
    });
    assert_eq!(syntax_at(&s, "int"), Syntax::Keyword);
    assert_eq!(syntax_at(&s, "main()"), Syntax::Plain);
    s.send(Event::SetTokens {
        path: "main.cpp".into(),
        layer: Layer::Semantic,
        lines: None,
        tokens: vec![token(0, 4, 8, Syntax::Function)],
    });
    assert_eq!(syntax_at(&s, "main()"), Syntax::Function);
    // They move with edits.
    s.run("key ctrl+home; type // x\\n");
    assert_eq!(syntax_at(&s, "main()"), Syntax::Function);
    assert_eq!(syntax_at(&s, "return"), Syntax::Keyword);
}

#[test]
fn colours_for_some_lines_replace_only_those() {
    let mut s = session();
    let set = |s: &mut Session, lines, tokens| {
        s.send(Event::SetTokens {
            path: "main.cpp".into(),
            layer: Layer::Base,
            lines,
            tokens,
        })
    };
    set(
        &mut s,
        None,
        vec![
            token(0, 0, 3, Syntax::Keyword),
            token(1, 4, 10, Syntax::Keyword),
        ],
    );
    set(
        &mut s,
        Some((1, 2)),
        vec![token(1, 11, 12, Syntax::Constant)],
    );
    assert_eq!(syntax_at(&s, "int"), Syntax::Keyword);
    assert_eq!(syntax_at(&s, "return"), Syntax::Plain);
    assert_eq!(syntax_at(&s, "0;"), Syntax::Constant);
}

#[test]
fn plugin_symbols_fill_the_outline_and_breadcrumbs() {
    let mut s = session();
    s.editor.set_breadcrumbs(true);
    s.send(Event::SetSymbols {
        path: "main.cpp".into(),
        symbols: vec![PluginSymbol {
            name: "main".into(),
            kind: "function".into(),
            detail: "int ()".into(),
            start: (0, 0),
            end: (2, 1),
            depth: 0,
        }],
    });
    s.run("key down");
    assert!(s.screen().contains("main.cpp › main"), "{}", s.screen());
    s.run("cmd view.outline");
    assert!(s.screen().contains("ƒ main function"), "{}", s.screen());
}

#[test]
fn plugin_folds_replace_indentation() {
    let mut s = Session::new("a {\nb\n}\n", Some("main.cpp"), 60, 12);
    s.run("key down; key ctrl+shift+[");
    assert!(
        s.screen().contains("  2 b"),
        "no indentation, no fold: {}",
        s.screen()
    );
    s.send(Event::SetFolds {
        path: "main.cpp".into(),
        ranges: vec![(0, 1)],
    });
    s.run("key ctrl+shift+[");
    assert!(!s.screen().contains("  2 b"), "{}", s.screen());
    assert!(s.screen().contains("a { ⋯"), "{}", s.screen());
}

#[test]
fn colours_for_a_file_not_open_are_dropped() {
    let mut s = session();
    s.send(Event::SetTokens {
        path: "other.cpp".into(),
        layer: Layer::Base,
        lines: None,
        tokens: vec![token(0, 0, 3, Syntax::Keyword)],
    });
    assert_eq!(syntax_at(&s, "int"), Syntax::Plain);
    let _ = Style::Normal;
}

#[test]
fn a_review_asks_for_its_files_colours() {
    let mut s = Session::new("int a;\n", Some("main.cpp"), 80, 12);
    s.send(Event::ShowReview {
        plugin: "p".into(),
        id: "1".into(),
        path: "main.cpp".into(),
        old: "int a;\n".into(),
        new: "int b;\n".into(),
    });
    let wanted = s.editor.colour_requests();
    assert_eq!(wanted.len(), 1, "{wanted:?}");
    let (language, text) = wanted[0].clone();
    assert_eq!(language, "cpp");
    assert!(text.contains("int a;") && text.contains("int b;"), "{text}");
    // The plugin colours each `int`.
    let tokens = text
        .lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with("int"))
        .map(|(i, _)| token(i as u32, 0, 3, Syntax::Keyword))
        .collect();
    s.send(Event::TextColours {
        language,
        text,
        tokens,
    });
    assert_eq!(syntax_at(&s, "int b"), Syntax::Keyword);
    assert!(s.editor.colour_requests().is_empty(), "asked once");
}

#[test]
fn markdown_code_blocks_ask_for_colours() {
    use merlin_core::{Node, Panel};
    let mut s = Session::new("", None, 80, 20);
    s.send(Event::RegisterPlugin {
        plugin: "chat".into(),
        commands: Vec::new(),
    });
    s.send(Event::SetPanel {
        plugin: "chat".into(),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes: vec![Node::Markdown("```rust\nfn x() {}\n```".into())],
        }),
    });
    s.run("key ctrl+alt+b");
    // Code blocks ask while they're drawn.
    let _ = s.screen();
    let wanted = s.editor.colour_requests();
    assert_eq!(wanted, [("rust".to_string(), "fn x() {}".to_string())]);
    s.send(Event::TextColours {
        language: "rust".into(),
        text: "fn x() {}".into(),
        tokens: vec![token(0, 0, 2, Syntax::Keyword)],
    });
    assert_eq!(syntax_at(&s, "fn x"), Syntax::Keyword);
}
