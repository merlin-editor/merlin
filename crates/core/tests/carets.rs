//! Multiple cursors: Ctrl+D, Ctrl+Shift+L, cursors above and below, and
//! Alt+click, with typing, deleting, moving, copying and pasting at all of
//! them as one undo step.

use merlin_core::Effect;
use merlin_core::screen::Style;
use merlin_core::script::Session;

fn session(text: &str) -> Session {
    Session::new(text, Some("notes.txt"), 60, 12)
}

#[test]
fn ctrl_d_selects_the_word_then_adds_the_next_occurrence() {
    let mut s = session("let foo = foo + bar + foo;\n");
    s.run("key right right right right; key ctrl+d");
    assert_eq!(s.editor.selection().as_deref(), Some("foo"));
    s.run("key ctrl+d; key ctrl+d");
    assert!(s.screen().contains("(3 selections)"), "{}", s.screen());
    s.run("type baz");
    assert_eq!(s.text(), "let baz = baz + bar + baz;\n");
    // As with one cursor, replacing the selections is one step and the
    // rest of the word another; each covers all three cursors.
    s.run("key ctrl+z");
    assert_eq!(s.text(), "let b = b + bar + b;\n");
    s.run("key ctrl+z");
    assert_eq!(s.text(), "let foo = foo + bar + foo;\n");
    assert!(
        !s.screen().contains("selections"),
        "undo goes back to one cursor"
    );
}

#[test]
fn select_all_occurrences_and_delete_them() {
    let mut s = session("a1 b a1 c a1\n");
    s.run("key shift+right shift+right; key ctrl+shift+l; key backspace");
    assert_eq!(s.text(), " b  c \n");
}

#[test]
fn cursors_above_and_below_type_on_every_line() {
    let mut s = session("one\ntwo\nthree\n");
    s.run("key shift+alt+down shift+alt+down");
    assert!(s.screen().contains("(3 selections)"), "{}", s.screen());
    s.run("type -x; key end; type .");
    assert_eq!(s.text(), "-xone.\n-xtwo.\n-xthree.\n");
    // The other cursors are drawn on screen.
    let render = s.editor.render();
    let carets = (0..render.height)
        .flat_map(|y| (0..render.width).map(move |x| (x, y)))
        .filter(|&(x, y)| render.cell(x, y).style == Style::Caret)
        .count();
    assert_eq!(carets, 2);
    s.run("key esc; type !");
    assert_eq!(s.text(), "-xone.\n-xtwo.\n-xthree.!\n");
}

#[test]
fn alt_click_adds_a_cursor() {
    let mut s = session("abc\ndef\n");
    let (x, y) = s.editor.text_origin();
    s.run(&format!("alt-click {} {}", x + 1, y + 1));
    s.run("type X");
    assert_eq!(s.text(), "Xabc\ndXef\n");
    // A plain click goes back to one cursor.
    s.run(&format!("click {} {}", x, y));
    s.run("type Y");
    assert_eq!(s.text(), "YXabc\ndXef\n");
}

#[test]
fn copy_and_paste_spread_one_line_per_cursor() {
    let mut s = session("a = 1\nb = 2\n\n\n");
    s.run("key shift+alt+down; key shift+right; key ctrl+c");
    assert!(
        s.effects.contains(&Effect::Copy("a\nb".into())),
        "{:?}",
        s.effects
    );
    s.run("key ctrl+end; key up; key esc");
    s.run("key shift+alt+up");
    s.send(merlin_core::Event::Clipboard(Some("a\nb".into())));
    assert_eq!(s.text(), "a = 1\nb = 2\na\nb\n");
}
