//! Milestone 6: the parts around the text as VS Code has them, the activity
//! bar, title bar, panel, notices and status bar, and what's drawn over
//! code.

use merlin_core::script::Session;
use merlin_core::style::{Hue, Style};
use merlin_core::{Diagnostic, Event, Focus, Node, Severity, Side, UiEvent};

fn project() -> Session {
    let mut s = Session::new("fn a() {\n    b(1);\n}\n", Some("src/main.rs"), 100, 24);
    s.files.insert("src/main.rs".into(), s.text());
    s.files
        .insert("src/lib.rs".into(), "pub fn b() {}\n".into());
    s
}

/// The status bar item whose text holds `needle`, by index.
fn status(s: &Session, needle: &str) -> usize {
    let view = s.editor.window_view();
    view.status
        .iter()
        .position(|i| i.text.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} isn't in the status bar:\n{}", s.screen()))
}

fn ui(s: &mut Session, event: UiEvent) {
    s.send(Event::Ui(event));
}

fn problem(s: &mut Session) {
    s.send(Event::SetDiagnostics {
        plugin: "lsp".into(),
        path: "src/lib.rs".into(),
        diagnostics: vec![Diagnostic {
            start: (0, 7),
            end: (0, 9),
            severity: Severity::Error,
            message: "something is wrong".into(),
            ..Default::default()
        }],
    });
}

#[test]
fn the_activity_bar_switches_views_and_hides_the_one_showing() {
    let mut s = project();
    let search = s
        .editor
        .window_view()
        .rail
        .iter()
        .position(|r| r.label == "Search")
        .unwrap();
    ui(&mut s, UiEvent::Rail(search));
    assert!(s.screen().contains("── left: Search"), "{}", s.screen());
    ui(&mut s, UiEvent::Rail(search));
    assert!(!s.editor.side_bar_visible(), "a second click hides it");
}

#[test]
fn the_source_control_icon_counts_changes() {
    let mut s = project();
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![("src/main.rs".into(), 'M'), ("x".into(), '?')],
        staged: vec![],
        changed: vec![("src/main.rs".into(), 'M'), ("x".into(), '?')],
    });
    assert!(s.screen().contains("Source Control (2)"), "{}", s.screen());
}

#[test]
fn problems_list_by_file_and_open_at_their_line() {
    let mut s = project();
    problem(&mut s);
    s.run("key ctrl+shift+m");
    let screen = s.screen();
    assert!(screen.contains("── bottom: Problems"), "{screen}");
    assert!(screen.contains("lib.rs  src"), "{screen}");
    assert!(
        screen.contains("something is wrong  [Ln 1, Col 8]"),
        "{screen}"
    );
    assert_eq!(s.editor.focus(), Focus::Bottom);
    s.click_entry("something is wrong", 1);
    assert_eq!(s.text(), "pub fn b() {}\n");
    assert_eq!(s.editor.cursor_utf16(), (0, 7));
}

#[test]
fn ctrl_j_toggles_the_panel() {
    let mut s = project();
    s.run("key ctrl+j");
    assert!(s.editor.bottom_visible());
    assert!(
        s.screen().contains("No problems have been detected"),
        "{}",
        s.screen()
    );
    s.run("key ctrl+j");
    assert!(!s.editor.bottom_visible());
}

#[test]
fn messages_collect_in_output() {
    let mut s = project();
    s.run("key ctrl+k; key x");
    s.run("cmd view.output");
    assert!(
        s.screen()
            .contains("[info] The key combination (Ctrl+K, X) is not a command"),
        "{}",
        s.screen()
    );
}

#[test]
fn a_failed_plugin_offers_its_output() {
    let mut s = project();
    s.send(Event::PluginFailed {
        plugin: "lsp".into(),
        message: "exited".into(),
    });
    let toast = s.editor.window_view().toast.expect("a notice");
    assert_eq!(toast.buttons, ["Show Output"]);
    ui(&mut s, UiEvent::ToastButton(0));
    assert!(s.editor.bottom_visible());
    assert!(
        s.screen().contains("[error] Plugin lsp failed: exited"),
        "{}",
        s.screen()
    );
}

#[test]
fn the_status_bar_opens_problems_and_go_to_line() {
    let mut s = project();
    let problems = status(&s, "⊗ 0");
    ui(&mut s, UiEvent::Status(problems));
    assert!(s.editor.bottom_visible());
    let position = status(&s, "Ln 1, Col 1");
    ui(&mut s, UiEvent::Status(position));
    assert_eq!(s.editor.focus(), Focus::Palette);
}

#[test]
fn the_explorer_counts_problems_and_colours_names() {
    let mut s = project();
    problem(&mut s);
    s.run("key ctrl+shift+e");
    let left = s.editor.window_view().left.expect("the Explorer");
    let entry = left
        .nodes
        .iter()
        .find_map(|n| match n {
            Node::Entry(e) if e.label == "lib.rs" => Some(e.clone()),
            _ => None,
        })
        .expect("lib.rs");
    assert_eq!(entry.tone, merlin_core::Tone::Error);
    assert_eq!(entry.icon.map(|i| i.1), Some(Hue::Orange), "the Rust icon");
    assert_eq!(entry.badge, "1", "a count at the right");
    let _ = Side::Left;
}

#[test]
fn guides_brackets_and_the_word_under_the_cursor() {
    let mut s = Session::new(
        "fn a() {\n    if b(1) {\n        c();\n    }\n    c();\n}\n",
        Some("main.rs"),
        80,
        16,
    );
    // Brackets are coloured in files a language plugin colours.
    s.send(merlin_core::Event::SetTokens {
        path: "main.rs".into(),
        layer: merlin_core::layers::Layer::Base,
        lines: None,
        tokens: Vec::new(),
    });
    let line = |s: &Session, n: usize| s.editor.text_views().pop().unwrap().lines[n].clone();
    // Line 3, indented twice: a guide at each level, the outer one the
    // active block's, the fn the cursor is in.
    assert_eq!(line(&s, 2).guides[0], Some(true));
    assert_eq!(line(&s, 2).guides[4], Some(false));
    // `{` at depth 0 and `(` inside `if b(` at depth 1.
    assert_eq!(line(&s, 0).brackets[7], Some(Hue::Bracket1));
    assert_eq!(line(&s, 1).brackets[8], Some(Hue::Bracket2));
    s.run("key down down; key end; key left left left");
    // The cursor is on `c`; the other `c` is marked.
    assert_eq!(line(&s, 4).styles[4], Style::WordMatch);
    assert_eq!(line(&s, 1).styles[4], Style::Normal);
}
