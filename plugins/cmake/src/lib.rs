//! CMake, as a plugin: colours for `CMakeLists.txt` and `.cmake` files,
//! Configure, Build and Test as tasks, from the project's presets when it
//! has them, and the build's `compile_commands.json` shared with the C/C++
//! plugin, so clangd finds it wherever the build is. neocmakelsp, when
//! it's installed, completes, explains and finds what's in them.

pub mod addfile;
pub mod cache;
pub mod ctest;
pub mod fileapi;
pub mod lex;
pub mod presets;
pub mod projects;

use std::collections::BTreeMap;

use merlin_lsp::{Spec, WithLsp};
use merlin_plugin::prelude::*;
use merlin_plugin::shared::{self, BuildSetup, CompileCommands};
use merlin_plugin::text::utf16_column;
use merlin_plugin::{Button, Chip, Row, StatusChip, Tone};

use ctest::CTest;
use fileapi::Target;
use lex::Kind;
use presets::{CommandTask, ConfigureTask, Options, Presets};
use projects::{Project, holding, join};

const MAX_BYTES: usize = 512 * 1024;

/// neocmakelsp: completion, hover, go to definition, formatting and
/// symbols. It completes only for editors that take snippets.
const CMAKE_SERVER: Spec = Spec {
    program: "neocmakelsp",
    args: &["stdio"],
    files: &[("CMakeLists.txt", "cmake"), ("cmake", "cmake")],
    settings: None,
    options: &[],
    flags: &[],
    restarts: &[],
    shared: &[],
    adjust: None,
    scan: None,
    snippets: true,
    case_blind: true,
    // The manifest's [programs] says how to install it, per system.
    install: None,
};

/// What a task without a command line does when Run Task picks it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    /// Configures with this command line, into this build folder, for
    /// people to read as `from`.
    Configure {
        command: String,
        dir: String,
        from: String,
        /// The project's folder.
        project: String,
    },
}

/// A project's build while another is in use.
#[derive(Default)]
struct Saved {
    build: Option<(String, String)>,
    targets: Vec<Target>,
    state: State,
    target: Option<String>,
    configured_at: Option<std::time::Instant>,
    pending: BTreeMap<String, String>,
}

#[derive(Default)]
struct Cmake {
    /// Run Task's tasks that run in the plugin, by name.
    actions: BTreeMap<String, Action>,
    /// Configures running, by terminal id: the folder, what made it, and
    /// the project's folder.
    running: BTreeMap<String, (String, String, String)>,
    /// The CMake projects in the workspace, and the folder of the one in
    /// use, empty at the top. The fields below are its.
    projects: Vec<Project>,
    project: String,
    /// Picked by hand, so the project doesn't follow the open file.
    pinned: bool,
    /// The other projects' builds.
    saved: BTreeMap<String, Saved>,
    last: Option<(String, Option<u64>)>,
    /// It has read the project once it had a workspace: a plugin starts
    /// before it has one.
    started: bool,
    /// The build in use: its folder and what made it, once configured.
    build: Option<(String, String)>,
    /// Its targets, from CMake's file API.
    targets: Vec<Target>,
    /// CTest's tests, and the files the Tests view has them in.
    tests: Vec<CTest>,
    test_files: Vec<String>,
    /// `ctest --show-only` running, and what it's printed so far.
    listing: Option<(u32, String)>,
    /// The tests a run in the terminal is for.
    test_run: Vec<String>,
    /// What the status bar says of the build.
    state: State,
    /// The target the status bar's Build, Run and Debug are for.
    target: Option<String>,
    /// The CMake view: targets and folders opened, or folded.
    opened: std::collections::BTreeSet<String>,
    folded: std::collections::BTreeSet<String>,
    /// The cache's filter, and whether advanced variables show.
    filter: String,
    advanced: bool,
    /// Cache changes waiting for Configure, and the variable being edited.
    pending: BTreeMap<String, String>,
    editing: Option<String>,
    /// When the build in use was configured, if this run did it.
    configured_at: Option<std::time::Instant>,
    /// How the last test run went: passed and failed.
    test_counts: (usize, usize),
    /// New sources it's asked about, so it asks once.
    asked: std::collections::BTreeSet<String>,
    /// The new source being asked about, and the target picked for it.
    adding: Option<(String, String)>,
}

/// How the build is, for the status bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    NotConfigured,
    /// Configuring, as `from` names it.
    Configuring(String),
    Failed(String),
    Ready,
}

/// `CMake · Debug` as `Debug`.
fn short(from: &str) -> &str {
    from.strip_prefix("CMake · ").unwrap_or(from)
}

/// How to configure, from the plugin's settings.
fn options() -> Options {
    let defaults = Options::default();
    let text = |key: &str, default: String| {
        settings::text(key)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or(default)
    };
    let cache = match host::get_setting("cache-variables") {
        Some(merlin_plugin::Setting::Other(list)) => {
            serde_json::from_str::<Vec<String>>(&list).unwrap_or_default()
        }
        _ => Vec::new(),
    };
    Options {
        folder: text("build-folder", defaults.folder),
        // An empty choice means CMake's own.
        build_type: settings::text("build-type").unwrap_or(defaults.build_type),
        generator: settings::text("generator").unwrap_or_default(),
        cache,
    }
}

/// A command line run in the project's folder.
fn in_folder(project: &str, command: &str) -> String {
    if project.is_empty() {
        command.to_string()
    } else {
        format!("cd {} && {command}", quote(project))
    }
}

/// A project's presets, read from its folder.
fn presets_of(project: &str) -> Presets {
    Presets::read(
        read(&join(project, "CMakePresets.json")),
        read(&join(project, "CMakeUserPresets.json")),
    )
}

/// A project's configures, with build folders relative to the workspace
/// and command lines run in its folder.
fn configures_of(project: &str, presets: &Presets, options: &Options) -> Vec<ConfigureTask> {
    presets
        .configure_tasks(options)
        .into_iter()
        .map(|t| ConfigureTask {
            command: in_folder(project, &t.command),
            dir: join(project, &t.dir),
            ..t
        })
        .collect()
}

/// A project's build that's configured already, and what made it.
fn detect(project: &str, options: &Options) -> Option<(String, String)> {
    presets_of(project)
        .configured(options, |p| read(&join(project, p)))
        .map(|(dir, from)| (join(project, &dir), from))
}

/// The targets in a project's build.
fn targets_of(dir: &str, project: &str) -> Vec<Target> {
    fileapi::targets(dir, project, |d| host::list_folder(d).ok(), read).unwrap_or_default()
}

/// A configure's arguments for CMake's debugger, which starts in the
/// workspace: the source and build folders in the project's.
fn debugger_args(project: &str, args: &[String]) -> Vec<String> {
    if project.is_empty() {
        return args.to_vec();
    }
    let mut out = Vec::new();
    let mut source = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        out.push(a.clone());
        if a == "-S" || a == "-B" {
            source |= a == "-S";
            if let Some(v) = it.next() {
                out.push(join(project, v));
            }
        }
    }
    if !source {
        out.splice(0..0, ["-S".to_string(), project.to_string()]);
    }
    out
}

/// A path relative to the workspace.
fn relative(path: &str) -> String {
    let root = host::workspace_root();
    path.strip_prefix(&format!("{}/", root.trim_end_matches('/')))
        .unwrap_or(path)
        .to_string()
}

impl Cmake {
    /// Reads the projects, the builds that are configured already and
    /// their targets, and offers their tasks: once, when it first has a
    /// workspace to read.
    fn start(&mut self) {
        if self.started || host::workspace_root().is_empty() {
            return;
        }
        self.started = true;
        self.find_projects();
        let options = options();
        for p in self.projects.clone() {
            if p.dir == self.project {
                continue;
            }
            if let Some((dir, from)) = detect(&p.dir, &options) {
                let targets = targets_of(&dir, &p.dir);
                let target = targets
                    .iter()
                    .find(|t| t.is_program())
                    .or(targets.first())
                    .map(|t| t.name.clone());
                self.saved.insert(
                    p.dir.clone(),
                    Saved {
                        build: Some((dir, from)),
                        targets,
                        state: State::Ready,
                        target,
                        ..Saved::default()
                    },
                );
            }
        }
        if let Some((dir, from)) = detect(&self.project, &options) {
            self.configured(dir, from);
        }
        self.find_tasks();
        self.chips();
    }

    /// Finds the workspace's projects, and the one in use: the open
    /// file's, else the one it was, else the first.
    fn find_projects(&mut self) {
        let files = host::list_files().unwrap_or_default();
        let mut found = projects::find(&files, read);
        if found.is_empty() {
            found.push(Project {
                dir: String::new(),
                name: "CMake".into(),
            });
        }
        self.projects = found;
        let open = host::active_path()
            .ok()
            .flatten()
            .and_then(|p| holding(&self.projects, &relative(&p)).map(|p| p.dir.clone()));
        if !self.projects.iter().any(|p| p.dir == self.project) || (!self.pinned && open.is_some())
        {
            self.project = open.unwrap_or_else(|| self.projects[0].dir.clone());
        }
    }

    fn several(&self) -> bool {
        self.projects.len() > 1
    }

    fn name_of(&self, project: &str) -> String {
        self.projects
            .iter()
            .find(|p| p.dir == project)
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }

    /// A task's name, `CMake: Build app`, as `CMake tools: Build app` when
    /// there are several projects.
    fn renamed(&self, project: &str, name: &str) -> String {
        if self.several() {
            name.replacen("CMake:", &format!("CMake {}:", self.name_of(project)), 1)
        } else {
            name.to_string()
        }
    }

    /// Makes another project the one in use: this one's build kept aside,
    /// that one's back, and clangd told.
    fn switch(&mut self, project: &str) {
        if project == self.project || !self.projects.iter().any(|p| p.dir == project) {
            return;
        }
        let old = Saved {
            build: self.build.take(),
            targets: std::mem::take(&mut self.targets),
            state: std::mem::take(&mut self.state),
            target: self.target.take(),
            configured_at: self.configured_at.take(),
            pending: std::mem::take(&mut self.pending),
        };
        let was = std::mem::replace(&mut self.project, project.to_string());
        self.saved.insert(was, old);
        self.editing = None;
        self.opened.clear();
        self.test_counts = (0, 0);
        self.show_tests(Vec::new());
        let new = self.saved.remove(project).unwrap_or_default();
        self.target = new.target;
        self.pending = new.pending;
        match new.build {
            Some((dir, from)) if new.state == State::Ready => {
                self.configured(dir, from);
                self.configured_at = new.configured_at;
            }
            Some((dir, from)) => {
                self.share(&dir, &from);
                self.targets = new.targets;
                self.build = Some((dir, from));
                self.state = new.state;
            }
            None => {
                self.state = new.state;
                let _ = shared::withdraw::<CompileCommands>();
            }
        }
        self.find_tasks();
        self.chips();
    }

    /// The project picker: each project with how its build is, and
    /// whether the project follows the open file.
    fn pick_project(&self) {
        let mut items: Vec<PickItem> = self
            .projects
            .iter()
            .map(|p| {
                let (build, state) = if p.dir == self.project {
                    (self.build.as_ref(), &self.state)
                } else {
                    let saved = self.saved.get(&p.dir);
                    (
                        saved.and_then(|s| s.build.as_ref()),
                        saved.map_or(&State::NotConfigured, |s| &s.state),
                    )
                };
                let how = match (state, build) {
                    (State::Failed(_), _) => "failed".to_string(),
                    (State::Configuring(_), _) => "configuring".into(),
                    (_, Some((_, from))) => short(from).to_string(),
                    _ => "not configured".into(),
                };
                let folder = if p.dir.is_empty() {
                    "the top".to_string()
                } else {
                    format!("{}/", p.dir)
                };
                PickItem {
                    id: format!("project:{}", p.dir),
                    label: p.name.clone(),
                    detail: format!("{folder} · {how}"),
                    group: String::new(),
                    current: p.dir == self.project,
                }
            })
            .collect();
        items.push(PickItem {
            id: "follow".into(),
            label: if self.pinned {
                "Follow the open file".into()
            } else {
                "✓ Follow the open file".into()
            },
            detail: format!("it's {} now", self.name_of(&self.project)),
            group: "Following".into(),
            current: false,
        });
        host::show_picker("cmake-project", "Pick the CMake project", &items);
    }

    /// The project of the open file, when following it.
    fn follow(&mut self) {
        if self.pinned || !self.several() {
            return;
        }
        let Some(path) = host::active_path().ok().flatten() else {
            return;
        };
        if let Some(p) = holding(&self.projects, &relative(&path)).map(|p| p.dir.clone()) {
            self.switch(&p);
        }
    }

    /// A build that's configured: shared with C/C++, and its targets read.
    fn configured(&mut self, dir: String, from: String) {
        self.share(&dir, &from);
        self.targets = targets_of(&dir, &self.project);
        self.build = Some((dir, from));
        self.state = State::Ready;
        // The target stays if the build still has it, else its first
        // program.
        if !self
            .target
            .as_ref()
            .is_some_and(|t| t == "all" || self.targets.iter().any(|x| &x.name == t))
        {
            self.target = self
                .targets
                .iter()
                .find(|t| t.is_program())
                .or(self.targets.first())
                .map(|t| t.name.clone());
        }
        self.find_tests();
        self.chips();
    }

    /// The status bar's items: the preset, or how the build is, and the
    /// target with Build, Run and Debug. And, while it isn't configured,
    /// how to set it up, for the C/C++ plugin to offer.
    fn chips(&self) {
        if self.state == State::NotConfigured {
            let _ = shared::provide(&BuildSetup {
                label: "Configure with CMake".into(),
                command: "cmake.configure".into(),
            });
        } else {
            let _ = shared::withdraw::<BuildSetup>();
        }
        let label = self.build.as_ref().map(|(_, from)| short(from).to_string());
        let mut chips = Vec::new();
        if self.several() {
            chips.push(StatusChip {
                id: "project".into(),
                text: format!("▣ {} ▾", self.name_of(&self.project)),
                tone: Tone::Normal,
                busy: false,
            });
        }
        chips.push(match &self.state {
            State::Configuring(from) => StatusChip {
                id: "preset".into(),
                text: format!("CMake: Configuring {}…", short(from)),
                tone: Tone::Normal,
                busy: true,
            },
            State::Failed(from) => StatusChip {
                id: "preset".into(),
                text: format!("CMake: {} failed ▾", short(from)),
                tone: Tone::Error,
                busy: false,
            },
            State::NotConfigured => StatusChip {
                id: "preset".into(),
                text: "CMake: Configure ▾".into(),
                tone: Tone::Normal,
                busy: false,
            },
            State::Ready => StatusChip {
                id: "preset".into(),
                text: format!("⚙ CMake: {} ▾", label.unwrap_or_default()),
                tone: Tone::Normal,
                busy: false,
            },
        });
        if self.state == State::Ready
            && let Some(target) = &self.target
        {
            chips.push(StatusChip {
                id: "target".into(),
                text: format!("◎ {target} ▾"),
                tone: Tone::Normal,
                busy: false,
            });
            chips.push(StatusChip {
                id: "build".into(),
                text: "⚒ Build".into(),
                tone: Tone::Normal,
                busy: false,
            });
            if self.program(target).is_some() {
                for (id, text) in [("run", "▶ Run"), ("debug", "⏵ Debug")] {
                    chips.push(StatusChip {
                        id: id.into(),
                        text: text.into(),
                        tone: Tone::Normal,
                        busy: false,
                    });
                }
            }
        }
        host::set_status_chips(&chips);
    }

    /// A target's program, if it's one.
    fn program(&self, name: &str) -> Option<&Target> {
        self.targets
            .iter()
            .find(|t| t.name == name && t.is_program() && t.artifact.is_some())
    }

    /// Picks the preset: each configure, then each from scratch.
    fn pick_preset(&self) {
        let prefix = self.renamed(&self.project, "CMake: Configure");
        let items: Vec<PickItem> = self
            .actions
            .iter()
            .filter(|(_, Action::Configure { project, .. })| *project == self.project)
            .map(|(name, action)| {
                let Action::Configure { dir, .. } = action;
                let label = name
                    .strip_prefix(&format!("{prefix} "))
                    .unwrap_or(name);
                let label = if *name == prefix { "Configure" } else { label };
                let scratch = name.ends_with("from Scratch");
                PickItem {
                    id: name.clone(),
                    label: if scratch {
                        format!("Configure {label}")
                    } else {
                        label.to_string()
                    },
                    detail: if scratch { String::new() } else { dir.clone() },
                    group: if scratch {
                        "From scratch".into()
                    } else {
                        String::new()
                    },
                    current: !scratch
                        && self.build.as_ref().is_some_and(|(d, _)| d == dir),
                }
            })
            .collect();
        // The presets first, then from scratch.
        let (mut items, fresh): (Vec<PickItem>, Vec<PickItem>) =
            items.into_iter().partition(|i| i.group.is_empty());
        items.extend(fresh);
        host::show_picker("cmake-preset", "Pick the preset", &items);
    }

    /// Picks the target: programs, then libraries, then all of them.
    fn pick_target(&self) {
        let mut items: Vec<PickItem> = Vec::new();
        for (group, programs) in [("Programs", true), ("Libraries", false)] {
            for t in self.targets.iter().filter(|t| t.is_program() == programs) {
                items.push(PickItem {
                    id: t.name.clone(),
                    label: t.name.clone(),
                    detail: if programs {
                        t.artifact.clone().unwrap_or_default()
                    } else {
                        t.kind.to_ascii_lowercase().replace('_', " ")
                    },
                    group: group.into(),
                    current: self.target.as_ref() == Some(&t.name),
                });
            }
        }
        items.push(PickItem {
            id: "all".into(),
            label: "all".into(),
            detail: "every target".into(),
            group: "Everything".into(),
            current: self.target.as_deref() == Some("all"),
        });
        host::show_picker("cmake-target", "Pick the target", &items);
    }

    /// Builds the target, then runs or debugs its program.
    fn build_target(&self, then: &str) {
        let (Some((dir, _)), Some(target)) = (&self.build, &self.target) else {
            return;
        };
        let build = if target == "all" {
            format!("cmake --build {}", quote(dir))
        } else {
            format!("cmake --build {} --target {}", quote(dir), quote(target))
        };
        let program = self.program(target);
        match (then, program) {
            ("run", Some(t)) => {
                let path = t.artifact.clone().unwrap_or_default();
                let command = format!("{build} && {}", quote(&run_path(&path)));
                let _ = host::run_in_terminal("cmake-run", &format!("Run {target}"), &command);
            }
            ("debug", Some(t)) => host::start_debugging(
                &DebugTarget {
                    name: t.name.clone(),
                    debugger: String::new(),
                    language: "cpp".into(),
                    program: t.artifact.clone().unwrap_or_default(),
                    args: Vec::new(),
                    build: self.renamed(&self.project, &format!("CMake: Build {}", t.name)),
                    detail: "CMake".into(),
                    launch: String::new(),
                },
                None,
            ),
            _ => {
                let _ = host::run_in_terminal("cmake-build", "CMake: Build", &build);
            }
        }
    }

    /// Asks CTest for the build's tests, without a terminal: they come to
    /// `process`.
    fn find_tests(&mut self) {
        let Some((dir, _)) = &self.build else {
            return;
        };
        let args = [
            "--test-dir".to_string(),
            dir.clone(),
            "--show-only=json-v1".into(),
        ];
        match host::spawn("ctest", &args, &[], Framing::Lines) {
            Ok(process) => self.listing = Some((process, String::new())),
            Err(err) => host::log(LogLevel::Info, &format!("No CTest tests: {err}")),
        }
    }

    /// Shows the tests in the Tests view, by the file each is added in,
    /// and takes them out of files that have none now.
    fn show_tests(&mut self, tests: Vec<CTest>) {
        let mut files: Vec<String> = tests.iter().map(|t| t.file.clone()).collect();
        files.sort();
        files.dedup();
        for gone in self.test_files.iter().filter(|f| !files.contains(f)) {
            host::set_tests(gone, "ctest", gone, &[]);
        }
        for file in &files {
            let items: Vec<TestItem> = tests
                .iter()
                .filter(|t| &t.file == file)
                .map(|t| TestItem {
                    id: t.name.clone(),
                    label: t.name.clone(),
                    line: t.line,
                    command: None,
                })
                .collect();
            host::set_tests(file, "ctest", file, &items);
        }
        self.test_files = files;
        self.tests = tests;
    }

    /// Builds, then runs these tests, or all of them, in parallel, in a
    /// terminal; how each went comes from what CTest prints.
    fn run_tests(&mut self, names: Option<Vec<String>>, failed: bool) {
        let Some((dir, _)) = self.build.clone() else {
            return host::notice(LogLevel::Info, "Configure the project first", &[]);
        };
        let dir = quote(&dir);
        let mut command =
            format!("cmake --build {dir} && ctest --test-dir {dir} --output-on-failure -j");
        if failed {
            command.push_str(" --rerun-failed");
        }
        let names = names.unwrap_or_else(|| self.tests.iter().map(|t| t.name.clone()).collect());
        if names.len() < self.tests.len() && !failed {
            let pattern: Vec<String> = names.iter().map(|n| ctest::exactly(n)).collect();
            command.push_str(&format!(" -R {}", quote(&pattern.join("|"))));
        }
        let running: Vec<TestResult> = names
            .iter()
            .map(|n| TestResult {
                id: n.clone(),
                outcome: TestOutcome::Running,
                message: String::new(),
                line: None,
            })
            .collect();
        host::set_test_results(&running);
        host::set_test_run("ctest", true, None);
        match host::run_in_terminal("ctest-run", "CTest", &command) {
            Ok(()) => self.test_run = names,
            Err(err) => {
                host::set_test_run("ctest", false, None);
                host::notice(LogLevel::Error, &err, &[]);
            }
        }
    }

    /// Debugs one test's own command line with the C/C++ plugin's
    /// debugger.
    fn debug_test(&self, name: &str) {
        let Some(test) = self.tests.iter().find(|t| t.name == name) else {
            return;
        };
        let Some((program, args)) = test.command.split_first() else {
            return;
        };
        let launch = test
            .folder
            .as_ref()
            .map(|f| serde_json::json!({ "cwd": f }).to_string())
            .unwrap_or_default();
        host::start_debugging(
            &DebugTarget {
                name: test.name.clone(),
                debugger: String::new(),
                language: "cpp".into(),
                program: program.clone(),
                args: args.to_vec(),
                build: String::new(),
                detail: "CTest".into(),
                launch,
            },
            None,
        );
    }

    /// Every project's tasks for Run Task, and its programs for Run and
    /// Debug.
    fn find_tasks(&mut self) {
        let options = options();
        let mut tasks = Vec::new();
        let mut debug = Vec::new();
        self.actions.clear();
        for p in self.projects.clone() {
            let presets = presets_of(&p.dir);
            let several = self.several();
            let detail = |d: &str| {
                if several {
                    format!("{} · {d}", p.name)
                } else {
                    d.to_string()
                }
            };
            for configure in configures_of(&p.dir, &presets, &options) {
                let name = self.renamed(&p.dir, &configure.name);
                // CMake's own debugger, stopping at breakpoints in the
                // project's CMake files as it configures.
                let bare = presets
                    .configure_tasks(&options)
                    .into_iter()
                    .find(|t| t.name == configure.name)
                    .map(|t| t.command)
                    .unwrap_or_default();
                if let Some((_, args)) = presets::words(&bare).split_first() {
                    debug.push(DebugTarget {
                        name: format!("{name} in CMake's Debugger"),
                        debugger: "cmake".into(),
                        language: "cmake".into(),
                        program: "cmake".into(),
                        args: debugger_args(&p.dir, args),
                        build: String::new(),
                        detail: detail(&configure.detail),
                        launch: String::new(),
                    });
                }
                // From scratch: the cache and what CMake found thrown away.
                let fresh = configure.command.replacen("cmake ", "cmake --fresh ", 1);
                for (name, command) in [
                    (name.clone(), configure.command.clone()),
                    (format!("{name} from Scratch"), fresh),
                ] {
                    tasks.push(Task {
                        name: name.clone(),
                        command: None,
                        group: TaskGroup::Other,
                        detail: detail(&configure.detail),
                    });
                    self.actions.insert(
                        name,
                        Action::Configure {
                            command,
                            dir: configure.dir.clone(),
                            from: configure.from.clone(),
                            project: p.dir.clone(),
                        },
                    );
                }
            }
            for task in presets.build_and_test_tasks(&options) {
                let CommandTask {
                    name,
                    command,
                    test,
                    detail: d,
                } = task;
                tasks.push(Task {
                    name: self.renamed(&p.dir, &name),
                    command: Some(in_folder(&p.dir, &command)),
                    group: if test {
                        TaskGroup::Test
                    } else {
                        TaskGroup::Build
                    },
                    detail: detail(&d),
                });
            }
            let (build, targets) = if p.dir == self.project {
                (self.build.clone(), self.targets.clone())
            } else {
                match self.saved.get(&p.dir) {
                    Some(s) => (s.build.clone(), s.targets.clone()),
                    None => (None, Vec::new()),
                }
            };
            let Some((dir, from)) = build else {
                continue;
            };
            let quoted = quote(&dir);
            let folder = format!("{dir}/");
            for target in &targets {
                let name = self.renamed(&p.dir, &format!("CMake: Build {}", target.name));
                tasks.push(Task {
                    name: name.clone(),
                    command: Some(format!(
                        "cmake --build {quoted} --target {}",
                        quote(&target.name)
                    )),
                    group: TaskGroup::Build,
                    detail: detail(&target.kind.to_ascii_lowercase().replace('_', " ")),
                });
                let Some(program) = target.artifact.as_ref().filter(|_| target.is_program()) else {
                    continue;
                };
                tasks.push(Task {
                    name: self.renamed(&p.dir, &format!("CMake: Run {}", target.name)),
                    command: Some(format!(
                        "cmake --build {quoted} --target {} && {}",
                        quote(&target.name),
                        quote(&run_path(program))
                    )),
                    group: TaskGroup::Other,
                    detail: detail(&folder),
                });
                // F5 debugs it with the C/C++ plugin's debugger, built
                // first by its task.
                debug.push(DebugTarget {
                    name: target.name.clone(),
                    debugger: String::new(),
                    language: "cpp".into(),
                    program: program.clone(),
                    args: Vec::new(),
                    build: name,
                    detail: detail(&from),
                    launch: String::new(),
                });
            }
            tasks.push(Task {
                name: self.renamed(&p.dir, "CMake: Clean"),
                command: Some(format!("cmake --build {quoted} --target clean")),
                group: TaskGroup::Build,
                detail: detail(&folder),
            });
            tasks.push(Task {
                name: self.renamed(&p.dir, "CMake: Install"),
                command: Some(format!("cmake --install {quoted}")),
                group: TaskGroup::Other,
                detail: detail(&folder),
            });
        }
        host::set_tasks("cmake", &tasks);
        host::set_debug_targets(&debug);
    }

    /// Runs a configure: CMake's file API asked first, so its targets can
    /// be read after.
    fn configure(&mut self, command: &str, dir: &str, from: &str) {
        let run = format!("configure:{dir}");
        let command = format!("{} && {command}", fileapi::query(dir));
        match host::run_in_terminal(&run, "CMake: Configure", &command) {
            Ok(()) => {
                self.running.insert(
                    run,
                    (dir.to_string(), from.to_string(), self.project.clone()),
                );
                self.state = State::Configuring(from.to_string());
                self.chips();
            }
            Err(err) => host::notice(LogLevel::Error, &err, &[]),
        }
    }

    /// Configures the build in use again, as when its files are saved.
    fn reconfigure(&mut self) {
        let Some((dir, _)) = self.build.clone() else {
            return;
        };
        let action = self.actions.values().find_map(|a| match a {
            Action::Configure {
                command,
                dir: d,
                from,
                ..
            } if *d == dir && !command.contains("--fresh") => Some((command.clone(), from.clone())),
            _ => None,
        });
        if let Some((command, from)) = action {
            // Cache changes from the view go along as -D.
            let mut command = command;
            for (name, value) in &self.pending {
                command.push_str(&format!(" {}", quote(&format!("-D{name}={value}"))));
            }
            self.editing = None;
            self.configure(&command, &dir, &from);
        }
    }

    fn share(&self, dir: &str, from: &str) {
        let value = CompileCommands {
            dir: dir.to_string(),
            from: from.to_string(),
        };
        if let Err(err) = shared::provide(&value) {
            host::log(LogLevel::Warn, &format!("Couldn't share the build: {err}"));
        }
    }
}

impl Cmake {
    /// The CMake view: the build, its targets by folder, its tests, and
    /// its cache.
    fn panel(&self) -> Panel {
        let mut panel = Panel::new("");
        let cache_text = self
            .build
            .as_ref()
            .and_then(|(dir, _)| read(&format!("{dir}/CMakeCache.txt")));
        // The build: what it is, and how it is.
        let generator = cache_text
            .as_deref()
            .and_then(|c| {
                c.lines()
                    .find_map(|l| l.strip_prefix("CMAKE_GENERATOR:INTERNAL="))
            })
            .unwrap_or_default();
        // Several projects: each with how its build is, and Configure.
        if self.several() {
            let open = !self.folded.contains("projects");
            panel = panel.row(section(
                "projects",
                "Projects",
                open,
                self.projects.len().to_string(),
            ));
            if open {
                for p in &self.projects {
                    let ours = p.dir == self.project;
                    let (build, state) = if ours {
                        (self.build.as_ref(), &self.state)
                    } else {
                        let saved = self.saved.get(&p.dir);
                        (
                            saved.and_then(|s| s.build.as_ref()),
                            saved.map_or(&State::NotConfigured, |s| &s.state),
                        )
                    };
                    let target = if ours { self.target.as_ref() } else { None };
                    let detail = match (state, build) {
                        (State::Failed(_), _) => "failed".to_string(),
                        (State::Configuring(_), _) => "configuring".into(),
                        (_, Some((_, from))) => match target {
                            Some(t) => format!("{} · ◎ {t}", short(from)),
                            None => short(from).to_string(),
                        },
                        _ => "not configured".into(),
                    };
                    let (icon, colour) = state_mark(state);
                    let mut row = Row::new(format!("project:{}", p.dir), p.name.clone())
                        .detail(detail)
                        .depth(1)
                        .icon(icon)
                        .icon_colour(colour);
                    row = if ours {
                        row.count("shown below")
                    } else {
                        row.action(Button::new(
                            format!("project-configure:{}", p.dir),
                            "Configure",
                        ))
                    };
                    panel = panel.row(row);
                }
            }
            panel = panel.divider();
        }
        let (label, detail, action) = match (&self.state, &self.build) {
            (State::Configuring(from), _) => {
                (format!("Configuring {}…", short(from)), String::new(), None)
            }
            (State::Failed(from), _) => (
                format!("{} failed to configure", short(from)),
                "Its errors are in Problems".into(),
                Some(("Configure", Tone::Error)),
            ),
            (State::Ready, Some((dir, from))) => {
                let when = match self.configured_at.map(|t| t.elapsed().as_secs() / 60) {
                    Some(0) => "Configured just now".to_string(),
                    Some(1) => "Configured 1 min ago".into(),
                    Some(m) => format!("Configured {m} min ago"),
                    None => "Configured".into(),
                };
                let what = if generator.is_empty() {
                    short(from).to_string()
                } else {
                    format!("{} · {generator}", short(from))
                };
                (
                    what,
                    format!("{when} · {dir}"),
                    Some(("Configure", Tone::Strong)),
                )
            }
            _ => (
                "Not configured".into(),
                "Configure to build, run and test".into(),
                Some(("Configure", Tone::Strong)),
            ),
        };
        let menu = [
            ("configure-fresh", "Configure from Scratch"),
            ("clean", "Clean"),
            ("install", "Install"),
            ("open-cache", "Open CMakeCache.txt"),
            (
                "advanced",
                if self.advanced {
                    "Hide Advanced Variables"
                } else {
                    "Show Advanced Variables"
                },
            ),
        ]
        .map(|(id, label)| Button::new(id, label))
        .to_vec();
        let label = if self.several() {
            format!("{} · {label}", self.name_of(&self.project))
        } else {
            label
        };
        // Configure shows always, as the mock-up's main button; a click on
        // the row configures too.
        let (icon, colour) = state_mark(&self.state);
        let mut row = Row::new("build", label)
            .detail(detail)
            .icon(icon)
            .icon_colour(colour);
        if let Some((label, tone)) = action {
            row = row.chip(Chip {
                id: "configure".into(),
                label: label.into(),
                tone,
            });
        }
        row.menu = menu;
        panel = panel.row(row);
        panel = panel.divider();
        // Targets, by their FOLDER property, else their directory.
        let open = !self.folded.contains("targets");
        panel = panel.row(section(
            "targets",
            "Targets",
            open,
            self.targets.len().to_string(),
        ));
        if open {
            let mut folders: Vec<String> = self
                .targets
                .iter()
                .map(|t| t.folder.clone().unwrap_or_else(|| t.dir.clone()))
                .collect();
            folders.sort();
            folders.dedup();
            for folder in folders {
                let in_it: Vec<&Target> = self
                    .targets
                    .iter()
                    .filter(|t| t.folder.as_ref().unwrap_or(&t.dir) == &folder)
                    .collect();
                // The top folder's targets sit at the top, with no row of
                // their own.
                let depth = if folder == "." {
                    1
                } else {
                    let open = !self.folded.contains(&format!("folder:{folder}"));
                    panel = panel.row(
                        Row::new(format!("folder:{folder}"), folder.clone())
                            .depth(1)
                            .expanded(open)
                            .icon("▸"),
                    );
                    if !open {
                        continue;
                    }
                    2
                };
                for t in in_it {
                    let opened = self.opened.contains(&t.name);
                    // Build, Run and Debug while the pointer is on it.
                    let mut row = Row::new(format!("target:{}", t.name), t.name.clone())
                        .detail(t.kind.to_ascii_lowercase().replace('_', " "))
                        .depth(depth)
                        .icon(if t.is_program() { "▶" } else { "▤" })
                        .icon_colour(if t.is_program() { "blue" } else { "grey" })
                        .action(Button::new(format!("build:{}", t.name), "Build"));
                    if t.is_program() {
                        row = row
                            .action(Button::new(format!("run:{}", t.name), "Run"))
                            .action(Button::new(format!("debug:{}", t.name), "Debug"));
                    }
                    if !t.sources.is_empty() {
                        row = row.expanded(opened);
                    }
                    if self.target.as_ref() == Some(&t.name) {
                        row = row.count("◎");
                    }
                    panel = panel.row(row);
                    if opened {
                        for source in &t.sources {
                            let name = source.rsplit('/').next().unwrap_or(source);
                            panel = panel.row(
                                Row::new(format!("source:{source}"), name)
                                    .depth(depth + 1)
                                    .icon("·"),
                            );
                        }
                    }
                }
            }
        }
        // CTest's tests, folded to their count.
        let (passed, failed) = self.test_counts;
        let count = if failed > 0 {
            format!("{failed} failed")
        } else if passed > 0 {
            format!("{passed} passed")
        } else {
            self.tests.len().to_string()
        };
        panel = panel.row(
            Row::new("tests", "Tests")
                .expanded(false)
                .count(count)
                .action(Button::new("run-tests", "Run All")),
        );
        panel = panel.divider();
        // The cache, to filter and change.
        let variables = cache_text
            .as_deref()
            .map(cache::variables)
            .unwrap_or_default();
        let shown: Vec<&cache::Variable> = variables
            .iter()
            .filter(|v| self.advanced || !v.advanced)
            .filter(|v| {
                self.filter.is_empty()
                    || v.name.to_lowercase().contains(&self.filter.to_lowercase())
            })
            .collect();
        let open = !self.folded.contains("cache");
        panel = panel.row(section("cache", "Cache", open, shown.len().to_string()));
        if open && !variables.is_empty() {
            panel = panel.input("cache-filter", "Filter variables");
            for v in shown.iter().take(200) {
                let pending = self.pending.get(&v.name);
                let value = pending.unwrap_or(&v.value);
                // A switch for a boolean, a choice for one with STRINGS, a
                // box for the rest: a click on the row changes it.
                let mut row = Row::new(format!("var:{}", v.name), v.name.clone()).depth(1);
                row = if v.is_bool() {
                    row.toggle(is_on(value))
                } else if !v.choices.is_empty() {
                    row.detail(format!("{value} ▾"))
                } else {
                    row.detail(value.clone())
                        .action(Button::new(format!("edit:{}", v.name), "Edit"))
                };
                if pending.is_some() {
                    row = row.count("changed");
                }
                panel = panel.row(row);
                if self.editing.as_ref() == Some(&v.name) {
                    panel = panel.input(format!("value:{}", v.name), value.clone());
                }
            }
        }
        // Changes wait until Configure passes them as -D.
        if !self.pending.is_empty() {
            let n = self.pending.len();
            panel = panel
                .footer()
                .text(format!(
                    "{n} change{} to the cache",
                    if n == 1 { "" } else { "s" }
                ))
                .node(Node::Buttons(vec![
                    Button::new("apply", "Configure"),
                    Button::new("undo", "Undo"),
                ]));
        }
        panel
    }

    /// Asks which target takes a new source file, once per file.
    fn offer_target(&mut self, path: &str) {
        let path = relative(path);
        if self.state != State::Ready || !addfile::is_source(&path) || self.asked.contains(&path) {
            return;
        }
        let candidates = addfile::candidates(&self.targets, &path);
        if candidates.is_empty() {
            return;
        }
        self.asked.insert(path.clone());
        let name = path.rsplit('/').next().unwrap_or(&path);
        // The nearest target picked first; each with where it's added.
        let options: Vec<PickItem> = candidates
            .iter()
            .map(|t| PickItem {
                id: t.name.clone(),
                label: t.name.clone(),
                detail: t
                    .defined
                    .as_ref()
                    .map(|(file, line)| format!("{}, line {line}", relative(file)))
                    .unwrap_or_default(),
                group: String::new(),
                current: false,
            })
            .collect();
        let chosen = candidates[0].name.clone();
        self.adding = Some((path.clone(), chosen.clone()));
        host::notice_with_choice(
            LogLevel::Info,
            &format!("Add {name} to a target? No target builds it yet."),
            &Choice {
                id: "cmake-add".into(),
                options,
                chosen,
            },
            &[
                Button::new("not-now", "Not Now"),
                Button::new("add", "Add"),
            ],
        );
    }

    /// Puts a source in the command that adds the target, as an edit
    /// Undo reverts.
    fn add_to_target(&self, target: &str, path: &str) {
        let Some(t) = self.targets.iter().find(|t| t.name == target) else {
            return;
        };
        let Some((file, line)) = &t.defined else {
            return;
        };
        let root = host::workspace_root();
        let file = file
            .strip_prefix(&format!("{}/", root.trim_end_matches('/')))
            .unwrap_or(file);
        let dir = file.rsplit_once('/').map_or(".", |(d, _)| d);
        let Some(text) = read(file) else {
            return;
        };
        let Some(((line, column), insert)) = addfile::insertion(&text, *line, path, dir) else {
            return host::notice(
                LogLevel::Warn,
                &format!("Couldn't find where {target} is added in {file}"),
                &[],
            );
        };
        let at = Position { line, column };
        let edit = FileEdit {
            path: file.to_string(),
            edits: vec![TextChange {
                start: at,
                end: at,
                text: insert,
            }],
        };
        if let Err(err) = host::apply_edits(&[edit]) {
            host::notice(LogLevel::Error, &err, &[]);
        }
    }

    /// A click in the view or on a chip.
    fn clicked(&mut self, id: &str) {
        if id == "add" {
            if let Some((path, target)) = self.adding.take() {
                self.add_to_target(&target, &path);
            }
            return;
        }
        if id == "not-now" {
            self.adding = None;
            return;
        }
        if let Some(rest) = id.strip_prefix("add:")
            && let Some((target, path)) = rest.split_once(':')
        {
            return self.add_to_target(target, path);
        }
        if let Some(project) = id.strip_prefix("project-configure:") {
            self.pinned = true;
            self.switch(project);
            return self.clicked("configure");
        }
        if let Some(project) = id.strip_prefix("project:") {
            self.pinned = true;
            self.switch(project);
            return;
        }
        if let Some(name) = id.strip_prefix("target:") {
            if !self.opened.remove(name) {
                self.opened.insert(name.to_string());
            }
            self.target = Some(name.to_string());
            self.chips();
            return;
        }
        if let Some(path) = id.strip_prefix("source:") {
            host::open_at(path, Position { line: 0, column: 0 });
            return;
        }
        for (prefix, then) in [("build:", "build"), ("run:", "run"), ("debug:", "debug")] {
            if let Some(name) = id.strip_prefix(prefix) {
                self.target = Some(name.to_string());
                self.chips();
                self.build_target(then);
                return;
            }
        }
        if let Some(rest) = id.strip_prefix("set:")
            && let Some((name, value)) = rest.split_once('=')
        {
            self.pending.insert(name.to_string(), value.to_string());
            return;
        }
        // A click on a variable's row flips a switch, opens a choice's
        // picker, or edits it in a box, as its Edit does.
        if let Some(name) = id.strip_prefix("var:") {
            let variables = self
                .build
                .as_ref()
                .and_then(|(dir, _)| read(&format!("{dir}/CMakeCache.txt")))
                .map(|c| cache::variables(&c))
                .unwrap_or_default();
            let Some(v) = variables.iter().find(|v| v.name == name) else {
                return;
            };
            let value = self.pending.get(name).unwrap_or(&v.value).clone();
            if v.is_bool() {
                let flipped = if is_on(&value) { "OFF" } else { "ON" };
                self.pending.insert(name.to_string(), flipped.into());
            } else if !v.choices.is_empty() {
                let items: Vec<PickItem> = v
                    .choices
                    .iter()
                    .map(|c| PickItem {
                        id: c.clone(),
                        label: c.clone(),
                        detail: String::new(),
                        group: String::new(),
                        current: *c == value,
                    })
                    .collect();
                host::show_picker(&format!("cmake-var:{name}"), name, &items);
            } else {
                self.editing = Some(name.to_string());
            }
            return;
        }
        if let Some(name) = id.strip_prefix("edit:") {
            self.editing = Some(name.to_string());
            return;
        }
        match id {
            "targets" | "cache" | "projects" => {
                if !self.folded.remove(id) {
                    self.folded.insert(id.to_string());
                }
            }
            id if id.starts_with("folder:") => {
                if !self.folded.remove(id) {
                    self.folded.insert(id.to_string());
                }
            }
            "configure" | "build" => {
                if self.build.is_some() {
                    self.reconfigure();
                } else {
                    self.pick_preset();
                }
            }
            "configure-fresh" => {
                if let Some((dir, _)) = &self.build {
                    let fresh = self.actions.iter().find_map(|(name, a)| {
                        let Action::Configure { dir: d, .. } = a;
                        (d == dir && name.ends_with("from Scratch")).then(|| a.clone())
                    });
                    if let Some(Action::Configure {
                        command, dir, from, ..
                    }) = fresh
                    {
                        self.configure(&command, &dir, &from);
                    }
                }
            }
            "clean" | "install" => {
                if let Some((dir, _)) = &self.build {
                    let command = if id == "clean" {
                        format!("cmake --build {} --target clean", quote(dir))
                    } else {
                        format!("cmake --install {}", quote(dir))
                    };
                    let _ = host::run_in_terminal(
                        &format!("cmake-{id}"),
                        &format!("CMake: {}", if id == "clean" { "Clean" } else { "Install" }),
                        &command,
                    );
                }
            }
            "open-cache" => {
                if let Some((dir, _)) = &self.build {
                    host::open_at(
                        &format!("{dir}/CMakeCache.txt"),
                        Position { line: 0, column: 0 },
                    );
                }
            }
            "advanced" => self.advanced = !self.advanced,
            "run-tests" => self.run_tests(None, false),
            "apply" => self.reconfigure(),
            "undo" => {
                self.pending.clear();
                self.editing = None;
            }
            _ => {}
        }
    }
}

/// A build's state as a mark and its colour: green when it's configured,
/// red when it failed, grey before.
fn state_mark(state: &State) -> (&'static str, &'static str) {
    match state {
        State::Ready => ("●", "green"),
        State::Failed(_) => ("✕", "red"),
        State::Configuring(_) => ("⟳", "blue"),
        State::NotConfigured => ("○", "grey"),
    }
}

/// Whether a cache value is CMake's true.
fn is_on(value: &str) -> bool {
    matches!(
        value.to_ascii_uppercase().as_str(),
        "ON" | "TRUE" | "1" | "YES" | "Y"
    )
}

/// A section's row, open or folded, with its count.
fn section(id: &str, label: &str, open: bool, count: String) -> Row {
    Row::new(id, label).expanded(open).count(count)
}

fn read(path: &str) -> Option<String> {
    host::read_file(path).ok()
}

fn quote(word: &str) -> String {
    if word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
    {
        word.to_string()
    } else {
        format!("\"{}\"", word.replace('"', "\\\""))
    }
}

/// A program's path as a shell runs it: `./` before a relative one.
fn run_path(program: &str) -> String {
    if program.starts_with('/') || program.starts_with("./") || program.contains(':') {
        program.to_string()
    } else {
        format!("./{program}")
    }
}

/// The open file, if it's CMake's.
fn active() -> Option<String> {
    let path = host::active_path().ok()??;
    is_cmake(&path).then_some(path)
}

fn is_cmake(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name == "CMakeLists.txt" || name.to_ascii_lowercase().ends_with(".cmake")
}

/// The lexer's runs as tokens, with UTF-16 columns.
fn tokens(text: &str) -> Vec<Token> {
    let lines: Vec<&str> = text.split('\n').collect();
    let at = |line: usize, chars: usize| Position {
        line: line as u32,
        column: utf16_column(lines.get(line).copied().unwrap_or_default(), chars),
    };
    lex::lex(text)
        .into_iter()
        .map(|r| Token {
            start: at(r.line, r.start),
            end: at(r.line, r.end),
            kind: match r.kind {
                Kind::Keyword => TokenKind::Keyword,
                Kind::Function => TokenKind::Function,
                Kind::String => TokenKind::String,
                Kind::Escape => TokenKind::Escape,
                Kind::Comment => TokenKind::Comment,
                Kind::Variable => TokenKind::Variable,
                Kind::Macro => TokenKind::Macro,
                Kind::Constant => TokenKind::Constant,
            },
        })
        .collect()
}

impl Plugin for Cmake {
    fn run_command(&mut self, id: &str) {
        self.start();
        if id == "find-tasks" {
            // Run Task opened: the presets read again, as they may have
            // changed.
            self.find_tasks();
            return;
        }
        if id == "configure" || id == "configure-fresh" {
            return self.clicked(id);
        }
        match id {
            "find-tests" => return self.find_tests(),
            "run-tests:all" => return self.run_tests(None, false),
            "run-tests:failed" => return self.run_tests(None, true),
            _ => {}
        }
        if id.starts_with("run-tests:group:") {
            return self.run_tests(None, false);
        }
        if let Some(file) = id.strip_prefix("run-tests:file:") {
            let names = self
                .tests
                .iter()
                .filter(|t| t.file == file)
                .map(|t| t.name.clone())
                .collect();
            return self.run_tests(Some(names), false);
        }
        if let Some(name) = id.strip_prefix("run-test:") {
            return self.run_tests(Some(vec![name.to_string()]), false);
        }
        if let Some(name) = id.strip_prefix("debug-test:") {
            return self.debug_test(name);
        }
        let Some(name) = id.strip_prefix("task:") else {
            return;
        };
        if let Some(Action::Configure {
            command,
            dir,
            from,
            project,
        }) = self.actions.get(name).cloned()
        {
            // Another project's: that one is in use now.
            self.switch(&project);
            self.configure(&command, &dir, &from);
        }
    }

    /// `ctest --show-only`'s output, line by line, read once it ends.
    fn process(&mut self, process: u32, event: ProcessEvent) {
        let Some((listing, text)) = &mut self.listing else {
            return;
        };
        if *listing != process {
            return;
        }
        match event {
            ProcessEvent::Message(line) => {
                text.push_str(&line);
                text.push('\n');
            }
            ProcessEvent::Exit(_) => {
                let text = std::mem::take(text);
                self.listing = None;
                let tests = ctest::tests(&text, &host::workspace_root());
                self.show_tests(tests);
            }
            ProcessEvent::Stderr(_) => {}
        }
    }

    /// A configure ended: once it worked, its build is the one in use. A
    /// test run ended: how each test went.
    fn terminal(&mut self, id: &str, output: &str, code: Option<i32>) {
        if id == "ctest-run" {
            let found = ctest::results(output);
            let mut results: Vec<TestResult> = found
                .iter()
                .map(|(name, outcome)| TestResult {
                    id: name.clone(),
                    outcome: match outcome {
                        ctest::Outcome::Passed => TestOutcome::Passed,
                        ctest::Outcome::Failed(_) => TestOutcome::Failed,
                        ctest::Outcome::Skipped => TestOutcome::Skipped,
                    },
                    message: match outcome {
                        ctest::Outcome::Failed(why) => why.clone(),
                        _ => String::new(),
                    },
                    line: None,
                })
                .collect();
            // A test that never ran, as when the build failed, isn't
            // running any more.
            for name in std::mem::take(&mut self.test_run) {
                if !found.iter().any(|(n, _)| *n == name) {
                    results.push(TestResult {
                        id: name,
                        outcome: TestOutcome::Skipped,
                        message: if code == Some(0) {
                            String::new()
                        } else {
                            "didn't run".into()
                        },
                        line: None,
                    });
                }
            }
            self.test_counts = (
                results
                    .iter()
                    .filter(|r| r.outcome == TestOutcome::Passed)
                    .count(),
                results
                    .iter()
                    .filter(|r| r.outcome == TestOutcome::Failed)
                    .count(),
            );
            host::set_test_results(&results);
            host::set_test_run("ctest", false, None);
            return;
        }
        let Some((dir, from, project)) = self.running.remove(id) else {
            return;
        };
        self.switch(&project);
        if code == Some(0) {
            self.configured_at = Some(std::time::Instant::now());
            self.pending.clear();
            self.configured(dir, from);
            self.find_tasks();
        } else {
            self.state = State::Failed(from);
            self.chips();
        }
    }

    fn setting_changed(&mut self, _key: &str) {
        self.find_tasks();
    }

    fn commands(&self) -> Vec<Command> {
        vec![
            Command::new("configure", "CMake: Configure"),
            Command::new("configure-fresh", "CMake: Configure from Scratch"),
        ]
    }

    fn view(&mut self, id: &str) -> Option<Panel> {
        self.start();
        (id == "cmake").then(|| self.panel())
    }

    /// The status bar's items, and the view's.
    fn click(&mut self, button: &str) {
        self.start();
        match button {
            "project" => self.pick_project(),
            "preset" => self.pick_preset(),
            "target" => self.pick_target(),
            "build" | "run" | "debug" => self.build_target(button),
            other => self.clicked(other),
        }
    }

    fn submit(&mut self, input: &str, text: &str) {
        if input == "cache-filter" {
            self.filter = text.trim().to_string();
        } else if let Some(name) = input.strip_prefix("value:") {
            self.pending
                .insert(name.to_string(), text.trim().to_string());
            self.editing = None;
        }
    }

    fn pick(&mut self, picker: &str, item: &str) {
        match picker {
            "cmake-project" => {
                if item == "follow" {
                    self.pinned = !self.pinned;
                    self.follow();
                    self.chips();
                } else if let Some(project) = item.strip_prefix("project:") {
                    self.pinned = true;
                    self.switch(project);
                }
            }
            picker if picker.starts_with("cmake-var:") => {
                let name = &picker["cmake-var:".len()..];
                self.pending.insert(name.to_string(), item.to_string());
            }
            "cmake-add" => {
                if let Some((_, target)) = &mut self.adding {
                    *target = item.to_string();
                }
            }
            "cmake-target" => {
                self.target = Some(item.to_string());
                self.chips();
            }
            "cmake-preset" => {
                let Some(Action::Configure {
                    command, dir, from, ..
                }) = self.actions.get(item).cloned()
                else {
                    return;
                };
                // Another build that's configured already is used as it
                // is; any other, or from scratch, configures.
                let configured = !item.ends_with("from Scratch")
                    && self.build.as_ref().is_some_and(|(d, _)| *d != dir)
                    && read(&format!("{dir}/CMakeCache.txt")).is_some();
                if configured {
                    self.configured(dir, from);
                    self.find_tasks();
                } else {
                    self.configure(&command, &dir, &from);
                }
            }
            _ => {}
        }
    }

    fn colour_text(&mut self, _language: &str, text: &str) -> Vec<Token> {
        if text.len() > MAX_BYTES {
            return Vec::new();
        }
        tokens(text)
    }

    fn buffer(&mut self, event: BufferEvent) {
        self.start();
        if matches!(event, BufferEvent::Switched) {
            self.follow();
        }
        if let BufferEvent::Saved(path) = &event {
            let path = relative(path);
            let presets =
                path.ends_with("CMakePresets.json") || path.ends_with("CMakeUserPresets.json");
            // A CMakeLists.txt may add or take away a project.
            if path.ends_with("CMakeLists.txt") {
                let before = self.projects.clone();
                self.find_projects();
                if self.projects != before {
                    self.find_tasks();
                    self.chips();
                }
            }
            if presets {
                self.find_tasks();
            }
            // Only the project in use configures, or takes new files.
            let ours = holding(&self.projects, &path).is_some_and(|p| p.dir == self.project);
            if !ours {
                return;
            }
            // A new source no target has: which target takes it.
            self.offer_target(&path);
            // A build that's configured configures again, as CMake Tools
            // does, unless that's turned off.
            if (presets || is_cmake(&path)) && settings::flag("configure-on-save").unwrap_or(true) {
                self.reconfigure();
            }
            return;
        }
        let Some(path) = active() else {
            return;
        };
        let version = host::active_version().ok();
        if let Some((last_path, last_version)) = &self.last
            && *last_path == path
            && version.is_some()
            && *last_version == version
        {
            return;
        }
        let Ok(text) = host::active_text() else {
            return;
        };
        if text.len() > MAX_BYTES {
            self.last = None;
            return;
        }
        host::set_tokens(&path, TokenLayer::Base, None, &tokens(&text));
        self.last = Some((path, version));
    }
}

merlin_plugin::export!(WithLsp<Cmake> => WithLsp::new(
    Cmake::default(),
    "CMake",
    &[CMAKE_SERVER],
));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmake_files_by_name_and_extension() {
        assert!(is_cmake("CMakeLists.txt"));
        assert!(is_cmake("src/lib/CMakeLists.txt"));
        assert!(is_cmake("cmake/FindFoo.cmake"));
        assert!(!is_cmake("notes.txt"));
        assert!(!is_cmake("MyCMakeLists.txt"));
    }

    #[test]
    fn a_projects_commands_run_in_its_folder() {
        assert_eq!(in_folder("", "cmake --preset debug"), "cmake --preset debug");
        assert_eq!(
            in_folder("tools", "cmake --preset debug"),
            "cd tools && cmake --preset debug"
        );
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            debugger_args("tools", &words(&["-S", ".", "-B", "build", "-DX=1"])),
            words(&["-S", "tools", "-B", "tools/build", "-DX=1"])
        );
        assert_eq!(
            debugger_args("tools", &words(&["--preset", "debug"])),
            words(&["-S", "tools", "--preset", "debug"]),
            "presets are read from the source folder"
        );
        assert_eq!(
            debugger_args("", &words(&["--preset", "debug"])),
            words(&["--preset", "debug"])
        );
    }
}
