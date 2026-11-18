//! The editor core. It does no IO: the frontend turns input into [`Event`]s,
//! feeds them to [`Editor::update`], and carries out the [`Effect`]s it gets
//! back (writing files, calling plugins). What it shows is plain data for
//! the window to draw, so the same code drives the window and the tests.

pub mod buffer;
/// The rope the text is kept in, for hosts that read files into one.
pub use ropey;
pub mod diff;
pub mod doc;
pub mod edit;
pub mod editor;
pub mod exclude;
mod fuzzy;
pub mod git;
pub mod gitbase;
pub mod history;
mod icons;
pub use icons::file_icon;
pub mod indent;
pub mod key;
pub mod lang;
pub mod layers;
pub mod lsp;
pub mod panel;
pub mod rows;
pub mod script;
pub mod search;
pub mod style;
pub mod symbols;

pub use editor::{
    AgentAnswer, AgentAsks, AgentCall, AgentContext, AgentMode, AgentOutcome, AgentPart,
    AgentRequest, AgentSpec, AgentTool, AgentUpdate, MissingProgram, PlanStep, ToolKind,
    line_counts,
};
pub use editor::{
    AutoSave, AutoSaveWhy, ChoiceView, Command, CompletionDoc, CompletionRow, ContextItem,
    ContextView, DialogView, Editor, Effect, Event, FORMAT_WAIT_MS, FileConfig, FileOp, FilePick,
    FindAction, FindView, Focus, FormatOnSave, GitAction, GitMove, HoverButton, KeyBinding, Keymap,
    LanguageCardHit, LanguageCardView, LanguageRowView, LeaderKey, Macro, MacroView, MenuView,
    MergeBarView, MergeKind, MergeNote, MergePaneView, MergeRow, MergeSide, MergeState, MergeView,
    MouseKind, Outcome, PaletteExtra, PaletteIcon, PaletteState, PaletteView, PluginSummary,
    PluginTest, PluginTestResult, Pointer, PopupKind, PopupView, ProblemView, RailItem,
    SETTINGS_PAGE, SearchAsk, SearchHit, Settings, Side, SideView, SignatureView, StateTone,
    StatusMark, StatusView, SyncKind, SyncOutcome, SyncState, TabView, Task, TaskGroup, TermOp,
    TermSession, TerminalView, TextView, ThemeKind, ToastView, UiEvent, UserKey, VimCursor,
    VimLineItem, VimLineView, VimMode, VimSettings, VimView, When, WhichKeyItem, WhichKeyView,
    WindowView, Workspace, from_tasks_file, plugin_mark,
};
pub use editor::{
    BreakBoxView, BreakField, BreakMark, Breakpoint, BreakpointState, DAP, DebugBarView,
    DebugButton, DebugStart, DebugState, DebugTarget, DebugThread, DebugUpdate, DebugVariable,
    Debugger, DisasmLine, FunctionStop, MemoryView, OutputKind, StackFrame, StopFilter,
};
pub use editor::{
    ButtonStyle, StepMark, UpdateAction, UpdateButton, UpdateCardView, UpdateHit, UpdateOffer,
    UpdateStep, UpdateStepView, UpdateView,
};
pub use editor::{
    DocEditor, PAGE_GAP, PX_PER_PT, PageSize, Pages, SurfaceAction, SurfaceView, THUMB, TILE,
    TextBox, TileKey, Tiles, page_layout,
};
pub use editor::{FileBarView, HierarchyItem, HierarchyMode};
pub use editor::{LogLevel, OutputBarView, OutputClick, OutputList, OutputRowView};
pub use editor::{
    MenuItem, MenuPlace, PanelTab, Place, ToolButton, ToolView, ViewIcon, ViewSpec, view_key,
};
pub use key::{Key, KeyCode};
pub use lang::{Completion, Diagnostic, InlayHint, InlayKind, Severity, Signature};
pub use merlin_syntax::Syntax;
pub use panel::{
    AskCard, Button, ChangedFile, ChangesBar, ChangesCard, Chip, Choice, ComposeBar, DrawColour,
    DrawItem, Drawing, EditCard, Entry, ImageNode, LogRow, Message, Node, Panel, PickItem,
    PluginImage, Shape, StepLine, StepState, Tone,
};
