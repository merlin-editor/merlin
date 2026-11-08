//! Tests, as VS Code's Testing view and rust-analyzer's lenses have them:
//! the project's tests in a tree by group, such as a crate, and file, ▶ Run
//! Test and Debug over each one in its file, a mark in the gutter for how
//! it went, and a failure's message under the line that failed. Plugins
//! find the tests, run them and say how they went: the Rust plugin with
//! cargo, say.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::{Editor, Effect, Focus};
use crate::panel::{Button, Entry, Node, Tone};
use crate::style::Hue;

/// The Tests view in the side bar.
pub(crate) const TESTS: &str = "merlin.tests";

/// The name test runs' terminal goes by, as one tab.
const RUN: &str = "Tests";

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

/// A group of a plugin's tests, such as a crate's.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct TestRoot {
    /// The group's name, or the plugin's for its own.
    pub name: String,
    /// The plugin that finds and runs them.
    pub kind: String,
    /// The group as the plugin names it: empty for its own.
    pub group: String,
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
    /// Plugins were asked for every test, as the view first showed.
    pub found: bool,
    /// Runs the tests a saved file has, or its language's.
    pub on_save: bool,
    /// Roots and files folded in the view.
    folded: HashSet<String>,
    /// Goes up whenever a test or result changes, for what's drawn.
    pub version: u64,
    /// Plugins' tests, by file.
    pub plugin_files: Vec<PluginFile>,
}

/// A plugin's test in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginTest {
    pub id: String,
    pub label: String,
    /// Its line, from 0.
    pub line: u32,
    /// The command line that runs it in the test terminal; none for one
    /// the plugin runs.
    pub command: Option<String>,
}

/// How a plugin's test went; no outcome while it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginTestResult {
    pub id: String,
    pub outcome: Option<Outcome>,
    pub message: String,
    pub line: Option<u32>,
}

/// A plugin's tests in one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PluginFile {
    pub plugin: String,
    pub path: String,
    pub group: String,
    pub tests: Vec<PluginTest>,
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
            let found: Vec<(String, usize)> = self
                .tests
                .plugin_files
                .iter()
                .filter(|f| f.path == path)
                .flat_map(|f| f.tests.iter().map(|t| (t.label.clone(), t.line as usize)))
                .collect();
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
                    && !message.is_empty()
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

    /// Asks every plugin for its tests, the first time the view shows.
    pub(crate) fn find_tests(&mut self, fx: &mut Vec<Effect>) {
        if std::mem::replace(&mut self.tests.found, true) {
            return;
        }
        for plugin in &self.plugins {
            fx.push(Effect::PluginCommand {
                plugin: plugin.name.clone(),
                command: "find-tests".into(),
            });
        }
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

    /// Runs some tests: one with a command line in the test terminal, the
    /// rest by their plugins, as `run-test:<id>`, `debug-test:<id>` or
    /// `run-tests:<scope>`.
    pub(crate) fn run_tests(&mut self, scope: Scope, debug: bool, fx: &mut Vec<Effect>) {
        self.find_tests(fx);
        let (plugins, what): (Vec<String>, String) = match &scope {
            Scope::Test(key) => {
                return match self.plugin_test(key) {
                    Some((plugin, test)) => self.run_plugin_test(&plugin, &test, debug, fx),
                    None => self.notify("Open the Tests view to find the tests first".into()),
                };
            }
            Scope::File(path) => (
                self.tests
                    .plugin_files
                    .iter()
                    .filter(|f| f.path == *path)
                    .map(|f| f.plugin.clone())
                    .collect(),
                format!("file:{path}"),
            ),
            Scope::Root(name) => {
                let Some(root) = self.tests.roots.iter().find(|r| r.name == *name) else {
                    return;
                };
                (vec![root.kind.clone()], format!("group:{}", root.group))
            }
            Scope::All => (
                self.tests.roots.iter().map(|r| r.kind.clone()).collect(),
                "all".into(),
            ),
            Scope::Failed => (
                self.tests
                    .results
                    .iter()
                    .filter(|(_, r)| r.outcome == Outcome::Failed)
                    .filter_map(|(key, _)| self.plugin_test(key).map(|(plugin, _)| plugin))
                    .collect(),
                "failed".into(),
            ),
        };
        let mut plugins = plugins;
        plugins.sort();
        plugins.dedup();
        if plugins.is_empty() {
            return self.notify(match scope {
                Scope::Failed => "No tests failed".into(),
                Scope::File(_) => "No tests in this file".into(),
                _ => "No tests found".into(),
            });
        }
        for plugin in plugins {
            fx.push(Effect::PluginCommand {
                plugin,
                command: format!("run-tests:{what}"),
            });
        }
    }

    /// A file saved: with On Save, its tests run, or, in a file without
    /// any, all of its language plugin's.
    pub(crate) fn tests_saved(&mut self, doc: u64, fx: &mut Vec<Effect>) {
        if !self.tests.on_save {
            return;
        }
        let Some(i) = self.index_of(doc) else { return };
        let Some(path) = self.docs[i]
            .buffer
            .path
            .as_ref()
            .map(|p| p.display().to_string())
        else {
            return;
        };
        if self.tests.plugin_files.iter().any(|f| f.path == path) {
            return self.run_tests(Scope::File(path), false, fx);
        }
        let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
        let owner = self.languages.iter().find_map(|(plugin, l)| {
            l.extensions
                .iter()
                .any(|e| e.eq_ignore_ascii_case(ext))
                .then(|| plugin.clone())
        });
        if let Some(plugin) = owner.filter(|p| self.tests.roots.iter().any(|r| r.kind == *p)) {
            fx.push(Effect::PluginCommand {
                plugin,
                command: "run-tests:all".into(),
            });
        }
    }

    /// A plugin's test by its key, with its plugin.
    fn plugin_test(&self, key: &str) -> Option<(String, PluginTest)> {
        let (path, label) = key.rsplit_once("::")?;
        self.tests.plugin_files.iter().find_map(|f| {
            let test = f
                .tests
                .iter()
                .find(|t| t.label == label)
                .filter(|_| f.path == path)?;
            Some((f.plugin.clone(), test.clone()))
        })
    }

    /// Runs a plugin's test: its command line in the test terminal, whose
    /// output the plugin hears, or in the plugin.
    fn run_plugin_test(
        &mut self,
        plugin: &str,
        test: &PluginTest,
        debug: bool,
        fx: &mut Vec<Effect>,
    ) {
        match (&test.command, debug) {
            (Some(command), false) => {
                self.run_for_plugin(plugin.to_string(), test.id.clone(), RUN, command, fx);
            }
            _ => {
                let what = if debug { "debug-test" } else { "run-test" };
                fx.push(Effect::PluginCommand {
                    plugin: plugin.to_string(),
                    command: format!("{what}:{}", test.id),
                });
            }
        }
    }

    /// A plugin's tests in a file: shown in the file, and in the view under
    /// their group.
    pub(crate) fn set_plugin_tests(
        &mut self,
        plugin: String,
        path: &Path,
        group: String,
        label: String,
        tests: Vec<PluginTest>,
    ) {
        let path = path.display().to_string();
        let t = &mut self.tests;
        t.plugin_files
            .retain(|f| !(f.plugin == plugin && f.path == path));
        for root in t.roots.iter_mut().filter(|r| r.kind == plugin) {
            root.files.retain(|f| f.path != path);
        }
        if !tests.is_empty() {
            let file = TestFile {
                label: if label.is_empty() {
                    path.clone()
                } else {
                    label
                },
                path: path.clone(),
                tests: tests
                    .iter()
                    .map(|t| TestCase {
                        name: t.label.clone(),
                        line: t.line as usize,
                    })
                    .collect(),
            };
            match t
                .roots
                .iter_mut()
                .find(|r| r.kind == plugin && r.group == group)
            {
                Some(root) => {
                    root.files.push(file);
                    root.files.sort_by(|a, b| a.label.cmp(&b.label));
                }
                None => {
                    let name = if group.is_empty() {
                        plugin.clone()
                    } else {
                        group.clone()
                    };
                    // Groups start folded but the first, as the view has
                    // room for.
                    if !t.roots.is_empty() {
                        t.folded.insert(format!("r:{name}"));
                    }
                    t.roots.push(TestRoot {
                        name,
                        kind: plugin.clone(),
                        group: group.clone(),
                        files: vec![file],
                        ..TestRoot::default()
                    });
                    t.roots
                        .sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
                }
            }
            t.plugin_files.push(PluginFile {
                plugin,
                path,
                group,
                tests,
            });
        }
        t.roots.retain(|r| !r.files.is_empty());
        t.version += 1;
        self.build_views();
    }

    /// A plugin's group of tests started or ended: all its groups for the
    /// empty one.
    pub(crate) fn set_test_run(
        &mut self,
        plugin: &str,
        group: &str,
        running: bool,
        seconds: Option<f64>,
    ) {
        for root in self
            .tests
            .roots
            .iter_mut()
            .filter(|r| r.kind == plugin && (group.is_empty() || r.group == group))
        {
            root.running = running;
            if seconds.is_some() {
                root.time = seconds;
            }
        }
        self.tests.version += 1;
        self.build_views();
    }

    /// How a plugin's tests went.
    pub(crate) fn set_plugin_results(&mut self, plugin: &str, results: Vec<PluginTestResult>) {
        let now = self.now;
        let t = &mut self.tests;
        for r in results {
            let Some((path, label)) = t.plugin_files.iter().find_map(|f| {
                let test = f
                    .tests
                    .iter()
                    .find(|x| x.id == r.id)
                    .filter(|_| f.plugin == plugin)?;
                Some((f.path.clone(), test.label.clone()))
            }) else {
                continue;
            };
            let running = r.outcome.is_none();
            for root in t.roots.iter_mut().filter(|x| x.kind == plugin) {
                root.running = running;
            }
            let Some(outcome) = r.outcome else { continue };
            t.results.insert(
                test_key(&path, &label),
                TestResult {
                    outcome,
                    message: r.message,
                    at: r.line.map(|l| (path.clone(), l as usize)),
                    when: now,
                },
            );
        }
        t.version += 1;
        self.build_views();
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
        if self.tests.roots.is_empty() {
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
            nodes.push(Node::entry(Entry {
                label: root.name.clone(),
                icon: Some(mark(outcome, root.running)),
                // Each plugin is named in its chip once there are several.
                chip: several.then(|| root.kind.clone()),
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
                nodes.push(Node::entry(Entry {
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
                    nodes.push(Node::entry(Entry {
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
            nodes.push(Node::entry(Entry {
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
                nodes.push(Node::entry(Entry {
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
