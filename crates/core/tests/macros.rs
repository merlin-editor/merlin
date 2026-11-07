//! Macros from the palette, with VS Code's keys: recorded, played and
//! saved.

use merlin_core::script::Session;
use merlin_core::{Effect, Event, Macro, Settings};

#[test]
fn records_typing_and_palette_commands_then_plays_them() {
    let mut s = Session::new("one\ntwo\nthree", Some("notes.rs"), 60, 12);
    s.run("key ctrl+alt+r; key end; type !");
    // The palette's command goes in, not the keys that ran it.
    s.run("key ctrl+shift+p; type toggle line comment; key enter");
    s.run("key down; key home; key ctrl+alt+r");
    assert_eq!(s.text(), "// one!\ntwo\nthree");
    assert!(s.editor.window_view().recording.is_none());
    s.run("key ctrl+alt+p");
    assert_eq!(s.text(), "// one!\n// two!\nthree");
    s.run("key ctrl+z");
    assert_eq!(s.text(), "// one!\ntwo\nthree", "one undo takes the run");
}

#[test]
fn shows_the_keys_while_recording() {
    let mut s = Session::new("x", Some("notes.rs"), 60, 12);
    s.run("key ctrl+alt+r; type ab; key esc");
    let rec = s.editor.window_view().recording.expect("recording");
    assert_eq!(rec.keys, "ab⎋");
    assert_eq!(rec.hint, "Ctrl+Alt+R stops");
}

#[test]
fn saves_the_last_macro_by_name_as_a_command() {
    let mut s = Session::new("a\nb", Some("notes.rs"), 60, 12);
    s.run("key ctrl+alt+r; type >; key ctrl+alt+r");
    s.run("key ctrl+shift+p; type save the last macro; key enter");
    s.run("type Quote it; key enter");
    let saved = Macro {
        name: "Quote it".into(),
        keys: ">".into(),
    };
    assert!(s.effects.contains(&Effect::SaveMacro(saved.clone())));
    s.run("key down; key home; key ctrl+shift+p; type macro: quote it; key enter");
    assert_eq!(s.text(), ">a\n>b");
    // And from the settings, next time.
    let mut s = Session::new("c", Some("notes.rs"), 60, 12);
    s.send(Event::Settings(Settings {
        macros: vec![saved],
        ..Settings::default()
    }));
    s.run("key ctrl+shift+p; type macro: quote it; key enter");
    assert_eq!(s.text(), ">c");
}
