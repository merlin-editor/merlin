mod carets;
mod chrome;
mod colours;
mod crumbs;
pub(crate) mod fold;
mod groups;
pub(crate) mod guides;
mod layout;
mod menu;
mod minimap;
mod mouse;
mod panels;
mod render;
mod settings;
mod views;
mod window;
mod workspace;
mod wrap;

pub use window::{
    PaletteView, RailItem, SideView, StatusView, TabView, ToastView, UiEvent, WindowView,
};

pub use settings::Settings;
pub use workspace::{GroupState, Tab, Workspace};

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::buffer::Buffer;
use crate::diff::LineKind;
use crate::doc::{Doc, TAB_WIDTH};
use crate::edit::Edit;
use crate::fuzzy;
use crate::history::{Kind, Revision};
use crate::key::{Key, KeyCode};
use crate::lang::{Completion, Diagnostic, Language, Mark, PluginSymbol, PluginToken};
use crate::layers::{DocSymbol, Layer, Token};
use crate::panel::{Panel, PickItem};

pub use views::SearchHit;

/// A plugin command. Its id is local to the plugin; the editor runs it as
/// `<plugin>.<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub title: String,
    /// A key that runs it, such as `ctrl+alt+w`.
    pub key: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Drag,
    Up,
    ScrollUp,
    ScrollDown,
    /// The wheel sideways, or Shift with the wheel.
    ScrollLeft,
    ScrollRight,
    /// The mouse moved with no button down.
    Move,
}

/// A left-button or wheel event, in screen cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mouse {
    pub kind: MouseKind,
    pub x: u16,
    pub y: u16,
    pub shift: bool,
    /// Alt+click adds a cursor.
    pub alt: bool,
    /// 2 for a double click and 3 for a triple click, on presses; else 1.
    pub clicks: u8,
}

#[derive(Clone, Debug)]
pub enum Event {
    Key(Key),
    Paste(String),
    Mouse(Mouse),
    /// A click on the frame of a window that draws its own; see
    /// [`Editor::set_windowed`].
    Ui(UiEvent),
    Resize(u16, u16),
    Command(String),
    Saved {
        doc: u64,
        revision: Revision,
        path: PathBuf,
    },
    SaveFailed(String),
    /// A file asked for with [`Effect::Open`].
    Opened {
        path: PathBuf,
        text: String,
    },
    /// Lays out the editor groups as last time, once their files are open.
    Restore(Workspace),
    /// The user's settings, with the workspace's over them.
    Settings(Settings),
    /// Colours for text the editor asked for with
    /// [`Editor::colour_requests`].
    TextColours {
        language: String,
        text: String,
        tokens: Vec<PluginToken>,
    },
    /// The project's files, asked for with [`Effect::ListFiles`].
    Files(Vec<String>),
    /// The system clipboard, asked for with [`Effect::ReadClipboard`].
    /// `None` if it can't be read; the editor then pastes what it last
    /// copied.
    Clipboard(Option<String>),
    Error(String),
    RegisterPlugin {
        plugin: String,
        commands: Vec<Command>,
    },
    PluginFailed {
        plugin: String,
        message: String,
    },
    SetPanel {
        plugin: String,
        panel: Option<Panel>,
    },
    FocusPanel(String),
    ShowPicker {
        plugin: String,
        id: String,
        title: String,
        items: Vec<PickItem>,
    },
    InsertText(String),
    Notify(String),
    /// A plugin's diagnostics for a file, replacing its earlier ones.
    SetDiagnostics {
        plugin: String,
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
    /// Text to show next to the cursor until the next key.
    ShowHover(String),
    /// Languages a plugin adds.
    RegisterLanguages {
        plugin: String,
        languages: Vec<Language>,
    },
    /// Colours for a file from a plugin, for lines `lines` (end excluded),
    /// or for all of it.
    SetTokens {
        path: PathBuf,
        layer: Layer,
        lines: Option<(u32, u32)>,
        tokens: Vec<PluginToken>,
    },
    /// The symbols in a file, from a plugin.
    SetSymbols {
        path: PathBuf,
        symbols: Vec<PluginSymbol>,
    },
    /// Where a file's blocks fold, as first and last lines, from a plugin.
    SetFolds {
        path: PathBuf,
        ranges: Vec<(u32, u32)>,
    },
    /// Completions for the word at the cursor.
    ShowCompletions(Vec<Completion>),
    /// Opens a file at a line and UTF-16 column, as go-to-definition does.
    OpenAt {
        path: PathBuf,
        line: u32,
        column: u32,
    },
    /// A plugin proposes changing a file from `old` to `new`, and asks the
    /// user to review it. The answer goes back as [`Effect::PluginReview`].
    ShowReview {
        plugin: String,
        id: String,
        path: PathBuf,
        old: String,
        new: String,
    },
    /// The plugin got its answer some other way; the review closes.
    CloseReview {
        plugin: String,
        id: String,
    },
    /// An open file changed on disk. It reloads unless it has unsaved
    /// changes.
    FileChanged {
        path: PathBuf,
        text: String,
    },
    /// A file's text as of the last commit, or `None` if git doesn't track
    /// it. Lines that differ get signs in the gutter.
    GitBase {
        path: PathBuf,
        text: Option<String>,
    },
    /// The project's files for the Explorer, asked for with
    /// [`Effect::ListTree`].
    Tree(Vec<String>),
    /// What a project search found, asked for with [`Effect::Search`].
    SearchResults {
        query: String,
        hits: Vec<SearchHit>,
    },
    /// The branch and changed files, asked for with [`Effect::GitStatus`].
    /// No branch means the project isn't in a git repository.
    GitStatus {
        branch: Option<String>,
        files: Vec<(String, char)>,
        /// Files with changes in the index, and with changes not yet staged,
        /// with their letters.
        staged: Vec<(String, char)>,
        changed: Vec<(String, char)>,
    },
    /// A file's changes since the last commit, to look at, asked for with
    /// [`Effect::OpenDiff`].
    ShowDiff {
        path: PathBuf,
        old: String,
        new: String,
    },
}

/// Work the editor asks its host to do. Results come back as events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The user changed a setting from a menu, such as View › Minimap: save
    /// it in their settings, as a TOML value.
    SetSetting {
        key: String,
        value: String,
    },
    /// Open the user's settings file, or the workspace's.
    OpenSettings {
        workspace: bool,
    },
    Save {
        doc: u64,
        path: PathBuf,
        text: String,
        revision: Revision,
    },
    Open(PathBuf),
    ListFiles,
    Copy(String),
    ReadClipboard,
    Quit,
    PluginCommand {
        plugin: String,
        command: String,
    },
    PluginClick {
        plugin: String,
        button: String,
    },
    PluginSubmit {
        plugin: String,
        input: String,
        text: String,
    },
    PluginPick {
        plugin: String,
        picker: String,
        item: String,
    },
    /// The user accepted or rejected a change shown with
    /// [`Event::ShowReview`].
    PluginReview {
        plugin: String,
        id: String,
        accepted: bool,
    },
    ListTree,
    /// Search the project's files for this text.
    Search(String),
    GitStatus,
    OpenDiff(PathBuf),
    /// Stage, unstage or commit; the frontend answers with
    /// [`Event::GitStatus`].
    Git(GitAction),
}

/// What Source Control asks git to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitAction {
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Commit(String),
}

/// Exactly one part of the UI has focus, and only it gets keys and pastes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Find,
    Palette,
    /// Plugin panels, on the right.
    Panel,
    /// The built-in views, on the left.
    SideBar,
    /// The panel under the editor: Problems and Output.
    Bottom,
    Menu,
}

/// Where panels go: built-in views in the side bar on the left, plugin
/// panels on the right, and Problems and Output in the panel at the bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Bottom,
}

/// Commands, titled as VS Code titles them. Menus show the part after the
/// colon.
const BUILTIN: &[(&str, &str)] = &[
    ("file.open", "Go: Go to File…"),
    ("file.save", "File: Save"),
    ("file.close", "File: Close Editor"),
    ("file.next", "View: Next Editor"),
    ("file.keep", "View: Keep Editor Open"),
    ("view.breadcrumbs", "View: Toggle Breadcrumbs"),
    ("view.sticky_scroll", "View: Toggle Sticky Scroll"),
    ("edit.fold", "Fold"),
    ("edit.unfold", "Unfold"),
    ("edit.fold_all", "Fold All"),
    ("edit.unfold_all", "Unfold All"),
    ("file.prev", "View: Previous Editor"),
    ("edit.undo", "Edit: Undo"),
    ("edit.redo", "Edit: Redo"),
    ("edit.select_all", "Selection: Select All"),
    ("edit.copy", "Edit: Copy"),
    ("edit.cut", "Edit: Cut"),
    ("edit.paste", "Edit: Paste"),
    ("edit.comment", "Edit: Toggle Line Comment"),
    ("edit.indent", "Selection: Indent Lines"),
    ("edit.outdent", "Selection: Outdent Lines"),
    ("find.open", "Edit: Find"),
    ("find.replace", "Edit: Replace"),
    ("palette.open", "View: Command Palette…"),
    ("view.sidebar", "View: Toggle Side Bar"),
    ("view.explorer", "View: Explorer"),
    ("view.search", "View: Search"),
    ("view.outline", "View: Outline"),
    ("settings.user", "Preferences: Open User Settings"),
    ("settings.workspace", "Preferences: Open Workspace Settings"),
    ("go.symbol", "Go to Symbol in Editor"),
    ("view.changes", "View: Source Control"),
    ("panel.toggle", "View: Focus Secondary Side Bar"),
    ("view.secondary", "View: Toggle Secondary Side Bar"),
    ("view.panel", "View: Toggle Panel"),
    ("view.minimap", "View: Toggle Minimap"),
    ("view.wrap", "View: Toggle Word Wrap"),
    ("view.split", "View: Split Editor Right"),
    ("view.split_down", "View: Split Editor Down"),
    ("edit.add_next", "Selection: Add Next Occurrence"),
    (
        "edit.select_all_matches",
        "Selection: Select All Occurrences",
    ),
    ("edit.cursor_above", "Selection: Add Cursor Above"),
    ("edit.cursor_below", "Selection: Add Cursor Below"),
    ("view.group1", "View: Focus First Editor Group"),
    ("view.group2", "View: Focus Second Editor Group"),
    ("view.group3", "View: Focus Third Editor Group"),
    ("view.problems", "View: Problems"),
    ("view.output", "View: Output"),
    ("go.line", "Go: Go to Line…"),
    ("review.next", "Go: Next Change to Review"),
    ("nav.back", "Go: Back"),
    ("app.about", "Help: About"),
    ("app.quit", "File: Exit"),
];

/// Keys that work wherever focus is, as VS Code binds them where the
/// terminal can tell them apart. Ctrl+Shift needs a terminal with the kitty
/// keyboard protocol; elsewhere the menus and F1 reach everything.
const GLOBAL_KEYS: &[(&str, &str)] = &[
    ("ctrl+s", "file.save"),
    ("ctrl+p", "file.open"),
    ("ctrl+o", "file.open"),
    ("ctrl+w", "file.close"),
    ("ctrl+q", "app.quit"),
    ("ctrl+shift+p", "palette.open"),
    ("f1", "palette.open"),
    ("ctrl+f", "find.open"),
    ("ctrl+r", "find.replace"),
    ("ctrl+b", "view.sidebar"),
    ("ctrl+alt+b", "panel.toggle"),
    ("ctrl+shift+e", "view.explorer"),
    ("ctrl+e", "view.explorer"),
    ("ctrl+shift+f", "view.search"),
    ("ctrl+shift+o", "go.symbol"),
    ("ctrl+shift+g", "view.changes"),
    ("ctrl+j", "view.panel"),
    ("ctrl+shift+m", "view.problems"),
    ("ctrl+shift+u", "view.output"),
    ("ctrl+g", "go.line"),
    ("ctrl+v", "edit.paste"),
    ("ctrl+pagedown", "file.next"),
    ("ctrl+pageup", "file.prev"),
    ("alt+left", "nav.back"),
    ("alt+z", "view.wrap"),
    ("ctrl+\\", "view.split"),
    ("ctrl+shift+[", "edit.fold"),
    ("ctrl+shift+]", "edit.unfold"),
    // kitty keeps Ctrl+Shift+[ and ] for its own windows.
    ("ctrl+alt+[", "edit.fold"),
    ("ctrl+alt+]", "edit.unfold"),
    ("ctrl+1", "view.group1"),
    ("ctrl+2", "view.group2"),
    ("ctrl+3", "view.group3"),
    ("shift+alt+up", "edit.cursor_above"),
    ("shift+alt+down", "edit.cursor_below"),
    ("ctrl+alt+up", "edit.cursor_above"),
    ("ctrl+alt+down", "edit.cursor_below"),
    ("f10", "menu.open"),
    ("alt+f", "menu.file"),
    ("alt+e", "menu.edit"),
    ("alt+s", "menu.selection"),
    ("alt+v", "menu.view"),
    ("alt+g", "menu.go"),
    ("alt+h", "menu.help"),
];

/// Keys that work in the text.
const EDITOR_KEYS: &[(&str, &str)] = &[
    ("ctrl+z", "edit.undo"),
    ("ctrl+y", "edit.redo"),
    ("ctrl+shift+z", "edit.redo"),
    ("ctrl+a", "edit.select_all"),
    ("ctrl+c", "edit.copy"),
    ("ctrl+x", "edit.cut"),
    ("ctrl+/", "edit.comment"),
    ("ctrl+d", "edit.add_next"),
    ("ctrl+shift+l", "edit.select_all_matches"),
];

fn set_flag(key: &str, on: bool) -> Effect {
    Effect::SetSetting {
        key: key.into(),
        value: on.to_string(),
    }
}

fn bound(table: &[(&str, &'static str)], key: Key) -> Option<&'static str> {
    table
        .iter()
        .find(|(spec, _)| Key::parse(spec) == Some(key))
        .map(|(_, id)| *id)
}

const PALETTE_ROWS: usize = 10;
/// Most matches the palette keeps, so a big project stays fast.
const PALETTE_LIMIT: usize = 500;
const WHEEL_LINES: usize = 3;

struct Plugin {
    name: String,
    commands: Vec<Command>,
    /// Keys bound to its commands, by local command id.
    keys: Vec<(Key, String)>,
    panel: Option<Panel>,
    error: Option<String>,
    side: Side,
}

struct Find {
    query: String,
    /// The replacement, when replacing.
    replace: Option<String>,
    /// Whether typing goes to the replacement.
    in_replace: bool,
    /// Where the cursor was when find opened; typing searches from here.
    origin: usize,
}

impl Find {
    /// The field typing goes to.
    fn field(&mut self) -> &mut String {
        if self.in_replace {
            self.replace.get_or_insert_default()
        } else {
            &mut self.query
        }
    }
}

enum Source {
    Commands,
    Files,
    Line,
    Symbols,
    Picker { plugin: String, id: String },
}

enum Popup {
    Hover(Vec<String>),
    Completion {
        items: Vec<Completion>,
        selected: usize,
        /// Where the word being completed starts.
        start: usize,
    },
}

const POPUP_WIDTH: usize = 72;
const HOVER_ROWS: usize = 14;
const COMPLETION_ROWS: usize = 8;

/// The palette lists commands, files, or a plugin's items.
struct Palette {
    query: String,
    selected: usize,
    title: Option<String>,
    source: Source,
    /// Ids and labels, for files and pickers. Commands are listed live.
    items: Vec<(String, String)>,
}

impl Palette {
    fn new(source: Source, title: Option<String>, items: Vec<(String, String)>) -> Self {
        Self {
            query: String::new(),
            selected: 0,
            title,
            source,
            items,
        }
    }
}

struct Message {
    text: String,
    error: bool,
    /// Buttons: label and the command each runs.
    actions: Vec<(String, String)>,
}

pub struct Editor {
    docs: Vec<Doc>,
    current: usize,
    next_id: u64,
    width: u16,
    height: u16,
    focus: Focus,
    find: Option<Find>,
    palette: Option<Palette>,
    /// The side bars, left and right, and the panel at the bottom.
    bars: [panels::Bar; 3],
    menu: Option<menu::Menu>,
    tree: Option<views::Tree>,
    search: views::Search,
    git: views::Git,
    /// Text typed into panel inputs, by plugin and input id.
    inputs: HashMap<(String, String), String>,
    plugins: Vec<Plugin>,
    message: Option<Message>,
    /// A command waiting to be pressed again to confirm, such as quitting
    /// with unsaved changes.
    armed: Option<&'static str>,
    /// What was last copied, for when the system clipboard can't be read.
    clipboard: String,
    /// Whether the left button went down in the text and is selecting.
    dragging: bool,
    /// Whether the left button went down on the scrollbar.
    scrollbar_drag: bool,
    /// Whether the left button went down on the minimap.
    minimap_drag: bool,
    /// A tab being dragged, by document id, to another place or group.
    tab_drag: Option<u64>,
    popup: Option<Popup>,
    /// Diagnostics by plugin and file, as plugins sent them.
    diagnostics: HashMap<(String, PathBuf), Vec<Diagnostic>>,
    /// Bumped when diagnostics change, so the Explorer rebuilds.
    diagnostics_version: u64,
    /// Where to go once a file being opened arrives.
    pending_goto: Option<(PathBuf, u32, u32)>,
    /// Where go-to jumped from, by document id and position.
    jumps: Vec<(u64, usize)>,
    /// Bracket depth at a line, by document, version and line.
    bracket_cache: std::cell::Cell<Option<(guides::DepthKey, usize)>>,
    /// The lines folded away, for a document, version and set of folds.
    #[allow(clippy::type_complexity)]
    fold_cache: std::cell::RefCell<Option<((u64, u64, Vec<usize>), Vec<std::ops::Range<usize>>)>>,
    /// Where the mouse is, for lighting up what's under it.
    hover: Option<(u16, u16)>,
    /// Whether to draw Nerd Font icons.
    nerd: bool,
    /// Whether to draw the minimap beside the scrollbar.
    minimap: bool,
    /// Long lines wrap instead of scrolling sideways.
    wrap: bool,
    /// Breadcrumbs under the tabs.
    breadcrumbs: bool,
    /// The blocks around the top of the view stay pinned there.
    sticky_scroll: bool,
    /// Editor groups side by side, and the focused one.
    groups: Vec<groups::Group>,
    group: usize,
    /// A group other than the focused one, while it's drawn.
    drawing: std::cell::Cell<Option<groups::Drawing>>,
    /// The group and document shown after the last event.
    shown: (usize, u64),
    /// The preview tab, which the next file opened from the Explorer
    /// replaces, as in VS Code; editing it keeps it.
    preview: Option<u64>,
    /// A file asked for as a preview, until it arrives.
    opening_preview: Option<PathBuf>,
    /// 1, 2 or 3 while handling a single, double or triple click; else 0.
    clicks: u8,
    /// Keys bound in the settings, and the commands they run.
    user_keys: Vec<(Key, String)>,
    /// Whether a click in the Explorer opens a preview tab.
    preview_tabs: bool,
    /// Colours plugins gave for diffs and code blocks.
    colours: colours::TextColours,
    /// Languages plugins added, with the plugin that added each.
    languages: Vec<(String, Language)>,
    /// The document and version the Outline was built for.
    outline_for: Option<(u64, u64)>,
    /// The project's name, for the title bar.
    project: String,
    /// Messages so far, newest last, for the Output panel.
    log: Vec<(bool, String)>,
    /// Problems as listed in the Problems panel: file, line and column.
    problems: Vec<(PathBuf, u32, u32)>,
    /// The window draws the frame; the screen holds only the text.
    windowed: bool,
}

impl Editor {
    pub fn new(text: &str, path: Option<PathBuf>) -> Self {
        let mut editor = Self {
            docs: vec![Doc::new(0, text, path)],
            current: 0,
            next_id: 1,
            width: 80,
            height: 24,
            focus: Focus::Editor,
            find: None,
            palette: None,
            bars: Default::default(),
            menu: None,
            tree: None,
            search: Default::default(),
            git: Default::default(),
            inputs: HashMap::new(),
            plugins: Vec::new(),
            message: None,
            armed: None,
            clipboard: String::new(),
            dragging: false,
            scrollbar_drag: false,
            minimap_drag: false,
            tab_drag: None,
            popup: None,
            diagnostics: HashMap::new(),
            diagnostics_version: 0,
            pending_goto: None,
            jumps: Vec::new(),
            bracket_cache: Default::default(),
            fold_cache: Default::default(),
            hover: None,
            nerd: false,
            minimap: false,
            wrap: false,
            breadcrumbs: false,
            sticky_scroll: false,
            groups: vec![groups::Group::new(0)],
            group: 0,
            drawing: Default::default(),
            shown: (0, 0),
            preview: None,
            opening_preview: None,
            clicks: 0,
            outline_for: None,
            languages: Vec::new(),
            colours: Default::default(),
            user_keys: Vec::new(),
            preview_tabs: true,
            project: "merlin".into(),
            log: Vec::new(),
            problems: Vec::new(),
            windowed: false,
        };
        editor.add_views();
        editor
    }

    /// Draws Nerd Font icons, for terminals with such a font.
    pub fn set_nerd_icons(&mut self, on: bool) {
        self.nerd = on;
        self.build_views();
    }

    /// Shows the braille minimap beside the scrollbar, in windows wide
    /// enough for it.
    pub fn set_minimap(&mut self, on: bool) {
        self.minimap = on;
    }

    pub fn minimap(&self) -> bool {
        self.minimap
    }

    pub fn set_wrap(&mut self, on: bool) {
        self.wrap = on;
    }

    /// Shows breadcrumbs under the tabs, as VS Code does. Off in a new
    /// editor, so tests and scripts keep their rows.
    pub fn set_breadcrumbs(&mut self, on: bool) {
        self.breadcrumbs = on;
    }

    pub fn breadcrumbs(&self) -> bool {
        self.breadcrumbs
    }

    pub fn set_sticky_scroll(&mut self, on: bool) {
        self.sticky_scroll = on;
    }

    pub fn sticky_scroll(&self) -> bool {
        self.sticky_scroll
    }

    pub fn wrap(&self) -> bool {
        self.wrap
    }

    /// Names the project, as the title bar shows it.
    pub fn set_project(&mut self, name: String) {
        self.project = name;
    }

    /// The document shown: the current one, or another group's while it's
    /// drawn.
    fn doc(&self) -> &Doc {
        &self.docs[self.pane_doc()]
    }

    fn doc_mut(&mut self) -> &mut Doc {
        &mut self.docs[self.current]
    }

    /// The buffer being edited.
    pub fn buffer(&self) -> &Buffer {
        &self.doc().buffer
    }

    /// Identifies the open file being edited, for as long as it's open.
    pub fn doc_id(&self) -> u64 {
        self.doc().id
    }

    pub fn cursor(&self) -> usize {
        self.doc().cursor
    }

    /// The selected text, if any.
    pub fn selection(&self) -> Option<String> {
        let doc = self.doc();
        doc.selection().map(|(s, e)| doc.slice(s, e))
    }

    /// The selection's start and end as lines and UTF-16 columns, if any.
    pub fn selection_utf16(&self) -> Option<((u32, u32), (u32, u32))> {
        let doc = self.doc();
        let (start, end) = doc.selection()?;
        let at = |pos: usize| {
            let (line, col) = doc.line_col(pos);
            let text: String = doc.line_chars(line).into_iter().collect();
            (
                line as u32,
                crate::lsp::char_to_utf16_col(&text, col) as u32,
            )
        };
        Some((at(start), at(end)))
    }

    /// Whether the active document is a review of a proposed change.
    pub fn in_review(&self) -> bool {
        self.doc().review.is_some()
    }

    /// Paths of the open files, in tab order.
    pub fn open_files(&self) -> Vec<Option<PathBuf>> {
        self.docs.iter().map(|d| d.buffer.path.clone()).collect()
    }

    /// The cursor's line and UTF-16 column, as LSP counts them.
    pub fn cursor_utf16(&self) -> (u32, u32) {
        let doc = self.doc();
        let (line, col) = doc.line_col(doc.cursor);
        let text: String = doc.line_chars(line).into_iter().collect();
        (
            line as u32,
            crate::lsp::char_to_utf16_col(&text, col) as u32,
        )
    }

    /// The text of the popup, if one is showing, for tests.
    pub fn popup_text(&self) -> Option<String> {
        match self.popup.as_ref()? {
            Popup::Hover(lines) => Some(lines.join("\n")),
            Popup::Completion { .. } => Some(
                self.completions()
                    .iter()
                    .map(|c| c.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        }
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Whether plugin panels show, on the right.
    pub fn panel_visible(&self) -> bool {
        self.bar(Side::Right).visible
    }

    /// Whether the panel under the editor shows.
    pub fn bottom_visible(&self) -> bool {
        self.bar(Side::Bottom).visible
    }

    /// Whether the side bar shows, on the left.
    pub fn side_bar_visible(&self) -> bool {
        self.bar(Side::Left).visible
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_ref().map(|m| m.text.as_str())
    }

    pub fn update(&mut self, event: Event) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.clicks = 0;
        // Only something the user does cancels a pending confirmation.
        let by_user = matches!(
            event,
            Event::Key(_) | Event::Paste(_) | Event::Command(_) | Event::Ui(_)
        )
            || matches!(event, Event::Mouse(m) if m.kind == MouseKind::Down);
        let armed = if by_user { self.armed.take() } else { None };
        if let Event::Mouse(m) = &event {
            self.hover = Some((m.x, m.y));
        }
        if by_user {
            // A click on a notice's button runs it before the notice goes.
            if let Event::Mouse(m) = &event
                && m.kind == MouseKind::Down
                && let Some(command) = self.toast_click(m.x, m.y)
            {
                self.message = None;
                self.run(&command, armed, &mut fx);
                return fx;
            }
            // A notice's button in a window needs the notice to run.
            if !matches!(event, Event::Ui(UiEvent::ToastButton(_))) {
                self.message = None;
            }
        }
        let before = self.view_key();
        match event {
            Event::Key(key) => self.key(key, armed, &mut fx),
            Event::Paste(text) => self.paste(&text),
            Event::Mouse(mouse) => self.mouse(mouse, &mut fx),
            Event::Ui(ui) => self.ui(ui, armed, &mut fx),
            Event::Resize(w, h) => {
                self.width = w;
                self.height = h;
            }
            Event::Command(id) => self.run(&id, armed, &mut fx),
            Event::Saved {
                doc,
                revision,
                path,
            } => {
                if let Some(d) = self.docs.iter_mut().find(|d| d.id == doc) {
                    d.buffer.mark_saved(revision);
                    d.buffer.path = Some(path.clone());
                }
                self.notify(format!("Saved {}", path.display()));
                if self.git.branch.is_some() {
                    fx.push(Effect::GitStatus);
                }
            }
            Event::SaveFailed(err) => self.error(format!("Save failed: {err}")),
            Event::Opened { path, text } => self.opened(path, &text),
            Event::Restore(ws) => self.restore(ws),
            Event::Settings(settings) => self.apply_settings(settings),
            Event::TextColours {
                language,
                text,
                tokens,
            } => self.text_coloured(language, text, tokens),
            Event::Files(files) => {
                if files.is_empty() {
                    self.notify("No files found".into());
                } else {
                    let items = files.into_iter().map(|f| (f.clone(), f)).collect();
                    self.palette = Some(Palette::new(Source::Files, Some("Open".into()), items));
                    self.focus = Focus::Palette;
                }
            }
            Event::Clipboard(text) => {
                let text = text.unwrap_or_else(|| self.clipboard.clone());
                if !text.is_empty() {
                    self.paste(&text);
                }
            }
            Event::Error(text) => self.error(text),
            Event::RegisterPlugin { plugin, commands } => self.register(plugin, commands),
            Event::PluginFailed { plugin, message } => {
                let entry = self.plugin_mut(&plugin);
                entry.commands.clear();
                entry.keys.clear();
                entry.panel = None;
                entry.error = Some(message.clone());
                self.error(format!("Plugin {plugin} failed: {message}"));
                self.add_action("Show Output", "view.output");
            }
            Event::SetPanel { plugin, panel } => self.plugin_mut(&plugin).panel = panel,
            Event::FocusPanel(plugin) => {
                self.bar_mut(Side::Right).active = Some(plugin);
                self.focus_panel(Side::Right);
            }
            Event::ShowPicker {
                plugin,
                id,
                title,
                items,
            } => {
                let items = items.into_iter().map(|i| (i.id, i.label)).collect();
                self.palette = Some(Palette::new(
                    Source::Picker { plugin, id },
                    Some(title),
                    items,
                ));
                self.focus = Focus::Palette;
            }
            Event::InsertText(text) => self.type_text(&text, Kind::Other),
            Event::Notify(text) => self.notify(text),
            Event::SetDiagnostics {
                plugin,
                path,
                diagnostics,
            } => {
                self.diagnostics.insert((plugin, path.clone()), diagnostics);
                self.diagnostics_version += 1;
                self.place_marks(&path);
            }
            Event::RegisterLanguages { plugin, languages } => {
                self.languages.retain(|(p, _)| *p != plugin);
                self.languages
                    .extend(languages.into_iter().map(|l| (plugin.clone(), l)));
            }
            Event::SetTokens {
                path,
                layer,
                lines,
                tokens,
            } => {
                if let Some(doc) = self.doc_at(&path) {
                    let place = |(l, c): (u32, u32)| doc.pos_at_utf16(l, c);
                    let tokens = tokens
                        .into_iter()
                        .map(|t| Token {
                            start: place(t.start),
                            end: place(t.end),
                            kind: t.kind,
                        })
                        .filter(|t| t.start < t.end)
                        .collect();
                    let within = lines.map(|(a, b)| place((a, 0))..place((b, 0)));
                    doc.layers.set(layer, within, tokens);
                }
            }
            Event::SetSymbols { path, symbols } => {
                if let Some(doc) = self.doc_at(&path) {
                    let place = |(l, c): (u32, u32)| doc.pos_at_utf16(l, c);
                    let symbols = symbols
                        .into_iter()
                        .map(|s| DocSymbol {
                            start: place(s.start),
                            end: place(s.end),
                            name: s.name,
                            kind: s.kind,
                            detail: s.detail,
                            depth: s.depth as usize,
                        })
                        .collect();
                    doc.layers.symbols = Some(symbols);
                    self.outline_for = None;
                }
            }
            Event::SetFolds { path, ranges } => {
                if let Some(doc) = self.doc_at(&path) {
                    let folds = ranges
                        .into_iter()
                        .filter(|(a, b)| b > a)
                        .map(|(a, b)| (doc.pos_at_utf16(a, 0), doc.pos_at_utf16(b, 0)))
                        .collect();
                    doc.layers.folds = Some(folds);
                }
            }
            Event::ShowHover(text) => {
                let lines: Vec<String> = text.trim().lines().map(String::from).collect();
                if self.focus == Focus::Editor && !lines.is_empty() {
                    self.popup = Some(Popup::Hover(lines));
                }
            }
            Event::ShowCompletions(items) => {
                if items.is_empty() {
                    self.notify("No completions".into());
                } else if self.focus == Focus::Editor {
                    let start = self.word_start(self.cursor());
                    self.popup = Some(Popup::Completion {
                        items,
                        selected: 0,
                        start,
                    });
                }
            }
            Event::OpenAt { path, line, column } => self.open_at(path, line, column, &mut fx),
            Event::ShowReview {
                plugin,
                id,
                path,
                old,
                new,
            } => self.show_review(plugin, id, path, &old, &new),
            Event::CloseReview { plugin, id } => self.close_review(&plugin, &id),
            Event::FileChanged { path, text } => self.file_changed(&path, &text),
            Event::GitBase { path, text } => {
                for doc in &mut self.docs {
                    if doc.buffer.path.as_ref() == Some(&path) {
                        doc.set_git_base(text.clone());
                    }
                }
            }
            Event::Tree(files) => {
                let tree = self.tree.get_or_insert_with(Default::default);
                tree.files = files;
                tree.built_for = None;
            }
            Event::SearchResults { query, hits } => {
                if query == self.search.query {
                    self.search.hits = hits;
                    self.search.searching = false;
                }
            }
            Event::GitStatus {
                branch,
                files,
                staged,
                changed,
            } => {
                self.git.branch = branch;
                self.git.files = files;
                self.git.staged = staged;
                self.git.changed = changed;
                self.git.version += 1;
            }
            Event::ShowDiff { path, old, new } => {
                let id = path.display().to_string();
                self.show_review(String::new(), id, path, &old, &new);
            }
        }
        for doc in &mut self.docs {
            let changes = doc.sync();
            // Other groups showing the document keep their place in it.
            for group in &mut self.groups {
                if let Some(view) = group.saved.get_mut(&doc.id) {
                    view.follow(&changes);
                }
            }
        }
        self.scroll_out_of_folds();
        // An edited preview stays open.
        if let Some(id) = self.preview
            && self
                .index_of(id)
                .is_none_or(|i| self.docs[i].buffer.is_dirty())
        {
            self.preview = None;
        }
        self.sync_groups();
        self.build_views();
        self.check_completion();
        // Follow the cursor only when something moved it, so a view scrolled
        // with the wheel stays put while plugins update their panels.
        if self.view_key() != before {
            // Moving into a folded block opens it.
            let line = self.doc().line_col(self.doc().cursor).0;
            if self.is_hidden(line) {
                self.reveal_cursor();
            }
            self.scroll_to_cursor();
        }
        for side in [Side::Left, Side::Right, Side::Bottom] {
            self.select(side, self.selected_index(side));
        }
        fx
    }

    /// What decides whether the view must scroll to the cursor.
    fn view_key(&self) -> (u64, usize, u64, u16, u16, bool, bool) {
        let doc = self.doc();
        (
            doc.id,
            doc.cursor,
            doc.buffer.version(),
            self.width,
            self.height,
            self.side_bar_visible(),
            self.panel_visible(),
        )
    }

    fn key(&mut self, key: Key, armed: Option<&'static str>, fx: &mut Vec<Effect>) {
        // Keys from the settings come first; one bound to nothing does
        // nothing.
        if let Some((_, command)) = self.user_keys.iter().find(|(k, _)| *k == key) {
            if command.is_empty() {
                return;
            }
            let command = command.clone();
            return self.run(&command, armed, fx);
        }
        if let Some(id) = bound(GLOBAL_KEYS, key) {
            return self.run(id, armed, fx);
        }
        if matches!(
            self.focus,
            Focus::Editor | Focus::Panel | Focus::SideBar | Focus::Bottom
        ) {
            let plugin_key = self.plugins.iter().find_map(|p| {
                let (_, command) = p.keys.iter().find(|(k, _)| *k == key)?;
                Some((p.name.clone(), command.clone()))
            });
            if let Some((plugin, command)) = plugin_key {
                return fx.push(Effect::PluginCommand { plugin, command });
            }
        }
        match self.focus {
            Focus::Editor => self.editor_key(key, fx),
            Focus::Find => self.find_key(key),
            Focus::Palette => self.palette_key(key, armed, fx),
            Focus::Panel => self.panel_key(Side::Right, key, fx),
            Focus::SideBar => self.panel_key(Side::Left, key, fx),
            Focus::Bottom => self.panel_key(Side::Bottom, key, fx),
            Focus::Menu => self.menu_key(key, fx),
        }
    }

    fn run(&mut self, id: &str, armed: Option<&'static str>, fx: &mut Vec<Effect>) {
        match id {
            "file.save" => {
                let doc = self.doc();
                match &doc.buffer.path {
                    Some(path) => fx.push(Effect::Save {
                        doc: doc.id,
                        path: path.clone(),
                        text: doc.buffer.text(),
                        revision: doc.buffer.revision(),
                    }),
                    None => self.error("No file name. Start with `merlin <file>`.".into()),
                }
            }
            "file.open" => fx.push(Effect::ListFiles),
            "file.close" => self.close(armed == Some("file.close")),
            "file.next" => self.next_tab(true),
            "file.keep" => self.keep_open(),
            "view.breadcrumbs" => {
                self.breadcrumbs = !self.breadcrumbs;
                fx.push(set_flag("editor.breadcrumbs", self.breadcrumbs));
            }
            "view.sticky_scroll" => {
                self.sticky_scroll = !self.sticky_scroll;
                fx.push(set_flag("editor.sticky_scroll", self.sticky_scroll));
            }
            "settings.user" => fx.push(Effect::OpenSettings { workspace: false }),
            "settings.workspace" => fx.push(Effect::OpenSettings { workspace: true }),
            "edit.fold" => self.fold(),
            "edit.unfold" => self.unfold(),
            "edit.fold_all" => self.fold_all(),
            "edit.unfold_all" => self.unfold_all(),
            "file.prev" => self.next_tab(false),
            "view.split" => self.split(false),
            "view.split_down" => self.split(true),
            "edit.add_next" => self.add_next_occurrence(),
            "edit.select_all_matches" => self.select_all_occurrences(),
            "edit.cursor_above" => self.add_caret_vertical(false),
            "edit.cursor_below" => self.add_caret_vertical(true),
            "view.group1" => self.focus_group(0),
            "view.group2" => self.focus_group(1),
            "view.group3" => self.focus_group(2),
            "nav.back" => self.go_back(),
            "view.sidebar" => self.toggle_side_bar(fx),
            "view.explorer" => self.show_view(views::EXPLORER, fx),
            "view.search" => self.show_view(views::SEARCH, fx),
            "view.outline" => self.show_view(views::OUTLINE, fx),
            "go.symbol" => {
                let items = crate::symbols::symbols(self.doc())
                    .into_iter()
                    .map(|s| {
                        let label = if s.kind == "#" || s.kind == "impl" {
                            s.name
                        } else {
                            format!("{} {}", s.kind, s.name)
                        };
                        (
                            s.line.to_string(),
                            format!("{}{label}", "  ".repeat(s.depth)),
                        )
                    })
                    .collect();
                self.palette = Some(Palette::new(
                    Source::Symbols,
                    Some("Go to symbol".into()),
                    items,
                ));
                self.focus = Focus::Palette;
            }
            "view.changes" => self.show_view(views::CHANGES, fx),
            "view.panel" => self.toggle_bottom(),
            "view.minimap" => {
                self.minimap = !self.minimap;
                fx.push(set_flag("editor.minimap", self.minimap));
            }
            "view.wrap" => {
                self.wrap = !self.wrap;
                fx.push(set_flag("editor.word_wrap", self.wrap));
                self.doc_mut().hscroll = 0;
                self.scroll_to_cursor();
            }
            "view.problems" => self.show_bottom(views::PROBLEMS),
            "view.output" => self.show_bottom(views::OUTPUT),
            "view.secondary" => {
                let bar = self.bar_mut(Side::Right);
                bar.visible = !bar.visible;
                if !bar.visible && self.focus == Focus::Panel {
                    self.focus = Focus::Editor;
                }
            }
            "review.next" => {
                let next = (1..=self.docs.len())
                    .map(|k| (self.current + k) % self.docs.len())
                    .find(|&i| {
                        self.docs[i]
                            .review
                            .as_ref()
                            .is_some_and(|r| !r.plugin.is_empty())
                    });
                match next {
                    Some(i) => {
                        self.current = i;
                        self.focus = Focus::Editor;
                    }
                    None => self.notify("No changes to review".into()),
                }
            }
            "go.line" => {
                self.palette = Some(Palette::new(
                    Source::Line,
                    Some("Go to line".into()),
                    Vec::new(),
                ));
                self.focus = Focus::Palette;
            }
            "edit.comment" => self.toggle_comment(),
            "edit.indent" => self.indent_lines(true),
            "edit.outdent" => self.indent_lines(false),
            "app.about" => self.notify(format!("merlin {}", env!("CARGO_PKG_VERSION"))),
            menu if menu.starts_with("menu.") => {
                let title = menu.strip_prefix("menu.").filter(|t| *t != "open");
                self.open_menu(title);
            }
            "edit.undo" => match self.doc_mut().buffer.undo() {
                Some(cursor) => self.set_cursor(cursor),
                None => self.notify("Nothing to undo".into()),
            },
            "edit.redo" => match self.doc_mut().buffer.redo() {
                Some(cursor) => self.set_cursor(cursor),
                None => self.notify("Nothing to redo".into()),
            },
            "edit.select_all" => {
                let doc = self.doc_mut();
                doc.anchor = Some(0);
                doc.cursor = doc.buffer.len();
                self.focus = Focus::Editor;
            }
            "edit.copy" => self.copy(false, fx),
            "edit.cut" => self.copy(true, fx),
            "edit.paste" => fx.push(Effect::ReadClipboard),
            "find.open" => self.open_find(false),
            "find.replace" => self.open_find(true),
            "palette.open" => {
                self.palette = Some(Palette::new(Source::Commands, None, Vec::new()));
                self.focus = Focus::Palette;
            }
            // Show and focus, then step through the panels, then hide.
            "panel.toggle" => self.toggle_plugin_panels(),
            "app.quit" => {
                let dirty = self.docs.iter().filter(|d| d.buffer.is_dirty()).count();
                if dirty > 0 && armed != Some("app.quit") {
                    self.armed = Some("app.quit");
                    let what = match dirty {
                        1 => "Unsaved changes".to_string(),
                        n => format!("{n} files have unsaved changes"),
                    };
                    self.error(format!(
                        "{what}. Press Ctrl+Q again to quit without saving."
                    ));
                } else {
                    fx.push(Effect::Quit);
                }
            }
            other => {
                let found = self.plugins.iter().find_map(|p| {
                    let local = other.strip_prefix(&p.name)?.strip_prefix('.')?;
                    p.commands
                        .iter()
                        .any(|c| c.id == local)
                        .then(|| (p.name.clone(), local.to_string()))
                });
                match found {
                    Some((plugin, command)) => fx.push(Effect::PluginCommand { plugin, command }),
                    None => self.error(format!("Unknown command: {other}")),
                }
            }
        }
    }

    // Files ------------------------------------------------------------------

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id - 1
    }

    /// The open document of `path`, if any.
    fn doc_at(&mut self, path: &std::path::Path) -> Option<&mut Doc> {
        self.docs
            .iter_mut()
            .find(|d| d.review.is_none() && d.buffer.path.as_deref() == Some(path))
    }

    /// The language a plugin added for the document drawn, if any.
    pub(crate) fn plugin_language(&self) -> Option<&Language> {
        let ext = self.doc().buffer.path.as_ref()?.extension()?.to_str()?;
        self.languages
            .iter()
            .map(|(_, l)| l)
            .find(|l| l.extensions.iter().any(|e| e == ext))
    }

    /// Switches to `path` if it's open, or asks the host to read it.
    fn open(&mut self, path: PathBuf, fx: &mut Vec<Effect>) {
        match self
            .docs
            .iter()
            .position(|d| d.buffer.path.as_ref() == Some(&path))
        {
            Some(i) => {
                self.current = i;
                self.focus = Focus::Editor;
            }
            None => fx.push(Effect::Open(path)),
        }
    }

    /// Opens a file as the preview tab, replacing the last preview, unless
    /// it's open already.
    pub(crate) fn open_preview(&mut self, path: PathBuf, fx: &mut Vec<Effect>) {
        let open = self
            .docs
            .iter()
            .any(|d| d.buffer.path.as_ref() == Some(&path));
        if !open {
            self.opening_preview = Some(path.clone());
        }
        self.open(path, fx);
    }

    /// Keeps the current tab open, if it's the preview.
    pub(crate) fn keep_open(&mut self) {
        if self.preview == Some(self.doc().id) {
            self.preview = None;
        }
    }

    fn opened(&mut self, path: PathBuf, text: &str) {
        self.focus = Focus::Editor;
        let as_preview = self.opening_preview.take().is_some_and(|p| p == path);
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.buffer.path.as_ref() == Some(&path))
        {
            self.current = i;
            return;
        }
        let doc = Doc::new(self.new_id(), text, Some(path.clone()));
        let id = doc.id;
        // The new preview takes the old one's place, if this group has it.
        let old = self
            .preview
            .filter(|old| as_preview && self.groups[self.group].tabs.contains(old))
            .and_then(|old| self.index_of(old));
        if as_preview {
            self.preview = Some(id);
        }
        if let Some(i) = old {
            let old_id = self.docs[i].id;
            self.docs[i] = doc;
            self.current = i;
            let group = &mut self.groups[self.group];
            for tab in &mut group.tabs {
                if *tab == old_id {
                    *tab = id;
                }
            }
            group.saved.remove(&old_id);
            if group.active == old_id {
                group.active = id;
            }
            self.shown = (self.group, id);
        } else if self.doc().is_blank() {
            self.docs[self.current] = doc;
        } else {
            self.current += 1;
            self.docs.insert(self.current, doc);
        }
        self.place_marks(&path);
        if let Some((goto, line, column)) = self.pending_goto.take() {
            if goto == path {
                self.go_to(line, column);
            } else {
                self.pending_goto = Some((goto, line, column));
            }
        }
    }

    // Language tooling -------------------------------------------------------

    /// Places every plugin's diagnostics for `path` in its document, if
    /// it's open.
    fn place_marks(&mut self, path: &PathBuf) {
        let Some(doc) = self
            .docs
            .iter_mut()
            .find(|d| d.buffer.path.as_ref() == Some(path))
        else {
            return;
        };
        let mut marks: Vec<Mark> = self
            .diagnostics
            .iter()
            .filter(|((_, p), _)| p == path)
            .flat_map(|(_, list)| list)
            .map(|d| {
                let start = doc.pos_at_utf16(d.start.0, d.start.1);
                Mark {
                    start,
                    end: doc.pos_at_utf16(d.end.0, d.end.1).max(start),
                    severity: d.severity,
                    message: d.message.clone(),
                }
            })
            .collect();
        marks.sort_by_key(|m| (m.start, m.severity));
        doc.marks = marks;
    }

    fn open_at(&mut self, path: PathBuf, line: u32, column: u32, fx: &mut Vec<Effect>) {
        self.jumps.push((self.doc().id, self.cursor()));
        self.focus = Focus::Editor;
        match self
            .docs
            .iter()
            .position(|d| d.buffer.path.as_ref() == Some(&path))
        {
            Some(i) => {
                self.current = i;
                self.go_to(line, column);
            }
            None => {
                self.pending_goto = Some((path.clone(), line, column));
                fx.push(Effect::Open(path));
            }
        }
    }

    /// Moves to an LSP-style position in the current file, with it in the
    /// middle of the view.
    fn go_to(&mut self, line: u32, column: u32) {
        let pos = self.doc().pos_at_utf16(line, column);
        self.set_cursor(pos);
        let half = self.text_height() / 2;
        let doc = self.doc_mut();
        doc.scroll = (line as usize).saturating_sub(half);
    }

    fn go_back(&mut self) {
        while let Some((id, pos)) = self.jumps.pop() {
            if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                self.current = i;
                self.set_cursor(pos);
                self.focus = Focus::Editor;
                return;
            }
        }
        self.notify("Nowhere to go back to".into());
    }

    // Reviews -----------------------------------------------------------------

    fn show_review(&mut self, plugin: String, id: String, path: PathBuf, old: &str, new: &str) {
        let back_to = match &self.doc().review {
            Some(review) => review.back_to,
            None => self.doc().id,
        };
        let doc_id = self.new_id();
        let doc = Doc::review(
            doc_id,
            (plugin.clone(), id.clone(), path, back_to),
            old,
            new,
        );
        let same = |d: &Doc| {
            d.review
                .as_ref()
                .is_some_and(|r| r.plugin == plugin && r.id == id)
        };
        match self.docs.iter().position(same) {
            Some(i) => {
                self.docs[i] = doc;
                self.current = i;
            }
            None => {
                self.current += 1;
                self.docs.insert(self.current, doc);
            }
        }
        self.colour_review(self.current);
        self.focus = Focus::Editor;
        self.popup = None;
        let doc = self.doc_mut();
        let line = doc.line_col(doc.cursor).0;
        doc.scroll = line.saturating_sub(3);
    }

    fn close_review(&mut self, plugin: &str, id: &str) {
        let Some(i) = self.docs.iter().position(|d| {
            d.review
                .as_ref()
                .is_some_and(|r| r.plugin == plugin && r.id == id)
        }) else {
            return;
        };
        let back_to = self.docs[i].review.as_ref().map(|r| r.back_to);
        let was_current = i == self.current;
        self.docs.remove(i);
        if self.docs.is_empty() {
            let id = self.new_id();
            self.docs.push(Doc::new(id, "", None));
        }
        if i < self.current || self.current >= self.docs.len() {
            self.current = self.current.saturating_sub(1);
        }
        if was_current {
            // Another review waiting comes next; else where the user was.
            if let Some(next) = self.docs.iter().position(|d| d.review.is_some()) {
                self.current = next;
            } else if let Some(j) = self.docs.iter().position(|d| Some(d.id) == back_to) {
                self.current = j;
            }
        }
    }

    /// Keys in a review. Returns whether the key was used; moving around
    /// and copying work as in any file.
    fn review_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        use KeyCode::*;
        let Some(review) = &self.doc().review else {
            return false;
        };
        let (plugin, id, back_to) = (review.plugin.clone(), review.id.clone(), review.back_to);
        let answer = |accepted: bool| Effect::PluginReview {
            plugin: plugin.clone(),
            id: id.clone(),
            accepted,
        };
        if key.ctrl || key.alt {
            return false;
        }
        // Changes since the last commit are only looked at: Enter opens the
        // file at that line.
        if plugin.is_empty() {
            match key.code {
                Enter => {
                    let line = self.doc().line_col(self.doc().cursor).0;
                    let review = self.doc().review.as_ref().unwrap();
                    let (path, number) = (
                        review.path.clone(),
                        review.lines.get(line).map_or(0, |l| l.1),
                    );
                    self.open_at(path, number as u32, 0, fx);
                }
                Char(']') => self.next_change(true),
                Char('[') => self.next_change(false),
                Esc => {
                    if let Some(i) = self.docs.iter().position(|d| d.id == back_to) {
                        self.current = i;
                    }
                }
                Left | Right | Up | Down | Home | End | PageUp | PageDown => return false,
                _ => self.notify(self.review_hint().into()),
            }
            return true;
        }
        match key.code {
            Char('y') | Enter => {
                fx.push(answer(true));
                self.close_review(&plugin, &id);
                self.notify("Accepted".into());
            }
            Char('n') => {
                fx.push(answer(false));
                self.close_review(&plugin, &id);
                self.notify("Rejected".into());
            }
            Char(']') => self.next_change(true),
            Char('[') => self.next_change(false),
            Esc => {
                if let Some(i) = self.docs.iter().position(|d| d.id == back_to) {
                    self.current = i;
                }
            }
            Left | Right | Up | Down | Home | End | PageUp | PageDown => return false,
            _ => self.notify(format!("Review: {}", self.review_hint())),
        }
        true
    }

    /// The keys a review takes.
    pub(crate) fn review_hint(&self) -> &'static str {
        match &self.doc().review {
            Some(r) if r.plugin.is_empty() => "Enter open the file · ] next change · Esc back",
            _ => "y accept · n reject · ] next change · Esc later",
        }
    }

    /// Moves to the next or previous run of changed lines in a review.
    fn next_change(&mut self, forward: bool) {
        let doc = self.doc();
        let Some(review) = &doc.review else { return };
        let changed = |i: usize| {
            review
                .lines
                .get(i)
                .is_some_and(|(k, _)| matches!(k, LineKind::Added | LineKind::Removed))
        };
        let line = doc.line_col(doc.cursor).0;
        let starts: Vec<usize> = (0..review.lines.len())
            .filter(|&i| changed(i) && (i == 0 || !changed(i - 1)))
            .collect();
        let target = if forward {
            starts.into_iter().find(|&i| i > line)
        } else {
            starts.into_iter().rev().find(|&i| i < line)
        };
        match target {
            Some(i) => {
                let pos = self.doc().line_start(i);
                self.set_cursor(pos);
            }
            None => self.notify("No more changes".into()),
        }
    }

    // Files on disk ------------------------------------------------------------

    /// Reloads a file that changed on disk, as one step that can be undone.
    fn file_changed(&mut self, path: &PathBuf, text: &str) {
        let Some(i) = self
            .docs
            .iter()
            .position(|d| d.buffer.path.as_ref() == Some(path))
        else {
            return;
        };
        let doc = &mut self.docs[i];
        let old = doc.buffer.text();
        if old == text {
            if doc.buffer.is_dirty() {
                doc.buffer.mark_saved(doc.buffer.revision());
            }
            return;
        }
        if doc.buffer.is_dirty() {
            let name = doc.name();
            return self.error(format!(
                "{name} changed on disk, and has unsaved changes here. Saving will overwrite it."
            ));
        }
        // The cursor and selection keep their line, wherever it moved to.
        let lines = crate::diff::line_map(&old, text);
        let keep = |pos: usize| {
            let (line, col) = doc.line_col(pos);
            (lines.get(line).copied().unwrap_or(line), col)
        };
        let (cursor, anchor) = (keep(doc.cursor), doc.anchor.map(keep));
        // Replace only what differs, so marks outside it stay put.
        let old: Vec<char> = old.chars().collect();
        let new: Vec<char> = text.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let edit = Edit {
            at: prefix,
            removed: old[prefix..old.len() - suffix].iter().collect(),
            inserted: new[prefix..new.len() - suffix].iter().collect(),
        };
        doc.buffer.begin(doc.cursor, Kind::Other);
        doc.buffer.apply(edit);
        let place = |doc: &Doc, (line, col): (usize, usize)| {
            let line = line.min(doc.last_line());
            doc.line_start(line) + col.min(doc.buffer.line_len(line))
        };
        doc.cursor = place(doc, cursor);
        doc.anchor = anchor.map(|a| place(doc, a));
        doc.buffer.commit(doc.cursor);
        doc.buffer.mark_saved(doc.buffer.revision());
        let name = doc.name();
        self.notify(format!("Reloaded {name}, changed on disk"));
    }

    fn word_start(&self, pos: usize) -> usize {
        let rope = self.doc().buffer.rope();
        let mut start = pos;
        while start > 0 && {
            let c = rope.char(start - 1);
            c.is_alphanumeric() || c == '_'
        } {
            start -= 1;
        }
        start
    }

    /// Completions matching the word typed so far, best first.
    fn completions(&self) -> Vec<&Completion> {
        let Some(Popup::Completion { items, start, .. }) = &self.popup else {
            return Vec::new();
        };
        let doc = self.doc();
        if doc.cursor < *start {
            return Vec::new();
        }
        let word = doc.slice(*start, doc.cursor).to_lowercase();
        let mut ranked: Vec<(i64, usize)> = items
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((fuzzy::score(&word, &c.label)?, i)))
            .collect();
        ranked.sort_by_key(|&(score, i)| (Reverse(score), i));
        ranked.into_iter().map(|(_, i)| &items[i]).collect()
    }

    /// Closes the completion list once nothing matches or the cursor left
    /// the word.
    fn check_completion(&mut self) {
        let Some(Popup::Completion { start, .. }) = &self.popup else {
            return;
        };
        let doc = self.doc();
        let same_line =
            doc.cursor >= *start && doc.line_col(*start).0 == doc.line_col(doc.cursor).0;
        let count = self.completions().len();
        if !same_line || count == 0 || self.focus != Focus::Editor {
            self.popup = None;
        } else if let Some(Popup::Completion { selected, .. }) = &mut self.popup {
            *selected = (*selected).min(count - 1);
        }
    }

    fn accept_completion(&mut self) {
        let chosen = match &self.popup {
            Some(Popup::Completion {
                selected, start, ..
            }) => self
                .completions()
                .get(*selected)
                .map(|c| (c.insert.clone(), *start)),
            _ => None,
        };
        self.popup = None;
        if let Some((insert, start)) = chosen {
            self.replace(start, self.cursor(), &insert, Kind::Other);
        }
    }

    /// Handles a key while a popup shows. Returns whether it used the key.
    fn popup_key(&mut self, key: Key) -> bool {
        let plain = !key.ctrl && !key.alt;
        match &mut self.popup {
            None => false,
            Some(Popup::Hover(_)) => {
                self.popup = None;
                key.code == KeyCode::Esc
            }
            Some(Popup::Completion { selected, .. }) => match key.code {
                KeyCode::Up if plain => {
                    *selected = selected.saturating_sub(1);
                    true
                }
                KeyCode::Down if plain => {
                    *selected += 1;
                    true
                }
                KeyCode::Enter | KeyCode::Tab if plain => {
                    self.accept_completion();
                    true
                }
                KeyCode::Esc => {
                    self.popup = None;
                    true
                }
                // Typing narrows the list; check_completion closes it if
                // nothing's left.
                KeyCode::Char(_) | KeyCode::Backspace if plain => false,
                _ => {
                    self.popup = None;
                    false
                }
            },
        }
    }

    fn close(&mut self, confirmed: bool) {
        // A file also open in another group stays open there.
        if self.close_shared_tab() {
            return;
        }
        if self.doc().buffer.is_dirty() && !confirmed {
            self.armed = Some("file.close");
            let name = self.doc().name();
            self.error(format!(
                "{name} has unsaved changes. Press Ctrl+W again to close it."
            ));
            return;
        }
        self.docs.remove(self.current);
        if self.docs.is_empty() {
            let id = self.new_id();
            self.docs.push(Doc::new(id, "", None));
        }
        self.current = self.current.min(self.docs.len() - 1);
    }

    // Editing ----------------------------------------------------------------

    fn editor_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        use KeyCode::*;
        if self.popup_key(key) {
            return;
        }
        if self.in_review() && self.review_key(key, fx) {
            return;
        }
        if key.alt {
            return;
        }
        if self.has_carets() {
            // Esc goes back to one cursor; commands such as undo run once;
            // everything else runs at every cursor.
            if key.code == Esc && !key.ctrl && !key.shift {
                return self.clear_carets();
            }
            if !(key.ctrl && bound(EDITOR_KEYS, key).is_some()) {
                let kind = match key.code {
                    Char(_) if !key.ctrl => Kind::Typing,
                    _ => Kind::Other,
                };
                return self.each_caret(kind, |e, _| e.caret_key(key, fx));
            }
        }
        self.caret_key(key, fx);
    }

    /// A key at one cursor.
    fn caret_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        use KeyCode::*;
        let page = self.text_height().max(1) as isize;
        if matches!(
            key.code,
            Left | Right | Up | Down | Home | End | PageUp | PageDown
        ) {
            let doc = self.doc_mut();
            let selection = doc.selection();
            if key.shift {
                let cursor = doc.cursor;
                doc.anchor.get_or_insert(cursor);
            } else {
                doc.anchor = None;
                // Left and Right first collapse a selection to that side.
                if let (Some((start, end)), false, Left | Right) = (selection, key.ctrl, key.code) {
                    doc.cursor = if key.code == Left { start } else { end };
                    doc.goal = None;
                    return;
                }
            }
            match (key.code, key.ctrl) {
                (Left, false) => doc.cursor = doc.cursor.saturating_sub(1),
                (Right, false) => doc.cursor = (doc.cursor + 1).min(doc.buffer.len()),
                (Left, true) => doc.cursor = doc.word_left(doc.cursor),
                (Right, true) => doc.cursor = doc.word_right(doc.cursor),
                (Up, _) => self.move_lines(-1),
                (Down, _) => self.move_lines(1),
                (PageUp, _) => self.move_lines(-page),
                (PageDown, _) => self.move_lines(page),
                (Home, true) => doc.cursor = 0,
                (End, true) => doc.cursor = doc.buffer.len(),
                (Home, false) => {
                    let (line, col) = doc.line_col(doc.cursor);
                    let indent = doc
                        .line_chars(line)
                        .iter()
                        .take_while(|c| c.is_whitespace())
                        .count();
                    // First non-blank, then column 0.
                    doc.cursor = doc.line_start(line) + if col == indent { 0 } else { indent };
                }
                (End, false) => {
                    let (line, _) = doc.line_col(doc.cursor);
                    doc.cursor = doc.line_start(line) + doc.buffer.line_len(line);
                }
                _ => {}
            }
            if !matches!(key.code, Up | Down | PageUp | PageDown) {
                self.doc_mut().goal = None;
            }
            return;
        }
        if key.ctrl {
            if let Some(id) = bound(EDITOR_KEYS, key) {
                self.run(id, None, fx);
            }
            return;
        }
        let doc = self.doc();
        let cursor = doc.cursor;
        match key.code {
            Esc => self.doc_mut().anchor = None,
            Char(c) => self.type_text(&c.to_string(), Kind::Typing),
            Enter => {
                let at = doc.selection().map_or(cursor, |(start, _)| start);
                let (line, col) = doc.line_col(at);
                let indent: String = doc
                    .line_chars(line)
                    .into_iter()
                    .take(col)
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                self.type_text(&format!("\n{indent}"), Kind::Other);
            }
            Tab => match doc.selection() {
                Some((start, end)) if doc.line_col(start).0 != doc.line_col(end).0 => {
                    self.indent_lines(true)
                }
                _ => self.type_text(&" ".repeat(TAB_WIDTH), Kind::Other),
            },
            BackTab => self.indent_lines(false),
            Backspace => match doc.selection() {
                Some((start, end)) => self.replace(start, end, "", Kind::Other),
                None if cursor > 0 => self.replace(cursor - 1, cursor, "", Kind::Other),
                None => {}
            },
            Delete => match doc.selection() {
                Some((start, end)) => self.replace(start, end, "", Kind::Other),
                None if cursor < doc.buffer.len() => {
                    self.replace(cursor, cursor + 1, "", Kind::Other)
                }
                None => {}
            },
            _ => {}
        }
        self.doc_mut().goal = None;
    }

    fn replace(&mut self, start: usize, end: usize, text: &str, kind: Kind) {
        if self.in_review() {
            return self.notify("A review is read-only: y accepts, n rejects".into());
        }
        let doc = self.doc_mut();
        let removed = doc.slice(start, end);
        doc.buffer.begin(doc.cursor, kind);
        doc.buffer.apply(Edit {
            at: start,
            removed,
            inserted: text.to_string(),
        });
        doc.cursor = start + text.chars().count();
        doc.anchor = None;
        doc.buffer.commit(doc.cursor);
    }

    /// Types or inserts `text`, replacing the selection.
    fn type_text(&mut self, text: &str, kind: Kind) {
        let doc = self.doc();
        match doc.selection() {
            Some((start, end)) => self.replace(start, end, text, Kind::Other),
            None => self.replace(doc.cursor, doc.cursor, text, kind),
        }
    }

    /// Applies `edits`, given left to right, as one undo step, keeping the
    /// cursor and selection on the same text.
    fn apply_all(&mut self, edits: Vec<Edit>) {
        let doc = self.doc_mut();
        doc.buffer.begin(doc.cursor, Kind::Other);
        // Right to left, so each edit's offset is still right.
        for edit in edits.iter().rev() {
            doc.buffer.apply(edit.clone());
        }
        doc.cursor = map_pos(doc.cursor, &edits);
        doc.anchor = doc.anchor.map(|a| map_pos(a, &edits));
        doc.buffer.commit(doc.cursor);
    }

    /// Indents or dedents every line the selection touches.
    fn indent_lines(&mut self, indent: bool) {
        let doc = self.doc();
        let (start, end) = doc.selection().unwrap_or((doc.cursor, doc.cursor));
        let first = doc.line_col(start).0;
        let (mut last, end_col) = doc.line_col(end);
        // A selection ending at column 0 doesn't include that line.
        if last > first && end_col == 0 {
            last -= 1;
        }
        let mut edits = Vec::new();
        for line in first..=last {
            let at = doc.line_start(line);
            let chars = doc.line_chars(line);
            if indent {
                if !chars.is_empty() {
                    edits.push(Edit {
                        at,
                        removed: String::new(),
                        inserted: " ".repeat(TAB_WIDTH),
                    });
                }
            } else {
                let n = if chars.first() == Some(&'\t') {
                    1
                } else {
                    chars
                        .iter()
                        .take(TAB_WIDTH)
                        .take_while(|c| **c == ' ')
                        .count()
                };
                if n > 0 {
                    edits.push(Edit {
                        at,
                        removed: chars[..n].iter().collect(),
                        inserted: String::new(),
                    });
                }
            }
        }
        if !edits.is_empty() {
            self.apply_all(edits);
        }
    }

    /// Comments out the lines the selection touches with the language's
    /// line comment, or uncomments them if they all are.
    fn toggle_comment(&mut self) {
        let doc = self.doc();
        let ext = doc
            .buffer
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let from_plugin = self.plugin_language().and_then(|l| l.line_comment.clone());
        let doc = self.doc();
        let prefix: &str = match ext {
            _ if from_plugin.is_some() => from_plugin.as_deref().unwrap_or_default(),
            "rs" | "js" | "jsx" | "ts" | "tsx" | "c" | "h" | "cc" | "cpp" | "hpp" | "go"
            | "java" | "kt" | "swift" | "cs" | "zig" | "json" => "//",
            "py" | "toml" | "sh" | "yaml" | "yml" | "rb" | "pl" | "conf" => "#",
            "lua" | "sql" | "hs" => "--",
            _ => return self.notify("No line comments in this kind of file".into()),
        };
        let (start, end) = doc.selection().unwrap_or((doc.cursor, doc.cursor));
        let first = doc.line_col(start).0;
        let (mut last, end_col) = doc.line_col(end);
        if last > first && end_col == 0 {
            last -= 1;
        }
        // Each non-blank line: where it starts, its indent and the rest.
        let lines: Vec<(usize, usize, String)> = (first..=last)
            .filter_map(|line| {
                let chars = doc.line_chars(line);
                let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
                (indent < chars.len()).then(|| {
                    (
                        doc.line_start(line),
                        indent,
                        chars[indent..].iter().collect(),
                    )
                })
            })
            .collect();
        let commented =
            !lines.is_empty() && lines.iter().all(|(_, _, rest)| rest.starts_with(prefix));
        let column = lines
            .iter()
            .map(|(_, indent, _)| *indent)
            .min()
            .unwrap_or(0);
        let edits: Vec<Edit> = lines
            .iter()
            .map(|(at, indent, rest)| {
                if commented {
                    let space = rest[prefix.len()..].starts_with(' ') as usize;
                    Edit {
                        at: at + indent,
                        removed: rest[..prefix.len() + space].to_string(),
                        inserted: String::new(),
                    }
                } else {
                    Edit {
                        at: at + column,
                        removed: String::new(),
                        inserted: format!("{prefix} "),
                    }
                }
            })
            .collect();
        if !edits.is_empty() {
            self.apply_all(edits);
        }
    }

    /// Copies the selection, or the whole line without one, and cuts it if
    /// asked.
    fn copy(&mut self, cut: bool, fx: &mut Vec<Effect>) {
        if self.focus != Focus::Editor {
            return;
        }
        if self.has_carets() {
            // Each cursor's piece, top to bottom: selections a line each,
            // or whole lines.
            let (texts, lines) = self.caret_texts();
            let text = if lines {
                texts.concat()
            } else {
                texts.join("\n")
            };
            self.clipboard = text.clone();
            fx.push(Effect::Copy(text));
            if cut {
                self.each_caret(Kind::Other, |e, _| {
                    let doc = e.doc();
                    let (start, end) = match doc.selection() {
                        Some(sel) => sel,
                        None if lines => {
                            let line = doc.line_col(doc.cursor).0;
                            let end = if line < doc.last_line() {
                                doc.line_start(line + 1)
                            } else {
                                doc.buffer.len()
                            };
                            (doc.line_start(line), end)
                        }
                        None => return,
                    };
                    e.replace(start, end, "", Kind::Other);
                });
            }
            return;
        }
        let doc = self.doc();
        let (start, end, text) = match doc.selection() {
            Some((start, end)) => (start, end, doc.slice(start, end)),
            None => {
                let (line, _) = doc.line_col(doc.cursor);
                let start = doc.line_start(line);
                let end = if line < doc.last_line() {
                    doc.line_start(line + 1)
                } else {
                    doc.buffer.len()
                };
                let mut text = doc.slice(start, end);
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                (start, end, text)
            }
        };
        self.clipboard = text.clone();
        fx.push(Effect::Copy(text));
        if cut && start < end {
            self.replace(start, end, "", Kind::Other);
        }
    }

    fn set_cursor(&mut self, cursor: usize) {
        let doc = self.doc_mut();
        doc.extra.clear();
        doc.cursor = cursor.min(doc.buffer.len());
        doc.goal = None;
        doc.anchor = None;
    }

    fn move_lines(&mut self, delta: isize) {
        if self.row_based() {
            return self.move_rows(delta);
        }
        let doc = self.doc_mut();
        let (line, col) = doc.line_col(doc.cursor);
        let goal = match doc.goal {
            Some(goal) => goal,
            None => {
                let goal = doc.display_col(line, col);
                doc.goal = Some(goal);
                goal
            }
        };
        let target = (line as isize + delta).clamp(0, doc.last_line() as isize) as usize;
        doc.cursor = doc.pos_at(target, goal);
    }

    fn paste(&mut self, text: &str) {
        let first_line = text.lines().next().unwrap_or("");
        match self.focus {
            Focus::Editor => {
                let text = text.replace("\r\n", "\n");
                // One line per cursor, when there are as many lines as
                // cursors, as VS Code pastes.
                let count = self.carets().len();
                let lines: Vec<&str> = text
                    .strip_suffix('\n')
                    .unwrap_or(&text)
                    .split('\n')
                    .collect();
                let spread = count > 1 && lines.len() == count;
                self.each_caret(Kind::Other, |e, k| {
                    let piece = if spread { lines[k] } else { text.as_str() };
                    e.type_text(piece, Kind::Other)
                });
            }
            Focus::Find => {
                if let Some(find) = &mut self.find {
                    find.field().push_str(first_line);
                    if !find.in_replace {
                        let origin = find.origin;
                        self.find_from(origin);
                    }
                }
            }
            Focus::Palette => {
                if let Some(palette) = &mut self.palette {
                    palette.query.push_str(first_line);
                    palette.selected = 0;
                }
            }
            Focus::Panel | Focus::SideBar | Focus::Bottom => {
                let side = self.focused_side().unwrap_or(Side::Right);
                if let Some(input) = self.focused_input(side) {
                    self.inputs.entry(input).or_default().push_str(first_line);
                }
            }
            Focus::Menu => {}
        }
    }

    // Find -------------------------------------------------------------------

    /// Opens find, starting from the selection if it's on one line.
    fn open_find(&mut self, replace: bool) {
        let doc = self.doc();
        let selected = doc
            .selection()
            .map(|(s, e)| doc.slice(s, e))
            .filter(|t| !t.contains('\n'));
        let origin = doc.selection().map_or(doc.cursor, |(start, _)| start);
        let find = self.find.get_or_insert_with(|| Find {
            query: String::new(),
            replace: None,
            in_replace: false,
            origin,
        });
        if let Some(text) = &selected {
            find.query = text.clone();
            find.origin = origin;
        }
        if replace {
            find.replace.get_or_insert_default();
        }
        find.in_replace = replace && !find.query.is_empty();
        self.focus = Focus::Find;
        if selected.is_some() {
            self.find_from(origin);
        }
    }

    fn find_key(&mut self, key: Key) {
        let cursor = self.cursor();
        let Some(find) = &mut self.find else {
            self.focus = Focus::Editor;
            return;
        };
        let mut search = false;
        match key.code {
            KeyCode::Esc => {
                self.find = None;
                self.focus = Focus::Editor;
            }
            KeyCode::Tab | KeyCode::BackTab if find.replace.is_some() => {
                find.in_replace = !find.in_replace
            }
            KeyCode::Down => self.find_from(cursor + 1),
            KeyCode::Up => self.find_before(cursor),
            KeyCode::Enter if find.in_replace && key.alt => self.replace_all(),
            KeyCode::Enter if find.in_replace => self.replace_one(),
            KeyCode::Enter => self.find_from(cursor + 1),
            KeyCode::Backspace => {
                find.field().pop();
                search = !find.in_replace;
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                find.field().push(c);
                search = !find.in_replace;
            }
            _ => {}
        }
        if search && let Some(find) = &self.find {
            self.find_from(find.origin);
        }
    }

    /// The query as characters, and whether to ignore case: only when the
    /// query is all lowercase.
    fn query(&self) -> Option<(Vec<char>, bool)> {
        let query = &self.find.as_ref()?.query;
        let fold = !query.chars().any(char::is_uppercase);
        Some((query.chars().collect::<Vec<_>>(), fold)).filter(|(q, _)| !q.is_empty())
    }

    /// Every match in the document, left to right.
    fn all_matches(&self, query: &[char], fold: bool) -> Vec<usize> {
        let doc = self.doc();
        (0..=doc.last_line())
            .flat_map(|line| {
                let start = doc.line_start(line);
                line_matches(&doc.line_chars(line), query, fold)
                    .into_iter()
                    .map(move |m| start + m)
            })
            .collect()
    }

    fn match_at(&self, pos: usize) -> bool {
        let Some((query, fold)) = self.query() else {
            return false;
        };
        let doc = self.doc();
        let (line, col) = doc.line_col(pos);
        let chars = doc.line_chars(line);
        chars.len() >= col + query.len()
            && chars[col..col + query.len()]
                .iter()
                .zip(&query)
                .all(|(a, b)| same(*a, *b, fold))
    }

    /// Moves to the first match at or after `from`, wrapping around. Matches
    /// come from the current text every time, so edits can't leave stale ones.
    fn find_from(&mut self, from: usize) {
        let Some((query, fold)) = self.query() else {
            return;
        };
        let matches = self.all_matches(&query, fold);
        let hit = matches.iter().find(|m| **m >= from).or(matches.first());
        self.go_to_match(hit.copied(), &query);
    }

    /// Moves to the last match before `before`, wrapping around.
    fn find_before(&mut self, before: usize) {
        let Some((query, fold)) = self.query() else {
            return;
        };
        let matches = self.all_matches(&query, fold);
        let hit = matches
            .iter()
            .rev()
            .find(|m| **m < before)
            .or(matches.last());
        self.go_to_match(hit.copied(), &query);
    }

    fn go_to_match(&mut self, hit: Option<usize>, query: &[char]) {
        match hit {
            Some(pos) => self.set_cursor(pos),
            None => self.error(format!("No match for {}", query.iter().collect::<String>())),
        }
    }

    /// Replaces the match at the cursor, if there is one, and moves to the
    /// next.
    fn replace_one(&mut self) {
        let Some((query, _)) = self.query() else {
            return;
        };
        let with = self
            .find
            .as_ref()
            .and_then(|f| f.replace.clone())
            .unwrap_or_default();
        let cursor = self.cursor();
        if self.match_at(cursor) {
            self.replace(cursor, cursor + query.len(), &with, Kind::Other);
        }
        self.find_from(self.cursor());
    }

    fn replace_all(&mut self) {
        let Some((query, fold)) = self.query() else {
            return;
        };
        let with = self
            .find
            .as_ref()
            .and_then(|f| f.replace.clone())
            .unwrap_or_default();
        let doc = self.doc();
        let edits: Vec<Edit> = self
            .all_matches(&query, fold)
            .into_iter()
            .map(|at| Edit {
                at,
                removed: doc.slice(at, at + query.len()),
                inserted: with.clone(),
            })
            .collect();
        match edits.len() {
            0 => self.error(format!("No match for {}", query.iter().collect::<String>())),
            n => {
                self.apply_all(edits);
                self.notify(format!("Replaced {n}"));
            }
        }
    }

    // Palette ----------------------------------------------------------------

    fn commands(&self) -> Vec<(String, String)> {
        BUILTIN
            .iter()
            .map(|(id, title)| (id.to_string(), title.to_string()))
            .chain(self.plugins.iter().flat_map(|p| {
                p.commands
                    .iter()
                    .map(|c| (format!("{}.{}", p.name, c.id), c.title.clone()))
            }))
            .collect()
    }

    /// The palette's items that match the query, best first.
    fn palette_items(&self) -> Vec<(String, String)> {
        let Some(palette) = &self.palette else {
            return Vec::new();
        };
        let commands;
        let items = match palette.source {
            Source::Commands => {
                commands = self.commands();
                &commands
            }
            Source::Line => {
                let lines = self.doc().last_line() + 1;
                let item = match palette.query.trim().parse::<usize>() {
                    Ok(n) if (1..=lines).contains(&n) => (n.to_string(), format!("Go to line {n}")),
                    _ => (
                        String::new(),
                        format!("Type a line number from 1 to {lines}"),
                    ),
                };
                return vec![item];
            }
            _ => &palette.items,
        };
        let query = palette.query.to_lowercase();
        let mut ranked: Vec<(i64, usize)> = items
            .iter()
            .enumerate()
            .filter_map(|(i, (_, label))| Some((fuzzy::score(&query, label)?, i)))
            .collect();
        ranked.sort_by_key(|&(score, i)| (Reverse(score), i));
        ranked
            .into_iter()
            .take(PALETTE_LIMIT)
            .map(|(_, i)| items[i].clone())
            .collect()
    }

    fn palette_key(&mut self, key: Key, armed: Option<&'static str>, fx: &mut Vec<Effect>) {
        let count = self.palette_items().len();
        let Some(palette) = &mut self.palette else {
            self.focus = Focus::Editor;
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.close_palette();
            }
            KeyCode::Up => palette.selected = palette.selected.saturating_sub(1),
            KeyCode::Down => palette.selected = (palette.selected + 1).min(count.saturating_sub(1)),
            KeyCode::Backspace => {
                palette.query.pop();
                palette.selected = 0;
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                palette.query.push(c);
                palette.selected = 0;
            }
            KeyCode::Enter => {
                let selected = palette.selected;
                let item = self.palette_items().into_iter().nth(selected);
                let Some(palette) = self.close_palette() else {
                    return;
                };
                match (item, palette.source) {
                    (Some((id, _)), Source::Commands) => self.run(&id, armed, fx),
                    (Some((path, _)), Source::Files) => self.open(PathBuf::from(path), fx),
                    (Some((line, _)), Source::Symbols) => {
                        if let Ok(line) = line.parse() {
                            self.go_to_block(line);
                        }
                    }
                    (Some((line, _)), Source::Line) => {
                        if let Ok(n) = line.parse::<u32>() {
                            self.jumps.push((self.doc().id, self.cursor()));
                            self.go_to(n - 1, 0);
                        }
                    }
                    (Some((item, _)), Source::Picker { plugin, id }) => {
                        fx.push(Effect::PluginPick {
                            plugin,
                            picker: id,
                            item,
                        })
                    }
                    (None, _) => {}
                }
            }
            _ => {}
        }
    }

    /// Closes the palette, returning focus to the panel if a plugin opened
    /// it as a picker.
    fn close_palette(&mut self) -> Option<Palette> {
        let palette = self.palette.take();
        let picker = matches!(
            palette,
            Some(Palette {
                source: Source::Picker { .. },
                ..
            })
        );
        self.focus = if picker && self.panel_visible() {
            Focus::Panel
        } else {
            Focus::Editor
        };
        palette
    }

    /// The palette's left edge, width, and first item shown.
    fn palette_geometry(&self) -> (u16, u16, usize) {
        // Room for the frame on each side.
        let width = self.width.saturating_sub(6).min(80);
        let selected = self.palette.as_ref().map_or(0, |p| p.selected);
        (
            (self.width - width) / 2,
            width,
            selected.saturating_sub(PALETTE_ROWS - 1),
        )
    }

    // Messages ----------------------------------------------------------------

    fn notify(&mut self, text: String) {
        self.log.push((false, text.clone()));
        self.message = Some(Message {
            text,
            error: false,
            actions: Vec::new(),
        });
    }

    fn error(&mut self, text: String) {
        self.log.push((true, text.clone()));
        self.message = Some(Message {
            text,
            error: true,
            actions: Vec::new(),
        });
    }

    /// Adds a button to the notice showing.
    fn add_action(&mut self, label: &str, command: &str) {
        if let Some(message) = &mut self.message {
            message.actions.push((label.into(), command.into()));
        }
    }
}

fn same(a: char, b: char, fold: bool) -> bool {
    a == b || (fold && a.to_lowercase().eq(b.to_lowercase()))
}

/// Where `query` starts in `chars`, without overlaps.
fn line_matches(chars: &[char], query: &[char], fold: bool) -> Vec<usize> {
    let mut found = Vec::new();
    let mut i = 0;
    while i + query.len() <= chars.len() {
        if chars[i..i + query.len()]
            .iter()
            .zip(query)
            .all(|(a, b)| same(*a, *b, fold))
        {
            found.push(i);
            i += query.len();
        } else {
            i += 1;
        }
    }
    found
}

/// Where `pos` ends up after `edits`, given left to right in the original
/// text's offsets.
fn map_pos(pos: usize, edits: &[Edit]) -> usize {
    let mut shift = 0isize;
    for edit in edits {
        let removed = edit.removed.chars().count();
        let inserted = edit.inserted.chars().count() as isize;
        if pos >= edit.at + removed {
            shift += inserted - removed as isize;
        } else if pos > edit.at {
            shift += edit.at as isize + inserted - pos as isize;
        }
    }
    (pos as isize + shift) as usize
}
