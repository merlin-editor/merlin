//! Double and triple clicks, and preview tabs.

use std::path::PathBuf;

use merlin_core::screen::Style;
use merlin_core::script::Session;

/// The screen cell where `text` first shows.
fn find(s: &Session, text: &str) -> (usize, usize) {
    let screen = s.screen();
    let (y, row) = screen
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(text))
        .unwrap_or_else(|| panic!("no {text:?} in\n{screen}"));
    (row[..row.find(text).unwrap()].chars().count(), y)
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

fn click(s: &mut Session, verb: &str, text: &str) {
    let (x, y) = find(s, text);
    s.run(&format!("{verb} {x} {y}"));
}

#[test]
fn a_single_click_opens_a_preview_that_the_next_replaces() {
    let mut s = explorer();
    click(&mut s, "click", "one.md");
    assert_eq!(names(&s), ["main.rs", "one.md"]);
    let (x, y) = find(&s, "one.md ×");
    assert_eq!(
        s.editor.render().cell(x as u16, y as u16).style,
        Style::TabPreviewActive
    );
    click(&mut s, "click", "two.md");
    assert_eq!(names(&s), ["main.rs", "two.md"], "the preview was replaced");
    assert_eq!(s.text(), "two\n");
}

#[test]
fn editing_or_double_clicking_keeps_the_tab() {
    let mut s = explorer();
    click(&mut s, "click", "one.md");
    s.run("type x");
    click(&mut s, "click", "two.md");
    assert_eq!(
        names(&s),
        ["main.rs", "one.md", "two.md"],
        "edited, so kept"
    );

    click(&mut s, "double-click", "two.md");
    let (x, y) = find(&s, "two.md ×");
    assert_eq!(
        s.editor.render().cell(x as u16, y as u16).style,
        Style::TabActive
    );
    s.files.insert("three.md".into(), "three\n".into());
    s.run("key ctrl+shift+e; key esc");
    click(&mut s, "click", "three.md");
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
