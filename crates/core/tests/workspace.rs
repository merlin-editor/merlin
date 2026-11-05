//! Bringing back the open tabs, groups and cursors of the last session in a
//! folder.

use std::path::PathBuf;

use merlin_core::script::Session;
use merlin_core::{Event, Workspace};

fn text() -> String {
    (1..=80).map(|i| format!("line {i}\n")).collect()
}

fn session() -> Session {
    let mut s = Session::new(&text(), Some("a.txt"), 100, 24);
    s.files.insert("a.txt".into(), text());
    s.files.insert("b.txt".into(), "bee\nbuzz\n".into());
    s
}

/// A new editor that opens the files `ws` names and restores it.
fn restored(ws: &Workspace) -> Session {
    let mut s = Session::new("", None, 100, 24);
    for path in ws.files() {
        let text = if path == *"a.txt" {
            text()
        } else {
            "bee\nbuzz\n".into()
        };
        s.send(Event::Opened { path, text });
    }
    s.send(Event::Restore(ws.clone()));
    s
}

#[test]
fn groups_tabs_and_cursors_come_back() {
    let mut s = session();
    s.run("key ctrl+g; type 40; key enter; key right; key right");
    s.run("cmd view.split; key ctrl+p; type b.txt; key enter; key down");
    s.run("cmd view.split_down; key ctrl+p; type a.txt; key enter");
    s.run("key ctrl+g; type 70; key enter");
    s.run("key ctrl+1");
    let ws = s.editor.workspace();
    assert_eq!(ws.groups.len(), 3);
    assert_eq!(ws.focused, 0);
    assert!(ws.groups[2].below);

    let again = Workspace::parse(&ws.to_text());
    assert_eq!(again, ws, "{}", ws.to_text());

    let mut r = restored(&again);
    assert_eq!(r.screen(), s.screen());
    r.run("key ctrl+2");
    assert_eq!(r.text(), "bee\nbuzz\n");
    assert!(r.screen().contains("Ln 2, Col 1"), "{}", r.screen());
    r.run("key ctrl+3");
    assert!(r.screen().contains("Ln 70, Col 1"), "{}", r.screen());
}

#[test]
fn missing_files_and_bad_lines_are_skipped() {
    let ws =
        Workspace::parse("focus 0\ngroup right 1\ntab 2:0 0 a.txt\ntab 0:0 0 gone.txt\nnonsense\n");
    let mut r = Session::new("", None, 100, 24);
    r.send(Event::Opened {
        path: "a.txt".into(),
        text: text(),
    });
    r.send(Event::Restore(ws));
    assert_eq!(r.editor.open_files(), [Some(PathBuf::from("a.txt"))]);
    assert!(r.screen().contains("Ln 3, Col 1"), "{}", r.screen());
}

#[test]
fn a_restored_group_keeps_its_share_beside_a_new_one() {
    let mut s = session();
    s.run("key ctrl+p; type b.txt; key enter");
    let mut s = restored(&s.editor.workspace());
    s.run("cmd view.split");
    let shares: Vec<f32> = s
        .editor
        .window_view()
        .groups
        .iter()
        .map(|g| g.width)
        .collect();
    assert_eq!(shares, [1.0, 1.0], "even, not all for the new one");
}
