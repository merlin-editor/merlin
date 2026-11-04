//! Editor groups: the editor split side by side, each group with its own
//! tabs, and a file open in two groups keeping a cursor in each.

use merlin_core::script::Session;

fn session() -> Session {
    let text: String = (1..=60).map(|i| format!("line {i}\n")).collect();
    let mut s = Session::new(&text, Some("a.txt"), 100, 20);
    s.files.insert("a.txt".into(), text);
    s.files.insert("b.txt".into(), "bee\n".into());
    s
}

fn tab_row(s: &Session) -> String {
    s.screen().lines().nth(1).unwrap().to_string()
}

#[test]
fn split_opens_the_file_again_to_the_right_with_its_own_cursor() {
    let mut s = session();
    s.run("cmd view.split");
    let tabs = tab_row(&s);
    assert_eq!(tabs.matches("a.txt").count(), 2, "{tabs}");
    // Moving in the right group leaves the left where it was.
    s.run("key ctrl+g; type 50; key enter");
    let screen = s.screen();
    let row = screen.lines().nth(2).unwrap();
    let (left, right) = row.split_at(row.find('│').unwrap());
    assert!(left.contains("line 1"), "{screen}");
    assert!(!right.contains("line 1 "), "{screen}");
    assert!(screen.contains("line 50"), "{screen}");
    // Back to the first group: its cursor is still on line 1.
    s.run("key ctrl+1");
    assert!(s.screen().contains("Ln 1, Col 1"), "{}", s.screen());
    s.run("key ctrl+2");
    assert!(s.screen().contains("Ln 50, Col 1"), "{}", s.screen());
}

#[test]
fn typing_in_one_group_shows_in_the_other() {
    let mut s = session();
    s.run("cmd view.split; type hello,");
    let screen = s.screen();
    assert_eq!(screen.matches("hello,line 1").count(), 2, "{screen}");
}

#[test]
fn files_open_in_the_focused_group_and_clicks_focus_a_group() {
    let mut s = session();
    s.run("cmd view.split");
    s.send(merlin_core::Event::Opened {
        path: "b.txt".into(),
        text: "bee\n".into(),
    });
    let tabs = tab_row(&s);
    let divider = s.screen().lines().nth(2).unwrap().find('│').unwrap();
    let b = tabs.find("b.txt").unwrap();
    assert!(b > divider, "b.txt opened on the right: {tabs}");
    // A click in the left group's text focuses it.
    s.run("click 20 5");
    assert!(s.screen().contains("a.txt"), "{}", s.screen());
    s.run("type X");
    assert!(s.text().contains('X'));
    assert_eq!(
        s.editor.buffer().path.as_deref(),
        Some(std::path::Path::new("a.txt"))
    );
}

#[test]
fn closing_the_last_tab_of_a_group_closes_the_group() {
    let mut s = session();
    s.run("cmd view.split");
    assert!(s.screen().lines().nth(2).unwrap().contains('│'));
    s.run("key ctrl+w");
    let screen = s.screen();
    assert!(!screen.lines().nth(2).unwrap().contains('│'), "{screen}");
    // The file is still open in the group that's left.
    assert!(tab_row(&s).contains("a.txt"), "{screen}");
    assert!(screen.contains("line 1"));
}

#[test]
fn a_selection_shows_only_in_its_own_group() {
    let mut s = session();
    s.run("cmd view.split; key ctrl+1; key shift+right shift+right shift+right shift+right");
    let render = s.editor.render();
    let screen = s.screen();
    let row = screen.lines().nth(2).unwrap();
    let divider = row.chars().position(|c| c == '│').unwrap() as u16;
    let selected = |from: u16, to: u16| {
        (from..to).any(|x| render.cell(x, 2).style == merlin_core::screen::Style::Selection)
    };
    assert!(selected(0, divider), "{screen}");
    assert!(!selected(divider, render.width), "{screen}");
}

#[test]
fn edits_in_one_group_move_the_other_groups_cursor_along() {
    let mut s = session();
    // Left group on line 50; the right one inserts three lines at the top.
    s.run("key ctrl+g; type 50; key enter; key right; key right");
    s.run("cmd view.split; key ctrl+home; type x\\ny\\nz\\n");
    s.run("key ctrl+1");
    let screen = s.screen();
    assert!(screen.contains("Ln 53, Col 3"), "{screen}");
    assert!(screen.contains("line 50"), "still in view: {screen}");
}

#[test]
fn split_down_puts_a_group_under_the_first() {
    let mut s = session();
    s.run("cmd view.split_down");
    let screen = s.screen();
    let lines: Vec<&str> = screen.lines().collect();
    let tab_rows = lines.iter().filter(|l| l.contains("a.txt")).count();
    assert!(tab_rows >= 2, "two tab rows: {screen}");
    assert!(lines.iter().any(|l| l.contains("──────")), "{screen}");
    // Each group scrolls on its own.
    s.run("key ctrl+g; type 40; key enter");
    s.run("key ctrl+1");
    assert!(s.screen().contains("Ln 1, Col 1"), "{}", s.screen());
    // A split right then makes a second column, beside both.
    s.run("cmd view.split");
    let screen = s.screen();
    let row = screen.lines().nth(1).unwrap();
    assert_eq!(row.matches("a.txt").count(), 2, "{screen}");
}

/// Where `text` first shows on the tab row of the group at column `from`.
fn tab_at(s: &Session, text: &str, from: usize) -> (usize, usize) {
    let screen = s.screen();
    for (y, line) in screen.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let rest: String = chars[from.min(chars.len())..].iter().collect();
        if let Some(i) = rest.find(text) {
            return (from + rest[..i].chars().count(), y);
        }
    }
    panic!("no {text:?} in\n{screen}");
}

#[test]
fn dragging_a_tab_to_another_group_moves_it_there() {
    let mut s = session();
    s.run("cmd view.split; key ctrl+1; key ctrl+o; type b.txt; key enter");
    let screen = s.screen();
    let row = screen.lines().nth(1).unwrap().to_string();
    let middle = row.find('│').map_or(50, |i| row[..i].chars().count());
    let (x, y) = tab_at(&s, "b.txt", 0);
    s.run(&format!(
        "press {x} {y}; drag {} {y}; release {} {y}",
        middle + 10,
        middle + 10
    ));
    let row = s.screen().lines().nth(1).unwrap().to_string();
    let (left, right) = row.split_at(row.find('│').unwrap());
    assert!(!left.contains("b.txt"), "{row}");
    assert!(right.contains("b.txt") && right.contains("a.txt"), "{row}");
    assert_eq!(s.text(), "bee\n", "the moved tab is focused");
}

#[test]
fn dragging_the_last_tab_out_closes_its_group() {
    let mut s = session();
    s.run("cmd view.split; key ctrl+o; type b.txt; key enter");
    s.run("key ctrl+w"); // the right group now has only a.txt
    let (x, y) = tab_at(&s, "a.txt", 50);
    s.run(&format!(
        "press {x} {y}; drag 5 {}; release 5 {}",
        y + 5,
        y + 5
    ));
    let row = s.screen().lines().nth(1).unwrap().to_string();
    assert!(!row.contains('│'), "one group left: {row}");
}

#[test]
fn dragging_a_tab_along_the_tab_row_reorders_it() {
    let mut s = session();
    s.run("key ctrl+o; type b.txt; key enter");
    let (x, y) = tab_at(&s, "b.txt", 0);
    let (to, _) = tab_at(&s, "a.txt", 0);
    s.run(&format!("press {x} {y}; drag {to} {y}; release {to} {y}"));
    let row = s.screen().lines().nth(1).unwrap().to_string();
    assert!(row.find("b.txt") < row.find("a.txt"), "{row}");
}
