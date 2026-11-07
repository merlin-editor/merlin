//! The palette as one search: scopes, sections, recent items and text.

use merlin_core::script::Session;
use merlin_core::{Event, PaletteIcon, PaletteView};

fn project() -> Session {
    let mut s = Session::new("", None, 80, 20);
    s.files.insert(
        "src/fold.rs".into(),
        "/// Each fold is the line it starts on.\nfn fold() {}\nfn fold_all() {}\n".into(),
    );
    s.files
        .insert("src/main.rs".into(), "fn main() { fold(); }\n".into());
    s.files.insert("README.md".into(), "# hi\n".into());
    s
}

fn palette(s: &Session) -> PaletteView {
    s.editor.window_view().palette.expect("the palette is open")
}

fn headings(p: &PaletteView) -> Vec<String> {
    p.extras.iter().filter_map(|e| e.heading.clone()).collect()
}

/// Lets the text search's pause go by.
fn pause(s: &mut Session) {
    if let Some((tick, _)) = s.editor.palette_search_wait() {
        s.send(Event::PaletteSearch(tick));
    }
}

#[test]
fn ctrl_p_opens_on_files_and_ctrl_shift_p_on_everything() {
    let mut s = project();
    s.run("key ctrl+p");
    let p = palette(&s);
    assert_eq!(p.kind, "Files");
    let on: Vec<&str> = p
        .scopes
        .iter()
        .filter(|c| c.2)
        .map(|c| c.0.as_str())
        .collect();
    assert_eq!(on, ["Files"]);
    // Backspace on the empty box widens it to everything.
    s.run("key backspace");
    assert_eq!(palette(&s).kind, "Everything");
    // The command palette searches everything, and > narrows it.
    s.run("key esc; key ctrl+shift+p");
    assert_eq!(palette(&s).kind, "Everything");
    s.run("type >");
    assert_eq!(palette(&s).kind, "Commands");
}

#[test]
fn everything_has_a_section_each_with_its_best_few() {
    let mut s = project();
    s.run("key ctrl+p; type fold; key enter");
    s.run("key ctrl+shift+p; key backspace; type fold");
    pause(&mut s);
    let p = palette(&s);
    assert_eq!(headings(&p), ["Commands", "Symbols", "Files", "Text"]);
    let labels: Vec<&str> = p.items.iter().map(|i| i.0.as_str()).collect();
    assert_eq!(&labels[..2], ["Fold", "Fold All"]);
    assert!(labels.contains(&"fn fold"), "{labels:?}");
    assert!(labels.contains(&"fold.rs"), "{labels:?}");
    let text = p.extras.iter().position(|e| e.code).expect("a text row");
    assert_eq!(p.extras[text].hit, Some((9, 13)), "the match is tinted");
    assert!(
        p.extras[text].more.as_ref().unwrap().0.contains("more in"),
        "{:?}",
        p.extras[text].more
    );
    assert!(matches!(p.extras[0].icon, PaletteIcon::Command));
}

#[test]
fn a_sign_typed_first_picks_the_scope() {
    let mut s = project();
    s.run("key ctrl+shift+p; key backspace; type @");
    let p = palette(&s);
    assert_eq!(p.kind, "Symbols");
    assert_eq!(p.sign, Some('@'));
    assert_eq!(p.query, "");
    s.run("key backspace; type >");
    assert_eq!(palette(&s).kind, "Commands");
    // Tab goes through the scopes, and round.
    s.run("key tab; key tab");
    assert_eq!(palette(&s).kind, "Text");
    s.run("key tab");
    assert_eq!(palette(&s).kind, "Everything");
    s.run("key shift+tab");
    assert_eq!(palette(&s).kind, "Text");
}

#[test]
fn text_is_searched_after_a_pause_and_enter_goes_there() {
    let mut s = project();
    s.run("key ctrl+shift+p; key backspace; type /fold()");
    assert!(s.editor.palette_search_wait().is_some(), "it waits");
    pause(&mut s);
    let p = palette(&s);
    assert_eq!(headings(&p), ["2 in 2 files"]);
    assert_eq!(p.items[1].0, "fn main() { fold(); }");
    s.run("key down; key enter");
    assert_eq!(s.text(), "fn main() { fold(); }\n");
    assert_eq!(s.editor.cursor(), 12);
}

#[test]
fn empty_it_shows_the_files_and_commands_used_last() {
    let mut s = project();
    s.run("key ctrl+p; type fold.rs; key enter; key ctrl+p; type readme; key enter");
    s.run("key ctrl+shift+p; type fold all; key enter");
    s.run("key ctrl+shift+p; key backspace");
    let p = palette(&s);
    assert_eq!(headings(&p), ["Recently opened", "Recently used"]);
    let labels: Vec<&str> = p.items.iter().map(|i| i.0.as_str()).collect();
    // Not the file in front.
    assert_eq!(labels, ["fold.rs", "Fold All"]);
}

#[test]
fn a_command_runs_and_a_file_opens() {
    let mut s = project();
    s.run("key ctrl+p; type main; key enter");
    assert_eq!(s.text(), "fn main() { fold(); }\n");
    s.run("key ctrl+shift+p; type toggle line comment; key enter");
    assert_eq!(s.text(), "// fn main() { fold(); }\n");
}

#[test]
fn the_chevron_opens_a_commands_choices() {
    let mut s = project();
    s.run("key ctrl+p; type fold.rs; key enter");
    s.run("key ctrl+shift+p; type fold all");
    let p = palette(&s);
    assert!(p.extras[0].choices, "Fold All offers levels");
    assert_eq!(p.extras[0].detail, "every block in the file");
    s.run("key right");
    let p = palette(&s);
    assert_eq!(p.kind, "Choices");
    assert_eq!(p.items[0].0, "Fold Level 1");
    // Backspace goes back to the commands.
    s.run("key backspace");
    assert_eq!(palette(&s).kind, "Commands");
    s.run("key esc");
}

#[test]
fn moving_through_results_previews_them_and_esc_goes_back() {
    let mut s = project();
    s.run("key ctrl+p; type fold.rs; key enter; key ctrl+p; type main; key enter");
    assert_eq!(s.text(), "fn main() { fold(); }\n");
    s.run("key ctrl+shift+p; key backspace; type @fold; key down");
    // The second symbol, fold_all, is in fold.rs, open behind.
    assert!(s.text().starts_with("/// Each fold"), "{}", s.text());
    assert_eq!(s.editor.cursor(), 53);
    s.run("key esc");
    assert_eq!(s.text(), "fn main() { fold(); }\n");
    assert_eq!(s.editor.cursor(), 0);
}

#[test]
fn project_symbols_come_from_language_plugins() {
    let mut s = project();
    s.send(Event::RegisterSymbolSearch {
        plugin: "lsp".into(),
        command: "project-symbols".into(),
    });
    s.run("key ctrl+shift+p; key backspace; type @widget");
    let (tick, _) = s.editor.palette_search_wait().expect("it waits");
    s.effects.clear();
    s.send(Event::PaletteSearch(tick));
    assert!(s.effects.contains(&merlin_core::Effect::SearchSymbols {
        plugin: "lsp".into(),
        command: "project-symbols".into(),
        query: "widget".into(),
    }));
    s.send(Event::ProjectSymbols {
        query: "widget".into(),
        symbols: vec![merlin_core::lang::ProjectSymbol {
            name: "Widget".into(),
            kind: "struct".into(),
            container: "ui".into(),
            path: "src/ui.rs".into(),
            line: 4,
            column: 11,
        }],
    });
    let p = palette(&s);
    assert_eq!(p.items[0].0, "struct Widget");
    assert_eq!(p.extras[0].detail, "ui · ui.rs:5");
    assert!(matches!(p.extras[0].icon, PaletteIcon::Symbol('T', true)));
}

#[test]
fn recents_are_kept_with_the_workspace() {
    let mut s = project();
    s.run("key ctrl+p; type fold.rs; key enter; key ctrl+p; type readme; key enter");
    s.run("key ctrl+shift+p; type fold all; key enter");
    let text = s.editor.workspace().to_text();
    assert!(text.contains("recent-command edit.fold_all"), "{text}");
    let ws = merlin_core::Workspace::parse(&text);
    assert_eq!(ws.recent_commands, ["edit.fold_all"]);
    assert_eq!(ws.recent_files[1].to_str(), Some("src/fold.rs"));
}

#[test]
fn fold_level_folds_the_blocks_that_deep() {
    let mut s = Session::new(
        "fn a() {\n    if x {\n        y();\n    }\n}\n",
        Some("a.rs"),
        60,
        12,
    );
    s.run("key ctrl+shift+p; type fold level 2; key enter");
    let screen = s.screen();
    assert!(!screen.contains("y();"), "{screen}");
    assert!(screen.contains("fn a()"), "{screen}");
}

#[test]
fn a_result_in_a_file_not_open_previews_in_the_preview_tab() {
    let mut s = project();
    s.run("key ctrl+p; type main; key enter");
    // fn fold() in fold.rs, then the call in main.rs.
    s.run("key ctrl+shift+p; key backspace; type /fold()");
    pause(&mut s);
    s.run("key down; key up");
    assert!(s.text().starts_with("/// Each fold"), "{}", s.text());
    assert_eq!(s.editor.cursor(), 43, "at the match");
    assert_eq!(s.editor.open_files().len(), 2);
    s.run("key esc");
    assert_eq!(s.text(), "fn main() { fold(); }\n");
    assert_eq!(s.editor.open_files().len(), 1, "the preview went");
    // Picked, it stays as a tab of its own.
    s.run("key ctrl+shift+p; key backspace; type /fold()");
    pause(&mut s);
    s.run("key down; key up; key enter");
    assert!(s.text().starts_with("/// Each fold"));
    assert_eq!(s.editor.open_files().len(), 2);
    let tabs = s.editor.window_view().tabs;
    assert!(tabs.iter().all(|t| !t.preview), "kept, not a preview");
}

#[test]
fn letters_together_rank_before_scattered_ones() {
    let mut s = project();
    s.run("key ctrl+shift+p; type >wrap");
    assert_eq!(palette(&s).items[0].0, "Toggle Word Wrap");
}
