//! Ctrl over a name: the name as a link to its definition, the definition
//! in a card after a rest, and Ctrl+click to go there.

use merlin_core::lang::Location;
use merlin_core::script::Session;
use merlin_core::style::Hue;
use merlin_core::{Effect, Event, MouseKind, PopupKind};

/// `shade` defined on line 0 and used on line 5, with a language server
/// that knows where definitions are, and another file that defines `tint`.
fn linking() -> Session {
    let mut s = Session::new(
        "fn shade(x: u8) -> u8 {\n    x + 1\n}\n\nfn main() {\n    shade(1);\n    tint(2);\n}\n",
        Some("main.rs"),
        60,
        30,
    );
    s.send(Event::Viewports(vec![(60, 30)]));
    s.send(Event::RegisterPlugin {
        plugin: "lsp".into(),
        commands: ["definition", "references", "hover"]
            .map(|id| merlin_core::Command {
                id: id.into(),
                title: id.into(),
                key: None,
            })
            .into(),
    });
    s.files.insert(
        "theme.rs".into(),
        "/// Tints it.\npub fn tint(x: u8) -> u8 {\n    x\n}\n".into(),
    );
    s.effects.clear();
    s
}

/// The mouse at text column `col` of `row`, with Ctrl and Alt as given.
fn mouse(s: &mut Session, kind: MouseKind, (row, col): (u16, u16), ctrl: bool, alt: bool) {
    let (x, y) = s.editor.text_origin();
    s.send(Event::Pointer(merlin_core::Pointer {
        group: 0,
        kind,
        row: (y + row) as f32,
        col: (x + col) as f32 + 0.2,
        shift: false,
        alt,
        ctrl,
        clicks: 1,
    }));
}

fn asked(s: &Session, command: &str) -> bool {
    s.effects
        .iter()
        .any(|e| matches!(e, Effect::PluginCommand { command: c, .. } if c == command))
}

fn at(path: &str, line: u32, col: u32, len: u32) -> Location {
    Location {
        path: path.into(),
        start: (line, col),
        end: (line, col + len),
    }
}

/// Holds Ctrl over `shade` on line 5 until the server's asked.
fn over_shade(s: &mut Session) {
    mouse(s, MouseKind::Move, (5, 6), true, false);
    let (tick, _) = s.editor.link_wait().expect("waits to ask");
    s.send(Event::LinkDue(tick));
}

fn linked(s: &Session, row: u16, col: u16) -> bool {
    let (x, y) = s.editor.text_origin();
    s.cell(x + col, y + row).unwrap().tint == Some(Hue::Link)
}

#[test]
fn ctrl_over_a_name_asks_after_a_stop_and_underlines_it_as_a_link() {
    let mut s = linking();
    over_shade(&mut s);
    assert!(asked(&s, "definition-link"));
    // The plugin asks about the name under the pointer, not the cursor.
    assert_eq!(s.editor.cursor_utf16(), (5, 4));
    assert!(!linked(&s, 5, 6), "not until the server says");
    s.send(Event::Definitions(vec![at("main.rs", 0, 3, 5)]));
    assert!((4..9).all(|c| linked(&s, 5, c)), "all of `shade`");
    assert!(!linked(&s, 5, 9), "not the bracket after it");
    assert!(s.editor.window_view().pointer_link);
}

#[test]
fn moving_along_the_same_name_asks_nothing_more() {
    let mut s = linking();
    over_shade(&mut s);
    s.send(Event::Definitions(vec![at("main.rs", 0, 3, 5)]));
    s.effects.clear();
    mouse(&mut s, MouseKind::Move, (5, 8), true, false);
    assert_eq!(s.editor.link_wait(), None);
    assert!(linked(&s, 5, 6));
}

#[test]
fn a_name_with_no_definition_and_letting_go_of_ctrl_show_no_link() {
    let mut s = linking();
    over_shade(&mut s);
    s.send(Event::Definitions(Vec::new()));
    // Asked once more, later, in case the server was busy.
    let (tick, ms) = s.editor.link_wait().expect("asks again");
    assert!(ms > 100);
    s.send(Event::LinkDue(tick));
    s.send(Event::Definitions(Vec::new()));
    assert_eq!(s.editor.link_wait(), None, "but only once");
    assert!(!linked(&s, 5, 6));
    assert!(!s.editor.window_view().pointer_link);
    // Off the name and back, it's asked again; this time it has one.
    mouse(&mut s, MouseKind::Move, (5, 1), true, false);
    over_shade(&mut s);
    s.send(Event::Definitions(vec![at("main.rs", 0, 3, 5)]));
    assert!(linked(&s, 5, 6));
    mouse(&mut s, MouseKind::Move, (5, 6), false, false);
    assert!(!linked(&s, 5, 6), "Ctrl let go");
}

#[test]
fn without_a_language_server_ctrl_changes_nothing() {
    let mut s = Session::new("fn a() {}\n", Some("main.rs"), 60, 8);
    s.send(Event::Viewports(vec![(60, 8)]));
    mouse(&mut s, MouseKind::Move, (0, 3), true, false);
    assert_eq!(s.editor.link_wait(), None);
}

#[test]
fn ctrl_click_goes_to_the_definition() {
    let mut s = linking();
    over_shade(&mut s);
    s.send(Event::Definitions(vec![at("main.rs", 0, 3, 5)]));
    mouse(&mut s, MouseKind::Down, (5, 6), true, false);
    assert_eq!(s.editor.cursor_utf16(), (0, 3));
}

#[test]
fn ctrl_click_before_the_answer_goes_when_it_comes() {
    let mut s = linking();
    mouse(&mut s, MouseKind::Down, (6, 6), true, false);
    assert!(asked(&s, "definition-link"));
    s.send(Event::Definitions(vec![at("theme.rs", 1, 7, 4)]));
    let view = s.editor.window_view();
    let tab = view.groups[0].tabs.iter().find(|t| t.active).unwrap();
    assert_eq!(tab.name, "theme.rs");
    assert_eq!(s.editor.cursor_utf16(), (1, 7));
}

#[test]
fn ctrl_alt_click_opens_it_to_the_side() {
    let mut s = linking();
    over_shade(&mut s);
    s.send(Event::Definitions(vec![at("theme.rs", 1, 7, 4)]));
    mouse(&mut s, MouseKind::Down, (5, 6), true, true);
    let view = s.editor.window_view();
    assert_eq!(view.groups.len(), 2, "split");
    assert!(view.groups[1].focused);
}

#[test]
fn ctrl_click_on_a_definition_finds_its_references() {
    let mut s = linking();
    mouse(&mut s, MouseKind::Move, (0, 4), true, false);
    let (tick, _) = s.editor.link_wait().unwrap();
    s.send(Event::LinkDue(tick));
    s.send(Event::Definitions(vec![at("main.rs", 0, 3, 5)]));
    s.effects.clear();
    mouse(&mut s, MouseKind::Down, (0, 4), true, false);
    assert!(asked(&s, "references"));
}

#[test]
fn several_definitions_open_in_the_peek_view() {
    let mut s = linking();
    over_shade(&mut s);
    s.send(Event::Definitions(vec![
        at("main.rs", 0, 3, 5),
        at("theme.rs", 1, 7, 4),
    ]));
    mouse(&mut s, MouseKind::Down, (5, 6), true, false);
    let peek = s.editor.text_views().into_iter().find_map(|v| v.peek);
    assert!(peek.is_some(), "{}", s.editor.dump());
}

#[test]
fn a_rest_with_ctrl_shows_the_definition_in_a_card() {
    let mut s = linking();
    mouse(&mut s, MouseKind::Move, (6, 5), true, false);
    let (tick, _) = s.editor.link_wait().unwrap();
    s.send(Event::LinkDue(tick));
    s.send(Event::Definitions(vec![at("theme.rs", 1, 7, 4)]));
    mouse(&mut s, MouseKind::Rest, (6, 5), true, false);
    let Some(PopupKind::Definition {
        file,
        line,
        first,
        lines,
        ..
    }) = s.editor.window_view().popup.map(|p| p.kind)
    else {
        panic!("{}", s.editor.dump());
    };
    assert_eq!((file.as_str(), line, first), ("theme.rs", 2, 1));
    assert_eq!(lines[0], "/// Tints it.", "with its doc comment");
    assert_eq!(lines[1], "pub fn tint(x: u8) -> u8 {");
    // Off the name, it goes.
    mouse(&mut s, MouseKind::Move, (6, 12), true, false);
    assert!(s.editor.window_view().popup.is_none());
}

#[test]
fn a_rest_without_ctrl_is_the_usual_hover() {
    let mut s = linking();
    mouse(&mut s, MouseKind::Rest, (5, 6), false, false);
    assert!(asked(&s, "hover-rest"));
    assert!(!asked(&s, "definition-link"));
}
