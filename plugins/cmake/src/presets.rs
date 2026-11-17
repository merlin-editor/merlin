//! A project's CMake presets, as far as tasks need them: the configure
//! presets and where each builds, and the build and test presets. Without
//! presets, the project builds in `build/`.

use serde_json::Value;

/// A configure task, which runs in the plugin so it hears when it ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigureTask {
    pub name: String,
    pub command: String,
    /// The build folder, relative to the workspace or absolute.
    pub dir: String,
    /// For people to read, as `CMake · debug`.
    pub from: String,
    pub detail: String,
}

/// A build or test task, which runs its command line in a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandTask {
    pub name: String,
    pub command: String,
    pub test: bool,
    pub detail: String,
}

/// One preset, with its file's name for the task list.
#[derive(Clone, Debug)]
struct Preset {
    name: String,
    display: String,
    hidden: bool,
    inherits: Vec<String>,
    binary_dir: Option<String>,
    file: &'static str,
}

#[derive(Debug, Default)]
pub struct Presets {
    configure: Vec<Preset>,
    build: Vec<Preset>,
    test: Vec<Preset>,
}

const PRESETS: &str = "CMakePresets.json";
const USER_PRESETS: &str = "CMakeUserPresets.json";

/// How to configure: the settings, for a project without presets, and
/// the cache variables every configure adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The build folder without presets.
    pub folder: String,
    /// `Debug` and so on without presets, or empty for CMake's own.
    pub build_type: String,
    /// `Ninja` and so on without presets, or empty for CMake's own.
    pub generator: String,
    /// `NAME=VALUE`, or `NAME:TYPE=VALUE`, for every configure.
    pub cache: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            folder: "build".into(),
            build_type: "Debug".into(),
            generator: String::new(),
            cache: Vec::new(),
        }
    }
}

/// What every configure adds: the file clangd reads, coloured compiler
/// messages, and the user's cache variables.
fn extras(options: &Options) -> String {
    let mut extras =
        String::from("-DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_COLOR_DIAGNOSTICS=ON");
    for variable in &options.cache {
        let variable = variable.trim().trim_start_matches("-D");
        if !variable.is_empty() {
            extras.push_str(&format!(" {}", quote(&format!("-D{variable}"))));
        }
    }
    extras
}

fn quote(word: &str) -> String {
    if word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./=:".contains(c))
    {
        word.to_string()
    } else {
        format!("\"{}\"", word.replace('"', "\\\""))
    }
}

/// A command line's words, as the shell splits them: quotes taken off, a
/// backslash keeping the next character.
pub fn words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                started = false;
            }
            (None, c) => word.push(c),
        }
    }
    if started || !word.is_empty() {
        words.push(word);
    }
    words
}

fn presets(json: &Value, key: &str, file: &'static str) -> Vec<Preset> {
    json[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let name = p["name"].as_str()?.to_string();
            let inherits = match &p["inherits"] {
                Value::String(s) => vec![s.clone()],
                Value::Array(list) => list
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                _ => Vec::new(),
            };
            Some(Preset {
                display: p["displayName"].as_str().unwrap_or(&name).to_string(),
                hidden: p["hidden"].as_bool().unwrap_or(false),
                binary_dir: p["binaryDir"].as_str().map(String::from),
                inherits,
                name,
                file,
            })
        })
        .collect()
}

impl Presets {
    /// The presets in the project's files, if it has them; the user's file
    /// adds to the project's.
    pub fn read(project: Option<String>, user: Option<String>) -> Self {
        let mut all = Self::default();
        for (text, file) in [(project, PRESETS), (user, USER_PRESETS)] {
            let Some(json) = text.and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
                continue;
            };
            all.configure
                .extend(presets(&json, "configurePresets", file));
            all.build.extend(presets(&json, "buildPresets", file));
            all.test.extend(presets(&json, "testPresets", file));
        }
        all
    }

    fn find<'a>(list: &'a [Preset], name: &str) -> Option<&'a Preset> {
        list.iter().find(|p| p.name == name)
    }

    /// A configure preset's build folder: its own, else the first it
    /// inherits, with `${sourceDir}` and `${presetName}` filled in.
    fn binary_dir(&self, preset: &Preset) -> Option<String> {
        let mut seen = Vec::new();
        let mut stack = vec![preset];
        while let Some(p) = stack.pop() {
            if seen.contains(&p.name) {
                continue;
            }
            seen.push(p.name.clone());
            if let Some(dir) = &p.binary_dir {
                return Some(expand(dir, &preset.name));
            }
            for parent in p.inherits.iter().rev() {
                stack.extend(Self::find(&self.configure, parent));
            }
        }
        None
    }

    fn visible(list: &[Preset]) -> impl Iterator<Item = &Preset> {
        list.iter().filter(|p| !p.hidden)
    }

    /// Configure tasks: one per visible configure preset whose build
    /// folder is known, or one into `build/` without presets.
    pub fn configure_tasks(&self, options: &Options) -> Vec<ConfigureTask> {
        let extras = extras(options);
        if self.configure.is_empty() {
            let folder = options.folder.trim_end_matches('/');
            let mut command = format!("cmake -S . -B {}", quote(folder));
            if !options.generator.is_empty() {
                command.push_str(&format!(" -G {}", quote(&options.generator)));
            }
            if !options.build_type.is_empty() {
                command.push_str(&format!(
                    " -DCMAKE_BUILD_TYPE={}",
                    quote(&options.build_type)
                ));
            }
            return vec![ConfigureTask {
                name: "CMake: Configure".into(),
                command: format!("{command} {extras}"),
                dir: folder.into(),
                from: format!("CMake · {folder}"),
                detail: format!("{folder}/"),
            }];
        }
        Self::visible(&self.configure)
            .filter_map(|p| {
                let dir = self.binary_dir(p)?;
                Some(ConfigureTask {
                    name: format!("CMake: Configure {}", p.display),
                    command: format!("cmake --preset {} {extras}", quote(&p.name)),
                    from: format!("CMake · {}", p.display),
                    detail: p.file.into(),
                    dir,
                })
            })
            .collect()
    }

    /// Build and test tasks: the build and test presets, else one of each
    /// per configure preset's folder, else `build/`'s.
    pub fn build_and_test_tasks(&self, options: &Options) -> Vec<CommandTask> {
        let mut tasks = Vec::new();
        let folder = options.folder.trim_end_matches('/');
        let folders: Vec<(String, String, String)> = if self.configure.is_empty() {
            vec![(folder.into(), String::new(), format!("{folder}/"))]
        } else {
            Self::visible(&self.configure)
                .filter_map(|p| {
                    Some((
                        self.binary_dir(p)?,
                        format!(" {}", p.display),
                        p.file.to_string(),
                    ))
                })
                .collect()
        };
        let build: Vec<&Preset> = Self::visible(&self.build).collect();
        if build.is_empty() {
            for (dir, label, detail) in &folders {
                tasks.push(CommandTask {
                    name: format!("CMake: Build{label}"),
                    command: format!("cmake --build {}", quote(dir)),
                    test: false,
                    detail: detail.to_string(),
                });
            }
        }
        for p in build {
            tasks.push(CommandTask {
                name: format!("CMake: Build {}", p.display),
                command: format!("cmake --build --preset {}", quote(&p.name)),
                test: false,
                detail: p.file.into(),
            });
        }
        let test: Vec<&Preset> = Self::visible(&self.test).collect();
        if test.is_empty() {
            for (dir, label, detail) in &folders {
                tasks.push(CommandTask {
                    name: format!("CMake: Test{label}"),
                    command: format!("ctest --test-dir {} --output-on-failure", quote(dir)),
                    test: true,
                    detail: detail.to_string(),
                });
            }
        }
        for p in test {
            tasks.push(CommandTask {
                name: format!("CMake: Test {}", p.display),
                command: format!("ctest --preset {} --output-on-failure", quote(&p.name)),
                test: true,
                detail: p.file.into(),
            });
        }
        tasks
    }

    /// The first build folder that's configured with compile commands on,
    /// as its `CMakeCache.txt` says, and what made it.
    pub fn configured(
        &self,
        options: &Options,
        read: impl Fn(&str) -> Option<String>,
    ) -> Option<(String, String)> {
        self.configure_tasks(options).into_iter().find_map(|t| {
            let cache = read(&format!("{}/CMakeCache.txt", t.dir.trim_end_matches('/')))?;
            cache
                .lines()
                .any(exports_compile_commands)
                .then_some((t.dir, t.from))
        })
    }
}

/// Whether a `CMakeCache.txt` line turns compile commands on, of any type:
/// a `-D` without one is stored as `UNINITIALIZED`.
fn exports_compile_commands(line: &str) -> bool {
    let Some(rest) = line.trim().strip_prefix("CMAKE_EXPORT_COMPILE_COMMANDS") else {
        return false;
    };
    let value = rest.split_once('=').map_or("", |(_, v)| v.trim());
    matches!(
        value.to_ascii_uppercase().as_str(),
        "ON" | "TRUE" | "YES" | "Y" | "1"
    )
}

/// A preset's folder with its macros filled in, relative to the
/// workspace where it's in it.
fn expand(dir: &str, preset: &str) -> String {
    let dir = dir
        .replace("${sourceDir}", ".")
        .replace("${presetName}", preset)
        .replace("${sourceParentDir}", "..");
    let dir = dir.strip_prefix("./").unwrap_or(&dir).to_string();
    if dir.is_empty() { ".".into() } else { dir }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_splits_into_words_as_the_shell_would() {
        assert_eq!(
            words(r#"cmake -S . -B "my build" -DX='a b' -DY=\"q\" ''"#),
            ["cmake", "-S", ".", "-B", "my build", "-DX=a b", "-DY=\"q\"", ""]
        );
    }

    const PROJECT: &str = r#"{
        "version": 6,
        "configurePresets": [
            {"name": "base", "hidden": true, "binaryDir": "${sourceDir}/out/${presetName}"},
            {"name": "debug", "displayName": "Debug", "inherits": "base"},
            {"name": "release", "inherits": ["base"], "binaryDir": "${sourceDir}/rel"}
        ],
        "buildPresets": [{"name": "debug", "configurePreset": "debug"}]
    }"#;

    #[test]
    fn without_presets_it_builds_in_build() {
        let presets = Presets::read(None, None);
        let configure = presets.configure_tasks(&Options::default());
        assert_eq!(configure.len(), 1);
        assert_eq!(
            configure[0].command,
            "cmake -S . -B build -DCMAKE_BUILD_TYPE=Debug \
             -DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_COLOR_DIAGNOSTICS=ON"
        );
        assert_eq!(configure[0].dir, "build");
        let others: Vec<(String, String)> = presets
            .build_and_test_tasks(&Options::default())
            .into_iter()
            .map(|t| (t.name, t.command))
            .collect();
        assert_eq!(
            others,
            [
                ("CMake: Build".into(), "cmake --build build".into()),
                (
                    "CMake: Test".into(),
                    "ctest --test-dir build --output-on-failure".into()
                ),
            ]
        );
    }

    #[test]
    fn presets_give_tasks_and_their_folders() {
        let user = r#"{"configurePresets": [{"name": "mine", "inherits": "debug"}]}"#;
        let presets = Presets::read(Some(PROJECT.into()), Some(user.into()));
        let configure: Vec<(String, String, String)> = presets
            .configure_tasks(&Options::default())
            .into_iter()
            .map(|t| (t.name, t.command, t.dir))
            .collect();
        assert_eq!(
            configure,
            [
                (
                    "CMake: Configure Debug".into(),
                    "cmake --preset debug -DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_COLOR_DIAGNOSTICS=ON".into(),
                    "out/debug".into()
                ),
                (
                    "CMake: Configure release".into(),
                    "cmake --preset release -DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_COLOR_DIAGNOSTICS=ON".into(),
                    "rel".into()
                ),
                (
                    "CMake: Configure mine".into(),
                    "cmake --preset mine -DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_COLOR_DIAGNOSTICS=ON".into(),
                    // Inherited, named for the preset that uses it.
                    "out/mine".into()
                ),
            ]
        );
        let names: Vec<String> = presets
            .build_and_test_tasks(&Options::default())
            .into_iter()
            .map(|t| t.command)
            .collect();
        assert_eq!(
            names,
            [
                "cmake --build --preset debug",
                "ctest --test-dir out/debug --output-on-failure",
                "ctest --test-dir rel --output-on-failure",
                "ctest --test-dir out/mine --output-on-failure",
            ]
        );
    }

    #[test]
    fn the_configured_folder_is_the_one_with_compile_commands_on() {
        let presets = Presets::read(Some(PROJECT.into()), None);
        let read = |path: &str| match path {
            "out/debug/CMakeCache.txt" => Some("CMAKE_BUILD_TYPE:STRING=Debug\n".into()),
            // As CMake stores a -D given without a type.
            "rel/CMakeCache.txt" => Some("CMAKE_EXPORT_COMPILE_COMMANDS:UNINITIALIZED=ON\n".into()),
            _ => None,
        };
        assert_eq!(
            presets.configured(&Options::default(), read),
            Some(("rel".into(), "CMake · release".into()))
        );
        assert_eq!(presets.configured(&Options::default(), |_| None), None);
    }

    #[test]
    fn settings_shape_the_configure() {
        let options = Options {
            folder: "out/".into(),
            build_type: "Release".into(),
            generator: "Ninja Multi-Config".into(),
            cache: vec!["BUILD_TESTING=OFF".into(), "-DNAME:STRING=a b".into()],
        };
        let configure = Presets::read(None, None).configure_tasks(&options);
        assert_eq!(
            configure[0].command,
            "cmake -S . -B out -G \"Ninja Multi-Config\" -DCMAKE_BUILD_TYPE=Release \
             -DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_COLOR_DIAGNOSTICS=ON \
             -DBUILD_TESTING=OFF \"-DNAME:STRING=a b\""
        );
        assert_eq!(configure[0].dir, "out");
        // With presets, only the cache variables come along.
        let presets = Presets::read(Some(PROJECT.into()), None);
        assert!(
            presets.configure_tasks(&options)[0]
                .command
                .ends_with("-DBUILD_TESTING=OFF \"-DNAME:STRING=a b\"")
        );
    }
}
