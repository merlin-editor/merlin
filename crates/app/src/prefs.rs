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
use merlin_plugins::{PluginInfo, SettingKind};

use crate::packs::{self, Icons, Look, Pack};
use crate::theme::{self, Palette};
use crate::{App, settings};

mod keys;
mod plugins;
mod themes;
mod view;

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
    pub open_choice: Option<String>,
    /// The plugin page's tab.
    pub page_tab: PageTab,
    /// A setting's text being typed.
    pub editing: Option<Editing>,
    /// The theme card the arrows are on, which the window previews.
    pub card: Option<usize>,
    pub scroll: f32,
    /// How far the page scrolls, as last drawn.
    pub room: f32,
}

/// A plugin page's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageTab {
    #[default]
    Settings,
    Allowed,
    Adds,
}

/// A setting's text being typed: into its field, or as a new item of a
/// list.
#[derive(Clone, Debug, PartialEq)]
pub struct Editing {
    pub key: String,
    pub text: String,
    /// A new item for a list, rather than the field's value.
    pub add: bool,
}

/// What a setting's control is.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Toggle(bool),
    /// Its values with their labels, and the value now.
    Choice(Vec<(String, String)>, String),
    Number(f32),
    /// Text typed into a field.
    Field(String, FieldKind),
    /// Strings as chips, with a way to add one.
    List(Vec<String>),
}

/// What a field takes.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Text,
    Path,
    /// Kept in the keyring, shown as dots. Its value is "set" or empty.
    Secret,
    Integer(Option<i64>, Option<i64>),
    Float(Option<f64>, Option<f64>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: String,
    pub about: String,
    pub key: String,
    pub control: Control,
    /// Set in the file being changed, so not the default there.
    pub changed: bool,
    /// Set in the workspace's file.
    pub in_workspace: bool,
    /// What the other file says, when the workspace's sets it.
    pub note: Option<String>,
    /// On the Workspace tab, a row the workspace sets: your own value, to
    /// go back to.
    pub yours: Option<String>,
    /// Only the user sets it, never a workspace.
    pub user_only: bool,
    /// Shown, but not set here: a user-only setting on the Workspace tab.
    pub locked: bool,
}

impl Row {
    /// The key as the row shows it: a plugin's own, without
    /// `plugins.<name>.` before it.
    pub fn key_shown(&self) -> &str {
        self.key
            .strip_prefix("plugins.")
            .and_then(|rest| rest.split_once('.'))
            .map_or(&self.key, |(_, key)| key)
    }
}

/// A heading's rows, on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub category: Category,
    pub title: String,
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
    /// Its tile's letters and colour.
    pub mark: (String, merlin_core::style::Hue),
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
    pub open_choice: Option<String>,
    pub page_tab: PageTab,
    pub editing: Option<Editing>,
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
    Toggle(String),
    OpenChoice(String),
    Choose(String, String),
    Step(String, f32),
    Reset(String),
    /// Starts typing into a field, or a new item of a list when `true`.
    Edit(String, bool),
    /// Takes an item out of a list.
    Unlist(String, usize),
    PageTab(PageTab),
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
    /// Its tile's letters and colour.
    pub mark: (String, merlin_core::style::Hue),
    pub dir: PathBuf,
    pub api: String,
    pub data_only: bool,
    pub enabled: bool,
    pub waiting: bool,
    pub failed: bool,
    pub read: bool,
    pub write: bool,
    pub workspace_read: bool,
    pub terminal: bool,
    pub processes: Vec<String>,
    pub running: Vec<(String, u32, Option<u64>)>,
    pub memory: usize,
    pub busy: Duration,
    pub commands: Vec<(String, Option<String>)>,
    pub languages: Vec<String>,
    pub looks: Vec<String>,
    /// Its settings, as rows.
    pub rows: Vec<Row>,
    /// The Uninstall button was pressed once, and waits for a second.
    pub uninstall_armed: bool,
    /// Installed with `merlin plugin install`, so it can be uninstalled.
    pub installed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PageAction {
    Enable(String, bool),
    Reload(String),
    /// Its channel of the Output panel.
    Output(String),
    Uninstall(String),
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

/// `editor.minimap` is `minimap` in `[editor]`.
fn part(key: &str) -> (&str, &str) {
    key.split_once('.').unwrap_or(("editor", key))
}

/// A setting by its key. A plugin's, `plugins.<name>.<key>`, may reach
/// into tables with more dots.
fn get<'a>(table: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let (section, name) = part(key);
    let table = table.get(section)?.as_table()?;
    if section != "plugins" {
        return table.get(name);
    }
    let mut parts = name.split('.');
    let mut value = table.get(parts.next()?)?;
    for part in parts {
        value = value.as_table()?.get(part)?;
    }
    Some(value)
}

/// A choice's value as people read it: `after_delay` is "After delay".
pub(crate) fn option_label(o: &str) -> String {
    let s = o.replace(['_', '-'], " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => s,
    }
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

    pub(crate) fn settings_action(&mut self, action: SettingsAction) -> Vec<Event> {
        let quote = |s: &str| toml::Value::String(s.into()).to_string();
        let ui = &mut self.settings_ui;
        if !matches!(action, SettingsAction::Choose(..)) {
            ui.open_choice = None;
        }
        if !matches!(action, SettingsAction::Edit(..)) {
            ui.editing = None;
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
                ui.scroll = 0.0;
                self.uninstall_armed = None;
                Vec::new()
            }
            SettingsAction::PageTab(tab) => {
                ui.page_tab = tab;
                Vec::new()
            }
            SettingsAction::Edit(key, add) => {
                let text = if add {
                    String::new()
                } else {
                    match self.find_row(&key).map(|r| r.control) {
                        // A secret is typed afresh, never shown.
                        Some(Control::Field(_, FieldKind::Secret)) => String::new(),
                        Some(Control::Field(text, _)) => text,
                        _ => String::new(),
                    }
                };
                self.settings_ui.editing = Some(Editing { key, text, add });
                Vec::new()
            }
            SettingsAction::Unlist(key, i) => {
                let Some(Control::List(mut items)) = self.find_row(&key).map(|r| r.control) else {
                    return Vec::new();
                };
                if i < items.len() {
                    items.remove(i);
                }
                let list = toml::Value::Array(items.into_iter().map(toml::Value::String).collect());
                self.write(&key, Some(list.to_string()))
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
                    .find_row(&key)
                    .is_some_and(|r| r.control == Control::Toggle(true));
                self.write(&key, Some((!on).to_string()))
            }
            SettingsAction::Choose(key, value) => {
                self.settings_ui.open_choice = None;
                // Numbers are written as numbers, but a plugin's choices
                // are always text.
                let value = match value.parse::<f64>() {
                    Ok(_) if !key.starts_with("plugins.") => value,
                    _ => quote(&value),
                };
                self.write(&key, Some(value))
            }
            SettingsAction::Step(key, by) => {
                let now = match self.find_row(&key).map(|r| r.control) {
                    Some(Control::Number(n)) => n,
                    _ => FONT_SIZE,
                };
                let next = (now + by).clamp(8.0, 32.0);
                self.write(&key, Some(format!("{next:.1}")))
            }
            SettingsAction::Reset(key) => match self.find_row(&key).map(|r| r.control) {
                Some(Control::Field(_, FieldKind::Secret)) => self.store_secret(&key, None),
                _ => self.write(&key, None),
            },
            SettingsAction::Unbind(command) => self.unbind(&command),
            SettingsAction::ResetKey(command) => self.reset_key(&command),
            SettingsAction::Page(action) => self.page_action(action),
            SettingsAction::OpenFile => match self.settings.ensure(self.settings_ui.workspace) {
                Ok(path) => self.open(&path),
                Err(err) => vec![Event::Error(format!("Can't open settings: {err}"))],
            },
        }
    }

    /// A key pressed while the Settings tab has the keyboard, or `None`
    /// for the editor to have it, as Ctrl+P and Ctrl+W are.
    pub(crate) fn settings_key(&mut self, key: Key) -> Option<Vec<Event>> {
        let plain = !key.ctrl && !key.alt;
        let ui = &mut self.settings_ui;
        if let Some(edit) = &mut ui.editing {
            match key.code {
                KeyCode::Esc => ui.editing = None,
                KeyCode::Enter => return Some(self.edit_done()),
                KeyCode::Backspace if key.ctrl => {
                    let keep = edit.text.trim_end().rfind([' ', '/', '-', '_', '.']);
                    edit.text.truncate(keep.map_or(0, |i| i + 1));
                }
                KeyCode::Backspace => {
                    edit.text.pop();
                }
                KeyCode::Char(c) if !key.ctrl && !key.alt => edit.text.push(c),
                // Ctrl+P and the like still reach the editor.
                _ if key.ctrl => return None,
                _ => {}
            }
            return Some(Vec::new());
        }
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

    /// Every row that can show: the pages' and the plugins'.
    fn find_row(&self, key: &str) -> Option<Row> {
        if let Some(rest) = key.strip_prefix("plugins.") {
            let name = rest.split('.').next()?;
            return self.plugin_rows(name).into_iter().find(|r| r.key == key);
        }
        self.sections()
            .into_iter()
            .flat_map(|s| s.rows)
            .find(|r| r.key == key)
    }

    /// Enter in a field: its text saved, as the setting takes it.
    fn edit_done(&mut self) -> Vec<Event> {
        let Some(edit) = self.settings_ui.editing.take() else {
            return Vec::new();
        };
        let Some(control) = self.find_row(&edit.key).map(|r| r.control) else {
            return Vec::new();
        };
        match typed(&control, &edit.text) {
            Ok(Typed::Set(toml::Value::String(text)))
                if matches!(control, Control::Field(_, FieldKind::Secret)) =>
            {
                self.store_secret(&edit.key, Some(&text))
            }
            Ok(Typed::Set(value)) => self.write(&edit.key, Some(value.to_string())),
            Ok(Typed::Reset) => self.write(&edit.key, None),
            Ok(Typed::Nothing) => Vec::new(),
            Err(err) => vec![Event::Error(err)],
        }
    }

    /// A plugin's secret setting, `plugins.<name>.<key>`, stored in the
    /// keyring or forgotten. Settings files never hold it.
    fn store_secret(&mut self, key: &str, value: Option<&str>) -> Vec<Event> {
        let Some((plugin, key)) = key
            .strip_prefix("plugins.")
            .and_then(|rest| rest.split_once('.'))
        else {
            return Vec::new();
        };
        if self.scripted {
            return Vec::new();
        }
        let snapshot = self.snapshot();
        self.host
            .set_secret(plugin, key, value, &snapshot)
            .unwrap_or_else(|err| vec![Event::Error(format!("Can't store {key}: {err}"))])
    }

    /// The wheel over the page.
    pub(crate) fn settings_scroll(&mut self, by: f32) {
        let ui = &mut self.settings_ui;
        ui.scroll = (ui.scroll + by).clamp(0.0, ui.room.max(0.0));
    }
}

/// What text typed into a setting does.
#[derive(Debug, PartialEq)]
enum Typed {
    Set(toml::Value),
    /// Nothing typed: the setting goes back to the default.
    Reset,
    Nothing,
}

/// Text typed into a field, or as a list's new item, as the setting's
/// value: a number clamped to its range, a list with the item added.
fn typed(control: &Control, text: &str) -> Result<Typed, String> {
    let text = text.trim();
    match control {
        Control::List(items) => {
            if text.is_empty() || items.iter().any(|i| i == text) {
                return Ok(Typed::Nothing);
            }
            let mut items: Vec<toml::Value> = items
                .iter()
                .map(|i| toml::Value::String(i.clone()))
                .collect();
            items.push(text.into());
            Ok(Typed::Set(toml::Value::Array(items)))
        }
        // Nothing typed keeps a secret: ↺ forgets it.
        Control::Field(_, FieldKind::Secret) if text.is_empty() => Ok(Typed::Nothing),
        Control::Field(_, FieldKind::Secret) => Ok(Typed::Set(text.into())),
        Control::Field(..) if text.is_empty() => Ok(Typed::Reset),
        Control::Field(_, FieldKind::Text | FieldKind::Path) => Ok(Typed::Set(text.into())),
        Control::Field(_, FieldKind::Integer(min, max)) => text
            .parse::<i64>()
            .map(|n| {
                Typed::Set(
                    n.clamp(min.unwrap_or(i64::MIN), max.unwrap_or(i64::MAX))
                        .into(),
                )
            })
            .map_err(|_| format!("{text} isn't a whole number")),
        Control::Field(_, FieldKind::Float(min, max)) => text
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(|n| {
                Typed::Set(
                    n.clamp(min.unwrap_or(f64::MIN), max.unwrap_or(f64::MAX))
                        .into(),
                )
            })
            .ok_or_else(|| format!("{text} isn't a number")),
        _ => Ok(Typed::Nothing),
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
    if let Some(desc) = p.description.as_deref().filter(|d| !d.is_empty()) {
        return desc.to_string();
    }
    let mut parts = Vec::new();
    if !p.languages.is_empty() {
        if p.permissions.processes.is_empty() {
            parts.push("Syntax highlighting & folding".into());
        } else {
            parts.push(p.languages.join(", "));
        }
    }
    if !p.looks.is_empty() {
        let n = p.looks.len();
        parts.push(format!("{n} look{}", if n == 1 { "" } else { "s" }));
    }
    if !p.permissions.processes.is_empty() {
        let servers: Vec<&str> = p
            .permissions
            .processes
            .iter()
            .map(|s| s.rsplit('/').next().unwrap_or(s.as_str()))
            .collect();
        parts.push(servers.join(", "));
    }
    if parts.is_empty() {
        "Commands and panels".into()
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

#[cfg(test)]
mod tests {
    use super::keys::*;
    use super::view::shown;
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

    /// A plugin with a choice, a list, a program's path and a number.
    fn plugin() -> PluginInfo {
        use merlin_plugins::SettingSpec;
        let spec = |key: &str, kind: SettingKind, default: toml::Value, user_only| SettingSpec {
            key: key.into(),
            title: key.into(),
            kind,
            default,
            description: String::new(),
            user_only,
        };
        PluginInfo {
            name: "rust".into(),
            title: "Rust".into(),
            schema: vec![
                spec(
                    "check-command",
                    SettingKind::Choice(vec![
                        ("check".into(), "cargo check".into()),
                        ("clippy".into(), "cargo clippy".into()),
                    ]),
                    "check".into(),
                    false,
                ),
                spec(
                    "features",
                    SettingKind::List,
                    toml::Value::Array(Vec::new()),
                    false,
                ),
                spec(
                    "server-path",
                    SettingKind::Path,
                    "rust-analyzer".into(),
                    true,
                ),
                spec(
                    "jobs",
                    SettingKind::Integer {
                        min: Some(1),
                        max: Some(64),
                    },
                    4.into(),
                    false,
                ),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn plugin_rows_show_each_file_and_lock_what_only_you_set() {
        let user: toml::Table =
            "[plugins.rust]\ncheck-command = \"check\"\nserver-path = \"/usr/bin/ra\"\n"
                .parse()
                .unwrap();
        let workspace: toml::Table =
            "[plugins.rust]\ncheck-command = \"clippy\"\nfeatures = [\"gui\"]\n"
                .parse()
                .unwrap();
        let row =
            |rows: &[Row], key: &str| rows.iter().find(|r| r.key_shown() == key).unwrap().clone();

        // On the User tab: yours, and the workspace's wins in it.
        let rows = plugins::rows_of(&plugin(), &user, &workspace, false);
        let check = row(&rows, "check-command");
        assert_eq!(check.key, "plugins.rust.check-command");
        assert!(check.changed && check.in_workspace && check.yours.is_none());
        assert_eq!(
            check.control,
            Control::Choice(
                vec![
                    ("check".into(), "cargo check".into()),
                    ("clippy".into(), "cargo clippy".into())
                ],
                "check".into()
            )
        );
        let path = row(&rows, "server-path");
        assert!(path.user_only && !path.locked && path.changed);

        // On the Workspace tab: the workspace's, with yours to go back to.
        let rows = plugins::rows_of(&plugin(), &user, &workspace, true);
        let check = row(&rows, "check-command");
        assert!(matches!(&check.control, Control::Choice(_, now) if now == "clippy"));
        assert_eq!(check.yours.as_deref(), Some("cargo check"));
        assert_eq!(
            row(&rows, "features").control,
            Control::List(vec!["gui".into()])
        );
        // A program's path is yours, shown but not set here.
        let path = row(&rows, "server-path");
        assert!(path.locked && !path.changed);
        assert_eq!(
            path.control,
            Control::Field("/usr/bin/ra".into(), FieldKind::Path)
        );
        assert_eq!(
            row(&rows, "jobs").control,
            Control::Field("4".into(), FieldKind::Integer(Some(1), Some(64)))
        );
    }

    #[test]
    fn typing_into_a_setting_gives_its_value() {
        let list = Control::List(vec!["gui".into()]);
        assert_eq!(
            typed(&list, " wgpu "),
            Ok(Typed::Set(toml::Value::Array(vec![
                "gui".into(),
                "wgpu".into()
            ])))
        );
        assert_eq!(typed(&list, "gui"), Ok(Typed::Nothing));
        let jobs = Control::Field("4".into(), FieldKind::Integer(Some(1), Some(64)));
        assert_eq!(typed(&jobs, "100"), Ok(Typed::Set(64.into())));
        assert!(typed(&jobs, "lots").is_err());
        assert_eq!(typed(&jobs, ""), Ok(Typed::Reset));
        // A secret is never reset by an empty field; ↺ forgets it.
        let secret = Control::Field("set".into(), FieldKind::Secret);
        assert_eq!(typed(&secret, ""), Ok(Typed::Nothing));
        assert_eq!(typed(&secret, "k3y"), Ok(Typed::Set("k3y".into())));
        let path = Control::Field(String::new(), FieldKind::Path);
        assert_eq!(
            typed(&path, "/usr/bin/ra"),
            Ok(Typed::Set("/usr/bin/ra".into()))
        );
    }

    #[test]
    fn plugin_settings_are_found_by_their_dotted_keys() {
        let table: toml::Table = "[plugins.testbed]\nstyle.loud = true\n".parse().unwrap();
        assert_eq!(
            get(&table, "plugins.testbed.style.loud"),
            Some(&toml::Value::Boolean(true))
        );
        assert!(get(&table, "plugins.testbed.style.quiet").is_none());
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
