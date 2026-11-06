//! The Settings tab, as the app keeps it, and plugin pages' actions. The
//! window draws it from [`SettingsView`], and says what was clicked with a
//! [`SettingsAction`]. Changes go to the user's or the workspace's
//! settings file, and apply at once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use merlin_core::panel::Tone;
use merlin_core::{Event, Key, KeyBinding, KeyCode, PluginSummary, ThemeKind};
use merlin_plugins::PluginInfo;

use crate::packs::{self, Icons, Look, Pack};
use crate::theme::{self, Palette};
use crate::{App, settings};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    Appearance,
    Editor,
    Files,
    Keys,
    Plugins,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Appearance,
        Category::Editor,
        Category::Files,
        Category::Keys,
        Category::Plugins,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::Editor => "Text Editor",
            Category::Files => "Files",
            Category::Keys => "Keyboard Shortcuts",
            Category::Plugins => "Plugins",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeysFilter {
    #[default]
    All,
    Changed,
    Plugins,
    Conflicts,
}

impl KeysFilter {
    pub const ALL: [KeysFilter; 4] = [
        KeysFilter::All,
        KeysFilter::Changed,
        KeysFilter::Plugins,
        KeysFilter::Conflicts,
    ];

    pub fn title(self) -> &'static str {
        match self {
            KeysFilter::All => "All",
            KeysFilter::Changed => "Changed",
            KeysFilter::Plugins => "Plugins",
            KeysFilter::Conflicts => "Conflicts",
        }
    }
}

/// Keys being pressed for a command: one, or two in a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    pub command: String,
    pub keys: Vec<Key>,
}

/// The Settings tab's state.
#[derive(Clone, Debug, Default)]
pub struct SettingsUi {
    pub category: Category,
    /// Changes go to the workspace's settings instead of the user's.
    pub workspace: bool,
    pub search: String,
    pub keys: KeysFilter,
    /// The command waiting for keys to be pressed for it.
    pub capture: Option<Capture>,
    /// The search box catches keys, to find what they run.
    pub record: bool,
    /// The keys row picked, by its place in the list shown.
    pub key_row: Option<usize>,
    /// The plugin whose page shows.
    pub plugin: Option<String>,
    /// The choice whose list is open, by its key.
    pub open_choice: Option<&'static str>,
    /// The theme card the arrows are on, which the window previews.
    pub card: Option<usize>,
    pub scroll: f32,
    /// How far the page scrolls, as last drawn.
    pub room: f32,
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
    /// Set in the workspace's file.
    pub in_workspace: bool,
    /// What the other file says, when the workspace's sets it.
    pub note: Option<String>,
}

/// A heading's rows, on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub category: Category,
    pub title: &'static str,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeCard {
    pub label: String,
    pub palette: Palette,
    pub light: bool,
    /// The mark it has: "✓", or with Follow the system, "✓ Dark" or
    /// "✓ Light".
    pub mark: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IconCard {
    pub label: String,
    pub icons: Icons,
    pub from: String,
    pub current: bool,
}

/// A keys row: a command, and whether its keys are another's too.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyRow {
    pub binding: KeyBinding,
    pub conflict: bool,
}

/// The card catching keys for a command.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureView {
    pub title: String,
    /// The keys pressed so far, as people read them.
    pub keys: Vec<String>,
    /// The command that has these keys now.
    pub clash: Option<String>,
    /// The command has keys already, which Replace drops.
    pub had: bool,
}

/// A plugin in the Plugins page's list.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginRow {
    pub name: String,
    pub title: String,
    pub on: bool,
    pub failed: bool,
    /// Bytes: its own, and its programs'.
    pub memory: u64,
    pub data_only: bool,
}

/// What the Settings tab shows.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsView {
    pub ui: SettingsUiView,
    /// Settings changed in the file being changed, by category.
    pub changed: [usize; 5],
    pub themes: Vec<ThemeCard>,
    pub follow: bool,
    pub icons: Vec<IconCard>,
    /// The layouts and other packs stacked, top first, with what each
    /// changes; and the ones not in use.
    pub layers: Vec<(String, String)>,
    pub available: Vec<(String, String)>,
    pub sections: Vec<Section>,
    pub keys: Vec<KeyRow>,
    /// How many each keys filter lists.
    pub key_counts: [usize; 4],
    pub capture: Option<CaptureView>,
    pub plugins: Vec<PluginRow>,
    pub page: Option<PageView>,
    pub file: PathBuf,
    /// The workspace's folder, by name.
    pub folder: String,
    /// What's wrong with a settings file that can't be read.
    pub error: Option<String>,
}

/// The parts of [`SettingsUi`] drawing needs, comparable for redraws.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsUiView {
    pub category: Category,
    pub workspace: bool,
    pub search: String,
    pub keys: KeysFilter,
    pub record: bool,
    pub key_row: Option<usize>,
    pub open_choice: Option<&'static str>,
    pub card: Option<usize>,
    pub scroll: f32,
}

/// A click in the Settings tab.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsAction {
    Category(Category),
    Workspace(bool),
    Theme(String),
    Follow,
    Icons(String),
    PackUp(usize),
    PackDown(usize),
    PackOff(usize),
    PackOn(String),
    Toggle(&'static str),
    OpenChoice(&'static str),
    Choose(&'static str, &'static str),
    Step(&'static str, f32),
    Reset(&'static str),
    ClearSearch,
    KeysFilter(KeysFilter),
    Record,
    /// A keys row clicked: its place, and how many clicks.
    KeyRow(usize, u8),
    Capture(String),
    CaptureDone {
        replace: bool,
    },
    Unbind(String),
    ResetKey(String),
    Plugin(String),
    Page(PageAction),
    OpenFile,
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

/// The settings as last read: merged, then each file on its own, and what
/// was wrong with them.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub merged: toml::Table,
    pub user: toml::Table,
    pub workspace: toml::Table,
    pub errors: Vec<String>,
}

/// Programs' memory, read from the system at most every few seconds.
#[derive(Debug, Default)]
pub struct Memory(RefCell<HashMap<u32, (Instant, Option<u64>)>>);

/// How often the Plugins page reads programs' memory again.
pub const MEMORY_EVERY: Duration = Duration::from_secs(3);

impl Memory {
    fn of(&self, pid: u32) -> Option<u64> {
        let mut seen = self.0.borrow_mut();
        let now = Instant::now();
        match seen.get(&pid) {
            Some((when, bytes)) if now.duration_since(*when) < MEMORY_EVERY => *bytes,
            _ => {
                let bytes = process_memory(pid);
                seen.insert(pid, (now, bytes));
                bytes
            }
        }
    }
}

const FONT_SIZE: f32 = 13.5;

/// What a row's control is, with its default.
enum Kind {
    Bool(bool),
    Pick(Vec<&'static str>, &'static str),
    Num(f32),
}

/// How a setting's value reads in a note.
fn shown(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Boolean(true) => "on".into(),
        toml::Value::Boolean(false) => "off".into(),
        other => other.to_string(),
    }
}

/// `editor.minimap` is `minimap` in `[editor]`.
fn part(key: &str) -> (&str, &str) {
    key.split_once('.').unwrap_or(("editor", key))
}

fn get<'a>(table: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let (section, name) = part(key);
    table.get(section)?.as_table()?.get(name)
}

impl App {
    /// The Settings tab opened, at a plugin's page if one is named.
    pub(crate) fn open_settings(&mut self, plugin: Option<String>) {
        self.make_previews();
        if let Some(name) = plugin {
            self.settings_ui.category = Category::Plugins;
            self.settings_ui.plugin = Some(name);
            self.settings_ui.search.clear();
            self.settings_ui.scroll = 0.0;
        }
    }

    /// The file changes go to.
    fn scope_file(&self) -> PathBuf {
        if self.settings_ui.workspace {
            self.settings.workspace.clone()
        } else {
            self.settings.user.clone()
        }
    }

    /// Every setting the tab shows, by page and heading.
    fn sections(&self) -> Vec<Section> {
        let ui = &self.settings_ui;
        let Loaded {
            user, workspace, ..
        } = &self.loaded;
        let scope = if ui.workspace { workspace } else { user };
        let row = |label: &'static str, about: &'static str, key: &'static str, kind: Kind| {
            // The file's own value, or on the Workspace tab yours under it.
            let value = get(scope, key).or_else(|| get(user, key).filter(|_| ui.workspace));
            let in_workspace = get(workspace, key).is_some();
            let control = match kind {
                Kind::Bool(d) => Control::Toggle(value.and_then(|v| v.as_bool()).unwrap_or(d)),
                Kind::Pick(options, d) => {
                    Control::Choice(options, value.map_or_else(|| d.to_string(), shown))
                }
                Kind::Num(d) => Control::Number(
                    value
                        .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
                        .map_or(d, |f| f as f32),
                ),
            };
            let note = match (in_workspace, ui.workspace) {
                (true, true) => {
                    let w = get(workspace, key).map(shown).unwrap_or_default();
                    Some(match get(user, key) {
                        Some(u) => format!("Yours is {}; this folder says {w}.", shown(u)),
                        None => format!("This folder says {w}; yours is the default."),
                    })
                }
                (true, false) => {
                    let w = get(workspace, key).map(shown).unwrap_or_default();
                    Some(format!("This folder says {w}, which wins in it."))
                }
                _ => None,
            };
            Row {
                label,
                about,
                key,
                control,
                changed: get(scope, key).is_some(),
                in_workspace,
                note,
            }
        };
        let on = Kind::Bool;
        let pick = Kind::Pick;
        #[rustfmt::skip]
        let sections = vec![
            Section {
                category: Category::Editor,
                title: "Font",
                rows: vec![
                    row("Font size", "The code's size, in points. Ctrl+= and Ctrl+- change it for now.", "editor.font_size", Kind::Num(FONT_SIZE)),
                    row("Line height", "Each row's height, times the font size.", "editor.line_height", pick(vec!["1.3", "1.55", "1.8"], "1.55")),
                    row("Font ligatures", "Runs like -> and != join into one picture, in fonts that have them.", "editor.font_ligatures", on(true)),
                    row("Interface text size", "Everything around the code grows with it; the code keeps its own size.", "editor.ui_font_size", pick(vec!["11", "12", "13", "14", "15", "16"], "13")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Cursor",
                rows: vec![
                    row("Cursor style", "How the cursor is drawn.", "editor.cursor_style", pick(vec!["line", "line-thin", "block", "block-outline", "underline", "underline-thin"], "line")),
                    row("Cursor blinking", "How the cursor blinks.", "editor.cursor_blinking", pick(vec!["blink", "smooth", "phase", "expand", "solid"], "blink")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Text",
                rows: vec![
                    row("Word wrap", "Long lines wrap at spaces instead of scrolling sideways. Alt+Z", "editor.word_wrap", on(false)),
                    row("Auto-close brackets", "Typing a bracket or quote adds its closer, and wraps a selection in the pair.", "editor.auto_closing_brackets", on(true)),
                    row("Detect indentation", "A file's own indentation wins over the two settings below.", "editor.detect_indentation", on(true)),
                    row("Insert spaces", "Tab and Enter indent with spaces rather than tabs.", "editor.insert_spaces", on(true)),
                    row("Tab size", "Spaces an indent level takes.", "editor.tab_size", pick(vec!["2", "4", "8"], "4")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Display",
                rows: vec![
                    row("Minimap", "The file's shape beside the scrollbar, with matches, problems and changes marked.", "editor.minimap", on(false)),
                    row("Minimap scale", "How big the minimap draws the file.", "editor.minimap_scale", pick(vec!["1", "2", "3"], "1")),
                    row("Breadcrumbs", "The file's folders and the symbols around the cursor, under the tabs.", "editor.breadcrumbs", on(true)),
                    row("Sticky scroll", "The first lines of the blocks in view stay pinned at the top.", "editor.sticky_scroll", on(true)),
                    row("Bracket pair colours", "Brackets are coloured by how deep they are.", "editor.bracket_pair_colorization", on(true)),
                    row("Colour swatches", "A bar in the colour under colours written in the code, such as #e6c07b.", "editor.color_decorators", on(true)),
                    row("Problems inline", "A line's worst error or warning is written after the end of the line.", "editor.inline_diagnostics", on(true)),
                    row("Inlay hints", "Types and parameter names from the language server, in quiet chips.", "editor.inlay_hints", on(true)),
                    row("Glyph icons", "Icons drawn from the code's font: a Nerd Font's, or plain symbols without one.", "editor.icons", pick(vec!["nerd", "plain"], "nerd")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Scrolling",
                rows: vec![
                    row("Smooth scrolling", "A wheel's steps glide to where they scroll.", "editor.smooth_scrolling", on(true)),
                    row("Scroll beyond last line", "The last line can scroll up to the top of the view.", "editor.scroll_beyond_last_line", on(true)),
                    row("Wheel sensitivity", "How far the wheel and touchpad scroll, times their own.", "editor.mouse_wheel_scroll_sensitivity", pick(vec!["0.5", "1", "2", "3"], "1")),
                    row("Fast scroll sensitivity", "Times that again with Alt held.", "editor.fast_scroll_sensitivity", pick(vec!["3", "5", "10"], "5")),
                    row("Scrollbar size", "The scrollbar's width, in points.", "editor.scrollbar_size", pick(vec!["10", "14", "18"], "14")),
                ],
            },
            Section {
                category: Category::Files,
                title: "Saving",
                rows: vec![
                    row("Auto save", "Files save by themselves: a while after each change, when the code loses the keyboard, or when the window does.", "files.auto_save", pick(vec!["off", "after_delay", "on_focus_change", "on_window_change"], "off")),
                    row("Auto save delay", "Milliseconds after the last change, with After delay.", "files.auto_save_delay", pick(vec!["500", "1000", "2000", "5000"], "1000")),
                    row("Format on save", "Saving formats the file first, with its language server. Changed lines formats only what git says you changed.", "editor.format_on_save", pick(vec!["off", "file", "changed_lines"], "off")),
                    row("Trim trailing whitespace", "Saving takes spaces and tabs off the ends of lines. Not in Markdown, where they break lines.", "files.trim_trailing_whitespace", on(false)),
                    row("Insert final newline", "Saving ends the file with a newline.", "files.insert_final_newline", on(false)),
                    row("Hot exit", "Quit without asking; unsaved files come back next time.", "files.hot_exit", on(true)),
                ],
            },
            Section {
                category: Category::Keys,
                title: "Keymap",
                rows: vec![
                    row("Keymap", "How keys work in the text. The rest of the window keeps its shortcuts, and the status bar's Keymap item switches.", "editor.keymap", pick(vec!["vscode", "vim"], "vscode")),
                    row("Leave Insert mode with", "Two letters typed quickly, as well as Esc.", "vim.insert_escape", pick(vec!["off", "jk", "jj", "kj"], "off")),
                    row("Yank to the clipboard", "y, d and p use the system clipboard, as clipboard=unnamedplus.", "vim.clipboard", on(false)),
                    row("Relative line numbers", "Counted from the cursor's line, in Normal mode.", "editor.line_numbers", pick(vec!["on", "relative"], "on")),
                ],
            },
            Section {
                category: Category::Files,
                title: "Tabs",
                rows: vec![
                    row("Preview tabs", "A single click in the Explorer opens a tab the next click replaces, in italics.", "editor.preview_tabs", on(true)),
                ],
            },
        ];
        sections
    }

    pub(crate) fn settings_view(&self) -> SettingsView {
        let ui = &self.settings_ui;
        let Loaded {
            user,
            workspace,
            errors,
            ..
        } = &self.loaded;
        let scope = if ui.workspace { workspace } else { user };
        let search = ui.search.to_lowercase();
        let searching = !search.is_empty() && !ui.record;

        let all = self.sections();
        let mut changed = [0; 5];
        for s in &all {
            changed[s.category.index()] += s.rows.iter().filter(|r| r.changed).count();
        }
        changed[Category::Appearance.index()] = [
            "theme",
            "theme_light",
            "theme_follows_system",
            "icon_theme",
            "packs",
        ]
        .iter()
        .filter(|k| get(scope, k).is_some())
        .count();
        changed[Category::Keys.index()] += scope
            .get("keys")
            .and_then(|t| t.as_table())
            .map_or(0, |t| t.len());
        changed[Category::Plugins.index()] = scope
            .get("plugins")
            .and_then(|t| t.as_table())
            .map_or(0, |t| t.len());
        let sections = all
            .into_iter()
            .filter(|s| searching || s.category == ui.category)
            .map(|mut s| {
                if searching {
                    s.rows.retain(|r| {
                        r.label.to_lowercase().contains(&search)
                            || r.about.to_lowercase().contains(&search)
                            || r.key.contains(&search)
                    });
                }
                s
            })
            .filter(|s| !s.rows.is_empty())
            .collect();

        let follow = get(&self.loaded.merged, "theme_follows_system")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let (dark, light) = self.theme_pair();
        let themes = self
            .previews
            .iter()
            .map(|(label, palette)| {
                let mark = if follow {
                    if *label == dark {
                        Some("✓ Dark")
                    } else if *label == light {
                        Some("✓ Light")
                    } else {
                        None
                    }
                } else {
                    (self
                        .choice
                        .colours
                        .as_deref()
                        .unwrap_or(theme::DEFAULT_NAME)
                        == label)
                        .then_some("✓")
                };
                ThemeCard {
                    label: label.clone(),
                    palette: *palette,
                    light: theme::is_light(palette),
                    mark,
                }
            })
            .collect();
        let icon_theme = self
            .choice
            .icons
            .clone()
            .unwrap_or_else(|| packs::LETTERS.into());
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
        let layers = self
            .choice
            .packs
            .iter()
            .map(|p| (p.clone(), what(p)))
            .collect();
        let available = self
            .packs
            .iter()
            .filter(|p| p.file.is_some() && !self.choice.packs.contains(&p.label))
            .filter(|p| {
                // Colour and icon themes have their own cards; a pack of
                // only the frame's icons stacks as layouts do.
                let parts = p.parts();
                (parts.layout || parts.fonts || is_ui_only(p)) && !parts.colours
            })
            .map(|p| (p.label.clone(), p.parts().describe()))
            .collect();

        // Keys are many: worked out only where they show.
        let (keys, key_counts) = if ui.category == Category::Keys || searching {
            self.key_rows(&search)
        } else {
            (Vec::new(), [0; 4])
        };
        let capture = ui.capture.as_ref().map(|c| {
            let bindings = self.editor.key_bindings();
            let spec: Vec<String> = c.keys.iter().map(Key::spec).collect();
            let spec = spec.join(" ");
            let clash = bindings
                .iter()
                .find(|b| b.command != c.command && b.keys.iter().any(|(s, _)| *s == spec))
                .filter(|_| !c.keys.is_empty())
                .map(|b| b.title.clone());
            let mine = bindings.iter().find(|b| b.command == c.command);
            CaptureView {
                title: mine.map_or_else(|| c.command.clone(), |b| b.title.clone()),
                keys: c.keys.iter().map(Key::to_string).collect(),
                clash,
                had: mine.is_some_and(|b| !b.keys.is_empty()),
            }
        });

        let plugins: Vec<PluginRow> = if ui.category == Category::Plugins {
            self.host
                .plugins()
                .into_iter()
                .map(|p| PluginRow {
                    memory: p.memory as u64
                        + p.processes
                            .iter()
                            .filter_map(|(_, pid)| self.memory.of(*pid))
                            .sum::<u64>(),
                    name: p.name,
                    title: p.title,
                    on: p.enabled,
                    failed: p.failed,
                    data_only: p.data_only,
                })
                .collect()
        } else {
            Vec::new()
        };
        let page = if ui.category == Category::Plugins {
            let name = ui
                .plugin
                .clone()
                .filter(|n| plugins.iter().any(|p| p.name == *n))
                .or_else(|| plugins.first().map(|p| p.name.clone()));
            name.and_then(|n| self.page_view(&n))
        } else {
            None
        };

        SettingsView {
            ui: SettingsUiView {
                category: ui.category,
                workspace: ui.workspace,
                search: ui.search.clone(),
                keys: ui.keys,
                record: ui.record,
                key_row: ui.key_row,
                open_choice: ui.open_choice,
                card: ui.card,
                scroll: ui.scroll,
            },
            changed,
            themes,
            follow,
            icons,
            layers,
            available,
            sections,
            keys,
            key_counts,
            capture,
            plugins,
            page,
            file: self.scope_file(),
            folder: self.root.file_name().map_or_else(
                || "this folder".into(),
                |n| n.to_string_lossy().into_owned(),
            ),
            // The file by its name, or the folder's by where it is in it.
            error: errors.first().map(|e| {
                let (user, workspace) = (&self.settings.user, &self.settings.workspace);
                let short = workspace.strip_prefix(&self.root).map_or_else(
                    |_| workspace.display().to_string(),
                    |p| p.display().to_string(),
                );
                e.replace(&workspace.display().to_string(), &short).replace(
                    &user.display().to_string(),
                    &user
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                )
            }),
        }
    }

    /// The keys rows the filter and search leave, and how many each filter
    /// would list.
    fn key_rows(&self, search: &str) -> (Vec<KeyRow>, [usize; 4]) {
        let ui = &self.settings_ui;
        let bindings = self.editor.key_bindings();
        // Keys two commands have, where the settings or a plugin gave one
        // of them: merlin's own share keys only where they can't clash.
        let mut owners: HashMap<&str, Vec<&KeyBinding>> = HashMap::new();
        for b in &bindings {
            for (spec, _) in &b.keys {
                owners.entry(spec).or_default().push(b);
            }
        }
        let clashes = |b: &KeyBinding| {
            b.keys.iter().any(|(spec, _)| {
                let o = &owners[spec.as_str()];
                o.len() > 1 && o.iter().any(|b| b.changed || b.plugin.is_some())
            })
        };
        let mut rows: Vec<KeyRow> = bindings
            .iter()
            .map(|b| KeyRow {
                conflict: clashes(b),
                binding: b.clone(),
            })
            .collect();
        let found = |r: &KeyRow| {
            let b = &r.binding;
            if search.is_empty() {
                true
            } else if ui.record {
                // A key caught: what it runs, alone or as a chord's start.
                b.keys.iter().any(|(_, shown)| {
                    let shown = shown.to_lowercase();
                    shown == search || shown.starts_with(&format!("{search} "))
                })
            } else {
                b.title.to_lowercase().contains(search)
                    || b.command.contains(search)
                    || b.keys
                        .iter()
                        .any(|(_, shown)| shown.to_lowercase().contains(search))
            }
        };
        rows.retain(found);
        let count = |f: KeysFilter| rows.iter().filter(|r| keys_filter(f, r)).count();
        let counts = KeysFilter::ALL.map(count);
        rows.retain(|r| keys_filter(ui.keys, r));
        rows.sort_by(|a, b| {
            let (a, b) = (&a.binding, &b.binding);
            a.keys
                .is_empty()
                .cmp(&b.keys.is_empty())
                .then(a.title.cmp(&b.title))
        });
        (rows, counts)
    }

    /// The colour themes for a dark desktop and a light one.
    fn theme_pair(&self) -> (String, String) {
        let merged = &self.loaded.merged;
        let dark = settings::editor_text(merged, "theme")
            .unwrap_or(theme::DEFAULT_NAME)
            .to_string();
        let light = settings::editor_text(merged, "theme_light")
            .map(str::to_string)
            .or_else(|| {
                self.previews
                    .iter()
                    .find(|(_, p)| theme::is_light(p))
                    .map(|(l, _)| l.clone())
            })
            .unwrap_or_else(|| dark.clone());
        (dark, light)
    }

    /// The colour theme to use now: with Follow the system, the one for
    /// the desktop's light or dark.
    pub(crate) fn chosen_colours(&self) -> Option<String> {
        let merged = &self.loaded.merged;
        let follow = get(merged, "theme_follows_system")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if follow && self.system_light {
            Some(self.theme_pair().1)
        } else {
            settings::editor_text(merged, "theme").map(str::to_string)
        }
    }

    /// Asks the desktop whether it wants light colours, off the window's
    /// thread, when a theme follows it; and changes the theme if that
    /// changed.
    pub(crate) fn check_system_theme(&mut self) {
        let follow = get(&self.loaded.merged, "theme_follows_system")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !follow || self.scripted {
            return;
        }
        self.jobs.run(crate::jobs::Lane::Ask, move || {
            let light = system_light();
            Box::new(move |app: &mut App| match light {
                Some(light) if light != app.system_light => {
                    app.system_light = light;
                    app.load_settings()
                }
                _ => Vec::new(),
            })
        });
    }

    /// Writes a setting to the file being changed, then applies them all.
    fn write(&mut self, key: &str, value: Option<String>) -> Vec<Event> {
        if self.scripted {
            return Vec::new();
        }
        let workspace = self.settings_ui.workspace;
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

    /// The window previews the theme the arrows or the pointer are on, or
    /// nothing.
    pub(crate) fn preview_card(&mut self, name: Option<String>) -> Vec<Event> {
        if self.choice.preview[ThemeKind::Colours as usize] == name {
            return Vec::new();
        }
        match name {
            Some(name) => self.preview(ThemeKind::Colours, name).into_iter().collect(),
            None => {
                self.choice.preview[ThemeKind::Colours as usize] = None;
                self.restack().err().map(Event::Error).into_iter().collect()
            }
        }
    }

    pub(crate) fn settings_action(&mut self, action: SettingsAction) -> Vec<Event> {
        let quote = |s: &str| toml::Value::String(s.into()).to_string();
        let ui = &mut self.settings_ui;
        if !matches!(action, SettingsAction::Choose(..)) {
            ui.open_choice = None;
        }
        match action {
            SettingsAction::Category(c) => {
                ui.category = c;
                ui.scroll = 0.0;
                ui.capture = None;
                ui.record = false;
                ui.key_row = None;
                ui.card = None;
                ui.search.clear();
                Vec::new()
            }
            SettingsAction::Workspace(on) => {
                ui.workspace = on;
                Vec::new()
            }
            SettingsAction::ClearSearch => {
                ui.search.clear();
                ui.record = false;
                Vec::new()
            }
            SettingsAction::KeysFilter(f) => {
                ui.keys = f;
                ui.key_row = None;
                ui.scroll = 0.0;
                Vec::new()
            }
            SettingsAction::Record => {
                ui.record = !ui.record;
                ui.search.clear();
                Vec::new()
            }
            SettingsAction::KeyRow(i, clicks) => {
                ui.key_row = Some(i);
                if clicks >= 2 {
                    return self.capture_row();
                }
                Vec::new()
            }
            SettingsAction::Capture(command) => {
                ui.capture = Some(Capture {
                    command,
                    keys: Vec::new(),
                });
                Vec::new()
            }
            SettingsAction::CaptureDone { replace } => self.capture_done(replace),
            SettingsAction::Plugin(name) => {
                ui.plugin = Some(name);
                self.uninstall_armed = None;
                Vec::new()
            }
            SettingsAction::OpenChoice(key) => {
                ui.open_choice = Some(key);
                Vec::new()
            }
            SettingsAction::Theme(name) => {
                ui.card = None;
                self.choice.preview[ThemeKind::Colours as usize] = None;
                let follow = get(&self.loaded.merged, "theme_follows_system")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let light = self
                    .previews
                    .iter()
                    .any(|(l, p)| *l == name && theme::is_light(p));
                let key = if follow && light {
                    "editor.theme_light"
                } else {
                    "editor.theme"
                };
                self.write(key, Some(quote(&name)))
            }
            SettingsAction::Follow => {
                let on = get(&self.loaded.merged, "theme_follows_system")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let events = self.write("editor.theme_follows_system", Some((!on).to_string()));
                self.check_system_theme();
                events
            }
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
                let on = self
                    .sections()
                    .iter()
                    .flat_map(|s| &s.rows)
                    .any(|r| r.key == key && r.control == Control::Toggle(true));
                self.write(key, Some((!on).to_string()))
            }
            SettingsAction::Choose(key, value) => {
                self.settings_ui.open_choice = None;
                // Numbers are written as numbers.
                let value = match value.parse::<f64>() {
                    Ok(_) => value.to_string(),
                    Err(_) => quote(value),
                };
                self.write(key, Some(value))
            }
            SettingsAction::Step(key, by) => {
                let now = self
                    .sections()
                    .iter()
                    .flat_map(|s| &s.rows)
                    .find_map(|r| match r.control {
                        Control::Number(n) if r.key == key => Some(n),
                        _ => None,
                    })
                    .unwrap_or(FONT_SIZE);
                let next = (now + by).clamp(8.0, 32.0);
                self.write(key, Some(format!("{next:.1}")))
            }
            SettingsAction::Reset(key) => self.write(key, None),
            SettingsAction::Unbind(command) => self.unbind(&command),
            SettingsAction::ResetKey(command) => self.reset_key(&command),
            SettingsAction::Page(action) => self.page_action(action),
            SettingsAction::OpenFile => match self.settings.ensure(self.settings_ui.workspace) {
                Ok(path) => self.open(&path),
                Err(err) => vec![Event::Error(format!("Can't open settings: {err}"))],
            },
        }
    }

    /// Starts catching keys for the keys row picked.
    fn capture_row(&mut self) -> Vec<Event> {
        let (rows, _) = self.key_rows(&self.settings_ui.search.to_lowercase());
        if let Some(row) = self.settings_ui.key_row.and_then(|i| rows.get(i)) {
            self.settings_ui.capture = Some(Capture {
                command: row.binding.command.clone(),
                keys: Vec::new(),
            });
        }
        Vec::new()
    }

    /// The keys caught go to the command, with its others or instead.
    fn capture_done(&mut self, replace: bool) -> Vec<Event> {
        let Some(c) = self.settings_ui.capture.take() else {
            return Vec::new();
        };
        if c.keys.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();
        if replace {
            events.extend(self.unbind(&c.command));
        }
        let spec: Vec<String> = c.keys.iter().map(Key::spec).collect();
        let value = toml::Value::String(c.command).to_string();
        events.extend(self.write(&format!("keys.{}", spec.join(" ")), Some(value)));
        events
    }

    /// Takes a command's keys off it: the settings' own come out of the
    /// file, and merlin's or a plugin's are bound to nothing.
    fn unbind(&mut self, command: &str) -> Vec<Event> {
        let Some(binding) = self
            .editor
            .key_bindings()
            .into_iter()
            .find(|k| k.command == command)
        else {
            return Vec::new();
        };
        let scope = if self.settings_ui.workspace {
            &self.loaded.workspace
        } else {
            &self.loaded.user
        };
        let mine: Vec<String> = scope
            .get("keys")
            .and_then(|t| t.as_table())
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();
        let mut events = Vec::new();
        for (spec, _) in binding.keys {
            let value = if mine.contains(&spec) {
                None
            } else {
                Some("\"\"".to_string())
            };
            events.extend(self.write(&format!("keys.{spec}"), value));
        }
        events
    }

    /// A command's keys back to merlin's: what the settings say about it
    /// comes out of the file.
    fn reset_key(&mut self, command: &str) -> Vec<Event> {
        let binding = self
            .editor
            .key_bindings()
            .into_iter()
            .find(|k| k.command == command);
        let scope = if self.settings_ui.workspace {
            &self.loaded.workspace
        } else {
            &self.loaded.user
        };
        let specs: Vec<String> = scope
            .get("keys")
            .and_then(|t| t.as_table())
            .map(|keys| {
                keys.iter()
                    .filter(|(spec, value)| {
                        let to = match value {
                            toml::Value::Table(t) => t.get("command").and_then(|c| c.as_str()),
                            other => other.as_str(),
                        };
                        // Its own, and its default keys, which another may
                        // have been given or which may be bound to nothing.
                        to == Some(command)
                            || binding.as_ref().is_some_and(|b| b.defaults.contains(spec))
                    })
                    .map(|(spec, _)| spec.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut events = Vec::new();
        for spec in specs {
            events.extend(self.write(&format!("keys.{spec}"), None));
        }
        events
    }

    /// A key pressed while the Settings tab has the keyboard, or `None`
    /// for the editor to have it, as Ctrl+P and Ctrl+W are.
    pub(crate) fn settings_key(&mut self, key: Key) -> Option<Vec<Event>> {
        let plain = !key.ctrl && !key.alt;
        let ui = &mut self.settings_ui;
        if let Some(c) = &mut ui.capture {
            match key.code {
                KeyCode::Esc if plain => ui.capture = None,
                KeyCode::Enter if plain && !c.keys.is_empty() => {
                    return Some(self.capture_done(false));
                }
                KeyCode::Backspace if plain => c.keys.clear(),
                _ => {
                    // A third key starts again.
                    if c.keys.len() >= 2 {
                        c.keys.clear();
                    }
                    c.keys.push(key);
                }
            }
            return Some(Vec::new());
        }
        if ui.record {
            match key.code {
                KeyCode::Esc if plain => {
                    ui.record = false;
                    ui.search.clear();
                }
                KeyCode::Backspace if plain => ui.search.clear(),
                _ => ui.search = key.to_string().to_lowercase(),
            }
            ui.key_row = None;
            return Some(Vec::new());
        }
        if !plain {
            // Ctrl+F: the search box has the typing already.
            return (key.ctrl && key.code == KeyCode::Char('f')).then(Vec::new);
        }
        let mut events = Vec::new();
        match key.code {
            KeyCode::Char(c) => {
                ui.search.push(c);
                ui.scroll = 0.0;
                ui.key_row = None;
            }
            KeyCode::Backspace => {
                ui.search.pop();
                ui.key_row = None;
            }
            KeyCode::Esc if ui.open_choice.is_some() => ui.open_choice = None,
            KeyCode::Esc if ui.card.is_some() => {
                ui.card = None;
                events = self.preview_card(None);
            }
            KeyCode::Esc if !ui.search.is_empty() => ui.search.clear(),
            KeyCode::Up | KeyCode::Down if ui.category == Category::Keys => {
                let down = key.code == KeyCode::Down;
                ui.key_row = Some(match (ui.key_row, down) {
                    (None, _) => 0,
                    (Some(i), true) => i + 1,
                    (Some(i), false) => i.saturating_sub(1),
                });
                let n = self
                    .key_rows(&self.settings_ui.search.to_lowercase())
                    .0
                    .len();
                let ui = &mut self.settings_ui;
                ui.key_row = ui.key_row.map(|i| i.min(n.saturating_sub(1)));
            }
            KeyCode::Enter if ui.category == Category::Keys && ui.key_row.is_some() => {
                return Some(self.capture_row());
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                if ui.category == Category::Appearance && ui.search.is_empty() =>
            {
                let n = self.previews.len();
                if n == 0 {
                    return Some(events);
                }
                let step: isize = match key.code {
                    KeyCode::Left => -1,
                    KeyCode::Right => 1,
                    KeyCode::Up => -4,
                    _ => 4,
                };
                let at = ui.card.map_or(-1, |i| i as isize) + step;
                let i = at.clamp(0, n as isize - 1) as usize;
                ui.card = Some(i);
                let name = self.previews[i].0.clone();
                events = self.preview_card(Some(name));
            }
            KeyCode::Enter if ui.category == Category::Appearance && ui.card.is_some() => {
                let i = ui.card.unwrap_or(0);
                if let Some((name, _)) = self.previews.get(i) {
                    let name = name.clone();
                    events = self.settings_action(SettingsAction::Theme(name));
                }
            }
            KeyCode::Up => ui.scroll = (ui.scroll - 40.0).max(0.0),
            KeyCode::Down => ui.scroll = (ui.scroll + 40.0).min(ui.room),
            KeyCode::PageUp => ui.scroll = (ui.scroll - 400.0).max(0.0),
            KeyCode::PageDown => ui.scroll = (ui.scroll + 400.0).min(ui.room),
            KeyCode::Home => ui.scroll = 0.0,
            KeyCode::End => ui.scroll = ui.room,
            KeyCode::Tab | KeyCode::BackTab => {
                let i = ui.category.index();
                let n = Category::ALL.len();
                let next = if key.code == KeyCode::Tab {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                return Some(self.settings_action(SettingsAction::Category(Category::ALL[next])));
            }
            _ => return None,
        }
        Some(events)
    }

    /// The wheel over the page.
    pub(crate) fn settings_scroll(&mut self, by: f32) {
        let ui = &mut self.settings_ui;
        ui.scroll = (ui.scroll + by).clamp(0.0, ui.room.max(0.0));
    }

    /// Every colour theme's palette and icon theme's icons, for the
    /// Settings tab's cards.
    pub(crate) fn make_previews(&mut self) {
        let base = self
            .packs
            .iter()
            .find(|p| p.label == theme::DEFAULT_NAME)
            .cloned();
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
                .and_then(|f| {
                    self.host
                        .plugins()
                        .into_iter()
                        .find(|i| f.starts_with(&i.dir))
                })
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
                .map(|(prog, pid)| (prog.clone(), *pid, self.memory.of(*pid)))
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
                    let mut off =
                        settings::editor_list(&self.settings.load().0, "disabled_plugins");
                    off.retain(|n| *n != name);
                    if !on {
                        off.push(name.clone());
                    }
                    let list =
                        toml::Value::Array(off.into_iter().map(toml::Value::String).collect());
                    if let Err(err) = self
                        .settings
                        .set("editor.disabled_plugins", &list.to_string())
                    {
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

/// Whether a keys row is one a filter lists.
fn keys_filter(f: KeysFilter, r: &KeyRow) -> bool {
    match f {
        KeysFilter::All => true,
        KeysFilter::Changed => r.binding.changed,
        KeysFilter::Plugins => r.binding.plugin.is_some(),
        KeysFilter::Conflicts => r.conflict,
    }
}

/// A pack with only the frame's icons, which layers over an icon theme
/// rather than being one.
fn is_ui_only(p: &Pack) -> bool {
    let icons = p.table.get("icons").and_then(|v| v.as_table());
    icons.is_some_and(|t| t.keys().all(|k| k == "ui"))
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

/// Whether the desktop asks for light colours, as the desktop portal says
/// (`color-scheme` 2), or GNOME's settings do; `None` if neither says.
fn system_light() -> Option<bool> {
    use std::process::Command;
    let portal = Command::new("busctl")
        .args([
            "--user",
            "call",
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            "ReadOne",
            "ss",
            "org.freedesktop.appearance",
            "color-scheme",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success());
    // `v u 1`: dark; `v u 2`: light; `v u 0`: no preference.
    if let Some(out) = portal {
        match String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .last()
        {
            Some("1") => return Some(false),
            Some("2") => return Some(true),
            _ => {}
        }
    }
    let gnome = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let scheme = String::from_utf8_lossy(&gnome.stdout);
    Some(scheme.contains("light"))
}

/// A process's resident memory in bytes, where the system says.
fn process_memory(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(changed: bool, plugin: Option<&str>) -> KeyRow {
        KeyRow {
            binding: KeyBinding {
                command: "edit.rename".into(),
                title: "Edit: Rename Symbol".into(),
                keys: vec![("f2".into(), "F2".into())],
                changed,
                defaults: vec!["f2".into()],
                plugin: plugin.map(str::to_string),
            },
            conflict: false,
        }
    }

    #[test]
    fn values_are_found_by_their_key_and_read_as_people_say_them() {
        let table: toml::Table =
            "[editor]\nminimap = true\ntab_size = 2\n[files]\nauto_save = \"after_delay\"\n"
                .parse()
                .unwrap();
        assert_eq!(get(&table, "minimap").map(shown).as_deref(), Some("on"));
        assert_eq!(
            get(&table, "editor.tab_size").map(shown).as_deref(),
            Some("2")
        );
        assert_eq!(
            get(&table, "files.auto_save").map(shown).as_deref(),
            Some("after_delay")
        );
        assert!(get(&table, "files.hot_exit").is_none());
    }

    #[test]
    fn keys_filters_list_what_they_say() {
        let plain = binding(false, None);
        let changed = binding(true, None);
        let plugin = binding(false, Some("Language"));
        assert!(keys_filter(KeysFilter::All, &plain));
        assert!(!keys_filter(KeysFilter::Changed, &plain));
        assert!(keys_filter(KeysFilter::Changed, &changed));
        assert!(keys_filter(KeysFilter::Plugins, &plugin));
        assert!(!keys_filter(KeysFilter::Conflicts, &plugin));
    }

    #[test]
    fn every_page_has_a_title_in_order() {
        let titles: Vec<_> = Category::ALL.iter().map(|c| c.title()).collect();
        assert_eq!(
            titles,
            [
                "Appearance",
                "Text Editor",
                "Files",
                "Keyboard Shortcuts",
                "Plugins"
            ]
        );
        assert_eq!(Category::Keys.index(), 3);
    }
}
