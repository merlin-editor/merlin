//! Settings, as VS Code keeps them: the user's in
//! `~/.config/merlin/settings.toml`, and a workspace's in
//! `.merlin/settings.toml` in its folder, which wins where both set a
//! value. Plugins' settings are under `[plugins.<name>]` in either.

use std::fs;
use std::path::{Path, PathBuf};

use merlin_core::{AutoSave, Settings};

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
# scroll_beyond_last_line = true
# font_family = "JetBrains Mono"
# font_size = 13.5
# line_height = 1.55   # times the font size below 8, else points
# font_ligatures = true
# cursor_style = "line"        # line-thin, block, block-outline, underline, underline-thin
# cursor_blinking = "blink"    # smooth, phase, expand, solid
# smooth_scrolling = true
# mouse_wheel_scroll_sensitivity = 1.0
# fast_scroll_sensitivity = 5.0   # with Alt held
# scrollbar_size = 14
# minimap_scale = 1.0
# bracket_pair_colorization = true
# color_decorators = true      # a swatch under colours such as #e6c07b
# inline_diagnostics = true    # a line's worst problem written after it
# inlay_hints = true           # types and parameter names from the language server
# format_on_save = "off"       # "file", or "changed_lines": only what git says changed
# icons = "nerd"      # or "plain"
# theme = "dark"
# ui_font_size = 13   # the window's own text, apart from the code
# keymap = "vscode"   # or "vim"
# line_numbers = "on" # or "relative", counted from the cursor in Vim's Normal mode

[files]
# trim_trailing_whitespace = false
# insert_final_newline = false
# auto_save = "off"   # after_delay, on_focus_change, on_window_change
# auto_save_delay = 1000   # milliseconds, with after_delay
# hot_exit = true     # quit without asking; unsaved files come back next time
# A project's .editorconfig wins over these and the indentation settings.

[vim]
# leader = "space"       # merlin's own commands start with it
# which_key_delay = 500  # milliseconds on a prefix before the next keys show; 0 never
# insert_escape = "jk"   # typed quickly, leaves Insert mode as Esc does
# clipboard = false      # y, d and p use the system clipboard
# keep = ["ctrl+p", "ctrl+shift+p", "ctrl+s", "ctrl+tab", "ctrl+`", "ctrl+c"]   # do merlin's thing in every mode

[keys]
# "ctrl+k" = "view.minimap"
# "ctrl+d" = ""       # unbinds a key
# "ctrl+alt+1" = "macro.saved.note_the_line"   # a saved macro's key

# Macros saved with Save the Last Macro As…, keys as Vim writes them:
# [[macros]]
# name = "Note the line"
# keys = "I// NOTE: <Esc>j"

# Plugins' settings, by plugin name:
# [plugins.rust]

# Registries to install plugins from, besides merlin's own. Only these
# settings, yours, can add one: a folder's settings can't.
# [plugin-sources.acme]
# url = "https://plugins.acme.dev/index.json"
# key = "RWQ..."          # the minisign public key its index is signed with
# [plugin-sources.merlin]
# enabled = false         # turns merlin's own registry off
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
        let config = merlin_plugins::config_home().unwrap_or_else(|| PathBuf::from("."));
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
                Err(err) => {
                    let line = err
                        .span()
                        .map(|s| text[..s.start.min(text.len())].lines().count().max(1));
                    errors.push(match line {
                        Some(n) => format!("{}, line {n}: {}", path.display(), err.message()),
                        None => format!("{}: {}", path.display(), err.message()),
                    })
                }
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

    /// Saves a macro in the user's settings as `[[macros]]`, in place of
    /// one with the same name.
    pub fn save_macro(&self, saved: &merlin_core::Macro) -> Result<(), String> {
        let path = &self.user;
        let text = fs::read_to_string(path).unwrap_or_default();
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        let list = doc
            .entry("macros")
            .or_insert_with(|| toml_edit::Item::ArrayOfTables(Default::default()))
            .as_array_of_tables_mut()
            .ok_or("macros isn't a list of tables")?;
        let mut table = toml_edit::Table::new();
        table["name"] = toml_edit::value(saved.name.as_str());
        table["keys"] = toml_edit::value(saved.keys.as_str());
        let old = list
            .iter()
            .position(|t| t.get("name").and_then(|n| n.as_str()) == Some(saved.name.as_str()));
        match old.and_then(|i| list.get_mut(i)) {
            Some(old) => *old = table,
            None => list.push(table),
        }
        fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
            .and_then(|()| fs::write(path, doc.to_string()))
            .map_err(|e| e.to_string())
    }

    /// Takes `key` out of the user's settings or the workspace's, so the
    /// default, or the user's under the workspace's, shows through.
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

/// Where `merlin plugin install` links plugins.
pub fn plugins_dir() -> Option<PathBuf> {
    Some(
        merlin_plugins::config_home()?
            .join("merlin")
            .join("plugins"),
    )
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
    let files = table.get("files").and_then(|f| f.as_table());
    let files_flag = |key: &str, default: bool| {
        files
            .and_then(|f| f.get(key))
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
    };
    // `"ctrl+k ctrl+m" = "view.minimap"`, or with a condition,
    // `"ctrl+d" = { command = "edit.add_next", when = "editorTextFocus" }`.
    let keys = table
        .get("keys")
        .and_then(|k| k.as_table())
        .map(|keys| {
            keys.iter()
                .filter_map(|(k, v)| {
                    let (command, when) = match v {
                        toml::Value::String(command) => (command.clone(), None),
                        toml::Value::Table(t) => (
                            t.get("command")?.as_str()?.to_string(),
                            t.get("when").and_then(|w| w.as_str()).map(str::to_string),
                        ),
                        _ => return None,
                    };
                    Some(merlin_core::UserKey {
                        keys: k.clone(),
                        command,
                        when,
                    })
                })
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
        scroll_beyond_last_line: flag("scroll_beyond_last_line", defaults.scroll_beyond_last_line),
        bracket_pair_colorization: flag(
            "bracket_pair_colorization",
            defaults.bracket_pair_colorization,
        ),
        color_decorators: flag("color_decorators", defaults.color_decorators),
        inline_diagnostics: flag("inline_diagnostics", defaults.inline_diagnostics),
        inlay_hints: flag("inlay_hints", defaults.inlay_hints),
        // Off, file or changed_lines; true is the file.
        format_on_save: match editor.and_then(|e| e.get("format_on_save")) {
            Some(toml::Value::Boolean(true)) => merlin_core::FormatOnSave::File,
            Some(toml::Value::String(s)) if s == "file" => merlin_core::FormatOnSave::File,
            Some(toml::Value::String(s)) if s == "changed_lines" => {
                merlin_core::FormatOnSave::ChangedLines
            }
            _ => defaults.format_on_save,
        },
        trim_trailing_whitespace: files_flag(
            "trim_trailing_whitespace",
            defaults.trim_trailing_whitespace,
        ),
        insert_final_newline: files_flag("insert_final_newline", defaults.insert_final_newline),
        auto_save: match files
            .and_then(|f| f.get("auto_save"))
            .and_then(|v| v.as_str())
        {
            Some("after_delay") => AutoSave::AfterDelay,
            Some("on_focus_change") => AutoSave::OnFocusChange,
            Some("on_window_change") => AutoSave::OnWindowChange,
            _ => defaults.auto_save,
        },
        hot_exit: files_flag("hot_exit", defaults.hot_exit),
        auto_save_delay: files
            .and_then(|f| f.get("auto_save_delay"))
            .and_then(|v| v.as_integer().or_else(|| v.as_str()?.parse().ok()))
            .map_or(defaults.auto_save_delay, |n| n.clamp(10, 600_000) as u64),
        keymap: match editor
            .and_then(|e| e.get("keymap"))
            .and_then(|v| v.as_str())
        {
            Some("vim") => merlin_core::Keymap::Vim,
            _ => defaults.keymap,
        },
        vim: {
            let vim = table.get("vim").and_then(|v| v.as_table());
            let d = defaults.vim.clone();
            merlin_core::VimSettings {
                insert_escape: vim
                    .and_then(|v| v.get("insert_escape"))
                    .and_then(|v| v.as_str())
                    .filter(|s| s.chars().count() == 2)
                    .map_or(d.insert_escape, str::to_string),
                clipboard: vim
                    .and_then(|v| v.get("clipboard"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(d.clipboard),
                leader: vim
                    .and_then(|v| v.get("leader"))
                    .and_then(|v| v.as_str())
                    .map_or(d.leader, str::to_string),
                which_key_delay: vim
                    .and_then(|v| v.get("which_key_delay"))
                    .and_then(|v| v.as_integer().or_else(|| v.as_str()?.parse().ok()))
                    .map_or(d.which_key_delay, |n| n.clamp(0, 10_000) as u64),
                keep: vim
                    .and_then(|v| v.get("keep"))
                    .and_then(|v| v.as_array())
                    .map_or(d.keep, |keys| {
                        keys.iter()
                            .filter_map(|k| k.as_str().map(str::to_string))
                            .collect()
                    }),
            }
        },
        relative_line_numbers: editor
            .and_then(|e| e.get("line_numbers"))
            .and_then(|v| v.as_str())
            == Some("relative"),
        keys,
        macros: table
            .get("macros")
            .and_then(|m| m.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|m| {
                        let m = m.as_table()?;
                        Some(merlin_core::Macro {
                            name: m.get("name")?.as_str()?.to_string(),
                            keys: m.get("keys")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// How the code's cursor is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorStyle {
    #[default]
    Line,
    LineThin,
    Block,
    BlockOutline,
    Underline,
    UnderlineThin,
}

/// How the cursor blinks, as VS Code's `cursorBlinking` has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blinking {
    #[default]
    Blink,
    /// Fades out and in at each change.
    Smooth,
    /// Fades the whole way, out and in.
    Phase,
    /// Shrinks to nothing and grows back.
    Expand,
    Solid,
}

/// What `[editor]` says about the window itself: the code's font, the
/// cursor, scrolling, and the bars beside the code.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// Over the packs' code font, if installed.
    pub font_family: Option<String>,
    /// Points, over the packs' size.
    pub font_size: Option<f32>,
    /// Each row's height: times the font size below 8, else points.
    pub line_height: Option<f32>,
    pub font_ligatures: bool,
    pub cursor_style: CursorStyle,
    pub cursor_blinking: Blinking,
    /// A wheel's steps glide to where they scroll.
    pub smooth_scrolling: bool,
    /// How far the wheel and touchpad scroll, times their own distance.
    pub wheel_sensitivity: f32,
    /// Times that again with Alt held.
    pub fast_sensitivity: f32,
    /// The scrollbar's width, in points.
    pub scrollbar_size: f32,
    /// The minimap's size, times its own.
    pub minimap_scale: f32,
    /// The size of the window's own text, in points, which scales the
    /// whole frame around the code.
    pub ui_font_size: Option<f32>,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            font_family: None,
            font_size: None,
            line_height: None,
            font_ligatures: true,
            cursor_style: CursorStyle::Line,
            cursor_blinking: Blinking::Blink,
            smooth_scrolling: true,
            wheel_sensitivity: 1.0,
            fast_sensitivity: 5.0,
            scrollbar_size: 14.0,
            minimap_scale: 1.0,
            ui_font_size: None,
        }
    }
}

/// The window's settings in a merged table.
pub fn window(table: &toml::Table) -> Window {
    let d = Window::default();
    let get = |key: &str| table.get("editor")?.as_table()?.get(key);
    // A number, whole or not, or the text the Settings window's choice
    // writes.
    let number = |key: &str| {
        get(key).and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|n| n as f64))
                .or_else(|| v.as_str()?.parse().ok())
                .map(|n| n as f32)
                .filter(|n| n.is_finite())
        })
    };
    let text = |key: &str| get(key).and_then(|v| v.as_str());
    Window {
        font_family: text("font_family")
            .filter(|f| !f.trim().is_empty())
            .map(str::to_string),
        font_size: number("font_size").map(|n| n.clamp(6.0, 100.0)),
        line_height: number("line_height").filter(|&n| n > 0.0),
        font_ligatures: get("font_ligatures")
            .and_then(|v| v.as_bool())
            .unwrap_or(d.font_ligatures),
        cursor_style: match text("cursor_style") {
            Some("line-thin") => CursorStyle::LineThin,
            Some("block") => CursorStyle::Block,
            Some("block-outline") => CursorStyle::BlockOutline,
            Some("underline") => CursorStyle::Underline,
            Some("underline-thin") => CursorStyle::UnderlineThin,
            _ => d.cursor_style,
        },
        cursor_blinking: match text("cursor_blinking") {
            Some("smooth") => Blinking::Smooth,
            Some("phase") => Blinking::Phase,
            Some("expand") => Blinking::Expand,
            Some("solid") => Blinking::Solid,
            _ => d.cursor_blinking,
        },
        smooth_scrolling: get("smooth_scrolling")
            .and_then(|v| v.as_bool())
            .unwrap_or(d.smooth_scrolling),
        wheel_sensitivity: number("mouse_wheel_scroll_sensitivity")
            .map_or(d.wheel_sensitivity, |n| n.clamp(0.1, 20.0)),
        fast_sensitivity: number("fast_scroll_sensitivity")
            .map_or(d.fast_sensitivity, |n| n.clamp(1.0, 50.0)),
        scrollbar_size: number("scrollbar_size").map_or(d.scrollbar_size, |n| n.clamp(4.0, 40.0)),
        minimap_scale: number("minimap_scale").map_or(d.minimap_scale, |n| n.clamp(1.0, 3.0)),
        ui_font_size: number("ui_font_size").map(|n| n.clamp(9.0, 26.0)),
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
        let dir =
            std::env::temp_dir().join(format!("merlin-settings-{name}-{}", std::process::id()));
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
        assert_eq!(
            s.keys,
            [merlin_core::UserKey::new("ctrl+k", "view.minimap")]
        );
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
    fn window_settings_read_with_their_defaults() {
        let table: toml::Table = "[editor]\nfont_size = 15\nline_height = 1.4\ncursor_style = \"block\"\ncursor_blinking = \"phase\"\nsmooth_scrolling = false\nscrollbar_size = 100\n"
            .parse()
            .unwrap();
        let w = window(&table);
        assert_eq!(w.font_size, Some(15.0));
        assert_eq!(w.line_height, Some(1.4));
        assert_eq!(w.cursor_style, CursorStyle::Block);
        assert_eq!(w.cursor_blinking, Blinking::Phase);
        assert!(!w.smooth_scrolling && w.font_ligatures);
        assert_eq!(w.scrollbar_size, 40.0, "kept in range");
        assert_eq!(w.wheel_sensitivity, 1.0);
        assert_eq!(window(&toml::Table::new()), Window::default());
    }

    #[test]
    fn a_broken_file_is_reported() {
        let f = files("broken");
        fs::create_dir_all(f.user.parent().unwrap()).unwrap();
        fs::write(&f.user, "[editor\n").unwrap();
        let (_, errors) = f.load();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn a_broken_file_says_its_line() {
        let f = files("line");
        fs::create_dir_all(f.user.parent().unwrap()).unwrap();
        fs::write(&f.user, "[editor]\nminimap = true\nbroken =\n").unwrap();
        let (_, errors) = f.load();
        assert!(errors[0].contains(", line 3: "), "{}", errors[0]);
    }

    #[test]
    fn a_setting_comes_out_and_the_rest_stays() {
        let f = files("remove");
        fs::create_dir_all(f.user.parent().unwrap()).unwrap();
        fs::write(&f.user, "[editor]\n# mine\nminimap = true\ntab_size = 2\n").unwrap();
        f.remove_in(false, "editor.minimap").unwrap();
        f.set_in(false, "keys.ctrl+.", "\"edit.rename\"").unwrap();
        let text = fs::read_to_string(&f.user).unwrap();
        assert!(!text.contains("minimap"));
        assert!(text.contains("tab_size = 2"));
        assert!(text.contains("\"ctrl+.\" = \"edit.rename\""), "{text}");
        let (user, _) = f.each();
        assert_eq!(user["editor"]["tab_size"].as_integer(), Some(2));
    }

    #[test]
    fn a_saved_macro_goes_in_as_macros_and_replaces_its_name() {
        let f = files("macros");
        fs::create_dir_all(f.user.parent().unwrap()).unwrap();
        fs::write(&f.user, "[editor]\nminimap = true\n").unwrap();
        let m = |name: &str, keys: &str| merlin_core::Macro {
            name: name.into(),
            keys: keys.into(),
        };
        f.save_macro(&m("Note", "I// <Esc>")).unwrap();
        f.save_macro(&m("Bang", "A!<Esc>")).unwrap();
        f.save_macro(&m("Note", "I# <Esc>")).unwrap();
        let text = fs::read_to_string(&f.user).unwrap();
        assert_eq!(text.matches("[[macros]]").count(), 2, "{text}");
        let (table, _) = f.load();
        assert_eq!(
            editor(&table).macros,
            [m("Note", "I# <Esc>"), m("Bang", "A!<Esc>")]
        );
    }
}
