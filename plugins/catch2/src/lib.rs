//! Catch2 testing plugin for C++.
//!
//! Discovers Catch2 test macros (`TEST_CASE`, `SCENARIO`, `TEMPLATE_TEST_CASE`,
//! `TEST_CASE_METHOD`, `SCENARIO_METHOD`) in open C++ files and registers
//! inline test lenses backed by `ctest`.

use merlin_plugin::prelude::*;

const EXTENSIONS: &[&str] = &["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx"];
const MAX_BYTES: usize = 512 * 1024;

const MACROS: &[(&str, usize)] = &[
    ("TEST_CASE", 0),
    ("SCENARIO", 0),
    ("TEMPLATE_TEST_CASE", 0),
    ("TEST_CASE_METHOD", 1),
    ("SCENARIO_METHOD", 1),
];

#[derive(Default)]
struct Catch2 {
    last: Option<(String, Option<u64>)>,
}

fn active_cpp_path() -> Option<String> {
    let path = host::active_path().ok()??;
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    if !EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }
    let abs = if path.starts_with('/') {
        path
    } else {
        format!("{}/{path}", host::workspace_root())
    };
    Some(abs)
}

/// Scans `text` for Catch2 test macros and returns their test items.
pub fn parse_tests(text: &str) -> Vec<TestItem> {
    let mut tests = Vec::new();
    for (line_idx, raw_line) in text.lines().enumerate() {
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }
        let Some((macro_name, arg_index, args)) = match_macro(trimmed) else {
            continue;
        };
        let Some(name) = nth_string_arg(args, arg_index) else {
            continue;
        };
        let label = if macro_name.starts_with("SCENARIO") {
            format!("Scenario: {name}")
        } else {
            name.clone()
        };
        let command = format!("ctest --output-on-failure -R {}", shell_quote(&name));
        tests.push(TestItem {
            id: name,
            label,
            line: line_idx as u32,
            command: Some(command),
        });
    }
    tests
}

fn match_macro(line: &str) -> Option<(&'static str, usize, &str)> {
    for &(name, arg_idx) in MACROS {
        if let Some(rest) = line.strip_prefix(name) {
            let rest = rest.trim_start();
            if let Some(inside) = rest.strip_prefix('(') {
                return Some((name, arg_idx, inside));
            }
        }
    }
    None
}

fn nth_string_arg(mut s: &str, mut index: usize) -> Option<String> {
    while index > 0 {
        let comma = s.find(',')?;
        s = &s[comma + 1..];
        index -= 1;
    }
    let quote = s.find('"')?;
    let after = &s[quote + 1..];
    let mut out = String::new();
    let mut escaped = false;
    for ch in after.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return (!out.is_empty()).then_some(out);
        } else {
            out.push(ch);
        }
    }
    None
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

impl Plugin for Catch2 {
    fn buffer(&mut self, event: BufferEvent) {
        if matches!(event, BufferEvent::Saved(_)) {
            return;
        }
        let Some(path) = active_cpp_path() else {
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
        let tests = parse_tests(&text);
        host::set_tests(&path, &tests);
        self.last = Some((path, version));
    }
}

merlin_plugin::export!(Catch2);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_catch2_test_macros() {
        let src = r#"
#include <catch2/catch_test_macros.hpp>

// TEST_CASE("commented out", "[skip]")
TEST_CASE("vectors can be sized and resized", "[vector]") {
    REQUIRE(1 == 1);
}

SCENARIO("vectors can be emptied", "[vector]") {
    GIVEN("A vector with some items") {}
}

TEST_CASE_METHOD(DatabaseFixture, "insert row succeeds", "[db]") {
    REQUIRE(true);
}
"#;
        let items = parse_tests(src);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, "vectors can be sized and resized");
        assert_eq!(items[0].line, 4);
        assert_eq!(
            items[0].command.as_deref(),
            Some("ctest --output-on-failure -R 'vectors can be sized and resized'")
        );
        assert_eq!(items[1].label, "Scenario: vectors can be emptied");
        assert_eq!(items[1].line, 8);
        assert_eq!(items[2].id, "insert row succeeds");
        assert_eq!(items[2].line, 12);
    }
}
