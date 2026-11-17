//! The CMake projects in a workspace: each `CMakeLists.txt` with a
//! `project()` call that no other one reaches through `add_subdirectory`,
//! so a project with subfolders is one project, and folders that hold
//! projects of their own (CMake Tools #1374) are several.

use std::collections::BTreeSet;

/// A project: its folder, relative to the workspace, empty at the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub dir: String,
    /// The folder's name, or the `project()` name at the top.
    pub name: String,
}

/// `path` in the project folder `dir`, relative to the workspace.
pub fn join(dir: &str, path: &str) -> String {
    let absolute = path.starts_with('/') || matches!(path.get(1..3), Some(":/" | ":\\"));
    if absolute || dir.is_empty() || dir == "." {
        path.to_string()
    } else if path.is_empty() || path == "." {
        dir.to_string()
    } else {
        format!("{}/{path}", dir.trim_end_matches('/'))
    }
}

/// `a/b/../c` as `a/c`; none if it climbs out of the workspace.
fn normal(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The calls of `command` in a CMake file, each with its first argument,
/// skipping comments and strings around them.
fn first_arguments(text: &str, command: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in text.lines() {
        let code = line.split('#').next().unwrap_or_default();
        let lower = code.to_ascii_lowercase();
        let mut from = 0;
        while let Some(at) = lower[from..].find(command) {
            let start = from + at;
            from = start + command.len();
            // A whole word: not `my_project(` for `project`.
            let before = lower[..start].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let rest = code[from..].trim_start();
            let Some(args) = rest.strip_prefix('(') else {
                continue;
            };
            let arg = args.trim_start();
            let word = if let Some(quoted) = arg.strip_prefix('"') {
                quoted.split('"').next().unwrap_or_default()
            } else {
                arg.split(|c: char| c.is_whitespace() || c == ')')
                    .next()
                    .unwrap_or_default()
            };
            found.push(word.to_string());
        }
    }
    found
}

/// The name a `project()` call gives, if the file has one.
pub fn project_name(text: &str) -> Option<String> {
    first_arguments(text, "project")
        .into_iter()
        .find(|n| !n.is_empty())
}

/// The folders `add_subdirectory` adds, relative to the file's folder,
/// leaving out any made from variables.
pub fn subdirectories(text: &str) -> Vec<String> {
    first_arguments(text, "add_subdirectory")
        .into_iter()
        .filter(|d| !d.is_empty() && !d.contains("${"))
        .collect()
}

/// Whether a folder is one CMake builds into, or fetches into, rather
/// than the project's own.
fn is_build(dir: &str, builds: &BTreeSet<String>) -> bool {
    dir.split('/').any(|p| p == "_deps" || p == "CMakeFiles")
        || builds
            .iter()
            .any(|b| dir == b || dir.starts_with(&format!("{b}/")))
}

/// The projects among the workspace's `files`, the top one first, then by
/// folder. `read` reads a file.
pub fn find(files: &[String], read: impl Fn(&str) -> Option<String>) -> Vec<Project> {
    let folder = |path: &str| path.rsplit_once('/').map_or(String::new(), |(d, _)| d.to_string());
    let builds: BTreeSet<String> = files
        .iter()
        .filter(|f| f.rsplit('/').next() == Some("CMakeCache.txt"))
        .map(|f| folder(f))
        .collect();
    let lists: Vec<String> = files
        .iter()
        .filter(|f| f.rsplit('/').next() == Some("CMakeLists.txt"))
        .map(|f| folder(f))
        .filter(|d| !is_build(d, &builds))
        .collect();
    let all: BTreeSet<&str> = lists.iter().map(String::as_str).collect();
    let texts: Vec<(String, String)> = lists
        .iter()
        .filter_map(|d| Some((d.clone(), read(&join(d, "CMakeLists.txt"))?)))
        .collect();
    let text_of = |d: &str| texts.iter().find(|(x, _)| x == d).map(|(_, t)| t.as_str());
    let candidates: Vec<(&str, String)> = texts
        .iter()
        .filter_map(|(d, t)| Some((d.as_str(), project_name(t)?)))
        .collect();
    // What each candidate reaches through add_subdirectory, however deep.
    let mut reached: BTreeSet<String> = BTreeSet::new();
    for (start, _) in &candidates {
        let mut stack = vec![start.to_string()];
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Some(dir) = stack.pop() {
            if !seen.insert(dir.clone()) {
                continue;
            }
            let Some(text) = text_of(&dir) else {
                continue;
            };
            for sub in subdirectories(text) {
                let Some(next) = normal(&join(&dir, &sub)) else {
                    continue;
                };
                if all.contains(next.as_str()) && next != *start {
                    reached.insert(next.clone());
                    stack.push(next);
                }
            }
        }
    }
    let mut projects: Vec<Project> = candidates
        .into_iter()
        .filter(|(d, _)| !reached.contains(*d))
        .map(|(d, name)| Project {
            dir: d.to_string(),
            name: match d.rsplit('/').next() {
                Some(last) if !d.is_empty() => last.to_string(),
                _ => name,
            },
        })
        .collect();
    projects.sort_by(|a, b| (!a.dir.is_empty(), &a.dir).cmp(&(!b.dir.is_empty(), &b.dir)));
    projects
}

/// The project whose folder holds `path`, the deepest one.
pub fn holding<'a>(projects: &'a [Project], path: &str) -> Option<&'a Project> {
    projects
        .iter()
        .filter(|p| p.dir.is_empty() || path == p.dir || path.starts_with(&format!("{}/", p.dir)))
        .max_by_key(|p| p.dir.len() + usize::from(!p.dir.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn find_in(files: &[(&str, &str)]) -> Vec<(String, String)> {
        let map: BTreeMap<String, String> = files
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect();
        let names: Vec<String> = map.keys().cloned().collect();
        find(&names, |p| map.get(p).cloned())
            .into_iter()
            .map(|p| (p.dir, p.name))
            .collect()
    }

    #[test]
    fn folders_with_projects_of_their_own() {
        let found = find_in(&[
            ("firmware/CMakeLists.txt", "project(fw C)\nadd_subdirectory(drivers)\n"),
            ("firmware/drivers/CMakeLists.txt", "project(drivers C)\n"),
            ("tools/CMakeLists.txt", "cmake_minimum_required(VERSION 3.20)\nPROJECT( tools CXX)\n"),
            ("tools/src/CMakeLists.txt", "add_executable(plot plot.cpp)\n"),
            ("sim/CMakeLists.txt", "project(\"sim\")\n"),
            ("README.md", ""),
        ]);
        assert_eq!(
            found,
            [
                ("firmware".to_string(), "firmware".to_string()),
                ("sim".into(), "sim".into()),
                ("tools".into(), "tools".into()),
            ],
            "drivers is part of firmware, and src has no project()"
        );
    }

    #[test]
    fn one_project_with_subfolders_and_one_it_doesnt_add() {
        let found = find_in(&[
            ("CMakeLists.txt", "project(robot)\nadd_subdirectory(libs) # libs\nadd_subdirectory(${X})\n"),
            ("libs/CMakeLists.txt", "add_subdirectory(../apps)\n"),
            ("apps/CMakeLists.txt", "project(apps)\n"),
            ("bootloader/CMakeLists.txt", "project(boot C)\n"),
            ("build/CMakeCache.txt", ""),
            ("build/_deps/fmt-src/CMakeLists.txt", "project(fmt)\n"),
            ("out/CMakeCache.txt", ""),
            ("out/thing/CMakeLists.txt", "project(thing)\n"),
        ]);
        assert_eq!(
            found,
            [
                (String::new(), "robot".to_string()),
                ("bootloader".into(), "bootloader".into()),
            ],
            "apps is reached through libs, and builds aren't searched"
        );
    }

    #[test]
    fn a_path_is_in_the_deepest_project_holding_it() {
        let projects = [
            Project {
                dir: String::new(),
                name: "robot".into(),
            },
            Project {
                dir: "tools".into(),
                name: "tools".into(),
            },
        ];
        assert_eq!(holding(&projects, "tools/src/plot.cpp").unwrap().name, "tools");
        assert_eq!(holding(&projects, "toolsmith/a.cpp").unwrap().name, "robot");
        assert_eq!(holding(&projects, "README.md").unwrap().name, "robot");
        assert!(holding(&projects[1..], "README.md").is_none());
        assert_eq!(join("tools", "build"), "tools/build");
        assert_eq!(join("", "build"), "build");
        assert_eq!(join("tools", "/abs/build"), "/abs/build");
        assert_eq!(normal("a/b/../c"), Some("a/c".into()));
        assert_eq!(normal("../x"), None);
    }
}
