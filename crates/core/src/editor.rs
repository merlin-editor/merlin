mod actions;
mod blame;
mod carets;
mod chrome;
mod colours;
mod commands;
mod compare;
mod compare_files;
mod completion;
mod context;
mod crumbs;
mod debug;
mod dialog;
mod dump;
mod editing;
mod events;
mod files;
mod find;
pub(crate) mod fold;
mod format;
mod groups;
mod hot_exit;
mod inline_change;
mod matcher;
pub use actions::{FORMAT_FILE, FORMAT_SELECTION};
pub use format::{FORMAT_WAIT_MS, FormatOnSave};
pub use groups::Edge;
pub(crate) mod guides;
mod keymap;
mod keys;
mod layout;
mod link;
mod macros;
mod search_all;
pub use macros::{Macro, MacroView};
mod menu;
mod merge;
mod merge_editor;
mod mouse;
mod open;
mod overview;
mod palette;
mod panels;
mod peek;
pub use blame::{BlameCardView, BlameCell, CommitInfo};
pub use compare::{DiffBarView, DiffMark, DiffMarks};
pub use debug::{
    BreakBoxView, BreakField, BreakMark, Breakpoint, BreakpointState, DAP, DebugBarView,
    DebugButton, DebugStart, DebugState, DebugTarget, DebugThread, DebugUpdate, DebugVariable,
    Debugger, FunctionStop, OutputKind, StackFrame, StopFilter,
};
pub use inline_change::ChangeView;
pub use merge_editor::{MergeBarView, MergeNote, MergePaneView, MergeRow, MergeSide, MergeView};
pub use peek::{PeekItem, PeekView, PreviewLine};
mod rename;
mod review;
pub(crate) use compare_files::COMPARE as COMPARE_REVIEW;
pub(crate) use rename::RENAME as RENAME_REVIEW;
pub use rename::RenameView;
mod save;
mod scm;
mod scroll;
mod select;
mod settings;
mod swatches;
mod tabs;
mod tasks;
mod terminal;
mod testing;
mod text_view;
mod update;
mod views;
mod vim;
pub use vim::{
    Keymap, LeaderKey, VimCursor, VimLineItem, VimLineView, VimMode, VimSettings, VimView,
    WhichKeyItem, WhichKeyView,
};
mod window;
mod workspace;
mod wrap;

pub use window::{
    PaletteExtra, PaletteIcon, PaletteView, RailItem, SideView, StatusView, TabView, ToastView,
    UiEvent, WindowView,
};

pub use context::{
    ContextItem, ContextView, HoverButton, PopupKind, PopupView, ProblemView, SignatureView,
};
pub use dialog::DialogView;
pub use files::{FileOp, FilePick};
pub use find::{FindAction, FindView};
pub use keymap::{UserKey, When};
pub use layout::Rect as CellRect;
pub use menu::MenuView;
pub use overview::{MINIMAP_COLUMNS, MinimapLine, MinimapView, ScrollbarView};
pub use save::{AutoSave, AutoSaveWhy, FileConfig};
pub use settings::Settings;
pub use tabs::GroupView;
pub use tasks::{Task, TaskGroup, from_tasks_file};
pub use terminal::{TermOp, TermSession, TerminalView};
pub use testing::{Outcome, PluginTest, PluginTestResult, TestFailureView, TestLensView};
pub use text_view::{CaretView, InlayView, LineView, OVERSCAN, TextView};
pub use workspace::{GroupState, Tab, Workspace};
pub use wrap::Row as WrapRow;

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use commands::BUILTIN;

use crate::buffer::Buffer;
use crate::diff::LineKind;
use crate::doc::Doc;
use crate::edit::Edit;
use crate::fuzzy;
use crate::history::{Kind, Revision};
use crate::indent::Indent;
use crate::key::{Key, KeyCode};
use crate::lang::{Completion, Diagnostic, Inlay, Language, Mark, PluginSymbol, PluginToken};
use crate::layers::{DocSymbol, Layer, Token};
use crate::panel::{Panel, PickItem};

pub use events::*;
pub use views::SearchHit;

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
    /// Case must match, even when the query is all lowercase.
    match_case: bool,
    /// Matches must be whole words.
    whole_word: bool,
    /// The query is a regular expression.
    regex: bool,
    /// Only text in this range is searched, as Find in Selection has it.
    scope: Option<(usize, usize)>,
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
    Files,
    Line,
    Symbols,
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
    /// Run Task's list.
    Tasks,
    /// The files whose unsaved changes came back.
    Restored,
    /// A name to save the last macro as.
    MacroName,
    /// What a command's › offers, such as Fold's levels: search rows' ids.
    Choices,
    /// Everything at once, or one scope of it.
    Search {
        scope: search_all::Scope,
        /// The scope's sign was typed, and shows before the query.
        signed: bool,
    },
}

enum Popup {
    /// The problems at a spot, worst first, then what the language server
    /// says about it, a line each. A hover the mouse brought up stays while
    /// the mouse is in `span`, the word and the problems' ranges.
    Hover {
        problems: Vec<context::ProblemView>,
        lines: Vec<String>,
        span: Option<(usize, usize)>,
        /// The code actions at the problems, for the fix it offers.
        fixes: Vec<crate::lang::CodeAction>,
    },
    /// Code actions to choose from, grouped by kind.
    Actions {
        actions: Vec<crate::lang::CodeAction>,
        selected: usize,
    },
    Completion {
        items: Vec<Completion>,
        selected: usize,
        /// Where the word being completed starts.
        start: usize,
    },
    /// The definition of the name under Ctrl, after a rest on it. It
    /// stays while the mouse is on the name, `span`.
    Definition {
        span: (usize, usize),
        card: link::DefinitionCard,
    },
}

/// The palette lists commands, files, or a plugin's items.
struct Palette {
    query: String,
    selected: usize,
    title: Option<String>,
    source: Source,
    /// Ids and labels, for files and pickers.
    items: Vec<(String, String)>,
    /// The search's rows, worked out as the query changes.
    rows: Vec<search_all::Row>,
}

impl Palette {
    fn new(source: Source, title: Option<String>, items: Vec<(String, String)>) -> Self {
        Self {
            query: String::new(),
            selected: 0,
            title,
            source,
            items,
            rows: Vec::new(),
        }
    }
}

struct Message {
    text: String,
    error: bool,
    /// Good news, as no conflicts left: a green dot.
    done: bool,
    /// Something went wrong that's put right, as a crash: an amber dot.
    warning: bool,
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
    /// Cursor positions in panel inputs, by plugin and input id.
    input_cursors: HashMap<(String, String), usize>,
    plugins: Vec<Plugin>,
    message: Option<Message>,
    /// A question that needs an answer first.
    dialog: Option<dialog::Dialog>,
    /// Text an input method is composing, and its cursor in it.
    preedit: Option<(String, Option<usize>)>,
    /// The selection last given to the selection clipboard: the file, its
    /// version and the range.
    selection_shared: Option<(u64, u64, usize, usize)>,
    /// Files to close once they're saved, by id.
    closing: Vec<u64>,
    /// Quitting once every file is saved.
    quitting: bool,
    /// Quitting keeps unsaved files for the next start, rather than asking.
    hot_exit: bool,
    /// Vim's state, while the keymap is Vim's.
    vim: Option<vim::Vim>,
    vim_settings: VimSettings,
    /// Line numbers count from the cursor's line, in Vim's Normal mode.
    relative_numbers: bool,
    /// Plugins' keys under the leader, by plugin.
    plugin_keys: Vec<(String, Vec<LeaderKey>)>,
    /// Each key counts, so a pause on a prefix shows which-key only if no
    /// key came since.
    which_key_tick: u64,
    which_key_shown: bool,
    recording: Option<macros::Recording>,
    macro_run: Option<macros::MacroRun>,
    /// Why the key a macro just played failed.
    macro_failed: Option<String>,
    /// The macro recorded or played last, and the Vim register it was in.
    last_macro: Option<String>,
    last_macro_register: Option<char>,
    /// The macros saved in the settings.
    macros: Vec<Macro>,
    /// The project's files, as the palette asked for them.
    palette_files: Vec<String>,
    palette_text: search_all::TextSearch,
    /// Plugins that search the project's symbols, and their commands.
    symbol_search: Vec<(String, String)>,
    /// The files in front last, and the commands the palette ran last,
    /// latest first.
    recent_files: Vec<PathBuf>,
    recent_commands: Vec<String>,
    /// Where the editor was when the palette's search opened, while its
    /// results are previewed: the doc, cursor and scroll, and the preview
    /// tab then.
    search_preview: Option<(u64, usize, usize, Option<u64>)>,
    /// The name under the pointer while Ctrl is held, and the ticks that
    /// wait on it.
    link: Option<link::Link>,
    link_tick: u64,
    /// Quit with unsaved files kept.
    keep_unsaved: bool,
    /// Files whose unsaved changes came back from the last run.
    restored: Vec<hot_exit::Restored>,
    /// A folder to open in a new window when this one quits.
    open_folder: Option<PathBuf>,
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
    /// Code actions asked for, and edits waiting for their files to open.
    actions: actions::Actions,
    /// What saving formats first, and a save waiting for it.
    format_on_save: format::FormatOnSave,
    waiting_format: Option<format::Waiting>,
    /// The box over a name being renamed (F2); the new name sent, with
    /// whether it's to review; and a rename being reviewed.
    rename_box: Option<rename::RenameBox>,
    renaming: Option<(String, String, bool)>,
    rename_review: Option<rename::Review>,
    /// The references peek view (Shift+F12), and what it was asked for:
    /// the document, line and name.
    peek: Option<peek::Peek>,
    peek_asked: Option<(u64, usize, String)>,
    /// Changes tabs show side by side or inline as the user chose with
    /// Alt+I, else as wide as the window is.
    diff_inline: Option<bool>,
    /// Changes tabs asked for again, to keep their place when they come.
    diff_refresh: HashSet<PathBuf>,
    /// A changes tab's file being opened beside it, which mustn't take
    /// its place.
    opening_beside: Option<PathBuf>,
    /// A change since the last commit opened in place from its gutter bar.
    change_peek: Option<inline_change::ChangePeek>,
    /// Files open in the merge editor, and the one asked for until it's
    /// open: its path, the base version, and what Incoming and Current are.
    merges: Vec<merge_editor::Merging>,
    pending_merge: Option<(String, String, (String, String))>,
    /// Files completed in the merge editor, to stage once they're saved.
    merge_staging: Vec<(u64, String)>,
    /// Alt+I in the merge editor: the Result alone, or side by side.
    merge_inline: Option<bool>,
    /// Compare Files, while it asks for files or reads them, and the file
    /// the Explorer's Select for Compare chose.
    comparing: Option<compare_files::Comparing>,
    compare_selected: Option<PathBuf>,
    /// The comparison a swap replaces once it's shown.
    compare_replace: Option<u64>,
    /// Two files compared side by side: the cursor is on the left one,
    /// where it was last clicked.
    compare_on_left: bool,
    /// Who last changed each line, by file, as of a version of its text,
    /// and what was last asked for. `now` is the time blame last came.
    blames: HashMap<PathBuf, (u64, crate::git::Blame)>,
    blame_asked: Option<(PathBuf, u64)>,
    now: i64,
    /// The document showing the blame column (Git: Blame), the line and
    /// commit its card is open for, commits each file's blame leaves out,
    /// and commits' messages and sizes as they come.
    blame_column: Option<u64>,
    blame_card: Option<(usize, usize)>,
    blame_ignore: HashMap<PathBuf, Vec<String>>,
    commit_info: HashMap<String, blame::CommitInfo>,
    /// Signature help: the document, where the call was when it showed,
    /// and the signature.
    signature: Option<(u64, usize, crate::lang::Signature)>,
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
    /// The rows before each line, by document, version, width, folds and
    /// the peek views' gaps.
    #[allow(clippy::type_complexity)]
    row_cache: std::cell::RefCell<
        Option<(
            (
                u64,
                u64,
                Option<usize>,
                Vec<usize>,
                Option<(usize, usize)>,
                Option<(usize, u64, bool)>,
                Option<(u64, usize, bool)>,
                u64,
            ),
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
    /// A file dropped on a group's side, to split there once it arrives.
    opening_split: Option<(PathBuf, groups::Edge)>,
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
    /// The last line can scroll to the top of the view.
    scroll_beyond_last_line: bool,
    /// Tidying on save, and saving by itself.
    saving: save::Saving,
    /// The integrated terminal's shells.
    terminals: terminal::Terminals,
    tasks: tasks::Tasks,
    tests: testing::Tests,
    debug: debug::Debugging,
    /// The shown document's tests, for its version and the results'.
    tests_cache: std::cell::RefCell<Option<testing::DocTestsCache>>,
    /// Brackets are coloured by how deep they are.
    bracket_colours: bool,
    /// Colours written in the code get a swatch.
    colour_swatches: bool,
    /// Problems are written out after their lines.
    inline_notes: bool,
    /// Inlay hints from plugins show.
    inlay_hints: bool,
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
            input_cursors: HashMap::new(),
            plugins: Vec::new(),
            message: None,
            dialog: None,
            preedit: None,
            selection_shared: None,
            closing: Vec::new(),
            quitting: false,
            hot_exit: true,
            vim: None,
            vim_settings: VimSettings::default(),
            relative_numbers: false,
            plugin_keys: Vec::new(),
            which_key_tick: 0,
            which_key_shown: false,
            recording: None,
            macro_run: None,
            macro_failed: None,
            last_macro: None,
            last_macro_register: None,
            macros: Vec::new(),
            palette_files: Vec::new(),
            palette_text: Default::default(),
            symbol_search: Vec::new(),
            recent_files: Vec::new(),
            recent_commands: Vec::new(),
            search_preview: None,
            link: None,
            link_tick: 0,
            keep_unsaved: false,
            restored: Vec::new(),
            open_folder: None,
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
            actions: Default::default(),
            format_on_save: Default::default(),
            waiting_format: None,
            rename_box: None,
            renaming: None,
            rename_review: None,
            peek: None,
            peek_asked: None,
            diff_inline: None,
            diff_refresh: HashSet::new(),
            opening_beside: None,
            change_peek: None,
            merges: Vec::new(),
            pending_merge: None,
            merge_staging: Vec::new(),
            merge_inline: None,
            comparing: None,
            compare_selected: None,
            compare_replace: None,
            compare_on_left: false,
            blames: HashMap::new(),
            blame_asked: None,
            now: 0,
            blame_column: None,
            blame_card: None,
            blame_ignore: HashMap::new(),
            commit_info: HashMap::new(),
            signature: None,
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
            opening_split: None,
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
            scroll_beyond_last_line: true,
            saving: save::Saving::default(),
            terminals: terminal::Terminals::default(),
            tasks: tasks::Tasks::default(),
            tests: testing::Tests::default(),
            debug: debug::Debugging::default(),
            tests_cache: Default::default(),
            bracket_colours: true,
            colour_swatches: true,
            inline_notes: true,
            inlay_hints: true,
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

    /// Draws Nerd Font icons, or plain ones without.
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

    /// The language a plugin added for the document drawn, if any.
    pub(crate) fn plugin_language(&self) -> Option<&Language> {
        let ext = self.doc().buffer.path.as_ref()?.extension()?.to_str()?;
        self.languages
            .iter()
            .map(|(_, l)| l)
            .find(|l| l.extensions.iter().any(|e| e == ext))
    }

    // Language tooling -------------------------------------------------------

    // Reviews -----------------------------------------------------------------

    /// The page shown where keys go, if a page is: its name.
    pub fn page_focused(&self) -> Option<&str> {
        if self.focus != Focus::Editor || self.palette.is_some() {
            return None;
        }
        self.doc().page.as_ref().map(|(name, _)| name.as_str())
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

    // Files on disk ------------------------------------------------------------

    fn close(&mut self, confirmed: bool) {
        // A file also open in another group stays open there.
        if self.close_shared_tab() {
            return;
        }
        if self.doc().buffer.is_dirty() && !confirmed {
            return self.ask(dialog::Ask::Close(self.doc().id));
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
            done: false,
            warning: false,
            actions: Vec::new(),
        });
    }

    /// A notice of something finished, with a green dot.
    fn notify_done(&mut self, text: String) {
        self.notify(text);
        if let Some(m) = &mut self.message {
            m.done = true;
        }
    }

    fn error(&mut self, text: String) {
        self.log.push((true, text.clone()));
        self.message = Some(Message {
            text,
            error: true,
            done: false,
            warning: false,
            actions: Vec::new(),
        });
    }

    /// A notice of something gone wrong and put right, with an amber dot.
    fn notify_warning(&mut self, text: String) {
        self.notify(text);
        if let Some(m) = &mut self.message {
            m.warning = true;
        }
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
