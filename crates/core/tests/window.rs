//! A window that draws its own frame: the screen is only the text, the
//! frame comes from the window view, and clicks on it arrive as UI events.

use merlin_core::script::Session;
use merlin_core::{Event, Focus, Node, Side, UiEvent};

fn windowed() -> Session {
    let mut s = Session::new("fn a() {\n    b(1);\n}\n", Some("src/main.rs"), 80, 20);
    s.files.insert("src/main.rs".into(), s.text());
    s.files
        .insert("src/lib.rs".into(), "pub fn b() {}\n".into());
    s.editor.set_windowed(true);
    s.send(Event::Resize(80, 20));
    s
}

#[test]
fn the_screen_is_only_the_text() {
    let s = windowed();
    let screen = s.screen();
    let first = screen.lines().next().unwrap_or_default();
    assert!(first.contains("fn a()"), "the first row is code:\n{screen}");
    assert!(!screen.contains("File"), "no menu bar:\n{screen}");
    assert!(!screen.contains("Ln 1"), "no status bar:\n{screen}");
}

#[test]
fn the_view_has_tabs_status_and_rail() {
    let s = windowed();
    let view = s.editor.window_view();
    assert_eq!(view.tabs.len(), 1);
    assert_eq!(view.tabs[0].name, "main.rs");
    assert!(view.tabs[0].active);
    assert!(
        view.status
            .iter()
            .any(|i| i.right && i.text.starts_with("Ln 1"))
    );
    let labels: Vec<_> = view.rail.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Explorer", "Search", "Source Control", "Outline", "Plugins"]
    );
}

#[test]
fn rail_clicks_show_and_hide_views() {
    let mut s = windowed();
    s.send(Event::Ui(UiEvent::Rail(1)));
    let view = s.editor.window_view();
    let left = view.left.expect("the side bar shows");
    assert!(view.rail[1].active);
    assert!(left.nodes.iter().any(|n| matches!(n, Node::Input { .. })));
    assert!(left.focused);
    s.send(Event::Ui(UiEvent::Rail(1)));
    assert!(
        s.editor.window_view().left.is_none(),
        "a second click hides it"
    );
}

#[test]
fn explorer_rows_open_files() {
    let mut s = windowed();
    s.send(Event::Ui(UiEvent::Rail(0)));
    s.send(Event::Tree(vec!["src/main.rs".into(), "src/lib.rs".into()]));
    let left = s.editor.window_view().left.expect("the Explorer shows");
    // The folder opens to show the current file; lib.rs is the row before it.
    let row = left
        .nodes
        .iter()
        .position(|n| matches!(n, Node::Entry(e) if e.label == "lib.rs"))
        .expect("lib.rs is listed");
    let index = left.targets[row].expect("rows are clickable");
    s.send(Event::Ui(UiEvent::Target {
        side: Side::Left,
        index,
    }));
    let view = s.editor.window_view();
    assert!(view.tabs.iter().any(|t| t.name == "lib.rs" && t.active));
}

#[test]
fn tabs_switch_and_close() {
    let mut s = windowed();
    s.send(Event::Command("file.open".into()));
    s.run("type lib.rs\nkey enter");
    assert_eq!(s.editor.window_view().tabs.len(), 2);
    s.send(Event::Ui(UiEvent::Tab {
        index: 0,
        clicks: 1,
    }));
    assert!(s.editor.window_view().tabs[0].active);
    s.send(Event::Ui(UiEvent::CloseTab(0)));
    let tabs = s.editor.window_view().tabs;
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].name, "lib.rs");
}

#[test]
fn the_palette_lists_commands_and_runs_a_clicked_one() {
    let mut s = windowed();
    s.send(Event::Command("palette.open".into()));
    s.run("type toggle word wrap");
    let palette = s.editor.window_view().palette.expect("the palette is open");
    assert_eq!(palette.kind, "Commands");
    assert_eq!(palette.query, "toggle word wrap");
    let row = palette
        .items
        .iter()
        .position(|(label, _)| label.contains("Word Wrap"))
        .expect("word wrap is listed");
    s.send(Event::Ui(UiEvent::PaletteRow(row)));
    assert!(s.editor.window_view().palette.is_none());
    assert!(s.editor.wrap());
    assert_eq!(s.editor.focus(), Focus::Editor);
}

#[test]
fn the_theme_picker_previews_and_esc_puts_the_old_one_back() {
    use merlin_core::{Effect, ThemeKind};
    let mut s = windowed();
    s.send(Event::Themes {
        kind: ThemeKind::Colours,
        themes: vec!["merlin Dark".into(), "Fuji".into(), "Matcha".into()],
        current: "merlin Dark".into(),
    });
    s.send(Event::Command("theme.pick".into()));
    let palette = s.editor.window_view().palette.expect("the picker is open");
    assert_eq!(palette.kind, "Themes");
    assert_eq!(palette.selected, 0, "the theme in use is selected");
    s.effects.clear();
    s.run("key down");
    assert_eq!(
        s.effects,
        [Effect::PreviewTheme(ThemeKind::Colours, "Fuji".into())]
    );
    s.effects.clear();
    s.run("key esc");
    assert_eq!(
        s.effects,
        [Effect::PreviewTheme(
            ThemeKind::Colours,
            "merlin Dark".into()
        )]
    );

    s.send(Event::Command("theme.pick".into()));
    s.run("type mat");
    s.effects.clear();
    s.run("key enter");
    assert_eq!(
        s.effects,
        [Effect::SetTheme(ThemeKind::Colours, "Matcha".into())]
    );
    s.send(Event::Command("theme.pick".into()));
    let palette = s.editor.window_view().palette.expect("the picker is open");
    assert_eq!(
        palette.items[palette.selected].0, "Matcha",
        "it opens on the new one"
    );
}

#[test]
fn icon_themes_have_their_own_picker() {
    use merlin_core::{Effect, ThemeKind};
    let mut s = windowed();
    s.send(Event::Themes {
        kind: ThemeKind::Icons,
        themes: vec!["Letters".into(), "Origami".into()],
        current: "Letters".into(),
    });
    s.send(Event::Command("icons.pick".into()));
    let palette = s.editor.window_view().palette.expect("the picker is open");
    assert_eq!(palette.title.as_deref(), Some("File Icon Theme"));
    s.effects.clear();
    s.run("key down\nkey enter");
    assert_eq!(
        s.effects,
        [
            Effect::PreviewTheme(ThemeKind::Icons, "Origami".into()),
            Effect::SetTheme(ThemeKind::Icons, "Origami".into())
        ]
    );
}

#[test]
fn rail_items_say_what_they_show() {
    let s = windowed();
    let ids: Vec<_> = s.editor.window_view().rail.iter().map(|r| r.id).collect();
    assert_eq!(
        ids,
        ["explorer", "search", "source-control", "outline", "plugins"]
    );
}

#[test]
fn the_plugins_view_lists_plugins_and_opens_their_pages() {
    use merlin_core::{PluginSummary, Tone};
    let mut s = windowed();
    s.send(Event::PluginList(vec![PluginSummary {
        name: "lsp".into(),
        title: "Language".into(),
        what: "Language servers".into(),
        tag: "runs programs".into(),
        tone: Tone::Warning,
    }]));
    let rail = s.editor.window_view().rail;
    let i = rail
        .iter()
        .position(|r| r.id == "plugins")
        .expect("a Plugins rail item");
    s.send(Event::Ui(UiEvent::Rail(i)));
    let left = s.editor.window_view().left.expect("the Plugins view shows");
    assert_eq!(left.title, "Plugins");
    let row = left
        .nodes
        .iter()
        .position(|n| matches!(n, Node::Entry(e) if e.label == "Language"))
        .expect("the plugin is listed");
    s.send(Event::Ui(UiEvent::Target {
        side: Side::Left,
        index: left.targets[row].unwrap(),
    }));
    let view = s.editor.window_view();
    assert_eq!(
        view.page.as_deref(),
        Some("lsp"),
        "its page is the active tab"
    );
    assert!(view.tabs.iter().any(|t| t.name == "Language" && t.active));

    // A page has no text to type into, and opening it again reuses its tab.
    s.run("type hello");
    assert_eq!(s.editor.window_view().page.as_deref(), Some("lsp"));
    assert!(!s.editor.buffer().is_dirty());
    let tabs = s.editor.window_view().tabs.len();
    s.send(Event::Ui(UiEvent::Target {
        side: Side::Left,
        index: left.targets[row].unwrap(),
    }));
    assert_eq!(s.editor.window_view().tabs.len(), tabs);
}

#[test]
fn ctrl_comma_opens_settings() {
    use merlin_core::Effect;
    let mut s = windowed();
    s.effects.clear();
    s.run("key ctrl+,");
    assert_eq!(s.effects, [Effect::OpenSettingsWindow]);
}

#[test]
fn key_bindings_show_the_settings_changes() {
    use merlin_core::{Key, Settings};
    let mut s = windowed();
    s.send(Event::Settings(Settings {
        keys: vec![
            ("ctrl+k".into(), "view.minimap".into()),
            ("ctrl+b".into(), String::new()),
        ],
        ..Settings::default()
    }));
    let bindings = s.editor.key_bindings();
    let find = |id: &str| bindings.iter().find(|b| b.command == id).unwrap().clone();
    let minimap = find("view.minimap");
    assert_eq!(minimap.keys, [("ctrl+k".to_string(), "Ctrl+K".to_string())]);
    assert!(minimap.changed);
    let sidebar = find("view.sidebar");
    assert!(
        sidebar.keys.is_empty() && sidebar.changed,
        "Ctrl+B was unbound"
    );
    let palette = find("palette.open");
    assert!(!palette.changed && palette.keys.len() == 2);
    for spec in [
        "ctrl+shift+p",
        "f1",
        "alt+z",
        "ctrl+,",
        "shift+alt+up",
        "ctrl+pagedown",
    ] {
        let key = Key::parse(spec).unwrap();
        assert_eq!(Key::parse(&key.spec()), Some(key), "{spec} reads back");
    }
}

#[test]
fn a_right_click_opens_a_menu_that_runs_its_items() {
    let mut s = windowed();
    let (x, y) = s.editor.text_origin();
    s.send(Event::ContextMenu(x + 4, y + 1));
    let view = s.editor.window_view();
    let menu = view.context.expect("a menu");
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"Copy") && labels.contains(&"Change All Occurrences"));
    assert!(
        !labels.contains(&"Go to Definition"),
        "no language server loaded"
    );
    assert_eq!(
        s.editor.cursor_utf16(),
        (1, 4),
        "the cursor went to the click"
    );
    let copy = labels.iter().position(|l| *l == "Copy").unwrap();
    s.send(Event::Ui(UiEvent::ContextItem(copy)));
    assert!(s.editor.window_view().context.is_none());
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, merlin_core::Effect::Copy(t) if t == "    b(1);\n"))
    );
}

#[test]
fn the_context_menu_takes_arrows_and_escape() {
    let mut s = windowed();
    let (x, y) = s.editor.text_origin();
    s.send(Event::ContextMenu(x, y));
    s.run("key down");
    assert_eq!(s.editor.window_view().context.unwrap().selected, Some(0));
    s.run("key esc");
    assert!(s.editor.window_view().context.is_none());
    assert_eq!(s.editor.focus(), Focus::Editor);
}

#[test]
fn a_window_drawing_the_text_leaves_it_out_of_the_screen() {
    let mut s = windowed();
    s.editor.set_own_text(true);
    s.send(Event::Resize(80, 20));
    assert!(!s.screen().contains("fn a()"), "{}", s.screen());
    let views = s.editor.text_views();
    assert_eq!(
        views.last().unwrap().lines[0]
            .chars
            .iter()
            .collect::<String>(),
        "fn a() {"
    );
}
