//! Tasks, as VS Code's Run Task has them: the commands the project's
//! `.merlin/tasks.toml` offers, and those plugins add, such as cargo's
//! and npm's. Each runs in a terminal tab of its own, named after it.

use std::path::PathBuf;

use super::{Editor, Effect, Focus, Palette, Source};

/// The project's own tasks, for Configure Tasks to open.
pub(crate) const TASKS_FILE: &str = ".merlin/tasks.toml";

/// What a task is for, for the group Run Task lists it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum TaskGroup {
    Test,
    Build,
    #[default]
    Other,
}

impl TaskGroup {
    fn heading(self) -> &'static str {
        match self {
            TaskGroup::Test => "Test",
            TaskGroup::Build => "Build and run",
            TaskGroup::Other => "Other",
        }
    }

    pub(crate) fn parse(name: &str) -> Self {
        match name {
            "test" => TaskGroup::Test,
            "build" | "run" => TaskGroup::Build,
            _ => TaskGroup::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Task {
    pub name: String,
    /// The command line the shell runs; none for one its plugin runs
    /// itself.
    pub command: Option<String>,
    pub group: TaskGroup,
    /// Where it's from: a file, or what its plugin says, such as `bin`.
    pub source: String,
    /// The plugin that adds it.
    pub plugin: Option<String>,
}

/// A task started, and the terminal it runs in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub name: String,
    pub term: u64,
    pub command: String,
    /// Its exit code once it's done.
    pub exited: Option<i32>,
    /// The plugin that ran it, and its id for the run, to hear what it
    /// printed.
    pub plugin: Option<(String, String)>,
    /// All it printed, kept for its plugin until it exits.
    pub output: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Tasks {
    /// What the project's files offer, as last read.
    pub found: Vec<Task>,
    /// Each plugin's, by plugin and the set they came in.
    pub plugins: Vec<((String, String), Vec<Task>)>,
    /// What to do once the files are read: list them, or run the first in
    /// a group.
    pub asking: Option<Ask>,
    /// The tasks started, oldest first.
    pub runs: Vec<Run>,
    /// Configure Tasks is opening the file, to fill in if it's new.
    pub example: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ask {
    List,
    First(TaskGroup),
}

/// A `"…"` TOML string's text, or a bare word.
fn unquote(value: &str) -> String {
    let v = value.trim();
    let inner = v
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')));
    match inner {
        Some(s) => s.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => v.to_string(),
    }
}

/// `.merlin/tasks.toml`'s tasks: `name = "command"` under `[tasks]`, or a
/// `[tasks.name]` table with `command` and, if it's for tests or
/// building, `group`.
pub fn from_tasks_file(text: &str) -> Vec<Task> {
    let mut tasks: Vec<Task> = Vec::new();
    let mut table: Option<Option<String>> = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(head) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            table = match head.trim() {
                "tasks" => Some(None),
                h => h.strip_prefix("tasks.").map(|n| Some(unquote(n))),
            };
            if let Some(Some(name)) = &table {
                tasks.push(Task {
                    name: name.clone(),
                    source: TASKS_FILE.into(),
                    ..Task::default()
                });
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (unquote(key), unquote(value));
        match &table {
            Some(None) => tasks.push(Task {
                name: key,
                command: Some(value),
                group: TaskGroup::Other,
                source: TASKS_FILE.into(),
                plugin: None,
            }),
            Some(Some(_)) => {
                if let Some(task) = tasks.last_mut() {
                    match key.as_str() {
                        "command" => task.command = Some(value),
                        "group" => task.group = TaskGroup::parse(&value),
                        _ => {}
                    }
                }
            }
            None => {}
        }
    }
    tasks.retain(|t| t.command.is_some());
    tasks
}

impl Editor {
    /// Every task, grouped: tests, then building and running, then the
    /// rest, the project's before plugins' in each.
    pub(crate) fn all_tasks(&self) -> Vec<Task> {
        let t = &self.tasks;
        let mut all: Vec<Task> = t
            .found
            .iter()
            .chain(t.plugins.iter().flat_map(|(_, tasks)| tasks))
            .cloned()
            .collect();
        all.sort_by_key(|task| task.group);
        all
    }

    /// The task Run Test Task or Run Build Task runs.
    pub(crate) fn first_task(&self, group: TaskGroup) -> Option<Task> {
        self.all_tasks().into_iter().find(|t| t.group == group)
    }

    /// The `task.` commands.
    pub(crate) fn task_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let ask = match id {
            "task.run" => Ask::List,
            "task.test" => Ask::First(TaskGroup::Test),
            "task.build" => Ask::First(TaskGroup::Build),
            "task.configure" => return self.configure_tasks(fx),
            "task.show" => {
                if let Some(run) = self.running_tasks().last() {
                    let term = run.term;
                    self.focus_terminal(term);
                }
                return;
            }
            "task.restart" => {
                if let Some(run) = self.tasks.runs.last().cloned() {
                    self.run_task_named(&run.name, fx);
                }
                return;
            }
            _ => return,
        };
        // The files may have changed since: plugins read theirs again, as
        // the project's own is.
        for plugin in &self.plugins {
            fx.push(Effect::PluginCommand {
                plugin: plugin.name.clone(),
                command: "find-tasks".into(),
            });
        }
        self.tasks.asking = Some(ask);
        fx.push(Effect::ReadFiles(vec![PathBuf::from(TASKS_FILE)]));
    }

    /// The task files, read: lists or runs what was asked for.
    pub(crate) fn tasks_read(&mut self, files: &[(PathBuf, Option<String>)], fx: &mut Vec<Effect>) {
        let Some(ask) = self.tasks.asking else {
            return;
        };
        let Some((_, text)) = files.iter().find(|(p, _)| p == &PathBuf::from(TASKS_FILE)) else {
            return;
        };
        self.tasks.asking = None;
        self.tasks.found = text.as_deref().map(from_tasks_file).unwrap_or_default();
        match ask {
            Ask::List => self.pick_task(),
            Ask::First(group) => match self.first_task(group) {
                Some(task) => self.start_task(&task, fx),
                None => self.pick_task(),
            },
        }
    }

    /// Run Task's list, with Configure Tasks at its end.
    fn pick_task(&mut self) {
        let mut items: Vec<(String, String)> = self
            .all_tasks()
            .into_iter()
            .map(|t| (task_id(&t), t.name))
            .collect();
        items.push(("configure".into(), "Configure tasks…".into()));
        self.palette = Some(Palette::new(
            Source::Tasks,
            Some("Run Task ›".into()),
            items,
        ));
        self.focus = Focus::Palette;
    }

    /// A task chosen in Run Task.
    pub(crate) fn task_picked(&mut self, id: &str, fx: &mut Vec<Effect>) {
        if id == "configure" {
            return self.configure_tasks(fx);
        }
        if let Some(task) = self.all_tasks().into_iter().find(|t| task_id(t) == id) {
            self.start_task(&task, fx);
        }
    }

    /// The task with this name, as a plugin or Restart asks.
    pub(crate) fn run_task_named(&mut self, name: &str, fx: &mut Vec<Effect>) {
        match self.all_tasks().into_iter().find(|t| t.name == name) {
            Some(task) => self.start_task(&task, fx),
            None => self.notify(format!("No task named {name}")),
        }
    }

    /// Runs a task in a terminal named after it, in place of its last run;
    /// a plugin's own runs in the plugin.
    pub(crate) fn start_task(&mut self, task: &Task, fx: &mut Vec<Effect>) {
        let Some(command) = &task.command else {
            if let Some(plugin) = &task.plugin {
                fx.push(Effect::PluginCommand {
                    plugin: plugin.clone(),
                    command: format!("task:{}", task.name),
                });
            }
            return;
        };
        self.start_command(&task.name, command, fx);
    }

    /// Runs `command` in a terminal tab named `name`, closing the one an
    /// earlier run of it had.
    pub(crate) fn start_command(&mut self, name: &str, command: &str, fx: &mut Vec<Effect>) -> u64 {
        if let Some(at) = self.tasks.runs.iter().position(|r| r.name == name) {
            let old = self.tasks.runs.remove(at);
            self.kill_terminal(old.term, fx);
        }
        let term = self.new_task_terminal(name, command, fx);
        self.tasks.runs.push(Run {
            name: name.to_string(),
            term,
            command: command.to_string(),
            exited: None,
            plugin: None,
            output: None,
        });
        term
    }

    /// Runs `command` in a terminal tab named `name` for a plugin, which
    /// hears all it printed, by `id`, once it ends.
    pub(crate) fn run_for_plugin(
        &mut self,
        plugin: String,
        id: String,
        name: &str,
        command: &str,
        fx: &mut Vec<Effect>,
    ) -> u64 {
        let term = self.start_command(name, command, fx);
        if let Some(run) = self.tasks.runs.last_mut() {
            run.plugin = Some((plugin, id));
        }
        term
    }

    /// A task's terminal ended: its plugin hears how, if a plugin ran it.
    pub(crate) fn task_exited(&mut self, term: u64, code: Option<i32>, fx: &mut Vec<Effect>) {
        let Some(run) = self.tasks.runs.iter_mut().find(|r| r.term == term) else {
            return;
        };
        run.exited = Some(code.unwrap_or(-1));
        if let Some((plugin, id)) = run.plugin.clone() {
            fx.push(Effect::PluginTerminal {
                plugin,
                id,
                output: run.output.take().unwrap_or_default(),
                code,
            });
        }
        self.build_views();
    }

    /// What a task printed, once it ends: kept for its plugin until it
    /// exits.
    pub(crate) fn task_output(&mut self, term: u64, text: &str) {
        let Some(run) = self.tasks.runs.iter_mut().rev().find(|r| r.term == term) else {
            return;
        };
        if run.plugin.is_some() {
            run.output = Some(text.to_string());
        }
        let name = run.name.clone();
        self.task_problems(&name, text);
    }

    /// The tasks still running, for the status bar.
    pub(crate) fn running_tasks(&self) -> Vec<&Run> {
        self.tasks
            .runs
            .iter()
            .filter(|r| r.exited.is_none())
            .collect()
    }

    /// Opens `.merlin/tasks.toml`, with an example in it when it's new.
    fn configure_tasks(&mut self, fx: &mut Vec<Effect>) {
        self.tasks.example = true;
        self.open(PathBuf::from(TASKS_FILE), fx);
    }

    /// A file opened: `.merlin/tasks.toml` new for Configure Tasks gets
    /// the example.
    pub(crate) fn tasks_opened(&mut self, path: &std::path::Path) {
        if !std::mem::take(&mut self.tasks.example) || path != std::path::Path::new(TASKS_FILE) {
            return;
        }
        if self.doc().buffer.path.as_deref() == Some(path) && self.doc().buffer.text().is_empty() {
            self.type_text(EXAMPLE, crate::history::Kind::Other);
        }
    }

    /// Run Task's detail for an item: where it's from, and its plugin.
    pub(crate) fn task_detail(&self, id: &str) -> Option<(String, Option<String>, TaskGroup)> {
        let task = self.all_tasks().into_iter().find(|t| task_id(t) == id)?;
        Some((task.source, task.plugin, task.group))
    }

    /// Run Task's group headings, for the window: the heading before each
    /// item, when the query's empty and there's more than one group.
    pub(crate) fn task_headings(&self, ids: &[String]) -> Vec<Option<String>> {
        let groups: Vec<Option<TaskGroup>> = ids
            .iter()
            .map(|id| self.task_detail(id).map(|(_, _, g)| g))
            .collect();
        let distinct = groups
            .iter()
            .flatten()
            .collect::<std::collections::BTreeSet<_>>();
        if distinct.len() < 2 {
            return vec![None; ids.len()];
        }
        let mut last = None;
        groups
            .into_iter()
            .map(|g| {
                let g = g?;
                (last.replace(g) != Some(g)).then(|| g.heading().to_string())
            })
            .collect()
    }
}

/// A task's id in Run Task's list: its plugin and name.
pub(crate) fn task_id(task: &Task) -> String {
    format!("{}:{}", task.plugin.as_deref().unwrap_or(""), task.name)
}

const EXAMPLE: &str = "\
# Tasks for Run Task. Each runs in a terminal tab of its own.
[tasks]
# lint = \"make lint\"

# A table can say what the task is for: test, build or other.
# [tasks.watch]
# command = \"make watch\"
# group = \"build\"
";
