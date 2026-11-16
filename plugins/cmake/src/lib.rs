//! CMake, as a plugin: colours for `CMakeLists.txt` and `.cmake` files,
//! Configure, Build and Test as tasks, from the project's presets when it
//! has them, and the build's `compile_commands.json` shared with the C/C++
//! plugin, so clangd finds it wherever the build is. neocmakelsp, when
//! it's installed, completes, explains and finds what's in them.

pub mod fileapi;
pub mod lex;
pub mod presets;

use std::collections::BTreeMap;

use merlin_lsp::{Spec, WithLsp};
use merlin_plugin::prelude::*;
use merlin_plugin::shared::{self, CompileCommands};
use merlin_plugin::text::utf16_column;

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
    }

    /// A build that's configured: shared with C/C++, and its targets read.
    fn configured(&mut self, dir: String, from: String) {
        self.share(&dir, &from);
        self.targets =
            fileapi::targets(&dir, |d| host::list_folder(d).ok(), read).unwrap_or_default();
        self.build = Some((dir, from));
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
        let Some(name) = id.strip_prefix("task:") else {
            return;
        };
        if let Some(Action::Configure { command, dir, from }) = self.actions.get(name).cloned() {
            self.configure(&command, &dir, &from);
        }
    }

    /// A configure ended: once it worked, its build is the one in use.
    fn terminal(&mut self, id: &str, _output: &str, code: Option<i32>) {
        let Some((dir, from)) = self.running.remove(id) else {
            return;
        };
        if code == Some(0) {
            self.configured(dir, from);
            self.find_tasks();
        }
    }

    fn setting_changed(&mut self, _key: &str) {
        self.find_tasks();
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
