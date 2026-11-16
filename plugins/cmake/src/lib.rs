//! CMake, as a plugin: colours for `CMakeLists.txt` and `.cmake` files,
//! Configure, Build and Test as tasks, from the project's presets when it
//! has them, and the build's `compile_commands.json` shared with the C/C++
//! plugin, so clangd finds it wherever the build is.

pub mod lex;
pub mod presets;

use std::collections::BTreeMap;

use merlin_plugin::prelude::*;
use merlin_plugin::shared::{self, CompileCommands};
use merlin_plugin::text::utf16_column;

use lex::Kind;
use presets::Presets;

const MAX_BYTES: usize = 512 * 1024;

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
}

impl Cmake {
    /// Reads the presets, offers their tasks, and shares the build that's
    /// already configured, if there is one: once, when it first has a
    /// workspace to read.
    fn start(&mut self) {
        if self.started || host::workspace_root().is_empty() {
            return;
        }
        self.started = true;
        let presets = self.find_tasks();
        if let Some((dir, from)) = presets.configured(|path| host::read_file(path).ok()) {
            self.share(&dir, &from);
        }
    }

    /// The project's presets, and its tasks from them, for Run Task.
    fn find_tasks(&mut self) -> Presets {
        let read = |name: &str| host::read_file(name).ok();
        let presets = Presets::read(read("CMakePresets.json"), read("CMakeUserPresets.json"));
        let mut tasks = Vec::new();
        self.actions.clear();
        for configure in presets.configure_tasks() {
            tasks.push(Task {
                name: configure.name.clone(),
                command: None,
                group: TaskGroup::Other,
                detail: configure.detail.clone(),
            });
            self.actions.insert(
                configure.name,
                Action::Configure {
                    command: configure.command,
                    dir: configure.dir,
                    from: configure.from,
                },
            );
        }
        for task in presets.build_and_test_tasks() {
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
        host::set_tasks("cmake", &tasks);
        presets
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
        let Some(Action::Configure { command, dir, from }) = self.actions.get(name).cloned() else {
            return;
        };
        let run = format!("configure:{dir}");
        match host::run_in_terminal(&run, "CMake: Configure", &command) {
            Ok(()) => {
                self.running.insert(run, (dir, from));
            }
            Err(err) => host::notice(LogLevel::Error, &err, &[]),
        }
    }

    /// A configure ended: once it worked, its build is the one clangd
    /// reads.
    fn terminal(&mut self, id: &str, _output: &str, code: Option<i32>) {
        let Some((dir, from)) = self.running.remove(id) else {
            return;
        };
        if code == Some(0) {
            self.share(&dir, &from);
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
            // Presets saved: their tasks change.
            if path.ends_with("CMakePresets.json") || path.ends_with("CMakeUserPresets.json") {
                self.find_tasks();
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

merlin_plugin::export!(Cmake);

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
