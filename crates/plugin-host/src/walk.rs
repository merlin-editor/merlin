//! Walking the project as its exclusions say: `.gitignore` and hidden
//! names as the settings have them, and Settings › Files' patterns,
//! whose folders aren't walked into at all.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use merlin_core::exclude::Matcher;

/// How many paths a walk lists at most.
pub const MAX: usize = 50_000;

/// A walk of `root` that leaves out what `rules` do; with `search`,
/// Exclude from search's patterns too.
pub fn walker(root: &Path, rules: &Matcher, search: bool) -> ignore::WalkBuilder {
    counting_walker(root, rules, search, None)
}

/// As [`walker`], counting into `skipped` what the patterns leave out.
pub fn counting_walker(
    root: &Path,
    rules: &Matcher,
    search: bool,
    skipped: Option<Arc<AtomicUsize>>,
) -> ignore::WalkBuilder {
    let mut walk = ignore::WalkBuilder::new(root);
    let r = &rules.rules;
    walk.hidden(!r.hidden)
        .git_ignore(r.gitignore)
        .git_global(r.gitignore)
        .git_exclude(r.gitignore);
    let root = root.to_path_buf();
    let rules = rules.clone();
    walk.filter_entry(move |e| {
        // git's own folder is never the project's.
        if e.file_name() == ".git" {
            return false;
        }
        let Ok(rel) = e.path().strip_prefix(&root) else {
            return true;
        };
        if rel.as_os_str().is_empty() {
            return true;
        }
        let mut rel = rel.to_string_lossy().replace('\\', "/");
        if e.file_type().is_some_and(|t| t.is_dir()) {
            rel.push('/');
        }
        let keep = rules.why(&rel, search).is_none();
        if !keep && let Some(count) = &skipped {
            count.fetch_add(1, Ordering::Relaxed);
        }
        keep
    });
    walk
}

/// The project's files, relative to `root` and sorted.
pub fn project_files(root: &Path, rules: &Matcher, search: bool) -> Vec<String> {
    let mut files: Vec<String> = walker(root, rules, search)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| {
            Some(
                e.path()
                    .strip_prefix(root)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
        })
        .take(MAX)
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use merlin_core::exclude::Exclusions;

    #[test]
    fn excluded_folders_arent_walked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for f in [
            "src/a.c",
            "third_party/z/z.c",
            "docs/api/i.html",
            ".hidden/h.c",
            "b.o",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let rules = Matcher::new(Exclusions {
            files: vec!["third_party/".into(), "*.o".into()],
            search: vec!["docs/api/".into()],
            ..Exclusions::default()
        });
        assert_eq!(
            project_files(root, &rules, false),
            ["docs/api/i.html", "src/a.c"]
        );
        assert_eq!(project_files(root, &rules, true), ["src/a.c"]);
        let shown = Matcher::new(Exclusions {
            hidden: true,
            ..Exclusions::default()
        });
        assert!(project_files(root, &shown, false).contains(&".hidden/h.c".to_string()));
    }
}
