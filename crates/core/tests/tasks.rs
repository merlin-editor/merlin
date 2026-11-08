//! Tasks and tests: Run Task's list from the project's file and plugins,
//! each task in a terminal of its own, and the Tests view with what
//! plugins find, run and report.

use merlin_core::script::Session;
use merlin_core::{Effect, Event, PluginTest, Task, TaskGroup, TermOp, from_tasks_file};

/// A plugin's task, with a command line or run in the plugin.
fn task(name: &str, command: Option<&str>, group: TaskGroup, source: &str) -> Task {
    Task {
        name: name.into(),
        command: command.map(Into::into),
        group,
        source: source.into(),
        plugin: None,
    }
}

fn project() -> Session {
    let mut s = Session::new("fn a() {}\n", Some("src/main.rs"), 100, 24);
    s.files.insert("src/main.rs".into(), s.text());
    s.send(Event::SetTasks {
        plugin: "rust".into(),
        set: "cargo".into(),
        tasks: vec![
            task(
                "cargo test --workspace",
                None,
                TaskGroup::Test,
                "Cargo.toml",
            ),
            task(
                "cargo build --release",
                Some("cargo build --release"),
                TaskGroup::Build,
                "Cargo.toml",
            ),
        ],
    });
    s.files.insert(
        ".merlin/tasks.toml".into(),
        "[tasks]\ncheck-budgets = \"scripts/budgets.sh\"\n\n[tasks.watch]\ncommand = \"cargo watch -x check\"\ngroup = \"build\"\n".into(),
    );
    s
}

fn spawned(s: &Session) -> Vec<String> {
    s.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Terminal(TermOp::Spawn {
                command: Some(c), ..
            }) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn tasks_come_from_the_projects_own_file() {
    let own = from_tasks_file("[tasks.lint]\ncommand = 'cargo clippy'\ngroup = \"test\"\n");
    assert_eq!(own[0].name, "lint");
    assert_eq!(own[0].command.as_deref(), Some("cargo clippy"));
    assert_eq!(own[0].group, TaskGroup::Test);
}

#[test]
fn run_task_lists_them_grouped_and_runs_one_in_its_own_terminal() {
    let mut s = project();
    s.run("cmd task.run");
    let view = s.editor.window_view();
    let palette = view.palette.expect("Run Task opens");
    assert_eq!(palette.title.as_deref(), Some("Run Task ›"));
    let labels: Vec<&str> = palette.items.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels[0], "cargo test --workspace");
    assert_eq!(*labels.last().unwrap(), "Configure tasks…");
    assert!(labels.contains(&"check-budgets"));
    assert_eq!(palette.extras[0].heading.as_deref(), Some("Test"));
    assert_eq!(palette.extras[0].detail, "Cargo.toml");
    assert_eq!(palette.extras[0].chip.as_deref(), Some("rust"));
    assert_eq!(palette.items[0].1.as_deref(), Some("Ctrl+; A"));
    let watch = labels.iter().position(|l| *l == "watch").unwrap();
    assert_eq!(palette.extras[watch].detail, ".merlin/tasks.toml");
    assert!(palette.extras.last().unwrap().muted);

    s.run("type budgets");
    s.run("key enter");
    assert_eq!(spawned(&s), ["scripts/budgets.sh"]);
    let view = s.editor.window_view();
    let bottom = view.bottom.expect("its terminal shows");
    assert!(
        bottom
            .panels
            .iter()
            .any(|t| t.on && t.title == "Terminal: check-budgets"),
        "{:?}",
        bottom.panels
    );
    assert!(view.status.iter().any(|i| i.text == "● check-budgets"));

    // Running it again takes the old terminal's place.
    s.run("cmd task.run");
    s.run("type budgets");
    s.run("key enter");
    assert_eq!(s.editor.window_view().terminal.unwrap().sessions.len(), 1);

    // Done, it leaves the status bar.
    let id = s.editor.window_view().terminal.unwrap().sessions[0].id;
    s.send(Event::TaskOutput {
        id,
        text: "ok".into(),
    });
    s.send(Event::TermExited { id, code: Some(0) });
    assert!(
        !s.editor
            .window_view()
            .status
            .iter()
            .any(|i| i.text.contains("check-budgets"))
    );
}

#[test]
fn the_build_task_key_runs_the_first_build_task() {
    let mut s = project();
    s.run("key ctrl+shift+b");
    assert_eq!(
        spawned(&s),
        ["cargo watch -x check"],
        "the project's own first"
    );
}

#[test]
fn a_plugins_sets_of_tasks_replace_only_themselves() {
    let mut s = project();
    s.send(Event::SetTasks {
        plugin: "rust".into(),
        set: "rust-analyzer".into(),
        tasks: vec![task(
            "cargo run --bin merlin",
            Some("cargo run --bin merlin"),
            TaskGroup::Build,
            "bin",
        )],
    });
    s.send(Event::SetTasks {
        plugin: "rust".into(),
        set: "cargo".into(),
        tasks: vec![task("cargo test", None, TaskGroup::Test, "Cargo.toml")],
    });
    s.run("cmd task.run");
    let labels: Vec<String> = s
        .editor
        .window_view()
        .palette
        .unwrap()
        .items
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert!(labels.contains(&"cargo test".to_string()), "{labels:?}");
    assert!(
        labels.contains(&"cargo run --bin merlin".to_string()),
        "{labels:?}"
    );
    assert!(
        !labels.contains(&"cargo build --release".to_string()),
        "{labels:?}"
    );
}

const SAVING: &str = "use x::y;\n\n#[test]\nfn trims_on_save() {\n    let text = \"a\\nb\";\n    assert_eq!(text, \"a\\nb\\n\");\n}\n\n#[test]\nfn auto_save_after_a_delay() {\n    assert_eq!(1 + 1, 2);\n}\n";

fn tested() -> Session {
    let mut s = Session::new(SAVING, Some("crates/demo/tests/saving.rs"), 100, 30);
    s.send(Event::RegisterPlugin {
        plugin: "rust".into(),
        commands: Vec::new(),
    });
    s
}

/// What the Rust plugin says it found.
fn found(s: &mut Session) {
    let test = |id: &str, label: &str, line| PluginTest {
        id: id.into(),
        label: label.into(),
        line,
        command: None,
    };
    s.send(Event::SetTests {
        plugin: "rust".into(),
        path: "crates/demo/src/lib.rs".into(),
        group: "demo".into(),
        label: "src/lib.rs".into(),
        tests: vec![test("crates/demo/src/lib.rs::unit", "unit", 4)],
    });
    s.send(Event::SetTests {
        plugin: "rust".into(),
        path: "crates/demo/tests/saving.rs".into(),
        group: "demo".into(),
        label: "tests/saving.rs".into(),
        tests: vec![
            test(
                "crates/demo/tests/saving.rs::trims_on_save",
                "trims_on_save",
                3,
            ),
            test(
                "crates/demo/tests/saving.rs::auto_save_after_a_delay",
                "auto_save_after_a_delay",
                9,
            ),
        ],
    });
}

fn asked(s: &Session, command: &str) -> bool {
    s.effects.contains(&Effect::PluginCommand {
        plugin: "rust".into(),
        command: command.into(),
    })
}

fn labels(s: &Session) -> Vec<String> {
    let view = s.editor.window_view();
    view.left
        .expect("the Tests view shows")
        .nodes
        .iter()
        .filter_map(|n| match n {
            merlin_core::Node::Entry(e) => Some(format!("{} {}", e.label, e.badge)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_tests_view_lists_plugins_tests_by_group_and_file_and_runs_them() {
    let mut s = tested();
    s.run("cmd view.tests");
    assert!(asked(&s, "find-tests"), "plugins are asked for their tests");
    found(&mut s);
    assert_eq!(
        labels(&s),
        [
            "demo not run",
            "src/lib.rs 1",
            "unit ",
            "tests/saving.rs 2",
            "trims_on_save ",
            "auto_save_after_a_delay "
        ]
    );
    // ▶ Run Test over each test in its file, in the row before its fn.
    let view = s.editor.text_views().remove(0);
    assert_eq!(view.tests.len(), 2);
    assert_eq!(view.tests[0].links[0].0, "▶ Run Test");
    let fn_row = view.lines.iter().find(|l| l.line == 3).unwrap().row;
    assert_eq!(view.tests[0].row, fn_row - 1);

    s.send(Event::Ui(merlin_core::UiEvent::Press(
        view.tests[0].links[0].1.clone(),
    )));
    assert!(asked(
        &s,
        "run-test:crates/demo/tests/saving.rs::trims_on_save"
    ));
    s.send(Event::SetTestRun {
        plugin: "rust".into(),
        group: "demo".into(),
        running: true,
        seconds: None,
    });
    assert_eq!(labels(&s)[0], "demo running");
    let result = |id: &str, outcome, message: &str, line| merlin_core::PluginTestResult {
        id: id.into(),
        outcome: Some(outcome),
        message: message.into(),
        line,
    };
    s.send(Event::SetTestResults {
        plugin: "rust".into(),
        results: vec![
            result(
                "crates/demo/tests/saving.rs::trims_on_save",
                merlin_core::Outcome::Failed,
                "left: \"a\\nb\"  right: \"a\\nb\\n\"",
                Some(5),
            ),
            result(
                "crates/demo/tests/saving.rs::auto_save_after_a_delay",
                merlin_core::Outcome::Passed,
                "",
                None,
            ),
        ],
    });
    s.send(Event::SetTestRun {
        plugin: "rust".into(),
        group: "demo".into(),
        running: false,
        seconds: Some(0.4),
    });
    assert_eq!(labels(&s)[0], "demo 0.4s");
    let view = s.editor.window_view();
    assert_eq!(view.left.unwrap().counts, Some((1, 1)));
    assert!(
        view.status
            .iter()
            .any(|i| i.text == "✕ 1 test failed" && i.error)
    );
    // The failure under the line that failed, and the marks.
    let text = s.editor.text_views().remove(0);
    let failure = &text.failures[0];
    assert_eq!(failure.text, "left: \"a\\nb\"  right: \"a\\nb\\n\"");
    let assert_row = text.lines.iter().find(|l| l.line == 5).unwrap().row;
    assert_eq!(failure.row, assert_row + 1);
    assert_eq!(failure.col, 4);
    assert_eq!(
        text.lines.iter().find(|l| l.line == 3).unwrap().test,
        Some(false)
    );
    assert_eq!(
        text.lines.iter().find(|l| l.line == 9).unwrap().test,
        Some(true)
    );
    // Run Failed, Run All, a group's and a file's go to the plugin.
    s.run("cmd tests.run_failed");
    assert!(asked(&s, "run-tests:failed"));
    s.run("cmd tests.run_all");
    assert!(asked(&s, "run-tests:all"));
    s.run("cmd tests.run_file");
    assert!(asked(&s, "run-tests:file:crates/demo/tests/saving.rs"));
    s.run("cmd view.tests");
    s.send(Event::Ui(merlin_core::UiEvent::Press(
        "t:run:r:demo".into(),
    )));
    assert!(asked(&s, "run-tests:group:demo"));
}

#[test]
fn a_tests_command_line_runs_in_the_terminal_and_its_plugin_hears_the_output() {
    let mut s = tested();
    s.send(Event::SetTests {
        plugin: "catch2".into(),
        path: "crates/demo/tests/saving.rs".into(),
        group: String::new(),
        label: String::new(),
        tests: vec![PluginTest {
            id: "trims".into(),
            label: "trims_on_save".into(),
            line: 3,
            command: Some("ctest -R trims".into()),
        }],
    });
    s.run("key ctrl+home; key down; key down; key down; cmd tests.run_at_cursor");
    assert_eq!(
        spawned(&s).last().map(String::as_str),
        Some("ctest -R trims")
    );
    let id = s.editor.window_view().terminal.unwrap().sessions[0].id;
    s.send(Event::TaskOutput {
        id,
        text: "1/1 Test #1: trims ... Passed".into(),
    });
    s.effects.clear();
    s.send(Event::TermExited { id, code: Some(0) });
    assert_eq!(
        s.effects,
        [Effect::PluginTerminal {
            plugin: "catch2".into(),
            id: "trims".into(),
            output: "1/1 Test #1: trims ... Passed".into(),
            code: Some(0),
        }]
    );
}

#[test]
fn a_plugin_runs_a_command_line_and_hears_what_it_printed() {
    let mut s = tested();
    s.send(Event::RunInTerminal {
        plugin: "rust".into(),
        id: "run-1".into(),
        name: "cargo test".into(),
        command: "cargo test --workspace".into(),
    });
    assert_eq!(spawned(&s), ["cargo test --workspace"]);
    let id = s.editor.window_view().terminal.unwrap().sessions[0].id;
    s.send(Event::TaskOutput {
        id,
        text: "test result: ok".into(),
    });
    s.effects.clear();
    s.send(Event::TermExited { id, code: Some(0) });
    assert!(s.effects.contains(&Effect::PluginTerminal {
        plugin: "rust".into(),
        id: "run-1".into(),
        output: "test result: ok".into(),
        code: Some(0),
    }));
}

#[test]
fn plugins_add_tasks_and_tests_that_they_run_and_report() {
    let mut s = Session::new(
        "def test_empty_input():\n    assert parse(\"\") == []\n\ndef test_unicode():\n    pass\n",
        Some("tests/test_parse.py"),
        100,
        30,
    );
    s.files.insert("tests/test_parse.py".into(), s.text());
    s.send(Event::SetTasks {
        plugin: "netlify".into(),
        set: String::new(),
        tasks: vec![merlin_core::Task {
            name: "Deploy preview".into(),
            command: None,
            group: TaskGroup::Build,
            source: "in the plugin".into(),
            plugin: None,
        }],
    });
    s.run("cmd task.run");
    let palette = s.editor.window_view().palette.unwrap();
    let at = palette
        .items
        .iter()
        .position(|(l, _)| l == "Deploy preview")
        .unwrap();
    assert_eq!(palette.extras[at].chip.as_deref(), Some("netlify"));
    assert_eq!(palette.extras[at].detail, "in the plugin");
    s.run("type deploy");
    s.run("key enter");
    assert!(s.effects.contains(&Effect::PluginCommand {
        plugin: "netlify".into(),
        command: "task:Deploy preview".into(),
    }));

    let test = |id: &str, label: &str, line| merlin_core::PluginTest {
        id: id.into(),
        label: label.into(),
        line,
        command: None,
    };
    s.send(Event::SetTests {
        plugin: "python".into(),
        path: "tests/test_parse.py".into(),
        group: String::new(),
        label: String::new(),
        tests: vec![
            test("p1", "test_empty_input", 0),
            test("p2", "test_unicode", 3),
        ],
    });
    // The first test is on line 0: no row over it, but the second has one.
    let view = s.editor.text_views().remove(0);
    assert_eq!(view.tests.len(), 1);
    s.send(Event::Ui(merlin_core::UiEvent::Press(
        view.tests[0].links[0].1.clone(),
    )));
    assert!(s.effects.contains(&Effect::PluginCommand {
        plugin: "python".into(),
        command: "run-test:p2".into(),
    }));
    s.send(Event::SetTestResults {
        plugin: "python".into(),
        results: vec![merlin_core::PluginTestResult {
            id: "p1".into(),
            outcome: Some(merlin_core::Outcome::Failed),
            message: "AssertionError: assert None == []".into(),
            line: Some(1),
        }],
    });
    let view = s.editor.text_views().remove(0);
    assert_eq!(view.failures[0].text, "AssertionError: assert None == []");
    assert_eq!(view.lines[0].test, Some(false));
    s.run("cmd view.tests");
    assert_eq!(labels(&s)[0], "python ");
}

#[test]
fn a_tasks_errors_become_problems_until_it_runs_clean() {
    let mut s = project();
    s.run("cmd task.run");
    s.run("type budgets");
    s.run("key enter");
    let id = s.editor.window_view().terminal.unwrap().sessions[0].id;
    let counts = |s: &Session| {
        s.editor
            .window_view()
            .status
            .iter()
            .map(|i| i.text.clone())
            .find(|t| t.starts_with('⊗'))
            .unwrap_or_default()
    };
    s.send(Event::TaskOutput {
        id,
        text: "src/main.rs:1:10: error: cannot find value `b` in this scope\n\
src/lib.rs:2:5: warning: unused import\n"
            .into(),
    });
    assert_eq!(counts(&s), "⊗ 1  ⚠ 1");
    // In the open file, under the word it points at: `a`.
    assert_eq!(marked(&s), 1);

    // A clean run takes them away.
    s.run("cmd task.run");
    s.run("type budgets");
    s.run("key enter");
    let id = s.editor.window_view().terminal.unwrap().sessions[0].id;
    s.send(Event::TaskOutput {
        id,
        text: "   Finished `dev` profile\n".into(),
    });
    assert_eq!(counts(&s), "⊗ 0  ⚠ 0");
    assert_eq!(marked(&s), 0);
}

/// How many characters of the text shown have a problem under them.
fn marked(s: &Session) -> usize {
    let view = s.editor.text_views().pop().unwrap();
    view.lines
        .iter()
        .flat_map(|l| &l.marks)
        .filter(|m| m.is_some())
        .count()
}
