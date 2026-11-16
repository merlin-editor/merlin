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
        path: "wit",
        world: "plugin",
        pub_export_macro: true,
        default_bindings_module: "merlin_plugin::bindings",
    });
}

/// What the editor offers: the open file, the status bar, panels, pickers,
/// processes and secrets.
pub use bindings::merlin::plugin::host;
pub use bindings::merlin::plugin::types::{
    ActionKind, AgentAnswer, AgentAsks, AgentCommand, AgentContext, AgentMode, AgentOutcome,
    AgentPart, AgentRequest, AgentTool, Breakpoint, BreakpointState, BufferEvent, Button, Chip,
    ChipRow, CodeAction, Colour, Command, Completion, DebugLaunch, DebugPause, DebugState,
    DebugTarget, DebugThread, DebugVariable, DetailedDiagnostic, Diagnostic, DocSymbol, DrawCurve,
    DrawImage, DrawItem, DrawLine, DrawPoint, DrawRect, DrawText, Drawing, EditCard, EmptyState,
    FetchRequest, FetchResponse, FileEdit, FoldRange, Framing, FunctionStop, Image, Indent, InlayHint,
    InlayKind, InlineValue, Input, Item, LineRange, Location, LogLevel, Message, Node, OutputKind,
    PageSize, Panel, PickItem, Picture, PlanStep, Position, ProcessEvent, Progress, ProjectSymbol,
    RenameInfo, RgbaPicture, Row, Selection, Setting, Severity, Shape, Signature, StackFrame,
    StatusItem, StatusState, StepState, StopFilter, Styled, Task, TaskGroup, TestItem, TestOutcome,
    TestResult, TextBox, TextChange, Tile, Token, TokenKind, TokenLayer, Tone, ToolKind,
};

/// UTF-16 columns, which positions count, as LSP does.
pub mod text {
    /// The UTF-16 column of character `chars` in `line`.
    pub fn utf16_column(line: &str, chars: usize) -> u32 {
        if line.is_ascii() {
            chars.min(line.len()) as u32
        } else {
            line.chars().take(chars).map(|c| c.len_utf16() as u32).sum()
        }
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

/// Values plugins share with each other. One plugin provides a value under
/// a key, and the plugins that watch the key read it and hear when it
/// changes. A CMake plugin, say, tells the C++ plugin where the build put
/// `compile_commands.json`, without either calling the other.
///
/// Declare the keys in the manifest:
///
/// ```toml
/// [shared]
/// provides = ["compile-commands"]   # in the CMake plugin
/// watches = ["compile-commands"]    # in the C++ plugin
/// ```
///
/// Each key has a type, so both sides agree on what's in it. The keys
/// every plugin may use are here, such as [`CompileCommands`]; a plugin's
/// own start with its name and a dot, and it can define its type the same
/// way:
///
/// ```ignore
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct Targets { names: Vec<String> }
/// impl Shared for Targets { const KEY: &'static str = "cmake.targets"; }
/// ```
///
/// A value lasts while its plugin runs: it goes when the plugin stops, and
/// the watchers hear. A plugin that starts later reads what's there.
pub mod shared {
    use crate::host;
    use serde::{Serialize, de::DeserializeOwned};

    /// A value plugins share, and the key it goes under.
    pub trait Shared: Serialize + DeserializeOwned {
        const KEY: &'static str;
    }

    /// Shares `value` under its key, in place of what this plugin shared
    /// there before. The key must be in the manifest's `shared.provides`.
    pub fn provide<T: Shared>(value: &T) -> Result<(), String> {
        let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
        host::share(T::KEY, Some(&json))
    }

    /// Takes back what this plugin shares under `T`'s key.
    pub fn withdraw<T: Shared>() -> Result<(), String> {
        host::share(T::KEY, None)
    }

    /// What other plugins share under `T`'s key, with each one's name, in
    /// the order they shared it. The key must be in the manifest's
    /// `shared.watches`. A value that doesn't read as `T` is left out.
    pub fn all<T: Shared>() -> Vec<(String, T)> {
        host::shared(T::KEY)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(plugin, json)| Some((plugin, serde_json::from_str(&json).ok()?)))
            .collect()
    }

    /// The first value shared under `T`'s key, if any.
    pub fn get<T: Shared>() -> Option<T> {
        all::<T>().into_iter().next().map(|(_, value)| value)
    }

    /// Where a build put `compile_commands.json`, for C and C++ language
    /// servers such as clangd. Build plugins provide it, as CMake's does
    /// after configuring; the C/C++ plugin watches it.
    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub struct CompileCommands {
        /// The folder `compile_commands.json` is in: absolute, or relative
        /// to the workspace root.
        pub dir: String,
        /// What made it, for people to read, as `CMake · debug`.
        #[serde(default)]
        pub from: String,
    }

    impl Shared for CompileCommands {
        const KEY: &'static str = "compile-commands";
    }
}

impl Picture {
    /// Pixels row by row, four bytes each: red, green, blue, alpha.
    pub fn rgba(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Picture::Rgba(RgbaPicture {
            width,
            height,
            pixels,
        })
    }
}

pub mod prelude {
    pub use crate::{
        ActionKind, Breakpoint, BreakpointState, BufferEvent, Button, Chip, CodeAction, Command,
        Completion, DebugLaunch, DebugPause, DebugState, DebugTarget, DebugThread, DebugVariable,
        DetailedDiagnostic, Diagnostic, DocSymbol, EditCard, FileEdit, FoldRange, Framing,
        FunctionStop, Indent, InlayHint, InlayKind, InlineValue, LineRange, Location, LogLevel, Message,
        Node, OutputKind, PageSize, Panel, PickItem, Picture, Plugin, Position, ProcessEvent,
        Progress, ProjectSymbol, RenameInfo, RgbaPicture, Row, Selection, Severity, Signature,
        StackFrame, StatusItem, StatusState, StopFilter, Task, TaskGroup, TestItem, TestOutcome,
        TestResult, TextBox, TextChange, Tile, Token, TokenKind, TokenLayer, Tone, host, settings,
        shared,
    };
}

/// A plugin. Every method has a default that does nothing, so implement only
/// what you use.
pub trait Plugin: 'static {
    /// Commands for the palette, with optional key bindings.
    fn commands(&self) -> Vec<Command> {
        Vec::new()
    }

    fn run_command(&mut self, _id: &str) {}

    /// One of the views `[[views]]` in the manifest declares, by its id,
    /// when it shows and something may have changed. The editor keeps
    /// focus, scrolling and typed text, so returning a new panel resets
    /// none of it. With nothing to show, the view says its manifest's
    /// `empty`. [`host::set_view_badge`] puts a count on its icon, and
    /// [`host::focus_view`] shows it.
    fn view(&mut self, _id: &str) -> Option<Panel> {
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

    /// One of the plugin's settings changed, by its dotted key: the user
    /// set it, or the workspace did. [`settings`] reads the new value.
    fn setting_changed(&mut self, _key: &str) {}

    /// What another plugin shares under a key the manifest watches changed,
    /// or went: see [`shared`], which reads it.
    fn shared_changed(&mut self, _key: &str) {}

    /// A command line from [`host::run_in_terminal`], or one of the
    /// plugin's tests', ended: all it printed, and its exit code.
    fn terminal(&mut self, _id: &str, _output: &str, _code: Option<i32>) {}

    /// The user cancelled work started with [`host::start_progress`].
    fn cancel(&mut self, _id: &str) {}

    /// Colours `text` as `language`, one of the languages in the
    /// manifest, by its id or an extension: for diffs and code blocks.
    fn colour_text(&mut self, _language: &str, _text: &str) -> Vec<Token> {
        Vec::new()
    }

    /// A file one of the manifest's `[[editors]]` opens, by the editor's
    /// id, with its bytes, as the document numbered `surface`: its pages'
    /// sizes in points. The editor owns scrolling and zoom, and asks
    /// [`Plugin::render_tile`] for what comes into view.
    fn open_document(
        &mut self,
        _surface: u32,
        _editor: &str,
        _path: &str,
        _data: Vec<u8>,
    ) -> Result<Vec<PageSize>, String> {
        Err("this plugin opens no documents".into())
    }

    /// Part of a page, as RGBA row by row.
    fn render_tile(&mut self, _surface: u32, _tile: Tile) -> Result<RgbaPicture, String> {
        Err("this plugin draws no documents".into())
    }

    /// Where a page's text is, for selecting, Find and Copy.
    fn page_text(&mut self, _surface: u32, _page: u32) -> Vec<TextBox> {
        Vec::new()
    }

    /// A document's tab closed: what it kept can go.
    fn close_document(&mut self, _surface: u32) {}

    /// A request from `host::fetch` ended.
    fn fetched(&mut self, _id: u32, _response: Result<FetchResponse, String>) {}

    /// The user sent a message to one of the plugin's agents, by its id,
    /// in chat `chat`: a new chat the first time it's seen.
    fn agent_prompt(&mut self, _chat: u32, _agent: &str, _text: &str, _context: Vec<AgentContext>) {
    }
    /// The user pressed Stop in chat `chat`.
    fn agent_cancel(&mut self, _chat: u32) {}
    /// Chat `chat` closed: stop its agent.
    fn agent_close(&mut self, _chat: u32) {}
    /// The user answered a permission card from `host::agent_ask`.
    fn agent_answer(&mut self, _chat: u32, _request: u32, _answer: AgentAnswer) {}
    /// A file from `host::agent_read` or `host::agent_write`.
    fn agent_file(&mut self, _chat: u32, _request: u32, _result: Result<String, String>) {}
    /// A run from `host::agent_run` ended: its output and exit code.
    fn agent_run(&mut self, _chat: u32, _run: u32, _result: Result<(String, Option<i32>), String>) {
    }
    /// The user picked one of the modes from `host::agent_modes`.
    fn agent_mode(&mut self, _chat: u32, _mode: &str) {}

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
            multiline: false,
        })
    }

    /// A box that grows to several lines, as a chat's: Enter adds a line,
    /// Ctrl+Enter sends it to [`Plugin::submit`].
    pub fn multiline_input(id: impl Into<String>, placeholder: impl Into<String>) -> Self {
        Node::Input(Input {
            id: id.into(),
            placeholder: placeholder.into(),
            secret: false,
            multiline: true,
        })
    }

    /// Chips in a row, and a hint at its end.
    pub fn chips(chips: Vec<Chip>, hint: impl Into<String>) -> Self {
        Node::Chips(ChipRow {
            chips,
            hint: hint.into(),
        })
    }

    /// A message: who from, its Markdown, and chips for what it's about.
    pub fn message(author: impl Into<String>, markdown: impl Into<String>) -> Message {
        Message {
            author: author.into(),
            mine: false,
            markdown: markdown.into(),
            chips: Vec::new(),
            streaming: false,
        }
    }

    /// Work under way, `done` from 0 to 1, or none while it can't say.
    pub fn progress(label: impl Into<String>, done: Option<f32>) -> Self {
        Node::Progress(Progress {
            label: label.into(),
            detail: String::new(),
            done,
        })
    }

    /// A picture given to `host::set_image`, fitted to the view's width;
    /// `alt` says what it shows until it's there.
    pub fn image(id: impl Into<String>, alt: impl Into<String>) -> Self {
        Node::Image(Image {
            id: id.into(),
            alt: alt.into(),
        })
    }

    /// What a view says while it's empty, with buttons.
    pub fn empty(title: impl Into<String>, text: impl Into<String>, buttons: Vec<Button>) -> Self {
        Node::EmptyState(EmptyState {
            title: title.into(),
            text: text.into(),
            buttons,
        })
    }

    /// A change to a file waiting for an answer, with its buttons.
    pub fn edit(path: impl Into<String>, added: u32, removed: u32, buttons: Vec<Button>) -> Self {
        Node::EditCard(EditCard {
            path: path.into(),
            added,
            removed,
            buttons,
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
            multiline: false,
        })
    }
}

impl Chip {
    /// A chip that's clicked, its click arriving at [`Plugin::click`].
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tone: Tone::Normal,
        }
    }

    /// A chip that's only a label.
    pub fn label(label: impl Into<String>, tone: Tone) -> Self {
        Self {
            id: String::new(),
            label: label.into(),
            tone,
        }
    }
}

impl Message {
    /// Yours, so boxed.
    pub fn mine(mut self) -> Self {
        self.mine = true;
        self
    }

    pub fn chip(mut self, chip: Chip) -> Self {
        self.chips.push(chip);
        self
    }

    /// Still coming: a caret follows it.
    pub fn streaming(mut self, on: bool) -> Self {
        self.streaming = on;
        self
    }
}

impl Row {
    /// A row of a tree or list; its clicks arrive at [`Plugin::click`].
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: String::new(),
            depth: 0,
            expanded: None,
            icon: String::new(),
            chips: Vec::new(),
            count: String::new(),
            menu: Vec::new(),
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// It has rows under it: open or closed.
    pub fn expanded(mut self, open: bool) -> Self {
        self.expanded = Some(open);
        self
    }

    pub fn icon(mut self, glyph: impl Into<String>) -> Self {
        self.icon = glyph.into();
        self
    }

    pub fn chip(mut self, chip: Chip) -> Self {
        self.chips.push(chip);
        self
    }

    pub fn count(mut self, count: impl Into<String>) -> Self {
        self.count = count.into();
        self
    }

    /// What its right-click offers.
    pub fn menu(mut self, button: Button) -> Self {
        self.menu.push(button);
        self
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

    pub fn row(self, row: Row) -> Self {
        self.node(Node::Row(row))
    }

    pub fn message(self, message: Message) -> Self {
        self.node(Node::Message(message))
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
    pub fn with<P: Plugin, R>(init: fn() -> P, f: impl FnOnce(&mut P) -> R) -> R {
        PLUGIN.with(|slot| {
            let mut slot = slot.borrow_mut();
            let plugin = slot
                .get_or_insert_with(|| Box::new(init()))
                .downcast_mut::<P>()
                .expect("a component exports one plugin type");
            f(plugin)
        })
    }

    pub fn activate<P: Plugin>(init: fn() -> P, state: Option<Vec<u8>>) -> Vec<Command> {
        with(init, |plugin: &mut P| {
            if let Some(state) = state {
                plugin.restore_state(state);
            }
            plugin.commands()
        })
    }
}

/// Exports a [`Plugin`] type as the component's plugin: either `export!(MyType)`
/// when `MyType: Default`, or `export!(MyType => init_expr)`.
#[macro_export]
macro_rules! export {
    ($plugin:ty) => {
        $crate::export!($plugin => <$plugin as ::core::default::Default>::default());
    };
    ($plugin:ty => $init:expr) => {
        struct __MerlinExports;

        fn __merlin_init() -> $plugin {
            $init
        }

        impl $crate::bindings::Guest for __MerlinExports {
            fn activate(state: Option<Vec<u8>>) -> Vec<$crate::Command> {
                $crate::runtime::activate::<$plugin>(__merlin_init, state)
            }
            fn save_state() -> Option<Vec<u8>> {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::save_state(p)
                })
            }
            fn run_command(id: String) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::run_command(p, &id)
                })
            }
            fn render_view(view: String) -> Option<$crate::Panel> {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::view(p, &view)
                })
            }
            fn on_click(button: String) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::click(p, &button)
                })
            }
            fn on_submit(input: String, text: String) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::submit(p, &input, &text)
                })
            }
            fn on_pick(picker: String, item: String) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::pick(p, &picker, &item)
                })
            }
            fn on_process(process: u32, event: $crate::ProcessEvent) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::process(p, process, event)
                })
            }
            fn on_buffer(event: $crate::BufferEvent) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::buffer(p, event)
                })
            }
            fn on_review(id: String, accepted: bool) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::review(p, &id, accepted)
                })
            }

            fn colour_text(language: String, text: String) -> Vec<$crate::Token> {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::colour_text(p, &language, &text)
                })
            }

            fn on_terminal(id: String, output: String, code: Option<i32>) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::terminal(p, &id, &output, code)
                })
            }

            fn on_cancel(id: String) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::cancel(p, &id)
                })
            }

            fn open_document(
                surface: u32,
                editor: String,
                path: String,
                data: Vec<u8>,
            ) -> Result<Vec<$crate::PageSize>, String> {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::open_document(p, surface, &editor, &path, data)
                })
            }

            fn render_tile(surface: u32, tile: $crate::Tile) -> Result<$crate::RgbaPicture, String> {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::render_tile(p, surface, tile)
                })
            }

            fn page_text(surface: u32, page: u32) -> Vec<$crate::TextBox> {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::page_text(p, surface, page)
                })
            }

            fn close_document(surface: u32) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::close_document(p, surface)
                })
            }

            fn on_fetch(id: u32, response: Result<$crate::FetchResponse, String>) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::fetched(p, id, response)
                })
            }

            fn on_agent_prompt(chat: u32, agent: String, text: String, context: Vec<$crate::AgentContext>) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::agent_prompt(p, chat, &agent, &text, context)
                })
            }

            fn on_agent_cancel(chat: u32) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::agent_cancel(p, chat)
                })
            }

            fn on_agent_close(chat: u32) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::agent_close(p, chat)
                })
            }

            fn on_agent_answer(chat: u32, request: u32, answer: $crate::AgentAnswer) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::agent_answer(p, chat, request, answer)
                })
            }

            fn on_agent_file(chat: u32, request: u32, got: Result<String, String>) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::agent_file(p, chat, request, got)
                })
            }

            fn on_agent_run(chat: u32, run: u32, ended: Result<(String, Option<i32>), String>) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::agent_run(p, chat, run, ended)
                })
            }

            fn on_agent_mode(chat: u32, mode: String) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::agent_mode(p, chat, &mode)
                })
            }

            fn on_setting_changed(key: String) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::setting_changed(p, &key)
                })
            }

            fn on_shared(key: String) {
                $crate::runtime::with(__merlin_init, |p: &mut $plugin| {
                    $crate::Plugin::shared_changed(p, &key)
                })
            }
        }

        $crate::bindings::export!(__MerlinExports with_types_in $crate::bindings);
    };
}
