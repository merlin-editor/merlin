//! The Settings window, as the app keeps it, and plugin pages' actions.
//! The window draws them from [`SettingsView`] and [`PageView`], and says
//! what was clicked with a [`SettingsAction`] or a [`PageAction`]. Changes
//! go to the user's or the workspace's settings file, and apply at once.

use std::path::PathBuf;
use std::time::Duration;

use merlin_core::panel::Tone;
use merlin_core::{Event, Key, KeyBinding, PluginSummary};
use merlin_plugins::PluginInfo;

use crate::packs::{self, Icons, Look, Pack};
use crate::theme::{self, Palette};
use crate::{App, settings};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    Appearance,
    Editor,
    Keys,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeysFilter {
    #[default]
    All,
    Changed,
    Plugins,
}

/// The Settings window while it's open.
#[derive(Clone, Debug, Default)]
pub struct SettingsUi {
    pub category: Category,
    /// Changes go to the workspace's settings instead of the user's.
    pub workspace: bool,
    pub search: String,
    pub keys: KeysFilter,
    /// The command waiting for a key to be pressed for it.
    pub capture: Option<String>,
    pub scroll: f32,
}

/// What a setting's control is.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Toggle(bool),
    Choice(Vec<&'static str>, String),
    Number(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: &'static str,
    pub about: &'static str,
    pub key: &'static str,
    pub control: Control,
    /// Set in the file being changed, so not the default there.
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeCard {
    pub label: String,
    pub palette: Palette,
    pub tag: &'static str,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IconCard {
    pub label: String,
    pub icons: Icons,
    pub from: String,
    pub current: bool,
}

/// What the Settings window shows.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsView {
    pub ui: SettingsUiView,
    /// Settings changed in the file being changed, by category.
    pub changed: [usize; 3],
    pub themes: Vec<ThemeCard>,
    pub icons: Vec<IconCard>,
    /// From the top: name, what kind of layer, and whether it's a pack.
    pub layers: Vec<(String, String, bool)>,
    pub in_use: Vec<(String, String)>,
    pub available: Vec<(String, String)>,
    pub rows: Vec<(&'static str, Vec<Row>)>,
    pub keys: Vec<KeyBinding>,
    pub file: PathBuf,
}

/// The parts of [`SettingsUi`] drawing needs, comparable for redraws.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsUiView {
    pub category: Category,
    pub workspace: bool,
    pub search: String,
    pub keys: KeysFilter,
    pub capture: Option<String>,
}

/// A click in the Settings window.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsAction {
    Close,
    Category(Category),
    Workspace(bool),
    Theme(String),
    Icons(String),
    PackUp(usize),
    PackDown(usize),
    PackOff(usize),
    PackOn(String),
    Toggle(&'static str),
    Choose(&'static str, &'static str),
    Step(&'static str, f32),
    KeysFilter(KeysFilter),
    Capture(String),
    Unbind(String, String),
    ResetKey(String),
    OpenFile,
    OpenPlugins,
}

/// What a plugin's page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct PageView {
    pub name: String,
    pub title: String,
    pub dir: PathBuf,
    pub api: String,
    pub data_only: bool,
    pub enabled: bool,
    pub waiting: bool,
    pub failed: bool,
    pub read: bool,
    pub write: bool,
    pub processes: Vec<String>,
    pub running: Vec<(String, u32, Option<u64>)>,
    pub memory: usize,
    pub busy: Duration,
    pub commands: Vec<(String, Option<String>)>,
    pub languages: Vec<String>,
    pub looks: Vec<String>,
    /// Its settings: key, value, and whether the user changed it.
    pub settings: Vec<(String, toml::Value, bool)>,
    /// The Uninstall button was pressed once, and waits for a second.
    pub uninstall_armed: bool,
    /// Installed with `merlin plugin install`, so it can be uninstalled.
    pub installed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PageAction {
    Enable(String, bool),
    Reload(String),
    Output,
    Uninstall(String),
    /// A plugin's setting: plugin, key, new value.
    Setting(String, String, toml::Value),
}

const FONT_SIZE: f32 = 13.5;

impl App {
    pub(crate) fn open_settings(&mut self) {
        self.settings_ui = Some(SettingsUi::default());
    }

    /// The file changes go to.
    fn scope_file(&self) -> PathBuf {
        let workspace = self.settings_ui.as_ref().is_some_and(|u| u.workspace);
        if workspace {
            self.settings.workspace.clone()
        } else {
            self.settings.user.clone()
        }
    }

    pub(crate) fn settings_view(&self) -> Option<SettingsView> {
        let ui = self.settings_ui.as_ref()?;
        let (user, workspace) = self.settings.each();
        let scope = if ui.workspace { &workspace } else { &user };
        let (merged, _) = self.settings.load();
        let set_here = |section: &str, key: &str| {
            scope
                .get(section)
                .and_then(|t| t.as_table())
                .is_some_and(|t| t.contains_key(key))
        };
        // `editor.minimap` is `minimap` in `[editor]`.
        let part = |key: &'static str| key.split_once('.').unwrap_or(("editor", key));
        let flag = |key: &'static str, default: bool| {
            let (table, name) = part(key);
            merged
                .get(table)
                .and_then(|e| e.get(name))
                .and_then(|v| v.as_bool())
                .unwrap_or(default)
        };
        let toggle = |label, about, key: &'static str, default| Row {
            label,
            about,
            key,
            control: Control::Toggle(flag(key, default)),
            changed: set_here(part(key).0, part(key).1),
        };
        // A value as a choice shows it, numbers as written.
        let value = |key: &'static str, default: &str| {
            let (table, name) = part(key);
            merged
                .get(table)
                .and_then(|e| e.get(name))
                .map_or(default.to_string(), |v| match v {
                    toml::Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
        };
        let choice = |label, about, key: &'static str, options, default| Row {
            label,
            about,
            key,
            control: Control::Choice(options, value(key, default)),
            changed: set_here(part(key).0, part(key).1),
        };
        let icons = settings::editor_text(&merged, "icons").unwrap_or("nerd").to_string();
        let tab_size = settings::editor(&merged).tab_size.to_string();
        let size = merged
            .get("editor")
            .and_then(|e| e.get("font_size"))
            .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
            .map_or(FONT_SIZE, |f| f as f32);
        let rows = vec![
            (
                "View",
                vec![
                    toggle("Minimap", "The file's shape beside the scrollbar, with matches, problems and changes marked.", "editor.minimap", false),
                    toggle("Breadcrumbs", "The file's folders and the symbols around the cursor, under the tabs.", "editor.breadcrumbs", true),
                    toggle("Sticky scroll", "The first lines of the blocks in view stay pinned at the top.", "editor.sticky_scroll", true),
                    choice("Minimap scale", "How big the minimap draws the file.", "editor.minimap_scale", vec!["1", "2", "3"], "1"),
                    choice("Scrollbar size", "The scrollbar's width, in points.", "editor.scrollbar_size", vec!["10", "14", "18"], "14"),
                    choice("Interface text size", "The size of the window's own text, in points. Everything around the code grows with it; the code keeps its own size.", "editor.ui_font_size", vec!["11", "12", "13", "14", "15", "16"], "13"),
                    toggle("Bracket pair colours", "Brackets are coloured by how deep they are.", "editor.bracket_pair_colorization", true),
                    toggle("Colour swatches", "A bar in the colour under colours written in the code, such as #e6c07b or rgb(0 120 255).", "editor.color_decorators", true),
                    toggle("Problems inline", "A line's worst error or warning is written after the end of the line.", "editor.inline_diagnostics", true),
                    toggle("Inlay hints", "Types and parameter names from the language server, in quiet chips between the code.", "editor.inlay_hints", true),
                ],
            ),
            (
                "Scrolling",
                vec![
                    toggle("Smooth scrolling", "A wheel's steps glide to where they scroll.", "editor.smooth_scrolling", true),
                    toggle("Scroll beyond last line", "The last line can scroll up to the top of the view.", "editor.scroll_beyond_last_line", true),
                    choice("Wheel sensitivity", "How far the wheel and touchpad scroll, times their own.", "editor.mouse_wheel_scroll_sensitivity", vec!["0.5", "1", "2", "3"], "1"),
                    choice("Fast scroll sensitivity", "Times that again with Alt held.", "editor.fast_scroll_sensitivity", vec!["3", "5", "10"], "5"),
                ],
            ),
            (
                "Text",
                vec![
                    toggle("Word wrap", "Long lines wrap at spaces instead of scrolling sideways. Alt+Z", "editor.word_wrap", false),
                    toggle("Auto-close brackets", "Typing a bracket or quote adds its closer, and wraps a selection in the pair.", "editor.auto_closing_brackets", true),
                    toggle("Detect indentation", "A file's own indentation wins over the two settings below.", "editor.detect_indentation", true),
                    toggle("Insert spaces", "Tab and Enter indent with spaces rather than tabs.", "editor.insert_spaces", true),
                    Row {
                        label: "Tab size",
                        about: "Spaces an indent level takes.",
                        key: "editor.tab_size",
                        control: Control::Choice(vec!["2", "4", "8"], tab_size),
                        changed: set_here("editor", "tab_size"),
                    },
                    Row {
                        label: "Font size",
                        about: "The code's size, in points. Over what a pack says.",
                        key: "editor.font_size",
                        control: Control::Number(size),
                        changed: set_here("editor", "font_size"),
                    },
                    choice("Line height", "Each row's height, times the font size.", "editor.line_height", vec!["1.3", "1.55", "1.8"], "1.55"),
                    toggle("Font ligatures", "Runs like -> and != join into one picture, in fonts that have them.", "editor.font_ligatures", true),
                    choice("Cursor style", "How the cursor is drawn.", "editor.cursor_style", vec!["line", "line-thin", "block", "block-outline", "underline", "underline-thin"], "line"),
                    choice("Cursor blinking", "How the cursor blinks.", "editor.cursor_blinking", vec!["blink", "smooth", "phase", "expand", "solid"], "blink"),
                ],
            ),
            (
                "Files",
                vec![
                    toggle("Preview tabs", "A single click in the Explorer opens a tab the next click replaces, in italics.", "editor.preview_tabs", true),
                    choice("Auto save", "Files save by themselves: a while after each change, when the code loses the keyboard, or when the window does.", "files.auto_save", vec!["off", "after_delay", "on_focus_change", "on_window_change"], "off"),
                    choice("Auto save delay", "Milliseconds after the last change, with after_delay.", "files.auto_save_delay", vec!["500", "1000", "2000", "5000"], "1000"),
                    toggle("Trim trailing whitespace", "Saving takes spaces and tabs off the ends of lines. Not in Markdown, where they break lines.", "files.trim_trailing_whitespace", false),
                    toggle("Insert final newline", "Saving ends the file with a newline.", "files.insert_final_newline", false),
                    choice("Format on save", "Saving formats the file first, with its language server. Changed lines formats only what git says you changed, so the rest of the file stays as it was.", "editor.format_on_save", vec!["off", "file", "changed_lines"], "off"),
                    Row {
                        label: "Glyph icons",
                        about: "Icons drawn from the code's font: a Nerd Font's, or plain symbols without one.",
                        key: "editor.icons",
                        control: Control::Choice(vec!["nerd", "plain"], icons),
                        changed: set_here("editor", "icons"),
                    },
                ],
            ),
        ];
        let search = ui.search.to_lowercase();
        let rows: Vec<_> = rows
            .into_iter()
            .map(|(title, rows)| {
                let rows: Vec<Row> = rows
                    .into_iter()
                    .filter(|r| {
                        search.is_empty()
                            || r.label.to_lowercase().contains(&search)
                            || r.key.contains(&search)
                    })
                    .collect();
                (title, rows)
            })
            .filter(|(_, rows)| !rows.is_empty())
            .collect();

        let colours = self.choice.colours.clone().unwrap_or_else(|| theme::DEFAULT_NAME.into());
        let icon_theme = self.choice.icons.clone().unwrap_or_else(|| packs::LETTERS.into());
        let themes = self
            .previews
            .iter()
            .map(|(label, palette)| ThemeCard {
                label: label.clone(),
                palette: *palette,
                tag: tag(palette),
                current: *label == colours,
            })
            .collect();
        let icons = self
            .icon_previews
            .iter()
            .map(|(label, icons, from)| IconCard {
                label: label.clone(),
                icons: icons.clone(),
                from: from.clone(),
                current: *label == icon_theme,
            })
            .collect();
        let what = |label: &str| {
            self.packs
                .iter()
                .find(|p| p.label == label)
                .map_or_else(|| "not installed".into(), |p| p.parts().describe())
        };
        let mut layers: Vec<(String, String, bool)> = self
            .choice
            .packs
            .iter()
            .map(|p| (p.clone(), what(p), true))
            .collect();
        layers.push((icon_theme, "icon theme".into(), false));
        layers.push((colours, "colour theme".into(), false));
        layers.push(("merlin".into(), "the base".into(), false));
        let in_use = self.choice.packs.iter().map(|p| (p.clone(), what(p))).collect();
        let available = self
            .packs
            .iter()
            .filter(|p| p.file.is_some() && !self.choice.packs.contains(&p.label))
            .filter(|p| {
                // Colour themes have their own grid; packs list the rest.
                let parts = p.parts();
                parts.layout || parts.fonts || parts.icons
            })
            .map(|p| (p.label.clone(), p.parts().describe()))
            .collect();

        let mut keys: Vec<KeyBinding> = self
            .editor
            .key_bindings()
            .into_iter()
            .filter(|k| match ui.keys {
                KeysFilter::All => true,
                KeysFilter::Changed => k.changed,
                KeysFilter::Plugins => k.plugin.is_some(),
            })
            .filter(|k| {
                search.is_empty()
                    || k.title.to_lowercase().contains(&search)
                    || k.keys.iter().any(|(_, shown)| shown.to_lowercase().contains(&search))
            })
            .collect();
        keys.sort_by(|a, b| a.keys.is_empty().cmp(&b.keys.is_empty()).then(a.title.cmp(&b.title)));
        let key_changes = scope.get("keys").and_then(|t| t.as_table()).map_or(0, |t| t.len());
        let looks_changed = ["theme", "icon_theme", "packs"]
            .iter()
            .filter(|k| set_here("editor", k))
            .count();
        let editor_changed = [
            "minimap",
            "breadcrumbs",
            "sticky_scroll",
            "word_wrap",
            "font_size",
            "preview_tabs",
            "icons",
            "auto_closing_brackets",
            "detect_indentation",
            "insert_spaces",
            "tab_size",
        ]
        .iter()
        .filter(|k| set_here("editor", k))
        .count();
        Some(SettingsView {
            ui: SettingsUiView {
                category: ui.category,
                workspace: ui.workspace,
                search: ui.search.clone(),
                keys: ui.keys,
                capture: ui.capture.clone(),
            },
            changed: [looks_changed, editor_changed, key_changes],
            themes,
            icons,
            layers,
            in_use,
            available,
            rows,
            keys,
            file: self.scope_file(),
        })
    }

    /// Writes a setting to the file being changed, then applies them all.
    fn write(&mut self, key: &str, value: Option<String>) -> Vec<Event> {
        let workspace = self.settings_ui.as_ref().is_some_and(|u| u.workspace);
        if self.scripted {
            return Vec::new();
        }
        let result = match &value {
            Some(v) => self.settings.set_in(workspace, key, v),
            None => self.settings.remove_in(workspace, key),
        };
        let mut events = match result {
            Ok(()) => Vec::new(),
            Err(err) => vec![Event::Error(format!("Can't save {key}: {err}"))],
        };
        events.extend(self.load_settings());
        events
    }

    fn write_packs(&mut self, packs: Vec<String>) -> Vec<Event> {
        let list = toml::Value::Array(packs.into_iter().map(toml::Value::String).collect());
        self.write("editor.packs", Some(list.to_string()))
    }

    pub(crate) fn settings_action(&mut self, action: SettingsAction) -> Vec<Event> {
        let Some(ui) = &mut self.settings_ui else {
            return Vec::new();
        };
        let quote = |s: &str| toml::Value::String(s.into()).to_string();
        match action {
            SettingsAction::Close => {
                self.settings_ui = None;
                Vec::new()
            }
            SettingsAction::Category(c) => {
                ui.category = c;
                ui.scroll = 0.0;
                ui.capture = None;
                Vec::new()
            }
            SettingsAction::Workspace(on) => {
                ui.workspace = on;
                Vec::new()
            }
            SettingsAction::KeysFilter(f) => {
                ui.keys = f;
                Vec::new()
            }
            SettingsAction::Capture(command) => {
                ui.capture = Some(command);
                Vec::new()
            }
            SettingsAction::Theme(name) => self.write("editor.theme", Some(quote(&name))),
            SettingsAction::Icons(name) => self.write("editor.icon_theme", Some(quote(&name))),
            SettingsAction::PackUp(i) => {
                let mut packs = self.choice.packs.clone();
                if i > 0 && i < packs.len() {
                    packs.swap(i, i - 1);
                }
                self.write_packs(packs)
            }
            SettingsAction::PackDown(i) => {
                let mut packs = self.choice.packs.clone();
                if i + 1 < packs.len() {
                    packs.swap(i, i + 1);
                }
                self.write_packs(packs)
            }
            SettingsAction::PackOff(i) => {
                let mut packs = self.choice.packs.clone();
                if i < packs.len() {
                    packs.remove(i);
                }
                self.write_packs(packs)
            }
            SettingsAction::PackOn(label) => {
                let mut packs = self.choice.packs.clone();
                if !packs.contains(&label) {
                    // New packs go on top, as in a resource pack list.
                    packs.insert(0, label);
                }
                self.write_packs(packs)
            }
            SettingsAction::Toggle(key) => {
                let Some(view) = self.settings_view() else {
                    return Vec::new();
                };
                let on = view
                    .rows
                    .iter()
                    .flat_map(|(_, rows)| rows)
                    .find(|r| r.key == key)
                    .is_some_and(|r| r.control == Control::Toggle(true));
                self.write(key, Some((!on).to_string()))
            }
            SettingsAction::Choose(key, value) => self.write(key, Some(quote(value))),
            SettingsAction::Step(key, by) => {
                let Some(view) = self.settings_view() else {
                    return Vec::new();
                };
                let now = view
                    .rows
                    .iter()
                    .flat_map(|(_, rows)| rows)
                    .find_map(|r| match r.control {
                        Control::Number(n) if r.key == key => Some(n),
                        _ => None,
                    })
                    .unwrap_or(FONT_SIZE);
                let next = (now + by).clamp(8.0, 32.0);
                self.write(key, Some(format!("{next:.1}")))
            }
            SettingsAction::Unbind(_, spec) => self.write(&format!("keys.{spec}"), Some("\"\"".into())),
            SettingsAction::ResetKey(command) => {
                let (user, workspace) = self.settings.each();
                let workspace_scope = ui.workspace;
                let table = if workspace_scope { workspace } else { user };
                let binding = self.editor.key_bindings().into_iter().find(|k| k.command == command);
                let mut events = Vec::new();
                if let Some(keys) = table.get("keys").and_then(|t| t.as_table()) {
                    for (spec, value) in keys {
                        let mine = value.as_str() == Some(command.as_str());
                        let unbound_default = value.as_str() == Some("")
                            && binding.as_ref().is_some_and(|b| b.defaults.contains(spec));
                        if mine || unbound_default {
                            events.extend(self.write(&format!("keys.{spec}"), None));
                        }
                    }
                }
                events
            }
            SettingsAction::OpenFile => {
                let workspace = ui.workspace;
                self.settings_ui = None;
                match self.settings.ensure(workspace) {
                    Ok(path) => self.open(&path),
                    Err(err) => vec![Event::Error(format!("Can't open settings: {err}"))],
                }
            }
            SettingsAction::OpenPlugins => {
                self.settings_ui = None;
                vec![Event::Command("view.plugins".into())]
            }
        }
    }

    /// A key pressed while the Settings window is open. Returns whether it
    /// took it.
    pub(crate) fn settings_key(&mut self, key: Key) -> Vec<Event> {
        use merlin_core::KeyCode;
        let Some(ui) = &mut self.settings_ui else {
            return Vec::new();
        };
        if let Some(command) = ui.capture.take() {
            if key.code == KeyCode::Esc {
                return Vec::new();
            }
            let value = toml::Value::String(command).to_string();
            return self.write(&format!("keys.{}", key.spec()), Some(value));
        }
        match key.code {
            KeyCode::Esc if !ui.search.is_empty() => ui.search.clear(),
            KeyCode::Esc => self.settings_ui = None,
            KeyCode::Backspace => {
                ui.search.pop();
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => ui.search.push(c),
            KeyCode::Tab => {
                ui.category = match ui.category {
                    Category::Appearance => Category::Editor,
                    Category::Editor => Category::Keys,
                    Category::Keys => Category::Appearance,
                };
            }
            _ => {}
        }
        Vec::new()
    }

    /// Every colour theme's palette and icon theme's icons, for the
    /// Settings window's previews.
    pub(crate) fn make_previews(&mut self) {
        let base = self.packs.iter().find(|p| p.label == theme::DEFAULT_NAME).cloned();
        let stack = |pack: &Pack| -> Option<Look> {
            match &base {
                Some(b) => Look::stack(&[b, pack]).ok(),
                None => Look::stack(&[pack]).ok(),
            }
        };
        self.previews = self
            .packs
            .iter()
            .filter(|p| p.parts().colours || p.label == theme::DEFAULT_NAME)
            .filter_map(|p| Some((p.label.clone(), stack(p)?.palette)))
            .collect();
        let from = |p: &Pack| {
            p.file
                .as_ref()
                .and_then(|f| self.host.plugins().into_iter().find(|i| f.starts_with(&i.dir)))
                .map_or_else(|| "built in".to_string(), |i| i.title)
        };
        self.icon_previews = self
            .packs
            .iter()
            .filter(|p| p.parts().icons && !p.parts().colours && !is_ui_only(p))
            .filter_map(|p| Some((p.label.clone(), stack(p)?.icons, from(p))))
            .collect();
    }

    /// The Plugins view's list, from what's loaded.
    pub(crate) fn plugin_summaries(&self) -> Event {
        let list = self
            .host
            .plugins()
            .into_iter()
            .map(|p| {
                let (tag, tone) = if p.failed {
                    ("failed", Tone::Error)
                } else if !p.enabled {
                    ("off", Tone::Muted)
                } else if p.waiting {
                    ("needs your OK", Tone::Warning)
                } else if p.data_only {
                    ("data only", Tone::Badge)
                } else if !p.permissions.processes.is_empty() {
                    ("runs programs", Tone::Warning)
                } else {
                    ("sandboxed", Tone::Success)
                };
                PluginSummary {
                    what: describe(&p),
                    name: p.name,
                    title: p.title,
                    tag: tag.into(),
                    tone,
                }
            })
            .collect();
        Event::PluginList(list)
    }

    pub(crate) fn page_view(&self, name: &str) -> Option<PageView> {
        let p = self.host.plugins().into_iter().find(|p| p.name == name)?;
        let changed = |k: &str| p.defaults.get(k) != p.settings.get(k);
        let mut settings: Vec<_> = p
            .settings
            .iter()
            .map(|(k, v)| (k.clone(), v.clone(), changed(k)))
            .collect();
        settings.sort_by(|a, b| a.0.cmp(&b.0));
        Some(PageView {
            commands: self.editor.plugin_commands(&p.name),
            installed: self.install_link(&p).is_some(),
            uninstall_armed: self.uninstall_armed.as_deref() == Some(name),
            running: p
                .processes
                .iter()
                .map(|(prog, pid)| (prog.clone(), *pid, process_memory(*pid)))
                .collect(),
            name: p.name,
            title: p.title,
            dir: p.dir,
            api: p.api,
            data_only: p.data_only,
            enabled: p.enabled,
            waiting: p.waiting,
            failed: p.failed,
            read: p.permissions.editor_read,
            write: p.permissions.editor_write,
            processes: p.permissions.processes,
            memory: p.memory,
            busy: p.busy,
            languages: p.languages,
            looks: p.looks,
            settings,
        })
    }

    /// The link `merlin plugin install` made for a plugin, if it has one.
    fn install_link(&self, p: &PluginInfo) -> Option<PathBuf> {
        let link = settings::plugins_dir()?.join(&p.name);
        let target = link.read_link().ok()?;
        (target.canonicalize().ok()? == p.dir.canonicalize().ok()?).then_some(link)
    }

    pub(crate) fn page_action(&mut self, action: PageAction) -> Vec<Event> {
        let mut events = Vec::new();
        match action {
            PageAction::Enable(name, on) => {
                events.extend(self.host.set_enabled(&name, on));
                if !self.scripted {
                    let mut off = settings::editor_list(&self.settings.load().0, "disabled_plugins");
                    off.retain(|n| *n != name);
                    if !on {
                        off.push(name.clone());
                    }
                    let list = toml::Value::Array(off.into_iter().map(toml::Value::String).collect());
                    if let Err(err) = self.settings.set("editor.disabled_plugins", &list.to_string()) {
                        events.push(Event::Error(format!("Can't save that: {err}")));
                    }
                }
            }
            PageAction::Reload(name) => events.extend(self.host.reload(&name)),
            PageAction::Output => events.push(Event::Command("view.output".into())),
            PageAction::Uninstall(name) => {
                if self.uninstall_armed.as_deref() != Some(name.as_str()) {
                    self.uninstall_armed = Some(name);
                    return events;
                }
                self.uninstall_armed = None;
                let Some(info) = self.host.plugins().into_iter().find(|p| p.name == name) else {
                    return events;
                };
                match self.install_link(&info).map(std::fs::remove_file) {
                    Some(Ok(())) => {
                        events.extend(self.host.unload(&name));
                        events.push(Event::Notify(format!(
                            "Uninstalled {}. Its files are still in {}.",
                            info.title,
                            info.dir.display()
                        )));
                        events.extend(self.refresh_looks());
                    }
                    Some(Err(err)) => events.push(Event::Error(format!("Can't uninstall: {err}"))),
                    None => events.push(Event::Error(format!(
                        "{} wasn't installed with merlin plugin install, so there's nothing to remove",
                        info.title
                    ))),
                }
            }
            PageAction::Setting(plugin, key, value) => {
                if !self.scripted {
                    let path = format!("plugins.{plugin}.{key}");
                    if let Err(err) = self.settings.set(&path, &value.to_string()) {
                        events.push(Event::Error(format!("Can't save {path}: {err}")));
                    }
                    events.extend(self.load_settings());
                }
            }
        }
        events.push(self.plugin_summaries());
        events
    }
}

/// A pack with only the frame's icons, which layers over an icon theme
/// rather than being one.
fn is_ui_only(p: &Pack) -> bool {
    let icons = p.table.get("icons").and_then(|v| v.as_table());
    icons.is_some_and(|t| t.keys().all(|k| k == "ui"))
}

/// A few words on what a theme is like, for its card.
fn tag(p: &Palette) -> &'static str {
    if theme::is_light(p) {
        "light"
    } else {
        ""
    }
}

/// What a plugin adds, in a few words.
fn describe(p: &PluginInfo) -> String {
    let mut parts = Vec::new();
    if !p.languages.is_empty() {
        parts.push(p.languages.join(", "));
    }
    if !p.looks.is_empty() {
        let n = p.looks.len();
        parts.push(format!("{n} look{}", if n == 1 { "" } else { "s" }));
    }
    if !p.permissions.processes.is_empty() {
        parts.push("language servers".into());
    }
    if parts.is_empty() {
        "commands and panels".into()
    } else {
        parts.join(" · ")
    }
}

/// A process's resident memory in bytes, where the system says.
fn process_memory(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}
