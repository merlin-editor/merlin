//! Tasks and tests: Run Task's list from the project's files, each task in
//! a terminal of its own, and the Tests view with its results.

use merlin_core::script::Session;
use merlin_core::{Effect, Event, TaskGroup, TermOp, from_cargo, from_package, from_tasks_file};

fn project() -> Session {
    let mut s = Session::new("fn a() {}\n", Some("src/main.rs"), 100, 24);
    s.files.insert("src/main.rs".into(), s.text());
    s.files.insert(
        "Cargo.toml".into(),
        "[workspace]\nmembers = [\"crates/core\"]\n".into(),
    );
    s.files.insert(
        "package.json".into(),
        r#"{ "name": "site", "scripts": { "dev": "vite", "lint": "eslint ." } }"#.into(),
    );
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
fn tasks_come_from_cargo_npm_and_the_projects_own_file() {
    let cargo = from_cargo("[package]\nname = \"x\"\n");
    let names: Vec<&str> = cargo.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "cargo test",
            "cargo build --release",
            "cargo run",
            "cargo clippy"
        ]
    );
    assert_eq!(cargo[0].group, TaskGroup::Test);
    let npm = from_package(r#"{"scripts":{"test":"jest","dev":"vite --port \"3000\""}}"#);
    assert_eq!(npm[0].command.as_deref(), Some("npm run test"));
    assert_eq!(npm[0].group, TaskGroup::Test);
    assert_eq!(npm[1].name, "dev");
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
            .any(|(_, title, on)| *on && title == "Terminal: check-budgets"),
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
    assert_eq!(spawned(&s), ["cargo build --release"]);
}

const SAVING: &str = "use x::y;\n\n#[test]\nfn trims_on_save() {\n    let text = \"a\\nb\";\n    assert_eq!(text, \"a\\nb\\n\");\n}\n\n#[test]\nfn auto_save_after_a_delay() {\n    assert_eq!(1 + 1, 2);\n}\n";

const OUTPUT: &str = "\
❯ cargo test -p demo --test saving
   Compiling demo v0.1.0
     Running tests/saving.rs (target/debug/deps/saving-170921b8c002b1d9)

running 2 tests
test auto_save_after_a_delay ... ok
test trims_on_save ... FAILED

failures:

---- trims_on_save stdout ----

thread 'trims_on_save' (251039) panicked at crates/demo/tests/saving.rs:6:5:
assertion `left == right` failed
  left: \"a\\nb\"
 right: \"a\\nb\\n\"
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    trims_on_save

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.40s
";

fn tested() -> Session {
    let mut s = Session::new(SAVING, Some("crates/demo/tests/saving.rs"), 100, 30);
    s.files
        .insert("crates/demo/tests/saving.rs".into(), SAVING.into());
    s.files.insert(
        "Cargo.toml".into(),
        "[workspace]\nmembers = [\"crates/demo\"]\n".into(),
    );
    s.files.insert(
        "crates/demo/Cargo.toml".into(),
        "[package]\nname = \"demo\"\n".into(),
    );
    s.files.insert(
        "crates/demo/src/lib.rs".into(),
        "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn unit() {}\n}\n".into(),
    );
    s
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
fn the_tests_view_finds_tests_by_crate_and_file_and_runs_them() {
    let mut s = tested();
    s.run("cmd view.tests");
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
    assert_eq!(
        spawned(&s),
        ["cargo test -p demo --test saving trims_on_save"]
    );
    let id = s.editor.window_view().terminal.unwrap().sessions[0].id;
    s.send(Event::TaskOutput {
        id,
        text: OUTPUT.into(),
    });
    s.send(Event::TermExited {
        id,
        code: Some(101),
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
    assert!(
        text.tests[0]
            .status
            .as_ref()
            .unwrap()
            .0
            .starts_with("✕ failed")
    );
    // Run Failed runs just it again.
    s.run("cmd tests.run_failed");
    assert_eq!(
        spawned(&s).last().unwrap(),
        "cargo test --workspace -- trims_on_save"
    );
}
