//! What plugins tell the editor about a language: its name and comments,
//! colours, symbols and folds.

use merlin_core::lang::{Language, PluginSymbol, PluginToken};
use merlin_core::layers::Layer;
use merlin_core::script::Session;
use merlin_core::style::Style;
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
            ..Default::default()
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

/// The colour of the text where `text` starts.
fn syntax_at(s: &Session, text: &str) -> Syntax {
    let (x, y) = s
        .find(text)
        .unwrap_or_else(|| panic!("no {text:?} in\n{}", s.screen()));
    s.cell(x, y).unwrap().syntax
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
    assert!(s.screen().contains("main  function"), "{}", s.screen());
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

fn plugin(s: &mut Session, name: &str, language: Option<&str>, key: &str) {
    if let Some(ext) = language {
        s.send(Event::RegisterLanguages {
            plugin: name.into(),
            languages: vec![Language {
                id: ext.into(),
                name: ext.into(),
                extensions: vec![ext.into()],
                line_comment: None,
                block_comment: None,
                ..Default::default()
            }],
        });
    }
    s.send(Event::RegisterPlugin {
        plugin: name.into(),
        commands: vec![merlin_core::Command {
            id: "hover".into(),
            title: format!("{name}: Show Hover"),
            key: Some(key.into()),
        }],
    });
}

fn ran(s: &Session) -> Vec<(String, String)> {
    s.effects
        .iter()
        .filter_map(|e| match e {
            merlin_core::Effect::PluginCommand { plugin, command } => {
                Some((plugin.clone(), command.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_language_plugins_key_runs_with_the_plugin_for_the_file() {
    let mut s = session();
    plugin(&mut s, "python", Some("py"), "alt+k");
    plugin(&mut s, "cpp", None, "alt+k");
    s.effects.clear();
    s.run("key alt+k");
    assert_eq!(ran(&s), [("cpp".into(), "hover".into())]);
}

#[test]
fn another_plugins_key_runs_its_own_command_of_the_same_name() {
    let mut s = session();
    plugin(&mut s, "cpp", None, "alt+k");
    plugin(&mut s, "lsp", None, "alt+l");
    plugin(&mut s, "notes", None, "alt+j");
    s.effects.clear();
    s.run("key alt+j");
    assert_eq!(ran(&s), [("notes".into(), "hover".into())]);
    s.effects.clear();
    s.run("key alt+l");
    assert_eq!(
        ran(&s),
        [("cpp".into(), "hover".into())],
        "but the general client's is for cpp's"
    );
}

#[test]
fn the_palette_lists_a_language_command_once_for_the_file() {
    let mut s = session();
    plugin(&mut s, "python", Some("py"), "alt+k");
    plugin(&mut s, "cpp", None, "alt+k");
    s.run("key ctrl+shift+p");
    s.run("type Show Hover");
    let screen = s.screen();
    let rows: Vec<_> = screen
        .lines()
        .filter(|l| l.trim_start_matches([' ', '›']).split("  ").next() == Some("Show Hover"))
        .collect();
    assert_eq!(rows.len(), 1, "{screen}");
    assert!(
        rows[0].ends_with("Alt+K"),
        "with the key that runs it: {screen}"
    );
}
