//! Vim's keys: modes, motions, operators, text objects, registers, the
//! dot and undo.

use merlin_core::script::Session;
use merlin_core::{Event, Key, KeyCode, Keymap, Settings, VimMode, VimSettings};

fn vim(text: &str) -> Session {
    let mut s = Session::new(text, Some("notes.rs"), 60, 12);
    s.send(Event::Settings(Settings {
        keymap: Keymap::Vim,
        ..Settings::default()
    }));
    s
}

fn mode(s: &Session) -> VimMode {
    s.editor.window_view().vim.expect("vim is on").mode
}

#[test]
fn normal_mode_moves_and_insert_mode_types() {
    let mut s = vim("hello world");
    assert_eq!(mode(&s), VimMode::Normal);
    s.run("type wix; key esc");
    assert_eq!(s.text(), "hello xworld");
    assert_eq!(mode(&s), VimMode::Normal);
    // Esc goes back over the letter typed.
    assert_eq!(s.editor.cursor(), 6);
    s.run("type Ab; key esc");
    assert_eq!(s.text(), "hello xworldb");
}

#[test]
fn operators_take_motions_counts_and_doubles() {
    let mut s = vim("one two three four\nfive\nsix");
    s.run("type dw");
    assert_eq!(s.text(), "two three four\nfive\nsix");
    s.run("type 2dw");
    assert_eq!(s.text(), "four\nfive\nsix");
    s.run("type dd");
    assert_eq!(s.text(), "five\nsix");
    s.run("type p");
    assert_eq!(
        s.text(),
        "five\nfour\nsix",
        "dd puts the line in the register"
    );
    s.run("type ggd$");
    assert_eq!(s.text(), "\nfour\nsix");
}

#[test]
fn dw_on_a_lines_last_word_keeps_the_line_break() {
    let mut s = vim("alpha beta\ngamma");
    s.run("type wdw");
    assert_eq!(s.text(), "alpha \ngamma");
}

#[test]
fn change_word_then_dot_repeats_it() {
    let mut s = vim("foo bar baz");
    s.run("type cwqux; key esc; type w.");
    assert_eq!(s.text(), "qux qux baz");
    s.run("type u");
    assert_eq!(s.text(), "qux bar baz", "u undoes the whole change");
    s.run("type u");
    assert_eq!(s.text(), "foo bar baz");
    s.run("key ctrl+r");
    assert_eq!(s.text(), "qux bar baz");
}

#[test]
fn text_objects_inside_brackets_and_quotes() {
    let mut s = vim("call(a, b) \"text here\"");
    s.run("type fbdi(");
    assert_eq!(s.text(), "call() \"text here\"");
    s.run("type fhci\"x; key esc");
    assert_eq!(s.text(), "call() \"x\"");
    s.run("type 0daw");
    assert_eq!(s.text(), "() \"x\"");
}

#[test]
fn find_on_the_line_and_repeat_it() {
    let mut s = vim("a,b,c,d");
    s.run("type f,");
    // The script's `;` separates its steps, so this one is sent.
    s.send(Event::Key(Key::plain(KeyCode::Char(';'))));
    assert_eq!(s.editor.cursor(), 3);
    s.run("type dt,");
    assert_eq!(s.text(), "a,b,d");
}

#[test]
fn visual_mode_selects_and_operates() {
    let mut s = vim("one two three");
    s.run("type vee");
    assert_eq!(mode(&s), VimMode::Visual);
    assert_eq!(s.editor.selection().as_deref(), Some("one two"));
    s.run("type d");
    assert_eq!(s.text(), " three");
    assert_eq!(mode(&s), VimMode::Normal);
    let mut s = vim("a\nb\nc\nd");
    s.run("type Vjy");
    s.run("type Gp");
    assert_eq!(s.text(), "a\nb\nc\nd\na\nb");
}

#[test]
fn visual_block_changes_every_line() {
    let mut s = vim("abc\nabc\nabc");
    s.run("key ctrl+v; type jjlc; type X; key esc");
    assert_eq!(s.text(), "Xc\nXc\nXc");
}

#[test]
fn registers_keep_yanks_apart_from_deletes() {
    let mut s = vim("keep\ngone");
    s.run("type yyjdd\"0p");
    assert_eq!(s.text(), "keep\nkeep");
    let mut s = vim("word");
    s.run("type \"ayiw$\"ap");
    assert_eq!(s.text(), "wordword");
}

#[test]
fn join_replace_and_case() {
    let mut s = vim("one\n    two\nthree");
    s.run("type J");
    assert_eq!(s.text(), "one two\nthree");
    s.run("type 0rX");
    assert_eq!(s.text(), "Xne two\nthree");
    s.run("type g~iw");
    assert_eq!(s.text(), "xNE two\nthree");
    s.run("type j0gUU");
    assert_eq!(s.text(), "xNE two\nTHREE");
}

#[test]
fn indent_and_comment_lines() {
    let mut s = vim("fn a() {}\nfn b() {}");
    s.run("type >>");
    assert_eq!(s.text(), "    fn a() {}\nfn b() {}");
    s.run("type gcj");
    // At the lines' least indent, as merlin comments.
    assert_eq!(s.text(), "//     fn a() {}\n// fn b() {}");
}

#[test]
fn numbers_go_up_and_down() {
    let mut s = vim("width = 41;");
    s.run("key ctrl+a");
    assert_eq!(s.text(), "width = 42;");
    s.run("type 50");
    s.run("key ctrl+x");
    assert_eq!(s.text(), "width = -8;");
}

#[test]
fn jk_leaves_insert_mode_when_set() {
    let mut s = Session::new("", Some("notes.rs"), 60, 12);
    s.send(Event::Settings(Settings {
        keymap: Keymap::Vim,
        vim: VimSettings {
            insert_escape: "jk".into(),
            ..VimSettings::default()
        },
        ..Settings::default()
    }));
    s.run("type ihijk");
    assert_eq!(s.text(), "hi");
    assert_eq!(mode(&s), VimMode::Normal);
}

#[test]
fn kept_ctrl_keys_do_merlins_thing() {
    let mut s = vim("text");
    s.run("key ctrl+shift+p");
    assert!(
        s.editor.window_view().palette.is_some(),
        "the palette opens"
    );
}

#[test]
fn the_status_bar_shows_the_mode_and_keys_so_far() {
    let mut s = vim("text");
    s.run("type 2d");
    let view = s.editor.window_view().vim.expect("vim");
    assert_eq!(view.pending, "2d");
    assert!(
        s.editor
            .window_view()
            .status
            .iter()
            .any(|i| i.text == "Keymap: Vim")
    );
}

#[test]
fn turning_it_off_types_as_before() {
    let mut s = vim("x");
    s.run("key ctrl+shift+p; type toggle vim; key enter");
    assert!(s.editor.window_view().vim.is_none());
    s.run("type ab");
    assert_eq!(s.text(), "abx");
}

#[test]
fn several_cursors_each_follow_the_keys() {
    let mut s = vim("one two\none two");
    s.run("key shift+alt+down; type dw");
    assert_eq!(s.text(), "two\ntwo");
}

#[test]
fn colon_s_replaces_in_the_line_or_the_file() {
    let mut s = vim("pinned pinned\npinned");
    s.run("type :s/pinned/kept/");
    let line = s.editor.window_view().vim_line.expect("the line is open");
    assert!(line.hint.starts_with("1 in 1 line"), "{}", line.hint);
    s.run("key enter");
    assert_eq!(s.text(), "kept pinned\npinned");
    s.run("type :%s/pinned/kept/g; key enter");
    assert_eq!(s.text(), "kept kept\nkept");
    s.run("type u");
    assert_eq!(s.text(), "kept pinned\npinned", "one undo takes it back");
}

#[test]
fn colon_s_in_visual_mode_takes_the_selection() {
    let mut s = vim("a\na\na");
    s.run("type Vj:");
    assert_eq!(s.editor.window_view().vim_line.unwrap().text, "'<,'>");
    s.run("type s/a/b/; key enter");
    assert_eq!(s.text(), "b\nb\na");
}

#[test]
fn the_preview_strikes_out_what_goes() {
    let mut s = vim("let pinned = 1;");
    s.run("type :s/pinned/kept/");
    let view = &s.editor.text_views()[0];
    let line = &view.lines[0];
    assert_eq!(line.styles[4], merlin_core::style::Style::Replaced);
    assert!(line.inlays.iter().any(|i| i.label == "kept" && i.col == 10));
    s.run("key esc");
    assert!(s.editor.window_view().vim_line.is_none());
    assert_eq!(s.text(), "let pinned = 1;", "Esc leaves the text as it was");
}

#[test]
fn search_moves_as_typed_and_n_goes_on() {
    let mut s = vim("one two one two one");
    s.run("type /two");
    assert_eq!(s.editor.cursor(), 4, "the first match as it's typed");
    s.run("key enter");
    assert_eq!(
        s.editor.window_view().vim.unwrap().note.as_deref(),
        Some("1 of 2")
    );
    s.run("type n");
    assert_eq!(s.editor.cursor(), 12);
    s.run("type N");
    assert_eq!(s.editor.cursor(), 4);
    s.run("type /nothing; key esc");
    assert_eq!(s.editor.cursor(), 4, "Esc goes back");
    s.run("type ?one; key enter");
    assert_eq!(s.editor.cursor(), 0);
}

#[test]
fn line_commands_take_ranges() {
    let mut s = vim("c\na\nb\na");
    s.run("type :sort u; key enter");
    assert_eq!(s.text(), "a\nb\nc");
    s.run("type :2d; key enter");
    assert_eq!(s.text(), "a\nc");
    s.run("type :g/c/d; key enter");
    assert_eq!(s.text(), "a");
    let mut s = vim("x\ny\nz");
    s.run("type :1m$; key enter");
    assert_eq!(s.text(), "y\nz\nx");
    s.run("type :%norm Ai; key enter");
    assert_eq!(s.text(), "yi\nzi\nxi");
    s.run("type :2; key enter");
    assert_eq!(s.editor.cursor(), 3);
}

#[test]
fn merlins_commands_are_there_by_name() {
    let mut s = vim("text");
    s.run("type :minimap");
    let line = s.editor.window_view().vim_line.unwrap();
    assert!(
        line.items
            .iter()
            .any(|i| i.merlin && i.name.contains("Minimap")),
        "{:?}",
        line.items
    );
    s.run("key enter");
    assert!(s.effects.iter().any(
        |e| matches!(e, merlin_core::Effect::SetSetting { key, .. } if key == "editor.minimap")
    ));
}

#[test]
fn up_brings_back_the_last_lines() {
    let mut s = vim("a a");
    s.run("type :s/a/b/; key enter");
    s.run("type :; key up");
    assert_eq!(s.editor.window_view().vim_line.unwrap().text, "s/a/b/");
}

#[test]
fn write_and_quit() {
    let mut s = vim("text");
    s.run("type x:w; key enter");
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, merlin_core::Effect::Save { .. }))
    );
}

fn which(s: &mut Session) -> Option<merlin_core::WhichKeyView> {
    // The window would wait the delay; the test sends the event at once.
    if let Some((tick, _)) = s.editor.which_key_wait() {
        s.send(Event::WhichKey(tick));
    }
    s.editor.window_view().which_key
}

#[test]
fn which_key_lists_the_keys_after_a_prefix() {
    let mut s = vim("text");
    s.run("type g");
    let card = which(&mut s).expect("a card after g");
    assert_eq!(card.chord, "g");
    assert!(
        card.items
            .iter()
            .any(|i| i.key == "d" && i.label == "Go to Definition")
    );
    s.run("type g");
    assert!(s.editor.window_view().which_key.is_none(), "a key hides it");
}

#[test]
fn a_key_before_the_pause_ends_shows_nothing() {
    let mut s = vim("text");
    s.run("type g");
    let (tick, _) = s.editor.which_key_wait().expect("waiting");
    s.run("type g");
    s.send(Event::WhichKey(tick));
    assert!(s.editor.window_view().which_key.is_none());
}

#[test]
fn the_leader_has_groups_and_runs_commands() {
    let mut s = vim("text");
    // The script trims a space, so the leader is sent.
    s.send(Event::Key(Key::plain(KeyCode::Char(' '))));
    let card = which(&mut s).expect("the leader's card");
    assert!(card.items.iter().any(|i| i.key == "f" && i.group));
    s.run("type f");
    let card = which(&mut s).expect("the files group");
    assert_eq!(card.chord, "Space f");
    s.run("key backspace");
    let card = which(&mut s).expect("back up a level");
    assert_eq!(card.chord, "Space");
    s.run("type w");
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, merlin_core::Effect::Save { .. })),
        "Space w saves"
    );
}

#[test]
fn plugins_add_leader_keys() {
    let mut s = vim("text");
    s.send(Event::RegisterKeys {
        plugin: "todo".into(),
        keys: vec![
            merlin_core::LeaderKey {
                keys: "x".into(),
                command: None,
                label: "+ todo".into(),
            },
            merlin_core::LeaderKey {
                keys: "x n".into(),
                command: Some("next".into()),
                label: "Next TODO".into(),
            },
        ],
    });
    s.send(Event::Key(Key::plain(KeyCode::Char(' '))));
    s.run("type x");
    let card = which(&mut s).expect("the plugin's group");
    assert_eq!(card.items[0].plugin.as_deref(), Some("todo"));
    s.run("type n");
    assert!(s.effects.contains(&merlin_core::Effect::PluginCommand {
        plugin: "todo".into(),
        command: "next".into()
    }));
}

#[test]
fn registers_and_operators_have_cards() {
    let mut s = vim("keep this");
    s.run("type yiw\"");
    let card = which(&mut s).expect("registers");
    let zero = card.items.iter().find(|i| i.key == "0").expect("the yank");
    assert_eq!(zero.preview.as_deref(), Some("keep"));
    s.run("key esc; type d");
    let card = which(&mut s).expect("motions");
    assert!(
        card.items
            .iter()
            .any(|i| i.key == "d" && i.label == "The whole line")
    );
}

#[test]
fn ctrl_k_chords_have_a_card_too() {
    let mut s = Session::new("text", Some("notes.rs"), 60, 12);
    s.run("key ctrl+k");
    let card = which(&mut s).expect("the chord's card");
    assert!(
        card.items
            .iter()
            .any(|i| i.label.contains("Format the Selection")),
        "{:?}",
        card.items
    );
}
