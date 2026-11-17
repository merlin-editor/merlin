//! What the project leaves out, changed from the Explorer: Exclude adds
//! a pattern to the workspace's settings, Include in Project and Undo
//! take one out wherever it's set. Saving applies them as any setting.

use super::*;

/// The settings key of a list: Exclude's, or Exclude from search's.
fn key(search: bool) -> &'static str {
    if search {
        "search.exclude"
    } else {
        "files.exclude"
    }
}

/// The patterns `table` has at `key`.
fn patterns(table: &toml::Table, key: &str) -> Vec<String> {
    let (section, last) = key.split_once('.').unwrap_or(("", key));
    table
        .get(section)
        .and_then(|s| s.get(last))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn as_toml(list: &[String]) -> String {
    toml::Value::Array(list.iter().cloned().map(toml::Value::String).collect()).to_string()
}

impl App {
    /// Adds `pattern` to the workspace's Exclude, or Exclude from search.
    pub(crate) fn exclude(&mut self, pattern: &str, search: bool) -> Vec<Event> {
        let key = key(search);
        let (_, workspace) = self.settings.each();
        let mut list = patterns(&workspace, key);
        if !list.iter().any(|p| p == pattern) {
            list.push(pattern.to_string());
        }
        if let Err(err) = self.settings.set_in(true, key, &as_toml(&list)) {
            return vec![Event::Error(format!("Couldn't exclude {pattern}: {err}"))];
        }
        self.load_settings()
    }

    /// Takes patterns out of the lists they're in, the user's and the
    /// workspace's.
    pub(crate) fn include(&mut self, rules: &[(String, bool)]) -> Vec<Event> {
        let (user, workspace) = self.settings.each();
        for (in_workspace, table) in [(false, &user), (true, &workspace)] {
            for search in [false, true] {
                let key = key(search);
                let list = patterns(table, key);
                let kept: Vec<String> = list
                    .iter()
                    .filter(|p| !rules.iter().any(|(r, s)| r == *p && *s == search))
                    .cloned()
                    .collect();
                if kept.len() == list.len() {
                    continue;
                }
                let done = if kept.is_empty() {
                    self.settings.remove_in(in_workspace, key)
                } else {
                    self.settings.set_in(in_workspace, key, &as_toml(&kept))
                };
                if let Err(err) = done {
                    return vec![Event::Error(format!("Couldn't change {key}: {err}"))];
                }
            }
        }
        self.load_settings()
    }

    /// The exclusions as the settings now say: when they changed, the
    /// Explorer's walked again and the plugins see the new rules. Quick
    /// Open lists the files afresh each time it opens.
    pub(crate) fn exclusions_changed(&mut self) -> Vec<Event> {
        let rules = settings::exclusions(
            &self.loaded.merged,
            &self.loaded.user,
            &self.loaded.workspace,
        );
        if rules == self.exclusions.rules {
            return Vec::new();
        }
        self.exclusions = std::sync::Arc::new(Matcher::new(rules));
        self.tree_hash = None;
        self.list_tree_later();
        let snapshot = self.snapshot();
        self.host.exclusions_changed(&snapshot)
    }

    /// Settings, at the Files page.
    pub(crate) fn open_settings_files(&mut self) {
        self.open_settings(None, false);
        self.settings_ui.category = prefs::Category::Files;
        // Where the patterns are: the folder's, if it has any.
        let here = ["files", "search"]
            .iter()
            .any(|t| !patterns(&self.loaded.workspace, &format!("{t}.exclude")).is_empty());
        self.settings_ui.workspace = here;
        self.settings_ui.plugin = None;
        self.settings_ui.search.clear();
        self.settings_ui.scroll = 0.0;
    }
}
