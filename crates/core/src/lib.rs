//! The editor core. It does no IO: the frontend turns input into [`Event`]s,
//! feeds them to [`Editor::update`], and carries out the [`Effect`]s it gets
//! back (writing files, calling plugins). Rendering produces a plain
//! [`screen::Screen`], so the same code drives the terminal and the tests.

pub mod buffer;
pub mod diff;
pub mod doc;
pub mod edit;
pub mod editor;
mod fuzzy;
pub mod history;
mod icons;
pub mod key;
pub mod lang;
pub mod layers;
pub mod lsp;
mod markdown;
pub mod panel;
pub mod screen;
pub mod script;
pub mod symbols;

pub use editor::{
    Command, Editor, Effect, Event, Focus, GitAction, Mouse, MouseKind, SearchHit, Settings, Side,
    Workspace,
};
pub use key::{Key, KeyCode};
pub use lang::{Completion, Diagnostic, Severity};
pub use merlin_syntax::Syntax;
pub use panel::{Button, Entry, Node, Panel, PickItem, Tone};
