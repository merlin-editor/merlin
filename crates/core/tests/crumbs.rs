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

#[test]
fn breadcrumbs_show_the_path_and_the_symbols_around_the_cursor() {
    let mut s = session();
    let row = |s: &Session| s.screen().lines().nth(2).unwrap().to_string();
    assert!(
        row(&s).contains("src › ◆ geo.rs › struct Point"),
        "{}",
        s.screen()
    );
    s.run("key ctrl+g; type 20; key enter");
    assert!(
        row(&s).contains("geo.rs › impl Point › fn far"),
        "{}",
        s.screen()
    );
    // Clicking a symbol goes to it.
    let r = row(&s);
    let x = r[..r.find("impl Point").unwrap()].chars().count();
    s.run(&format!("click {x} 2"));
    assert!(s.screen().contains("Ln 5, Col 1"), "{}", s.screen());
}

#[test]
fn sticky_scroll_pins_the_blocks_in_view() {
    let mut s = session();
    s.run("key ctrl+g; type 40; key enter");
    let screen = s.screen();
    let lines: Vec<&str> = screen.lines().collect();
    assert!(lines[3].contains("5 impl Point {"), "{screen}");
    assert!(
        lines[4].contains("6     fn far(&self) -> bool {"),
        "{screen}"
    );
    // Clicking a pinned line goes there.
    s.run("click 20 3");
    assert!(s.screen().contains("Ln 5, Col 1"), "{}", s.screen());
}

#[test]
fn sticky_scroll_never_covers_the_cursor() {
    let mut s = session();
    s.run("key ctrl+g; type 40; key enter");
    // Scroll down until the cursor leaves the view: on every row it passes
    // through, its line shows.
    let mut seen = 0;
    for _ in 0..20 {
        s.run("wheel down 30 10");
        let Some((_, y)) = s.editor.render().cursor else {
            break;
        };
        let screen = s.screen();
        let row = screen.lines().nth(y as usize).unwrap();
        assert!(row.contains("40 "), "{screen}");
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
    assert!(screen.contains("OUTLINE"), "{screen}");
    assert!(screen.contains("◇ Point struct"), "{screen}");
    assert!(screen.contains("{ impl Point"), "{screen}");
    assert!(screen.contains("ƒ far fn"), "{screen}");
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
    // The side bar: the first columns of each row.
    let screen: String = s
        .screen()
        .lines()
        .map(|l| l.chars().take(28).collect::<String>() + "\n")
        .collect();
    assert!(screen.contains("# Title"), "{screen}");
    assert!(
        screen.contains("# Usage") && screen.contains("# Tests"),
        "{screen}"
    );
    assert!(!screen.contains("not a heading"), "{screen}");
}
