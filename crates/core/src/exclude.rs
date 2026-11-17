//! What the project leaves out: Settings › Files' Exclude and Exclude
//! from search, `.gitignore` and hidden names. One set of rules that the
//! Explorer, Quick Open, Search, symbols and plugins all follow.
//!
//! A pattern is a glob. One ending in `/` names a folder and everything
//! in it; one with no other `/` matches at any depth, as `*.o` or
//! `third_party/`; the rest are from the project's top, as
//! `build/_deps/`. `*` stays within a folder and `**` crosses them.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// The rules as the settings give them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exclusions {
    /// Left out everywhere: `[files] exclude`.
    pub files: Vec<String>,
    /// Kept in the Explorer, left out of Quick Open, Search and symbols:
    /// `[search] exclude`.
    pub search: Vec<String>,
    /// Files git ignores are left out too.
    pub gitignore: bool,
    /// Names starting with a dot are shown.
    pub hidden: bool,
}

impl Default for Exclusions {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            search: Vec::new(),
            gitignore: true,
            hidden: false,
        }
    }
}

/// Why a path is left out, for the Explorer's mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Why {
    GitIgnore,
    Hidden,
    /// By a pattern of Exclude.
    Excluded,
    /// By a pattern of Exclude from search.
    SearchOnly,
}

impl Why {
    /// The Explorer's mark.
    pub fn label(self) -> &'static str {
        match self {
            Why::GitIgnore => ".gitignore",
            Why::Hidden => "hidden",
            Why::Excluded => "excluded",
            Why::SearchOnly => "excluded from search",
        }
    }
}

/// One list of patterns, ready to match.
#[derive(Clone, Debug)]
struct Patterns {
    /// Patterns for files and folders.
    any: GlobSet,
    /// Patterns that end in `/`: folders only.
    folders: GlobSet,
}

impl Patterns {
    fn new(patterns: &[String]) -> Self {
        let mut any = GlobSetBuilder::new();
        let mut folders = GlobSetBuilder::new();
        for pattern in patterns {
            let Some((glob, folder)) = glob(pattern) else {
                continue;
            };
            let Ok(glob) = GlobBuilder::new(&glob).literal_separator(true).build() else {
                continue;
            };
            if folder {
                folders.add(glob);
            } else {
                any.add(glob);
            }
        }
        Self {
            any: any.build().unwrap_or_else(|_| GlobSet::empty()),
            folders: folders.build().unwrap_or_else(|_| GlobSet::empty()),
        }
    }

    /// Whether `rel` itself matches: a folder's ends in `/`.
    fn matches(&self, rel: &str) -> bool {
        match rel.strip_suffix('/') {
            Some(dir) => self.folders.is_match(dir) || self.any.is_match(dir),
            None => self.any.is_match(rel),
        }
    }

    /// Whether `rel` or a folder it's in matches.
    fn covers(&self, rel: &str) -> bool {
        let dir = rel.ends_with('/');
        let rel = rel.trim_end_matches('/');
        let mut at = 0;
        loop {
            match rel[at..].find('/') {
                Some(i) => {
                    if self.matches(&format!("{}/", &rel[..at + i])) {
                        return true;
                    }
                    at += i + 1;
                }
                None => {
                    let last = if dir {
                        format!("{rel}/")
                    } else {
                        rel.to_string()
                    };
                    return self.matches(&last);
                }
            }
        }
    }
}

/// A pattern as a glob from the top, and whether it's for folders only.
fn glob(pattern: &str) -> Option<(String, bool)> {
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
    Some((glob, folder))
}

/// The rules, ready to match paths relative to the project's top.
#[derive(Clone, Debug)]
pub struct Matcher {
    pub rules: Exclusions,
    files: Patterns,
    search: Patterns,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new(Exclusions::default())
    }
}

impl Matcher {
    pub fn new(rules: Exclusions) -> Self {
        Self {
            files: Patterns::new(&rules.files),
            search: Patterns::new(&rules.search),
            rules,
        }
    }

    /// Why Exclude, or with `search` Exclude from search too, leaves
    /// `rel` itself out, not counting the folders it's in. A folder's
    /// path ends in `/`. Hidden names and `.gitignore` are the walker's.
    pub fn why(&self, rel: &str, search: bool) -> Option<Why> {
        if self.files.matches(rel) {
            Some(Why::Excluded)
        } else if search && self.search.matches(rel) {
            Some(Why::SearchOnly)
        } else {
            None
        }
    }

    /// Whether `rel`, or a folder it's in, is left out by the patterns.
    pub fn excluded(&self, rel: &str, search: bool) -> bool {
        self.files.covers(rel) || (search && self.search.covers(rel))
    }

    /// The patterns that leave `rel` out, from both lists, with whether
    /// each is Exclude from search's.
    pub fn rules_for(&self, rel: &str) -> Vec<(String, bool)> {
        let mut found = Vec::new();
        for (list, search) in [(&self.rules.files, false), (&self.rules.search, true)] {
            for pattern in list {
                let one = Patterns::new(std::slice::from_ref(pattern));
                if one.covers(rel) {
                    found.push((pattern.clone(), search));
                }
            }
        }
        found
    }
}

/// The pattern that leaves out `rel`, a path from the project's top, as
/// the Explorer's Exclude writes it: a folder ends in `/`.
pub fn pattern_for(rel: &str, folder: bool) -> String {
    let rel = rel.trim_matches('/');
    if folder {
        format!("{rel}/")
    } else {
        rel.to_string()
    }
}

/// A pattern as a regular expression for a path from the project's top,
/// as clangd's `PathMatch` takes them: a folder's matches what's in it.
pub fn regex_for(pattern: &str) -> Option<String> {
    let (glob, folder) = glob(pattern)?;
    let mut out = String::new();
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                // `**/` is any number of folders, none too.
                if chars.peek() == Some(&'/') {
                    chars.next();
                    out.push_str("(.*/)?");
                } else {
                    out.push_str(".*");
                }
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '[' | ']' | '{' | '}' => out.push(c),
            c if "\\.+()|^$".contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    if folder {
        out.push_str("/.*");
    } else {
        // A file, or a folder matched without the `/`.
        out.push_str("(/.*)?");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(files: &[&str], search: &[&str]) -> Matcher {
        Matcher::new(Exclusions {
            files: files.iter().map(|s| s.to_string()).collect(),
            search: search.iter().map(|s| s.to_string()).collect(),
            ..Exclusions::default()
        })
    }

    #[test]
    fn folders_files_and_depth() {
        let m = matcher(&["third_party/", "*.o", "build/_deps/"], &["docs/api/"]);
        // A folder pattern with no other `/`: at any depth, and what's in it.
        assert_eq!(m.why("third_party/", false), Some(Why::Excluded));
        assert_eq!(m.why("lib/third_party/", false), Some(Why::Excluded));
        assert!(m.excluded("third_party/zlib/zlib.h", false));
        // But not a file of that name.
        assert_eq!(m.why("third_party", false), None);
        assert!(m.excluded("src/a.o", false));
        // With a `/`: from the top only.
        assert!(m.excluded("build/_deps/fmt/a.h", false));
        assert!(!m.excluded("x/build/_deps/a.h", false));
        // Search only.
        assert!(!m.excluded("docs/api/index.html", false));
        assert!(m.excluded("docs/api/index.html", true));
        assert_eq!(m.why("docs/api/", true), Some(Why::SearchOnly));
        assert!(!m.excluded("src/main.cpp", true));
        assert_eq!(
            m.rules_for("lib/third_party/a.c"),
            [("third_party/".to_string(), false)]
        );
        assert_eq!(pattern_for("third_party", true), "third_party/");
    }

    #[test]
    fn as_regexes_for_clangd() {
        let re = |p: &str| regex::Regex::new(&format!("^{}$", regex_for(p).unwrap())).unwrap();
        let tp = re("third_party/");
        assert!(tp.is_match("third_party/zlib/a.c"));
        assert!(tp.is_match("lib/third_party/a.c"));
        assert!(!tp.is_match("third_party.c"));
        let generated = re("**/*.generated.h");
        assert!(generated.is_match("a.generated.h"));
        assert!(generated.is_match("src/x/a.generated.h"));
        assert!(!generated.is_match("src/a.h"));
        let deps = re("build/_deps/");
        assert!(deps.is_match("build/_deps/fmt/format.h"));
        assert!(!deps.is_match("x/build/_deps/a.h"));
    }
}
