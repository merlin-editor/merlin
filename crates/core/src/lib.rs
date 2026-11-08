//! The editor core. It does no IO: the frontend turns input into [`Event`]s,
//! feeds them to [`Editor::update`], and carries out the [`Effect`]s it gets
//! back (writing files, calling plugins). What it shows is plain data for
//! the window to draw, so the same code drives the window and the tests.

pub mod buffer;
pub mod diff;
pub mod doc;
pub mod edit;
pub mod editor;
mod fuzzy;
pub mod git;
pub mod history;
mod icons;
pub use icons::file_icon;
pub mod indent;
pub mod key;
pub mod lang;
pub mod layers;
pub mod lsp;
pub mod panel;
pub mod script;
pub mod style;
pub mod symbols;

pub use editor::{
    AutoSave, AutoSaveWhy, Command, ContextItem, ContextView, DialogView, Editor, Effect, Event,
    FORMAT_WAIT_MS, FileConfig, FileOp, FilePick, FindAction, FindView, Focus, FormatOnSave,
    GitAction, GitMove, HoverButton, KeyBinding, Keymap, LanguageCardHit, LanguageCardView,
    LanguageRowView, LeaderKey, Macro, MacroView, MenuView, MergeBarView, MergeKind, MergeNote,
    MergePaneView, MergeRow, MergeSide, MergeState, MergeView, MouseKind, Outcome, PaletteExtra,
    PaletteIcon, PaletteView, PluginSummary, PluginTest, PluginTestResult, Pointer, PopupKind,
    PopupView, ProblemView, RailItem, SETTINGS_PAGE, SearchHit, Settings, Side, SideView,
    SignatureView, StatusMark, StatusView, SyncKind, SyncOutcome, SyncState, TabView, Task,
    TaskGroup, TermOp, TermSession, TerminalView, TextView, ThemeKind, ToastView, UiEvent, UserKey,
    VimCursor, VimLineItem, VimLineView, VimMode, VimSettings, VimView, When, WhichKeyItem,
    WhichKeyView, WindowView, Workspace, from_tasks_file, plugin_mark,
};
pub use editor::{
    BreakBoxView, BreakField, BreakMark, Breakpoint, BreakpointState, DAP, DebugBarView,
    DebugButton, DebugStart, DebugState, DebugTarget, DebugThread, DebugUpdate, DebugVariable,
    Debugger, FunctionStop, OutputKind, StackFrame, StopFilter,
};
pub use editor::{LogLevel, OutputBarView, OutputClick, OutputList, OutputRowView};
pub use editor::{PanelTab, Place, ToolButton, ToolView, ViewIcon, ViewSpec, view_key};
pub use key::{Key, KeyCode};
pub use lang::{Completion, Diagnostic, InlayHint, InlayKind, Severity, Signature};
pub use merlin_syntax::Syntax;
pub use panel::{Button, Entry, LogRow, Node, Panel, PickItem, Tone};
