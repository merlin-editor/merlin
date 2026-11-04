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
    let start = buf.version();
    for (i, c) in ["a", "b"].iter().enumerate() {
        buf.begin(i, Kind::Typing);
        buf.apply(insert(i, c));
        buf.commit(i + 1);
    }
    assert_eq!(buf.revision(), 1, "typing merged into one revision");
    assert_eq!(
        buf.version() - start,
        2,
        "but each keystroke is a new version"
    );
}

#[test]
fn changes_are_kept_as_language_servers_count_them() {
    let mut buf = Buffer::new("ab\ncd\n", None);
    let start = buf.version();
    buf.begin(0, Kind::Other);
    // "é" is one UTF-16 unit, "😀" two.
    buf.apply(insert(3, "é😀"));
    buf.commit(5);
    buf.begin(0, Kind::Other);
    buf.apply(Edit {
        at: 5,
        removed: "c".into(),
        inserted: "X".into(),
    });
    buf.commit(6);
    let text = buf.snapshot();
    let changes = text.changes_since(start).expect("kept");
    assert_eq!(changes.len(), 2);
    assert_eq!((changes[0].start, changes[0].end), ((1, 0), (1, 0)));
    // After "é😀": three units in.
    assert_eq!((changes[1].start, changes[1].end), ((1, 3), (1, 4)));
    assert_eq!(changes[1].text, "X");
    let version = text.version().unwrap();
    assert_eq!(text.changes_since(version).unwrap(), []);
    // Another buffer's version, or one to come, isn't one of its.
    let other = Buffer::new("", None).snapshot().version().unwrap();
    assert!(text.changes_since(other).is_none());
    assert!(text.changes_since(version + 1).is_none());
    // Undo is one version of the edits it reverts.
    buf.undo();
    let after = buf.snapshot();
    assert_eq!(after.changes_since(version).unwrap().len(), 1);
    // Text made up for a test has no versions.
    assert!(
        merlin_core::buffer::TextSnapshot::from("x")
            .changes_since(0)
            .is_none()
    );
    assert_eq!(after.to_string(), "ab\né😀cd\n");
}
