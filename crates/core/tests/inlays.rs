//! Inlay hints: language servers' types and parameter names written
//! between the characters, moving the text after them along.

use merlin_core::script::Session;
use merlin_core::{Event, InlayHint, InlayKind, Settings};

fn session() -> Session {
    Session::new("let mix = 1;\nlet b = f(mix);\n", Some("main.rs"), 60, 8)
}

fn hints(s: &mut Session) {
    s.send(Event::SetInlayHints {
        path: "main.rs".into(),
        hints: vec![
            InlayHint {
                at: (0, 7),
                label: ": i32".into(),
                kind: InlayKind::Type,
            },
            InlayHint {
                at: (1, 10),
                label: "x:".into(),
                kind: InlayKind::Parameter,
            },
        ],
    });
}

/// The primary caret's column on screen, from the text's left.
fn caret_x(s: &Session) -> u16 {
    let views = s.editor.text_views();
    views.last().unwrap().carets[0].cell.unwrap().0
}

fn inlays(s: &Session, line: usize) -> Vec<(usize, usize, String)> {
    let views = s.editor.text_views();
    let view = views.last().unwrap();
    view.lines
        .iter()
        .find(|l| l.line == line)
        .map(|l| l.inlays.iter().map(|i| (i.col, i.cols, i.label.clone())).collect())
        .unwrap_or_default()
}

#[test]
fn a_hint_takes_room_and_the_text_after_it_moves_along() {
    let mut s = session();
    hints(&mut s);
    // ": i32" is five columns, and a chip a column wider.
    assert_eq!(inlays(&s, 0), vec![(7, 6, ": i32".into())]);
    s.run("key right; key right; key right; key right; key right; key right; key right");
    assert_eq!(caret_x(&s), 7, "after `mix`, before the hint");
    s.run("key right");
    assert_eq!(caret_x(&s), 14, "past the hint and the space");
    s.run("key end");
    assert_eq!(caret_x(&s), 18);
}

#[test]
fn a_hint_moves_with_edits_before_it() {
    let mut s = session();
    hints(&mut s);
    s.run("key down; type a");
    assert_eq!(inlays(&s, 1), vec![(11, 3, "x:".into())]);
    assert_eq!(inlays(&s, 0), vec![(7, 6, ": i32".into())]);
}

#[test]
fn turning_hints_off_takes_them_away() {
    let mut s = session();
    hints(&mut s);
    s.send(Event::Settings(Settings {
        inlay_hints: false,
        ..Default::default()
    }));
    assert!(inlays(&s, 0).is_empty());
    hints(&mut s);
    assert!(inlays(&s, 0).is_empty(), "and new ones don't show");
    s.run("key end");
    assert_eq!(caret_x(&s), 12);
}
