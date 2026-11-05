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
    s.run("cmd view.split; key ctrl+1; key ctrl+p; type b.txt; key enter");
    assert_eq!(tabs(&s), [vec!["a.txt", "b.txt"], vec!["a.txt"]]);
    drag_tab(&mut s, 0, 1, 1, None);
    assert_eq!(tabs(&s), [vec!["a.txt"], vec!["a.txt", "b.txt"]]);
    assert_eq!(s.text(), "bee\n", "the moved tab is focused");
}

#[test]
fn dragging_the_last_tab_out_closes_its_group() {
    let mut s = session();
    s.run("cmd view.split; key ctrl+p; type b.txt; key enter");
    s.run("key ctrl+w"); // the right group now has only a.txt
    drag_tab(&mut s, 1, 0, 0, None);
    assert_eq!(tabs(&s).len(), 1, "one group left");
}

#[test]
fn dragging_a_tab_along_the_tab_row_reorders_it() {
    let mut s = session();
    s.run("key ctrl+p; type b.txt; key enter");
    assert_eq!(tabs(&s), [["a.txt", "b.txt"]]);
    drag_tab(&mut s, 0, 1, 0, Some(0));
    assert_eq!(tabs(&s), [["b.txt", "a.txt"]]);
}

/// Each group's tab names, and whether it's under the one before.
fn layout(s: &Session) -> Vec<(Vec<String>, bool)> {
    s.editor
        .window_view()
        .groups
        .iter()
        .map(|g| (g.tabs.iter().map(|t| t.name.clone()).collect(), g.below))
        .collect()
}

fn names(names: &[&str], below: bool) -> (Vec<String>, bool) {
    (names.iter().map(|n| n.to_string()).collect(), below)
}

#[test]
fn dropping_a_tab_on_a_side_of_the_code_splits_there() {
    use merlin_core::editor::Edge;
    let mut s = session();
    s.run("key ctrl+p; type b.txt; key enter");
    // b.txt onto the right half: a new column, b.txt moved there.
    ui(
        &mut s,
        UiEvent::SplitTab {
            group: 0,
            edge: Edge::Right,
        },
    );
    assert_eq!(
        layout(&s),
        [names(&["a.txt"], false), names(&["b.txt"], false)]
    );
    assert_eq!(s.text(), "bee\n", "the moved tab is focused");
    // Its only tab on its own side: nothing to split.
    ui(
        &mut s,
        UiEvent::SplitTab {
            group: 1,
            edge: Edge::Left,
        },
    );
    assert_eq!(layout(&s).len(), 2);
    // Onto the top of the first group: above it, in its column.
    ui(
        &mut s,
        UiEvent::SplitTab {
            group: 0,
            edge: Edge::Up,
        },
    );
    assert_eq!(
        layout(&s),
        [names(&["b.txt"], false), names(&["a.txt"], true)]
    );
    // And onto the left of that column: a column before it.
    ui(
        &mut s,
        UiEvent::GroupTab {
            group: 1,
            index: 0,
            clicks: 1,
        },
    );
    s.run("key ctrl+p; type b.txt; key enter");
    ui(
        &mut s,
        UiEvent::SplitTab {
            group: 1,
            edge: Edge::Left,
        },
    );
    assert_eq!(
        layout(&s),
        [
            names(&["b.txt"], false),
            names(&["b.txt"], false),
            names(&["a.txt"], true)
        ]
    );
}

#[test]
fn split_left_and_up_put_the_new_group_before() {
    let mut s = session();
    s.run("cmd view.split_left");
    assert_eq!(
        layout(&s),
        [names(&["a.txt"], false), names(&["a.txt"], false)]
    );
    let focused = s.editor.window_view().groups.iter().position(|g| g.focused);
    assert_eq!(focused, Some(0), "the new group, on the left");
    s.run("cmd view.split_up");
    let below: Vec<bool> = layout(&s).iter().map(|g| g.1).collect();
    assert_eq!(below, [false, true, false]);
}

#[test]
fn a_file_dropped_from_the_explorer_opens_where_it_lands() {
    use merlin_core::editor::Edge;
    let mut s = session();
    s.run("key ctrl+shift+e; key esc");
    let (side, index) = s.entry("b.txt").expect("in the Explorer");
    s.send(Event::Ui(UiEvent::DropEntry {
        side,
        index,
        group: 0,
        edge: Some(Edge::Down),
    }));
    assert_eq!(
        layout(&s),
        [names(&["a.txt"], false), names(&["b.txt"], true)]
    );
    let tab = &s.editor.window_view().groups[1].tabs[0];
    assert!(!tab.preview, "kept, not a preview");
    // On the middle: a tab of that group.
    s.files.insert("c.txt".into(), "sea\n".into());
    s.run("key ctrl+shift+e; key esc");
    let (side, index) = s.entry("c.txt").unwrap();
    s.send(Event::Ui(UiEvent::DropEntry {
        side,
        index,
        group: 0,
        edge: None,
    }));
    assert_eq!(layout(&s)[0], names(&["a.txt", "c.txt"], false));
}

#[test]
fn pinned_tabs_stay_first_and_closing_others_skips_them() {
    let mut s = session();
    s.files.insert("c.txt".into(), "sea\n".into());
    s.run("key ctrl+p; type b.txt; key enter; key ctrl+p; type c.txt; key enter");
    s.run("cmd file.pin");
    assert_eq!(tabs(&s), [["c.txt", "a.txt", "b.txt"]]);
    assert!(s.editor.window_view().groups[0].tabs[0].pinned);
    // Dragging an unpinned tab before it leaves it after.
    drag_tab(&mut s, 0, 2, 0, Some(0));
    assert_eq!(tabs(&s), [["c.txt", "b.txt", "a.txt"]]);
    s.run("cmd file.close_others");
    assert_eq!(tabs(&s), [["c.txt", "b.txt"]], "b.txt was focused");
    s.run("cmd file.close_all");
    assert_eq!(tabs(&s), [["c.txt"]], "the pinned one stays");
    s.run("key ctrl+1; cmd file.unpin");
    assert!(!s.editor.window_view().groups[0].tabs[0].pinned);
}

#[test]
fn the_tab_menu_offers_what_applies() {
    let mut s = session();
    s.send(Event::Ui(UiEvent::TabMenu));
    let labels = |s: &Session| -> Vec<String> {
        s.editor
            .window_view()
            .context
            .map(|c| c.items.into_iter().map(|i| i.label).collect())
            .unwrap_or_default()
    };
    let menu = labels(&s);
    assert!(
        menu.contains(&"Pin".into()) && !menu.contains(&"Unpin".into()),
        "{menu:?}"
    );
    assert!(!menu.contains(&"Keep Open".into()), "not a preview");
    for item in [
        "Close Saved",
        "Copy Relative Path",
        "Reveal in Explorer View",
        "Split Up",
        "Split Left",
    ] {
        assert!(menu.contains(&item.into()), "{item} in {menu:?}");
    }
    s.run("key esc; cmd file.pin");
    s.send(Event::Ui(UiEvent::TabMenu));
    assert!(labels(&s).contains(&"Unpin".into()));
}

#[test]
fn pins_are_kept_with_the_workspace() {
    let mut s = session();
    s.run("key ctrl+p; type b.txt; key enter; cmd file.pin");
    let text = s.editor.workspace().to_text();
    assert!(text.contains("pin 0:0 0 b.txt"), "{text}");
    let ws = merlin_core::editor::Workspace::parse(&text);
    assert!(ws.groups[0].tabs[0].pinned);
}
