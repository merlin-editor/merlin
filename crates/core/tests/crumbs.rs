//! Breadcrumbs, sticky scroll and the symbols they show.

use merlin_core::script::Session;

fn code() -> String {
    let mut text = String::from("struct Point {\n    x: i32,\n}\n\nimpl Point {\n");
    text += "    fn far(&self) -> bool {\n";
    for i in 0..40 {
        text += &format!("        let v{i} = {i};\n");
    }
    text += "        true\n    }\n}\n";
    text
}

fn session() -> Session {
    let mut s = Session::new(&code(), Some("src/geo.rs"), 80, 20);
    s.editor.set_breadcrumbs(true);
    s.editor.set_sticky_scroll(true);
    s
}

/// The focused group's breadcrumbs.
fn crumbs(s: &Session) -> Vec<String> {
    let view = s.editor.window_view();
    view.groups.into_iter().find(|g| g.focused).unwrap().crumbs
}

#[test]
fn breadcrumbs_show_the_path_and_the_symbols_around_the_cursor() {
    let mut s = session();
    let joined = |s: &Session| crumbs(s).join(" › ");
    assert!(
        joined(&s).contains("geo.rs › struct Point"),
        "{}",
        joined(&s)
    );
    assert!(s.screen().contains("crumbs: src › "), "{}", s.screen());
    s.run("key ctrl+g; type 20; key enter");
    assert!(
        joined(&s).contains("geo.rs › impl Point › fn far"),
        "{}",
        joined(&s)
    );
    // Clicking a symbol goes to it.
    let index = crumbs(&s)
        .iter()
        .position(|c| c.contains("impl Point"))
        .unwrap();
    s.send(merlin_core::Event::Ui(merlin_core::UiEvent::GroupCrumb {
        group: 0,
        index,
    }));
    assert!(s.screen().contains("Ln 5, Col 1"), "{}", s.screen());
}

#[test]
fn sticky_scroll_pins_the_blocks_in_view() {
    let mut s = session();
    s.run("key ctrl+g; type 40; key enter");
    let view = s.editor.text_views().pop().unwrap();
    let pinned: Vec<String> = view
        .sticky
        .iter()
        .map(|l| l.chars.iter().collect::<String>())
        .collect();
    assert_eq!(pinned, ["impl Point {", "    fn far(&self) -> bool {"]);
    // Clicking a pinned line goes there.
    s.run("click 20 0");
    assert!(s.screen().contains("Ln 5, Col 1"), "{}", s.screen());
}

#[test]
fn sticky_scroll_never_covers_the_cursor() {
    let mut s = session();
    s.run("key ctrl+g; type 40; key enter");
    // Scroll down until the cursor leaves the view: on every row it passes
    // through, it's below the pinned lines.
    let mut seen = 0;
    for _ in 0..20 {
        s.run("wheel down");
        let Some((_, y)) = s.cursor() else {
            break;
        };
        let pinned = s.editor.text_views().pop().unwrap().sticky.len();
        assert!(y as usize >= pinned, "{}", s.screen());
        seen += 1;
    }
    assert!(seen > 0);
}

#[test]
fn no_breadcrumbs_unless_turned_on() {
    let s = Session::new(&code(), Some("src/geo.rs"), 80, 20);
    assert!(!s.screen().contains("geo.rs ›"));
}

#[test]
fn the_outline_lists_symbols_and_goes_to_them() {
    let mut s = Session::new(&code(), Some("src/geo.rs"), 100, 30);
    s.run("cmd view.outline");
    let screen = s.screen();
    assert!(screen.contains("── left: Outline"), "{screen}");
    assert!(screen.contains("Point  struct"), "{screen}");
    assert!(screen.contains("  impl Point"), "{screen}");
    assert!(screen.contains("    far  fn"), "{screen}");
    s.run("key down; key down; key enter");
    assert!(s.screen().contains("Ln 6, Col 5"), "{}", s.screen());
}

#[test]
fn go_to_symbol_filters_and_jumps() {
    let mut s = session();
    s.run("key ctrl+shift+o; type far; key enter");
    assert!(s.screen().contains("Ln 6, Col 5"), "{}", s.screen());
}

#[test]
fn markdown_headings_are_symbols() {
    let text = "# Title\n\nIntro.\n\n## Usage\n\n```sh\n# not a heading\n```\n\n## Tests\n";
    let mut s = Session::new(text, Some("README.md"), 100, 30);
    s.run("cmd view.outline");
    // The side bar, before the text.
    let screen = s.screen();
    let side = &screen[..screen.find("── group").unwrap()];
    assert!(side.contains("Title"), "{screen}");
    assert!(side.contains("Usage") && side.contains("Tests"), "{screen}");
    assert!(!side.contains("not a heading"), "{screen}");
}
