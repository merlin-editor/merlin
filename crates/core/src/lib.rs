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
pub mod history;
mod icons;
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
    FileConfig, FileOp, FilePick, FindAction, FindView, Focus, GitAction, KeyBinding, MenuView,
    MouseKind, PaletteView, PluginSummary, Pointer, PopupKind, PopupView, RailItem, SearchHit,
    Settings, Side, SideView, StatusView, TabView, TextView, ThemeKind, ToastView, UiEvent,
    UserKey, When, WindowView, Workspace,
};
pub use key::{Key, KeyCode};
pub use lang::{Completion, Diagnostic, Severity};
pub use merlin_syntax::Syntax;
pub use panel::{Button, Entry, Node, Panel, PickItem, Tone};
