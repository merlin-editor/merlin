//! Plugins' views from their manifests: each where its plugin puts it,
//! moved where the user wants, and remembered with the workspace.

use merlin_core::script::Session;
use merlin_core::{
    Effect, Event, Node, Panel, Place, ToolButton, UiEvent, ViewIcon, ViewSpec, Workspace, view_key,
};

fn view(id: &str, title: &str, place: Place) -> ViewSpec {
    ViewSpec {
        id: id.into(),
        title: title.into(),
        icon: Some(ViewIcon::Glyph("☰".into())),
        place,
        empty: format!("No {id} yet."),
        toolbar: vec![ToolButton {
            command: "refresh".into(),
            icon: Some(ViewIcon::Glyph("↻".into())),
        }],
    }
}

fn with_views() -> Session {
    let mut s = Session::new("fn a() {}\n", Some("src/main.rs"), 80, 20);
    s.files.insert("src/main.rs".into(), s.text());
    s.send(Event::Resize(80, 20));
    s.send(Event::RegisterPlugin {
        plugin: "cargo".into(),
        commands: vec![merlin_core::Command {
            id: "refresh".into(),
            title: "Cargo: Refresh".into(),
            key: None,
        }],
    });
    s.send(Event::RegisterViews {
        plugin: "cargo".into(),
        views: vec![
            view("crates", "Crates", Place::Left),
            view("chat", "Chat", Place::Right),
            view("runs", "Runs", Place::Bottom),
        ],
    });
    s
}

fn rail_labels(s: &Session) -> Vec<String> {
    s.editor
        .window_view()
        .rail
        .iter()
        .map(|r| r.label.clone())
        .collect()
}

#[test]
fn a_left_view_goes_in_the_rail_after_a_line() {
    let s = with_views();
    let rail = s.editor.window_view().rail;
    let crates = rail.iter().position(|r| r.label == "Crates").unwrap();
    assert!(rail[crates].divider, "a line before the plugins' views");
    assert_eq!(rail[crates].icon, Some(ViewIcon::Glyph("☰".into())));
    assert!(
        !rail_labels(&s).contains(&"Chat".to_string()),
        "a right view has the right side bar's tabs, not the rail"
    );
}

#[test]
fn a_view_shows_its_empty_text_toolbar_and_badge() {
    let mut s = with_views();
    let i = s
        .editor
        .window_view()
        .rail
        .iter()
        .position(|r| r.label == "Crates")
        .unwrap();
    s.send(Event::Ui(UiEvent::Rail(i)));
    assert_eq!(s.editor.visible_views(), [view_key("cargo", "crates")]);
    let left = s.editor.window_view().left.expect("the view shows");
    assert_eq!(left.from.as_deref(), Some("cargo"));
    assert_eq!(
        left.nodes,
        [Node::Styled(
            "No crates yet.".into(),
            merlin_core::Tone::Muted
        )]
    );
    assert_eq!(left.toolbar[0].command, "cargo.refresh");
    assert_eq!(left.toolbar[0].title, "Cargo: Refresh");
    s.send(Event::Ui(UiEvent::Press("run:cargo.refresh".into())));
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginCommand {
            plugin: "cargo".into(),
            command: "refresh".into()
        })
    );
    s.send(Event::ViewBadge {
        plugin: "cargo".into(),
        view: "crates".into(),
        count: 2,
    });
    let rail = s.editor.window_view().rail;
    assert_eq!(rail.iter().find(|r| r.label == "Crates").unwrap().badge, 2);
}

#[test]
fn clicks_and_typing_in_a_view_go_to_its_plugin() {
    let mut s = with_views();
    s.send(Event::FocusView {
        plugin: "cargo".into(),
        view: "chat".into(),
    });
    s.send(Event::SetPanel {
        plugin: view_key("cargo", "chat"),
        panel: Some(Panel {
            title: "ignored".into(),
            nodes: vec![Node::Input {
                id: "ask".into(),
                placeholder: "Ask".into(),
                secret: false,
                multiline: false,
            }],
        }),
    });
    let right = s
        .editor
        .window_view()
        .right
        .expect("the right side bar shows");
    assert_eq!(right.title, "Chat", "the manifest's title stays");
    assert!(right.panels.iter().any(|t| t.title == "Chat" && t.on));
    s.run("type why; key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginSubmit {
            plugin: "cargo".into(),
            input: "ask".into(),
            text: "why".into()
        })
    );
}

#[test]
fn a_bottom_view_is_a_tab_in_the_panel_marked_with_its_plugin() {
    let mut s = with_views();
    s.send(Event::FocusView {
        plugin: "cargo".into(),
        view: "runs".into(),
    });
    let bottom = s.editor.window_view().bottom.expect("the panel shows");
    let tab = bottom.panels.iter().find(|t| t.title == "Runs").unwrap();
    assert!(tab.on);
    assert_eq!(tab.from.as_deref(), Some("cargo"));
    assert!(
        bottom.panels.iter().any(|t| t.title == "Problems"),
        "beside Problems"
    );
}

#[test]
fn moving_a_view_shows_it_there_and_the_workspace_remembers() {
    let mut s = with_views();
    let i = s
        .editor
        .window_view()
        .rail
        .iter()
        .position(|r| r.label == "Crates")
        .unwrap();
    s.send(Event::Ui(UiEvent::RailMenu(i)));
    let menu = s.editor.window_view().context.expect("its menu opens");
    assert_eq!(menu.title.as_deref(), Some("Crates · from cargo"));
    let labels: Vec<_> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Move to the Right Side Bar",
            "Move to the Panel",
            "Open as an Editor Tab",
            "",
            "Hide"
        ]
    );
    s.send(Event::Ui(UiEvent::ContextItem(0)));
    assert!(!rail_labels(&s).contains(&"Crates".to_string()));
    let right = s.editor.window_view().right.expect("it shows on the right");
    assert_eq!(right.title, "Crates");

    let ws = s.editor.workspace();
    assert_eq!(
        ws.view_places,
        [(view_key("cargo", "crates"), Place::Right)]
    );
    let text = ws.to_text();
    assert!(text.contains("view cargo/crates right"), "{text}");
    assert_eq!(Workspace::parse(&text).view_places, ws.view_places);

    // The next start: the plugin puts it on the left, the user had it on
    // the right.
    let mut again = Session::new("", None, 80, 20);
    again.send(Event::Restore(Workspace::parse(&text)));
    again.send(Event::RegisterViews {
        plugin: "cargo".into(),
        views: vec![view("crates", "Crates", Place::Left)],
    });
    assert!(!rail_labels(&again).contains(&"Crates".to_string()));
    again.send(Event::FocusView {
        plugin: "cargo".into(),
        view: "crates".into(),
    });
    assert!(again.editor.window_view().right.is_some());
}

#[test]
fn reset_location_puts_a_view_back_where_its_plugin_has_it() {
    let mut s = with_views();
    s.send(Event::Ui(UiEvent::Press(format!(
        "view-menu:{}",
        view_key("cargo", "crates")
    ))));
    s.send(Event::Ui(UiEvent::ContextItem(1))); // Move to the Panel
    assert!(s.editor.window_view().bottom.is_some());
    s.send(Event::Ui(UiEvent::Press(format!(
        "view-menu:{}",
        view_key("cargo", "crates")
    ))));
    let menu = s.editor.window_view().context.unwrap();
    let reset = menu
        .items
        .iter()
        .position(|i| i.label == "Reset Location")
        .expect("it can go back");
    s.send(Event::Ui(UiEvent::ContextItem(reset)));
    assert!(rail_labels(&s).contains(&"Crates".to_string()));
    assert!(s.editor.workspace().view_places.is_empty());
}

#[test]
fn a_view_can_open_as_an_editor_tab_and_take_keys_there() {
    let mut s = with_views();
    s.send(Event::Ui(UiEvent::Press(format!(
        "view-menu:{}",
        view_key("cargo", "chat")
    ))));
    let menu = s.editor.window_view().context.unwrap();
    let tab = menu
        .items
        .iter()
        .position(|i| i.label == "Open as an Editor Tab")
        .unwrap();
    s.send(Event::Ui(UiEvent::ContextItem(tab)));
    let view = s.editor.window_view();
    assert!(view.tabs.iter().any(|t| t.name == "Chat" && t.active));
    assert_eq!(view.pages.len(), 1);
    assert_eq!(view.pages[0].0, "view:cargo/chat");
    assert!(view.pages[0].1.focused);
    assert_eq!(s.editor.visible_views(), [view_key("cargo", "chat")]);
    s.send(Event::SetPanel {
        plugin: view_key("cargo", "chat"),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes: vec![Node::Input {
                id: "ask".into(),
                placeholder: "Ask".into(),
                secret: false,
                multiline: false,
            }],
        }),
    });
    s.run("type hi; key enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginSubmit {
            plugin: "cargo".into(),
            input: "ask".into(),
            text: "hi".into()
        })
    );
    // Its tab's right-click is the view's menu.
    s.send(Event::Ui(UiEvent::TabMenu));
    let menu = s.editor.window_view().context.unwrap();
    assert!(
        menu.items
            .iter()
            .any(|i| i.label == "Move to the Right Side Bar")
    );
}

#[test]
fn hiding_a_view_leaves_it_in_the_palette() {
    let mut s = with_views();
    s.send(Event::Ui(UiEvent::Press(format!(
        "view-menu:{}",
        view_key("cargo", "crates")
    ))));
    let menu = s.editor.window_view().context.unwrap();
    let hide = menu.items.iter().position(|i| i.label == "Hide").unwrap();
    s.send(Event::Ui(UiEvent::ContextItem(hide)));
    assert!(!rail_labels(&s).contains(&"Crates".to_string()));
    assert_eq!(
        s.editor.workspace().hidden_views,
        [view_key("cargo", "crates")]
    );
    let right = {
        s.send(Event::FocusView {
            plugin: "cargo".into(),
            view: "chat".into(),
        });
        s.editor.window_view().right.unwrap()
    };
    assert_eq!(right.hidden, 1, "the right side bar's + lists it");
    s.send(Event::Ui(UiEvent::Press("views-hidden".into())));
    let palette = s
        .editor
        .window_view()
        .palette
        .expect("the palette lists views");
    assert!(
        palette
            .items
            .iter()
            .any(|(label, _)| label == "Show Crates"),
        "{:?}",
        palette.items
    );
    s.run("key esc");
    s.run("key ctrl+shift+p; type show crates; key enter");
    assert!(rail_labels(&s).contains(&"Crates".to_string()));
    assert!(
        s.editor
            .window_view()
            .left
            .is_some_and(|l| l.title == "Crates")
    );
}

#[test]
fn views_go_when_their_plugin_does() {
    let mut s = with_views();
    s.send(Event::FocusView {
        plugin: "cargo".into(),
        view: "chat".into(),
    });
    s.send(Event::RegisterViews {
        plugin: "cargo".into(),
        views: Vec::new(),
    });
    assert!(!rail_labels(&s).contains(&"Crates".to_string()));
    assert!(s.editor.visible_views().is_empty());
}

#[test]
fn dragging_a_view_moves_it_and_back_where_it_was_forgets() {
    let mut s = with_views();
    let crates = view_key("cargo", "crates");
    s.send(Event::Ui(UiEvent::MoveView {
        name: crates.clone(),
        place: Place::Bottom,
    }));
    let bottom = s.editor.window_view().bottom.expect("the panel shows it");
    assert_eq!(bottom.title, "Crates");
    assert_eq!(
        s.editor.workspace().view_places,
        [(crates.clone(), Place::Bottom)]
    );
    s.send(Event::Ui(UiEvent::MoveView {
        name: crates,
        place: Place::Left,
    }));
    assert!(s.editor.workspace().view_places.is_empty());
    assert!(rail_labels(&s).contains(&"Crates".to_string()));
}

#[test]
fn chips_buttons_and_a_rows_menu_reach_the_plugin() {
    use merlin_core::{Button, Chip, EditCard, Entry, Message};
    let mut s = with_views();
    s.send(Event::FocusView {
        plugin: "cargo".into(),
        view: "chat".into(),
    });
    let chip = |id: &str| Chip {
        id: id.into(),
        label: id.into(),
        tone: merlin_core::Tone::Normal,
    };
    s.send(Event::SetPanel {
        plugin: view_key("cargo", "chat"),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes: vec![
                Node::entry(Entry {
                    id: "dep".into(),
                    label: "wgpu".into(),
                    menu: vec![Button {
                        id: "update:wgpu".into(),
                        label: "Update".into(),
                    }],
                    ..Entry::default()
                }),
                Node::Message(Box::new(Message {
                    author: "Bot".into(),
                    markdown: "hi".into(),
                    chips: vec![chip("file"), Chip::default()],
                    ..Message::default()
                })),
                Node::Edit(Box::new(EditCard {
                    path: "a.rs".into(),
                    added: 1,
                    removed: 2,
                    buttons: vec![Button {
                        id: "apply".into(),
                        label: "Apply".into(),
                    }],
                })),
            ],
        }),
    });
    let right = s.editor.window_view().right.unwrap();
    // The row, the message's one clickable chip, and the edit's button.
    assert_eq!(right.targets, [Some(0), Some(1), Some(2)]);
    for (index, id) in [(1, "file"), (2, "apply")] {
        s.send(Event::Ui(UiEvent::Target {
            side: merlin_core::Side::Right,
            index,
            clicks: 1,
        }));
        assert_eq!(
            s.effects.last(),
            Some(&Effect::PluginClick {
                plugin: "cargo".into(),
                button: id.into()
            })
        );
    }
    s.send(Event::Ui(UiEvent::TargetMenu {
        side: merlin_core::Side::Right,
        index: 0,
    }));
    let menu = s.editor.window_view().context.expect("the row's menu");
    assert_eq!(menu.items[0].label, "Update");
    s.send(Event::Ui(UiEvent::ContextItem(0)));
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginClick {
            plugin: "cargo".into(),
            button: "update:wgpu".into()
        })
    );
}

#[test]
fn a_box_keeps_focus_when_a_reply_comes_in_above_it() {
    let mut s = with_views();
    let input = Node::Input {
        id: "ask".into(),
        placeholder: "Ask".into(),
        secret: false,
        multiline: true,
    };
    // Shown before the plugin's first panel, so focus starts by place.
    s.send(Event::FocusView {
        plugin: "cargo".into(),
        view: "chat".into(),
    });
    s.send(Event::SetPanel {
        plugin: view_key("cargo", "chat"),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes: vec![input.clone()],
        }),
    });
    s.run("type why; key ctrl+enter");
    s.send(Event::SetPanel {
        plugin: view_key("cargo", "chat"),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes: vec![
                Node::Button(merlin_core::Button {
                    id: "copy".into(),
                    label: "Copy".into(),
                }),
                input,
            ],
        }),
    });
    s.run("type more; key ctrl+enter");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginSubmit {
            plugin: "cargo".into(),
            input: "ask".into(),
            text: "more".into()
        })
    );
}

#[test]
fn plugins_commands_join_the_right_click_menus() {
    use merlin_core::{MenuItem, MenuPlace};
    let mut s = with_views();
    s.send(Event::RegisterPlugin {
        plugin: "chat".into(),
        commands: vec![merlin_core::Command {
            id: "ask".into(),
            title: "Chat: Ask".into(),
            key: None,
        }],
    });
    s.send(Event::RegisterMenus {
        plugin: "chat".into(),
        items: vec![
            MenuItem {
                place: MenuPlace::Tab,
                command: "ask".into(),
                title: Some("Ask About This File".into()),
            },
            MenuItem {
                place: MenuPlace::Explorer,
                command: "ask".into(),
                title: None,
            },
        ],
    });
    s.send(Event::Ui(UiEvent::TabMenu));
    let menu = s.editor.window_view().context.expect("the tab's menu");
    let last = menu.items.last().unwrap();
    assert_eq!(last.label, "Ask About This File");
    assert_eq!(s.editor.menu_target().as_deref(), Some("src/main.rs"));
    s.send(Event::Ui(UiEvent::ContextItem(menu.items.len() - 1)));
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginCommand {
            plugin: "chat".into(),
            command: "ask".into()
        })
    );
}

#[test]
fn a_view_shown_before_it_draws_focuses_its_box_when_it_does() {
    let mut s = with_views();
    s.send(Event::FocusView {
        plugin: "cargo".into(),
        view: "chat".into(),
    });
    s.send(Event::SetPanel {
        plugin: view_key("cargo", "chat"),
        panel: Some(Panel {
            title: "Chat".into(),
            nodes: vec![
                Node::Button(merlin_core::Button {
                    id: "copy".into(),
                    label: "Copy".into(),
                }),
                Node::Input {
                    id: "ask".into(),
                    placeholder: "Ask".into(),
                    secret: false,
                    multiline: true,
                },
            ],
        }),
    });
    assert_eq!(s.editor.window_view().right.unwrap().selected, 1);
}

#[test]
fn work_under_way_shows_in_the_status_bar_and_can_be_cancelled() {
    let mut s = with_views();
    s.send(Event::ProgressStart {
        plugin: "cargo".into(),
        id: "index".into(),
        title: "Indexing".into(),
        cancellable: true,
    });
    s.send(Event::ProgressReport {
        plugin: "cargo".into(),
        id: "index".into(),
        message: "crates".into(),
        done: Some(400),
    });
    let status = s.editor.window_view().status;
    let i = status
        .iter()
        .position(|item| item.text == "⟳ Indexing · crates 40%")
        .expect("it shows");
    assert!(status[i].clickable);
    s.send(Event::Ui(UiEvent::Status(i)));
    let toast = s.editor.window_view().toast.expect("how it's going");
    assert_eq!(toast.text, "Indexing: crates (40%)");
    assert_eq!(toast.buttons, ["Cancel"]);
    s.send(Event::Ui(UiEvent::ToastButton(0)));
    assert_eq!(
        s.effects.last(),
        Some(&Effect::PluginCancel {
            plugin: "cargo".into(),
            id: "index".into()
        })
    );
    s.send(Event::ProgressEnd {
        plugin: "cargo".into(),
        id: Some("index".into()),
    });
    assert!(
        !s.editor
            .window_view()
            .status
            .iter()
            .any(|item| item.text.contains("Indexing"))
    );
}
