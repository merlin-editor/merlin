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

/// The configure command adds this, so every build has the file clangd
/// reads.
const EXPORT: &str = "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON";

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
    pub fn configure_tasks(&self) -> Vec<ConfigureTask> {
        if self.configure.is_empty() {
            return vec![ConfigureTask {
                name: "CMake: Configure".into(),
                command: format!("cmake -S . -B build {EXPORT}"),
                dir: "build".into(),
                from: "CMake · build".into(),
                detail: "build/".into(),
            }];
        }
        Self::visible(&self.configure)
            .filter_map(|p| {
                let dir = self.binary_dir(p)?;
                Some(ConfigureTask {
                    name: format!("CMake: Configure {}", p.display),
                    command: format!("cmake --preset {} {EXPORT}", quote(&p.name)),
                    from: format!("CMake · {}", p.display),
                    detail: p.file.into(),
                    dir,
                })
            })
            .collect()
    }

    /// Build and test tasks: the build and test presets, else one of each
    /// per configure preset's folder, else `build/`'s.
    pub fn build_and_test_tasks(&self) -> Vec<CommandTask> {
        let mut tasks = Vec::new();
        let folders: Vec<(String, String, &'static str)> = if self.configure.is_empty() {
            vec![("build".into(), String::new(), "build/")]
        } else {
            Self::visible(&self.configure)
                .filter_map(|p| Some((self.binary_dir(p)?, format!(" {}", p.display), p.file)))
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
    pub fn configured(&self, read: impl Fn(&str) -> Option<String>) -> Option<(String, String)> {
        self.configure_tasks().into_iter().find_map(|t| {
            let cache = read(&format!("{}/CMakeCache.txt", t.dir.trim_end_matches('/')))?;
            cache
                .lines()
                .any(|l| l.trim() == "CMAKE_EXPORT_COMPILE_COMMANDS:BOOL=ON")
                .then_some((t.dir, t.from))
        })
    }
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
        let configure = presets.configure_tasks();
        assert_eq!(configure.len(), 1);
        assert_eq!(
            configure[0].command,
            "cmake -S . -B build -DCMAKE_EXPORT_COMPILE_COMMANDS=ON"
        );
        assert_eq!(configure[0].dir, "build");
        let others: Vec<(String, String)> = presets
            .build_and_test_tasks()
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
            .configure_tasks()
            .into_iter()
            .map(|t| (t.name, t.command, t.dir))
            .collect();
        assert_eq!(
            configure,
            [
                (
                    "CMake: Configure Debug".into(),
                    "cmake --preset debug -DCMAKE_EXPORT_COMPILE_COMMANDS=ON".into(),
                    "out/debug".into()
                ),
                (
                    "CMake: Configure release".into(),
                    "cmake --preset release -DCMAKE_EXPORT_COMPILE_COMMANDS=ON".into(),
                    "rel".into()
                ),
                (
                    "CMake: Configure mine".into(),
                    "cmake --preset mine -DCMAKE_EXPORT_COMPILE_COMMANDS=ON".into(),
                    // Inherited, named for the preset that uses it.
                    "out/mine".into()
                ),
            ]
        );
        let names: Vec<String> = presets
            .build_and_test_tasks()
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
            "rel/CMakeCache.txt" => Some("CMAKE_EXPORT_COMPILE_COMMANDS:BOOL=ON\n".into()),
            _ => None,
        };
        assert_eq!(
            presets.configured(read),
            Some(("rel".into(), "CMake · release".into()))
        );
        assert_eq!(presets.configured(|_| None), None);
    }
}
