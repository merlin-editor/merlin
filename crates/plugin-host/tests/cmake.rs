//! The CMake plugin, with the testbed watching what it shares. Build both
//! first: `cargo build --release` in `plugins/cmake` and `plugins/testbed`.

use std::fs;
use std::path::{Path, PathBuf};

use merlin_core::{Event, Syntax};
use merlin_plugins::{BufferEvent, Config, PluginHost, Snapshot};

fn plugin(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins")
        .join(name);
    assert!(
        dir.join("target/wasm32-wasip2/release").exists(),
        "build the {name} plugin first (see the top of this file)"
    );
    dir
}

/// A project with a CMakeLists.txt, and a host with both plugins.
fn setup(name: &str, configured: bool) -> (PluginHost, PathBuf, Vec<Event>) {
    let root = std::env::temp_dir().join(format!("merlin-cmake-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("build")).unwrap();
    fs::write(root.join("CMakeLists.txt"), "project(demo CXX)\n").unwrap();
    if configured {
        fs::write(
            root.join("build/CMakeCache.txt"),
            "CMAKE_EXPORT_COMPILE_COMMANDS:BOOL=ON\n",
        )
        .unwrap();
    }
    let mut host = PluginHost::new(Config {
        data_dir: root.join(".data"),
        ask: false,
        keyring: false,
        background: false,
    })
    .unwrap();
    let mut events = host.load(&plugin("testbed")).unwrap();
    events.extend(host.load(&plugin("cmake")).unwrap());
    (host, root, events)
}

fn at(root: &Path, path: &str, text: &str) -> Snapshot {
    Snapshot {
        path: Some(path.into()),
        text: text.into(),
        root: root.to_path_buf(),
        ..Snapshot::default()
    }
}

fn notices(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Notify(m) | Event::PluginNotice { text: m, .. } => Some(m.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_configured_build_is_shared_when_it_starts_and_cmake_lists_are_coloured() {
    let (mut host, root, events) = setup("start", true);
    let languages = events.iter().find_map(|e| match e {
        Event::RegisterLanguages { plugin, languages } if plugin == "cmake" => Some(languages),
        _ => None,
    });
    let cmake = &languages.expect("CMake's language")[0];
    assert!(cmake.matches(Path::new("src/CMakeLists.txt")));
    assert!(!cmake.matches(Path::new("notes.txt")));

    // Opening CMakeLists.txt starts it: it shares build/, and colours.
    let text = "add_executable(demo main.cpp)\n";
    let mut events = host.buffer_event(&BufferEvent::Switched, &at(&root, "CMakeLists.txt", text));
    events.extend(host.poll_processes(|| at(&root, "CMakeLists.txt", text)).0);
    assert!(
        notices(&events).contains(&"compile-commands now [cmake: build]".to_string()),
        "{events:?}"
    );
    let tokens = events.iter().find_map(|e| match e {
        Event::SetTokens { tokens, .. } => Some(tokens),
        _ => None,
    });
    assert_eq!(tokens.expect("colours")[0].kind, Syntax::Function);
    let tasks = events.iter().find_map(|e| match e {
        Event::SetTasks { plugin, tasks, .. } if plugin == "cmake" => Some(tasks),
        _ => None,
    });
    let names: Vec<&str> = tasks
        .expect("tasks")
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names, ["CMake: Configure", "CMake: Build", "CMake: Test"]);
    fs::remove_dir_all(root).ok();
}

#[test]
fn configuring_shares_the_build_once_it_works() {
    let (mut host, root, _) = setup("configure", false);
    let snapshot = at(&root, "CMakeLists.txt", "project(demo)\n");
    let events = host.buffer_event(&BufferEvent::Switched, &snapshot);
    assert!(
        notices(&events).is_empty(),
        "nothing configured yet: {events:?}"
    );
    let events = host.run_command("cmake", "task:CMake: Configure", snapshot.clone());
    let (id, command) = events
        .iter()
        .find_map(|e| match e {
            Event::RunInTerminal { id, command, .. } => Some((id.clone(), command.clone())),
            _ => None,
        })
        .expect("a terminal run");
    assert_eq!(
        command,
        "cmake -S . -B build -DCMAKE_EXPORT_COMPILE_COMMANDS=ON"
    );
    // A failed run shares nothing; one that works does.
    let mut events = host.terminal("cmake", &id, "error", Some(1), snapshot.clone());
    events.extend(host.poll_processes(|| snapshot.clone()).0);
    assert!(notices(&events).is_empty(), "{events:?}");
    host.run_command("cmake", "task:CMake: Configure", snapshot.clone());
    let mut events = host.terminal("cmake", &id, "done", Some(0), snapshot.clone());
    events.extend(host.poll_processes(|| snapshot.clone()).0);
    assert_eq!(notices(&events), ["compile-commands now [cmake: build]"]);
    fs::remove_dir_all(root).ok();
}
