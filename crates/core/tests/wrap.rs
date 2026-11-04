//! Word wrap: long lines break into rows at spaces, and the cursor, mouse
//! and scrolling follow the rows.

use merlin_core::script::Session;

/// 60 columns leave 56 for text after the gutter.
fn session() -> Session {
    let long = "word ".repeat(30);
    let text = format!("short\n{long}\nlast\n");
    let mut s = Session::new(&text, Some("notes.txt"), 60, 12);
    s.run("cmd view.wrap");
    s
}

fn text_rows(s: &Session) -> Vec<String> {
    s.editor.text_rows()
}

#[test]
fn long_lines_wrap_at_spaces_with_one_line_number() {
    let s = session();
    let rows = text_rows(&s);
    assert!(rows[0].contains("  1 short"), "{rows:#?}");
    assert!(rows[1].contains("  2 word word"), "{rows:#?}");
    // The rest of line 2 carries on without a number, and breaks only
    // between words.
    assert!(rows[2].contains("    word word"), "{rows:#?}");
    assert!(!rows[1].trim_end().ends_with("wo"), "{rows:#?}");
    let line_3 = rows.iter().position(|r| r.contains("  3 last")).unwrap();
    assert!(line_3 >= 4, "{rows:#?}");
}

#[test]
fn up_and_down_move_by_row_and_end_goes_to_the_line_end() {
    let mut s = session();
    s.run("key down down");
    // Down twice from line 1 lands on line 2's second row, not line 3.
    let (x, y) = s.editor.text_origin();
    assert_eq!(s.cursor(), Some((x, y + 2)));
    let pos = s.editor.cursor();
    assert!(pos > 6 + 40 && pos < 6 + 150, "{pos}");
    s.run("key end");
    assert_eq!(s.editor.cursor(), 6 + 150);
}

#[test]
fn a_click_on_a_wrapped_row_places_the_cursor_there() {
    let mut s = session();
    let (x, y) = s.editor.text_origin();
    s.run(&format!("click {} {}", x + 5, y + 2));
    let row = &text_rows(&s)[2];
    let first_word = row.find("word").unwrap_or_else(|| panic!("{row:?}")) as u16;
    let pos = s.editor.cursor();
    let rendered = s.cursor().unwrap();
    assert_eq!(rendered, (x + 5, y + 2));
    assert!(pos > 6 + 40, "{pos} {first_word}");
}

#[test]
fn the_view_scrolls_by_rows_to_keep_the_cursor_in_sight() {
    let long = "word ".repeat(60);
    let text = format!("{long}\n").repeat(4);
    let mut s = Session::new(&text, Some("notes.txt"), 60, 12);
    s.run("cmd view.wrap; key ctrl+end");
    let (_, y) = s.editor.text_origin();
    let (_, cy) = s.cursor().unwrap();
    assert!(cy >= y, "cursor on screen");
    assert!(s.screen().contains("  5"), "{}", s.screen());
}

#[test]
fn wrap_toggles_off_again() {
    let mut s = session();
    s.run("cmd view.wrap");
    let rows = text_rows(&s);
    assert!(rows[2].contains("  3 last"), "{rows:#?}");
}
