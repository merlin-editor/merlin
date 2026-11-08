//! The Output panel's channels: merlin's messages, each plugin's log and
//! its programs' stderr, picked by channel and level, and plugins' notices
//! with their buttons and the way to quieten them.

use merlin_core::script::Session;
use merlin_core::style::Hue;
use merlin_core::{
    Button, Effect, Event, Key, KeyCode, LogLevel, Node, OutputClick, OutputList, PluginSummary,
    Tone, UiEvent,
};

fn session() -> Session {
    let mut s = Session::new("fn main() {}\n", Some("src/main.rs"), 100, 30);
    s.send(Event::PluginList(vec![PluginSummary {
        name: "rust".into(),
        title: "Rust".into(),
        what: String::new(),
        tag: String::new(),
        tone: Tone::Normal,
        mark: ("RS".into(), Hue::Orange),
    }]));
    s
}

fn log(s: &mut Session, level: LogLevel, text: &str) {
    s.send(Event::PluginLog {
        plugin: "rust".into(),
        level,
        text: text.into(),
    });
}

/// The Output panel's lines: level and text.
fn lines(s: &Session) -> Vec<(String, String)> {
    let view = s.editor.window_view();
    let bottom = view.bottom.expect("the panel shows");
    bottom
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Log(l) => Some((l.level.clone(), l.text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_plugin_logs_to_its_own_channel() {
    let mut s = session();
    s.send(Event::Notify("Saved".into()));
    log(&mut s, LogLevel::Info, "Started rust-analyzer");
    log(&mut s, LogLevel::Stderr, "thread 'main' panicked");
    s.send(Event::ShowOutput("rust".into()));
    assert_eq!(
        lines(&s),
        [
            ("info".to_string(), "Started rust-analyzer".to_string()),
            ("stderr".to_string(), "thread 'main' panicked".to_string()),
        ]
    );
    let bar = s.editor.window_view().output_bar.expect("the bar shows");
    assert_eq!(bar.channel, "Rust");
    assert_eq!(bar.mark, Some(("RS".into(), Hue::Orange)));

    // merlin's channel has merlin's messages.
    s.send(Event::ShowOutput("merlin".into()));
    assert_eq!(lines(&s), [("info".to_string(), "Saved".to_string())]);
}

#[test]
fn the_channel_list_marks_errors_not_seen_yet() {
    let mut s = session();
    s.send(Event::ShowOutput("merlin".into()));
    log(
        &mut s,
        LogLevel::Error,
        "rust-analyzer exited with code 101",
    );
    s.send(Event::Ui(UiEvent::Output(OutputClick::Open(
        OutputList::Channels,
    ))));
    let bar = s.editor.window_view().output_bar.unwrap();
    let rust = bar.rows.iter().find(|r| r.label == "Rust").unwrap();
    assert!(rust.unseen_error);
    assert_eq!(rust.count, Some(1));
    assert_eq!(rust.heading.as_deref(), Some("Plugins"));

    // Picking it shows it, and it's seen.
    let i = bar.rows.iter().position(|r| r.label == "Rust").unwrap();
    s.send(Event::Ui(UiEvent::Output(OutputClick::Row(i))));
    assert_eq!(lines(&s).len(), 1);
    s.send(Event::Ui(UiEvent::Output(OutputClick::Open(
        OutputList::Channels,
    ))));
    let bar = s.editor.window_view().output_bar.unwrap();
    assert!(!bar.rows.iter().any(|r| r.unseen_error));
}

#[test]
fn the_level_hides_quieter_lines() {
    let mut s = session();
    log(&mut s, LogLevel::Debug, "sent didOpen");
    log(&mut s, LogLevel::Warn, "inlay hints failed");
    s.send(Event::ShowOutput("rust".into()));
    assert_eq!(lines(&s).len(), 1, "info and up by default");

    s.send(Event::Ui(UiEvent::Output(OutputClick::Open(
        OutputList::Levels,
    ))));
    let all = s
        .editor
        .window_view()
        .output_bar
        .unwrap()
        .rows
        .iter()
        .position(|r| r.label == "All")
        .unwrap();
    s.send(Event::Ui(UiEvent::Output(OutputClick::Row(all))));
    assert_eq!(lines(&s).len(), 2);
}

#[test]
fn a_plugin_notice_has_its_buttons_and_a_way_to_quieten_it() {
    let mut s = session();
    s.send(Event::PluginNotice {
        plugin: "rust".into(),
        level: LogLevel::Error,
        text: "rust-analyzer stopped".into(),
        buttons: vec![Button {
            id: "command:restart".into(),
            label: "Restart".into(),
        }],
    });
    let toast = s.editor.window_view().toast.expect("a notice");
    assert!(toast.error);
    assert_eq!(toast.buttons, ["Open Output", "Restart"]);
    assert_eq!(toast.link.as_deref(), Some("Notices from Rust…"));

    // It's in the plugin's channel too.
    s.send(Event::ShowOutput("rust".into()));
    assert_eq!(
        lines(&s),
        [("error".to_string(), "rust-analyzer stopped".to_string())]
    );

    // Notices from Rust… asks, and saves the answer as its setting.
    s.send(Event::PluginNotice {
        plugin: "rust".into(),
        level: LogLevel::Warn,
        text: "hover failed".into(),
        buttons: Vec::new(),
    });
    s.send(Event::Ui(UiEvent::ToastButton(1)));
    let palette = s.editor.window_view().palette.expect("asks how many");
    assert_eq!(palette.title.as_deref(), Some("Notices from Rust"));
    s.send(Event::Key(Key::plain(KeyCode::Down)));
    s.send(Event::Key(Key::plain(KeyCode::Enter)));
    assert!(
        s.effects.contains(&Effect::SetSetting {
            key: "plugins.rust.notices".into(),
            value: "\"errors\"".into(),
        }),
        "{:?}",
        s.effects
    );
}

#[test]
fn a_notice_button_can_run_the_plugins_command() {
    let mut s = session();
    s.send(Event::RegisterPlugin {
        plugin: "rust".into(),
        commands: vec![merlin_core::Command {
            id: "restart".into(),
            title: "Rust: Restart Servers".into(),
            key: None,
        }],
    });
    s.send(Event::PluginNotice {
        plugin: "rust".into(),
        level: LogLevel::Error,
        text: "rust-analyzer stopped".into(),
        buttons: vec![Button {
            id: "command:restart".into(),
            label: "Restart".into(),
        }],
    });
    s.effects.clear();
    s.send(Event::Ui(UiEvent::ToastButton(1)));
    assert!(
        s.effects.iter().any(|e| matches!(
            e,
            Effect::PluginCommand { plugin, command } if plugin == "rust" && command == "restart"
        )),
        "{:?}",
        s.effects
    );
}
