//! Editor groups: the editor split side by side, each group with its own
//! tabs, and a file open in two groups keeping a cursor in each.

use merlin_core::script::Session;
use merlin_core::{Event, UiEvent};

fn session() -> Session {
    let text: String = (1..=60).map(|i| format!("line {i}\n")).collect();
    let mut s = Session::new(&text, Some("a.txt"), 100, 20);
    s.files.insert("a.txt".into(), text);
    s.files.insert("b.txt".into(), "bee\n".into());
    s
}

/// Each group's tab names.
fn tabs(s: &Session) -> Vec<Vec<String>> {
    s.editor
        .window_view()
        .groups
        .iter()
        .map(|g| g.tabs.iter().map(|t| t.name.clone()).collect())
        .collect()
}

fn ui(s: &mut Session, event: UiEvent) {
    s.send(Event::Ui(event));
}

#[test]
fn split_opens_the_file_again_to_the_right_with_its_own_cursor() {
    let mut s = session();
    s.run("cmd view.split");
    assert_eq!(tabs(&s), [["a.txt"], ["a.txt"]]);
    // Moving in the right group leaves the left where it was.
    s.run("key ctrl+g; type 50; key enter");
    assert!(s.editor.group_rows(0)[0].contains("line 1"));
    assert!(!s.editor.group_rows(1).iter().any(|r| r.ends_with("line 1")));
    assert!(s.editor.group_rows(1).iter().any(|r| r.contains("line 50")));
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
    s.send(Event::Opened {
        path: "b.txt".into(),
        text: "bee\n".into(),
    });
    assert_eq!(tabs(&s), [vec!["a.txt"], vec!["a.txt", "b.txt"]]);
    // A click in the left group's text focuses it.
    s.run("group 1; click 20 5");
    s.run("type X");
    assert!(s.text().contains('X'));
    assert_eq!(
        s.editor.buffer().path.as_deref(),
        Some(std::path::Path::new("a.txt"))
    );
    assert!(s.editor.window_view().groups[0].focused);
}

#[test]
fn closing_the_last_tab_of_a_group_closes_the_group() {
    let mut s = session();
    s.run("cmd view.split");
    s.run("key ctrl+w");
    // The file is still open in the group that's left.
    assert_eq!(tabs(&s), [["a.txt"]]);
    assert!(s.screen().contains("line 1"));
}

#[test]
fn a_selection_shows_only_in_its_own_group() {
    let mut s = session();
    s.run("cmd view.split; key ctrl+1; key shift+right shift+right shift+right shift+right");
    let views = s.editor.text_views();
    let selected = |g: usize| {
        views
            .iter()
            .filter(|v| v.group == g)
            .flat_map(|v| &v.lines)
            .any(|l| l.styles.contains(&merlin_core::style::Style::Selection))
    };
    assert!(selected(0) && !selected(1), "{}", s.screen());
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
    let groups = s.editor.window_view().groups;
    assert_eq!(groups.len(), 2);
    assert!(!groups[0].below && groups[1].below);
    // Each group scrolls on its own.
    s.run("key ctrl+g; type 40; key enter");
    s.run("key ctrl+1");
    assert!(s.screen().contains("Ln 1, Col 1"), "{}", s.screen());
    // A split right then makes a second column, beside both.
    s.run("cmd view.split");
    let groups = s.editor.window_view().groups;
    assert_eq!(groups.len(), 3);
    assert!(
        groups[1].below && !groups[2].below,
        "a column of its own: {groups:?}"
    );
}

/// Drags group `from`'s tab `index` to group `to`, before tab `at`.
fn drag_tab(s: &mut Session, from: usize, index: usize, to: usize, at: Option<usize>) {
    ui(
        s,
        UiEvent::GroupTab {
            group: from,
            index,
            clicks: 1,
        },
    );
    ui(s, UiEvent::DropTab { group: to, at });
}

#[test]
fn dragging_a_tab_to_another_group_moves_it_there() {
    let mut s = session();
    s.run("cmd view.split; key ctrl+1; key ctrl+o; type b.txt; key enter");
    assert_eq!(tabs(&s), [vec!["a.txt", "b.txt"], vec!["a.txt"]]);
    drag_tab(&mut s, 0, 1, 1, None);
    assert_eq!(tabs(&s), [vec!["a.txt"], vec!["a.txt", "b.txt"]]);
    assert_eq!(s.text(), "bee\n", "the moved tab is focused");
}

#[test]
fn dragging_the_last_tab_out_closes_its_group() {
    let mut s = session();
    s.run("cmd view.split; key ctrl+o; type b.txt; key enter");
    s.run("key ctrl+w"); // the right group now has only a.txt
    drag_tab(&mut s, 1, 0, 0, None);
    assert_eq!(tabs(&s).len(), 1, "one group left");
}

#[test]
fn dragging_a_tab_along_the_tab_row_reorders_it() {
    let mut s = session();
    s.run("key ctrl+o; type b.txt; key enter");
    assert_eq!(tabs(&s), [["a.txt", "b.txt"]]);
    drag_tab(&mut s, 0, 1, 0, Some(0));
    assert_eq!(tabs(&s), [["b.txt", "a.txt"]]);
}
