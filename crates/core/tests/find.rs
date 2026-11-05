//! The find box's toggles: Match Case, Whole Word, regular expressions
//! and Find in Selection.

use merlin_core::script::Session;
use merlin_core::{Event, FindAction, FindView, UiEvent};

fn session(text: &str) -> Session {
    Session::new(text, Some("a.txt"), 60, 8)
}

fn find(s: &Session) -> FindView {
    s.editor.window_view().find.expect("the find box is open")
}

fn ui(s: &mut Session, action: FindAction) {
    s.send(Event::Ui(UiEvent::Find(action)));
}

#[test]
fn match_case_turns_off_ignoring_case() {
    let mut s = session("Cat cat CAT");
    s.run("key ctrl+f; type cat");
    assert_eq!(find(&s).total, 3);
    s.run("key alt+c");
    let view = find(&s);
    assert!(view.match_case && !view.ignore_case);
    assert_eq!(view.total, 1);
    assert_eq!(s.editor.cursor(), 4);
    ui(&mut s, FindAction::MatchCase);
    assert_eq!(find(&s).total, 3, "the button toggles it back");
}

#[test]
fn whole_word_skips_parts_of_words() {
    let mut s = session("cat concat cat_x cats cat");
    s.run("key ctrl+f; type cat; key alt+w");
    assert_eq!(find(&s).total, 2);
    s.run("key enter");
    assert_eq!(
        s.editor.cursor(),
        22,
        "the last one, past the parts of words"
    );
}

#[test]
fn a_regular_expression_matches_and_replaces_with_groups() {
    let mut s = session("a1 b22 c333");
    s.run("key ctrl+h; type [a-z](\\\\d+); key alt+r");
    assert!(find(&s).regex);
    assert_eq!(find(&s).total, 3);
    s.run("key tab; type <$1>; key alt+enter");
    assert_eq!(s.text(), "<1> <22> <333>");
    s.run("key esc ctrl+z");
    assert_eq!(s.text(), "a1 b22 c333", "one step to undo");
}

#[test]
fn a_regular_expression_with_whole_words_and_case() {
    let mut s = session("Foo foo food");
    s.run("key ctrl+f; type fo+; key alt+r");
    assert_eq!(find(&s).total, 3, "case ignored for a lowercase query");
    s.run("key alt+w");
    assert_eq!(find(&s).total, 2);
    s.run("key alt+c");
    assert_eq!(find(&s).total, 1);
}

#[test]
fn a_broken_regular_expression_says_so_and_matches_nothing() {
    let mut s = session("a(b");
    s.run("key ctrl+f; type a(; key alt+r");
    let view = find(&s);
    assert!(view.invalid);
    assert_eq!(view.total, 0);
    s.run("key alt+r");
    assert!(!find(&s).invalid, "as plain text it's fine");
    assert_eq!(find(&s).total, 1);
}

#[test]
fn find_in_selection_searches_only_the_selection() {
    let mut s = session("x\nx\nx\nx\n");
    // Select the middle two lines; a selection over lines turns it on.
    s.run("key down shift+down shift+down; key ctrl+f");
    assert!(find(&s).in_selection);
    s.run("type x");
    assert_eq!(find(&s).total, 2);
    s.run("key tab");
    s.run("key alt+l");
    assert!(!find(&s).in_selection);
    assert_eq!(find(&s).total, 4);
}

#[test]
fn replace_all_in_selection_leaves_the_rest() {
    let mut s = session("a a\na a\na a\n");
    s.run("key down shift+down shift+end; key ctrl+h; type a; key tab; type b; key alt+enter");
    assert_eq!(s.text(), "a a\nb b\nb b\n");
}

#[test]
fn the_range_moves_with_edits_before_it() {
    let mut s = session("x\ny y\nx\n");
    s.run("key down shift+down; key ctrl+f; type y");
    assert_eq!(find(&s).total, 2);
    // Back in the code, with the box still open, type above the range.
    s.run("click 10 0; key ctrl+home; type zz");
    assert!(find(&s).in_selection);
    assert_eq!(find(&s).total, 2, "both still inside it");
}
