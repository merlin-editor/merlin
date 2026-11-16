//! CTest's tests: the list `ctest --show-only=json-v1` prints, with the
//! `add_test` line each comes from, and how a run went, from what `ctest`
//! prints as it runs.

use serde_json::Value;

/// A test CTest knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CTest {
    pub name: String,
    /// The file its `add_test` is in, relative to the workspace or
    /// absolute, and the line, from 0.
    pub file: String,
    pub line: u32,
    /// The program and its arguments, for debugging it.
    pub command: Vec<String>,
    /// The folder it runs in, if CTest says.
    pub folder: Option<String>,
}

/// How one test went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed(String),
    Skipped,
}

/// The tests in `ctest --show-only=json-v1`'s output. Paths inside `root`
/// are made relative to it, as the editor names files.
pub fn tests(json: &str, root: &str) -> Vec<CTest> {
    let Ok(json) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let graph = &json["backtraceGraph"];
    let files: Vec<&str> = graph["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let nodes = graph["nodes"].as_array();
    let local = |path: &str| {
        let root = root.trim_end_matches('/');
        match path.strip_prefix(root).and_then(|p| p.strip_prefix('/')) {
            Some(inside) if !root.is_empty() => inside.to_string(),
            _ => path.to_string(),
        }
    };
    json["tests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|test| {
            let name = test["name"].as_str()?.to_string();
            // The node of the `add_test` call: its file and line, from 1.
            let node = test["backtrace"]
                .as_u64()
                .and_then(|i| nodes?.get(i as usize));
            let file = node
                .and_then(|n| files.get(n["file"].as_u64()? as usize))
                .map(|f| local(f))
                .unwrap_or_default();
            let line = node
                .and_then(|n| n["line"].as_u64())
                .map_or(0, |l| l.saturating_sub(1) as u32);
            let command = test["command"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_str().map(String::from))
                .collect();
            let folder = test["properties"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["name"] == "WORKING_DIRECTORY")
                .and_then(|p| p["value"].as_str().map(String::from));
            Some(CTest {
                name,
                file,
                line,
                command,
                folder,
            })
        })
        .collect()
}

/// How each test went in what `ctest` printed, by name, as
/// `1/2 Test #2: fails ....***Failed  0.00 sec`.
pub fn results(output: &str) -> Vec<(String, Outcome)> {
    output
        .lines()
        .filter_map(|line| {
            let rest = line.trim().split_once(" Test ")?.1;
            let rest = rest.trim_start().strip_prefix('#')?;
            let (_, rest) = rest.split_once(": ")?;
            let (name, verdict) = rest.split_once(" .")?;
            let verdict = verdict.trim_start_matches('.').trim_start_matches(' ');
            let verdict = verdict.trim_start_matches('*').trim();
            let outcome = if verdict.starts_with("Passed") {
                Outcome::Passed
            } else if verdict.starts_with("Not Run") || verdict.contains("Disabled") {
                Outcome::Skipped
            } else {
                let why = verdict
                    .split("  ")
                    .next()
                    .unwrap_or(verdict)
                    .trim()
                    .to_string();
                Outcome::Failed(why)
            };
            Some((name.trim().to_string(), outcome))
        })
        .collect()
}

/// A test's name as a regular expression that matches only it, for
/// `ctest -R`.
pub fn exactly(name: &str) -> String {
    let mut pattern = String::from("^");
    for c in name.chars() {
        if "\\.^$|()[]{}*+?".contains(c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('$');
    pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{
        "backtraceGraph": {
            "commands": ["add_test"],
            "files": ["/w/CMakeLists.txt", "/w/tests/CMakeLists.txt"],
            "nodes": [
                {"file": 0},
                {"command": 0, "file": 0, "line": 5, "parent": 0},
                {"file": 1},
                {"command": 0, "file": 1, "line": 2, "parent": 2}
            ]
        },
        "kind": "ctestInfo",
        "tests": [
            {"backtrace": 1, "command": ["/w/build/app"], "name": "unit",
             "properties": [{"name": "WORKING_DIRECTORY", "value": "/w/build"}]},
            {"backtrace": 3, "command": ["/w/build/app", "1"], "name": "fails"}
        ]
    }"#;

    #[test]
    fn tests_come_with_their_add_test_line() {
        let found = tests(LIST, "/w");
        assert_eq!(
            found,
            [
                CTest {
                    name: "unit".into(),
                    file: "CMakeLists.txt".into(),
                    line: 4,
                    command: vec!["/w/build/app".into()],
                    folder: Some("/w/build".into()),
                },
                CTest {
                    name: "fails".into(),
                    file: "tests/CMakeLists.txt".into(),
                    line: 1,
                    command: vec!["/w/build/app".into(), "1".into()],
                    folder: None,
                },
            ]
        );
        assert!(tests("not json", "/w").is_empty());
    }

    #[test]
    fn results_from_what_ctest_prints() {
        let output = "Test project /w/build\n    Start 2: fails\n    Start 1: unit\n\
            1/3 Test #2: fails ............................***Failed    0.00 sec\n\
            2/3 Test #1: unit .............................   Passed    0.00 sec\n\
            3/3 Test #3: slow .............................***Timeout   1.50 sec\n\
            4/4 Test #4: off ..............................***Not Run (Disabled)   0.00 sec\n\
            \n50% tests passed, 1 tests failed out of 2\n";
        assert_eq!(
            results(output),
            [
                ("fails".into(), Outcome::Failed("Failed".into())),
                ("unit".into(), Outcome::Passed),
                ("slow".into(), Outcome::Failed("Timeout".into())),
                ("off".into(), Outcome::Skipped),
            ]
        );
    }

    #[test]
    fn a_name_matches_only_itself() {
        assert_eq!(exactly("unit"), "^unit$");
        assert_eq!(exactly("a.b(c)"), "^a\\.b\\(c\\)$");
    }
}
