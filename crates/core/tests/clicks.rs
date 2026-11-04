//! Double and triple clicks, and preview tabs.

use std::path::PathBuf;

use merlin_core::script::Session;

/// Where `text` first shows in the text.
fn find(s: &Session, text: &str) -> (u16, u16) {
    s.find(text)
        .unwrap_or_else(|| panic!("no {text:?} in\n{}", s.screen()))
}

#[test]
fn double_click_selects_a_word_and_triple_click_its_line() {
    let mut s = Session::new("let answer = 42;\nnext\n", Some("a.rs"), 60, 10);
    let (x, y) = find(&s, "answer");
    s.run(&format!("double-click {} {y}", x + 2));
    assert_eq!(s.editor.selection().as_deref(), Some("answer"));
    s.run(&format!("triple-click {} {y}", x + 2));
    assert_eq!(s.editor.selection().as_deref(), Some("let answer = 42;\n"));
    s.run("type x");
    assert_eq!(s.text(), "xnext\n");
}

#[test]
fn double_click_between_words_selects_the_character() {
    let mut s = Session::new("a = b\n", Some("a.rs"), 60, 10);
    let (x, y) = find(&s, "=");
    s.run(&format!("double-click {x} {y}"));
    assert_eq!(s.editor.selection().as_deref(), Some("="));
}

fn explorer() -> Session {
    let mut s = Session::new("fn a() {}\n", Some("main.rs"), 100, 20);
    s.files.insert("main.rs".into(), s.text());
    s.files.insert("one.md".into(), "one\n".into());
    s.files.insert("two.md".into(), "two\n".into());
    s.run("key ctrl+shift+e; key esc");
    s
}

fn names(s: &Session) -> Vec<String> {
    s.editor
        .open_files()
        .into_iter()
        .map(|p| p.unwrap_or_default().display().to_string())
        .collect()
}

/// The focused group's tabs: (name, active, preview).
fn tabs(s: &Session) -> Vec<(String, bool, bool)> {
    let view = s.editor.window_view();
    let group = view.groups.iter().find(|g| g.focused).unwrap();
    group
        .tabs
        .iter()
        .map(|t| (t.name.clone(), t.active, t.preview))
        .collect()
}

#[test]
fn a_single_click_opens_a_preview_that_the_next_replaces() {
    let mut s = explorer();
    s.click_entry("one.md", 1);
    assert_eq!(names(&s), ["main.rs", "one.md"]);
    assert!(
        tabs(&s).contains(&("one.md".into(), true, true)),
        "a preview"
    );
    s.click_entry("two.md", 1);
    assert_eq!(names(&s), ["main.rs", "two.md"], "the preview was replaced");
    assert_eq!(s.text(), "two\n");
}

#[test]
fn editing_or_double_clicking_keeps_the_tab() {
    let mut s = explorer();
    s.click_entry("one.md", 1);
    s.run("type x");
    s.click_entry("two.md", 1);
    assert_eq!(
        names(&s),
        ["main.rs", "one.md", "two.md"],
        "edited, so kept"
    );

    s.click_entry("two.md", 2);
    assert!(tabs(&s).contains(&("two.md".into(), true, false)), "kept");
    s.files.insert("three.md".into(), "three\n".into());
    s.run("key ctrl+shift+e; key esc");
    s.click_entry("three.md", 1);
    assert_eq!(names(&s).len(), 4);
}

#[test]
fn enter_in_the_explorer_opens_a_kept_tab() {
    let mut s = explorer();
    s.run("key ctrl+shift+e; key down; key enter");
    assert_eq!(
        s.editor.open_files()[1],
        Some(PathBuf::from("one.md")),
        "{}",
        s.screen()
    );
    s.run("key ctrl+shift+e; key down; key enter");
    assert_eq!(names(&s).len(), 3);
}
