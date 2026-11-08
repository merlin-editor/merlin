//! The window's data as plain text, a part a line: what tests read, and
//! what `--exec` prints. It says what's on screen and where it is, not
//! how it's drawn.

use std::fmt::Write;

use super::window::SideView;
use super::{Editor, PopupKind, TextView};
use crate::panel::Node;

impl Editor {
    /// What a window shows now, as text.
    pub fn dump(&self) -> String {
        let view = self.window_view();
        let texts = self.text_views();
        let mut out = String::new();
        let o = &mut out;
        let branch = view
            .branch
            .as_deref()
            .map(|b| format!(" ⎇ {b}"))
            .unwrap_or_default();
        let _ = writeln!(o, "{}{branch}", view.project);
        let rail: Vec<String> = view
            .rail
            .iter()
            .map(|r| {
                let badge = if r.badge > 0 {
                    format!(" ({})", r.badge)
                } else {
                    String::new()
                };
                let on = if r.active { "*" } else { "" };
                format!("{}{on}{badge}", r.label)
            })
            .collect();
        let _ = writeln!(o, "rail: {}", rail.join(" | "));
        if let Some(side) = &view.left {
            dump_side(o, "left", side);
        }
        for (g, group) in view.groups.iter().enumerate() {
            let focused = if group.focused { ", focused" } else { "" };
            let _ = writeln!(o, "── group {}{focused} ──", g + 1);
            let tabs: Vec<String> = group
                .tabs
                .iter()
                .map(|t| {
                    let mark = if t.dirty { " ●" } else { "" };
                    if t.active {
                        format!("[{}{mark}]", t.name)
                    } else {
                        format!("{}{mark}", t.name)
                    }
                })
                .collect();
            let _ = writeln!(o, "tabs: {}", tabs.join(" "));
            if !group.crumbs.is_empty() {
                let _ = writeln!(o, "crumbs: {}", group.crumbs.join(" › "));
            }
            if group.focused && view.page.is_some() {
                let _ = writeln!(o, "page: {}", view.page.as_deref().unwrap_or_default());
            }
            for text in texts.iter().filter(|t| t.group == g) {
                dump_text(o, text);
            }
            if group.focused {
                if let Some(find) = &view.find {
                    let at = match (find.current, find.total) {
                        (Some(c), total) => format!("{} of {total}", c + 1),
                        (None, 0) => "No results".into(),
                        (None, 1) => "1 match".into(),
                        (None, total) => format!("{total} matches"),
                    };
                    let _ = writeln!(o, "find: [{}] {at}", find.query);
                    if let Some(with) = &find.replace {
                        let _ = writeln!(o, "replace: [{with}]");
                    }
                }
                if let Some(popup) = &view.popup {
                    match &popup.kind {
                        PopupKind::Rename(r) => {
                            let _ = writeln!(o, "rename: [{}] {}", r.text, r.note);
                        }
                        PopupKind::Exception { title, message } => {
                            let _ = writeln!(o, "{title}: {message}");
                        }
                        PopupKind::Breakpoint(b) => {
                            let _ = writeln!(o, "breakpoint {}: [{}]", b.field, b.text);
                        }
                        PopupKind::Hover {
                            problems, lines, ..
                        } => {
                            let _ = writeln!(o, "hover:");
                            for p in problems {
                                let _ = writeln!(
                                    o,
                                    "  {:?} {} {}: {}",
                                    p.severity, p.source, p.code, p.message
                                );
                            }
                            for line in lines {
                                let _ = writeln!(o, "  {line}");
                            }
                        }
                        PopupKind::Actions { actions, selected } => {
                            let _ = writeln!(o, "code actions:");
                            for (i, a) in actions.iter().enumerate() {
                                let mark = if i == *selected { "›" } else { " " };
                                let _ = writeln!(o, " {mark} {:?} {}", a.kind, a.title);
                            }
                        }
                        PopupKind::Completion { items, selected } => {
                            let _ = writeln!(o, "completions:");
                            for (i, (label, detail)) in items.iter().enumerate() {
                                let mark = if i == *selected { "›" } else { " " };
                                let _ = writeln!(o, " {mark} {label}  {detail}");
                            }
                        }
                        PopupKind::Definition {
                            file, line, lines, ..
                        } => {
                            let _ = writeln!(o, "definition: {file} · line {line}");
                            for l in lines {
                                let _ = writeln!(o, "  {l}");
                            }
                        }
                    }
                }
            }
        }
        if let Some(side) = &view.right {
            dump_side(o, "right", side);
        }
        if let Some(side) = &view.bottom {
            dump_side(o, "bottom", side);
        }
        let left: Vec<&str> = view
            .status
            .iter()
            .filter(|s| !s.right)
            .map(|s| s.text.as_str())
            .collect();
        // The language's { } as its state, and a spinner as ⟳.
        let right: Vec<String> = view
            .status
            .iter()
            .filter(|s| s.right)
            .map(|s| match s.mark {
                Some(super::StatusMark::Braces(state)) => {
                    format!("{{{}}} {}", format!("{state:?}").to_lowercase(), s.text)
                }
                Some(super::StatusMark::Spinner) => format!("⟳ {}", s.text),
                None => s.text.clone(),
            })
            .collect();
        let problem = view
            .problem
            .as_ref()
            .map(|(p, _)| format!("  {p}  "))
            .unwrap_or_default();
        let _ = writeln!(
            o,
            "status: {}{problem} | {}",
            left.join("  "),
            right.join("   ")
        );
        if let Some(card) = &view.language_card {
            let _ = writeln!(o, "language card: {} · {}", card.title, card.from);
            for row in &card.rows {
                let state = row
                    .state
                    .map_or("·".to_string(), |s| format!("{s:?}").to_lowercase());
                let more: String = [
                    row.progress.map(|p| format!(" {p}%")),
                    row.button.as_ref().map(|(b, _)| format!(" [{b}]")),
                    row.link.as_ref().map(|l| format!(" <{l}>")),
                ]
                .into_iter()
                .flatten()
                .collect();
                let _ = writeln!(o, "  {state} {}: {}{more}", row.name, row.detail);
                if let Some(hint) = &row.hint {
                    let _ = writeln!(o, "    {hint} [Copy]");
                }
            }
        }
        if let Some(toast) = &view.toast {
            let kind = if toast.error { "error" } else { "notice" };
            let buttons: String = toast.buttons.iter().map(|b| format!(" [{b}]")).collect();
            let _ = writeln!(o, "{kind}: {}{buttons}", toast.text);
        }
        if let Some(menu) = &view.menu {
            let _ = writeln!(
                o,
                "menu: {}",
                menu.titles.get(menu.open).cloned().unwrap_or_default()
            );
            dump_items(o, &menu.items, menu.selected);
        }
        if let Some(menu) = &view.context {
            let _ = writeln!(o, "context menu:");
            dump_items(o, &menu.items, menu.selected);
        }
        if let Some(palette) = &view.palette {
            let title = palette.title.as_deref().unwrap_or("palette");
            let _ = writeln!(o, "{title}: > {}", palette.query);
            for (i, (label, key)) in palette.items.iter().enumerate() {
                let mark = if i == palette.selected { "›" } else { " " };
                let key = key.as_deref().map(|k| format!("  {k}")).unwrap_or_default();
                let _ = writeln!(o, " {mark} {label}{key}");
            }
        }
        out
    }
}

/// A group's text: each row with its number, a sign, the text in view and
/// a mark for a folded block, and `^` under each cursor.
fn dump_text(o: &mut String, view: &TextView) {
    for (y, row) in rows_of(view) {
        let _ = writeln!(o, "{row}");
        let carets: Vec<usize> = view
            .carets
            .iter()
            .filter_map(|c| c.cell)
            .filter(|(_, cy)| *cy as i32 == y)
            .map(|(cx, _)| cx as usize)
            .collect();
        if !carets.is_empty() && view.focused {
            let mut marks = " ".repeat(view.gutter);
            let last = carets.iter().max().copied().unwrap_or(0);
            for x in 0..=last {
                marks.push(if carets.contains(&x) { '^' } else { ' ' });
            }
            let _ = writeln!(o, "{marks}");
        }
        if let Some(peek) = view.peek.as_ref().filter(|p| p.row == y + 1) {
            dump_peek(o, peek);
        }
        if let Some(change) = view.change.as_ref().filter(|c| c.row == y + 1) {
            let stage = if change.staged { "Unstage" } else { "Stage" };
            let _ = writeln!(
                o,
                "┌ Change {} of {} · since the last commit  [{stage}] [Revert] ↑ ↓ ✕",
                change.index, change.count
            );
            for (sign, text) in &change.lines {
                let _ = writeln!(o, "│ {sign} {text}");
            }
        }
    }
}

/// The peek view: its header, the preview's lines, then the list.
fn dump_peek(o: &mut String, peek: &super::PeekView) {
    let _ = writeln!(o, "┌ {} · {}", peek.name, peek.summary);
    for line in &peek.preview {
        let _ = writeln!(o, "│ {:>3} {}", line.number, line.text);
    }
    for (i, item) in peek.items.iter().enumerate() {
        let mark = if peek.selected == Some(i) { "›" } else { " " };
        let _ = match item {
            super::PeekItem::File {
                name,
                folder,
                count,
                open,
                ..
            } => {
                let arrow = if *open { "▾" } else { "▸" };
                writeln!(o, "│{mark}{arrow} {name} {folder} {count}")
            }
            super::PeekItem::Place { line, text, .. } => {
                writeln!(o, "│{mark}   {line} {text}")
            }
        };
    }
    let _ = writeln!(o, "└");
}

/// Each row of a view on screen, by its row: the gutter's sign and number,
/// a fold arrow, the text in view with tabs as spaces, and `⋯` after a
/// folded block.
fn rows_of(view: &TextView) -> Vec<(i32, String)> {
    let shown = |row: i32| (0..view.area.h as i32).contains(&row);
    let mut out = Vec::new();
    for line in &view.lines {
        for (r, row) in line.rows.iter().enumerate() {
            let y = line.row + r as i32;
            if !shown(y) {
                continue;
            }
            let number = if r == 0 {
                line.number.clone()
            } else {
                " ".repeat(line.number.len())
            };
            let sign = line.sign.map_or(String::new(), |(c, _)| c.to_string());
            let end = line
                .rows
                .get(r + 1)
                .map_or(line.chars.len(), |next| next.start);
            let mut text = " ".repeat(row.indent);
            let mut d = row.col;
            for &c in line.chars[row.start..end]
                .iter()
                .take_while(|c| **c != '\n' && **c != '\r')
            {
                let w = crate::doc::advance(c, d);
                if c == '\t' {
                    text.extend(std::iter::repeat_n(' ', w));
                } else {
                    text.push(c);
                }
                d += w;
            }
            if !view.wrapping {
                text = text.chars().skip(view.hscroll).collect();
            }
            text = text.chars().take(view.columns).collect();
            let arrow = match line.fold_arrow {
                Some(true) => "▸",
                Some(false) => "▾",
                None => "",
            };
            let folded = if line.folded && r + 1 == line.rows.len() {
                " ⋯"
            } else {
                ""
            };
            out.push((y, format!("{sign}{number}{arrow}{text}{folded}")));
        }
    }
    out
}

impl Editor {
    /// The focused group's text, a row a line, as [`Editor::dump`] shows it.
    pub fn text_rows(&self) -> Vec<String> {
        self.text_views()
            .pop()
            .map(|v| rows_of(&v).into_iter().map(|(_, r)| r).collect())
            .unwrap_or_default()
    }

    /// Group `g`'s text, a row a line.
    pub fn group_rows(&self, g: usize) -> Vec<String> {
        self.text_views()
            .into_iter()
            .find(|v| v.group == g)
            .map(|v| rows_of(&v).into_iter().map(|(_, r)| r).collect())
            .unwrap_or_default()
    }
}

fn dump_side(o: &mut String, which: &str, side: &SideView) {
    let tabs: Vec<String> = side
        .panels
        .iter()
        .map(|tab| {
            if tab.on {
                format!("[{}]", tab.title)
            } else {
                tab.title.clone()
            }
        })
        .collect();
    // The side bar shows one view; the activity bar picks it.
    let tabs = if tabs.len() > 1 && which != "left" {
        format!("  {}", tabs.join(" "))
    } else {
        String::new()
    };
    let _ = writeln!(o, "── {which}: {}{tabs} ──", side.title);
    for (i, node) in side.nodes.iter().enumerate() {
        let selected =
            side.targets.get(i).copied().flatten() == Some(side.selected) && side.focused;
        let mark = if selected { "›" } else { " " };
        let line = match node {
            Node::Heading(t) => t.to_uppercase(),
            Node::Text(t) | Node::Styled(t, _) => t.clone(),
            Node::Markdown(t) => t.clone(),
            Node::Log(l) => format!("{} {:<6} {}", l.time, l.level, l.text),
            Node::Button(b) => format!("[{}]", b.label),
            Node::Buttons(bs) | Node::Tools(bs) => bs
                .iter()
                .map(|b| format!("[{}]", b.label))
                .collect::<Vec<_>>()
                .join(" "),
            Node::Item { label, .. } => label.clone(),
            Node::Entry(e) if e.heading => format!("{}  {}", e.label.to_uppercase(), e.badge),
            Node::Entry(e) => {
                let twisty = match e.fold {
                    Some(true) => "▾ ".to_string(),
                    Some(false) => "▸ ".to_string(),
                    None => e.twisty.map_or(String::new(), |t| format!("{t} ")),
                };
                let counts = e
                    .counts
                    .map_or(String::new(), |(a, r)| format!("  +{a} −{r}"));
                let links: String = e.links.iter().map(|(l, _)| format!("  [{l}]")).collect();
                let detail = if e.detail.is_empty() {
                    String::new()
                } else {
                    format!("  {}", e.detail)
                };
                let badge = if e.badge.is_empty() {
                    String::new()
                } else {
                    format!("  {}", e.badge)
                };
                let check = match e.check {
                    Some(crate::panel::Check::On) => "[x] ",
                    Some(crate::panel::Check::Off) => "[ ] ",
                    Some(crate::panel::Check::Some) => "[-] ",
                    None => "",
                };
                let label = match &e.swap {
                    Some(s) => format!("{} {}~{}~{}{}", s.line, s.before, s.old, s.new, s.after),
                    None if e.strike => format!("~{}~", e.label),
                    None => e.label.clone(),
                };
                let label = match &e.chip {
                    Some(chip) => format!("{label} [{chip}]"),
                    None => label,
                };
                let label = match &e.note {
                    Some((note, _)) => format!("{label}  {note}"),
                    None => label,
                };
                format!(
                    "{}{twisty}{check}{label}{detail}{counts}{badge}{links}",
                    "  ".repeat(e.depth)
                )
            }
            Node::Header {
                title,
                detail,
                buttons,
            } => {
                let title: String = title.iter().map(|t| t.0.as_str()).collect();
                let buttons: Vec<String> = buttons
                    .iter()
                    .map(|(b, key)| format!("[{} {key}]", b.label))
                    .collect();
                format!("{title} · {detail}  {}", buttons.join(" "))
            }
            Node::Banner {
                title,
                detail,
                buttons,
                ..
            } => {
                let title: String = title.iter().map(|t| t.0.as_str()).collect();
                let buttons: Vec<String> =
                    buttons.iter().map(|b| format!("[{}]", b.label)).collect();
                format!("▌ {title}\n▌ {detail}\n▌ {}", buttons.join(" "))
            }
            Node::Wide { button, ready } => {
                let dim = if *ready { "" } else { " (waiting)" };
                format!("[{}]{dim}", button.label)
            }
            Node::Chips { chips, hint } => {
                let chips: Vec<String> = chips.iter().map(|c| format!("({})", c.label)).collect();
                format!("{}  {hint}", chips.join(" "))
                    .trim_end()
                    .to_string()
            }
            Node::Message(m) => {
                let who = if m.mine { "You" } else { m.author.as_str() };
                let chips: String = m.chips.iter().map(|c| format!(" ({})", c.label)).collect();
                let caret = if m.streaming { "▌" } else { "" };
                format!("{who}:{chips}\n{}{caret}", m.markdown)
            }
            Node::Edit(e) => {
                let buttons: Vec<String> =
                    e.buttons.iter().map(|b| format!("[{}]", b.label)).collect();
                format!(
                    "✎ {} +{} −{}  {}",
                    e.path,
                    e.added,
                    e.removed,
                    buttons.join(" ")
                )
            }
            Node::Progress {
                label,
                detail,
                done,
            } => {
                let at = done.map_or("…".to_string(), |d| format!("{}%", d / 10));
                format!("{label} {at} {detail}").trim_end().to_string()
            }
            Node::Empty {
                title,
                text,
                buttons,
            } => {
                let buttons: Vec<String> =
                    buttons.iter().map(|b| format!("[{}]", b.label)).collect();
                format!("{title}\n{text}\n{}", buttons.join(" "))
            }
            Node::Input {
                id,
                placeholder,
                secret,
                ..
            } => {
                let typed = side
                    .inputs
                    .iter()
                    .find(|(i, _)| i == id)
                    .map(|(_, t)| t.as_str());
                match typed {
                    Some(t) if *secret && !t.is_empty() => {
                        format!("[{}]", "•".repeat(t.chars().count()))
                    }
                    Some(t) if !t.is_empty() => format!("[{t}]"),
                    _ => format!("[{placeholder}]"),
                }
            }
            Node::Divider => "──".into(),
            Node::Footer => continue,
        };
        for (k, part) in line.lines().enumerate() {
            let mark = if k == 0 { mark } else { " " };
            let _ = writeln!(o, "{mark} {part}");
        }
    }
}

fn dump_items(o: &mut String, items: &[super::ContextItem], selected: Option<usize>) {
    for (i, item) in items.iter().enumerate() {
        if item.label.is_empty() {
            let _ = writeln!(o, "  ──");
            continue;
        }
        let mark = if selected == Some(i) { "›" } else { " " };
        let key = item
            .key
            .as_deref()
            .map(|k| format!("  {k}"))
            .unwrap_or_default();
        let _ = writeln!(o, " {mark} {}{key}", item.label);
    }
}
