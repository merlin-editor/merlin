//! Plugin panels: what plugins describe, and how the editor lays it out.

use merlin_syntax::Syntax;

use crate::screen::{Hue, Style, char_width, str_width};

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
                    .collect(),
                Node::Input { id, .. } => vec![Target::Input(id.clone())],
                _ => Vec::new(),
            })
            .collect()
    }
}

/// A laid-out panel row.
pub(crate) struct Row {
    pub segments: Vec<(String, Style, Syntax)>,
    /// Background for the whole row, for inputs.
    pub fill: Option<Style>,
    /// Column of the cursor, on a focused input's last row.
    pub cursor: Option<u16>,
    /// Clickable spans: start and end column, and the target's index in
    /// [`Panel::targets`].
    pub hits: Vec<(u16, u16, usize)>,
    /// Text colours at columns, for icons and guides.
    pub tints: Vec<(u16, Hue)>,
}

impl Row {
    pub fn text(text: &str, style: Style) -> Self {
        Self::from_segments(vec![(text.to_string(), style, Syntax::Plain)])
    }

    pub fn from_segments(segments: Vec<(String, Style, Syntax)>) -> Self {
        Self {
            segments,
            fill: None,
            cursor: None,
            hits: Vec::new(),
            tints: Vec::new(),
        }
    }

    pub fn blank() -> Self {
        Self::from_segments(Vec::new())
    }

    pub fn is_blank(&self) -> bool {
        self.segments.iter().all(|(t, _, _)| t.trim().is_empty())
    }
}

pub(crate) struct Layout {
    pub body: Vec<Row>,
    pub footer: Vec<Row>,
}

/// Most rows an input grows to before it scrolls.
const INPUT_ROWS: usize = 5;

/// Lays out `panel` at `width` columns. `focused` indexes
/// [`Panel::targets`], and `input_text` gives the text typed in an input.
pub(crate) fn layout(
    panel: &Panel,
    width: usize,
    focused: Option<usize>,
    input_text: impl Fn(&str) -> String,
    colour: crate::markdown::Colour,
) -> Layout {
    let mut layout = Layout {
        body: Vec::new(),
        footer: Vec::new(),
    };
    let mut in_footer = false;
    let mut target = 0;
    for node in &panel.nodes {
        let rows = if in_footer {
            &mut layout.footer
        } else {
            &mut layout.body
        };
        match node {
            Node::Footer => in_footer = true,
            Node::Heading(text) => rows.push(Row::text(text, Style::Heading)),
            Node::Text(text) => push_wrapped(rows, text, Style::Normal, width),
            Node::Styled(text, tone) => push_wrapped(rows, text, tone_style(*tone), width),
            Node::Divider => rows.push(Row::text(&"─".repeat(width), Style::Muted)),
            Node::Button(b) => rows.push(button_row(std::slice::from_ref(b), &mut target, focused)),
            Node::Buttons(bs) => rows.push(button_row(bs, &mut target, focused)),
            Node::Item { label, tone, .. } => {
                let style = if focused == Some(target) {
                    Style::Selected
                } else {
                    tone_style(*tone)
                };
                let mut row = Row::text(label, style);
                row.hits = vec![(0, width as u16, target)];
                if focused == Some(target) {
                    row.fill = Some(Style::Selected);
                }
                rows.push(row);
                target += 1;
            }
            Node::Entry(entry) => {
                rows.push(entry_row(entry, width, target, focused));
                target += 1 + entry.action.is_some() as usize;
            }
            Node::Markdown(text) => rows.extend(crate::markdown::layout(text, width, colour)),
            Node::Input {
                id,
                placeholder,
                secret,
            } => {
                let mut text = input_text(id);
                if *secret {
                    text = "•".repeat(text.chars().count());
                }
                push_input(rows, &text, placeholder, width, target, focused);
                target += 1;
            }
        }
    }
    layout
}

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Normal => Style::Normal,
        Tone::Muted => Style::Muted,
        Tone::Strong => Style::Heading,
        Tone::Warning => Style::Warning,
        Tone::Error => Style::Error,
        Tone::Success => Style::Success,
        Tone::Badge => Style::Badge,
    }
}

fn push_wrapped(rows: &mut Vec<Row>, text: &str, style: Style, width: usize) {
    for line in text.split('\n') {
        for part in wrap(line, width) {
            rows.push(Row::text(&part, style));
        }
    }
}

fn button_row(buttons: &[Button], target: &mut usize, focused: Option<usize>) -> Row {
    let mut segments = Vec::new();
    let mut hits = Vec::new();
    let mut col = 0;
    for (i, b) in buttons.iter().enumerate() {
        if i > 0 {
            segments.push((" ".to_string(), Style::Normal, Syntax::Plain));
            col += 1;
        }
        let style = if focused == Some(*target) {
            Style::ButtonFocused
        } else {
            Style::Button
        };
        let label = format!(" {} ", b.label);
        let w = str_width(&label) as u16;
        hits.push((col, col + w, *target));
        col += w;
        segments.push((label, style, Syntax::Plain));
        *target += 1;
    }
    Row {
        segments,
        fill: None,
        cursor: None,
        hits,
        tints: Vec::new(),
    }
}

/// An entry's row: indent guides, arrow, icon, label and detail, and the
/// badge at the right edge.
fn entry_row(entry: &Entry, width: usize, target: usize, focus: Option<usize>) -> Row {
    let focused = focus == Some(target);
    let pick = |style: Style| if focused { Style::Selected } else { style };
    let mut segments = Vec::new();
    let mut tints = Vec::new();
    let mut col = 0;
    for level in 0..entry.depth {
        segments.push(("│ ".to_string(), pick(Style::Normal), Syntax::Plain));
        tints.push(((level * 2) as u16, Hue::TreeGuide));
        col += 2;
    }
    if let Some(twisty) = entry.twisty {
        segments.push((format!("{twisty} "), pick(Style::Normal), Syntax::Plain));
        col += 2;
    }
    if let Some((icon, hue)) = entry.icon {
        segments.push((format!("{icon} "), pick(Style::Normal), Syntax::Plain));
        tints.push((col as u16, hue));
        col += 2;
    }
    let badge_w = str_width(&entry.badge);
    // The action takes two cells and a space before the badge.
    let action_w = if entry.action.is_some() { 3 } else { 0 };
    let room = width.saturating_sub(col + badge_w + (badge_w > 0) as usize + action_w);
    let label = fit(&entry.label, room);
    let used = str_width(&label);
    segments.push((label, pick(tone_style(entry.tone)), Syntax::Plain));
    if !entry.detail.is_empty() && used + 2 < room {
        let detail = fit(&entry.detail, room - used - 1);
        segments.push((format!(" {detail}"), pick(Style::Muted), Syntax::Plain));
    }
    let mut hits = vec![(0, width as u16, target)];
    if badge_w > 0 || action_w > 0 {
        let shown: usize = segments.iter().map(|(t, _, _)| str_width(t)).sum();
        let pad = width.saturating_sub(shown + badge_w + action_w);
        segments.push((" ".repeat(pad), pick(Style::Normal), Syntax::Plain));
        if let Some((glyph, _)) = &entry.action {
            let at = (shown + pad) as u16;
            let style = if focus == Some(target + 1) {
                Style::ButtonFocused
            } else {
                pick(Style::Muted)
            };
            segments.push((format!(" {glyph}"), style, Syntax::Plain));
            segments.push((" ".into(), pick(Style::Normal), Syntax::Plain));
            // The button's cells come first, so a click there hits it.
            hits.insert(0, (at, at + 2, target + 1));
        }
        segments.push((
            entry.badge.clone(),
            pick(tone_style(entry.badge_tone)),
            Syntax::Plain,
        ));
    }
    Row {
        segments,
        fill: focused.then_some(Style::Selected),
        cursor: None,
        hits,
        tints,
    }
}

/// `text` cut to `width` cells, ending in … if it was cut.
pub(crate) fn fit(text: &str, width: usize) -> String {
    if str_width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = char_width(c);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

fn push_input(
    rows: &mut Vec<Row>,
    text: &str,
    placeholder: &str,
    width: usize,
    target: usize,
    focused: Option<usize>,
) {
    let focused = focused == Some(target);
    let hits = vec![(0, width as u16, target)];
    let inner = width.saturating_sub(2).max(1);
    if text.is_empty() {
        rows.push(Row {
            segments: vec![
                ("› ".into(), Style::Input, Syntax::Plain),
                (
                    placeholder.to_string(),
                    Style::InputPlaceholder,
                    Syntax::Plain,
                ),
            ],
            fill: Some(Style::Input),
            cursor: focused.then_some(2),
            hits,
            tints: Vec::new(),
        });
        return;
    }
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        lines.extend(split_width(line, inner));
    }
    let skip = lines.len().saturating_sub(INPUT_ROWS);
    let shown = lines.len() - skip;
    for (i, line) in lines.into_iter().skip(skip).enumerate() {
        let prefix = if i == 0 && skip == 0 { "› " } else { "  " };
        let last = i + 1 == shown;
        rows.push(Row {
            cursor: (focused && last).then(|| (2 + str_width(&line)) as u16),
            segments: vec![
                (prefix.into(), Style::Input, Syntax::Plain),
                (line, Style::Input, Syntax::Plain),
            ],
            fill: Some(Style::Input),
            hits: hits.clone(),
            tints: Vec::new(),
        });
    }
}

/// Cuts `text` into pieces at most `width` cells wide. An empty line stays
/// one empty piece.
fn split_width(text: &str, width: usize) -> Vec<String> {
    let mut pieces = vec![String::new()];
    let mut used = 0;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > width && used > 0 {
            pieces.push(String::new());
            used = 0;
        }
        pieces.last_mut().unwrap().push(c);
        used += w;
    }
    pieces
}

/// Word-wraps one line. Words longer than `width` are split.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut len = 0;
    for word in text.split_whitespace() {
        let word_w = str_width(word);
        if len > 0 && len + 1 + word_w > width {
            lines.push(std::mem::take(&mut current));
            len = 0;
        }
        if len == 0 && word_w > width {
            let mut pieces = split_width(word, width);
            current = pieces.pop().unwrap_or_default();
            len = str_width(&current);
            lines.extend(pieces);
            continue;
        }
        if len > 0 {
            current.push(' ');
            len += 1;
        }
        len += word_w;
        current.push_str(word);
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}
