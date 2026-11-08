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
    assert_eq!(notices(events), ["Reloaded Testbed"]);
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

#[test]
fn data_only_plugins_have_no_panel_and_ask_nothing() {
    let root = std::env::temp_dir().join(format!("merlin-api-data-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("looks");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        "name = \"looks\"\ntitle = \"Looks\"\napi = \"0.8\"\n\n[[packs]]\nlabel = \"Flat\"\nfile = \"flat.toml\"\n",
    )
    .unwrap();
    fs::write(dir.join("flat.toml"), "[layout]\ngap = 0\n").unwrap();
    let mut host = PluginHost::new(Config {
        data_dir: root.join("data"),
        ask: true,
        keyring: false,
    })
    .unwrap();
    host.load(&dir).unwrap();
    assert!(
        host.render_panels(&snapshot()).is_empty(),
        "no permission card for a plugin that runs nothing"
    );
    assert!(host.click("looks", "merlin:allow", snapshot()).is_empty());
    let packs = host.packs();
    assert_eq!(packs.len(), 1);
    assert_eq!(packs[0].0, "Flat");
    assert!(packs[0].1.ends_with("flat.toml"));
}

#[test]
fn plugin_pages_count_memory_and_time_and_plugins_turn_off_and_on() {
    let (dir, mut config) = setup("info");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    host.run_command("testbed", "count", snapshot());
    let info = host.plugins().into_iter().next().unwrap();
    assert_eq!(info.title, "Testbed");
    assert!(info.enabled && !info.data_only && !info.waiting);
    assert!(info.memory > 0, "its memory is counted");
    assert!(!info.busy.is_zero(), "its time is counted");
    assert!(info.permissions.processes.contains(&"cat".to_string()));

    let events = host.set_enabled("testbed", false);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::SetPanel { panel: None, .. })),
        "its panel goes: {events:?}"
    );
    assert!(
        host.run_command("testbed", "count", snapshot()).is_empty(),
        "off, it runs nothing"
    );
    assert!(
        host.render_panels(&snapshot()).is_empty(),
        "no permission card when off"
    );
    let info = host.plugins().into_iter().next().unwrap();
    assert!(!info.enabled && !info.waiting && info.memory == 0);

    let events = host.set_enabled("testbed", true);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::RegisterPlugin { commands, .. } if !commands.is_empty())),
        "on again, it registers its commands: {events:?}"
    );
    assert!(host.unload("testbed").len() == 2);
    assert!(host.plugins().is_empty());
}

#[test]
fn compiled_cache_drops_stale_files_for_the_same_plugin() {
    let (dir, mut config) = setup("cache-prune");
    config.ask = false;
    let compiled = config.data_dir.join("compiled");
    fs::create_dir_all(&compiled).unwrap();
    let stale = compiled.join("testbed-0000000000000000.cwasm");
    let other = compiled.join("other-0000000000000000.cwasm");
    fs::write(&stale, b"old").unwrap();
    fs::write(&other, b"keep").unwrap();

    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();

    assert!(!stale.exists(), "stale cache for testbed should be removed");
    assert!(other.exists(), "other plugins' caches are untouched");
    let entries: Vec<_> = fs::read_dir(&compiled)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("testbed-") && n.ends_with(".cwasm"))
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
}

#[test]
fn pure_language_plugins_skip_unrelated_files_and_panel_renders() {
    let (_, mut config) = setup("pure-lang");
    config.ask = false;
    let json_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/json");
    let mut host = PluginHost::new(config).unwrap();
    host.load(&json_dir).unwrap();
    let after_load = host.plugins()[0].busy;

    let mut rs = snapshot();
    rs.path = Some("src/main.rs".into());
    rs.text = "fn main() {}\n".into();
    assert!(host.buffer_event(&BufferEvent::Switched, &rs).is_empty());
    assert!(host.buffer_event(&BufferEvent::Changed, &rs).is_empty());
    assert!(
        host.buffer_event(&BufferEvent::Saved("a.json".into()), &rs)
            .is_empty()
    );
    assert!(host.render_panels(&rs).is_empty());
    assert_eq!(
        host.plugins()[0].busy,
        after_load,
        "pure language plugin should not enter Wasm for unrelated files or panel renders"
    );

    let mut json = snapshot();
    json.path = Some("package.json".into());
    json.text = merlin_core::buffer::Buffer::new("{\"a\": 1}\n", None).snapshot();
    let first = host.buffer_event(&BufferEvent::Switched, &json);
    assert!(
        first.iter().any(|e| matches!(e, Event::SetTokens { .. })),
        "{first:?}"
    );
    let second = host.buffer_event(&BufferEvent::Changed, &json);
    assert!(
        second.is_empty(),
        "unchanged (path, version) should short-circuit without re-colouring: {second:?}"
    );
}

#[test]
fn language_plugins_route_lsp_commands_per_language_without_key_conflicts() {
    use merlin_core::{Editor, Effect, Key};

    let (_, mut config) = setup("per-lang-lsp");
    config.ask = false;
    let plugins_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    let mut host = PluginHost::new(config).unwrap();

    let mut editor = Editor::new("fn main() {}\n", Some("src/main.rs".into()));
    for name in ["rust", "cpp"] {
        for event in host.load(&plugins_dir.join(name)).unwrap() {
            editor.update(event);
        }
    }
    assert!(
        editor.window_view().toast.is_none(),
        "shared language shortcuts should not report key conflicts: {:?}",
        editor.window_view().toast
    );

    let fx = editor.update(Event::Key(Key::parse("alt+d").unwrap()));
    assert!(
        fx.iter().any(|e| matches!(
            e,
            Effect::PluginCommand { plugin, command }
                if plugin == "rust" && command == "definition"
        )),
        "expected alt+d in .rs file to route to rust plugin: {fx:?}"
    );

    editor.update(Event::Opened {
        path: "src/main.cpp".into(),
        text: "int main() { return 0; }\n".into(),
    });
    let fx = editor.update(Event::Key(Key::parse("alt+d").unwrap()));
    assert!(
        fx.iter().any(|e| matches!(
            e,
            Effect::PluginCommand { plugin, command }
                if plugin == "cpp" && command == "definition"
        )),
        "expected alt+d in .cpp file to route to cpp plugin: {fx:?}"
    );
}

#[test]
fn catch2_plugin_discovers_cpp_tests_and_skips_other_languages() {
    let (_, mut config) = setup("catch2");
    config.ask = false;
    let catch2_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/catch2");
    let mut host = PluginHost::new(config).unwrap();
    host.load(&catch2_dir).unwrap();
    let after_load = host.plugins()[0].busy;

    let mut rs = snapshot();
    rs.path = Some("src/lib.rs".into());
    rs.text = "#[test]\nfn it_works() {}\n".into();
    assert!(host.buffer_event(&BufferEvent::Switched, &rs).is_empty());
    assert_eq!(
        host.plugins()[0].busy,
        after_load,
        "catch2 plugin should skip non-C++ files without entering Wasm"
    );

    let mut cpp = snapshot();
    cpp.path = Some("/workspace/tests/math_test.cpp".into());
    cpp.text = merlin_core::buffer::Buffer::new(
        "#include <catch2/catch_test_macros.hpp>\n\
         TEST_CASE(\"addition works\", \"[math]\") {\n\
             REQUIRE(1 + 1 == 2);\n\
         }\n\
         SCENARIO(\"subtraction works\", \"[math]\") {\n\
             REQUIRE(2 - 1 == 1);\n\
         }\n",
        None,
    )
    .snapshot();
    let events = host.buffer_event(&BufferEvent::Switched, &cpp);
    let Some(Event::SetTests {
        plugin,
        path,
        tests,
    }) = events.iter().find(|e| matches!(e, Event::SetTests { .. }))
    else {
        panic!("expected SetTests from catch2 plugin, got: {events:?}");
    };
    assert_eq!(plugin, "catch2");
    assert_eq!(path, "/workspace/tests/math_test.cpp");
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0].id, "addition works");
    assert_eq!(tests[0].line, 1);
    assert_eq!(
        tests[0].command.as_deref(),
        Some("ctest --output-on-failure -R 'addition works'")
    );
    assert_eq!(tests[1].label, "Scenario: subtraction works");
    assert_eq!(tests[1].line, 4);
}
