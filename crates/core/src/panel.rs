//! Plugin panels: what plugins describe, and how the editor lays it out.

use crate::style::Hue;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Normal,
    Muted,
    Strong,
    Warning,
    Error,
    Success,
    /// A count in a pill.
    Badge,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Button {
    pub id: String,
    pub label: String,
}

/// A line of the Output panel: when, how serious, and what. Drawn on one
/// line in a fixed-width font, so a long log costs no wrapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRow {
    pub time: String,
    pub level: String,
    /// The level's colour.
    pub tone: Tone,
    pub text: String,
}

/// A picture a plugin gave, kept by the editor: premultiplied RGBA, row by
/// row. Each one made has a new `version`, which is what tells two apart.
#[derive(Debug)]
pub struct PluginImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub version: u64,
}

impl PluginImage {
    /// A picture from premultiplied pixels, with a version of its own.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            width,
            height,
            pixels,
            version: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl PartialEq for PluginImage {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

impl Eq for PluginImage {}

/// A plugin's picture in a view: its id, what it shows in words, and the
/// picture, filled in by the editor as the view is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageNode {
    pub id: String,
    pub alt: String,
    pub image: Option<std::sync::Arc<PluginImage>>,
}

/// A colour in a drawing: one of the theme's by name, or its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawColour {
    Theme(String),
    Rgb(u32),
}

/// A shape in a drawing, in the drawing's units.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect {
        rect: [f32; 4],
        radius: f32,
        fill: Option<DrawColour>,
        stroke: Option<DrawColour>,
    },
    Line {
        from: [f32; 2],
        to: [f32; 2],
        width: f32,
        colour: DrawColour,
    },
    /// Cubic Bézier curves end to end: a start, then three points each.
    Curve {
        points: Vec<[f32; 2]>,
        width: f32,
        colour: DrawColour,
    },
    Text {
        at: [f32; 2],
        text: String,
        size: f32,
        colour: DrawColour,
        mono: bool,
    },
    /// A plugin's picture by its id, stretched to `rect`; the picture is
    /// filled in as the view is shown.
    Image {
        rect: [f32; 4],
        id: String,
        image: Option<std::sync::Arc<PluginImage>>,
    },
}

/// A shape, and the id a click on it sends; empty for none.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawItem {
    pub id: String,
    pub shape: Shape,
}

/// Shapes `width` by `height` units, scaled to the view's width.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    pub width: f32,
    pub height: f32,
    pub items: Vec<DrawItem>,
}

/// Its sizes are never NaN, which the host sees to.
impl Eq for Drawing {}

/// One element of a plugin panel. Plugins describe what to show; the editor
/// owns layout, focus, scrolling and input text, so a re-render can't reset
/// them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Heading(String),
    Text(String),
    Styled(String, Tone),
    Button(Button),
    Buttons(Vec<Button>),
    /// Small buttons side by side, as a view's tools: Run All, Run Failed.
    /// The editor's own views have these; plugins don't.
    Tools(Vec<Button>),
    /// A row that can be focused and clicked like a button, drawn as text:
    /// a file in a tree, a problem in a list.
    Item {
        id: String,
        label: String,
        tone: Tone,
    },
    /// A row in a tree or list of the editor's own views: a file, a search
    /// hit, a problem. Boxed, as it's much the biggest.
    Entry(Box<Entry>),
    /// Markdown: headings, lists, quotes, code blocks.
    Markdown(String),
    /// A text field. A secret one shows dots instead of the text.
    Input {
        id: String,
        placeholder: String,
        secret: bool,
        multiline: bool,
    },
    Divider,
    /// A line of a log, in the Output panel.
    Log(LogRow),
    /// Nodes after this stay at the bottom; the ones before it scroll.
    Footer,
    /// A title row with buttons at its right end, each with its key; the
    /// last is the main one. The title's toned parts are code, as names.
    /// The editor's own views have these; plugins don't.
    Header {
        title: Vec<(String, Tone)>,
        detail: String,
        buttons: Vec<(Button, String)>,
    },
    /// A tinted box for what's under way, as a merge: a title, its strong
    /// parts in bold, a line under it, and buttons.
    Banner {
        title: Vec<(String, bool)>,
        detail: String,
        tone: Tone,
        buttons: Vec<Button>,
    },
    /// The main button, as wide as the panel; dimmed while it can't be
    /// used yet, when a click says why instead.
    Wide {
        button: Button,
        ready: bool,
    },
    /// Chips in a row, each clicked on its own, and a quiet hint at the
    /// row's right end.
    Chips {
        chips: Vec<Chip>,
        hint: String,
    },
    /// A message in a conversation.
    Message(Box<Message>),
    /// A change to a file waiting for an answer.
    Edit(Box<EditCard>),
    /// Work under way: what, a line more, and how far in thousandths;
    /// none while it can't say.
    Progress {
        label: String,
        detail: String,
        done: Option<u16>,
    },
    /// What a view says while it's empty, in its middle: a title, a
    /// sentence, and buttons, the first the main one.
    Empty {
        title: String,
        text: String,
        buttons: Vec<Button>,
    },
    /// A plugin's picture, fitted to the view's width.
    Image(Box<ImageNode>),
    /// Shapes a plugin drew, in the theme's colours and the editor's
    /// fonts, scaled to the view's width.
    Drawing(Box<Drawing>),
    /// An agent's plan, ticked off as it goes. The editor's own views have
    /// these; plugins don't.
    Steps(Vec<(String, StepState)>),
    /// One step an agent took, on a quiet line: "✓ Edited `palette.rs` +5 −6".
    Step(Box<StepLine>),
    /// A permission card: what an agent wants to do, and the answers.
    Ask(Box<AskCard>),
    /// The files an agent's turn changed, each with Review and Revert.
    Changes(Box<ChangesCard>),
    /// A command to copy, in a box with Copy at its right end.
    Command {
        text: String,
        copy: Button,
    },
    /// A bar of chips with a quiet line at its right, kept at the top of
    /// the view when it's the first node: an agent's header.
    Bar {
        chips: Vec<Chip>,
        hint: String,
    },
    /// Your message in an agent's chat, by its turn: a bubble that shows
    /// Edit, Copy and Rewind when hovered (`agent:turn-edit:n`,
    /// `agent:turn-copy:n`, `agent:rewind:n`).
    YouSaid {
        turn: usize,
        message: Box<Message>,
    },
    /// The start of an agent's reply: who (its mark and name) and the mode
    /// it wrote in, a badge whose colour runs down the reply's side until
    /// [`Node::ReplyEnd`].
    Reply {
        author: String,
        mode: Option<(String, String)>,
    },
    ReplyEnd,
    /// A run of an agent's steps, folded to one line: what they did, how
    /// long they took, and whether one is still running. Clicked to open.
    Fold {
        id: String,
        label: String,
        detail: String,
        open: bool,
        running: bool,
    },
    /// Under a finished reply: its model, time, tokens and cost, and Copy.
    TurnFoot {
        text: String,
        copy: Button,
    },
    /// Above an agent's box: its plan, folded to the step it's on (ticked
    /// so far of all), or open to every step. Clicked: `agent:plan`.
    PlanDock {
        steps: Vec<(String, StepState)>,
        open: bool,
    },
    /// Above an agent's box: a message sent while it works, waiting its
    /// turn, with Edit (`agent:queue-edit:n`), ✕ (`agent:queue-drop:n`)
    /// and Send Now (`agent:queue-now:n`).
    Queued {
        index: usize,
        text: String,
    },
    /// Above an agent's box: the files it changed, waiting to be kept or
    /// undone, with Review, Undo All and Keep All, and open, each file's
    /// Undo and Keep.
    ChangesBar(Box<ChangesBar>),
    /// The bottom row of an agent's box, under its text: the agent's
    /// mode, merlin's edit pill and the plugin's chips, how full the
    /// context is, and Send or Stop.
    ComposeBar(Box<ComposeBar>),
    /// A small menu card: a title, then items, each id, label and what it
    /// does at the right. As an agent's slash commands.
    Menu {
        title: String,
        items: Vec<(String, String, String)>,
    },
}

/// How far a step has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StepState {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
}

/// A step an agent took, as one line.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StepLine {
    pub mark: StepState,
    pub verb: String,
    /// A file or a command, in code.
    pub name: String,
    pub added: u32,
    pub removed: u32,
    /// Links at the right end, as "Terminal ↗", then quiet buttons, as Stop.
    pub links: Vec<Button>,
    pub buttons: Vec<Button>,
}

/// The files an agent changed, as the bar above its box shows them.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ChangesBar {
    /// Each file's path, lines added and removed.
    pub files: Vec<(String, u32, u32)>,
    pub open: bool,
}

impl ChangesBar {
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = [
            "agent:changes",
            "agent:review-all",
            "agent:revert-all",
            "agent:keep-all",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if self.open {
            for (path, ..) in &self.files {
                ids.push(format!("agent:review:{path}"));
                ids.push(format!("agent:revert:{path}"));
                ids.push(format!("agent:keep:{path}"));
            }
        }
        ids
    }
}

/// The row under an agent chat's text box.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ComposeBar {
    /// The agent's mode, as its badge: its name and colour (a hue's name,
    /// `#rrggbb`, or empty for the accent). Clicked: `agent:modes`.
    pub mode: Option<(String, String)>,
    /// merlin's edit pill, then the plugin's chips.
    pub chips: Vec<Chip>,
    /// How full the context is, in percent, and what it says on hover.
    /// Clicked: `agent:ring`.
    pub ring: Option<(u8, String)>,
    /// Stop while the agent works (`agent:stop`); Send otherwise
    /// (`agent:send`).
    pub stop: bool,
}

impl ComposeBar {
    /// Its clickable parts' ids, in the order Tab visits them.
    pub fn ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        if self.mode.is_some() {
            ids.push("agent:modes".to_string());
        }
        ids.extend(
            self.chips
                .iter()
                .filter(|c| !c.id.is_empty())
                .map(|c| c.id.clone()),
        );
        if self.ring.is_some() {
            ids.push("agent:ring".into());
        }
        ids.push(
            if self.stop {
                "agent:stop"
            } else {
                "agent:send"
            }
            .into(),
        );
        ids
    }
}

/// A permission card.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AskCard {
    /// The question; the parts marked true are code.
    pub title: Vec<(String, bool)>,
    /// Amber, for something outside the workspace.
    pub warn: bool,
    /// A command or a path, in a box.
    pub code: Option<String>,
    /// A change, as diff lines: `-` and `+` first.
    pub diff: Vec<String>,
    pub note: String,
    /// The main answer first; a label's parts in backticks are code.
    pub buttons: Vec<Button>,
    /// The quiet answer at the end, as Deny.
    pub quiet: Option<Button>,
    pub hint: String,
}

/// The files a turn changed.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ChangesCard {
    pub title: String,
    pub revert_all: Button,
    pub files: Vec<ChangedFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ChangedFile {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub review: Button,
    pub revert: Button,
}

/// A small label, clicked on its own when it has an id.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Chip {
    pub id: String,
    pub label: String,
    pub tone: Tone,
}

/// A message: who from, its Markdown, and chips for what it's about.
/// Yours are boxed; one still coming has a caret after it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Message {
    pub author: String,
    pub mine: bool,
    pub markdown: String,
    pub chips: Vec<Chip>,
    pub streaming: bool,
}

/// A change to a file: its path, lines added and removed, and buttons,
/// the last the main one.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EditCard {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub buttons: Vec<Button>,
}

impl Node {
    pub fn entry(entry: Entry) -> Node {
        Node::Entry(Box::new(entry))
    }
}

/// A checkbox: ticked, not, or some of what it's for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    On,
    Off,
    Some,
}

/// A line of code with one name in it changed, as a rename shows it: the
/// old name struck through, the new one after it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Swap {
    /// From 1.
    pub line: usize,
    pub before: String,
    pub old: String,
    /// Empty when the place is left out.
    pub new: String,
    pub after: String,
}

/// A clickable row with an icon, as VS Code's trees draw them.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Entry {
    pub id: String,
    pub label: String,
    pub tone: Tone,
    /// Levels of nesting, each drawn as an indent guide.
    pub depth: usize,
    /// A folder's open or closed arrow.
    pub twisty: Option<char>,
    pub icon: Option<(char, Hue)>,
    /// Letters on a tile in place of the icon, in its colour, as a
    /// plugin's.
    pub monogram: Option<String>,
    /// Dimmed text after the label, such as the folder a file is in.
    pub detail: String,
    /// At the right edge: a count or a git letter.
    pub badge: String,
    pub badge_tone: Tone,
    /// A button before the badge, such as + to stage a file: its glyph and
    /// id. It takes focus after the entry.
    pub action: Option<(char, String)>,
    /// A checkbox after the arrow, clicked on its own.
    pub check: Option<Check>,
    /// Shown in place of the label.
    pub swap: Option<Box<Swap>>,
    /// An arrow that unfolds what's under a row that isn't a folder, such
    /// as a changed file's hunks: whether it's open. Clicked on its own.
    pub fold: Option<bool>,
    /// Lines added and removed, after the label in green and red.
    pub counts: Option<(usize, usize)>,
    /// Buttons as words at the row's end, such as Stage, with their ids:
    /// the first always shows, the rest while the row is hovered. They take
    /// focus after the entry and its action.
    pub links: Vec<(String, String)>,
    /// The label and detail in the code font, as a hunk's line.
    pub code: bool,
    /// Drawn as a section's heading, its label in capitals, with its badge
    /// as a count and its action on hover.
    pub heading: bool,
    /// The label struck through, as a deleted file's.
    pub strike: bool,
    /// A small note in the code font after the label, as a mode change's
    /// `644 → 755`.
    pub chip: Option<String>,
    /// A short note after the label in its tone, as a conflicted file's
    /// `2 left`.
    pub note: Option<(String, Tone)>,
    /// What a right-click on it offers, each a button whose click goes
    /// where the row's does.
    pub menu: Vec<Button>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub title: String,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickItem {
    pub id: String,
    pub label: String,
}

/// The chips that can be clicked, as targets.
fn chip_targets(chips: &[Chip]) -> Vec<Target> {
    chips
        .iter()
        .filter(|c| !c.id.is_empty())
        .map(|c| Target::Button(c.id.clone()))
        .collect()
}

/// Something in a panel that can take focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Button(String),
    Input(String),
}

impl Panel {
    /// Every id a click can arrive with, to change: as merlin keeps its
    /// own (`agent:`) out of a plugin's pieces.
    pub fn ids_mut(&mut self, mut f: impl FnMut(&mut String)) {
        for node in &mut self.nodes {
            match node {
                Node::Button(b) | Node::Wide { button: b, .. } => f(&mut b.id),
                Node::Buttons(bs) | Node::Tools(bs) | Node::Banner { buttons: bs, .. } => {
                    bs.iter_mut().for_each(|b| f(&mut b.id))
                }
                Node::Item { id, .. } | Node::Input { id, .. } => f(id),
                Node::Entry(e) => {
                    f(&mut e.id);
                    e.menu.iter_mut().for_each(|b| f(&mut b.id));
                    if let Some((_, id)) = &mut e.action {
                        f(id);
                    }
                    e.links.iter_mut().for_each(|(_, id)| f(id));
                }
                Node::Chips { chips, .. } | Node::Bar { chips, .. } => {
                    chips.iter_mut().for_each(|c| f(&mut c.id))
                }
                Node::Message(m) => m.chips.iter_mut().for_each(|c| f(&mut c.id)),
                Node::ComposeBar(c) => c.chips.iter_mut().for_each(|c| f(&mut c.id)),
                Node::Edit(e) => e.buttons.iter_mut().for_each(|b| f(&mut b.id)),
                Node::Empty { buttons, .. } => buttons.iter_mut().for_each(|b| f(&mut b.id)),
                Node::Drawing(d) => d.items.iter_mut().for_each(|i| f(&mut i.id)),
                Node::Menu { items, .. } => items.iter_mut().for_each(|(id, ..)| f(id)),
                _ => {}
            }
        }
    }

    /// Focusable elements, in the order Tab visits them.
    pub fn targets(&self) -> Vec<Target> {
        let button = |b: &Button| Target::Button(b.id.clone());
        self.nodes
            .iter()
            .flat_map(|node| match node {
                Node::Button(b) => vec![button(b)],
                Node::Buttons(bs) | Node::Tools(bs) | Node::Banner { buttons: bs, .. } => {
                    bs.iter().map(button).collect()
                }
                Node::Wide { button: b, .. } => vec![button(b)],
                Node::Item { id, .. } => vec![Target::Button(id.clone())],
                Node::Entry(e) => std::iter::once(Target::Button(e.id.clone()))
                    .chain(e.action.as_ref().map(|(_, id)| Target::Button(id.clone())))
                    .chain(e.links.iter().map(|(_, id)| Target::Button(id.clone())))
                    .collect(),
                Node::Input { id, .. } => vec![Target::Input(id.clone())],
                Node::Chips { chips, .. } => chip_targets(chips),
                Node::Message(m) => chip_targets(&m.chips),
                Node::Edit(e) => e.buttons.iter().map(button).collect(),
                Node::Empty { buttons, .. } => buttons.iter().map(button).collect(),
                Node::Step(s) => s.links.iter().chain(&s.buttons).map(button).collect(),
                Node::Ask(a) => a.buttons.iter().chain(&a.quiet).map(button).collect(),
                Node::Changes(c) => std::iter::once(button(&c.revert_all))
                    .chain(
                        c.files
                            .iter()
                            .flat_map(|f| [button(&f.review), button(&f.revert)]),
                    )
                    .collect(),
                Node::Command { copy, .. } => vec![button(copy)],
                Node::Bar { chips, .. } => chip_targets(chips),
                Node::ComposeBar(c) => c.ids().into_iter().map(Target::Button).collect(),
                Node::Fold { id, .. } => vec![Target::Button(id.clone())],
                Node::YouSaid { turn, .. } => ["turn-edit", "turn-copy", "rewind"]
                    .iter()
                    .map(|w| Target::Button(format!("agent:{w}:{turn}")))
                    .collect(),
                Node::PlanDock { .. } => vec![Target::Button("agent:plan".into())],
                Node::Queued { index, .. } => ["queue-edit", "queue-drop", "queue-now"]
                    .iter()
                    .map(|w| Target::Button(format!("agent:{w}:{index}")))
                    .collect(),
                Node::ChangesBar(c) => c.ids().into_iter().map(Target::Button).collect(),
                Node::TurnFoot { copy, .. } => vec![button(copy)],
                Node::Menu { items, .. } => items
                    .iter()
                    .map(|(id, ..)| Target::Button(id.clone()))
                    .collect(),
                _ => Vec::new(),
            })
            .collect()
    }
}
