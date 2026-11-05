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

/// Unsaved changes, then the file changes on disk: a question.
fn conflict() -> Session {
    let mut s = session();
    s.run("type x");
    s.send(Event::FileChanged {
        path: "src/main.rs".into(),
        text: "other\n".into(),
    });
    s
}

#[test]
fn unsaved_changes_are_not_overwritten_by_the_disk_without_asking() {
    let mut s = conflict();
    assert!(s.text().starts_with('x'));
    let dialog = s.editor.window_view().dialog.expect("a question");
    assert!(
        dialog.title.contains("main.rs changed on disk"),
        "{}",
        dialog.title
    );
    assert_eq!(
        dialog.buttons,
        ["Reload from Disk", "Overwrite", "Keep Editing"]
    );
    // Up by itself, mid-typing: Enter changes nothing.
    assert_eq!((dialog.selected, dialog.primary), (2, 2));
    s.run("key enter");
    assert!(s.editor.window_view().dialog.is_none());
    assert!(s.text().starts_with('x') && s.editor.buffer().is_dirty());
}

#[test]
fn reloading_from_disk_can_be_undone() {
    let mut s = conflict();
    // Changed again while asked: the newest is what reloads.
    s.send(Event::FileChanged {
        path: "src/main.rs".into(),
        text: "newer\n".into(),
    });
    s.send(Event::Ui(merlin_core::UiEvent::DialogButton(0)));
    assert_eq!(s.text(), "newer\n");
    assert!(!s.editor.buffer().is_dirty());
    s.run("key ctrl+z");
    assert!(s.text().starts_with('x'), "{}", s.text());
}

#[test]
fn overwriting_saves_the_changes_here() {
    let mut s = conflict();
    s.send(Event::Ui(merlin_core::UiEvent::DialogButton(1)));
    assert!(s.files["src/main.rs"].starts_with('x'));
    assert!(!s.editor.buffer().is_dirty());
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
    // comes after Commit, and the Changes heading and its +.
    s.run("key down down down down");
    s.run("key enter");
    assert!(s.effects.contains(&Effect::OpenDiff("src/main.rs".into())));
    s.send(Event::ShowDiff {
        path: "src/main.rs".into(),
        old: "fn a() {}\n".into(),
        new: "fn a() {}\nfn b() {}\n".into(),
    });
    let screen = s.screen();
    assert!(screen.contains("[main.rs (changes)]"), "{screen}");
    assert!(screen.contains("Alt+S stage"), "{screen}");
    // Typing there edits the file, on the changed line; nothing's
    // answered, and Esc goes back to the file.
    s.run("type y");
    assert!(answers(&s).is_empty(), "nothing to answer");
    s.run("key esc");
    assert_eq!(s.text(), "fn a() {}\nyfn b() {}\n");
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

const PALETTE_DIFF: &str = "\
diff --git a/src/palette.rs b/src/palette.rs
index 1111111..2222222 100644
--- a/src/palette.rs
+++ b/src/palette.rs
@@ -1,3 +1,4 @@
+/// A colour a few steps from the accent.
 pub fn shade(accent: u32, step: u32) -> u32 {
-    accent + step
+    accent + step * 2
 }
@@ -5,4 +6,4 @@ pub fn shade(accent: u32, step: u32) -> u32 {
 pub fn theme_colours(accent: u32) -> Vec<u32> {
-    let steps = [1u32, 2, 3];
+    let steps = [1u32, 2, 3, 4];
     steps.iter().map(|s| shade(accent, *s)).collect()
 }
";

fn changed_palette(s: &mut Session) {
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![("src/palette.rs".into(), 'M'), ("new.txt".into(), '?')],
        staged: vec![],
        changed: vec![("src/palette.rs".into(), 'M'), ("new.txt".into(), '?')],
    });
    s.send(Event::GitDiffs {
        staged: vec![],
        changed: merlin_core::git::parse(PALETTE_DIFF),
    });
}

fn target(s: &Session, id: &str) -> usize {
    let left = s.editor.window_view().left.unwrap();
    let targets = Panel {
        title: String::new(),
        nodes: left.nodes.clone(),
    }
    .targets();
    targets
        .iter()
        .position(|t| matches!(t, merlin_core::panel::Target::Button(b) if b == id))
        .unwrap_or_else(|| panic!("no {id} in {targets:?}"))
}

fn click_target(s: &mut Session, id: &str) {
    let index = target(s, id);
    s.send(Event::Ui(merlin_core::UiEvent::Target {
        side: merlin_core::Side::Left,
        index,
        clicks: 1,
    }));
}

#[test]
fn a_changed_files_hunks_unfold_under_it_to_stage_one_at_a_time() {
    let mut s = session();
    changed_palette(&mut s);
    s.run("key ctrl+shift+g");
    let screen = s.screen();
    assert!(
        screen.contains("▸ palette.rs  src  +3 −2  M"),
        "a file shows its counts, folded: {screen}"
    );
    // Its arrow unfolds the hunks, each with its line, its first change,
    // and Stage and Discard.
    let index = target(&s, "c:src/palette.rs");
    s.send(Event::Ui(merlin_core::UiEvent::Fold {
        side: merlin_core::Side::Left,
        index,
    }));
    let screen = s.screen();
    assert!(screen.contains("▾ palette.rs"), "{screen}");
    assert!(
        screen.contains("@@ 1  /// A colour a few steps from the accent.  [Stage]  [Discard]"),
        "{screen}"
    );
    assert!(screen.contains("@@ 3  accent + step * 2"), "{screen}");
    assert!(
        screen.contains("@@ 7  let steps = [1u32, 2, 3, 4];"),
        "{screen}"
    );
    // Stage applies that change alone to the index.
    click_target(&mut s, "hs:src/palette.rs:2");
    let staged = s.effects.iter().find_map(|e| match e {
        Effect::Git(GitAction::Apply {
            patch,
            cached: true,
            reverse: false,
        }) => Some(patch.clone()),
        _ => None,
    });
    let staged = staged.expect("a patch to stage");
    assert!(staged.contains("@@ -5,4 +6,4 @@"), "{staged}");
    assert!(!staged.contains("A colour"), "only that hunk: {staged}");
}

#[test]
fn discarding_a_hunk_or_a_file_can_be_undone() {
    let mut s = session();
    changed_palette(&mut s);
    s.run("key ctrl+shift+g");
    s.send(Event::Ui(merlin_core::UiEvent::Fold {
        side: merlin_core::Side::Left,
        index: target(&s, "c:src/palette.rs"),
    }));
    click_target(&mut s, "hd:src/palette.rs:0");
    assert!(
        s.effects.iter().any(|e| matches!(
            e,
            Effect::Git(GitAction::Discard { path, patch: Some(p) })
                if path == "src/palette.rs" && p.contains("+/// A colour")
        )),
        "{:?}",
        s.effects
    );
    // Discarding asks nothing, and says so with Undo.
    s.send(Event::Discarded {
        path: "src/palette.rs".into(),
        before: "the text before\n".into(),
        all: false,
    });
    let screen = s.screen();
    assert!(
        screen.contains("Discarded a change to palette.rs"),
        "{screen}"
    );
    s.effects.clear();
    s.send(Event::Ui(merlin_core::UiEvent::ToastButton(0)));
    assert!(
        s.effects.contains(&Effect::Git(GitAction::Undiscard {
            path: "src/palette.rs".into(),
            text: "the text before\n".into(),
        })),
        "{:?}",
        s.effects
    );
    // The file's menu discards all of it; a file git doesn't track goes
    // to the trash, after asking.
    s.effects.clear();
    s.send(Event::Ui(merlin_core::UiEvent::TargetMenu {
        side: merlin_core::Side::Left,
        index: target(&s, "c:src/palette.rs"),
    }));
    let screen = s.screen();
    assert!(screen.contains("Discard Changes"), "{screen}");
    assert!(screen.contains("Ctrl+Del"), "{screen}");
    s.send(Event::Ui(merlin_core::UiEvent::CloseContext));
    s.effects.clear();
    s.send(Event::Ui(merlin_core::UiEvent::TargetMenu {
        side: merlin_core::Side::Left,
        index: target(&s, "c:src/palette.rs"),
    }));
    let items = s.editor.window_view().context.unwrap().items;
    let discard = items
        .iter()
        .position(|i| i.label == "Discard Changes")
        .unwrap();
    s.send(Event::Ui(merlin_core::UiEvent::ContextItem(discard)));
    assert!(s.effects.contains(&Effect::Git(GitAction::Discard {
        path: "src/palette.rs".into(),
        patch: None,
    })));
    s.send(Event::Ui(merlin_core::UiEvent::TargetMenu {
        side: merlin_core::Side::Left,
        index: target(&s, "c:new.txt"),
    }));
    s.send(Event::Ui(merlin_core::UiEvent::ContextItem(discard)));
    let dialog = s
        .editor
        .window_view()
        .dialog
        .expect("an untracked file asks");
    assert_eq!(dialog.title, "Are you sure you want to delete 'new.txt'?");
}

const HEAD_PALETTE: &str = "\
pub fn shade(accent: u32, step: u32) -> u32 {
    accent + step
}

pub fn theme_colours(accent: u32) -> Vec<u32> {
    let steps = [1u32, 2, 3];
    steps.iter().map(|s| shade(accent, *s)).collect()
}
";

const NOW_PALETTE: &str = "\
/// A colour a few steps from the accent.
pub fn shade(accent: u32, step: u32) -> u32 {
    accent + step * 2
}

pub fn theme_colours(accent: u32) -> Vec<u32> {
    let steps = [1u32, 2, 3, 4];
    steps.iter().map(|s| shade(accent, *s)).collect()
}
";

/// The palette's changes tab, wide, with the steps change staged.
fn palette_changes() -> Session {
    let mut s = Session::new("fn a() {}\n", Some("src/main.rs"), 160, 16);
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![("src/palette.rs".into(), 'M')],
        staged: vec![("src/palette.rs".into(), 'M')],
        changed: vec![("src/palette.rs".into(), 'M')],
    });
    let staged = "\
diff --git a/src/palette.rs b/src/palette.rs
--- a/src/palette.rs
+++ b/src/palette.rs
@@ -5,4 +5,4 @@ pub fn shade(accent: u32, step: u32) -> u32 {
 pub fn theme_colours(accent: u32) -> Vec<u32> {
-    let steps = [1u32, 2, 3];
+    let steps = [1u32, 2, 3, 4];
     steps.iter().map(|s| shade(accent, *s)).collect()
 }
";
    let changed = "\
diff --git a/src/palette.rs b/src/palette.rs
--- a/src/palette.rs
+++ b/src/palette.rs
@@ -1,3 +1,4 @@
+/// A colour a few steps from the accent.
 pub fn shade(accent: u32, step: u32) -> u32 {
-    accent + step
+    accent + step * 2
 }
";
    s.send(Event::GitDiffs {
        staged: merlin_core::git::parse(staged),
        changed: merlin_core::git::parse(changed),
    });
    s.send(Event::ShowDiff {
        path: "src/palette.rs".into(),
        old: HEAD_PALETTE.into(),
        new: NOW_PALETTE.into(),
    });
    s
}

fn applied(s: &Session) -> Vec<(String, bool)> {
    s.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Git(GitAction::Apply { patch, reverse, .. }) => Some((patch.clone(), *reverse)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_changes_tab_has_a_box_for_each_change_and_line() {
    let mut s = palette_changes();
    let views = s.editor.text_views();
    assert_eq!(views.len(), 2, "side by side");
    let marks = views[1].diff.clone().expect("the new side's marks");
    let changes: Vec<_> = marks.rows.iter().filter_map(|m| m.change).collect();
    use merlin_core::panel::Check;
    assert_eq!(
        changes.iter().map(|c| c.1).collect::<Vec<_>>(),
        [Check::Off, Check::Off, Check::On],
        "the steps change is staged"
    );
    // A removed line pairs with the line that replaced it: a hatched row
    // on the old side where the doc comment was added.
    assert_eq!(views[0].diff.as_ref().unwrap().fillers, [0]);
    assert!(
        s.screen().contains("Staged 1 of 3 changes"),
        "{}",
        s.screen()
    );
    // The first box stages only the doc comment.
    s.send(Event::Ui(merlin_core::UiEvent::DiffStage(0)));
    let got = applied(&s);
    assert_eq!(got.len(), 1);
    assert!(!got[0].1);
    assert!(got[0].0.contains("+/// A colour"), "{}", got[0].0);
    assert!(!got[0].0.contains("+    accent + step * 2"), "{}", got[0].0);
    // The staged change's box unstages it, from the staged diff.
    s.effects.clear();
    s.send(Event::Ui(merlin_core::UiEvent::DiffStage(2)));
    let got = applied(&s);
    assert!(got[0].1, "backwards");
    assert!(got[0].0.contains("+    let steps = [1u32, 2, 3, 4];"));
    // A row's own box: the changed line, its old and new halves.
    s.effects.clear();
    let row = marks
        .rows
        .iter()
        .filter_map(|m| m.line)
        .nth(1)
        .expect("the second change's row");
    assert!(!row.1);
    s.send(Event::Ui(merlin_core::UiEvent::DiffLine(row.0)));
    let got = applied(&s);
    assert!(
        got[0]
            .0
            .contains("-    accent + step\n+    accent + step * 2\n }"),
        "{}",
        got[0].0
    );
    assert!(!got[0].0.contains("+/// A colour"), "{}", got[0].0);
}

#[test]
fn reverting_a_change_goes_back_to_the_last_commit() {
    let mut s = palette_changes();
    // The cursor starts on the first change; Alt+F5 goes to the next.
    s.run("key alt+f5");
    s.run("key alt+r");
    let patch = s.effects.iter().find_map(|e| match e {
        Effect::Git(GitAction::Discard { patch: Some(p), .. }) => Some(p.clone()),
        _ => None,
    });
    let patch = patch.expect("a patch to revert");
    assert!(
        patch.contains("-    accent + step\n+    accent + step * 2\n"),
        "{patch}"
    );
    assert!(!patch.contains("A colour"), "only that change: {patch}");
    // Alt+I shows it inline, and again side by side.
    s.run("key alt+i");
    assert_eq!(s.editor.text_views().len(), 1);
    s.run("key alt+i");
    assert_eq!(s.editor.text_views().len(), 2);
    // Alt+S still opens the Selection menu outside the changes tab.
    let bar = s.editor.window_view().diff_bar.expect("the bar");
    assert_eq!((bar.change, bar.changes), (2, 3));
}

#[test]
fn typing_in_the_changes_tab_edits_the_file() {
    let mut s = Session::new(NOW_PALETTE, Some("src/palette.rs"), 160, 16);
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![("src/palette.rs".into(), 'M')],
        staged: vec![],
        changed: vec![("src/palette.rs".into(), 'M')],
    });
    s.send(Event::ShowDiff {
        path: "src/palette.rs".into(),
        old: HEAD_PALETTE.into(),
        new: NOW_PALETTE.into(),
    });
    assert!(
        s.screen().contains("[palette.rs (changes)]"),
        "{}",
        s.screen()
    );
    // The cursor starts on the first change, the added doc comment.
    s.run("type X");
    let views = s.editor.text_views();
    let new_side: String = views[1]
        .lines
        .iter()
        .map(|l| l.chars.iter().collect::<String>() + "\n")
        .collect();
    assert!(new_side.contains("X/// A colour"), "{new_side}");
    assert_eq!(
        views[1].carets.first().and_then(|c| c.cell),
        Some((1, 0)),
        "the cursor shows after the X"
    );
    // The file has it, unsaved, and staging waits for a save.
    s.send(Event::Ui(merlin_core::UiEvent::DiffStage(0)));
    assert!(
        s.screen().contains("Save palette.rs first"),
        "{}",
        s.screen()
    );
    assert!(applied(&s).is_empty());
    // Ctrl+S saves the file, and once git has the saved file the boxes
    // come back.
    s.run("key ctrl+s");
    assert!(
        s.saved
            .iter()
            .any(|(p, t)| p.ends_with("palette.rs") && t.starts_with("X///")),
        "{:?}",
        s.saved
    );
    let changed = "\
diff --git a/src/palette.rs b/src/palette.rs
--- a/src/palette.rs
+++ b/src/palette.rs
@@ -1,3 +1,4 @@
+X/// A colour a few steps from the accent.
 pub fn shade(accent: u32, step: u32) -> u32 {
-    accent + step
+    accent + step * 2
 }
";
    s.send(Event::GitDiffs {
        staged: vec![],
        changed: merlin_core::git::parse(changed),
    });
    s.send(Event::ShowDiff {
        path: "src/palette.rs".into(),
        old: HEAD_PALETTE.into(),
        new: format!("X{NOW_PALETTE}"),
    });
    let marks = s.editor.text_views()[1].diff.clone().unwrap();
    assert!(marks.rows.iter().any(|m| m.line.is_some()), "{marks:?}");
    // Undo in the tab undoes in the file.
    s.run("key ctrl+z");
    let views = s.editor.text_views();
    let new_side: String = views[1]
        .lines
        .iter()
        .map(|l| l.chars.iter().collect::<String>() + "\n")
        .collect();
    assert!(!new_side.contains("X/// A colour"), "{new_side}");
    // The last commit's side is read-only.
    s.run("key down down");
    s.run("type Y");
    assert!(s.screen().contains("can't be edited"), "{}", s.screen());
    s.run("key esc");
    assert_eq!(s.text(), NOW_PALETTE);
}

#[test]
fn a_gutter_bar_opens_its_change_in_place() {
    let mut s = Session::new(NOW_PALETTE, Some("src/palette.rs"), 100, 20);
    s.send(Event::GitBase {
        path: "src/palette.rs".into(),
        text: Some(HEAD_PALETTE.into()),
    });
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![("src/palette.rs".into(), 'M')],
        staged: vec![],
        changed: vec![("src/palette.rs".into(), 'M')],
    });
    s.send(Event::GitDiffs {
        staged: vec![],
        changed: merlin_core::git::parse(PALETTE_DIFF),
    });
    // The bar beside line 3, the changed one.
    s.run("click 0 2");
    let screen = s.screen();
    assert!(
        screen.contains(
            "┌ Change 2 of 3 · since the last commit  [Stage] [Revert] ↑ ↓ ✕\n\
             │ -     accent + step\n\
             │ +     accent + step * 2"
        ),
        "{screen}"
    );
    // Stage stages that change alone.
    s.send(Event::Ui(merlin_core::UiEvent::ChangeStage));
    let got = applied(&s);
    assert_eq!(got.len(), 1, "{:?}", s.effects);
    assert!(
        got[0]
            .0
            .contains("-    accent + step\n+    accent + step * 2\n")
    );
    assert!(!got[0].0.contains("A colour"));
    // ↓ goes to the next change, with the cursor.
    s.send(Event::Ui(merlin_core::UiEvent::ChangeStep(true)));
    assert!(s.screen().contains("┌ Change 3 of 3"), "{}", s.screen());
    assert_eq!(s.editor.cursor_utf16().0, 6);
    // Revert puts its line back, as an edit Ctrl+Z undoes.
    s.send(Event::Ui(merlin_core::UiEvent::ChangeRevert));
    assert!(
        s.text().contains("let steps = [1u32, 2, 3];"),
        "{}",
        s.text()
    );
    assert!(!s.screen().contains("┌ Change"));
    s.run("key ctrl+z");
    assert!(s.text().contains("[1u32, 2, 3, 4]"));
    // Alt+F3 goes to the change before, and Esc closes an open one.
    s.run("key alt+f3");
    assert_eq!(s.editor.cursor_utf16().0, 2);
    s.run("click 0 0");
    assert!(s.screen().contains("┌ Change 1 of 3"), "{}", s.screen());
    s.run("key esc");
    assert!(!s.screen().contains("┌ Change"));
}

#[test]
fn the_sync_button_says_what_it_will_do() {
    use merlin_core::{SyncKind, SyncOutcome, SyncState};
    let mut s = session();
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![],
        staged: vec![],
        changed: vec![],
    });
    let sync = |s: &mut Session, upstream: Option<&str>, ahead, behind| {
        s.send(Event::GitSync(SyncState {
            upstream: upstream.map(str::to_string),
            ahead,
            behind,
            remote: Some("origin".into()),
        }))
    };
    s.run("key ctrl+shift+g");
    sync(&mut s, Some("origin/main"), 1, 0);
    assert!(
        s.screen().contains("[✓ Commit] [↑1 Push]"),
        "{}",
        s.screen()
    );
    assert!(
        s.screen().contains(" ↓0 ↑1 "),
        "the status bar: {}",
        s.screen()
    );
    sync(&mut s, Some("origin/main"), 0, 2);
    assert!(s.screen().contains("[↓2 Pull]"), "{}", s.screen());
    sync(&mut s, Some("origin/main"), 1, 2);
    assert!(s.screen().contains("[↓2 ↑1 Sync]"), "{}", s.screen());
    sync(&mut s, None, 0, 0);
    assert!(s.screen().contains("[⇡ Publish Branch]"), "{}", s.screen());
    sync(&mut s, Some("origin/main"), 1, 0);
    // A click pushes, and says so while it runs; another click cancels.
    s.effects.clear();
    s.run("key ctrl+shift+p; type Git: Push; key enter");
    assert!(
        s.effects
            .contains(&Effect::Git(GitAction::Sync(SyncKind::Push)))
    );
    assert!(s.screen().contains("[⟳ Pushing…]"), "{}", s.screen());
    // Rejected: it says why, with Pull and Push.
    sync(&mut s, Some("origin/main"), 1, 2);
    s.send(Event::Synced {
        kind: SyncKind::Push,
        outcome: SyncOutcome::Rejected,
        output: " ! [rejected]        main -> main (fetch first)\n".into(),
    });
    let screen = s.screen();
    assert!(
        screen.contains("The push was rejected: origin has 2 commits this branch doesn't."),
        "{screen}"
    );
    assert!(screen.contains("Pull and Push"), "{screen}");
    s.effects.clear();
    s.send(Event::Ui(merlin_core::UiEvent::ToastButton(1)));
    assert!(
        s.effects
            .contains(&Effect::Git(GitAction::Sync(SyncKind::Sync)))
    );
    // A remote that wants a password says what to set up.
    s.send(Event::Synced {
        kind: SyncKind::Sync,
        outcome: SyncOutcome::NeedsAuth {
            remote: "origin".into(),
        },
        output: String::new(),
    });
    assert!(
        s.screen()
            .contains("origin wants a password. Set up an SSH key or a git credential helper"),
        "{}",
        s.screen()
    );
    // Conflicts offer Resolve and Abort Pull.
    s.send(Event::Synced {
        kind: SyncKind::Pull,
        outcome: SyncOutcome::Conflicts(vec!["src/main.rs".into(), "a.rs".into()]),
        output: String::new(),
    });
    let screen = s.screen();
    assert!(
        screen.contains("The pull stopped on conflicts in 2 files."),
        "{screen}"
    );
    s.effects.clear();
    s.send(Event::Ui(merlin_core::UiEvent::ToastButton(0)));
    assert!(s.effects.contains(&Effect::Git(GitAction::AbortPull)));
}
