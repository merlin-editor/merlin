//! The workbench around the text: reviewing changes a plugin proposes,
//! files that change on disk, git, the file tree, search, menus, and
//! Markdown in panels.

use merlin_core::script::Session;
use merlin_core::style::Style;
use merlin_core::{Effect, Event, GitAction, Node, Panel};

fn session() -> Session {
    let mut s = Session::new("fn a() {}\nfn b() {}\n", Some("src/main.rs"), 60, 12);
    s.files.insert("src/main.rs".into(), s.text());
    s
}

fn review(id: &str) -> Event {
    Event::ShowReview {
        plugin: "agent".into(),
        id: id.into(),
        path: "src/main.rs".into(),
        old: "fn a() {}\nfn b() {}\n".into(),
        new: "fn a() {}\nfn c() {}\nfn b() {}\n".into(),
    }
}

fn answers(s: &Session) -> Vec<(String, bool)> {
    s.effects
        .iter()
        .filter_map(|e| match e {
            Effect::PluginReview { id, accepted, .. } => Some((id.clone(), *accepted)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_review_shows_the_change_with_signs_and_line_numbers() {
    let mut s = session();
    s.send(review("e1"));
    let screen = s.screen();
    assert!(screen.contains("[Review main.rs]"), "{screen}");
    let rows = s.editor.text_rows();
    assert!(rows.contains(&"+  2 fn c() {}".to_string()), "{rows:?}");
    assert!(rows.contains(&"   3 fn b() {}".to_string()), "{rows:?}");
    assert!(screen.contains("y accept · n reject"), "{screen}");
    let (x, y) = s.find("fn c").unwrap();
    assert_eq!(s.cell(x, y).unwrap().style, Style::DiffAdded);
}

#[test]
fn a_review_is_read_only() {
    let mut s = session();
    s.send(review("e1"));
    s.run("type x; key backspace; paste hi");
    assert!(s.text().contains("fn c() {}"));
    assert!(!s.text().contains('x'));
    assert!(answers(&s).is_empty());
}

#[test]
fn y_accepts_and_goes_back_to_the_file() {
    let mut s = session();
    s.run("key down");
    s.send(review("e1"));
    s.run("type y");
    assert_eq!(answers(&s), [("e1".to_string(), true)]);
    assert_eq!(s.text(), "fn a() {}\nfn b() {}\n");
    assert_eq!(s.editor.cursor(), 10, "back where the cursor was");
    assert!(!s.screen().contains("Review"));
}

#[test]
fn n_rejects_and_the_next_review_comes_up() {
    let mut s = session();
    s.send(review("e1"));
    s.send(review("e2"));
    s.run("type n");
    assert_eq!(answers(&s), [("e2".to_string(), false)]);
    assert!(s.editor.in_review(), "e1 is still waiting");
    s.run("key enter");
    assert_eq!(answers(&s)[1], ("e1".to_string(), true));
    assert!(!s.editor.in_review());
}

#[test]
fn esc_leaves_the_review_open_and_the_plugin_can_close_it() {
    let mut s = session();
    s.send(review("e1"));
    s.run("key esc");
    assert!(!s.editor.in_review());
    assert!(s.screen().contains("Review main.rs"), "still a tab");
    s.send(Event::CloseReview {
        plugin: "agent".into(),
        id: "e1".into(),
    });
    assert!(!s.screen().contains("Review main.rs"));
    assert!(answers(&s).is_empty());
}

#[test]
fn brackets_jump_between_changes() {
    let old: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    let new = old
        .replace("line 2\n", "two\n")
        .replace("line 20\n", "twenty\n");
    let mut s = session();
    s.send(Event::ShowReview {
        plugin: "agent".into(),
        id: "e1".into(),
        path: "notes.txt".into(),
        old,
        new,
    });
    let line = |s: &Session| {
        s.text()
            .chars()
            .take(s.editor.cursor())
            .filter(|c| *c == '\n')
            .count()
    };
    let first = line(&s);
    assert!(s.text().lines().nth(first).unwrap().contains("line 2"));
    s.run("type ]");
    assert!(s.text().lines().nth(line(&s)).unwrap().contains("line 20"));
    s.run("type [");
    assert_eq!(line(&s), first);
}

#[test]
fn a_file_changed_on_disk_reloads_and_keeps_the_cursor() {
    let mut s = session();
    s.run("key down");
    s.send(Event::FileChanged {
        path: "src/main.rs".into(),
        text: "// new\nfn a() {}\nfn b() {}\n".into(),
    });
    assert_eq!(s.text(), "// new\nfn a() {}\nfn b() {}\n");
    assert_eq!(s.editor.cursor(), 17, "still on `fn b`");
    assert!(!s.editor.buffer().is_dirty());
    s.run("key ctrl+z");
    assert_eq!(s.text(), "fn a() {}\nfn b() {}\n", "undo goes back");
}

#[test]
fn a_reload_changing_lines_around_the_cursor_leaves_it_on_its_line() {
    let mut s = Session::new("a - b\nkeep\nc - d\n", Some("m.py"), 60, 12);
    s.run("key down end");
    s.send(Event::FileChanged {
        path: "m.py".into(),
        text: "a + b\nkeep\nc + d\n".into(),
    });
    assert_eq!(s.editor.cursor(), 10, "end of `keep`");
}

#[test]
fn unsaved_changes_are_not_overwritten_by_the_disk() {
    let mut s = session();
    s.run("type x");
    s.send(Event::FileChanged {
        path: "src/main.rs".into(),
        text: "other\n".into(),
    });
    assert!(s.text().starts_with('x'));
    assert!(s.screen().contains("changed on disk"), "{}", s.screen());
}

#[test]
fn git_signs_show_in_their_own_column() {
    let mut s = session();
    let (x, y) = s.editor.text_origin();
    // The gutter is "  1 ", after the activity bar.
    let signs = x - 4;
    s.send(Event::GitBase {
        path: "src/main.rs".into(),
        text: Some("fn a() {}\n".into()),
    });
    assert_eq!(s.editor.text_origin().0, x + 1, "the gutter grew a column");
    assert_eq!(s.cell(x + 1, y).unwrap().ch, 'f');
    assert_eq!(s.cell(signs, y).unwrap().ch, ' ');
    assert_eq!(s.cell(signs, y + 1).unwrap().style, Style::GitAdded);
    s.run("key home; type // ");
    assert_eq!(s.cell(signs, y).unwrap().style, Style::GitChanged);
}

#[test]
fn the_explorer_opens_folders_and_files() {
    let mut s = session();
    s.files.insert("README.md".into(), "# hi\n".into());
    s.files.insert("src/lib.rs".into(), "// lib\n".into());
    s.files.insert("docs/guide.md".into(), "guide\n".into());
    s.run("key ctrl+shift+e");
    let screen = s.screen();
    assert!(screen.contains("── left: Explorer"), "{screen}");
    assert!(screen.contains("  ▸ docs"), "{screen}");
    assert!(
        screen.contains("  ▾ src"),
        "the open file's folder is open: {screen}"
    );
    assert!(
        screen.contains("›     main.rs"),
        "focused, inside src: {screen}"
    );
    // Focus starts on the open file; lib.rs is just above it.
    s.run("key up; key enter");
    assert_eq!(s.text(), "// lib\n");
    assert!(s.editor.side_bar_visible(), "the side bar stays");
    s.run("key ctrl+shift+e; key up; key up; key up; key right");
    assert!(s.screen().contains("▾ docs"), "{}", s.screen());
    s.run("key down; key enter");
    assert_eq!(s.text(), "guide\n");
    s.run("key ctrl+b");
    assert!(!s.editor.side_bar_visible());
}

#[test]
fn markdown_in_a_panel() {
    let mut s = session();
    s.send(Event::SetPanel {
        plugin: "agent".into(),
        panel: Some(Panel {
            title: "Agent".into(),
            nodes: vec![Node::Markdown(
                "Use **this**:\n```rust\nlet x = 1;\n```\n- one\n- two".into(),
            )],
        }),
    });
    s.run("key ctrl+alt+b");
    // The window draws it; the panel passes it on as it is.
    let right = s.editor.window_view().right.expect("the panel");
    assert!(matches!(&right.nodes[0], Node::Markdown(t) if t.contains("```rust")));
    assert!(s.screen().contains("let x = 1;"), "{}", s.screen());
}

#[test]
fn plugins_can_see_the_selection() {
    let mut s = session();
    assert_eq!(s.editor.selection_utf16(), None);
    s.run("key right right right; key shift+down");
    assert_eq!(s.editor.selection_utf16(), Some(((0, 3), (1, 3))));
}

#[test]
fn search_finds_lines_across_the_project_and_opens_them() {
    let mut s = Session::new("fn a() {}\nfn b() {}\n", Some("src/main.rs"), 100, 12);
    s.files.insert("src/main.rs".into(), s.text());
    s.files
        .insert("src/lib.rs".into(), "// lib\npub fn b() {}\n".into());
    s.run("key ctrl+shift+f; type fn b; key enter");
    let screen = s.screen();
    assert!(screen.contains("2 results in 2 files"), "{screen}");
    assert!(screen.contains("▾ lib.rs  src"), "{screen}");
    // Down past the first file's name, to its line.
    s.run("key down; key down; key enter");
    assert_eq!(s.text(), "// lib\npub fn b() {}\n");
    assert_eq!(s.editor.cursor_utf16(), (1, 4));
    s.run("key ctrl+shift+f");
    assert!(s.screen().contains("› [fn b]"), "the query stays");
}

#[test]
fn source_control_lists_changes_and_opens_their_diff() {
    let mut s = session();
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![("src/main.rs".into(), 'M'), ("new.txt".into(), '?')],
        staged: vec![],
        changed: vec![("src/main.rs".into(), 'M'), ("new.txt".into(), '?')],
    });
    s.run("key ctrl+shift+g");
    let screen = s.screen();
    assert!(screen.contains("main.rs  src  M"), "{screen}");
    assert!(screen.contains("new.txt  U"), "{screen}");
    assert!(
        screen.contains("⎇ main"),
        "the status bar names the branch: {screen}"
    );
    // Focus starts in the commit message, as in VS Code; the first file
    // comes after the Commit and Stage All buttons.
    s.run("key down down down");
    s.run("key enter");
    assert!(s.effects.contains(&Effect::OpenDiff("src/main.rs".into())));
    s.send(Event::ShowDiff {
        path: "src/main.rs".into(),
        old: "fn a() {}\n".into(),
        new: "fn a() {}\nfn b() {}\n".into(),
    });
    let screen = s.screen();
    assert!(screen.contains("[main.rs (changes)]"), "{screen}");
    assert!(screen.contains("Enter open the file"), "{screen}");
    s.run("type y");
    assert!(answers(&s).is_empty(), "nothing to answer");
    s.run("key enter");
    assert_eq!(s.text(), "fn a() {}\nfn b() {}\n");
    assert_eq!(s.editor.cursor_utf16(), (1, 0), "on the changed line");
}

#[test]
fn menus_open_with_alt_and_run_commands() {
    let mut s = session();
    s.run("key alt+v");
    let screen = s.screen();
    assert!(screen.contains("Command Palette…"), "{screen}");
    assert!(screen.contains("Ctrl+Shift+P"), "keys show: {screen}");
    s.run("key right; key down; key enter");
    assert_eq!(
        s.editor.focus(),
        merlin_core::Focus::Palette,
        "Go > Go to Line…"
    );
    s.run("type 2; key enter");
    assert_eq!(s.editor.cursor_utf16(), (1, 0));
}

#[test]
fn menus_open_and_run_with_the_mouse() {
    use merlin_core::UiEvent;
    let mut s = Session::new("fn a() {}\nfn b() {}\n", Some("src/main.rs"), 60, 20);
    // The menu button opens the menus; a title then shows its own.
    s.run("key f10");
    let titles = s.editor.window_view().menu.expect("the menus").titles;
    let edit = titles.iter().position(|t| t == "Edit").unwrap();
    s.send(Event::Ui(UiEvent::MenuTitle(edit)));
    let menu = s.editor.window_view().menu.expect("the Edit menu");
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert!(!labels.contains(&"Select All"), "that's under Selection");
    let comment = labels
        .iter()
        .position(|l| *l == "Toggle Line Comment")
        .unwrap();
    s.send(Event::Ui(UiEvent::MenuItem(comment)));
    assert_eq!(s.text(), "// fn a() {}\nfn b() {}\n");
}

#[test]
fn ctrl_slash_comments_and_uncomments_lines() {
    let mut s = Session::new("def f():\n    return 1\n", Some("m.py"), 60, 12);
    s.run("key shift+down; key shift+down; key ctrl+/");
    assert_eq!(s.text(), "# def f():\n#     return 1\n");
    s.run("key ctrl+/");
    assert_eq!(s.text(), "def f():\n    return 1\n");
}

#[test]
fn clicking_a_file_in_the_explorer_opens_it() {
    let mut s = Session::new("fn a() {}\n", Some("src/main.rs"), 100, 30);
    s.files.insert("src/main.rs".into(), s.text());
    s.files.insert("README.md".into(), "# hi\n".into());
    s.run("key ctrl+shift+e; key esc");
    s.click_entry("README.md", 1);
    assert_eq!(s.text(), "# hi\n", "{}", s.screen());
}

#[test]
fn wide_windows_show_a_review_side_by_side() {
    let mut s = Session::new("fn a() {}\nfn b() {}\n", Some("src/main.rs"), 150, 14);
    s.send(Event::ShowReview {
        plugin: "agent".into(),
        id: "e1".into(),
        path: "src/main.rs".into(),
        old: "fn a() {}\nfn b() {}\nfn c() {}\n".into(),
        new: "fn a() {}\nfn B() {}\nfn c() {}\n".into(),
    });
    // Two views of the group: the old text on the left, the new on the
    // right.
    let views = s.editor.text_views();
    assert_eq!(views.len(), 2);
    assert!(views[0].area.x < views[1].area.x, "old on the left");
    let text = |v: &merlin_core::TextView| -> String {
        v.lines
            .iter()
            .map(|l| l.chars.iter().collect::<String>() + "\n")
            .collect()
    };
    assert!(text(&views[0]).contains("fn b()") && text(&views[1]).contains("fn B()"));
    assert!(
        text(&views[0]).contains("fn c()") && text(&views[1]).contains("fn c()"),
        "unchanged lines on both sides"
    );
    s.run("type y");
    assert_eq!(answers(&s), [("e1".to_string(), true)]);

    // Narrow, it's one column again.
    let mut s = Session::new("", Some("src/main.rs"), 80, 14);
    s.send(review("e2"));
    assert_eq!(s.editor.text_views().len(), 1);
    assert!(
        s.editor.text_rows().contains(&"+  2 fn c() {}".to_string()),
        "{}",
        s.screen()
    );
}

#[test]
fn shift_and_the_wheel_scroll_sideways() {
    let line = (0..60).map(|i| format!("{i:02}")).collect::<String>();
    let mut s = Session::new(&line, Some("n.txt"), 40, 8);
    s.run("wheel right");
    let row = &s.editor.text_rows()[0];
    assert!(row.starts_with("  1 030405"), "six columns along: {row}");
    s.run("wheel left");
    assert!(s.editor.text_rows()[0].starts_with("  1 000102"));
}

#[test]
fn source_control_stages_unstages_and_commits() {
    let mut s = session();
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![("src/main.rs".into(), 'M'), ("new.txt".into(), '?')],
        staged: vec![("src/main.rs".into(), 'M')],
        changed: vec![("new.txt".into(), '?')],
    });
    s.run("key ctrl+shift+g");
    let screen = s.screen();
    assert!(screen.contains("STAGED CHANGES  1"), "{screen}");
    assert!(screen.contains("\n  CHANGES  1"), "{screen}");
    assert!(screen.contains("main.rs  src  M"), "{screen}");
    // A click on + stages the file: the target after the file's own.
    let left = s.editor.window_view().left.unwrap();
    let row = left
        .nodes
        .iter()
        .position(|n| matches!(n, Node::Entry(e) if e.label == "new.txt"))
        .unwrap();
    let plus = left.targets[row].unwrap() + 1;
    let click = |s: &mut Session, index: usize| {
        s.send(Event::Ui(merlin_core::UiEvent::Target {
            side: merlin_core::Side::Left,
            index,
            clicks: 1,
        }))
    };
    click(&mut s, plus);
    assert!(
        s.effects
            .contains(&Effect::Git(GitAction::Stage("new.txt".into())))
    );
    // Enter in the message commits what's staged, and clears it.
    click(&mut s, 0);
    s.run("type Fix things; key enter");
    assert!(
        s.effects
            .contains(&Effect::Git(GitAction::Commit("Fix things".into()))),
        "{:?}",
        s.effects
    );
    assert!(!s.screen().contains("Fix things"));
    // With nothing staged, Commit says so instead.
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![("new.txt".into(), '?')],
        staged: vec![],
        changed: vec![("new.txt".into(), '?')],
    });
    s.effects.clear();
    click(&mut s, 0);
    s.run("type More; key enter");
    assert!(s.effects.iter().all(|e| !matches!(e, Effect::Git(_))));
    assert!(s.screen().contains("Nothing staged"), "{}", s.screen());
}
