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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub id: String,
    pub label: String,
}

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
    /// A row that can be focused and clicked like a button, drawn as text:
    /// a file in a tree, a problem in a list.
    Item {
        id: String,
        label: String,
        tone: Tone,
    },
    /// A row in a tree or list of the editor's own views: a file, a search
    /// hit, a problem.
    Entry(Entry),
    /// Markdown: headings, lists, quotes, code blocks.
    Markdown(String),
    /// A text field. A secret one shows dots instead of the text.
    Input {
        id: String,
        placeholder: String,
        secret: bool,
    },
    Divider,
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

/// Something in a panel that can take focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Button(String),
    Input(String),
}

impl Panel {
    /// Focusable elements, in the order Tab visits them.
    pub fn targets(&self) -> Vec<Target> {
        let button = |b: &Button| Target::Button(b.id.clone());
        self.nodes
            .iter()
            .flat_map(|node| match node {
                Node::Button(b) => vec![button(b)],
                Node::Buttons(bs) => bs.iter().map(button).collect(),
                Node::Item { id, .. } => vec![Target::Button(id.clone())],
                Node::Entry(e) => std::iter::once(Target::Button(e.id.clone()))
                    .chain(e.action.as_ref().map(|(_, id)| Target::Button(id.clone())))
                    .chain(e.links.iter().map(|(_, id)| Target::Button(id.clone())))
                    .collect(),
                Node::Input { id, .. } => vec![Target::Input(id.clone())],
                _ => Vec::new(),
            })
            .collect()
    }
}
