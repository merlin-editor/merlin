//! CMake, as a plugin: colours for `CMakeLists.txt` and `.cmake` files,
//! Configure, Build and Test as tasks, from the project's presets when it
//! has them, and the build's `compile_commands.json` shared with the C/C++
//! plugin, so clangd finds it wherever the build is. neocmakelsp, when
//! it's installed, completes, explains and finds what's in them.

pub mod ctest;
pub mod fileapi;
pub mod lex;
pub mod presets;

use std::collections::BTreeMap;

use merlin_lsp::{Spec, WithLsp};
use merlin_plugin::prelude::*;
use merlin_plugin::shared::{self, CompileCommands};
use merlin_plugin::text::utf16_column;
use merlin_plugin::{StatusChip, Tone};

use ctest::CTest;
use fileapi::Target;
use lex::Kind;
use presets::{Options, Presets};

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
    },
}

#[derive(Default)]
struct Cmake {
    /// Run Task's tasks that run in the plugin, by name.
    actions: BTreeMap<String, Action>,
    /// Configures running, by terminal id: the folder and what made it.
    running: BTreeMap<String, (String, String)>,
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

impl Cmake {
    /// Reads the presets, the build that's already configured and its
    /// targets, and offers their tasks: once, when it first has a
    /// workspace to read.
    fn start(&mut self) {
        if self.started || host::workspace_root().is_empty() {
            return;
        }
        self.started = true;
        let presets = Presets::read(read("CMakePresets.json"), read("CMakeUserPresets.json"));
        if let Some((dir, from)) = presets.configured(&options(), read) {
            self.configured(dir, from);
        }
        self.find_tasks();
        self.chips();
    }

    /// A build that's configured: shared with C/C++, and its targets read.
    fn configured(&mut self, dir: String, from: String) {
        self.share(&dir, &from);
        self.targets =
            fileapi::targets(&dir, |d| host::list_folder(d).ok(), read).unwrap_or_default();
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
    /// target with Build, Run and Debug.
    fn chips(&self) {
        let label = self.build.as_ref().map(|(_, from)| short(from).to_string());
        let mut chips = vec![match &self.state {
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
        }];
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
        let items: Vec<PickItem> = self
            .actions
            .iter()
            .map(|(name, action)| {
                let Action::Configure { dir, .. } = action;
                let label = name.strip_prefix("CMake: Configure ").unwrap_or(name);
                let label = if name == "CMake: Configure" {
                    "Configure"
                } else {
                    label
                };
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
                });
            }
        }
        items.push(PickItem {
            id: "all".into(),
            label: "all".into(),
            detail: "every target".into(),
            group: "Everything".into(),
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
                    build: format!("CMake: Build {}", t.name),
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

    /// The project's tasks for Run Task, and its programs for Run and
    /// Debug.
    fn find_tasks(&mut self) -> Presets {
        let presets = Presets::read(read("CMakePresets.json"), read("CMakeUserPresets.json"));
        let options = options();
        let mut tasks = Vec::new();
        self.actions.clear();
        for configure in presets.configure_tasks(&options) {
            // From scratch: the cache and what CMake found thrown away.
            let fresh = configure.command.replacen("cmake ", "cmake --fresh ", 1);
            for (name, command) in [
                (configure.name.clone(), configure.command.clone()),
                (format!("{} from Scratch", configure.name), fresh),
            ] {
                tasks.push(Task {
                    name: name.clone(),
                    command: None,
                    group: TaskGroup::Other,
                    detail: configure.detail.clone(),
                });
                self.actions.insert(
                    name,
                    Action::Configure {
                        command,
                        dir: configure.dir.clone(),
                        from: configure.from.clone(),
                    },
                );
            }
        }
        for task in presets.build_and_test_tasks(&options) {
            tasks.push(Task {
                name: task.name,
                command: Some(task.command),
                group: if task.test {
                    TaskGroup::Test
                } else {
                    TaskGroup::Build
                },
                detail: task.detail,
            });
        }
        let mut debug = Vec::new();
        if let Some((dir, from)) = &self.build {
            let build = quote(dir);
            let detail = format!("{dir}/");
            for target in &self.targets {
                let name = format!("CMake: Build {}", target.name);
                tasks.push(Task {
                    name: name.clone(),
                    command: Some(format!(
                        "cmake --build {build} --target {}",
                        quote(&target.name)
                    )),
                    group: TaskGroup::Build,
                    detail: target.kind.to_ascii_lowercase().replace('_', " "),
                });
                let Some(program) = target.artifact.as_ref().filter(|_| target.is_program()) else {
                    continue;
                };
                tasks.push(Task {
                    name: format!("CMake: Run {}", target.name),
                    command: Some(format!(
                        "cmake --build {build} --target {} && {}",
                        quote(&target.name),
                        quote(&run_path(program))
                    )),
                    group: TaskGroup::Other,
                    detail: detail.clone(),
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
                    detail: from.clone(),
                    launch: String::new(),
                });
            }
            tasks.push(Task {
                name: "CMake: Clean".into(),
                command: Some(format!("cmake --build {build} --target clean")),
                group: TaskGroup::Build,
                detail: detail.clone(),
            });
            tasks.push(Task {
                name: "CMake: Install".into(),
                command: Some(format!("cmake --install {build}")),
                group: TaskGroup::Other,
                detail,
            });
        }
        host::set_tasks("cmake", &tasks);
        host::set_debug_targets(&debug);
        presets
    }

    /// Runs a configure: CMake's file API asked first, so its targets can
    /// be read after.
    fn configure(&mut self, command: &str, dir: &str, from: &str) {
        let run = format!("configure:{dir}");
        let command = format!("{} && {command}", fileapi::query(dir));
        match host::run_in_terminal(&run, "CMake: Configure", &command) {
            Ok(()) => {
                self.running
                    .insert(run, (dir.to_string(), from.to_string()));
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
            } if *d == dir && !command.contains("--fresh") => Some((command.clone(), from.clone())),
            _ => None,
        });
        if let Some((command, from)) = action {
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
        if let Some(Action::Configure { command, dir, from }) = self.actions.get(name).cloned() {
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
            host::set_test_results(&results);
            host::set_test_run("ctest", false, None);
            return;
        }
        let Some((dir, from)) = self.running.remove(id) else {
            return;
        };
        if code == Some(0) {
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

    /// The status bar's items.
    fn click(&mut self, button: &str) {
        self.start();
        match button {
            "preset" => self.pick_preset(),
            "target" => self.pick_target(),
            "build" | "run" | "debug" => self.build_target(button),
            _ => {}
        }
    }

    fn pick(&mut self, picker: &str, item: &str) {
        match picker {
            "cmake-target" => {
                self.target = Some(item.to_string());
                self.chips();
            }
            "cmake-preset" => {
                let Some(Action::Configure { command, dir, from }) =
                    self.actions.get(item).cloned()
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
        if let BufferEvent::Saved(path) = &event {
            let presets =
                path.ends_with("CMakePresets.json") || path.ends_with("CMakeUserPresets.json");
            if presets {
                self.find_tasks();
            }
            // A build that's configured configures again, as CMake Tools
            // does, unless that's turned off.
            if (presets || is_cmake(path)) && settings::flag("configure-on-save").unwrap_or(true) {
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
}
