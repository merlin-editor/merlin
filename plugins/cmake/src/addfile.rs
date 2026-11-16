//! Adding a new source file to a target, as Qt Creator does: which targets
//! to offer, nearest folder first, and where in the command that adds the
//! target the file goes.

use crate::fileapi::Target;

/// The extensions of files a target compiles.
const SOURCES: &[&str] = &["c", "cc", "cpp", "cxx", "c++", "m", "mm", "cu"];

/// Whether `path` is a file a target would compile.
pub fn is_source(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| SOURCES.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Up to three targets for a new file, those whose folder holds it first,
/// deepest first; none if a target has it already.
pub fn candidates<'a>(targets: &'a [Target], path: &str) -> Vec<&'a Target> {
    if targets.iter().any(|t| t.sources.iter().any(|s| s == path)) {
        return Vec::new();
    }
    let holds =
        |t: &Target| t.dir == "." || path.starts_with(&format!("{}/", t.dir.trim_end_matches('/')));
    let mut found: Vec<&Target> = targets.iter().filter(|t| t.defined.is_some()).collect();
    found.sort_by_key(|t| {
        let depth = if holds(t) {
            t.dir.split('/').count() as i64
        } else {
            -1
        };
        (std::cmp::Reverse(depth), t.name.clone())
    });
    found.truncate(3);
    found
}

/// Where to put `file` in `text`, the CMakeLists.txt whose `line`, from
/// one, adds the target: before the command's closing `)`, as a line and
/// a UTF-16 column, and what to put there: the file relative to `dir`,
/// the CMakeLists.txt's folder.
pub fn insertion(text: &str, line: u32, file: &str, dir: &str) -> Option<((u32, u32), String)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let start = line.checked_sub(1)? as usize;
    // From the command's `(` to its matching `)`, past strings and
    // comments.
    let mut depth = 0;
    let mut quoted = false;
    for (n, l) in lines.iter().enumerate().skip(start) {
        let chars: Vec<char> = l.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if quoted {
                if c == '\\' {
                    i += 1;
                } else if c == '"' {
                    quoted = false;
                }
            } else {
                match c {
                    '"' => quoted = true,
                    '#' => break,
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            let column: u32 = chars[..i].iter().map(|c| c.len_utf16() as u32).sum();
                            let relative = if dir == "." || dir.is_empty() {
                                file.to_string()
                            } else {
                                file.strip_prefix(&format!("{}/", dir.trim_end_matches('/')))
                                    .unwrap_or(file)
                                    .to_string()
                            };
                            // On a line of its own after a multi-line list.
                            let before = chars[..i].iter().collect::<String>();
                            let text = if before.trim().is_empty() && n > start {
                                format!("    {relative}\n{before}")
                            } else {
                                format!(" {relative}")
                            };
                            let column = if before.trim().is_empty() && n > start {
                                0
                            } else {
                                column
                            };
                            return Some(((n as u32, column), text));
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, dir: &str, sources: &[&str]) -> Target {
        Target {
            name: name.into(),
            kind: "STATIC_LIBRARY".into(),
            artifact: None,
            sources: sources.iter().map(|s| s.to_string()).collect(),
            folder: None,
            dir: dir.into(),
            defined: Some((format!("{dir}/CMakeLists.txt"), 1)),
        }
    }

    #[test]
    fn sources_and_the_targets_offered_for_them() {
        assert!(is_source("libs/render/gl.cpp") && !is_source("README.md"));
        let targets = [
            target("app", "apps", &["apps/main.cpp"]),
            target("render", "libs/render", &["libs/render/render.cpp"]),
            target("top", ".", &[]),
        ];
        let names = |path: &str| -> Vec<String> {
            candidates(&targets, path)
                .iter()
                .map(|t| t.name.clone())
                .collect()
        };
        assert_eq!(names("libs/render/gl.cpp"), ["render", "top", "app"]);
        assert!(names("apps/main.cpp").is_empty(), "app has it already");
    }

    #[test]
    fn the_file_goes_before_the_commands_closing_paren() {
        let text = "project(x)\nadd_library(render SHARED render.cpp)\n";
        assert_eq!(
            insertion(text, 2, "libs/render/gl.cpp", "libs/render"),
            Some(((1, 36), " gl.cpp".into()))
        );
        let listed = "add_executable(app\n    main.cpp\n    \"a (b).cpp\"\n)\n";
        assert_eq!(
            insertion(listed, 1, "apps/extra.cpp", "apps"),
            Some(((3, 0), "    extra.cpp\n".into())),
            "a closing paren on its own line keeps the list's shape"
        );
    }
}
