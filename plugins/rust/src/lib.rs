//! Rust language plugin: tree-sitter syntax highlighting, `rust-analyzer`,
//! and cargo's side: the project's tests by crate, run in the terminal and
//! read back from what cargo prints, Run Task's cargo tasks, and a test
//! under the debugger.

mod cargo;

use std::collections::{BTreeMap, BTreeSet};

use merlin_grammar::Grammar;
use merlin_lsp::{Spec, WithLsp};
use merlin_plugin::{
    BufferEvent, DebugTarget, Plugin, Task, TaskGroup, TestItem, TestOutcome, TestResult, Token,
    host,
};

use crate::cargo::{Outcome, TestFile};

const RUST_SERVER: Spec = Spec {
    program: "rust-analyzer",
    args: &[],
    files: &[("rs", "rust")],
    settings: None,
    options: &[
        ("check-command", "check.command"),
        ("features", "cargo.features"),
    ],
    install: Some("rustup component add rust-analyzer"),
};

merlin_plugin::export!(WithLsp<Rust> => WithLsp::new(Rust::new(), "Rust", &[RUST_SERVER]));

/// What a command line run in the terminal was for.
enum Run {
    /// Tests, of one crate or of all.
    Tests { krate: Option<String> },
    /// A test's binary, built to debug the test.
    Build { test: String },
}

pub struct Rust {
    grammar: Grammar,
    /// The crates' folders and names, deepest first, once read.
    crates: Option<Vec<(String, String)>>,
    workspace: bool,
    /// The files with tests, by path.
    files: BTreeMap<String, TestFile>,
    /// The tests that failed last.
    failed: BTreeSet<String>,
    runs: BTreeMap<String, Run>,
    next_run: u64,
}

impl Rust {
    fn new() -> Self {
        Self {
            grammar: Grammar::new(
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY,
                &["rs"],
            ),
            crates: None,
            workspace: false,
            files: BTreeMap::new(),
            failed: BTreeSet::new(),
            runs: BTreeMap::new(),
            next_run: 0,
        }
    }

    /// The project's crates, read the first time, with Run Task's cargo
    /// tasks from its Cargo.toml.
    fn crates(&mut self) -> &[(String, String)] {
        if self.crates.is_none() {
            let files = host::list_files().unwrap_or_default();
            let mut crates = Vec::new();
            for path in files
                .iter()
                .filter(|f| *f == "Cargo.toml" || f.ends_with("/Cargo.toml"))
            {
                let Ok(text) = host::read_file(path) else {
                    continue;
                };
                let dir = path.strip_suffix("Cargo.toml").unwrap_or_default();
                let dir = dir.trim_end_matches('/').to_string();
                if dir.is_empty() {
                    self.workspace = cargo::manifest_kind(&text).0;
                    tasks(&text);
                }
                if let Some(name) = cargo::package_name(&text) {
                    crates.push((dir, name));
                }
            }
            crates.sort_by_key(|(dir, _)| std::cmp::Reverse(dir.len()));
            self.crates = Some(crates);
        }
        self.crates.as_deref().unwrap_or_default()
    }

    /// A file's tests, found in its text, for the Tests view and its ▶s.
    fn publish(&mut self, path: &str, text: &str) {
        let tests = cargo::rust_tests(text);
        let Some((krate, label)) =
            cargo::crate_of(self.crates(), path).map(|(k, l)| (k.to_string(), l))
        else {
            return;
        };
        let file = TestFile {
            path: path.to_string(),
            krate: krate.clone(),
            label: label.clone(),
            tests,
        };
        let known = self.files.get(path);
        if known == Some(&file) || (known.is_none() && file.tests.is_empty()) {
            return;
        }
        let items: Vec<TestItem> = file
            .tests
            .iter()
            .map(|(name, line)| TestItem {
                id: cargo::test_id(path, name),
                label: name.clone(),
                line: *line,
                command: None,
            })
            .collect();
        host::set_tests(path, &krate, &label, &items);
        if file.tests.is_empty() {
            self.files.remove(path);
        } else {
            self.files.insert(path.to_string(), file);
        }
    }

    /// Every test in the project, for the Tests view, with its crates read
    /// again.
    fn find_tests(&mut self) {
        self.crates = None;
        self.crates();
        let files = host::list_files().unwrap_or_default();
        for path in files.iter().filter(|f| {
            f.ends_with(".rs")
                && ["src/", "tests/"]
                    .iter()
                    .any(|d| f.starts_with(d) || f.contains(&format!("/{d}")))
        }) {
            if let Ok(text) = host::read_file(path) {
                self.publish(path, &text);
            }
        }
    }

    /// The active file, relative to the workspace, if it's Rust.
    fn active_rust(&self) -> Option<String> {
        let path = host::active_path().ok()??;
        let root = format!("{}/", host::workspace_root());
        let path = path.strip_prefix(&root).map_or(path.clone(), String::from);
        path.ends_with(".rs").then_some(path)
    }

    /// Runs cargo in the test terminal, to read how the tests went.
    fn run(&mut self, command: String, krate: Option<String>) {
        let id = self.run_id();
        if let Err(err) = host::run_in_terminal(&id, "cargo test", &command) {
            return host::notify(&err);
        }
        host::set_test_run(krate.as_deref().unwrap_or_default(), true, None);
        self.runs.insert(id, Run::Tests { krate });
    }

    fn run_id(&mut self) -> String {
        self.next_run += 1;
        format!("run-{}", self.next_run)
    }

    fn all(&self) -> &'static str {
        if self.workspace { " --workspace" } else { "" }
    }

    /// The editor's `run-tests:<scope>`.
    fn run_tests(&mut self, scope: &str) {
        self.crates();
        let all = self.all();
        match scope {
            "all" => self.run(format!("cargo test{all}"), None),
            "failed" => {
                let names: Vec<String> = self
                    .failed
                    .iter()
                    .filter_map(|id| id.rsplit_once("::").map(|(_, n)| n.to_string()))
                    .collect();
                if names.is_empty() {
                    return host::notify("No tests failed");
                }
                self.run(format!("cargo test{all} -- {}", names.join(" ")), None);
            }
            _ => {
                if let Some(krate) = scope.strip_prefix("group:") {
                    self.run(format!("cargo test -p {krate}"), Some(krate.to_string()));
                } else if let Some(file) =
                    scope.strip_prefix("file:").and_then(|p| self.files.get(p))
                {
                    let (command, krate) = (cargo::file_command(file), file.krate.clone());
                    self.run(command, Some(krate));
                }
            }
        }
    }

    /// A test's file and name, by its id.
    fn test(&self, id: &str) -> Option<(&TestFile, String)> {
        let (path, name) = id.rsplit_once("::")?;
        Some((self.files.get(path)?, name.to_string()))
    }

    /// Debugs a test: its binary is built first, and found in what cargo
    /// printed.
    fn debug_test(&mut self, test: &str) {
        let Some((file, _)) = self.test(test) else {
            return;
        };
        let command = cargo::build_command(file);
        let id = self.run_id();
        if let Err(err) = host::run_in_terminal(&id, "Build test", &command) {
            return host::notify(&err);
        }
        self.runs.insert(
            id,
            Run::Build {
                test: test.to_string(),
            },
        );
    }

    /// A test run ended: each test's result, and each crate's time.
    fn tests_ran(&mut self, krate: Option<String>, output: &str) {
        let report = cargo::parse_report(output);
        let files: Vec<TestFile> = self.files.values().cloned().collect();
        let mut results = Vec::new();
        let mut times: BTreeMap<String, f64> = BTreeMap::new();
        for (target, unit, name, outcome) in &report.tests {
            let Some(file) =
                cargo::match_test(&files, target, unit.as_deref(), krate.as_deref(), name)
            else {
                continue;
            };
            let short = name.rsplit("::").next().unwrap_or(name);
            let id = cargo::test_id(&file.path, short);
            let (message, at) = report.failures.get(name).cloned().unwrap_or_default();
            // Where it failed, in its own file; else its `fn`.
            let line = at
                .filter(|(p, _)| *p == file.path)
                .map(|(_, l)| l)
                .or_else(|| {
                    (*outcome == Outcome::Failed)
                        .then(|| file.tests.iter().find(|(n, _)| n == short).map(|(_, l)| *l))
                        .flatten()
                });
            match outcome {
                Outcome::Failed => self.failed.insert(id.clone()),
                _ => self.failed.remove(&id),
            };
            results.push(TestResult {
                id,
                outcome: match outcome {
                    Outcome::Passed => TestOutcome::Passed,
                    Outcome::Failed => TestOutcome::Failed,
                    Outcome::Skipped => TestOutcome::Skipped,
                },
                message,
                line,
            });
            times.entry(file.krate.clone()).or_default();
        }
        // Each target's time goes to the crate its tests are in.
        for (target, unit, secs) in &report.times {
            let krate = report
                .tests
                .iter()
                .filter(|(t, u, _, _)| t == target && u == unit)
                .find_map(|(t, u, name, _)| {
                    cargo::match_test(&files, t, u.as_deref(), krate.as_deref(), name)
                })
                .map(|f| f.krate.clone());
            if let Some(krate) = krate {
                *times.entry(krate).or_default() += secs;
            }
        }
        host::set_test_results(&results);
        host::set_test_run("", false, None);
        for (krate, secs) in times {
            host::set_test_run(&krate, false, Some(secs));
        }
    }

    /// A test's binary built: the debugger starts on it, or, without one,
    /// rust-gdb in a terminal.
    fn built(&mut self, test: &str, output: &str, code: Option<i32>) {
        let Some((file, name)) = self.test(test) else {
            return;
        };
        if code != Some(0) {
            return host::notify(&format!("{name} didn't build, so debugging didn't start"));
        }
        let binaries = cargo::executables(output);
        let Some(binary) = cargo::binary_for(&binaries, &file.label) else {
            return host::notify("The test built, but cargo didn't say where its binary is");
        };
        let program = format!("{}/{binary}", host::workspace_root());
        let args = vec![
            name.clone(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ];
        let fallback = format!("rust-gdb -q --args {program} {}", args.join(" "));
        host::start_debugging(
            &DebugTarget {
                name: format!("Test {name}"),
                debugger: String::new(),
                language: "rust".into(),
                program,
                args,
                build: String::new(),
                detail: String::new(),
                launch: "{}".into(),
            },
            Some(&fallback),
        );
    }
}

/// Run Task's cargo tasks, from the workspace's Cargo.toml.
fn tasks(text: &str) {
    let tasks: Vec<Task> = cargo::cargo_tasks(text)
        .into_iter()
        .map(|(name, command, group)| Task {
            name,
            command,
            group: match group {
                "test" => TaskGroup::Test,
                "build" => TaskGroup::Build,
                _ => TaskGroup::Other,
            },
            detail: "Cargo.toml".into(),
        })
        .collect();
    host::set_tasks("cargo", &tasks);
}

impl Plugin for Rust {
    fn run_command(&mut self, id: &str) {
        if id == "find-tests" {
            self.find_tests();
        } else if id == "find-tasks" {
            // Run Task opened: Cargo.toml read again, as it may have
            // changed.
            self.crates = None;
            self.crates();
        } else if let Some(scope) = id.strip_prefix("run-tests:") {
            self.run_tests(scope);
        } else if let Some(test) = id.strip_prefix("run-test:") {
            self.crates();
            if let Some((file, name)) = self.test(test) {
                let (command, krate) = (cargo::test_command(file, &name), file.krate.clone());
                self.run(command, Some(krate));
            }
        } else if let Some(test) = id.strip_prefix("debug-test:") {
            self.debug_test(test);
        } else if id
            .strip_prefix("task:")
            .is_some_and(|t| t.starts_with("cargo test"))
        {
            self.run_tests("all");
        }
    }

    fn buffer(&mut self, event: BufferEvent) {
        self.grammar.buffer(event.clone());
        // The shown file's tests, as it changes.
        if matches!(event, BufferEvent::Switched | BufferEvent::Changed)
            && let Some(path) = self.active_rust()
            && let Ok(text) = host::active_text()
        {
            self.publish(&path, &text);
        }
    }

    fn terminal(&mut self, id: &str, output: &str, code: Option<i32>) {
        match self.runs.remove(id) {
            Some(Run::Tests { krate }) => self.tests_ran(krate, output),
            Some(Run::Build { test }) => self.built(&test, output, code),
            None => {}
        }
    }

    fn colour_text(&mut self, _language: &str, text: &str) -> Vec<Token> {
        self.grammar.colour_text(text)
    }
}
