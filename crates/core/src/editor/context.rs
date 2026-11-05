//! What floats over the text in a window: the right-click menu, and hover
//! and completion as cards by the text they're about. Hover also comes
//! from resting the mouse on a word, as other editors do it.

use super::actions::ActionsFor;
use super::{Editor, Effect, Focus, Popup};
use crate::icons::{Icon, icon};
use crate::key::{Key, KeyCode};
use crate::lang::Severity;
use merlin_syntax::Syntax;

/// The right-click menu's commands, by id; `-` is a separator. Plugin
/// commands that aren't loaded are left out.
const ITEMS: &[(&str, &str)] = &[
    ("lsp.definition", "Go to Definition"),
    ("view.hover", "Show Hover"),
    ("-", ""),
    ("edit.select_all_matches", "Change All Occurrences"),
    ("edit.expand_selection", "Expand Selection"),
    ("-", ""),
    ("edit.cut", "Cut"),
    ("edit.copy", "Copy"),
    ("edit.paste", "Paste"),
    ("-", ""),
    ("edit.comment", "Toggle Line Comment"),
    ("-", ""),
    ("palette.open", "Command Palette…"),
];

pub(crate) struct ContextMenu {
    /// Command ids, `-` for separators.
    pub(crate) items: Vec<&'static str>,
    pub(crate) selected: Option<usize>,
}

/// The right-click menu, as the window draws it where it was opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextView {
    pub items: Vec<ContextItem>,
    pub selected: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextItem {
    /// Empty for a separator.
    pub label: String,
    pub key: Option<String>,
}

/// A card by the text: what the cursor's on, or what can complete it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupView {
    /// The cell, in the text's grid, it's anchored to: it goes below it,
    /// or above when there's no room.
    pub anchor: (u16, u16),
    pub kind: PopupKind,
}

/// Signature help over the call being typed: the cell of where the call
/// is, and the signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureView {
    pub anchor: (u16, u16),
    pub signature: crate::lang::Signature,
}

/// A button in the hover card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoverButton {
    /// Copy what the card's problems say.
    Copy,
    /// Show every problem in the Problems panel.
    Problems,
    /// Open the page that explains this problem's code.
    Link(usize),
    /// Go to the definition of the word the card is about.
    Definition,
    /// Make the likeliest fix.
    Fix,
    /// List every fix and action at the problem.
    MoreFixes,
}

/// A problem in a hover card: what it says, and what reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProblemView {
    pub severity: Severity,
    /// The severity's icon, as Problems shows it.
    pub icon: char,
    pub message: String,
    pub source: String,
    pub code: String,
    /// A page that explains the code, or empty.
    pub link: String,
    /// The other checkers that report it too.
    pub also: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopupKind {
    /// The rename box (F2), over the name.
    Rename(super::RenameView),
    /// The problems at the spot, worst first, then the language server's
    /// hover as Markdown, a line each. Either may be empty.
    Hover {
        problems: Vec<ProblemView>,
        lines: Vec<String>,
        /// Whether it can go to the definition of what it's about.
        definition: bool,
        /// The colours of each of `lines` that's code, by character, as the
        /// plugin that adds its language gives them; empty for the rest.
        colours: Vec<Vec<(usize, usize, Syntax)>>,
        /// The likeliest fix, by its title, and whether there are others.
        fix: Option<String>,
        more_fixes: bool,
    },
    /// Code actions grouped by kind, and the one Enter takes.
    Actions {
        actions: Vec<crate::lang::CodeAction>,
        selected: usize,
    },
    /// Labels and details, and the one Enter takes.
    Completion {
        items: Vec<(String, String)>,
        selected: usize,
    },
}

impl Editor {
    /// Opens the right-click menu for cell (`x`, `y`). In the text, the
    /// cursor goes there first, unless it's in the selection.
    pub(crate) fn open_context(&mut self, x: u16, y: u16) {
        let text = self.layout().text;
        if !text.contains(x, y) || self.doc().page.is_some() {
            return;
        }
        self.popup = None;
        self.probe = None;
        let pos = self.pos_at_screen(x, y);
        let inside = self
            .carets()
            .iter()
            .filter_map(|c| c.selection())
            .any(|(a, b)| (a..=b).contains(&pos));
        if !inside {
            self.set_cursor(pos);
        }
        let loaded = |id: &str| {
            let Some((plugin, command)) = id.split_once('.') else {
                return true;
            };
            if !["lsp"].contains(&plugin) {
                return true;
            }
            self.plugins
                .iter()
                .any(|p| p.name == plugin && p.commands.iter().any(|c| c.id == command))
        };
        let mut items: Vec<&'static str> = ITEMS
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| loaded(id))
            .collect();
        // No separator first, last or twice.
        items.dedup_by(|a, b| *a == "-" && *b == "-");
        while items.first() == Some(&"-") {
            items.remove(0);
        }
        while items.last() == Some(&"-") {
            items.pop();
        }
        self.context = Some(ContextMenu {
            items,
            selected: None,
        });
        self.focus = Focus::Menu;
    }

    pub(crate) fn close_context(&mut self) {
        self.context = None;
        if self.focus == Focus::Menu && self.menu.is_none() {
            self.focus = Focus::Editor;
        }
    }

    pub(crate) fn context_view(&self) -> Option<ContextView> {
        let menu = self.context.as_ref()?;
        let items = menu
            .items
            .iter()
            .map(|&id| match id {
                "-" => ContextItem {
                    label: String::new(),
                    key: None,
                },
                _ => ContextItem {
                    label: ITEMS
                        .iter()
                        .chain(super::tabs::TAB_ITEMS)
                        .chain(super::files::EXPLORER_ITEMS)
                        .find(|(i, _)| *i == id)
                        .map_or(id, |(_, l)| l)
                        .to_string(),
                    key: self.key_hint(id),
                },
            })
            .collect();
        Some(ContextView {
            items,
            selected: menu.selected,
        })
    }

    /// Runs the menu's `index`th item.
    pub(crate) fn context_item(&mut self, index: usize, fx: &mut Vec<Effect>) {
        let Some(id) = self
            .context
            .as_ref()
            .and_then(|m| m.items.get(index))
            .copied()
        else {
            return;
        };
        if id == "-" {
            return;
        }
        self.close_context();
        match id.split_once('.') {
            Some((plugin, command)) if self.plugins.iter().any(|p| p.name == plugin) => {
                fx.push(Effect::PluginCommand {
                    plugin: plugin.into(),
                    command: command.into(),
                })
            }
            _ => self.run(id, fx),
        }
    }

    /// Keys while the menu is open: arrows, Enter, Esc.
    pub(crate) fn context_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        let Some(menu) = &mut self.context else {
            return;
        };
        let n = menu.items.len();
        let step = |from: Option<usize>, down: bool, items: &[&str]| {
            let mut i = from.unwrap_or(if down { n - 1 } else { 0 });
            for _ in 0..n {
                i = if down { (i + 1) % n } else { (i + n - 1) % n };
                if items[i] != "-" {
                    break;
                }
            }
            Some(i)
        };
        match key.code {
            KeyCode::Esc => self.close_context(),
            KeyCode::Down => menu.selected = step(menu.selected, true, &menu.items),
            KeyCode::Up => menu.selected = step(menu.selected, false, &menu.items),
            KeyCode::Enter => {
                if let Some(i) = menu.selected {
                    self.context_item(i, fx);
                }
            }
            _ => {}
        }
    }

    /// The mouse rested on cell (`x`, `y`): asks the language server what's
    /// there, without moving the cursor.
    pub(crate) fn hover_at(&mut self, x: u16, y: u16, fx: &mut Vec<Effect>) {
        let text = self.layout().text;
        let hoverable = self.focus == Focus::Editor
            && self.popup.is_none()
            && self.context.is_none()
            && text.contains(x, y)
            && (x as usize) >= text.x as usize + self.gutter();
        if !hoverable {
            return;
        }
        let pos = self.pos_at_screen(x, y);
        let doc = self.doc();
        let (start, end) = (doc.word_start_at(pos), doc.word_end_at(pos));
        // Past a line's end there's no word to ask about.
        let word = (start < end && (start..end).contains(&pos)).then_some((start, end));
        let (problems, span) = self.problems_at(pos);
        if !problems.is_empty() {
            // The card shows the problems at once, and the language
            // server's part joins them when it comes.
            self.hover_anchor = Some(word.map_or(span.0, |w| w.0));
            self.popup = Some(Popup::Hover {
                problems,
                lines: Vec::new(),
                span: Some(join(span, word)),
                fixes: Vec::new(),
            });
        }
        if let Some(word) = word
            && self.has_lsp("hover")
        {
            self.probe = Some(word);
            fx.push(Effect::PluginCommand {
                plugin: "lsp".into(),
                command: "hover".into(),
            });
        }
        // And what would fix them, for the card's buttons.
        if self.popup.is_some() {
            self.ask_actions(ActionsFor::Hover, fx);
        }
    }

    /// Shows the hover card at the cursor, as resting the mouse there
    /// would: its problems at once, and the language server's hover when
    /// it comes.
    pub(crate) fn hover_at_cursor(&mut self, fx: &mut Vec<Effect>) {
        self.probe = None;
        self.hover_anchor = None;
        let (problems, _) = self.problems_at(self.doc().cursor);
        self.popup = (!problems.is_empty()).then(|| Popup::Hover {
            problems,
            lines: Vec::new(),
            span: None,
            fixes: Vec::new(),
        });
        if self.has_lsp("hover") {
            fx.push(Effect::PluginCommand {
                plugin: "lsp".into(),
                command: "hover".into(),
            });
        }
        if self.popup.is_some() {
            self.ask_actions(ActionsFor::Hover, fx);
        }
    }

    /// The language server's hover came: it joins the problems a card
    /// already shows, or brings up a card of its own with the problems at
    /// its spot.
    pub(crate) fn show_hover(&mut self, server: Vec<String>) {
        if let Some(Popup::Hover { lines, .. }) = &mut self.popup
            && lines.is_empty()
        {
            *lines = server;
            return;
        }
        // A hover the mouse asked for is about the word it's on.
        self.hover_anchor = self.probe.map(|(start, _)| start);
        let at = self.hover_anchor.unwrap_or(self.doc().cursor);
        let (problems, span) = self.problems_at(at);
        let span = match (problems.is_empty(), self.probe) {
            (true, probe) => probe,
            (false, probe) => Some(join(span, probe)),
        };
        self.popup = Some(Popup::Hover {
            problems,
            lines: server,
            span,
            fixes: Vec::new(),
        });
    }

    /// The problems at `pos`, worst first, and the range they cover
    /// together. A problem two checkers report shows once, the fuller of
    /// the two, saying who else reports it: the same first line, or the
    /// same code where they overlap, as rust-analyzer and cargo check word
    /// and place one error a little differently. A hint that goes with a problem, as rustc's
    /// suggestions do, is a `help:` note under it.
    fn problems_at(&self, pos: usize) -> (Vec<ProblemView>, (usize, usize)) {
        let mut found: Vec<&crate::lang::Mark> = self
            .doc()
            .marks
            .iter()
            .filter(|m| m.start <= pos && pos < m.end.max(m.start + 1))
            .collect();
        found.sort_by_key(|m| (m.severity, std::cmp::Reverse(m.message.len())));
        let span = found.iter().fold((pos, pos), |(a, b), m| {
            (a.min(m.start), b.max(m.end.max(m.start + 1)))
        });
        let mut problems: Vec<ProblemView> = Vec::new();
        // Where each problem is, to know a second checker's copy of it.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for m in found {
            let first = m.message.lines().next().unwrap_or_default();
            let copy = problems.iter().zip(&ranges).position(|(p, range)| {
                p.message.lines().next().unwrap_or_default() == first
                    || (!m.code.is_empty()
                        && p.code == m.code
                        && p.severity == m.severity
                        && range.0 < m.end.max(m.start + 1)
                        && m.start < range.1)
            });
            if let Some(i) = copy {
                let p = &mut problems[i];
                if !m.source.is_empty() && m.source != p.source && !p.also.contains(&m.source) {
                    p.also.push(m.source.clone());
                }
                if p.link.is_empty() {
                    p.link = m.link.clone();
                }
                continue;
            }
            let owner = problems.iter_mut().find(|p| {
                m.severity == Severity::Hint
                    && !m.code.is_empty()
                    && (&p.source, &p.code) == (&m.source, &m.code)
            });
            if let Some(owner) = owner {
                owner.message = format!("{}\nhelp: {}", owner.message, m.message.trim());
                continue;
            }
            let glyph = match m.severity {
                Severity::Error => Icon::Error,
                Severity::Warning => Icon::Warning,
                _ => Icon::Info,
            };
            problems.push(ProblemView {
                severity: m.severity,
                icon: icon(glyph, self.nerd),
                message: m.message.trim().to_string(),
                source: m.source.clone(),
                code: m.code.clone(),
                link: m.link.clone(),
                also: Vec::new(),
            });
            ranges.push((m.start, m.end.max(m.start + 1)));
        }
        (problems, span)
    }

    /// Asks the language server where the word at the cursor is defined.
    pub(crate) fn go_to_definition(&mut self, fx: &mut Vec<Effect>) {
        if self.has_lsp("definition") {
            fx.push(Effect::PluginCommand {
                plugin: "lsp".into(),
                command: "definition".into(),
            });
        }
    }

    /// The colours of a hover's code blocks, a line each: each block as the
    /// language its fence names, or else as the file's, coloured by the
    /// plugin that adds that language. Asked for the first time they're
    /// wanted, so they come a frame later.
    fn hover_colours(&self, lines: &[String]) -> Vec<Vec<(usize, usize, Syntax)>> {
        let mut out = vec![Vec::new(); lines.len()];
        let file = self
            .doc()
            .buffer
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut i = 0;
        while i < lines.len() {
            let Some(fence) = lines[i].trim().strip_prefix("```") else {
                i += 1;
                continue;
            };
            let language = if fence.trim().is_empty() {
                file.clone()
            } else {
                fence.trim().to_string()
            };
            let first = i + 1;
            let end = (first..lines.len())
                .find(|&k| lines[k].trim().starts_with("```"))
                .unwrap_or(lines.len());
            let text = lines[first..end].join("\n");
            let spans = self.colours.get(&language, &text).unwrap_or_default();
            // Split the block's spans into its lines.
            let mut start = 0;
            for (k, line) in lines[first..end].iter().enumerate() {
                let len = line.chars().count();
                out[first + k] = spans
                    .iter()
                    .filter(|&&(a, b, _)| a < start + len && b > start)
                    .map(|&(a, b, kind)| (a.max(start) - start, b.min(start + len) - start, kind))
                    .collect();
                start += len + 1;
            }
            i = end + 1;
        }
        out
    }

    pub(crate) fn has_lsp(&self, command: &str) -> bool {
        self.plugins
            .iter()
            .any(|p| p.name == "lsp" && p.commands.iter().any(|c| c.id == command))
    }

    /// The mouse moved: a hover it brought up goes once it leaves the word
    /// and the problems the card is about.
    pub(crate) fn pointer_moved(&mut self, x: u16, y: u16) {
        let text = self.layout().text;
        let pos = text.contains(x, y).then(|| self.pos_at_screen(x, y));
        let outside = |range: Option<(usize, usize)>| {
            range.is_some_and(|(start, end)| pos.is_none_or(|p| !(start..end).contains(&p)))
        };
        if outside(self.probe) {
            self.probe = None;
        }
        let span = match &self.popup {
            Some(Popup::Hover { span, .. }) if self.hover_anchor.is_some() => *span,
            _ => None,
        };
        if outside(span) {
            self.popup = None;
            self.hover_anchor = None;
        }
    }

    /// A button in the hover card: copy its problems, or show them all in
    /// Problems.
    pub(crate) fn hover_button(&mut self, button: HoverButton, fx: &mut Vec<Effect>) {
        let Some(Popup::Hover { problems, .. }) = &self.popup else {
            return;
        };
        match button {
            HoverButton::Copy => {
                let text = problems
                    .iter()
                    .map(|p| p.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                fx.push(Effect::Copy(text));
                self.notify("Copied the problem".into());
            }
            HoverButton::Problems => {
                self.popup = None;
                self.hover_anchor = None;
                self.show_bottom(super::views::PROBLEMS);
            }
            HoverButton::Link(i) => {
                if let Some(p) = problems.get(i).filter(|p| !p.link.is_empty()) {
                    fx.push(Effect::OpenUrl(p.link.clone()));
                }
            }
            HoverButton::Fix => {
                if let Some(Popup::Hover { fixes, .. }) = &self.popup
                    && let Some(fix) = Self::hover_fix(fixes)
                {
                    let id = fix.id.clone();
                    self.choose_action(id, fx);
                }
            }
            HoverButton::MoreFixes => self.more_fixes(),
            HoverButton::Definition => {
                // The mouse has left the word for the card, so the cursor
                // goes to it first.
                if let Some(at) = self.hover_anchor.take() {
                    self.set_cursor(at);
                }
                self.popup = None;
                self.probe = None;
                self.go_to_definition(fx);
            }
        }
    }

    /// Where the cursor counts as being for plugins: the hovered word
    /// while a hover is asked for, or the problem the mouse's hover card
    /// is about, else the cursor.
    pub(crate) fn plugin_cursor(&self) -> usize {
        let hovered = match self.popup {
            Some(Popup::Hover { .. }) => self.hover_anchor,
            _ => None,
        };
        self.probe
            .map(|(start, _)| start)
            .or(hovered)
            .unwrap_or(self.doc().cursor)
    }

    pub(crate) fn popup_view(&self) -> Option<PopupView> {
        let text = self.layout().text;
        let anchor_at = |at: usize| {
            let (x, y) = self.cell_of(at)?;
            Some((x.saturating_sub(text.x), y.saturating_sub(text.y)))
        };
        // The rename box takes the place of any other.
        if let (Some(view), Some(at)) = (self.rename_view(), self.rename_anchor()) {
            return Some(PopupView {
                anchor: anchor_at(at)?,
                kind: PopupKind::Rename(view),
            });
        }
        let popup = self.popup.as_ref()?;
        let at = self.hover_anchor.unwrap_or(self.doc().cursor);
        let anchor = anchor_at(at)?;
        let kind = match popup {
            Popup::Hover {
                problems,
                lines,
                fixes,
                ..
            } => {
                let fix = Self::hover_fix(fixes);
                PopupKind::Hover {
                    problems: problems.clone(),
                    lines: lines.clone(),
                    definition: !lines.is_empty() && self.has_lsp("definition"),
                    colours: self.hover_colours(lines),
                    fix: fix.map(|f| f.title.clone()),
                    more_fixes: fixes.len() > fix.is_some() as usize,
                }
            }
            Popup::Actions { actions, selected } => PopupKind::Actions {
                actions: actions.clone(),
                selected: *selected,
            },
            Popup::Completion { selected, .. } => PopupKind::Completion {
                items: self
                    .completions()
                    .iter()
                    .map(|c| (c.label.clone(), c.detail.clone()))
                    .collect(),
                selected: *selected,
            },
        };
        Some(PopupView { anchor, kind })
    }

    pub(crate) fn signature_view(&self) -> Option<SignatureView> {
        let (_, at, signature) = self.signature.as_ref()?;
        let text = self.layout().text;
        let (x, y) = self.cell_of(*at)?;
        Some(SignatureView {
            anchor: (x.saturating_sub(text.x), y.saturating_sub(text.y)),
            signature: signature.clone(),
        })
    }
}

/// The range covering `a` and `b`.
fn join(a: (usize, usize), b: Option<(usize, usize)>) -> (usize, usize) {
    b.map_or(a, |b| (a.0.min(b.0), a.1.max(b.1)))
}
