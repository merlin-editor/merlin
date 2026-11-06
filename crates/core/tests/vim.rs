//! Vim's keys: modes, motions, operators, text objects, registers, the
//! dot and undo.

use merlin_core::script::Session;
use merlin_core::{Event, Key, KeyCode, Keymap, Settings, VimMode, VimSettings};

fn vim(text: &str) -> Session {
    let mut s = Session::new(text, Some("notes.rs"), 60, 12);
    s.send(Event::Settings(Settings {
        keymap: Keymap::Vim,
        ..Settings::default()
    }));
    s
}

fn mode(s: &Session) -> VimMode {
    s.editor.window_view().vim.expect("vim is on").mode
}

#[test]
fn normal_mode_moves_and_insert_mode_types() {
    let mut s = vim("hello world");
    assert_eq!(mode(&s), VimMode::Normal);
    s.run("type wix; key esc");
    assert_eq!(s.text(), "hello xworld");
    assert_eq!(mode(&s), VimMode::Normal);
    // Esc goes back over the letter typed.
    assert_eq!(s.editor.cursor(), 6);
    s.run("type Ab; key esc");
    assert_eq!(s.text(), "hello xworldb");
}

#[test]
fn operators_take_motions_counts_and_doubles() {
    let mut s = vim("one two three four\nfive\nsix");
    s.run("type dw");
    assert_eq!(s.text(), "two three four\nfive\nsix");
    s.run("type 2dw");
    assert_eq!(s.text(), "four\nfive\nsix");
    s.run("type dd");
    assert_eq!(s.text(), "five\nsix");
    s.run("type p");
    assert_eq!(
        s.text(),
        "five\nfour\nsix",
        "dd puts the line in the register"
    );
    s.run("type ggd$");
    assert_eq!(s.text(), "\nfour\nsix");
}

#[test]
fn dw_on_a_lines_last_word_keeps_the_line_break() {
    let mut s = vim("alpha beta\ngamma");
    s.run("type wdw");
    assert_eq!(s.text(), "alpha \ngamma");
}

#[test]
fn change_word_then_dot_repeats_it() {
    let mut s = vim("foo bar baz");
    s.run("type cwqux; key esc; type w.");
    assert_eq!(s.text(), "qux qux baz");
    s.run("type u");
    assert_eq!(s.text(), "qux bar baz", "u undoes the whole change");
    s.run("type u");
    assert_eq!(s.text(), "foo bar baz");
    s.run("key ctrl+r");
    assert_eq!(s.text(), "qux bar baz");
}

#[test]
fn text_objects_inside_brackets_and_quotes() {
    let mut s = vim("call(a, b) \"text here\"");
    s.run("type fbdi(");
    assert_eq!(s.text(), "call() \"text here\"");
    s.run("type fhci\"x; key esc");
    assert_eq!(s.text(), "call() \"x\"");
    s.run("type 0daw");
    assert_eq!(s.text(), "() \"x\"");
}

#[test]
fn find_on_the_line_and_repeat_it() {
    let mut s = vim("a,b,c,d");
    s.run("type f,");
    // The script's `;` separates its steps, so this one is sent.
    s.send(Event::Key(Key::plain(KeyCode::Char(';'))));
    assert_eq!(s.editor.cursor(), 3);
    s.run("type dt,");
    assert_eq!(s.text(), "a,b,d");
}

#[test]
fn visual_mode_selects_and_operates() {
    let mut s = vim("one two three");
    s.run("type vee");
    assert_eq!(mode(&s), VimMode::Visual);
    assert_eq!(s.editor.selection().as_deref(), Some("one two"));
    s.run("type d");
    assert_eq!(s.text(), " three");
    assert_eq!(mode(&s), VimMode::Normal);
    let mut s = vim("a\nb\nc\nd");
    s.run("type Vjy");
    s.run("type Gp");
    assert_eq!(s.text(), "a\nb\nc\nd\na\nb");
}

#[test]
fn visual_block_changes_every_line() {
    let mut s = vim("abc\nabc\nabc");
    s.run("key ctrl+v; type jjlc; type X; key esc");
    assert_eq!(s.text(), "Xc\nXc\nXc");
}

#[test]
fn registers_keep_yanks_apart_from_deletes() {
    let mut s = vim("keep\ngone");
    s.run("type yyjdd\"0p");
    assert_eq!(s.text(), "keep\nkeep");
    let mut s = vim("word");
    s.run("type \"ayiw$\"ap");
    assert_eq!(s.text(), "wordword");
}

#[test]
fn join_replace_and_case() {
    let mut s = vim("one\n    two\nthree");
    s.run("type J");
    assert_eq!(s.text(), "one two\nthree");
    s.run("type 0rX");
    assert_eq!(s.text(), "Xne two\nthree");
    s.run("type g~iw");
    assert_eq!(s.text(), "xNE two\nthree");
    s.run("type j0gUU");
    assert_eq!(s.text(), "xNE two\nTHREE");
}

#[test]
fn indent_and_comment_lines() {
    let mut s = vim("fn a() {}\nfn b() {}");
    s.run("type >>");
    assert_eq!(s.text(), "    fn a() {}\nfn b() {}");
    s.run("type gcj");
    // At the lines' least indent, as merlin comments.
    assert_eq!(s.text(), "//     fn a() {}\n// fn b() {}");
}

#[test]
fn numbers_go_up_and_down() {
    let mut s = vim("width = 41;");
    s.run("key ctrl+a");
    assert_eq!(s.text(), "width = 42;");
    s.run("type 50");
    s.run("key ctrl+x");
    assert_eq!(s.text(), "width = -8;");
}

#[test]
fn jk_leaves_insert_mode_when_set() {
    let mut s = Session::new("", Some("notes.rs"), 60, 12);
    s.send(Event::Settings(Settings {
        keymap: Keymap::Vim,
        vim: VimSettings {
            insert_escape: "jk".into(),
            ..VimSettings::default()
        },
        ..Settings::default()
    }));
    s.run("type ihijk");
    assert_eq!(s.text(), "hi");
    assert_eq!(mode(&s), VimMode::Normal);
}

#[test]
fn kept_ctrl_keys_do_merlins_thing() {
    let mut s = vim("text");
    s.run("key ctrl+shift+p");
    assert!(
        s.editor.window_view().palette.is_some(),
        "the palette opens"
    );
}

#[test]
fn the_status_bar_shows_the_mode_and_keys_so_far() {
    let mut s = vim("text");
    s.run("type 2d");
    let view = s.editor.window_view().vim.expect("vim");
    assert_eq!(view.pending, "2d");
    assert!(
        s.editor
            .window_view()
            .status
            .iter()
            .any(|i| i.text == "Keymap: Vim")
    );
}

#[test]
fn turning_it_off_types_as_before() {
    let mut s = vim("x");
    s.run("key ctrl+shift+p; type toggle vim; key enter");
    assert!(s.editor.window_view().vim.is_none());
    s.run("type ab");
    assert_eq!(s.text(), "abx");
}

#[test]
fn several_cursors_each_follow_the_keys() {
    let mut s = vim("one two\none two");
    s.run("key shift+alt+down; type dw");
    assert_eq!(s.text(), "two\ntwo");
}
