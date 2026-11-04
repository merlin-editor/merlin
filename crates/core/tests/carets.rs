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

#[test]
fn shift_alt_drag_selects_a_box() {
    let mut s = session("abcdef\nghijkl\nmnopqr\n");
    let (x, y) = s.editor.text_origin();
    s.run(&format!(
        "box-press {} {}; drag {} {}; release {} {}",
        x + 1,
        y,
        x + 3,
        y + 2,
        x + 3,
        y + 2
    ));
    s.run("type _");
    assert_eq!(s.text(), "a_def\ng_jkl\nm_pqr\n");
}

#[test]
fn ctrl_u_takes_back_the_last_cursor() {
    let mut s = session("foo foo foo\n");
    s.run("key ctrl+d ctrl+d ctrl+d; key ctrl+u");
    assert!(s.screen().contains("(2 selections)"), "{}", s.screen());
    s.run("key ctrl+u");
    assert!(!s.screen().contains("selections"));
    assert_eq!(s.editor.selection().as_deref(), Some("foo"));
}

#[test]
fn ctrl_k_ctrl_d_skips_to_the_next_occurrence() {
    let mut s = session("a x a x a\n");
    s.run("key ctrl+d ctrl+d; key ctrl+k ctrl+d; type b");
    assert_eq!(s.text(), "b x a x b\n", "the middle one was skipped");
}

#[test]
fn an_unknown_chord_says_so() {
    let mut s = session("a\n");
    s.run("key ctrl+k q");
    assert_eq!(s.text(), "a\n", "the second key isn't typed");
    assert!(s.editor.message().unwrap().contains("is not a command"));
}

#[test]
fn expanding_grows_by_word_inside_brackets_and_lines() {
    let mut s = Session::new("fn f() {\n    call(one, two);\n}\n", Some("a.rs"), 60, 12);
    s.run("key down ctrl+right ctrl+right right");
    let grow = |s: &mut Session| {
        s.run("key shift+alt+right");
        s.editor.selection().unwrap_or_default()
    };
    assert_eq!(grow(&mut s), "one");
    assert_eq!(grow(&mut s), "one, two");
    assert_eq!(grow(&mut s), "(one, two)");
    assert_eq!(
        grow(&mut s),
        "call(one, two)",
        "the call, as its word reaches"
    );
    assert_eq!(grow(&mut s), "call(one, two);");
    assert_eq!(grow(&mut s), "    call(one, two);\n");
    assert_eq!(grow(&mut s), "\n    call(one, two);\n");
    s.run("key shift+alt+left shift+alt+left");
    assert_eq!(s.editor.selection().as_deref(), Some("call(one, two);"));
    s.run("key left; key shift+alt+left");
    assert_eq!(s.editor.selection(), None, "moving forgets the growing");
}

#[test]
fn dragging_after_a_double_click_selects_whole_words() {
    let mut s = session("alpha beta gamma\n");
    let (x, y) = s.editor.text_origin();
    s.run(&format!(
        "double-press {} {}; drag {} {}; release {} {}",
        x + 1,
        y,
        x + 7,
        y,
        x + 7,
        y
    ));
    assert_eq!(s.editor.selection().as_deref(), Some("alpha beta"));
}
