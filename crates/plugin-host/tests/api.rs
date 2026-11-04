//! The plugin API through the testbed plugin: the permission prompt, secrets, state
//! kept across a reload, and buffer events. Build it first:
//! `cargo build --release --manifest-path plugins/testbed/Cargo.toml`

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use merlin_core::{Event, Node};
use merlin_plugins::{BufferEvent, Config, PluginHost, Snapshot};

const WASM: &str = "target/wasm32-wasip2/release/testbed.wasm";

/// A fresh copy of the testbed, and a data folder of its own.
fn setup(name: &str) -> (PathBuf, Config) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/testbed");
    assert!(
        source.join(WASM).exists(),
        "build the testbed plugin first (see the top of this file)"
    );
    let root = std::env::temp_dir().join(format!("merlin-api-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("testbed");
    fs::create_dir_all(dir.join("target/wasm32-wasip2/release")).unwrap();
    fs::copy(source.join(WASM), dir.join(WASM)).unwrap();
    fs::copy(source.join("plugin.toml"), dir.join("plugin.toml")).unwrap();
    let config = Config {
        data_dir: root.join("data"),
        ask: true,
        keyring: false,
    };
    (dir, config)
}

fn snapshot() -> Snapshot {
    Snapshot {
        root: std::env::temp_dir(),
        ..Snapshot::default()
    }
}

fn panel_text(host: &mut PluginHost) -> String {
    host.render_panels(&snapshot())
        .into_iter()
        .filter_map(|e| match e {
            Event::SetPanel {
                panel: Some(panel), ..
            } => Some(panel.nodes),
            _ => None,
        })
        .flatten()
        .map(|n| match n {
            Node::Heading(t) | Node::Text(t) | Node::Styled(t, _) => t,
            Node::Buttons(bs) => bs.iter().map(|b| format!("[{}]", b.label)).collect(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn notices(events: Vec<Event>) -> Vec<String> {
    events
        .into_iter()
        .filter_map(|e| match e {
            Event::Notify(m) => Some(m),
            _ => None,
        })
        .collect()
}

#[test]
fn permissions_are_asked_for_once_and_remembered() {
    let (dir, config) = setup("ask");
    let data = config.data_dir.clone();
    let mut host = PluginHost::new(config).unwrap();
    let events = host.load(&dir).unwrap();
    assert!(
        matches!(events.last(), Some(Event::FocusPanel(p)) if p == "testbed"),
        "{events:?}"
    );
    let card = panel_text(&mut host);
    assert!(card.contains("Allow Testbed?"), "{card}");
    assert!(card.contains("• read the open file"), "{card}");
    assert!(card.contains("run `cat`, outside the sandbox"), "{card}");
    assert!(
        host.run_command("testbed", "count", snapshot()).is_empty(),
        "not loaded yet"
    );

    let events = host.click("testbed", "merlin:allow", snapshot());
    assert!(
        matches!(events.first(), Some(Event::RegisterPlugin { .. })),
        "{events:?}"
    );
    assert!(panel_text(&mut host).contains("clicks: 0"));

    let mut again = PluginHost::new(Config {
        data_dir: data,
        ask: true,
        keyring: false,
    })
    .unwrap();
    let events = again.load(&dir).unwrap();
    assert!(
        matches!(events.first(), Some(Event::RegisterPlugin { .. })),
        "remembered: {events:?}"
    );
}

#[test]
fn asking_for_more_permissions_asks_again() {
    let (dir, config) = setup("more");
    let data = config.data_dir.clone();
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    host.click("testbed", "merlin:allow", snapshot());

    let manifest = fs::read_to_string(dir.join("plugin.toml")).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        manifest.replace("[\"cat\"]", "[\"cat\", \"sh\"]"),
    )
    .unwrap();
    let mut again = PluginHost::new(Config {
        data_dir: data,
        ask: true,
        keyring: false,
    })
    .unwrap();
    let events = again.load(&dir).unwrap();
    assert!(
        matches!(events.last(), Some(Event::FocusPanel(_))),
        "{events:?}"
    );
    assert!(panel_text(&mut again).contains("run `sh`"));
}

#[test]
fn declining_drops_the_plugin() {
    let (dir, config) = setup("deny");
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    let events = host.click("testbed", "merlin:deny", snapshot());
    assert_eq!(notices(events), ["Testbed was not loaded"]);
    assert!(panel_text(&mut host).is_empty());
    assert!(host.load(&dir).is_ok(), "it can be loaded again later");
}

#[test]
fn secrets_are_kept_per_plugin_in_a_private_file() {
    let (dir, mut config) = setup("secrets");
    config.ask = false;
    let data = config.data_dir.clone();
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    host.run_command("testbed", "set-secret", snapshot());
    let shown = notices(host.run_command("testbed", "show-secret", snapshot()));
    assert_eq!(shown, ["secret: Some(\"hunter2\")"]);

    let file = data.join("secrets/testbed.toml");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    host.run_command("testbed", "delete-secret", snapshot());
    let shown = notices(host.run_command("testbed", "show-secret", snapshot()));
    assert_eq!(shown, ["secret: None"]);
}

#[test]
fn a_reload_keeps_saved_state_and_processes() {
    let (dir, mut config) = setup("reload");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    for command in ["count", "count", "spawn"] {
        host.run_command("testbed", command, snapshot());
    }

    let wasm = dir.join(WASM);
    let later = fs::metadata(&wasm).unwrap().modified().unwrap() + Duration::from_secs(5);
    fs::File::options()
        .write(true)
        .open(&wasm)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let events = host.reload_changed();
    assert_eq!(notices(events), ["Reloaded testbed"]);
    assert!(panel_text(&mut host).contains("clicks: 2"));

    // The cat started before the reload still answers.
    host.run_command("testbed", "send", snapshot());
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        host.poll_processes(snapshot);
        let shown = panel_text(&mut host);
        if shown.contains("ping 2") {
            break;
        }
        assert!(Instant::now() < until, "no echo after the reload:\n{shown}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn plugins_hear_about_buffer_changes() {
    let (dir, mut config) = setup("buffer");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    host.buffer_event(&BufferEvent::Changed, &snapshot());
    host.buffer_event(&BufferEvent::Saved("a.txt".into()), &snapshot());
    assert!(panel_text(&mut host).contains("log: changed, saved a.txt"));
}

#[test]
fn commands_carry_their_keys() {
    let (_, mut config) = setup("keys");
    config.ask = false;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/word-count");
    let mut host = PluginHost::new(config).unwrap();
    let events = host.load(&source).unwrap();
    let Some(Event::RegisterPlugin { commands, .. }) = events.first() else {
        panic!("{events:?}");
    };
    assert_eq!(commands[0].key.as_deref(), Some("ctrl+alt+w"));
}

#[test]
fn diffs_go_to_the_editor_and_the_answer_comes_back() {
    let (dir, mut config) = setup("diff");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    let events = host.run_command("testbed", "propose", snapshot());
    let shown = events.iter().any(|e| {
        matches!(e, Event::ShowReview { id, path, old, new, .. }
            if id == "edit-1" && path == Path::new("notes.txt") && old == "one\n" && new == "one\ntwo\n")
    });
    assert!(shown, "{events:?}");
    host.review("testbed", "edit-1", true, snapshot());
    assert!(panel_text(&mut host).contains("log: edit-1 accepted"));
}

#[test]
fn spawning_needs_the_program_in_the_manifest() {
    let (dir, mut config) = setup("spawn");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    let events = host.run_command("testbed", "spawn-forbidden", snapshot());
    assert!(
        notices(events)
            .iter()
            .any(|n| n.contains("permission denied: processes (sh)")),
    );
}

#[test]
fn settings_come_from_the_manifest_and_the_user() {
    let (dir, mut config) = setup("settings");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    let events = host.run_command("testbed", "show-setting", snapshot());
    assert_eq!(
        notices(events),
        ["greeting: Some(\"hello\"), loud: Some(false)"]
    );
    let user: toml::Table =
        toml::from_str("[testbed]\ngreeting = \"hi\"\nstyle.loud = true\n").unwrap();
    host.set_settings(user);
    let events = host.run_command("testbed", "show-setting", snapshot());
    assert_eq!(
        notices(events),
        ["greeting: Some(\"hi\"), loud: Some(true)"]
    );
}

#[test]
fn plugins_add_languages_colours_and_folds() {
    let (dir, mut config) = setup("languages");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    let events = host.load(&dir).unwrap();
    let languages = events.iter().find_map(|e| match e {
        Event::RegisterLanguages { languages, .. } => Some(languages.clone()),
        _ => None,
    });
    let languages = languages.expect("the manifest's languages");
    assert_eq!(languages[0].name, "Notes");
    assert_eq!(languages[0].line_comment.as_deref(), Some(";"));
    let events = host.run_command("testbed", "colour", snapshot());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::SetTokens { tokens, lines: None, .. }
            if tokens.len() == 1 && tokens[0].kind == merlin_core::Syntax::Keyword)),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::SetFolds { ranges, .. } if ranges == &[(0, 2)]))
    );
}
