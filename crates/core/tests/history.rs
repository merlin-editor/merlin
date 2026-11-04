use merlin_core::buffer::Buffer;
use merlin_core::edit::Edit;
use merlin_core::history::Kind;

fn insert(at: usize, text: &str) -> Edit {
    Edit {
        at,
        removed: String::new(),
        inserted: text.into(),
    }
}

#[test]
fn nested_transactions_are_one_undo_step() {
    // ttt#1252: nested transactions broke the outer undo group.
    let mut buf = Buffer::new("", None);
    buf.begin(0, Kind::Other);
    buf.apply(insert(0, "a"));
    buf.begin(1, Kind::Other);
    buf.apply(insert(1, "b"));
    buf.commit(2);
    buf.apply(insert(2, "c"));
    buf.commit(3);
    assert_eq!(buf.text(), "abc");
    assert_eq!(buf.undo().map(|c| c[0].cursor), Some(0));
    assert_eq!(buf.text(), "");
    assert_eq!(buf.undo(), None);
}

#[test]
fn undo_is_refused_inside_a_transaction() {
    let mut buf = Buffer::new("x", None);
    buf.begin(0, Kind::Other);
    buf.apply(insert(0, "a"));
    assert_eq!(buf.undo(), None);
    buf.commit(1);
    assert_eq!(buf.undo().map(|c| c[0].cursor), Some(0));
    assert_eq!(buf.text(), "x");
}

#[test]
fn undo_and_redo_round_trip_replacements() {
    let mut buf = Buffer::new("hello world", None);
    buf.begin(6, Kind::Other);
    buf.apply(Edit {
        at: 6,
        removed: "world".into(),
        inserted: "there".into(),
    });
    buf.commit(11);
    buf.undo();
    assert_eq!(buf.text(), "hello world");
    assert_eq!(buf.redo().map(|c| c[0].cursor), Some(11));
    assert_eq!(buf.text(), "hello there");
}

#[test]
fn version_changes_even_when_typing_merges() {
    let mut buf = Buffer::new("", None);
    for (i, c) in ["a", "b"].iter().enumerate() {
        buf.begin(i, Kind::Typing);
        buf.apply(insert(i, c));
        buf.commit(i + 1);
    }
    assert_eq!(buf.revision(), 1, "typing merged into one revision");
    assert_eq!(buf.version(), 2, "but each keystroke is a new version");
}
