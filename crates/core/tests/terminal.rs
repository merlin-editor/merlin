//! The integrated terminal: opening shells, splitting, where keys and
//! pastes go, and shells ending.

use merlin_core::script::Session;
use merlin_core::{Effect, Event, Focus, Key, TermOp, UiEvent};

fn session() -> Session {
    Session::new("fn a() {}\n", Some("a.rs"), 80, 20)
}

fn ops(s: &Session) -> Vec<TermOp> {
    s.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Terminal(op) => Some(op.clone()),
            _ => None,
        })
        .collect()
}

fn key(spec: &str) -> Key {
    Key::parse(spec).unwrap()
}

#[test]
fn ctrl_backtick_opens_a_shell_with_the_keys() {
    let mut s = session();
    s.run("key ctrl+`");
    assert_eq!(
        ops(&s),
        vec![TermOp::Spawn {
            id: 1,
            command: None
        }]
    );
    let view = s.editor.window_view();
    let term = view.terminal.expect("the terminal panel shows");
    assert_eq!(term.panes, vec![1]);
    assert!(term.focused);
    assert_eq!(view.focus, Focus::Bottom);
    // Typing goes to the shell, not the file.
    s.run("type l; key ctrl+c");
    assert_eq!(s.text(), "fn a() {}\n");
    assert!(ops(&s).contains(&TermOp::Key {
        id: 1,
        key: key("l")
    }));
    assert!(ops(&s).contains(&TermOp::Key {
        id: 1,
        key: key("ctrl+c")
    }));
}

#[test]
fn the_workbench_keeps_a_few_keys() {
    let mut s = session();
    s.run("key ctrl+`; key ctrl+shift+p");
    assert!(
        s.editor.window_view().palette.is_some(),
        "the palette opens"
    );
    // Ctrl+` gives a shown terminal the keys, and with them, hides it.
    s.run("key esc; key ctrl+`");
    assert!(s.editor.window_view().terminal.unwrap().focused);
    s.run("key ctrl+`");
    assert!(s.editor.window_view().terminal.is_none());
}

#[test]
fn ctrl_backtick_again_shows_the_same_shell() {
    let mut s = session();
    s.run("key ctrl+`; key ctrl+`; key ctrl+`");
    assert_eq!(
        ops(&s),
        vec![TermOp::Spawn {
            id: 1,
            command: None
        }],
        "only one shell started"
    );
    assert!(s.editor.window_view().terminal.unwrap().focused);
}

#[test]
fn splitting_shows_two_side_by_side() {
    let mut s = session();
    s.run("key ctrl+`; key ctrl+shift+5");
    let term = s.editor.window_view().terminal.unwrap();
    assert_eq!(term.panes, vec![1, 2]);
    assert_eq!(term.active, Some(2));
    // A new terminal, not split, shows on its own.
    s.run("key ctrl+shift+`");
    let term = s.editor.window_view().terminal.unwrap();
    assert_eq!(term.panes, vec![3]);
    assert_eq!(term.sessions.len(), 3);
    // Picking one in the list shows its split again.
    s.send(Event::Ui(UiEvent::Terminal(1)));
    assert_eq!(s.editor.window_view().terminal.unwrap().panes, vec![1, 2]);
}

#[test]
fn pasting_goes_to_the_shell() {
    let mut s = session();
    s.run("key ctrl+`");
    s.send(Event::Paste("echo hi".into()));
    assert!(ops(&s).contains(&TermOp::Paste {
        id: 1,
        text: "echo hi".into()
    }));
    assert_eq!(s.text(), "fn a() {}\n");
}

#[test]
fn a_shell_ending_stays_until_killed() {
    let mut s = session();
    s.run("key ctrl+`; key ctrl+shift+5");
    s.send(Event::TermTitle {
        id: 2,
        title: "htop".into(),
    });
    s.send(Event::TermExited {
        id: 2,
        code: Some(0),
    });
    let term = s.editor.window_view().terminal.unwrap();
    assert_eq!(term.sessions[1].title, "htop");
    assert_eq!(term.sessions[1].exited, Some(0));
    s.run("cmd terminal.kill");
    assert!(ops(&s).contains(&TermOp::Kill { id: 2 }));
    let term = s.editor.window_view().terminal.unwrap();
    assert_eq!(term.panes, vec![1]);
    assert_eq!(term.active, Some(1));
    // Killing the last hides the panel and gives the keys back to the code.
    s.run("cmd terminal.kill");
    assert!(s.editor.window_view().terminal.is_none());
    assert_eq!(s.editor.window_view().focus, Focus::Editor);
}

#[test]
fn the_panel_tab_opens_a_shell_when_there_is_none() {
    let mut s = session();
    s.send(Event::Ui(UiEvent::PanelTitle {
        side: merlin_core::Side::Bottom,
        name: "merlin.terminal".into(),
    }));
    assert_eq!(
        ops(&s),
        vec![TermOp::Spawn {
            id: 1,
            command: None
        }]
    );
}
