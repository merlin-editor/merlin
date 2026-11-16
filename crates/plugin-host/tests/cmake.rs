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
    assert_eq!(
        names,
        [
            "CMake: Configure",
            "CMake: Configure from Scratch",
            "CMake: Build",
            "CMake: Test",
            "CMake: Clean",
            "CMake: Install"
        ]
    );
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
    // CMake's file API is asked first, so the targets can be read after.
    assert!(
        command.starts_with("cmake -E make_directory build/.cmake/api/v1/query/client-merlin && "),
        "{command}"
    );
    assert!(
        command.ends_with(
            "&& cmake -S . -B build -DCMAKE_BUILD_TYPE=Debug \
             -DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_COLOR_DIAGNOSTICS=ON"
        ),
        "{command}"
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

/// With the real cmake: a configure's targets become Build and Run tasks,
/// and its program something F5 debugs. Only where cmake is installed.
#[test]
fn a_real_configure_gives_targets_tasks_and_debugging() {
    if std::process::Command::new("cmake")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("cmake isn't installed; skipping");
        return;
    }
    let (mut host, root, _) = setup("targets", false);
    fs::write(
        root.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.20)\nproject(demo C)\n\
         add_library(shapes STATIC shapes.c)\nadd_executable(app main.c)\n\
         target_link_libraries(app PRIVATE shapes)\n",
    )
    .unwrap();
    fs::write(root.join("shapes.c"), "int area(void) { return 12; }\n").unwrap();
    fs::write(
        root.join("main.c"),
        "int area(void);\nint main(void) { return area() - 12; }\n",
    )
    .unwrap();
    let snapshot = at(&root, "CMakeLists.txt", "");
    host.buffer_event(&BufferEvent::Switched, &snapshot);
    let events = host.run_command("cmake", "task:CMake: Configure", snapshot.clone());
    let (id, command) = events
        .iter()
        .find_map(|e| match e {
            Event::RunInTerminal { id, command, .. } => Some((id.clone(), command.clone())),
            _ => None,
        })
        .expect("a terminal run");
    // The editor's terminal runs it; here a shell does.
    let ran = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let mut events = host.terminal("cmake", &id, "", Some(0), snapshot.clone());
    events.extend(host.poll_processes(|| snapshot.clone()).0);
    let tasks = events
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::SetTasks { plugin, tasks, .. } if plugin == "cmake" => Some(tasks.clone()),
            _ => None,
        })
        .expect("tasks");
    let names: Vec<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
    for name in ["CMake: Build app", "CMake: Run app", "CMake: Build shapes"] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
    assert!(
        !names.contains(&"CMake: Run shapes"),
        "a library doesn't run"
    );
    let targets = events
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::SetDebugTargets { targets, .. } => Some(targets.clone()),
            _ => None,
        })
        .expect("debug targets");
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].name, "app");
    assert_eq!(targets[0].program, "build/app");
    assert_eq!(targets[0].build, "CMake: Build app");
    fs::remove_dir_all(root).ok();
}
