//! Milestone 1: selection, clipboard, several files, find and replace,
//! character widths and the mouse.

use merlin_core::script::Session;
use merlin_core::{Button, Effect, Event, Focus, Node, Panel};

fn session(text: &str) -> Session {
    Session::new(text, Some("notes.txt"), 40, 6)
}

#[test]
fn shift_arrows_select_and_typing_replaces_the_selection() {
    let mut s = session("hello world");
    s.run("key ctrl+right shift+ctrl+right");
    assert_eq!(s.editor.selection().as_deref(), Some(" world"));
    s.run("type !");
    assert_eq!(s.text(), "hello!");
    s.run("key ctrl+z");
    assert_eq!(
        s.text(),
        "hello world",
        "the replacement is its own undo step"
    );
}

#[test]
fn left_collapses_a_selection_to_its_start() {
    let mut s = session("abcdef");
    s.run("key right shift+right shift+right left; type X");
    assert_eq!(s.text(), "aXbcdef");
}

#[test]
fn cut_and_paste_use_the_editor_clipboard_when_the_system_one_is_unreadable() {
    let mut s = session("one two");
    s.run("key shift+ctrl+right; key ctrl+x");
    assert_eq!(s.text(), " two");
    assert!(s.effects.contains(&Effect::Copy("one".into())));
    s.run("key ctrl+end ctrl+v");
    assert_eq!(s.text(), " twoone");
}

#[test]
fn copy_without_a_selection_takes_the_line() {
    let mut s = session("first\nsecond");
    s.run("key ctrl+c down home ctrl+v");
    assert_eq!(s.text(), "first\nfirst\nsecond");
}

#[test]
fn select_all_then_delete_empties_the_file() {
    let mut s = session("a\nb\nc");
    s.run("key ctrl+a backspace");
    assert_eq!(s.text(), "");
}

#[test]
fn tab_indents_and_shift_tab_dedents_selected_lines() {
    let mut s = session("a\nb\nc");
    s.run("key shift+down shift+down shift+end tab");
    assert_eq!(s.text(), "    a\n    b\n    c");
    s.run("key backtab");
    assert_eq!(s.text(), "a\nb\nc");
    s.run("key ctrl+z ctrl+z");
    assert_eq!(s.text(), "a\nb\nc", "each is one undo step");
}

#[test]
fn find_ignores_case_unless_the_query_has_capitals() {
    let mut s = session("Alpha alpha");
    s.run("key ctrl+f; type alpha");
    assert_eq!(s.editor.cursor(), 0);
    assert!(s.screen().contains("2 matches"), "{}", s.screen());
    s.run("key backspace backspace backspace backspace backspace; type Alpha; key enter");
    assert_eq!(s.editor.cursor(), 0, "the only exact match");
}

#[test]
fn replace_one_then_all_and_undo_all_at_once() {
    let mut s = Session::new("cat cat cat cat", Some("n.txt"), 60, 8);
    s.run("key ctrl+r; type cat; key tab; type dog; key enter");
    assert_eq!(s.text(), "dog cat cat cat");
    s.run("key alt+enter");
    assert_eq!(s.text(), "dog dog dog dog");
    assert_eq!(s.editor.message(), Some("Replaced 3"));
    s.run("key esc ctrl+z");
    assert_eq!(s.text(), "dog cat cat cat");
}

#[test]
fn find_starts_from_the_selected_text() {
    let mut s = session("foo bar foo");
    s.run("key shift+right shift+right shift+right ctrl+f enter");
    assert_eq!(s.editor.cursor(), 8);
}

/// The screen row showing the first visible line of text, and those after.
fn text_rows(s: &Session) -> Vec<String> {
    let top = s.editor.text_origin().1 as usize;
    s.screen().lines().skip(top).map(String::from).collect()
}

#[test]
fn tabs_and_wide_characters_take_their_real_width() {
    let mut s = session("\tx\n日本y");
    let screen = s.screen();
    assert!(
        screen.contains("  1 │   x"),
        "tab to column 4, with a guide:\n{screen}"
    );
    assert!(screen.contains("  2 日本y"), "{screen}");
    s.run("key down end");
    // The cursor is drawn after two double-width characters and the y.
    let (x, y) = s.editor.text_origin();
    assert_eq!(s.editor.render().cursor, Some((x + 5, y + 1)));
}

#[test]
fn long_lines_scroll_sideways() {
    // No repeats, so where it's scrolled to is unambiguous.
    let line = (0..60).map(|i| format!("{i:02}")).collect::<String>();
    let mut s = session(&line);
    s.run("key end");
    let screen = s.screen();
    assert!(text_rows(&s)[0].ends_with("5859"), "{screen}");
    assert!(!screen.contains("  1 abc"), "{screen}");
    s.run("key home");
    assert!(s.screen().contains("  1 000102"));
}

fn project() -> Session {
    let mut s = Session::new("", None, 60, 10);
    s.files
        .insert("src/main.rs".into(), "fn main() {}\n".into());
    s.files
        .insert("src/editor.rs".into(), "struct Editor;\n".into());
    s.files.insert("README.md".into(), "# hi\n".into());
    s
}

#[test]
fn opening_a_file_replaces_the_blank_scratch_buffer() {
    let mut s = project();
    s.run("key ctrl+o; type edrs; key enter");
    assert_eq!(s.text(), "struct Editor;\n");
    assert_eq!(s.editor.open_files().len(), 1);
}

#[test]
fn open_files_get_tabs_and_switch() {
    let mut s = project();
    s.run("key ctrl+o; type main; key enter; key ctrl+o; type readme; key enter");
    let screen = s.screen();
    assert!(screen.contains(" ◆ main.rs    ≡ README.md ×"), "{screen}");
    s.run("key ctrl+pageup");
    assert_eq!(s.text(), "fn main() {}\n");
    s.run("key ctrl+o; type readme; key enter");
    assert_eq!(
        s.editor.open_files().len(),
        2,
        "already open, so it switches"
    );
    assert_eq!(s.text(), "# hi\n");
}

#[test]
fn closing_an_unsaved_file_needs_a_second_press() {
    let mut s = project();
    s.run("key ctrl+o; type main; key enter; key ctrl+o; type readme; key enter");
    s.run("type x; key ctrl+w");
    assert_eq!(s.editor.open_files().len(), 2);
    s.run("key ctrl+w");
    assert_eq!(s.editor.open_files().len(), 1);
    assert_eq!(s.text(), "fn main() {}\n");
}

#[test]
fn quitting_counts_unsaved_files_across_tabs() {
    let mut s = project();
    s.run("key ctrl+o; type main; key enter; type a; key ctrl+o; type readme; key enter; type b");
    s.run("key ctrl+q");
    assert!(!s.quit);
    assert_eq!(
        s.editor.message(),
        Some("2 files have unsaved changes. Press Ctrl+Q again to quit without saving.")
    );
}

#[test]
fn saving_marks_the_file_that_was_saved() {
    let mut s = project();
    s.run("key ctrl+o; type main; key enter; type a; key ctrl+s");
    assert_eq!(s.files["src/main.rs"], "afn main() {}\n");
    assert!(!s.editor.buffer().is_dirty());
}

#[test]
fn clicking_places_the_cursor_and_dragging_selects() {
    let mut s = session("hello\nworld");
    let (x, y) = s.editor.text_origin();
    s.run(&format!("click {} {}", x + 2, y + 1));
    assert_eq!(s.editor.cursor(), 8);
    s.run(&format!(
        "press {x} {y}; drag {} {y}; release {} {y}",
        x + 5,
        x + 5
    ));
    assert_eq!(s.editor.selection().as_deref(), Some("hello"));
    s.run(&format!("shift-click {} {}", x + 3, y + 1));
    assert_eq!(s.editor.selection().as_deref(), Some("hello\nwor"));
}

#[test]
fn the_wheel_scrolls_without_snapping_back_on_plugin_updates() {
    let mut s = session(&(1..=30).map(|n| format!("line {n}\n")).collect::<String>());
    s.run("wheel down 5 2; wheel down 5 2");
    assert!(text_rows(&s)[0].starts_with("  7 line 7"), "{}", s.screen());
    s.send(Event::SetPanel {
        plugin: "p".into(),
        panel: None,
    });
    assert!(text_rows(&s)[0].starts_with("  7 line 7"), "{}", s.screen());
    s.run("key down");
    assert!(
        text_rows(&s)[0].starts_with("  2 line 2"),
        "moving the cursor brings it back"
    );
}

#[test]
fn clicking_a_panel_button_presses_it() {
    let mut s = Session::new("", None, 60, 8);
    s.send(Event::SetPanel {
        plugin: "demo".into(),
        panel: Some(Panel {
            title: "Demo".into(),
            nodes: vec![Node::Buttons(vec![
                Button {
                    id: "one".into(),
                    label: "One".into(),
                },
                Button {
                    id: "two".into(),
                    label: "Two".into(),
                },
            ])],
        }),
    });
    s.run("key ctrl+alt+b esc");
    let screen = s.screen();
    let (y, row) = screen
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("Two"))
        .unwrap();
    let x = row
        .chars()
        .take_while(|_| true)
        .collect::<String>()
        .find("Two")
        .unwrap();
    let x = row[..x].chars().count();
    s.run(&format!("click {x} {y}"));
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginClick {
            plugin: "demo".into(),
            button: "two".into()
        })
    );
    assert_eq!(s.editor.focus(), Focus::Panel);
}

#[test]
fn clicking_a_palette_item_runs_it() {
    let mut s = project();
    // The frame and input take rows 1 and 2; items start on row 3.
    s.run("key ctrl+o; click 10 4");
    assert_eq!(s.editor.open_files().len(), 1);
    assert_ne!(s.text(), "", "opened the second file in the list");
    assert_eq!(s.editor.focus(), Focus::Editor);
}

#[test]
fn clicking_a_tab_switches_to_it() {
    let mut s = project();
    s.run("key ctrl+o; type main; key enter; key ctrl+o; type readme; key enter");
    // Tabs are on the row below the menu bar, right of the activity bar.
    s.run("click 6 1");
    assert_eq!(s.text(), "fn main() {}\n");
}
