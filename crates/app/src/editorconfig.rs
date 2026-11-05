//! `.editorconfig` files: what they say about a file, from the file's
//! folder up to one marked `root = true` or the top of the disk.

use std::fs;
use std::path::Path;

use globset::GlobBuilder;
use merlin_core::FileConfig;

/// What the `.editorconfig` files above `file` (an absolute path) say
/// about it. Nearer files win, and later sections in a file win.
pub fn for_file(file: &Path) -> FileConfig {
    // Each folder's file, nearest first, up to a root.
    let mut texts = Vec::new();
    for dir in file.ancestors().skip(1) {
        if let Ok(text) = fs::read_to_string(dir.join(".editorconfig")) {
            let root = is_root(&text);
            texts.push((dir.to_path_buf(), text));
            if root {
                break;
            }
        }
    }
    let mut config = FileConfig::default();
    // Farthest first, so nearer files' values land over them.
    for (dir, text) in texts.iter().rev() {
        let Ok(relative) = file.strip_prefix(dir) else {
            continue;
        };
        apply(&mut config, text, &relative.to_string_lossy());
    }
    config
}

fn is_root(text: &str) -> bool {
    for line in lines(text) {
        if line.starts_with('[') {
            return false;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case("root")
        {
            return v.trim().eq_ignore_ascii_case("true");
        }
    }
    false
}

/// The lines that mean something: trimmed, without blanks or comments.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with(';'))
}

/// Applies the sections of one file whose glob matches `relative`, a
/// path from that file's folder with `/` between parts.
fn apply(config: &mut FileConfig, text: &str, relative: &str) {
    let relative = relative.replace('\\', "/");
    let mut matches = false;
    for line in lines(text) {
        if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            matches = glob_matches(glob, &relative);
            continue;
        }
        if !matches {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_ascii_lowercase();
        // `unset` takes a value back to the settings'.
        let unset = value == "unset";
        let flag = || match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        let number = || value.parse::<usize>().ok().filter(|&n| n > 0);
        match key.as_str() {
            "indent_style" => {
                config.tabs = match value.as_str() {
                    "tab" => Some(true),
                    "space" => Some(false),
                    _ if unset => None,
                    _ => config.tabs,
                }
            }
            // `indent_size = tab` means as wide as a tab.
            "indent_size" if value == "tab" => config.indent_size = config.tab_width,
            "indent_size" => config.indent_size = if unset { None } else { number() },
            "tab_width" => config.tab_width = if unset { None } else { number() },
            "trim_trailing_whitespace" => config.trim_trailing_whitespace = flag(),
            "insert_final_newline" => config.insert_final_newline = flag(),
            _ => {}
        }
    }
}

/// Whether an `.editorconfig` section's glob takes in `path`. A glob
/// without a `/` matches the name in any folder; one with a `/` is from
/// the file's own folder.
fn glob_matches(glob: &str, path: &str) -> bool {
    let glob = if glob.contains('/') {
        glob.trim_start_matches('/').to_string()
    } else {
        format!("**/{glob}")
    };
    // `**` reaches into folders even inside a name, as in `lib/**.rs`,
    // which globset only does for a `**` on its own.
    let glob = glob
        .split("**")
        .collect::<Vec<_>>()
        .iter()
        .enumerate()
        .fold(String::new(), |mut out, (i, part)| {
            if i > 0 {
                out.push_str(if part.is_empty() || part.starts_with('/') {
                    "**"
                } else {
                    "**/*"
                });
            }
            out.push_str(part);
            out
        });
    GlobBuilder::new(&glob)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .is_ok_and(|g| g.compile_matcher().is_match(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str, path: &str) -> FileConfig {
        let mut c = FileConfig::default();
        apply(&mut c, text, path);
        c
    }

    #[test]
    fn sections_match_by_glob_and_later_ones_win() {
        let text = "root = true\n\n[*]\nindent_style = space\nindent_size = 4\n\
                    trim_trailing_whitespace = true\n\n[*.{js,ts}]\nindent_size = 2\n\n\
                    [Makefile]\nindent_style = tab\n";
        let rs = config(text, "src/main.rs");
        assert_eq!(rs.tabs, Some(false));
        assert_eq!(rs.indent_size, Some(4));
        assert_eq!(rs.trim_trailing_whitespace, Some(true));
        assert_eq!(config(text, "web/app.ts").indent_size, Some(2));
        assert_eq!(config(text, "sub/Makefile").tabs, Some(true));
        assert!(is_root(text));
    }

    #[test]
    fn a_glob_with_a_slash_is_from_the_folder() {
        let text = "[lib/**.rs]\ninsert_final_newline = true\n";
        assert_eq!(config(text, "lib/a/b.rs").insert_final_newline, Some(true));
        assert_eq!(config(text, "src/lib/b.rs").insert_final_newline, None);
    }

    #[test]
    fn nearer_files_win_up_to_a_root() {
        let dir = std::env::temp_dir().join(format!("merlin-ec-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join(".editorconfig"), "[*]\ntab_width = 3\n").unwrap();
        fs::write(
            dir.join("a/.editorconfig"),
            "root = true\n[*]\nindent_size = 8\ntab_width = 5\n",
        )
        .unwrap();
        fs::write(dir.join("a/b/.editorconfig"), "[*.rs]\nindent_size = 2\n").unwrap();
        let c = for_file(&dir.join("a/b/x.rs"));
        assert_eq!(c.indent_size, Some(2));
        // The top file is past the root, so its 3 doesn't count.
        assert_eq!(c.tab_width, Some(5));
        let _ = fs::remove_dir_all(&dir);
    }
}
