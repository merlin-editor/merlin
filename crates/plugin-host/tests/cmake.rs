//! The CMake plugin, with the testbed watching what it shares. Build both
//! first: `cargo build --release` in `plugins/cmake` and `plugins/testbed`.

use std::fs;
use std::path::{Path, PathBuf};

use merlin_core::{Event, Syntax};
use merlin_plugins::{BufferEvent, Config, PluginHost, Snapshot};

mod common;
use common::Folder;

/// The project's folder, removed once neocmakelsp's `cmake
/// --system-information`, which it runs in the project when it starts, has
/// finished and cleaned up after itself, or it would put back what was
/// removed under it.
struct Project(Option<Folder>);

impl std::ops::Deref for Project {
    type Target = Path;
    fn deref(&self) -> &Path {
        self.0.as_ref().unwrap()
    }
}

impl AsRef<Path> for Project {
    fn as_ref(&self) -> &Path {
        self
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let folder = self.0.take().unwrap();
        let info = folder.join("__cmake_systeminformation");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while info.exists() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

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
fn setup(name: &str, configured: bool) -> (PluginHost, Project, Vec<Event>) {
    let root = Folder::new(&format!("merlin-cmake-{name}-"));
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
    (host, Project(Some(root)), events)
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

    // Started again, it finds the build cmake configured, with its
    // targets, and shares it, though the file opened is a C file: the
    // project's CMakeLists.txt starts it.
    let mut again = PluginHost::new(Config {
        data_dir: root.join(".data-again"),
        ask: false,
        keyring: false,
        background: false,
    })
    .unwrap();
    again.load(&plugin("testbed")).unwrap();
    again.load(&plugin("cmake")).unwrap();
    let c_file = at(&root, "main.c", "int main(void) { return 0; }\n");
    let mut events = again.buffer_event(&BufferEvent::Switched, &c_file);
    events.extend(again.poll_processes(|| c_file.clone()).0);
    assert!(
        notices(&events).contains(&"compile-commands now [cmake: build]".to_string()),
        "{:?}",
        notices(&events)
    );
}

/// With the real cmake and ctest: the tests show under the `add_test`
/// lines they come from, and a run says how each went. Only where cmake
/// is installed.
#[test]
fn ctest_tests_are_listed_and_run() {
    use std::time::{Duration, Instant};
    if std::process::Command::new("ctest")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("ctest isn't installed; skipping");
        return;
    }
    let (mut host, root, _) = setup("ctest", false);
    fs::write(
        root.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.20)\nproject(demo C)\nenable_testing()\n\
         add_executable(app main.c)\nadd_test(NAME unit COMMAND app)\n\
         add_test(NAME fails COMMAND app 1)\n",
    )
    .unwrap();
    fs::write(
        root.join("main.c"),
        "int main(int c, char **v) { return c > 1; }\n",
    )
    .unwrap();
    let snapshot = at(&root, "CMakeLists.txt", "");
    host.buffer_event(&BufferEvent::Switched, &snapshot);
    let shell = |command: &str| {
        let ran = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&root)
            .output()
            .unwrap();
        String::from_utf8_lossy(&ran.stdout).into_owned()
    };
    let run = |host: &mut PluginHost, command: &str| {
        host.run_command("cmake", command, snapshot.clone())
            .into_iter()
            .find_map(|e| match e {
                Event::RunInTerminal { id, command, .. } => Some((id, command)),
                _ => None,
            })
            .expect("a terminal run")
    };
    let (id, command) = run(&mut host, "task:CMake: Configure");
    shell(&command);
    host.terminal("cmake", &id, "", Some(0), snapshot.clone());

    // The tests come once ctest has listed them.
    let until = Instant::now() + Duration::from_secs(20);
    let tests = loop {
        let (events, _) = host.poll_processes(|| snapshot.clone());
        if let Some(found) = events.into_iter().find_map(|e| match e {
            Event::SetTests { path, tests, .. } if !tests.is_empty() => Some((path, tests)),
            _ => None,
        }) {
            break found;
        }
        assert!(Instant::now() < until, "no tests listed");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(tests.0, Path::new("CMakeLists.txt"));
    let found: Vec<(&str, u32)> = tests.1.iter().map(|t| (t.id.as_str(), t.line)).collect();
    assert_eq!(
        found,
        [("unit", 4), ("fails", 5)],
        "by their add_test lines"
    );

    // One test runs alone, built first.
    let (id, command) = run(&mut host, "run-test:fails");
    assert!(
        command.starts_with("cmake --build build && ctest --test-dir build"),
        "{command}"
    );
    assert!(command.ends_with("-R \"^fails$\""), "{command}");
    let output = shell(&command);
    let events = host.terminal("cmake", &id, &output, Some(8), snapshot.clone());
    let results = events
        .into_iter()
        .find_map(|e| match e {
            Event::SetTestResults { results, .. } => Some(results),
            _ => None,
        })
        .expect("results");
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0].id, "fails");
    assert_eq!(format!("{:?}", results[0].outcome), "Some(Failed)");
    let (id, command) = run(&mut host, "run-tests:all");
    let output = shell(&command);
    let results = host
        .terminal("cmake", &id, &output, Some(8), snapshot.clone())
        .into_iter()
        .find_map(|e| match e {
            Event::SetTestResults { results, .. } => Some(results),
            _ => None,
        })
        .expect("results");
    let passed: Vec<&str> = results
        .iter()
        .filter(|r| format!("{:?}", r.outcome) == "Some(Passed)")
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(passed, ["unit"]);
}

#[test]
fn cmakes_own_errors_are_read_from_a_configure() {
    let (_host, _root, events) = setup("matcher", false);
    let matchers = events
        .into_iter()
        .find_map(|e| match e {
            Event::RegisterProblemMatchers { plugin, matchers } if plugin == "cmake" => {
                Some(matchers)
            }
            _ => None,
        })
        .expect("CMake's problem matcher");
    let m = &matchers[0];
    let first = m.lines[0]
        .regex
        .captures("CMake Error at libs/CMakeLists.txt:13 (find_package):")
        .expect("the place");
    let group = |i: Option<usize>| i.and_then(|i| first.get(i)).map_or("", |g| g.as_str());
    assert_eq!(
        [
            group(m.lines[0].severity),
            group(m.lines[0].file),
            group(m.lines[0].line)
        ],
        ["Error", "libs/CMakeLists.txt", "13"]
    );
    assert!(
        m.lines[0]
            .regex
            .is_match("CMake Warning (dev) at CMakeLists.txt:4 (add_library):")
    );
    let second = m.lines[1]
        .regex
        .captures("  By not providing \"Findfmt.cmake\" in CMAKE_MODULE_PATH")
        .expect("the message");
    assert!(second[1].starts_with("By not providing"));
}
