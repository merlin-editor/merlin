//! Cargo's side of Rust: the project's crates, the tests in their files,
//! the command lines that run a crate's, a file's or a test's, and what
//! cargo test prints, read back into how each test went.

use std::collections::HashMap;

/// How a test went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    Skipped,
}

/// The tests in Rust source: functions marked `#[test]`, or a test
/// attribute such as `#[tokio::test]`, with the line of each `fn`.
pub fn rust_tests(text: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut marked = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("#[") {
            let attr = t.trim_start_matches("#[");
            if attr.starts_with("test]") || attr.contains("::test") {
                marked = true;
            }
            continue;
        }
        if !marked || t.is_empty() || t.starts_with("//") {
            continue;
        }
        marked = false;
        let Some(at) = t.find("fn ") else { continue };
        let name: String = t[at + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.push((name, i as u32));
        }
    }
    out
}

/// A Cargo.toml's package name.
pub fn package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "name"
        {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Whether a Cargo.toml is a workspace's, and whether it's a package's.
pub fn manifest_kind(text: &str) -> (bool, bool) {
    let sections: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('['))
        .collect();
    (
        sections.contains(&"[workspace]"),
        sections.contains(&"[package]"),
    )
}

/// The tasks a Cargo.toml offers, as name, command line and group: test,
/// build and clippy, across the workspace when it is one, and run when
/// it's a package. Test runs in the plugin, so its results are read.
pub fn cargo_tasks(text: &str) -> Vec<(String, Option<String>, &'static str)> {
    let (workspace, package) = manifest_kind(text);
    if !workspace && !package {
        return Vec::new();
    }
    let all = if workspace { " --workspace" } else { "" };
    let mut tasks = vec![
        (format!("cargo test{all}"), None, "test"),
        (
            "cargo build --release".to_string(),
            Some("cargo build --release".to_string()),
            "build",
        ),
    ];
    if package {
        tasks.push(("cargo run".into(), Some("cargo run".into()), "build"));
    }
    let clippy = format!("cargo clippy{all}");
    tasks.push((clippy.clone(), Some(clippy), "build"));
    tasks
}

/// A file with tests, in its crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestFile {
    /// Its path in the project.
    pub path: String,
    pub krate: String,
    /// Its path in its crate, such as `tests/find.rs`.
    pub label: String,
    /// Each test's name and the line of its `fn`.
    pub tests: Vec<(String, u32)>,
}

/// A test's id: its file and name.
pub fn test_id(path: &str, name: &str) -> String {
    format!("{path}::{name}")
}

/// The crate a file is in, by the crates' folders and names, deepest
/// first: its name and the file's path in it.
pub fn crate_of<'a>(crates: &'a [(String, String)], path: &str) -> Option<(&'a str, String)> {
    crates.iter().find_map(|(dir, name)| {
        let label = match dir.is_empty() {
            true => path.to_string(),
            false => path.strip_prefix(&format!("{dir}/"))?.to_string(),
        };
        Some((name.as_str(), label))
    })
}

/// The crate's target for a file, as cargo's arguments: an integration
/// test's own, or the crate's unit tests with the file's tests named.
fn target(file: &TestFile, names: &[&str]) -> String {
    match file
        .label
        .strip_prefix("tests/")
        .and_then(|f| f.strip_suffix(".rs"))
    {
        Some(stem) if !stem.contains('/') => {
            let filter = names.join(" ");
            format!(" --test {stem} {filter}").trim_end().to_string()
        }
        _ => format!(" -- {}", names.join(" ")),
    }
}

/// Runs a file's tests: an integration test's file whole, a unit test
/// file's tests by name.
pub fn file_command(file: &TestFile) -> String {
    let names: Vec<&str> = match file.label.starts_with("tests/") {
        true => Vec::new(),
        false => file.tests.iter().map(|(n, _)| n.as_str()).collect(),
    };
    format!("cargo test -p {}{}", file.krate, target(file, &names))
}

/// Runs one test.
pub fn test_command(file: &TestFile, name: &str) -> String {
    format!("cargo test -p {}{}", file.krate, target(file, &[name]))
}

/// Builds a test's binary without running it, for the debugger.
pub fn build_command(file: &TestFile) -> String {
    let only = match file
        .label
        .strip_prefix("tests/")
        .and_then(|f| f.strip_suffix(".rs"))
    {
        Some(stem) if !stem.contains('/') => format!(" --test {stem}"),
        _ => String::new(),
    };
    format!("cargo test --no-run -p {}{only}", file.krate)
}

/// What one test run printed, as cargo and libtest print it.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// Each test: the target it's in (`tests/find.rs`, or `src/lib.rs`
    /// for unit tests), the crate a unit target names, its name, and how
    /// it went.
    pub tests: Vec<(String, Option<String>, String, Outcome)>,
    /// Each failure's message and where it failed, by test name.
    pub failures: HashMap<String, (String, Option<(String, u32)>)>,
    /// Each target's time, in seconds.
    pub times: Vec<(String, Option<String>, f64)>,
}

/// Reads cargo test's output.
pub fn parse_report(text: &str) -> Report {
    let mut report = Report::default();
    let mut target = String::new();
    let mut unit: Option<String> = None;
    let mut failing: Option<String> = None;
    let mut message: Vec<String> = Vec::new();
    let mut at: Option<(String, u32)> = None;
    let mut panicked = false;
    let finish = |report: &mut Report,
                  failing: &mut Option<String>,
                  message: &mut Vec<String>,
                  at: &mut Option<(String, u32)>| {
        if let Some(name) = failing.take() {
            let lines: Vec<String> = std::mem::take(message);
            // An assertion's left and right say more than its first line.
            let sides: Vec<&String> = lines
                .iter()
                .filter(|l| l.starts_with("left:") || l.starts_with("right:"))
                .collect();
            let text = if sides.is_empty() {
                lines.join(" ")
            } else {
                sides
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            report.failures.insert(name, (text, at.take()));
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Running ") {
            // `Running unittests src/lib.rs (target/debug/deps/merlin_core-…)`
            // or `Running tests/find.rs (target/debug/deps/find-…)`.
            let rest = rest.strip_prefix("unittests ").unwrap_or(rest);
            let (path, bin) = rest.split_once(" (").unwrap_or((rest, ""));
            target = path.to_string();
            unit = (!path.starts_with("tests/")).then(|| {
                let file = bin.rsplit('/').next().unwrap_or(bin);
                file.rsplit_once('-')
                    .map_or(file, |(name, _)| name)
                    .to_string()
            });
            continue;
        }
        if line.starts_with("Doc-tests ") {
            target.clear();
            continue;
        }
        if let Some(rest) = line.strip_prefix("test ")
            && let Some((name, how)) = rest.rsplit_once(" ... ")
            && !target.is_empty()
        {
            let outcome = match how {
                "ok" => Outcome::Passed,
                "FAILED" => Outcome::Failed,
                _ if how.starts_with("ignored") => Outcome::Skipped,
                _ => continue,
            };
            report
                .tests
                .push((target.clone(), unit.clone(), name.to_string(), outcome));
            continue;
        }
        if let Some(rest) = line.strip_prefix("test result: ")
            && let Some(secs) = rest
                .rsplit_once("finished in ")
                .and_then(|(_, t)| t.trim_end_matches('s').parse::<f64>().ok())
            && !target.is_empty()
        {
            report.times.push((target.clone(), unit.clone(), secs));
            continue;
        }
        if let Some(name) = line
            .strip_prefix("---- ")
            .and_then(|l| l.strip_suffix(" stdout ----"))
        {
            finish(&mut report, &mut failing, &mut message, &mut at);
            failing = Some(name.to_string());
            panicked = false;
            continue;
        }
        // Without its output captured, a test's panic is printed as it
        // happens, under no heading of its own.
        if failing.is_none()
            && line.contains("panicked at ")
            && let Some(name) = line
                .strip_prefix("thread '")
                .and_then(|l| l.split_once('\''))
                .map(|(name, _)| name)
        {
            failing = Some(name.to_string());
        }
        if failing.is_none() {
            continue;
        }
        // The message ends at the note about backtraces, or the list of
        // failures.
        if line == "failures:" || line.starts_with("note: run with") {
            finish(&mut report, &mut failing, &mut message, &mut at);
            panicked = false;
            continue;
        }
        if let Some(rest) = line.split_once("panicked at ").map(|(_, r)| r) {
            // `crates/core/tests/saving.rs:28:5:`
            let mut parts = rest.trim_end_matches(':').rsplitn(3, ':');
            let (_col, line_no, path) = (parts.next(), parts.next(), parts.next());
            if let (Some(path), Some(n)) = (path, line_no.and_then(|n| n.parse::<u32>().ok())) {
                at = Some((path.to_string(), n.saturating_sub(1)));
            }
            panicked = true;
            continue;
        }
        if panicked && !line.is_empty() {
            message.push(line.to_string());
        }
    }
    finish(&mut report, &mut failing, &mut message, &mut at);
    report
}

/// The test a line of cargo's output names, among `files`: its file, and
/// the crate it's in. `krate` is the crate the run was for, if one.
pub fn match_test<'a>(
    files: &'a [TestFile],
    target: &str,
    unit: Option<&str>,
    krate: Option<&str>,
    name: &str,
) -> Option<&'a TestFile> {
    let short = name.rsplit("::").next().unwrap_or(name);
    let modules: Vec<&str> = name.split("::").collect();
    let mut best: Option<(usize, &TestFile)> = None;
    for file in files {
        let snake = file.krate.replace('-', "_");
        if unit.is_some_and(|u| u != snake) || krate.is_some_and(|k| k != file.krate) {
            continue;
        }
        let fits = match unit {
            Some(_) => !file.label.starts_with("tests/"),
            None => file.label == target,
        };
        if !fits || !file.tests.iter().any(|(t, _)| t == short) {
            continue;
        }
        // The file whose path has most of the test's modules.
        let score = modules.iter().filter(|m| file.label.contains(*m)).count();
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, file));
        }
    }
    best.map(|(_, file)| file)
}

/// The test binaries cargo's `--no-run` printed, as source and binary:
/// `Executable unittests src/lib.rs (target/debug/deps/x-1a2b)`. A
/// terminal may have wrapped the line at its width, so the binary runs on
/// to its closing bracket across lines.
pub fn executables(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("Executable ") {
        rest = &rest[at + "Executable ".len()..];
        let Some(close) = rest.find(')') else { break };
        let entry: String = rest[..close]
            .chars()
            .filter(|c| *c != '\n' && *c != '\r')
            .collect();
        rest = &rest[close..];
        let entry = entry.strip_prefix("unittests ").unwrap_or(&entry);
        if let Some((src, bin)) = entry.split_once(" (") {
            found.push((src.trim().to_string(), bin.trim().to_string()));
        }
    }
    found
}

/// The binary for a test in `label`'s file: the file's own target, or the
/// crate's library, then its binary.
pub fn binary_for<'a>(binaries: &'a [(String, String)], label: &str) -> Option<&'a str> {
    binaries
        .iter()
        .find(|(src, _)| src == label)
        .or_else(|| match label.starts_with("tests/") {
            true => None,
            false => ["src/lib.rs", "src/main.rs"]
                .iter()
                .find_map(|want| binaries.iter().find(|(src, _)| src == want)),
        })
        .or(binaries.first())
        .map(|(_, bin)| bin.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "\
❯ cargo test -p demo --test saving
   Compiling demo v0.1.0
     Running tests/saving.rs (target/debug/deps/saving-170921b8c002b1d9)

running 2 tests
test auto_save_after_a_delay ... ok
test trims_on_save ... FAILED

failures:

---- trims_on_save stdout ----

thread 'trims_on_save' (251039) panicked at crates/demo/tests/saving.rs:6:5:
assertion `left == right` failed
  left: \"a\\nb\"
 right: \"a\\nb\\n\"
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    trims_on_save

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.40s
";

    fn saving() -> TestFile {
        TestFile {
            path: "crates/demo/tests/saving.rs".into(),
            krate: "demo".into(),
            label: "tests/saving.rs".into(),
            tests: vec![
                ("trims_on_save".into(), 3),
                ("auto_save_after_a_delay".into(), 9),
            ],
        }
    }

    #[test]
    fn tests_are_the_functions_marked_as_tests() {
        let text = "#[test]\nfn a() {}\n\n#[tokio::test]\nasync fn b() {}\n\nfn c() {}\n";
        assert_eq!(
            rust_tests(text),
            [("a".to_string(), 1), ("b".to_string(), 4)]
        );
    }

    #[test]
    fn cargos_tasks_span_the_workspace_and_run_a_package() {
        let names: Vec<String> = cargo_tasks("[package]\nname = \"x\"\n")
            .into_iter()
            .map(|(n, ..)| n)
            .collect();
        assert_eq!(
            names,
            [
                "cargo test",
                "cargo build --release",
                "cargo run",
                "cargo clippy"
            ]
        );
        let ws = cargo_tasks("[workspace]\nmembers = []\n");
        assert_eq!(ws[0].0, "cargo test --workspace");
        assert_eq!(
            ws[0].1, None,
            "the plugin runs tests, to read how they went"
        );
    }

    #[test]
    fn a_tests_command_names_its_target() {
        let file = saving();
        assert_eq!(
            test_command(&file, "trims_on_save"),
            "cargo test -p demo --test saving trims_on_save"
        );
        assert_eq!(file_command(&file), "cargo test -p demo --test saving");
        assert_eq!(
            build_command(&file),
            "cargo test --no-run -p demo --test saving"
        );
        let unit = TestFile {
            path: "crates/demo/src/lib.rs".into(),
            krate: "demo".into(),
            label: "src/lib.rs".into(),
            tests: vec![("unit".into(), 4)],
        };
        assert_eq!(file_command(&unit), "cargo test -p demo -- unit");
    }

    #[test]
    fn a_runs_output_says_how_each_test_went_and_why_one_failed() {
        let report = parse_report(OUTPUT);
        assert_eq!(report.tests.len(), 2);
        assert_eq!(report.tests[1].3, Outcome::Failed);
        let (message, at) = &report.failures["trims_on_save"];
        assert_eq!(message, "left: \"a\\nb\"  right: \"a\\nb\\n\"");
        assert_eq!(at, &Some(("crates/demo/tests/saving.rs".to_string(), 5)));
        assert_eq!(report.times, [("tests/saving.rs".to_string(), None, 0.4)]);
        let files = [saving()];
        let (target, unit, name, _) = &report.tests[1];
        let file = match_test(&files, target, unit.as_deref(), Some("demo"), name);
        assert_eq!(
            file.map(|f| f.path.as_str()),
            Some("crates/demo/tests/saving.rs")
        );
    }

    #[test]
    fn a_run_without_capturing_says_why_it_failed_too() {
        let text = "\
     Running tests/saving.rs (target/debug/deps/saving-17)

running 1 test
thread 'trims_on_save' (5) panicked at crates/demo/tests/saving.rs:6:5:
assertion `left == right` failed
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test trims_on_save ... FAILED

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
";
        let report = parse_report(text);
        assert_eq!(report.failures["trims_on_save"].0, "left: 1  right: 2");
    }

    #[test]
    fn a_binary_wrapped_by_the_terminal_is_joined() {
        let text = "  Executable unittests src/main.rs (target/debug/deps/dbgdemo-79f066730ab665\nf5)\n  Executable tests/find.rs (target/debug/deps/find-12)\n";
        let found = executables(text);
        assert_eq!(
            found,
            [
                (
                    "src/main.rs".to_string(),
                    "target/debug/deps/dbgdemo-79f066730ab665f5".to_string()
                ),
                (
                    "tests/find.rs".to_string(),
                    "target/debug/deps/find-12".to_string()
                ),
            ]
        );
        assert_eq!(
            binary_for(&found, "tests/find.rs"),
            Some("target/debug/deps/find-12")
        );
        assert_eq!(
            binary_for(&found, "src/parse.rs"),
            Some("target/debug/deps/dbgdemo-79f066730ab665f5")
        );
    }
}
