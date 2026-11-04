//! Settings, as VS Code keeps them: the user's in
//! `~/.config/merlin/settings.toml`, and a workspace's in
//! `.merlin/settings.toml` in its folder, which wins where both set a
//! value. Plugins' settings are under `[plugins.<name>]` in either.

use std::path::{Path, PathBuf};
use std::{env, fs};

use merlin_core::Settings;

/// What a new settings file starts with.
const USER_TEMPLATE: &str = r#"# merlin settings. Saving this file applies them.
# A workspace's .merlin/settings.toml overrides these.

[editor]
# minimap = false
# word_wrap = false
# breadcrumbs = true
# sticky_scroll = true
# preview_tabs = true
# tab_size = 4
# insert_spaces = true
# detect_indentation = true    # a file's own indentation wins
# auto_closing_brackets = true
# icons = "nerd"      # or "plain"
# theme = "dark"

[keys]
# "ctrl+k" = "view.minimap"
# "ctrl+d" = ""       # unbinds a key

# Plugins' settings, by plugin name:
# [plugins.lsp]
"#;

const WORKSPACE_TEMPLATE: &str = r#"# Settings for this folder, over your own (Preferences: Open User Settings).

[editor]
"#;

pub struct Files {
    pub user: PathBuf,
    pub workspace: PathBuf,
}

impl Files {
    pub fn new(root: &Path) -> Self {
        let config = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            user: config.join("merlin").join("settings.toml"),
            workspace: root.join(".merlin").join("settings.toml"),
        }
    }

    pub fn is_settings(&self, path: &Path) -> bool {
        path == self.user || path == self.workspace
    }

    /// The settings file, made with a template first if there's none.
    pub fn ensure(&self, workspace: bool) -> std::io::Result<PathBuf> {
        let (path, template) = if workspace {
            (&self.workspace, WORKSPACE_TEMPLATE)
        } else {
            (&self.user, USER_TEMPLATE)
        };
        if !path.exists() {
            fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
            fs::write(path, template)?;
        }
        Ok(path.clone())
    }

    /// The user's settings with the workspace's over them, and what was
    /// wrong with either file.
    pub fn load(&self) -> (toml::Table, Vec<String>) {
        let mut merged = toml::Table::new();
        let mut errors = Vec::new();
        for path in [&self.user, &self.workspace] {
            let Ok(text) = fs::read_to_string(path) else {
                continue;
            };
            match text.parse::<toml::Table>() {
                Ok(table) => merlin_plugins::merge(&mut merged, &table),
                Err(err) => errors.push(format!("{}: {}", path.display(), err.message())),
            }
        }
        (merged, errors)
    }

    /// Sets `key` (such as `editor.minimap`) to a TOML value in the user's
    /// settings, keeping the rest of the file as it is.
    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.set_in(false, key, value)
    }

    /// Sets `key` in the user's settings or the workspace's.
    pub fn set_in(&self, workspace: bool, key: &str, value: &str) -> Result<(), String> {
        let value: toml_edit::Value = value.parse().map_err(|e| format!("{e}"))?;
        self.edit(workspace, key, |table, last| {
            table[last] = toml_edit::value(value);
        })
    }

    /// Takes `key` out of the user's settings or the workspace's, so the
    /// default, or the user's under the workspace's, shows through.
    #[expect(
        dead_code,
        reason = "for the Settings window in prefs.rs, not wired in yet"
    )]
    pub fn remove_in(&self, workspace: bool, key: &str) -> Result<(), String> {
        self.edit(workspace, key, |table, last| {
            table.remove(last);
        })
    }

    fn edit(
        &self,
        workspace: bool,
        key: &str,
        change: impl FnOnce(&mut toml_edit::Table, &str),
    ) -> Result<(), String> {
        let path = if workspace {
            &self.workspace
        } else {
            &self.user
        };
        let text = fs::read_to_string(path).unwrap_or_default();
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        // Dots part tables, except in `[keys]`, where the rest is one key
        // even if it has dots, as "ctrl+." does.
        let mut parts: Vec<&str> = match key.strip_prefix("keys.") {
            Some(rest) => vec!["keys", rest],
            None => key.split('.').collect(),
        };
        let last = parts.pop().ok_or("empty key")?;
        let mut table = doc.as_table_mut();
        for part in parts {
            let entry = table
                .entry(part)
                .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
            table = entry
                .as_table_mut()
                .ok_or_else(|| format!("{part} isn't a table"))?;
        }
        change(table, last);
        fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
            .and_then(|()| fs::write(path, doc.to_string()))
            .map_err(|e| e.to_string())
    }

    /// Each file's settings on their own: the user's, then the workspace's.
    #[expect(
        dead_code,
        reason = "for the Settings window in prefs.rs, not wired in yet"
    )]
    pub fn each(&self) -> (toml::Table, toml::Table) {
        let read = |p: &Path| {
            fs::read_to_string(p)
                .ok()
                .and_then(|t| t.parse::<toml::Table>().ok())
                .unwrap_or_default()
        };
        (read(&self.user), read(&self.workspace))
    }
}

/// The editor's settings in a merged table.
pub fn editor(table: &toml::Table) -> Settings {
    let defaults = Settings::default();
    let editor = table.get("editor").and_then(|e| e.as_table());
    let flag = |key: &str, default: bool| {
        editor
            .and_then(|e| e.get(key))
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
    };
    let keys = table
        .get("keys")
        .and_then(|k| k.as_table())
        .map(|keys| {
            keys.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Settings {
        minimap: flag("minimap", defaults.minimap),
        word_wrap: flag("word_wrap", defaults.word_wrap),
        breadcrumbs: flag("breadcrumbs", defaults.breadcrumbs),
        sticky_scroll: flag("sticky_scroll", defaults.sticky_scroll),
        preview_tabs: flag("preview_tabs", defaults.preview_tabs),
        tab_size: editor
            .and_then(|e| e.get("tab_size"))
            // A number, or the text the Settings window's choice writes.
            .and_then(|v| v.as_integer().or_else(|| v.as_str()?.parse().ok()))
            .map_or(defaults.tab_size, |n| n.clamp(1, 16) as usize),
        insert_spaces: flag("insert_spaces", defaults.insert_spaces),
        detect_indentation: flag("detect_indentation", defaults.detect_indentation),
        auto_closing_brackets: flag("auto_closing_brackets", defaults.auto_closing_brackets),
        keys,
    }
}

/// A text setting of `[editor]`, such as `icons` or `theme`.
pub fn editor_text<'a>(table: &'a toml::Table, key: &str) -> Option<&'a str> {
    table.get("editor")?.as_table()?.get(key)?.as_str()
}

/// A list of strings under `[editor]`, such as the theme packs.
pub fn editor_list(table: &toml::Table, key: &str) -> Vec<String> {
    table
        .get("editor")
        .and_then(|e| e.as_table())
        .and_then(|e| e.get(key))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The `[plugins]` table, for the plugin host.
pub fn plugins(table: &toml::Table) -> toml::Table {
    table
        .get("plugins")
        .and_then(|p| p.as_table())
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(name: &str) -> Files {
        let dir = env::temp_dir().join(format!("merlin-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Files {
            user: dir.join("user/settings.toml"),
            workspace: dir.join("ws/.merlin/settings.toml"),
        }
    }

    #[test]
    fn the_workspace_wins_and_tables_merge() {
        let f = files("merge");
        fs::create_dir_all(f.user.parent().unwrap()).unwrap();
        fs::create_dir_all(f.workspace.parent().unwrap()).unwrap();
        fs::write(
            &f.user,
            "[editor]\nminimap = true\nword_wrap = true\n[keys]\n\"ctrl+k\" = \"view.minimap\"\n[plugins.lsp]\na = 1\nb = 2\n",
        )
        .unwrap();
        fs::write(
            &f.workspace,
            "[editor]\nword_wrap = false\n[plugins.lsp]\nb = 3\n",
        )
        .unwrap();
        let (table, errors) = f.load();
        assert!(errors.is_empty());
        let s = editor(&table);
        assert!(s.minimap && !s.word_wrap && s.breadcrumbs);
        assert_eq!(s.keys, [("ctrl+k".to_string(), "view.minimap".to_string())]);
        let lsp = plugins(&table)["lsp"].as_table().unwrap().clone();
        assert_eq!(lsp["a"].as_integer(), Some(1));
        assert_eq!(lsp["b"].as_integer(), Some(3));
    }

    #[test]
    fn setting_a_value_keeps_comments() {
        let f = files("set");
        f.ensure(false).unwrap();
        f.set("editor.minimap", "true").unwrap();
        f.set("editor.minimap", "false").unwrap();
        let text = fs::read_to_string(&f.user).unwrap();
        assert!(text.contains("# merlin settings"), "{text}");
        let set = text
            .lines()
            .filter(|l| l.trim() == "minimap = false")
            .count();
        assert_eq!(set, 1, "{text}");
        assert!(text.contains("# minimap = false"), "{text}");
        let (table, _) = f.load();
        assert!(!editor(&table).minimap);
    }

    #[test]
    fn a_broken_file_is_reported() {
        let f = files("broken");
        fs::create_dir_all(f.user.parent().unwrap()).unwrap();
        fs::write(&f.user, "[editor\n").unwrap();
        let (_, errors) = f.load();
        assert_eq!(errors.len(), 1);
    }
}
