//! The text as data, for windows that draw it themselves: lines with their
//! colours and styles, and the cursors in columns.

use merlin_core::screen::Style;
use merlin_core::script::Session;

fn rust(text: &str) -> Session {
    Session::new(text, Some("main.rs"), 60, 10)
}

#[test]
fn lines_on_screen_with_their_numbers() {
    let s = rust("fn main() {\n    let x = 1;\n}\n");
    let views = s.editor.text_views();
    let view = views.last().unwrap();
    let texts: Vec<String> = view
        .lines
        .iter()
        .map(|l| l.chars.iter().collect())
        .collect();
    assert_eq!(texts, ["fn main() {", "    let x = 1;", "}", ""]);
    assert_eq!(view.lines[1].number.trim(), "2");
    assert_eq!(view.lines[1].row, 1);
    assert!(!view.beyond.is_empty(), "rows past the end");
}

#[test]
fn carets_are_in_lines_and_columns() {
    let mut s = rust("one\ntwo\nthree\n");
    s.run("key down right right; key shift+alt+down");
    let views = s.editor.text_views();
    let carets = &views.last().unwrap().carets;
    assert_eq!((carets[0].line, carets[0].col), (2, 2), "the primary first");
    assert_eq!((carets[1].line, carets[1].col), (1, 2));
    assert_eq!(carets[0].cell, Some((2, 2)));
}

#[test]
fn selections_and_the_current_line_are_styles() {
    let mut s = rust("abc def\nxyz\n");
    s.run("key shift+ctrl+right");
    let views = s.editor.text_views();
    let line = &views.last().unwrap().lines[0];
    assert_eq!(&line.styles[..3], [Style::Selection; 3]);
    assert_eq!(
        line.styles[3],
        Style::Normal,
        "no current line with a selection"
    );
    s.run("key down");
    let views = s.editor.text_views();
    let line = &views.last().unwrap().lines[1];
    assert_eq!(line.base, Style::CurrentLine);
}

#[test]
fn scrolled_and_folded_views_start_and_skip_where_the_screen_does() {
    let text: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let mut s = rust(&text);
    s.run("key ctrl+end");
    let views = s.editor.text_views();
    let view = views.last().unwrap();
    assert_eq!(view.lines[0].line, view.scroll);
    assert!(view.scroll > 0);

    let mut s = rust("fn a() {\n    x\n    y\n}\nz\n");
    s.run("key ctrl+shift+[");
    let views = s.editor.text_views();
    let view = views.last().unwrap();
    assert!(view.lines[0].folded);
    assert_eq!(
        view.lines[1].line, 3,
        "the folded lines are skipped to the closer"
    );
}
