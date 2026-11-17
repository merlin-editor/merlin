//! Settings › Files' Exclude kept out of clangd's background index, when
//! the C/C++ setting asks: a block in the project's `.clangd` between
//! merlin's own comment lines, the rest of the file left alone. clangd
//! can't be told what to skip on its command line, only there.

use merlin_plugin::{LogLevel, host};

const START: &str = "# merlin: Settings › Files' Exclude, kept out of clangd's index. merlin rewrites the lines up to its end line.";
const END: &str = "# merlin: end";

/// A pattern as a regular expression for clangd's `PathMatch`, which
/// matches paths from the `.clangd`'s folder: a folder's matches what's
/// in it, and one with no other `/` matches at any depth.
pub fn regex_for(pattern: &str) -> Option<String> {
    let p = pattern.trim().trim_start_matches("./");
    if p.is_empty() || p.starts_with('#') {
        return None;
    }
    let folder = p.ends_with('/');
    let p = p.trim_end_matches('/');
    let anchored = p.starts_with('/');
    let p = p.trim_start_matches('/');
    if p.is_empty() {
        return None;
    }
    let glob = if anchored || p.contains('/') {
        p.to_string()
    } else {
        format!("**/{p}")
    };
    let mut out = String::new();
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    out.push_str("(.*/)?");
                } else {
                    out.push_str(".*");
                }
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            c if "\\.+()|^${}[]".contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push_str(if folder { "/.*" } else { "(/.*)?" });
    Some(out)
}

/// `text` with merlin's block for `patterns` in place of any it had, or
/// without one for none.
pub fn with_block(text: &str, patterns: &[String]) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim_end() == START {
            inside = true;
        } else if inside && line.trim_end() == END {
            inside = false;
        } else if !inside {
            kept.push(line);
        }
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    let mut out = kept.join("\n");
    let regexes: Vec<String> = patterns.iter().filter_map(|p| regex_for(p)).collect();
    if !regexes.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        // Its own fragment, with `---` either side, whatever's around it.
        let quoted: Vec<String> = regexes
            .iter()
            .map(|r| format!("'{}'", r.replace('\'', "''")))
            .collect();
        out.push_str(&format!(
            "{START}\n---\nIf:\n  PathMatch: [{}]\nIndex:\n  Background: Skip\n---\n{END}",
            quoted.join(", ")
        ));
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Writes the block as the setting and the exclusions say, if that
/// changes the file.
pub fn sync() {
    let on = merlin_plugin::settings::flag("index-excluded").unwrap_or(false);
    let patterns = if on {
        host::excluded_patterns()
    } else {
        Vec::new()
    };
    let old = host::read_file(".clangd").unwrap_or_default();
    let new = with_block(&old, &patterns);
    if new == old || (old.is_empty() && new.is_empty()) {
        return;
    }
    match host::write_file(".clangd", &new) {
        Ok(()) if patterns.is_empty() => {
            host::log(LogLevel::Info, "Took merlin's exclusions out of .clangd")
        }
        Ok(()) => host::notice(
            LogLevel::Info,
            &format!(
                "Changed .clangd: clangd won't index {}",
                patterns.join(", ")
            ),
            &[],
        ),
        Err(err) => host::notice(
            LogLevel::Warn,
            &format!("Couldn't change .clangd: {err}"),
            &[],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_is_added_replaced_and_taken_out() {
        let user = "CompileFlags:\n  Add: [-Wall]\n";
        let one = with_block(user, &["third_party/".into()]);
        assert!(one.starts_with(user));
        assert!(one.contains("PathMatch: ['(.*/)?third_party/.*']"));
        assert!(one.contains("Background: Skip"));
        let two = with_block(&one, &["build/_deps/".into(), "*.o".into()]);
        assert_eq!(two.matches(START).count(), 1);
        assert!(two.contains("'build/_deps/.*', '(.*/)?[^/]*\\.o(/.*)?'"));
        assert_eq!(with_block(&two, &[]), user);
        assert_eq!(with_block("", &[]), "");
    }
}
