//! The plugin API through the testbed plugin: the permission prompt, secrets, state
//! kept across a reload, and buffer events. Build it first:
//! `cargo build --release --manifest-path plugins/testbed/Cargo.toml`

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use merlin_core::{Event, Node};
use merlin_plugins::{BufferEvent, Config, PluginHost, SettingKind, Snapshot};

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
        background: false,
    };
    (dir, config)
}

fn snapshot() -> Snapshot {
    Snapshot {
        root: std::env::temp_dir(),
        ..Snapshot::default()
    }
}

/// The testbed's views, which show what it has.
fn shown() -> Vec<String> {
    ["activity", "chat", "runs"]
        .map(|v| merlin_core::view_key("testbed", v))
        .into()
}

fn panel_text(host: &mut PluginHost) -> String {
    host.render_panels(&snapshot(), &shown(), true)
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

/// Lines for the plugin's Output channel.
fn logged(events: Vec<Event>) -> Vec<String> {
    events
        .into_iter()
        .filter_map(|e| match e {
            Event::PluginLog { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn notices(events: Vec<Event>) -> Vec<String> {
    events
        .into_iter()
        .filter_map(|e| match e {
            Event::Notify(m) | Event::PluginNotice { text: m, .. } => Some(m),
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
        background: false,
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
        background: false,
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
    // A reload because the file changed is only logged.
    let events = host.reload_changed();
    assert!(notices(events.clone()).is_empty());
    assert_eq!(logged(events), ["Reloaded the plugin, keeping its state"]);
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
    host.set_settings(user, toml::Table::new(), &snapshot());
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
        format!(
            "name = \"looks\"\ntitle = \"Looks\"\napi = \"{}\"\n\n[[packs]]\nlabel = \"Flat\"\nfile = \"flat.toml\"\n",
            merlin_plugins::API_VERSION
        ),
    )
    .unwrap();
    fs::write(dir.join("flat.toml"), "[layout]\ngap = 0\n").unwrap();
    let mut host = PluginHost::new(Config {
        data_dir: root.join("data"),
        ask: true,
        keyring: false,
        background: false,
    })
    .unwrap();
    host.load(&dir).unwrap();
    assert!(
        host.render_panels(&snapshot(), &shown(), true).is_empty(),
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
        host.render_panels(&snapshot(), &shown(), true).is_empty(),
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
    let events = host.unload("testbed");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::RegisterViews { views, .. } if views.is_empty())),
        "unloaded, its views go: {events:?}"
    );
    assert!(host.plugins().is_empty());
}

#[test]
fn views_come_from_the_manifest_and_draw_only_while_they_show() {
    let (dir, mut config) = setup("views");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    let events = host.load(&dir).unwrap();
    let views = events
        .iter()
        .find_map(|e| match e {
            Event::RegisterViews { views, .. } => Some(views.clone()),
            _ => None,
        })
        .expect("its views are announced");
    let places: Vec<_> = views.iter().map(|v| (v.id.as_str(), v.place)).collect();
    assert_eq!(
        places,
        [
            ("activity", merlin_core::Place::Left),
            ("chat", merlin_core::Place::Right),
            ("runs", merlin_core::Place::Bottom),
        ]
    );
    assert_eq!(views[0].toolbar[0].command, "count");
    assert_eq!(
        views[0].toolbar[0].icon,
        Some(merlin_core::ViewIcon::Glyph("+".into()))
    );
    assert!(
        matches!(&views[1].icon, Some(merlin_core::ViewIcon::Svg(p)) if p.ends_with("icons/chat.svg")),
        "an icon file is in its folder: {:?}",
        views[1].icon
    );
    assert_eq!(views[2].empty, "No runs yet.");

    // Only what shows is drawn, each by its view's id.
    let chat = merlin_core::view_key("testbed", "chat");
    let events = host.render_panels(&snapshot(), std::slice::from_ref(&chat), false);
    let drawn: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::SetPanel { plugin, panel } => Some((plugin.clone(), panel.is_some())),
            _ => None,
        })
        .collect();
    assert_eq!(drawn, [(chat.clone(), true)]);
    host.submit("testbed", "ask", "why", snapshot());
    let events = host.render_panels(&snapshot(), &[chat], false);
    assert!(
        format!("{events:?}").contains("asked: why"),
        "a view's input reaches its plugin: {events:?}"
    );

    // A count on its icon, and a way to show it.
    let events = host.run_command("testbed", "count", snapshot());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::ViewBadge { view, count: 1, .. } if view == "activity"))
    );
    let events = host.run_command("testbed", "show-chat", snapshot());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::FocusView { view, .. } if view == "chat"))
    );
}

#[test]
fn a_view_with_a_place_merlin_doesnt_know_is_refused() {
    let (dir, config) = setup("bad-view");
    let manifest = fs::read_to_string(dir.join("plugin.toml")).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        manifest.replace("where = \"bottom\"", "where = \"top\""),
    )
    .unwrap();
    let mut host = PluginHost::new(config).unwrap();
    let err = format!("{:#}", host.load(&dir).unwrap_err());
    assert!(err.contains("left, right, bottom or tab"), "{err}");
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
    assert!(host.render_panels(&rs, &[], true).is_empty());
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
        ..
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

#[test]
fn logs_and_notices_follow_the_plugins_settings() {
    let (dir, mut config) = setup("logs");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    // Info and up by default.
    let events = host.run_command("testbed", "log", snapshot());
    assert_eq!(logged(events), ["something to see"]);
    assert_eq!(
        notices(host.run_command("testbed", "warn", snapshot())),
        ["careful"]
    );

    let user: toml::Table =
        toml::from_str("[testbed]\nnotices = \"errors\"\nlog-level = \"debug\"\n").unwrap();
    host.set_settings(user, toml::Table::new(), &snapshot());
    let events = host.run_command("testbed", "log", snapshot());
    assert_eq!(logged(events), ["quiet detail", "something to see"]);
    // A warning is logged instead; an error still shows.
    let events = host.run_command("testbed", "warn", snapshot());
    assert!(notices(events.clone()).is_empty());
    assert_eq!(logged(events), ["careful"]);
    assert_eq!(
        notices(host.run_command("testbed", "fail", snapshot())),
        ["it broke"]
    );
}

#[test]
fn described_settings_have_their_types_and_scopes() {
    let (dir, mut config) = setup("schema");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    let info = host
        .plugins()
        .into_iter()
        .find(|p| p.name == "testbed")
        .unwrap();
    let spec = |key: &str| info.schema.iter().find(|s| s.key == key).unwrap().clone();
    assert_eq!(
        spec("mode").kind,
        SettingKind::Choice(vec![
            ("fast".into(), "Quickly".into()),
            ("careful".into(), "Careful".into())
        ])
    );
    assert!(spec("server-path").user_only);
    // Plain values are settings too, typed by their defaults, and every
    // plugin that runs has Notices and Log level.
    assert_eq!(spec("style.loud").kind, SettingKind::Flag);
    assert_eq!(spec("greeting").kind, SettingKind::Text);
    assert!(info.schema.iter().any(|s| s.key == "notices"));
    assert_eq!(info.defaults.get("mode"), Some(&"fast".into()));

    // The workspace sets the mode, but not the program's path.
    let user: toml::Table = toml::from_str("[testbed]\nserver-path = \"/bin/cat\"\n").unwrap();
    let workspace: toml::Table =
        toml::from_str("[testbed]\nmode = \"careful\"\nserver-path = \"/tmp/evil\"\n").unwrap();
    let events = host.set_settings(user, workspace, &snapshot());
    assert!(
        events
            .iter()
            .all(|e| !matches!(e, Event::PluginFailed { .. })),
        "{events:?}"
    );
    assert_eq!(
        notices(host.run_command("testbed", "show-path", snapshot())),
        ["mode: Some(\"careful\"), path: Some(\"/bin/cat\")"]
    );
    // It heard which changed.
    let text = panel_text(&mut host);
    assert!(text.contains("mode changed"), "{text}");
    assert!(text.contains("server-path changed"), "{text}");
    fs::remove_dir_all(dir.parent().unwrap()).ok();
}

#[test]
fn a_secret_setting_is_kept_out_of_the_settings_files() {
    let (dir, mut config) = setup("secret-setting");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    let info = |host: &PluginHost| {
        host.plugins()
            .into_iter()
            .find(|p| p.name == "testbed")
            .unwrap()
    };
    let token = info(&host)
        .schema
        .into_iter()
        .find(|s| s.key == "token")
        .unwrap();
    assert_eq!(token.kind, SettingKind::Secret);
    assert!(token.user_only, "no workspace sets a secret");
    assert!(info(&host).defaults.get("token").is_none());
    assert!(info(&host).secrets.is_empty());

    host.set_secret("testbed", "token", Some("hunter3"), &snapshot())
        .unwrap();
    assert_eq!(info(&host).secrets, ["token"]);
    assert_eq!(
        notices(host.run_command("testbed", "show-secret", snapshot())),
        ["secret: Some(\"hunter3\")"]
    );
    assert!(panel_text(&mut host).contains("token changed"));

    host.set_secret("testbed", "token", None, &snapshot())
        .unwrap();
    assert!(info(&host).secrets.is_empty());
    fs::remove_dir_all(dir.parent().unwrap()).ok();
}

#[test]
fn views_draw_rows_messages_edits_chips_progress_and_empty_states() {
    use merlin_core::Tone;
    let (dir, mut config) = setup("rich");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    let draw = |host: &mut PluginHost, view: &str| -> Vec<Node> {
        let key = merlin_core::view_key("testbed", view);
        host.render_panels(&snapshot(), &[key], false)
            .into_iter()
            .find_map(|e| match e {
                Event::SetPanel { panel: Some(p), .. } => Some(p.nodes),
                _ => None,
            })
            .expect("the view draws")
    };

    // A tree row that opens, with its count, chips and menu.
    host.click("testbed", "toggle", snapshot());
    let rows: Vec<_> = draw(&mut host, "activity")
        .into_iter()
        .filter_map(|n| match n {
            Node::Entry(e) => Some(*e),
            _ => None,
        })
        .collect();
    assert_eq!(rows[0].twisty, Some('▾'));
    assert_eq!(rows[0].badge, "0");
    let clicks = &rows[1];
    assert_eq!((clicks.depth, clicks.badge.as_str()), (1, "→ more"));
    assert_eq!(clicks.badge_tone, Tone::Warning);
    let menu: Vec<_> = clicks.menu.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(menu, ["Count Again", "Reset"]);

    // A conversation: messages, a change to review, and a box to ask in.
    host.submit("testbed", "ask", "why", snapshot());
    let chat = draw(&mut host, "chat");
    let messages: Vec<_> = chat
        .iter()
        .filter_map(|n| match n {
            Node::Message(m) => Some(m),
            _ => None,
        })
        .collect();
    assert!(messages[0].mine && messages[0].chips[0].label == "notes.txt 1–3");
    assert!(messages[1].markdown.contains("```rust"));
    assert!(
        chat.iter()
            .any(|n| matches!(n, Node::Edit(e) if e.added == 1))
    );
    assert!(chat.iter().any(|n| matches!(
        n,
        Node::Input {
            multiline: true,
            ..
        }
    )));
    assert!(chat.iter().any(|n| matches!(
        n,
        Node::Chips { hint, .. } if hint == "Ctrl Enter to send"
    )));

    // Runs: empty, then under way.
    assert!(matches!(
        &draw(&mut host, "runs")[..],
        [Node::Empty { title, .. }] if title == "No runs yet"
    ));
    host.click("testbed", "spawn", snapshot());
    assert!(matches!(
        &draw(&mut host, "runs")[..],
        [
            Node::Progress { done: None, .. },
            Node::Progress { done: Some(0), .. },
            Node::Image(_)
        ]
    ));
}

#[test]
fn a_plugin_with_activate_on_waits_until_its_file_opens_or_it_is_used() {
    let (dir, mut config) = setup("lazy");
    config.ask = false;
    let manifest = fs::read_to_string(dir.join("plugin.toml")).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        manifest.replace(
            "[permissions]",
            "activate-on = [\"language:notes\"]\n\n[permissions]",
        ),
    )
    .unwrap();
    // The first time, it starts, so its commands are known.
    let mut host = PluginHost::new(config.clone()).unwrap();
    host.load(&dir).unwrap();
    assert!(!host.plugins()[0].dormant);
    drop(host);

    // Next time it waits, its commands in the palette all the same.
    let mut host = PluginHost::new(config).unwrap();
    let events = host.load(&dir).unwrap();
    assert!(host.plugins()[0].dormant && host.plugins()[0].memory == 0);
    assert!(events.iter().any(
        |e| matches!(e, Event::RegisterPlugin { commands, .. } if commands.iter().any(|c| c.id == "count"))
    ));
    let mut rs = snapshot();
    rs.path = Some("main.rs".into());
    host.buffer_event(&BufferEvent::Switched, &rs);
    assert!(
        host.plugins()[0].dormant,
        "another language's file doesn't start it"
    );
    let mut notes = snapshot();
    notes.path = Some("todo.notes".into());
    host.buffer_event(&BufferEvent::Switched, &notes);
    assert!(!host.plugins()[0].dormant, "its own language's file does");
}

#[test]
fn a_command_starts_a_waiting_plugin() {
    let (dir, mut config) = setup("lazy-command");
    config.ask = false;
    let manifest = fs::read_to_string(dir.join("plugin.toml")).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        manifest.replace(
            "[permissions]",
            "activate-on = [\"file:nowhere.txt\"]\n\n[permissions]",
        ),
    )
    .unwrap();
    PluginHost::new(config.clone()).unwrap().load(&dir).unwrap();
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    assert!(host.plugins()[0].dormant);
    let events = host.run_command("testbed", "count", snapshot());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::ViewBadge { count: 1, .. })),
        "it started and ran: {events:?}"
    );
    assert!(!host.plugins()[0].dormant);
}

#[test]
fn a_plugin_needing_one_thats_missing_stops_and_says_so() {
    let (dir, mut config) = setup("depends");
    config.ask = false;
    let manifest = fs::read_to_string(dir.join("plugin.toml")).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        manifest.replace("[permissions]", "depends = [\"lsp\"]\n\n[permissions]"),
    )
    .unwrap();
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    let events = host.check_depends();
    assert!(
        matches!(&events[..], [Event::PluginFailed { message, .. }] if message.contains("needs the lsp plugin")),
        "{events:?}"
    );
    assert!(host.plugins()[0].failed);
}

#[test]
fn a_menu_command_knows_which_file_its_menu_was_on() {
    let (dir, mut config) = setup("menus");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    let events = host.load(&dir).unwrap();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::RegisterMenus { items, .. } if items.len() == 3
            && items[0].title.as_deref() == Some("Ask Testbed About This")
    )));
    let mut on = snapshot();
    on.menu_target = Some("src/lib.rs".into());
    host.run_command("testbed", "about", on);
    let chat = merlin_core::view_key("testbed", "chat");
    let events = host.render_panels(&snapshot(), &[chat], false);
    assert!(
        format!("{events:?}").contains("about src/lib.rs"),
        "{events:?}"
    );
}

#[test]
fn progress_shows_and_cancel_reaches_the_plugin() {
    let (dir, mut config) = setup("progress");
    config.ask = false;
    let mut host = PluginHost::new(config).unwrap();
    host.load(&dir).unwrap();
    let events = host.run_command("testbed", "spawn", snapshot());
    assert!(events.iter().any(|e| matches!(
        e,
        Event::ProgressStart { id, cancellable: true, .. } if id == "cat"
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::ProgressReport { message, done: None, .. } if message == "waiting for input"
    )));
    let events = host.cancel("testbed", "cat", snapshot());
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::ProgressEnd { id: Some(id), .. } if id == "cat"
        )),
        "{events:?}"
    );
}

/// In the background, as the window runs plugins: a slow call returns at
/// once, its events come from polling, calls keep their order, and views
/// asked for again while it's busy are drawn once, as they are last.
#[test]
fn calls_in_the_background_do_not_wait() {
    let (dir, config) = setup("background");
    let mut host = PluginHost::new(Config {
        ask: false,
        background: true,
        ..config
    })
    .unwrap();
    host.load(&dir).unwrap();
    let started = Instant::now();
    assert!(host.run_command("testbed", "slow", snapshot()).is_empty());
    let waited = started.elapsed();
    for _ in 0..3 {
        assert!(host.render_panels(&snapshot(), &shown(), true).is_empty());
    }
    let until = Instant::now() + Duration::from_secs(10);
    let mut events = Vec::new();
    while !events
        .iter()
        .any(|e| matches!(e, Event::SetPanel { plugin, .. } if plugin == "testbed/activity"))
    {
        assert!(Instant::now() < until, "nothing came back: {events:?}");
        std::thread::sleep(Duration::from_millis(5));
        events.extend(host.poll_processes(snapshot).0);
    }
    assert!(
        waited < started.elapsed() / 2,
        "the call waited {waited:?} of {:?}",
        started.elapsed()
    );
    assert_eq!(notices(events.clone()), ["slow done true"]);
    let drawn: Vec<String> = events
        .into_iter()
        .filter_map(|e| match e {
            Event::SetPanel {
                plugin,
                panel: Some(panel),
            } if plugin == "testbed/activity" => Some(format!("{:?}", panel.nodes[0])),
            _ => None,
        })
        .collect();
    // Drawn once, after the slow command, so it counts it.
    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert!(drawn[0].contains("clicks: 1"), "{drawn:?}");
}

/// A picture handed over once is kept by the editor, shown by an image
/// node, and handed over again only when it changes; one that can't be
/// read says why.
#[test]
fn pictures_are_handed_over_once() {
    let (dir, config) = setup("pictures");
    let mut host = PluginHost::new(Config {
        ask: false,
        ..config
    })
    .unwrap();
    host.load(&dir).unwrap();
    let images = |events: &[Event]| -> Vec<(u32, u32)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::SetImage {
                    id, image: Some(i), ..
                } if id == "chart" => Some((i.width, i.height)),
                _ => None,
            })
            .collect()
    };
    let first = host.render_panels(&snapshot(), &shown(), true);
    assert_eq!(images(&first), [(240, 96)]);
    assert!(first.iter().any(|e| matches!(
        e,
        Event::SetPanel { panel: Some(p), .. }
            if p.nodes.iter().any(|n| matches!(n, Node::Image(i) if i.id == "chart"))
    )));
    // Drawn again with nothing changed: not handed over again.
    assert!(images(&host.render_panels(&snapshot(), &shown(), true)).is_empty());
    host.run_command("testbed", "count", snapshot());
    assert_eq!(
        images(&host.render_panels(&snapshot(), &shown(), true)),
        [(240, 96)]
    );

    let said = notices(host.run_command("testbed", "bad-image", snapshot()));
    assert!(said[0].starts_with("not a PNG"), "{said:?}");
}
