//! Saving: trimming whitespace and the final newline, what an
//! `.editorconfig` says, saving by itself, and the colour settings.

use std::path::PathBuf;

use merlin_core::script::Session;
use merlin_core::{AutoSave, AutoSaveWhy, Effect, Event, FileConfig, Settings};

fn session(text: &str, path: &str) -> Session {
    let mut s = Session::new(text, Some(path), 60, 8);
    s.files.insert(path.into(), s.text());
    s
}

fn settings(s: &mut Session, settings: Settings) {
    s.send(Event::Settings(settings));
}

fn saves(s: &Session) -> usize {
    s.effects
        .iter()
        .filter(|e| matches!(e, Effect::Save { .. }))
        .count()
}

#[test]
fn saving_trims_and_ends_with_a_newline_as_one_undo() {
    let mut s = session("a  \n\tb\t\nc", "x.rs");
    settings(
        &mut s,
        Settings {
            trim_trailing_whitespace: true,
            insert_final_newline: true,
            ..Settings::default()
        },
    );
    s.run("key ctrl+s");
    assert_eq!(s.text(), "a\n\tb\nc\n");
    assert_eq!(s.files["x.rs"], "a\n\tb\nc\n", "what's saved is tidied");
    s.run("key ctrl+z");
    assert_eq!(s.text(), "a  \n\tb\t\nc");
}

#[test]
fn nothing_changes_by_default() {
    let mut s = session("a  \nb", "x.rs");
    s.run("key ctrl+s");
    assert_eq!(s.files["x.rs"], "a  \nb");
}

#[test]
fn markdown_keeps_its_trailing_spaces_unless_editorconfig_says() {
    let mut s = session("line  \nnext", "README.md");
    settings(
        &mut s,
        Settings {
            trim_trailing_whitespace: true,
            ..Settings::default()
        },
    );
    s.run("key ctrl+s");
    assert_eq!(s.files["README.md"], "line  \nnext");
    s.send(Event::FileConfig {
        path: PathBuf::from("README.md"),
        config: FileConfig {
            trim_trailing_whitespace: Some(true),
            ..FileConfig::default()
        },
    });
    s.run("key ctrl+s");
    assert_eq!(s.files["README.md"], "line\nnext");
}

#[test]
fn editorconfig_wins_over_the_settings() {
    let mut s = session("fn a() {\n}\n", "a.rs");
    s.send(Event::FileConfig {
        path: PathBuf::from("a.rs"),
        config: FileConfig {
            tabs: Some(false),
            indent_size: Some(2),
            insert_final_newline: Some(false),
            trim_trailing_whitespace: Some(false),
            ..FileConfig::default()
        },
    });
    settings(
        &mut s,
        Settings {
            insert_spaces: false,
            insert_final_newline: true,
            ..Settings::default()
        },
    );
    s.run("key end tab");
    assert_eq!(s.text(), "fn a() {  \n}\n", "two spaces, not a tab");
    s.run("key ctrl+a; type x; key ctrl+s");
    assert_eq!(s.files["a.rs"], "x", "no newline added");
}

#[test]
fn editorconfig_tabs_use_tab_width() {
    let mut s = session("a\n", "Makefile");
    s.send(Event::FileConfig {
        path: PathBuf::from("Makefile"),
        config: FileConfig {
            tabs: Some(true),
            tab_width: Some(8),
            ..FileConfig::default()
        },
    });
    s.run("key end tab");
    assert_eq!(s.text(), "a\t\n");
}

#[test]
fn auto_save_after_a_delay_saves_quietly() {
    let mut s = session("a", "a.rs");
    settings(
        &mut s,
        Settings {
            auto_save: AutoSave::AfterDelay,
            auto_save_delay: 500,
            ..Settings::default()
        },
    );
    assert_eq!(s.editor.auto_save_delay(), Some(500));
    s.run("type b");
    s.send(Event::AutoSave(AutoSaveWhy::Delay));
    assert_eq!(s.files["a.rs"], "ba");
    assert_eq!(s.editor.message(), None, "no notice for each save");
    // Losing the window doesn't save with this setting; nothing's changed
    // anyway.
    s.send(Event::AutoSave(AutoSaveWhy::Window));
    assert_eq!(saves(&s), 1);
}

#[test]
fn saving_by_itself_leaves_trailing_spaces_on_the_cursor_line() {
    let mut s = session("a  \nb", "a.rs");
    settings(
        &mut s,
        Settings {
            trim_trailing_whitespace: true,
            auto_save: AutoSave::AfterDelay,
            ..Settings::default()
        },
    );
    s.run("key down end");
    s.send(Event::Paste("x ".into()));
    s.send(Event::AutoSave(AutoSaveWhy::Delay));
    assert_eq!(s.files["a.rs"], "a\nbx ", "the space just typed stays");
    s.run("key ctrl+s");
    assert_eq!(s.files["a.rs"], "a\nbx", "saving by hand trims it");
}

#[test]
fn auto_save_is_off_by_default() {
    let mut s = session("a", "a.rs");
    assert_eq!(s.editor.auto_save_delay(), None);
    s.run("type b");
    s.send(Event::AutoSave(AutoSaveWhy::Delay));
    s.send(Event::AutoSave(AutoSaveWhy::Window));
    assert_eq!(saves(&s), 0);
}

#[test]
fn untitled_files_never_save_by_themselves() {
    let mut s = Session::new("", None, 60, 8);
    settings(
        &mut s,
        Settings {
            auto_save: AutoSave::OnWindowChange,
            ..Settings::default()
        },
    );
    s.run("type hi");
    s.send(Event::AutoSave(AutoSaveWhy::Window));
    assert_eq!(saves(&s), 0);
    assert!(
        !s.effects.iter().any(|e| matches!(e, Effect::PickFile(_))),
        "and don't ask for a name"
    );
}

#[test]
fn on_focus_change_saves_the_file_left() {
    let mut s = session("one", "a.rs");
    s.files.insert("b.rs".into(), "two".into());
    s.send(Event::Settings(Settings {
        auto_save: AutoSave::OnFocusChange,
        ..Settings::default()
    }));
    s.run("type x");
    assert_eq!(saves(&s), 0);
    s.send(Event::Command("file.open_file".into()));
    s.send(Event::Opened {
        path: "b.rs".into(),
        text: "two".into(),
    });
    assert_eq!(s.files["a.rs"], "xone", "switching files saved it");
    s.send(Event::AutoSave(AutoSaveWhy::Window));
    assert_eq!(saves(&s), 1, "b.rs has no changes");
}

#[test]
fn bracket_colours_and_swatches_can_be_turned_off() {
    let mut s = session("f(#ff0000)", "a.css");
    // Brackets are coloured in files a language plugin colours.
    s.send(Event::SetTokens {
        path: "a.css".into(),
        layer: merlin_core::layers::Layer::Base,
        lines: None,
        tokens: Vec::new(),
    });
    let line = |s: &Session| s.editor.text_views().pop().unwrap().lines[0].clone();
    assert!(line(&s).brackets[1].is_some());
    assert_eq!(line(&s).swatches, vec![(2, 7, 0xff0000ff)]);
    settings(
        &mut s,
        Settings {
            bracket_pair_colorization: false,
            color_decorators: false,
            ..Settings::default()
        },
    );
    assert!(line(&s).brackets.iter().all(Option::is_none));
    assert!(line(&s).swatches.is_empty());
}
