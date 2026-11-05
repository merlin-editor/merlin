//! Write merlin plugins in Rust.
//!
//! Implement [`Plugin`] for a type, overriding only the methods you need,
//! and export it:
//!
//! ```ignore
//! use merlin_plugin::prelude::*;
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! impl Plugin for Hello {
//!     fn commands(&self) -> Vec<Command> {
//!         vec![Command::new("hi", "Hello: Say Hi").key("ctrl+alt+h")]
//!     }
//!
//!     fn run_command(&mut self, _id: &str) {
//!         host::notify("Hi!");
//!     }
//! }
//!
//! merlin_plugin::export!(Hello);
//! ```
//!
//! Build with `cargo build --release --target wasm32-wasip2`, and point the
//! manifest's `component` at the `.wasm` file.

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "plugin",
        pub_export_macro: true,
        default_bindings_module: "merlin_plugin::bindings",
    });
}

/// What the editor offers: the open file, the status bar, panels, pickers,
/// processes and secrets.
pub use bindings::merlin::plugin::host;
pub use bindings::merlin::plugin::types::{
    BufferEvent, Button, Command, Completion, DetailedDiagnostic, Diagnostic, DocSymbol, FoldRange, Framing, InlayHint,
    InlayKind, Input, Item, LineRange, Node, Panel, PickItem, Position, ProcessEvent, Selection,
    Setting, Severity, Signature, Styled, TextChange, Token, TokenKind, TokenLayer, Tone,
};

/// UTF-16 columns, which positions count, as LSP does.
pub mod text {
    /// The UTF-16 column of character `chars` in `line`.
    pub fn utf16_column(line: &str, chars: usize) -> u32 {
        line.chars().take(chars).map(|c| c.len_utf16() as u32).sum()
    }
}

/// Reading the plugin's settings: `[plugins.<name>]` in the user's or the
/// workspace's settings.toml, over the defaults in the manifest's
/// `[settings]`. Read them when you need them, as they can change.
pub mod settings {
    use crate::{Setting, host};

    pub fn text(key: &str) -> Option<String> {
        match host::get_setting(key)? {
            Setting::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn flag(key: &str) -> Option<bool> {
        match host::get_setting(key)? {
            Setting::Flag(b) => Some(b),
            _ => None,
        }
    }

    pub fn integer(key: &str) -> Option<i64> {
        match host::get_setting(key)? {
            Setting::Integer(i) => Some(i),
            _ => None,
        }
    }

    pub fn float(key: &str) -> Option<f64> {
        match host::get_setting(key)? {
            Setting::Float(f) => Some(f),
            Setting::Integer(i) => Some(i as f64),
            _ => None,
        }
    }
}

pub mod prelude {
    pub use crate::{
        BufferEvent, Button, Command, Completion, DetailedDiagnostic, Diagnostic, DocSymbol, FoldRange, Framing,
        InlayHint, InlayKind, LineRange, Node, Panel, PickItem, Plugin, Position, ProcessEvent,
        Selection, Severity, Signature, TextChange, Token, TokenKind, TokenLayer, Tone, host,
        settings,
    };
}

/// A plugin. Every method has a default that does nothing, so implement only
/// what you use. The editor makes one with [`Default`] when it loads the
/// plugin.
pub trait Plugin: Default + 'static {
    /// Commands for the palette, with optional key bindings.
    fn commands(&self) -> Vec<Command> {
        Vec::new()
    }

    fn run_command(&mut self, _id: &str) {}

    /// The plugin's side panel, if it has one. Called when the panel is
    /// visible and something may have changed. The editor keeps focus,
    /// scrolling and typed text, so returning a new panel resets none of it.
    fn panel(&mut self) -> Option<Panel> {
        None
    }

    fn click(&mut self, _button: &str) {}

    /// Enter in a panel input.
    fn submit(&mut self, _input: &str, _text: &str) {}

    /// The choice from [`host::show_picker`].
    fn pick(&mut self, _picker: &str, _item: &str) {}

    /// Output from a process started with [`host::spawn`].
    fn process(&mut self, _process: u32, _event: ProcessEvent) {}

    /// The open files changed.
    fn buffer(&mut self, _event: BufferEvent) {}

    /// The user accepted or rejected a diff shown with [`host::show_diff`].
    fn review(&mut self, _id: &str, _accepted: bool) {}

    /// Colours `text` as `language`, one of the languages in the
    /// manifest, by its id or an extension: for diffs and code blocks.
    fn colour_text(&mut self, _language: &str, _text: &str) -> Vec<Token> {
        Vec::new()
    }

    /// State to keep across a hot reload. Returning some also keeps the
    /// plugin's processes running.
    fn save_state(&self) -> Option<Vec<u8>> {
        None
    }

    /// Called on the fresh instance after a hot reload, with what
    /// [`Plugin::save_state`] returned.
    fn restore_state(&mut self, _state: Vec<u8>) {}
}

impl Command {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            key: None,
        }
    }

    /// Binds a key, such as `ctrl+alt+w`.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Button {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

impl Node {
    pub fn text(text: impl Into<String>) -> Self {
        Node::Text(text.into())
    }

    pub fn styled(text: impl Into<String>, tone: Tone) -> Self {
        Node::Styled(Styled {
            text: text.into(),
            tone,
        })
    }

    pub fn input(id: impl Into<String>, placeholder: impl Into<String>) -> Self {
        Node::Input(Input {
            id: id.into(),
            placeholder: placeholder.into(),
            secret: false,
        })
    }

    /// A row that's clicked like a button but drawn as text.
    pub fn item(id: impl Into<String>, label: impl Into<String>, tone: Tone) -> Self {
        Node::Item(Item {
            id: id.into(),
            label: label.into(),
            tone,
        })
    }

    pub fn markdown(text: impl Into<String>) -> Self {
        Node::Markdown(text.into())
    }

    /// An input that shows dots, for passwords and API keys.
    pub fn secret_input(id: impl Into<String>, placeholder: impl Into<String>) -> Self {
        Node::Input(Input {
            id: id.into(),
            placeholder: placeholder.into(),
            secret: true,
        })
    }
}

/// Panels build up a node at a time:
/// `Panel::new("Stats").heading("This file").text("12 words")`.
impl Panel {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            nodes: Vec::new(),
        }
    }

    pub fn node(mut self, node: Node) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn heading(self, text: impl Into<String>) -> Self {
        self.node(Node::Heading(text.into()))
    }

    pub fn text(self, text: impl Into<String>) -> Self {
        self.node(Node::text(text))
    }

    pub fn styled(self, text: impl Into<String>, tone: Tone) -> Self {
        self.node(Node::styled(text, tone))
    }

    pub fn button(self, id: impl Into<String>, label: impl Into<String>) -> Self {
        self.node(Node::Button(Button::new(id, label)))
    }

    pub fn item(self, id: impl Into<String>, label: impl Into<String>, tone: Tone) -> Self {
        self.node(Node::item(id, label, tone))
    }

    pub fn markdown(self, text: impl Into<String>) -> Self {
        self.node(Node::markdown(text))
    }

    pub fn input(self, id: impl Into<String>, placeholder: impl Into<String>) -> Self {
        self.node(Node::input(id, placeholder))
    }

    pub fn divider(self) -> Self {
        self.node(Node::Divider)
    }

    /// Nodes after this stay at the bottom of the panel.
    pub fn footer(self) -> Self {
        self.node(Node::Footer)
    }
}

#[doc(hidden)]
pub mod runtime {
    use std::any::Any;
    use std::cell::RefCell;

    use crate::{Command, Plugin};

    thread_local! {
        static PLUGIN: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None) };
    }

    /// Runs `f` on the plugin, making it on first use.
    pub fn with<P: Plugin, R>(f: impl FnOnce(&mut P) -> R) -> R {
        PLUGIN.with(|slot| {
            let mut slot = slot.borrow_mut();
            let plugin = slot
                .get_or_insert_with(|| Box::new(P::default()))
                .downcast_mut::<P>()
                .expect("a component exports one plugin type");
            f(plugin)
        })
    }

    pub fn activate<P: Plugin>(state: Option<Vec<u8>>) -> Vec<Command> {
        with(|plugin: &mut P| {
            if let Some(state) = state {
                plugin.restore_state(state);
            }
            plugin.commands()
        })
    }
}

/// Exports a [`Plugin`] type as the component's plugin.
#[macro_export]
macro_rules! export {
    ($plugin:ty) => {
        struct __MerlinExports;

        impl $crate::bindings::Guest for __MerlinExports {
            fn activate(state: Option<Vec<u8>>) -> Vec<$crate::Command> {
                $crate::runtime::activate::<$plugin>(state)
            }
            fn save_state() -> Option<Vec<u8>> {
                $crate::runtime::with(|p: &mut $plugin| $crate::Plugin::save_state(p))
            }
            fn run_command(id: String) {
                $crate::runtime::with(|p: &mut $plugin| $crate::Plugin::run_command(p, &id))
            }
            fn render_panel() -> Option<$crate::Panel> {
                $crate::runtime::with(|p: &mut $plugin| $crate::Plugin::panel(p))
            }
            fn on_click(button: String) {
                $crate::runtime::with(|p: &mut $plugin| $crate::Plugin::click(p, &button))
            }
            fn on_submit(input: String, text: String) {
                $crate::runtime::with(|p: &mut $plugin| $crate::Plugin::submit(p, &input, &text))
            }
            fn on_pick(picker: String, item: String) {
                $crate::runtime::with(|p: &mut $plugin| $crate::Plugin::pick(p, &picker, &item))
            }
            fn on_process(process: u32, event: $crate::ProcessEvent) {
                $crate::runtime::with(|p: &mut $plugin| $crate::Plugin::process(p, process, event))
            }
            fn on_buffer(event: $crate::BufferEvent) {
                $crate::runtime::with(|p: &mut $plugin| $crate::Plugin::buffer(p, event))
            }
            fn on_review(id: String, accepted: bool) {
                $crate::runtime::with(|p: &mut $plugin| $crate::Plugin::review(p, &id, accepted))
            }

            fn colour_text(language: String, text: String) -> Vec<$crate::Token> {
                $crate::runtime::with(|p: &mut $plugin| {
                    $crate::Plugin::colour_text(p, &language, &text)
                })
            }
        }

        $crate::bindings::export!(__MerlinExports with_types_in $crate::bindings);
    };
}
