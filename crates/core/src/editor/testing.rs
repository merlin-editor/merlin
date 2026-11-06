//! Tests, as VS Code's Testing view and rust-analyzer's lenses have them:
//! the project's tests in a tree by crate and file, ▶ Run Test and Debug
//! over each one in its file, a mark in the gutter for how it went, and a
//! failure's message under the line that failed. Tests run as tasks do,
//! in the terminal, and the results come from what cargo prints.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::{Editor, Effect, Focus};
use crate::panel::{Button, Entry, Node, Tone};
use crate::style::Hue;

/// The Tests view in the side bar.
pub(crate) const TESTS: &str = "merlin.tests";

/// The name test runs' terminal goes by, as one tab.
const RUN: &str = "cargo test";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    Skipped,
}

/// A test: its function's name and line, from 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestCase {
    pub name: String,
    pub line: usize,
}

/// A file with tests: its path in the project, and as its crate has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestFile {
    pub path: String,
    pub label: String,
    pub tests: Vec<TestCase>,
}

/// A crate's tests, or a plugin's.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct TestRoot {
    pub name: String,
    /// What finds and runs them: `cargo`, or a plugin's name.
    pub kind: String,
    pub files: Vec<TestFile>,
    /// How long its last run took, in seconds.
    pub time: Option<f64>,
    pub running: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestResult {
    pub outcome: Outcome,
    /// Why it failed, on one line.
    pub message: String,
    /// The line it failed on: a path in the project, and a line from 0.
    pub at: Option<(String, usize)>,
    pub when: i64,
}

#[derive(Debug, Default)]
pub(crate) struct Tests {
    pub roots: Vec<TestRoot>,
    /// Results by test: `path::name`.
    pub results: HashMap<String, TestResult>,
    /// The files asked for to find the tests in.
    reading: Option<Vec<PathBuf>>,
    /// Waiting for the project's files, to find the tests.
    listing: bool,
    pub found: bool,
    /// The project's Cargo.toml is a workspace's.
    workspace: bool,
    /// Runs the tests a saved file has, or its crate's.
    pub on_save: bool,
    /// Roots and files folded in the view.
    folded: HashSet<String>,
    /// The terminal the tests run in, and the crate the run was for.
    run: Option<(u64, Option<String>)>,
    /// Goes up whenever a test or result changes, for what's drawn.
    pub version: u64,
}

/// What a click or a command runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    All,
    Failed,
    Root(String),
    File(String),
    Test(String),
}

/// A test's key in the results: its file and name.
pub(crate) fn test_key(path: &str, name: &str) -> String {
    format!("{path}::{name}")
}

/// The tests in Rust source: functions marked `#[test]`, or a test
/// attribute such as `#[tokio::test]`, with the line of each `fn`.
pub fn rust_tests(text: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut marked = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("#[") {
            let attr = t.trim_start_matches("#[");
            if attr.starts_with("test]") || attr.contains("::test") {
                marked = true;
            }
            continue;
        }
        if !marked || t.is_empty() || t.starts_with("//") {
            continue;
        }
        marked = false;
        let Some(at) = t.find("fn ") else { continue };
        let name: String = t[at + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.push((name, i));
        }
    }
    out
}

/// A Cargo.toml's package name.
fn package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "name"
        {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// What one test run printed, as cargo and libtest print it.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Report {
    /// Each test: the target it's in (`tests/find.rs`, or `src/lib.rs`
    /// for unit tests), the crate a unit target names, its name, and how
    /// it went.
    pub tests: Vec<(String, Option<String>, String, Outcome)>,
    /// Each failure's message and where it failed, by test name.
    pub failures: HashMap<String, (String, Option<(String, usize)>)>,
    /// Each target's time, in seconds.
    pub times: Vec<(String, Option<String>, f64)>,
}

/// Reads cargo test's output.
pub(crate) fn parse_report(text: &str) -> Report {
    let mut report = Report::default();
    let mut target = String::new();
    let mut unit: Option<String> = None;
    let mut failing: Option<String> = None;
    let mut message: Vec<String> = Vec::new();
    let mut at: Option<(String, usize)> = None;
    let mut panicked = false;
    let finish = |report: &mut Report,
                  failing: &mut Option<String>,
                  message: &mut Vec<String>,
                  at: &mut Option<(String, usize)>| {
        if let Some(name) = failing.take() {
            let lines: Vec<String> = std::mem::take(message);
            // An assertion's left and right say more than its first line.
            let sides: Vec<&String> = lines
                .iter()
                .filter(|l| l.starts_with("left:") || l.starts_with("right:"))
                .collect();
            let text = if sides.is_empty() {
                lines.join(" ")
            } else {
                sides
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            report.failures.insert(name, (text, at.take()));
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Running ") {
            // `Running unittests src/lib.rs (target/debug/deps/merlin_core-…)`
            // or `Running tests/find.rs (target/debug/deps/find-…)`.
            let rest = rest.strip_prefix("unittests ").unwrap_or(rest);
            let (path, bin) = rest.split_once(" (").unwrap_or((rest, ""));
            target = path.to_string();
            unit = (!path.starts_with("tests/")).then(|| {
                let file = bin.rsplit('/').next().unwrap_or(bin);
                file.rsplit_once('-')
                    .map_or(file, |(name, _)| name)
                    .to_string()
            });
            continue;
        }
        if line.starts_with("Doc-tests ") {
            target.clear();
            continue;
        }
        if let Some(rest) = line.strip_prefix("test ")
            && let Some((name, how)) = rest.rsplit_once(" ... ")
            && !target.is_empty()
        {
            let outcome = match how {
                "ok" => Outcome::Passed,
                "FAILED" => Outcome::Failed,
                _ if how.starts_with("ignored") => Outcome::Skipped,
                _ => continue,
            };
            report
                .tests
                .push((target.clone(), unit.clone(), name.to_string(), outcome));
            continue;
        }
        if let Some(rest) = line.strip_prefix("test result: ")
            && let Some(secs) = rest
                .rsplit_once("finished in ")
                .and_then(|(_, t)| t.trim_end_matches('s').parse::<f64>().ok())
            && !target.is_empty()
        {
            report.times.push((target.clone(), unit.clone(), secs));
            continue;
        }
        if let Some(name) = line
            .strip_prefix("---- ")
            .and_then(|l| l.strip_suffix(" stdout ----"))
        {
            finish(&mut report, &mut failing, &mut message, &mut at);
            failing = Some(name.to_string());
            panicked = false;
            continue;
        }
        if failing.is_none() {
            continue;
        }
        if line == "failures:" || line.starts_with("note: run with") {
            if line == "failures:" {
                finish(&mut report, &mut failing, &mut message, &mut at);
            }
            panicked = false;
            continue;
        }
        if let Some(rest) = line.split_once("panicked at ").map(|(_, r)| r) {
            // `crates/core/tests/saving.rs:28:5:`
            let mut parts = rest.trim_end_matches(':').rsplitn(3, ':');
            let (_col, line_no, path) = (parts.next(), parts.next(), parts.next());
            if let (Some(path), Some(n)) = (path, line_no.and_then(|n| n.parse::<usize>().ok())) {
                at = Some((path.to_string(), n.saturating_sub(1)));
            }
            panicked = true;
            continue;
        }
        if panicked && !line.is_empty() {
            message.push(line.to_string());
        }
    }
    finish(&mut report, &mut failing, &mut message, &mut at);
    report
}

/// A test's lens, the row over its `fn`, in the text view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestLensView {
    /// The screen row, from the top of the text.
    pub row: i32,
    /// Columns in from the text's left, to line up with the `fn`.
    pub col: usize,
    /// Run Test and Debug, with the ids a click presses.
    pub links: Vec<(String, String)>,
    /// How it last went, and whether that was a failure.
    pub status: Option<(String, bool)>,
}

/// A failure's message, in the row under the line that failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestFailureView {
    pub row: i32,
    pub col: usize,
    pub text: String,
}

/// The tests of the file shown, and the rows they add.
#[derive(Debug, Default)]
pub(crate) struct DocTests {
    /// Each test's name, its `fn` line, and its result's key.
    pub tests: Vec<(String, usize, String)>,
    /// Lines a lens row follows: the one before each `fn`.
    pub lenses: HashMap<usize, usize>,
    /// Lines a failure's row follows, with its message.
    pub failures: HashMap<usize, String>,
}

/// The shown document's tests, by its id and version and the results'.
pub(crate) type DocTestsCache = ((u64, u64, u64), Rc<DocTests>);

impl Editor {
    /// The tests of the document shown, found again when it or the
    /// results change.
    pub(crate) fn doc_tests(&self) -> Rc<DocTests> {
        let doc = self.doc();
        let key = (doc.id, doc.buffer.version(), self.tests.version);
        if let Some((k, tests)) = self.tests_cache.borrow().as_ref()
            && *k == key
        {
            return tests.clone();
        }
        let mut out = DocTests::default();
        let path = doc.buffer.path.as_ref().map(|p| p.display().to_string());
        if let Some(path) = path.filter(|_| doc.review.is_none()) {
            let found = if path.ends_with(".rs") {
                rust_tests(&doc.buffer.text())
            } else {
                Vec::new()
            };
            for (i, (name, line)) in found.into_iter().enumerate() {
                if line > 0 {
                    out.lenses.insert(line - 1, i);
                }
                let key = test_key(&path, &name);
                if let Some(TestResult {
                    outcome: Outcome::Failed,
                    message,
                    at: Some((file, at)),
                    ..
                }) = self.tests.results.get(&key)
                    && *file == path
                    && *at <= doc.last_line()
                {
                    out.failures.insert(*at, message.clone());
                }
                out.tests.push((name, line, key));
            }
        }
        let out = Rc::new(out);
        *self.tests_cache.borrow_mut() = Some((key, out.clone()));
        out
    }

    /// Rows the tests add after `line`: a lens, a failure, or both.
    pub(crate) fn test_gap(&self, line: usize) -> usize {
        let tests = self.doc_tests();
        tests.lenses.contains_key(&line) as usize + tests.failures.contains_key(&line) as usize
    }

    pub(crate) fn has_test_rows(&self) -> bool {
        let tests = self.doc_tests();
        !tests.lenses.is_empty() || !tests.failures.is_empty()
    }

    /// The lens and failure rows after `line`, from screen row `row`, and
    /// the row after them.
    pub(crate) fn test_rows(
        &self,
        line: usize,
        mut row: i32,
        lenses: &mut Vec<TestLensView>,
        failures: &mut Vec<TestFailureView>,
    ) -> i32 {
        let tests = self.doc_tests();
        let indent = |l: usize| {
            self.doc()
                .line_chars(l)
                .iter()
                .take_while(|c| **c == ' ' || **c == '\t')
                .count()
        };
        if let Some(&i) = tests.lenses.get(&line) {
            let (_, fn_line, key) = &tests.tests[i];
            let status = self.tests.results.get(key).map(|r| {
                let when = crate::git::ago(r.when, self.now);
                match r.outcome {
                    Outcome::Passed => (format!("✓ passed {when}"), false),
                    Outcome::Failed => (format!("✕ failed {when}"), true),
                    Outcome::Skipped => ("ignored".to_string(), false),
                }
            });
            lenses.push(TestLensView {
                row,
                col: indent(*fn_line),
                links: vec![
                    ("▶ Run Test".into(), format!("test:run:{key}")),
                    ("Debug".into(), format!("test:debug:{key}")),
                ],
                status,
            });
            row += 1;
        }
        if let Some(text) = tests.failures.get(&line) {
            failures.push(TestFailureView {
                row,
                col: indent(line),
                text: text.clone(),
            });
            row += 1;
        }
        row
    }

    /// The gutter's mark on a test's `fn` line: whether it passed.
    pub(crate) fn test_mark(&self, line: usize) -> Option<bool> {
        let tests = self.doc_tests();
        let (_, _, key) = tests.tests.iter().find(|(_, l, _)| *l == line)?;
        match self.tests.results.get(key)?.outcome {
            Outcome::Passed => Some(true),
            Outcome::Failed => Some(false),
            Outcome::Skipped => None,
        }
    }

    /// The `tests.` commands.
    pub(crate) fn tests_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
        match id {
            "tests.run_all" => self.run_tests(Scope::All, false, fx),
            "tests.run_failed" => self.run_tests(Scope::Failed, false, fx),
            "tests.run_file" => {
                if let Some(path) = self.doc().buffer.path.clone() {
                    self.run_tests(Scope::File(path.display().to_string()), false, fx);
                }
            }
            "tests.run_at_cursor" => {
                let line = self.doc().line_col(self.cursor()).0;
                let tests = self.doc_tests();
                match tests.tests.iter().rev().find(|(_, l, _)| *l <= line) {
                    Some((_, _, key)) => self.run_tests(Scope::Test(key.clone()), false, fx),
                    None => self.notify("No test at the cursor".into()),
                }
            }
            "tests.on_save" => {
                self.tests.on_save = !self.tests.on_save;
                self.build_views();
            }
            "view.tests" => {
                self.show_view(TESTS, fx);
            }
            _ => {}
        }
    }

    /// A click on a lens's Run Test or Debug.
    pub(crate) fn test_press(&mut self, id: &str, fx: &mut Vec<Effect>) {
        if let Some(key) = id.strip_prefix("test:run:") {
            self.run_tests(Scope::Test(key.into()), false, fx);
        } else if let Some(key) = id.strip_prefix("test:debug:") {
            self.run_tests(Scope::Test(key.into()), true, fx);
        }
    }

    /// Finds the project's tests, the first time the view shows: the
    /// project's files are listed, then its crates' and Rust files read.
    pub(crate) fn find_tests(&mut self, fx: &mut Vec<Effect>) {
        if self.tests.found || self.tests.listing || self.tests.reading.is_some() {
            return;
        }
        self.tests.listing = true;
        // The host sends the list only when it changed: the Explorer's may
        // be it.
        match self
            .tree
            .as_ref()
            .map(|t| t.files.clone())
            .filter(|f| !f.is_empty())
        {
            Some(files) => self.tests_listed(&files, fx),
            None => fx.push(Effect::ListTree),
        }
    }

    /// The project's files, listed: reads what has tests in it.
    pub(crate) fn tests_listed(&mut self, files: &[String], fx: &mut Vec<Effect>) {
        if !std::mem::take(&mut self.tests.listing) {
            return;
        }
        let wanted: Vec<PathBuf> = files
            .iter()
            .filter(|f| {
                f.ends_with("Cargo.toml")
                    || (f.ends_with(".rs") && (f.starts_with("tests/") || f.contains("/tests/")))
                    || (f.ends_with(".rs") && (f.starts_with("src/") || f.contains("/src/")))
            })
            .map(PathBuf::from)
            .collect();
        self.tests.reading = Some(wanted.clone());
        fx.push(Effect::ReadFiles(wanted));
    }

    /// The crates' and Rust files read: the tree of tests.
    pub(crate) fn tests_read(&mut self, files: &[(PathBuf, Option<String>)]) {
        let Some(wanted) = &self.tests.reading else {
            return;
        };
        if files.len() != wanted.len() || files.iter().zip(wanted).any(|((p, _), w)| p != w) {
            return;
        }
        self.tests.reading = None;
        self.tests.found = true;
        // Each crate, by its folder, deepest first.
        let mut crates: Vec<(String, String)> = files
            .iter()
            .filter(|(p, _)| p.file_name().is_some_and(|n| n == "Cargo.toml"))
            .filter_map(|(p, text)| {
                let text = text.as_deref()?;
                if p == Path::new("Cargo.toml") && text.lines().any(|l| l.trim() == "[workspace]") {
                    self.tests.workspace = true;
                }
                let dir = p
                    .parent()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default();
                Some((dir, package_name(text)?))
            })
            .collect();
        crates.sort_by_key(|(dir, _)| std::cmp::Reverse(dir.len()));
        let mut roots: Vec<TestRoot> = Vec::new();
        for (path, text) in files {
            let (Some(text), Some(name)) = (text.as_deref(), path.to_str()) else {
                continue;
            };
            if !name.ends_with(".rs") {
                continue;
            }
            let tests = rust_tests(text);
            if tests.is_empty() {
                continue;
            }
            let Some((dir, krate)) = crates
                .iter()
                .find(|(dir, _)| dir.is_empty() || name.starts_with(&format!("{dir}/")))
            else {
                continue;
            };
            let label = match dir.is_empty() {
                true => name.to_string(),
                false => name[dir.len() + 1..].to_string(),
            };
            let file = TestFile {
                path: name.to_string(),
                label,
                tests: tests
                    .into_iter()
                    .map(|(name, line)| TestCase { name, line })
                    .collect(),
            };
            match roots.iter_mut().find(|r| r.name == *krate) {
                Some(root) => root.files.push(file),
                None => roots.push(TestRoot {
                    name: krate.clone(),
                    kind: "cargo".into(),
                    files: vec![file],
                    ..TestRoot::default()
                }),
            }
        }
        for root in &mut roots {
            root.files.sort_by(|a, b| a.label.cmp(&b.label));
        }
        roots.sort_by(|a, b| a.name.cmp(&b.name));
        // Crates start folded but the first, as the view has room for.
        for root in roots.iter().skip(1) {
            self.tests.folded.insert(format!("r:{}", root.name));
        }
        self.tests.roots = roots;
        self.tests.version += 1;
        self.build_views();
    }

    /// The crate and file a test's key names.
    fn test_place(&self, key: &str) -> Option<(&TestRoot, &TestFile, String)> {
        let (path, name) = key.rsplit_once("::")?;
        for root in &self.tests.roots {
            if let Some(file) = root.files.iter().find(|f| f.path == path) {
                return Some((root, file, name.to_string()));
            }
        }
        None
    }

    fn file_place(&self, path: &str) -> Option<(&TestRoot, &TestFile)> {
        self.tests
            .roots
            .iter()
            .find_map(|r| r.files.iter().find(|f| f.path == path).map(|f| (r, f)))
    }

    /// Runs some tests in the test terminal, or one under the debugger in
    /// a terminal of its own.
    pub(crate) fn run_tests(&mut self, scope: Scope, debug: bool, fx: &mut Vec<Effect>) {
        // The crate's target for a file: an integration test's own, or
        // the crate's unit tests with the file's tests named.
        let target = |file: &TestFile, names: &[&str]| -> String {
            match file
                .label
                .strip_prefix("tests/")
                .and_then(|f| f.strip_suffix(".rs"))
            {
                Some(stem) if !stem.contains('/') => {
                    let filter = names.join(" ");
                    format!(" --test {stem} {filter}").trim_end().to_string()
                }
                _ => format!(" -- {}", names.join(" ")),
            }
        };
        let (command, krate) = match &scope {
            Scope::All => {
                let all = if self.tests.workspace {
                    " --workspace"
                } else {
                    ""
                };
                (format!("cargo test{all}"), None)
            }
            Scope::Root(name) => (format!("cargo test -p {name}"), Some(name.clone())),
            Scope::File(path) => {
                let Some((root, file)) = self.file_place(path) else {
                    return self.notify("No tests in this file".into());
                };
                let names: Vec<&str> = match file.label.starts_with("tests/") {
                    true => Vec::new(),
                    false => file.tests.iter().map(|t| t.name.as_str()).collect(),
                };
                (
                    format!("cargo test -p {}{}", root.name, target(file, &names)),
                    Some(root.name.clone()),
                )
            }
            Scope::Test(key) => {
                let Some((root, file, name)) = self.test_place(key) else {
                    return self.notify("Open the Tests view to find the tests first".into());
                };
                (
                    format!(
                        "cargo test -p {}{}",
                        root.name,
                        target(file, &[name.as_str()])
                    ),
                    Some(root.name.clone()),
                )
            }
            Scope::Failed => {
                let failed: Vec<String> = self
                    .tests
                    .results
                    .iter()
                    .filter(|(_, r)| r.outcome == Outcome::Failed)
                    .filter_map(|(k, _)| k.rsplit_once("::").map(|(_, n)| n.to_string()))
                    .collect();
                if failed.is_empty() {
                    return self.notify("No tests failed".into());
                }
                let all = if self.tests.workspace {
                    " --workspace"
                } else {
                    ""
                };
                (format!("cargo test{all} -- {}", failed.join(" ")), None)
            }
        };
        if debug {
            // Under rust-gdb, in the terminal, as a runner cargo starts the
            // test with.
            let command = command.replacen(
                "cargo test",
                "cargo test --config \"target.'cfg(all())'.runner='rust-gdb -q --args'\"",
                1,
            );
            let name = match &scope {
                Scope::Test(key) => key.rsplit("::").next().unwrap_or(key).to_string(),
                _ => "tests".into(),
            };
            self.start_command(&format!("Debug {name}"), &command, fx);
            return;
        }
        for root in &mut self.tests.roots {
            root.running = krate.as_ref().is_none_or(|k| *k == root.name);
        }
        let term = self.start_command(RUN, &command, fx);
        self.tests.run = Some((term, krate));
        self.tests.version += 1;
        self.build_views();
    }

    /// A test run's output: each test's result.
    pub(crate) fn tests_output(&mut self, term: u64, text: &str) {
        let krate = match &self.tests.run {
            Some((t, k)) if *t == term => k.clone(),
            // A test task run from Run Task counts too.
            _ if self
                .tasks
                .runs
                .iter()
                .any(|r| r.term == term && r.command.starts_with("cargo test")) =>
            {
                None
            }
            _ => return,
        };
        self.tests.run = None;
        let report = parse_report(text);
        let now = self.now;
        let mut times: HashMap<String, f64> = HashMap::new();
        for (target, unit, name, outcome) in &report.tests {
            let Some((key, root)) =
                self.match_test(target, unit.as_deref(), krate.as_deref(), name)
            else {
                continue;
            };
            let (message, at) = report.failures.get(name).cloned().unwrap_or_default();
            let at = at
                .filter(|(p, _)| self.file_place(p).is_some())
                .or_else(|| {
                    (*outcome == Outcome::Failed)
                        .then(|| self.test_place(&key))
                        .flatten()
                        .and_then(|(_, file, n)| {
                            let t = file.tests.iter().find(|t| t.name == n)?;
                            Some((file.path.clone(), t.line))
                        })
                });
            self.tests.results.insert(
                key,
                TestResult {
                    outcome: *outcome,
                    message,
                    at,
                    when: now,
                },
            );
            times.entry(root).or_default();
        }
        // Each target's time goes to the crate its tests are in.
        for (target, unit, secs) in &report.times {
            let root = report
                .tests
                .iter()
                .filter(|(t, u, _, _)| t == target && u == unit)
                .find_map(|(t, u, name, _)| {
                    self.match_test(t, u.as_deref(), krate.as_deref(), name)
                })
                .map(|(_, root)| root);
            if let Some(root) = root {
                *times.entry(root).or_default() += secs;
            }
        }
        for root in &mut self.tests.roots {
            if let Some(t) = times.get(&root.name) {
                root.time = Some(*t);
            }
            root.running = false;
        }
        self.tests.version += 1;
        self.build_views();
    }

    /// The test a line of cargo's output names: its key and crate.
    fn match_test(
        &self,
        target: &str,
        unit: Option<&str>,
        krate: Option<&str>,
        name: &str,
    ) -> Option<(String, String)> {
        let short = name.rsplit("::").next().unwrap_or(name);
        let modules: Vec<&str> = name.split("::").collect();
        let mut best: Option<(usize, String, String)> = None;
        for root in &self.tests.roots {
            let snake = root.name.replace('-', "_");
            if unit.is_some_and(|u| u != snake) || krate.is_some_and(|k| k != root.name) {
                continue;
            }
            for file in &root.files {
                let fits = match unit {
                    Some(_) => !file.label.starts_with("tests/"),
                    None => file.label == target,
                };
                if !fits || !file.tests.iter().any(|t| t.name == short) {
                    continue;
                }
                // The file whose path has most of the test's modules.
                let score = modules.iter().filter(|m| file.label.contains(*m)).count();
                if best.as_ref().is_none_or(|(s, _, _)| score > *s) {
                    best = Some((score, test_key(&file.path, short), root.name.clone()));
                }
            }
        }
        best.map(|(_, key, root)| (key, root))
    }

    /// A Rust file saved: its tests found again, and run with On Save.
    pub(crate) fn tests_saved(&mut self, doc: u64, fx: &mut Vec<Effect>) {
        let Some(i) = self.index_of(doc) else { return };
        let Some(path) = self.docs[i]
            .buffer
            .path
            .as_ref()
            .map(|p| p.display().to_string())
        else {
            return;
        };
        if !path.ends_with(".rs") || !self.tests.found {
            return;
        }
        let found = rust_tests(&self.docs[i].buffer.text());
        let mut root_name = None;
        for root in &mut self.tests.roots {
            if let Some(file) = root.files.iter_mut().find(|f| f.path == path) {
                file.tests = found
                    .iter()
                    .map(|(name, line)| TestCase {
                        name: name.clone(),
                        line: *line,
                    })
                    .collect();
                root_name = Some(root.name.clone());
            }
        }
        self.tests.version += 1;
        self.build_views();
        if self.tests.on_save {
            match root_name {
                Some(_) => self.run_tests(Scope::File(path), false, fx),
                None => self.run_tests(Scope::All, false, fx),
            }
        }
    }

    /// The counts the view's title shows: passed and failed.
    pub(crate) fn test_counts(&self) -> (usize, usize) {
        let count = |o| {
            self.tests
                .results
                .values()
                .filter(|r| r.outcome == o)
                .count()
        };
        (count(Outcome::Passed), count(Outcome::Failed))
    }

    /// How a root, file or test went: failed if any failed, passed if all
    /// that ran passed, none if none ran.
    fn outcome_of<'a>(&self, keys: impl Iterator<Item = String> + 'a) -> Option<Outcome> {
        let mut any = None;
        for key in keys {
            match self.tests.results.get(&key).map(|r| r.outcome) {
                Some(Outcome::Failed) => return Some(Outcome::Failed),
                Some(Outcome::Passed) => any = Some(Outcome::Passed),
                _ => {}
            }
        }
        any
    }

    /// The Tests view: Run All, Run Failed and On Save, the tests by crate
    /// and file, then the tasks run.
    pub(crate) fn tests_nodes(&self) -> Vec<Node> {
        let on_save = if self.tests.on_save {
            "● On Save"
        } else {
            "⟳ On Save"
        };
        let mut nodes = vec![Node::Tools(vec![
            Button {
                id: "t:all".into(),
                label: "▶ Run All".into(),
            },
            Button {
                id: "t:failed".into(),
                label: "Run Failed".into(),
            },
            Button {
                id: "t:on_save".into(),
                label: on_save.into(),
            },
        ])];
        let mark = |o: Option<Outcome>, running: bool| -> (char, Hue) {
            match (running, o) {
                (true, _) => ('⟳', Hue::Grey),
                (_, Some(Outcome::Failed)) => ('✕', Hue::Red),
                (_, Some(Outcome::Passed)) => ('✓', Hue::Green),
                _ => ('○', Hue::Grey),
            }
        };
        if !self.tests.found {
            nodes.push(Node::Styled("Finding tests…".into(), Tone::Muted));
        } else if self.tests.roots.is_empty() {
            nodes.push(Node::Styled("No tests found".into(), Tone::Muted));
        }
        let several = self
            .tests
            .roots
            .iter()
            .map(|r| &r.kind)
            .collect::<HashSet<_>>()
            .len()
            > 1;
        for root in &self.tests.roots {
            let keys = || {
                root.files
                    .iter()
                    .flat_map(|f| f.tests.iter().map(|t| test_key(&f.path, &t.name)))
            };
            let outcome = self.outcome_of(keys());
            let id = format!("r:{}", root.name);
            let open = !self.tests.folded.contains(&id);
            nodes.push(Node::Entry(Entry {
                label: root.name.clone(),
                icon: Some(mark(outcome, root.running)),
                detail: if several {
                    root.kind.clone()
                } else {
                    String::new()
                },
                badge: match (root.running, outcome, root.time) {
                    (true, _, _) => "running".into(),
                    (_, None, _) => "not run".into(),
                    (_, _, Some(t)) => format!("{t:.1}s"),
                    _ => String::new(),
                },
                tone: if outcome.is_none() && !root.running {
                    Tone::Muted
                } else {
                    Tone::Normal
                },
                action: Some(('▶', format!("t:run:{id}"))),
                id: format!("t:{id}"),
                ..Default::default()
            }));
            if !open {
                continue;
            }
            for file in &root.files {
                let id = format!("f:{}", file.path);
                let keys = file.tests.iter().map(|t| test_key(&file.path, &t.name));
                let outcome = self.outcome_of(keys);
                let open = !self.tests.folded.contains(&id);
                nodes.push(Node::Entry(Entry {
                    label: file.label.clone(),
                    icon: Some(mark(outcome, root.running)),
                    depth: 1,
                    badge: file.tests.len().to_string(),
                    action: Some(('▶', format!("t:run:{id}"))),
                    id: format!("t:{id}"),
                    ..Default::default()
                }));
                if !open {
                    continue;
                }
                for test in &file.tests {
                    let key = test_key(&file.path, &test.name);
                    let outcome = self.tests.results.get(&key).map(|r| r.outcome);
                    nodes.push(Node::Entry(Entry {
                        label: test.name.clone(),
                        icon: Some(mark(outcome, root.running)),
                        depth: 2,
                        action: Some(('▶', format!("t:run:t:{key}"))),
                        id: format!("t:t:{key}"),
                        ..Default::default()
                    }));
                }
            }
        }
        if !self.tasks.runs.is_empty() {
            nodes.push(Node::Entry(Entry {
                id: "t:sec".into(),
                label: "Tasks".into(),
                heading: true,
                ..Default::default()
            }));
            for run in &self.tasks.runs {
                let (icon, badge) = match run.exited {
                    None => (('●', Hue::Green), "running".to_string()),
                    Some(0) => (('○', Hue::Grey), "done".to_string()),
                    Some(_) => (('✕', Hue::Red), "failed".to_string()),
                };
                nodes.push(Node::Entry(Entry {
                    id: format!("t:task:{}", run.term),
                    label: run.name.clone(),
                    icon: Some(icon),
                    badge,
                    ..Default::default()
                }));
            }
        }
        nodes
    }

    /// A click in the Tests view.
    pub(crate) fn tests_click(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let Some(id) = id.strip_prefix("t:") else {
            return;
        };
        match id {
            "all" => return self.run_tests(Scope::All, false, fx),
            "failed" => return self.run_tests(Scope::Failed, false, fx),
            "on_save" => return self.tests_command("tests.on_save", fx),
            "sec" => return,
            _ => {}
        }
        if let Some(what) = id.strip_prefix("run:") {
            let scope = if let Some(name) = what.strip_prefix("r:") {
                Scope::Root(name.into())
            } else if let Some(path) = what.strip_prefix("f:") {
                Scope::File(path.into())
            } else if let Some(key) = what.strip_prefix("t:") {
                Scope::Test(key.into())
            } else {
                return;
            };
            return self.run_tests(scope, false, fx);
        }
        if let Some(term) = id.strip_prefix("task:").and_then(|t| t.parse().ok()) {
            return self.focus_terminal(term);
        }
        if let Some(key) = id.strip_prefix("t:") {
            if let Some((_, file, name)) = self.test_place(key) {
                let line = file
                    .tests
                    .iter()
                    .find(|t| t.name == name)
                    .map_or(0, |t| t.line);
                let path = PathBuf::from(&file.path);
                self.open_at(path, line as u32, 0, fx);
                self.focus = Focus::Editor;
            }
            return;
        }
        // A crate or file: fold or unfold it.
        if !self.tests.folded.remove(id) {
            self.tests.folded.insert(id.to_string());
        }
        self.build_views();
    }
}
