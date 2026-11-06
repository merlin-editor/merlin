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
