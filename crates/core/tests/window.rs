//! A window that draws its own frame: the screen is only the text, the
//! frame comes from the window view, and clicks on it arrive as UI events.

use merlin_core::script::Session;
use merlin_core::{Event, Focus, Node, Side, UiEvent};

fn windowed() -> Session {
    let mut s = Session::new("fn a() {\n    b(1);\n}\n", Some("src/main.rs"), 80, 20);
    s.files.insert("src/main.rs".into(), s.text());
    s.files
        .insert("src/lib.rs".into(), "pub fn b() {}\n".into());
    s.editor.set_windowed(true);
    s.send(Event::Resize(80, 20));
    s
}

#[test]
fn the_screen_is_only_the_text() {
    let s = windowed();
    let screen = s.screen();
    let first = screen.lines().next().unwrap_or_default();
    assert!(first.contains("fn a()"), "the first row is code:\n{screen}");
    assert!(!screen.contains("File"), "no menu bar:\n{screen}");
    assert!(!screen.contains("Ln 1"), "no status bar:\n{screen}");
}

#[test]
fn the_view_has_tabs_status_and_rail() {
    let s = windowed();
    let view = s.editor.window_view();
    assert_eq!(view.tabs.len(), 1);
    assert_eq!(view.tabs[0].name, "main.rs");
    assert!(view.tabs[0].active);
    assert!(view.status.iter().any(|i| i.right && i.text.starts_with("Ln 1")));
    let labels: Vec<_> = view.rail.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["Explorer", "Search", "Source Control", "Outline"]);
}

#[test]
fn rail_clicks_show_and_hide_views() {
    let mut s = windowed();
    s.send(Event::Ui(UiEvent::Rail(1)));
    let view = s.editor.window_view();
    let left = view.left.expect("the side bar shows");
    assert!(view.rail[1].active);
    assert!(left.nodes.iter().any(|n| matches!(n, Node::Input { .. })));
    assert!(left.focused);
    s.send(Event::Ui(UiEvent::Rail(1)));
    assert!(s.editor.window_view().left.is_none(), "a second click hides it");
}

#[test]
fn explorer_rows_open_files() {
    let mut s = windowed();
    s.send(Event::Ui(UiEvent::Rail(0)));
    s.send(Event::Tree(vec!["src/main.rs".into(), "src/lib.rs".into()]));
    let left = s.editor.window_view().left.expect("the Explorer shows");
    // The folder opens to show the current file; lib.rs is the row before it.
    let row = left
        .nodes
        .iter()
        .position(|n| matches!(n, Node::Entry(e) if e.label == "lib.rs"))
        .expect("lib.rs is listed");
    let index = left.targets[row].expect("rows are clickable");
    s.send(Event::Ui(UiEvent::Target {
        side: Side::Left,
        index,
    }));
    let view = s.editor.window_view();
    assert!(view.tabs.iter().any(|t| t.name == "lib.rs" && t.active));
}

#[test]
fn tabs_switch_and_close() {
    let mut s = windowed();
    s.send(Event::Command("file.open".into()));
    s.run("type lib.rs\nkey enter");
    assert_eq!(s.editor.window_view().tabs.len(), 2);
    s.send(Event::Ui(UiEvent::Tab {
        index: 0,
        clicks: 1,
    }));
    assert!(s.editor.window_view().tabs[0].active);
    s.send(Event::Ui(UiEvent::CloseTab(0)));
    let tabs = s.editor.window_view().tabs;
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].name, "lib.rs");
}

#[test]
fn the_palette_lists_commands_and_runs_a_clicked_one() {
    let mut s = windowed();
    s.send(Event::Command("palette.open".into()));
    s.run("type toggle word wrap");
    let palette = s.editor.window_view().palette.expect("the palette is open");
    assert_eq!(palette.kind, "Commands");
    assert_eq!(palette.query, "toggle word wrap");
    let row = palette
        .items
        .iter()
        .position(|(label, _)| label.contains("Word Wrap"))
        .expect("word wrap is listed");
    s.send(Event::Ui(UiEvent::PaletteRow(row)));
    assert!(s.editor.window_view().palette.is_none());
    assert!(s.editor.wrap());
    assert_eq!(s.editor.focus(), Focus::Editor);
}
