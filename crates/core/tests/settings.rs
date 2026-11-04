//! Settings: applying them, keys bound in them, and menus that change them.

use merlin_core::script::Session;
use merlin_core::{Effect, Event, Settings, UserKey};

#[test]
fn settings_turn_things_on_and_off() {
    let mut s = Session::new("fn a() {}\n", Some("src/a.rs"), 80, 12);
    s.send(Event::Settings(Settings::default()));
    assert!(
        s.screen().contains("src › ◆ a.rs"),
        "breadcrumbs are on: {}",
        s.screen()
    );
    s.send(Event::Settings(Settings {
        breadcrumbs: false,
        minimap: true,
        ..Settings::default()
    }));
    assert!(!s.screen().contains("src ›"), "{}", s.screen());
    assert!(s.editor.minimap());
}

#[test]
fn keys_in_the_settings_run_commands_or_nothing() {
    let mut s = Session::new("one\n", Some("a.txt"), 80, 12);
    s.send(Event::Settings(Settings {
        keys: vec![
            UserKey::new("ctrl+k", "view.minimap"),
            UserKey::new("ctrl+a", ""),
        ],
        ..Settings::default()
    }));
    s.run("key ctrl+k");
    assert!(s.editor.minimap());
    s.run("key ctrl+a; type x");
    assert_eq!(s.text(), "xone\n", "ctrl+a no longer selects all");
}

#[test]
fn unknown_keys_are_reported() {
    let mut s = Session::new("one\n", Some("a.txt"), 80, 12);
    s.send(Event::Settings(Settings {
        keys: vec![UserKey::new("hyper+q", "view.minimap")],
        ..Settings::default()
    }));
    assert!(s.editor.message().unwrap_or_default().contains("hyper+q"));
}

#[test]
fn settings_bind_chords_and_keys_that_apply_only_sometimes() {
    let mut s = Session::new("one two\n", Some("a.txt"), 80, 12);
    s.send(Event::Settings(Settings {
        keys: vec![
            UserKey::new("ctrl+k ctrl+m", "view.minimap"),
            // Only with something selected: otherwise Ctrl+L is still
            // VS Code's.
            UserKey {
                keys: "ctrl+l".into(),
                command: "edit.delete_line".into(),
                when: Some("editorTextFocus && editorHasSelection".into()),
            },
        ],
        ..Settings::default()
    }));
    s.run("key ctrl+k; key ctrl+m");
    assert!(s.editor.minimap(), "the chord ran");
    s.run("key ctrl+l");
    assert_eq!(
        s.text(),
        "one two\n",
        "no selection: Ctrl+L selects the line"
    );
    assert_eq!(s.editor.selection().as_deref(), Some("one two\n"));
    s.run("key ctrl+l");
    assert_eq!(s.text(), "", "with one, it deletes the line");
    // Menus show the keys the settings gave.
    let minimap = s
        .editor
        .key_bindings()
        .into_iter()
        .find(|b| b.command == "view.minimap")
        .unwrap();
    assert_eq!(minimap.keys[0].1, "Ctrl+K Ctrl+M");
    assert!(minimap.changed);
}

#[test]
fn bad_conditions_are_reported() {
    let mut s = Session::new("one\n", Some("a.txt"), 80, 12);
    s.send(Event::Settings(Settings {
        keys: vec![UserKey {
            keys: "ctrl+k".into(),
            command: "view.minimap".into(),
            when: Some("editorTextFocus &&".into()),
        }],
        ..Settings::default()
    }));
    assert!(
        s.editor
            .message()
            .unwrap_or_default()
            .contains("empty part")
    );
}

#[test]
fn the_keys_list_shows_chords() {
    let s = Session::new("one\n", Some("a.txt"), 80, 12);
    let fold_all = s
        .editor
        .key_bindings()
        .into_iter()
        .find(|b| b.command == "edit.fold_all")
        .unwrap();
    assert_eq!(
        fold_all.keys,
        [("ctrl+k ctrl+0".into(), "Ctrl+K Ctrl+0".into())]
    );
}

#[test]
fn toggling_from_the_view_menu_saves_the_setting() {
    let mut s = Session::new("one\n", Some("a.txt"), 80, 12);
    let fx = s.editor.update(Event::Command("view.minimap".into()));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::SetSetting { key, value }
        if key == "editor.minimap" && value == "true")),
        "{fx:?}"
    );
}

#[test]
fn preview_tabs_can_be_turned_off() {
    let mut s = Session::new("fn a() {}\n", Some("main.rs"), 100, 20);
    s.files.insert("main.rs".into(), s.text());
    s.files.insert("one.md".into(), "one\n".into());
    s.files.insert("two.md".into(), "two\n".into());
    s.send(Event::Settings(Settings {
        preview_tabs: false,
        breadcrumbs: false,
        ..Settings::default()
    }));
    s.run("key ctrl+shift+e; key esc");
    for name in ["one.md", "two.md"] {
        let screen = s.screen();
        let (y, row) = screen
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains(name))
            .unwrap();
        let x = row[..row.find(name).unwrap()].chars().count();
        s.run(&format!("click {x} {y}"));
    }
    assert_eq!(s.editor.open_files().len(), 3);
}
