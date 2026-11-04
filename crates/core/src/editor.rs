mod carets;
mod chrome;
mod colours;
mod commands;
mod context;
mod crumbs;
mod dump;
mod editing;
mod files;
mod find;
pub(crate) mod fold;
mod groups;
pub(crate) mod guides;
mod keymap;
mod keys;
mod layout;
mod menu;
mod mouse;
mod overview;
mod palette;
mod panels;
mod scroll;
mod select;
mod settings;
mod tabs;
mod text_view;
mod views;
mod window;
mod workspace;
mod wrap;

pub use window::{
    PaletteView, RailItem, SideView, StatusView, TabView, ToastView, UiEvent, WindowView,
};

pub use context::{ContextItem, ContextView, PopupKind, PopupView};
pub use files::FileOp;
pub use find::{FindAction, FindView};
pub use keymap::{UserKey, When};
pub use layout::Rect as CellRect;
pub use menu::MenuView;
pub use overview::{MINIMAP_COLUMNS, MinimapLine, MinimapView, ScrollbarView};
pub use settings::Settings;
pub use tabs::GroupView;
pub use text_view::{CaretView, LineView, OVERSCAN, TextView};
pub use workspace::{GroupState, Tab, Workspace};
pub use wrap::Row as WrapRow;

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::PathBuf;

use commands::BUILTIN;
use find::line_matches;

use crate::buffer::Buffer;
use crate::diff::LineKind;
use crate::doc::Doc;
use crate::edit::Edit;
use crate::fuzzy;
use crate::history::{Kind, Revision};
use crate::indent::Indent;
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
    /// The mouse moved with no button down.
    Move,
    /// A right click, for the menu there.
    Context,
    /// The mouse has rested a while, for a hover.
    Rest,
}

/// The mouse over an editor group's text, as a window measures it: rows
/// down from the top of the view, parts of a row scrolled off included,
/// and columns from the group's left edge, the gutter's included. Drags
/// and the release that ends them are in the group they began in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointer {
    pub group: usize,
    pub kind: MouseKind,
    pub row: f32,
    pub col: f32,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    /// 2 for a double click and 3 for a triple click, on presses; else 1.
    pub clicks: u8,
}

/// A [`Pointer`] in cells of its group's grid, as the text's own code
/// takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mouse {
    pub kind: MouseKind,
    pub x: u16,
    pub y: u16,
    pub shift: bool,
    /// Alt+click adds a cursor.
    pub alt: bool,
    /// Ctrl+click goes to the definition of what's clicked.
    pub ctrl: bool,
    /// 2 for a double click and 3 for a triple click, on presses; else 1.
    pub clicks: u8,
}

#[derive(Clone, Debug)]
pub enum Event {
    Key(Key),
    Paste(String),
    /// A click on the frame of a window that draws its own; see
    /// [`Editor::set_windowed`].
    Ui(UiEvent),
    Resize(u16, u16),
    /// Each editor group's text area, in columns and rows of the code's
    /// font, as the window has laid them out.
    Viewports(Vec<(u16, u16)>),
    Pointer(Pointer),
    /// Scrolls a group's text by rows, parts of one too, and columns.
    Scroll {
        group: usize,
        rows: f32,
        cols: f32,
    },
    /// Puts a group's top at `row` rows from the top of its file, as the
    /// scrollbar and minimap do.
    ScrollTo {
        group: usize,
        row: f64,
    },
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
    /// Unsaved changes kept from a run that didn't close cleanly: the
    /// file's text as it was, or an untitled file's with no path. The file
    /// opens with them, unsaved.
    Recovered {
        path: Option<PathBuf>,
        text: String,
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
    /// [`Effect::ListTree`]. Folders end in `/`, so empty ones show too.
    Tree(Vec<String>),
    /// A file or folder was renamed, by the host after
    /// [`Effect::File`].
    Renamed {
        from: PathBuf,
        to: PathBuf,
    },
    /// A file or folder went to the trash.
    Trashed(PathBuf),
    /// The plugins loaded, for the Plugins view.
    PluginList(Vec<PluginSummary>),
    /// The colour or icon themes there are, by name, and the one in use,
    /// for the theme pickers.
    Themes {
        kind: ThemeKind,
        themes: Vec<String>,
        current: String,
    },
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

/// A plugin as the Plugins view lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSummary {
    pub name: String,
    pub title: String,
    /// What it adds, in a few words.
    pub what: String,
    /// How it runs: "runs programs", "sandboxed", "data only", "off".
    pub tag: String,
    pub tone: crate::panel::Tone,
}

/// A command and the keys that run it, for the Keys settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub command: String,
    pub title: String,
    /// Each key as settings write it (`ctrl+k`) and as people read it
    /// (`Ctrl+K`).
    pub keys: Vec<(String, String)>,
    /// The settings change its keys.
    pub changed: bool,
    /// The keys it has without the settings, as settings write them.
    pub defaults: Vec<String>,
    /// The plugin it's from, by title.
    pub plugin: Option<String>,
}

/// What a theme picker picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    Colours,
    Icons,
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
    /// Show a colour or icon theme from a theme picker while it's chosen
    /// from, without keeping it.
    PreviewTheme(ThemeKind, String),
    /// Use a theme from now on, and save it in the user's settings.
    SetTheme(ThemeKind, String),
    /// Open the Settings window, which the window draws.
    OpenSettingsWindow,
    /// Create, rename or trash a file or folder in the project, then send
    /// what changed and list the tree again.
    File(FileOp),
    /// Make the code's font bigger or smaller by `n` steps, or with 0 go
    /// back to its size.
    Zoom(i8),
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

/// Most matches the palette keeps, so a big project stays fast.
const PALETTE_LIMIT: usize = 500;

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
    /// A path to save the file as, typed in.
    SaveAs,
    /// A name for a new file or folder in `dir`, a new name for `from`, or
    /// Enter to trash `path`.
    NewFile {
        dir: String,
    },
    NewFolder {
        dir: String,
    },
    Rename {
        from: String,
    },
    Trash {
        path: String,
    },
    Picker {
        plugin: String,
        id: String,
    },
    /// Colour or icon themes; Esc goes back to the one in use when it
    /// opened.
    Themes {
        kind: ThemeKind,
        original: String,
    },
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
    /// Where a Shift+Alt+drag box selection started: line and column.
    box_from: Option<(usize, usize)>,
    /// The word or line a double or triple click selected, and the clicks,
    /// while dragging from it.
    drag_unit: Option<(usize, usize, u8)>,
    /// Selections Shift+Alt+Right grew from, for Shift+Alt+Left.
    expanded: Vec<(usize, usize)>,
    /// Cursors before more were added, for Ctrl+U: the document and its
    /// version, the primary cursor and the others.
    cursor_undo: Vec<(u64, u64, crate::doc::Caret, Vec<crate::doc::Caret>)>,
    /// The first key of a chord such as Ctrl+K Ctrl+D, once pressed.
    chord: Option<Key>,
    /// Files closed, last last, for Ctrl+Shift+T.
    closed: Vec<PathBuf>,
    /// The Explorer row right-clicked, which its menu's commands are for.
    explorer_row: Option<String>,
    /// The project's folder, for absolute paths.
    root: PathBuf,
    popup: Option<Popup>,
    /// The right-click menu.
    context: Option<context::ContextMenu>,
    /// The word the mouse rests on, while its hover is asked for or shown.
    probe: Option<(usize, usize)>,
    /// Where a hover the mouse brought up is about, instead of the cursor.
    hover_anchor: Option<usize>,
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
    /// The widest line, by document and version.
    widest_cache: std::cell::Cell<Option<((u64, u64), usize)>>,
    /// The minimap's lines, by document, version and colours.
    #[allow(clippy::type_complexity)]
    minimap_cache:
        std::cell::RefCell<Option<((u64, u64, u64), std::rc::Rc<Vec<overview::MinimapLine>>)>>,
    /// The rows before each line, by document, version, width and folds.
    #[allow(clippy::type_complexity)]
    row_cache: std::cell::RefCell<
        Option<(
            (u64, u64, Option<usize>, Vec<usize>),
            std::rc::Rc<Vec<usize>>,
        )>,
    >,
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
    drawing: std::cell::RefCell<Option<groups::Drawing>>,
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
    /// The settings' key bindings.
    user_keys: Vec<keymap::Binding>,
    /// Every key binding, in the order they're tried.
    keymap: Vec<keymap::Binding>,
    /// Each group's text area from the window, in columns and rows.
    viewports: Vec<(u16, u16)>,
    /// Sideways scrolling short of a whole column, for touchpads.
    sideways: f32,
    /// Whether a click in the Explorer opens a preview tab.
    preview_tabs: bool,
    /// Indentation for files that have none of their own, or all files
    /// when it isn't detected.
    indent: Indent,
    detect_indentation: bool,
    /// Brackets and quotes close themselves.
    auto_close: bool,
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
    /// The plugins loaded, for the Plugins view.
    plugin_list: Vec<PluginSummary>,
    /// Colour themes and icon themes by name, and the one of each in use.
    themes: [Vec<String>; 2],
    theme: [String; 2],
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
            box_from: None,
            drag_unit: None,
            expanded: Vec::new(),
            cursor_undo: Vec::new(),
            chord: None,
            closed: Vec::new(),
            explorer_row: None,
            root: PathBuf::new(),
            popup: None,
            context: None,
            probe: None,
            hover_anchor: None,
            diagnostics: HashMap::new(),
            diagnostics_version: 0,
            pending_goto: None,
            jumps: Vec::new(),
            bracket_cache: Default::default(),
            fold_cache: Default::default(),
            row_cache: Default::default(),
            widest_cache: Default::default(),
            minimap_cache: Default::default(),
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
            keymap: keymap::builtin(),
            viewports: Vec::new(),
            sideways: 0.0,
            preview_tabs: true,
            indent: Indent::spaces(4),
            detect_indentation: true,
            auto_close: true,
            project: "merlin".into(),
            log: Vec::new(),
            problems: Vec::new(),
            plugin_list: Vec::new(),
            themes: [Vec::new(), Vec::new()],
            theme: [String::new(), String::new()],
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
    /// The project's folder, for the Explorer's Copy Path.
    pub fn set_root(&mut self, root: PathBuf) {
        self.root = root;
    }

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
    /// Every open file with unsaved changes: its id, path (none if
    /// untitled) and buffer version.
    pub fn unsaved(&self) -> Vec<(u64, Option<PathBuf>, u64)> {
        self.docs
            .iter()
            .filter(|d| d.buffer.is_dirty() && d.page.is_none())
            .map(|d| (d.id, d.buffer.path.clone(), d.buffer.version()))
            .collect()
    }

    /// The text of the open file with id `doc`.
    pub fn text_of(&self, doc: u64) -> Option<String> {
        self.docs
            .iter()
            .find(|d| d.id == doc)
            .map(|d| d.buffer.text())
    }

    pub fn open_files(&self) -> Vec<Option<PathBuf>> {
        self.docs.iter().map(|d| d.buffer.path.clone()).collect()
    }

    /// The cursor's line and UTF-16 column, as LSP counts them.
    pub fn cursor_utf16(&self) -> (u32, u32) {
        let doc = self.doc();
        let (line, col) = doc.line_col(self.plugin_cursor());
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
        ) || matches!(event, Event::Pointer(p) if p.kind == MouseKind::Down);
        let armed = if by_user { self.armed.take() } else { None };
        if by_user {
            // A notice's button needs the notice to run.
            if !matches!(event, Event::Ui(UiEvent::ToastButton(_))) {
                self.message = None;
            }
        }
        let before = self.view_key();
        match event {
            Event::Key(key) => {
                self.probe = None;
                self.hover_anchor = None;
                self.key(key, armed, &mut fx)
            }
            Event::Paste(text) => self.paste(&text),
            Event::Ui(ui) => self.ui(ui, armed, &mut fx),
            // Every group as big, until the window says otherwise.
            Event::Resize(w, h) => {
                self.width = w;
                self.height = h;
                self.viewports.clear();
            }
            Event::Viewports(sizes) => self.viewports = sizes,
            Event::Pointer(p) => self.pointer(p, &mut fx),
            Event::Scroll { group, rows, cols } => self.scroll_group(group, rows, cols),
            Event::ScrollTo { group, row } => self.in_group(group, |e| e.set_top_row(row)),
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
                // Its panel says what went wrong, until it loads again.
                let title = entry
                    .panel
                    .as_ref()
                    .map_or(plugin.clone(), |p| p.title.clone());
                entry.panel = Some(Panel {
                    title,
                    nodes: vec![
                        crate::panel::Node::Styled(
                            format!("{plugin} failed"),
                            crate::panel::Tone::Error,
                        ),
                        crate::panel::Node::Text(message.clone()),
                    ],
                });
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
                // A hover the mouse asked for is about the word it's on.
                self.hover_anchor = self.probe.map(|(start, _)| start);
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
            Event::Recovered { path, text } => self.recovered(path, &text),
            Event::GitBase { path, text } => {
                for doc in &mut self.docs {
                    if doc.buffer.path.as_ref() == Some(&path) {
                        doc.set_git_base(text.clone());
                    }
                }
            }
            Event::PluginList(list) => {
                self.plugin_list = list;
            }
            Event::Themes {
                kind,
                themes,
                current,
            } => {
                let i = kind as usize;
                self.themes[i] = themes;
                self.theme[i] = current;
            }
            Event::Renamed { from, to } => self.renamed(from, to),
            Event::Trashed(path) => self.trashed(path),
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
                    // A fold whose first line was joined to another goes.
                    let rope = doc.buffer.rope();
                    view.folds.retain(|&f| f == 0 || rope.char(f - 1) == '\n');
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

    /// Opens a page, such as a plugin's, in a tab of its own, or goes to
    /// the tab it's in.
    pub(crate) fn open_page(&mut self, name: &str, title: &str) {
        match self
            .docs
            .iter()
            .position(|d| d.page.as_ref().is_some_and(|(n, _)| n == name))
        {
            Some(i) => self.current = i,
            None => {
                let doc = Doc::page(self.new_id(), name, title);
                self.current = (self.current + 1).min(self.docs.len());
                self.docs.insert(self.current, doc);
            }
        }
        self.focus = Focus::Editor;
        self.popup = None;
    }

    /// A plugin's commands, by title, with their keys.
    pub fn plugin_commands(&self, plugin: &str) -> Vec<(String, Option<String>)> {
        self.plugins
            .iter()
            .filter(|p| p.name == plugin)
            .flat_map(|p| {
                p.commands.iter().map(|c| {
                    let key = p
                        .keys
                        .iter()
                        .find(|(_, id)| *id == c.id)
                        .map(|(k, _)| k.to_string());
                    (c.title.clone(), key)
                })
            })
            .collect()
    }

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
    fn recovered(&mut self, path: Option<PathBuf>, text: &str) {
        let at = path.as_ref().and_then(|p| {
            self.docs
                .iter()
                .position(|d| d.buffer.path.as_ref() == Some(p))
        });
        let i = match at {
            Some(i) => i,
            // An untitled file, or one that's gone from disk.
            None => {
                let id = self.new_id();
                self.docs.push(Doc::new(id, "", path));
                self.docs.len() - 1
            }
        };
        let doc = &mut self.docs[i];
        let old = doc.buffer.text();
        if old != text {
            replace_text(doc, &old, text);
        }
        self.current = i;
        self.sync_groups();
    }

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
        replace_text(doc, &old, text);
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
        let gone = self.docs.remove(self.current);
        if let Some(path) = gone.buffer.path {
            self.closed.retain(|p| *p != path);
            self.closed.push(path);
            if self.closed.len() > 50 {
                self.closed.remove(0);
            }
        }
        if self.docs.is_empty() {
            let id = self.new_id();
            self.docs.push(Doc::new(id, "", None));
        }
        self.current = self.current.min(self.docs.len() - 1);
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

/// Changes `doc`'s text from `old` to `text` as one undo step, keeping the
/// cursor and selection on their lines.
fn replace_text(doc: &mut Doc, old: &str, text: &str) {
    // The cursor and selection keep their line, wherever it moved to.
    let lines = crate::diff::line_map(old, text);
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
}
