//! Behaviour tests, driven by scripts the way a user would drive the editor.
//! Several pin down bug classes reported against ttt.

use merlin_core::script::Session;
use merlin_core::{Button, Effect, Event, Focus, Node, Panel, PickItem, Settings};

fn session(text: &str) -> Session {
    Session::new(text, Some("notes.txt"), 40, 6)
}

#[test]
fn renders_text_gutter_and_status() {
    let mut s = session("");
    s.run("type fn main() {\\nlet x = 1\\;");
    assert_eq!(
        s.editor.text_rows(),
        ["  1 fn main() {", "  2     let x = 1;", "  3 }"]
    );
    let screen = s.screen();
    assert!(screen.contains("tabs: [notes.txt ●]"), "{screen}");
    assert!(screen.contains("Ln 2, Col 15"), "{screen}");
    assert!(screen.contains("Plain Text"), "{screen}");
}

#[test]
fn enter_keeps_indentation_and_opens_a_block() {
    let mut s = session("");
    s.run("type     if x {\\nbody");
    assert_eq!(s.text(), "    if x {\n        body\n    }");
    s.run("key end enter; type more");
    assert_eq!(s.text(), "    if x {\n        body\n        more\n    }");
}

#[test]
fn typing_is_undone_a_word_at_a_time() {
    let mut s = session("");
    s.run("type one two");
    s.run("key ctrl+z");
    assert_eq!(s.text(), "one");
    s.run("key ctrl+z");
    assert_eq!(s.text(), "");
}

#[test]
fn undo_past_a_save_is_dirty_and_redo_back_is_clean() {
    // ttt#1314: undo could clear the unsaved state and lose edits.
    let mut s = session("");
    s.run("type a\\nb; key ctrl+s");
    assert!(!s.editor.buffer().is_dirty());
    s.run("key ctrl+z");
    assert!(s.editor.buffer().is_dirty(), "text differs from the file");
    s.run("key ctrl+y");
    assert!(!s.editor.buffer().is_dirty(), "text matches the file again");
}

#[test]
fn typing_after_a_save_is_not_merged_into_the_saved_revision() {
    let mut s = session("");
    s.run("type ab; key ctrl+s; type c");
    assert!(s.editor.buffer().is_dirty());
    s.run("key ctrl+z");
    assert_eq!(s.text(), "ab");
    assert!(!s.editor.buffer().is_dirty());
}

#[test]
fn a_slow_save_marks_the_revision_it_wrote() {
    let mut s = session("");
    s.run("type ab");
    let effects = s
        .editor
        .update(Event::Key(merlin_core::Key::parse("ctrl+s").unwrap()));
    let Some(Effect::Save { path, revision, .. }) = effects.into_iter().next() else {
        panic!("expected a save");
    };
    s.run("type \\ncd"); // typed while the write is in flight
    s.send(Event::Saved {
        doc: s.editor.doc_id(),
        revision,
        path,
    });
    assert!(s.editor.buffer().is_dirty());
}

#[test]
fn paste_goes_to_the_focused_find_field() {
    // ttt#1304: Ctrl+V with find open pasted into the editor.
    let mut s = session("alpha beta gamma");
    s.run("key ctrl+f; paste beta");
    assert_eq!(s.text(), "alpha beta gamma");
    assert_eq!(s.editor.cursor(), 6, "jumped to the match");
}

#[test]
fn escape_returns_focus_to_the_editor() {
    // ttt#1305: find kept focus after closing.
    let mut s = session("abc");
    s.run("key ctrl+f; type b; key esc; type X");
    assert_eq!(s.editor.focus(), Focus::Editor);
    assert_eq!(s.text(), "aXbc");
}

#[test]
fn find_uses_fresh_matches_after_edits() {
    // ttt#1317: replace used stale match ranges after edits.
    let mut s = session("x1 x2");
    s.run("key ctrl+f; type x; key esc");
    s.run("key ctrl+home; type xx; key ctrl+f; key enter");
    assert_eq!(s.text(), "xxx1 x2");
    assert_eq!(s.editor.cursor(), 2, "the next x, counting the new ones");
}

#[test]
fn quitting_with_unsaved_changes_asks_first() {
    let mut s = session("");
    s.send(Event::Settings(Settings {
        hot_exit: false,
        ..Settings::default()
    }));
    s.run("type a; key ctrl+q");
    assert!(!s.quit);
    assert!(s.editor.window_view().dialog.is_some());
    // Cancel: nothing happens, and keys reach the text again.
    s.run("key left; key enter");
    assert!(!s.quit && s.editor.window_view().dialog.is_none());
    s.run("type b");
    assert_eq!(s.text(), "ab");
    // Don't Save quits as it is.
    s.run("key ctrl+q; key tab; key enter");
    assert!(s.quit);
    assert!(s.saved.is_empty());
}

#[test]
fn quitting_with_nothing_unsaved_just_quits() {
    let mut s = session("");
    s.run("key ctrl+q");
    assert!(s.quit);
}

#[test]
fn palette_runs_plugin_commands() {
    let mut s = session("");
    s.send(Event::RegisterPlugin {
        plugin: "wc".into(),
        commands: vec![merlin_core::Command {
            id: "count".into(),
            title: "Word Count: Show".into(),
            key: None,
        }],
    });
    s.run("key ctrl+shift+p; type wordc; key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginCommand {
            plugin: "wc".into(),
            command: "count".into()
        })
    );
}

fn panel(buttons: &[&str]) -> Event {
    Event::SetPanel {
        plugin: "demo".into(),
        panel: Some(Panel {
            title: "Demo".into(),
            nodes: buttons
                .iter()
                .map(|b| {
                    Node::Button(Button {
                        id: b.to_string(),
                        label: b.to_string(),
                    })
                })
                .collect(),
        }),
    }
}

#[test]
fn panel_selection_survives_a_rerender() {
    // ttt#1223: plugin state was reapplied on every redraw, so a tree the
    // user collapsed sprang open again. Here the editor owns selection.
    let mut s = session("");
    s.send(panel(&["one", "two", "three"]));
    s.run("key ctrl+alt+b; key down down");
    s.send(panel(&["one", "two", "three"]));
    s.run("key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginClick {
            plugin: "demo".into(),
            button: "three".into()
        })
    );
}

#[test]
fn panel_selection_is_clamped_when_buttons_go_away() {
    let mut s = session("");
    s.send(panel(&["one", "two", "three"]));
    s.run("key ctrl+alt+b; key down down");
    s.send(panel(&["one"]));
    s.run("key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginClick {
            plugin: "demo".into(),
            button: "one".into()
        })
    );
}

#[test]
fn a_failed_plugin_loses_its_commands_and_shows_why() {
    let mut s = session("");
    s.send(panel(&["one"]));
    s.send(Event::PluginFailed {
        plugin: "demo".into(),
        message: "out of fuel".into(),
    });
    s.run("key ctrl+alt+b");
    assert!(s.screen().contains("demo failed"));
    assert!(s.screen().contains("out of fuel"));
}

#[test]
fn view_scrolls_to_keep_the_cursor_visible() {
    let mut s = session(&(1..=20).map(|n| format!("line {n}\n")).collect::<String>());
    s.run("key pagedown pagedown");
    let line = s.text()[..s.editor.cursor()].matches('\n').count() + 1;
    assert!(line > 4, "moved down");
    let screen = s.screen();
    assert!(screen.contains(&format!("line {line}\n")), "{screen}");
    assert!(!screen.contains("line 1\n"), "{screen}");
}

#[test]
fn utf16_columns_account_for_emoji() {
    // ttt#1253: LSP positions were wrong after an emoji.
    use merlin_core::lsp::{char_to_utf16_col, utf16_to_char_col};
    let line = "a😀b";
    assert_eq!(char_to_utf16_col(line, 2), 3);
    assert_eq!(utf16_to_char_col(line, 3), 2);
    assert_eq!(utf16_to_char_col(line, 2), 2, "mid-pair rounds up");
}

fn chat_panel(permission: bool) -> Event {
    let mut nodes = vec![Node::Text("transcript".into()), Node::Footer];
    if permission {
        nodes.push(Node::Buttons(vec![
            Button {
                id: "allow".into(),
                label: "Allow".into(),
            },
            Button {
                id: "reject".into(),
                label: "Reject".into(),
            },
        ]));
    }
    nodes.push(Node::Input {
        id: "prompt".into(),
        placeholder: "Ask…".into(),
        secret: false,
        multiline: false,
    });
    Event::SetPanel {
        plugin: "chat".into(),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes,
        }),
    }
}

#[test]
fn panel_input_keeps_its_text_across_rerenders_and_submits() {
    let mut s = session("");
    s.send(chat_panel(false));
    s.run("key ctrl+alt+b; type hello");
    s.send(chat_panel(false));
    s.run("paste  world; key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginSubmit {
            plugin: "chat".into(),
            input: "prompt".into(),
            text: "hello world".into()
        })
    );
    assert_eq!(s.text(), "", "nothing reached the buffer");
}

#[test]
fn focus_stays_on_the_input_when_buttons_appear_above_it() {
    let mut s = session("");
    s.send(chat_panel(false));
    s.run("key ctrl+alt+b; type ab");
    s.send(chat_panel(true));
    s.run("type c; key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginSubmit {
            plugin: "chat".into(),
            input: "prompt".into(),
            text: "abc".into()
        })
    );
}

#[test]
fn footer_stays_at_the_bottom_and_the_body_follows_new_content() {
    let mut s = Session::new("", None, 60, 8);
    let lines: Vec<Node> = (1..=20)
        .map(|n| Node::Text(format!("message {n}")))
        .collect();
    let nodes = lines
        .into_iter()
        .chain([
            Node::Footer,
            Node::Input {
                id: "p".into(),
                placeholder: "Ask…".into(),
                secret: false,
                multiline: false,
            },
        ])
        .collect();
    s.send(Event::SetPanel {
        plugin: "chat".into(),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes,
        }),
    });
    s.run("key ctrl+alt+b");
    // The window keeps what's after the footer at the bottom and scrolls
    // the rest; the input after it has the focus.
    let view = s.editor.window_view();
    let right = view.right.expect("the panel");
    let footer = right.nodes.iter().position(|n| *n == Node::Footer).unwrap();
    assert!(matches!(right.nodes[footer + 1], Node::Input { .. }));
    assert!(s.screen().contains("› [Ask…]"), "{}", s.screen());
}

#[test]
fn a_plugin_picker_reports_the_choice() {
    let mut s = session("");
    s.send(Event::ShowPicker {
        plugin: "chat".into(),
        id: "model".into(),
        title: "Model".into(),
        items: ["alpha", "beta"]
            .map(|i| PickItem {
                id: i.into(),
                label: i.to_uppercase(),
            })
            .into(),
    });
    s.run("type bet; key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginPick {
            plugin: "chat".into(),
            picker: "model".into(),
            item: "beta".into()
        })
    );
}

#[test]
fn focus_returns_to_the_input_when_the_focused_button_goes_away() {
    // Answering a permission removes its buttons. Focus must go back to the
    // input, not slide onto whatever button now sits in that position.
    let mut s = session("");
    s.send(Event::SetPanel {
        plugin: "chat".into(),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes: vec![
                Node::Footer,
                Node::Buttons(vec![Button {
                    id: "allow".into(),
                    label: "Allow".into(),
                }]),
                Node::Buttons(vec![Button {
                    id: "mode".into(),
                    label: "Mode".into(),
                }]),
                Node::Input {
                    id: "prompt".into(),
                    placeholder: "Ask…".into(),
                    secret: false,
                    multiline: false,
                },
            ],
        }),
    });
    s.run("key ctrl+alt+b; key up up; key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginClick {
            plugin: "chat".into(),
            button: "allow".into()
        })
    );
    s.send(Event::SetPanel {
        plugin: "chat".into(),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes: vec![
                Node::Footer,
                Node::Buttons(vec![Button {
                    id: "mode".into(),
                    label: "Mode".into(),
                }]),
                Node::Input {
                    id: "prompt".into(),
                    placeholder: "Ask…".into(),
                    secret: false,
                    multiline: false,
                },
            ],
        }),
    });
    s.run("type more; key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginSubmit {
            plugin: "chat".into(),
            input: "prompt".into(),
            text: "more".into()
        })
    );
}

fn register(keys: &[(&str, &str)]) -> Event {
    Event::RegisterPlugin {
        plugin: "wc".into(),
        commands: keys
            .iter()
            .map(|(id, key)| merlin_core::Command {
                id: id.to_string(),
                title: format!("Word Count: {id}"),
                key: Some(key.to_string()),
            })
            .collect(),
    }
}

#[test]
fn plugin_keys_run_their_commands_and_show_in_the_palette() {
    let mut s = Session::new("", None, 70, 6);
    s.send(register(&[("show", "ctrl+alt+w")]));
    s.run("key ctrl+alt+w");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginCommand {
            plugin: "wc".into(),
            command: "show".into()
        })
    );
    s.run("key ctrl+shift+p; type word count");
    assert!(s.screen().contains("Ctrl+Alt+W"), "{}", s.screen());
}

#[test]
fn plugin_keys_cannot_take_editor_keys() {
    let mut s = Session::new("", Some("a.txt"), 80, 6);
    s.send(register(&[
        ("save", "ctrl+s"),
        ("plain", "w"),
        ("ok", "alt+x"),
    ]));
    assert_eq!(
        s.editor.message(),
        Some("Plugin wc: ctrl+s is taken, w needs Ctrl or Alt and a letter")
    );
    s.run("key ctrl+s");
    assert!(
        matches!(s.effects.last(), Some(Effect::Save { .. })),
        "the editor keeps Ctrl+S"
    );
}

#[test]
fn secret_inputs_show_dots() {
    let mut s = Session::new("", None, 60, 6);
    s.send(Event::SetPanel {
        plugin: "p".into(),
        panel: Some(Panel {
            title: "Login".into(),
            nodes: vec![Node::Input {
                id: "key".into(),
                placeholder: "API key".into(),
                secret: true,
                multiline: false,
            }],
        }),
    });
    s.run("key ctrl+alt+b; type hunter2");
    assert!(s.screen().contains("› [•••••••]"), "{}", s.screen());
    assert!(!s.screen().contains("hunter2"));
    s.run("key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginSubmit {
            plugin: "p".into(),
            input: "key".into(),
            text: "hunter2".into()
        })
    );
}

#[test]
fn multiline_input_editing_and_cursor_movement() {
    let mut s = Session::new("", None, 60, 10);
    s.send(Event::SetPanel {
        plugin: "p".into(),
        panel: Some(Panel {
            title: "Notes".into(),
            nodes: vec![Node::Input {
                id: "note".into(),
                placeholder: "Note".into(),
                secret: false,
                multiline: true,
            }],
        }),
    });
    s.run("key ctrl+alt+b");
    s.run("type hello; key enter; type world");
    s.run("key up");
    s.run("type !");
    s.run("key home; type >");
    s.run("key down");
    s.run("key end; type .");
    s.run("key left left left; key delete");
    s.run("key ctrl+enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginSubmit {
            plugin: "p".into(),
            input: "note".into(),
            text: ">hello!\nword.".into()
        })
    );
}
