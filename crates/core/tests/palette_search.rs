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
fn ctrl_p_and_ctrl_shift_p_open_on_their_scopes() {
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
    s.run("key esc; key ctrl+shift+p");
    assert_eq!(palette(&s).kind, "Commands");
    // Backspace on the empty box widens it to everything.
    s.run("key backspace");
    assert_eq!(palette(&s).kind, "Everything");
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
