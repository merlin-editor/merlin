//! The window's frame around the text, as VS Code has it: the activity bar,
//! the title bar's command center and layout buttons, status bar items and
//! notices. Each part says where its clickable pieces are, so drawing and
//! the mouse agree.

use super::layout::Rect;
use super::views::{CHANGES, EXPLORER, OUTLINE, SEARCH};
use super::{Editor, Effect, Focus, Side};
use crate::icons::{Icon, icon};
use crate::lang::Severity;
use crate::screen::{Screen, Style, Surface, str_width};

/// What clicking part of the frame does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Run(&'static str),
    /// Shows a built-in view in the side bar, or hides the side bar if it's
    /// showing it already.
    View(&'static str),
    /// The same for a plugin's panel in the secondary side bar.
    Plugin(String),
}

/// An activity bar icon: its row, glyph, whether its view shows, a count,
/// and what clicking does.
pub(crate) struct ActivityItem {
    pub y: u16,
    pub glyph: char,
    pub active: bool,
    pub badge: usize,
    pub action: Action,
}

/// A status bar item: its column, text, style, and what clicking does.
pub(crate) struct StatusItem {
    pub x: u16,
    pub text: String,
    pub style: Style,
    pub action: Option<Action>,
}

/// Where a notice is: its box and its buttons, each a column, width and
/// command.
pub(crate) struct Toast {
    pub area: Rect,
    pub buttons: Vec<(u16, u16, String)>,
}

impl Editor {
    pub(crate) fn activity_items(&self) -> Vec<ActivityItem> {
        let Some(area) = self.layout().activity else {
            return Vec::new();
        };
        let left = self
            .bar(Side::Left)
            .visible
            .then(|| self.active_panel_name(Side::Left));
        let right = self
            .bar(Side::Right)
            .visible
            .then(|| self.active_panel_name(Side::Right));
        let mut items = Vec::new();
        let mut y = area.y + 1;
        for (view, glyph) in [
            (EXPLORER, Icon::Explorer),
            (SEARCH, Icon::Search),
            (CHANGES, Icon::SourceControl),
            (OUTLINE, Icon::Outline),
        ] {
            items.push(ActivityItem {
                y,
                glyph: icon(glyph, self.nerd),
                active: left.as_ref().is_some_and(|a| a.as_deref() == Some(view)),
                badge: if view == CHANGES {
                    self.git.files.len()
                } else {
                    0
                },
                action: Action::View(view),
            });
            y += 2;
        }
        for name in self.panel_names(Side::Right) {
            items.push(ActivityItem {
                y,
                glyph: icon(Icon::Plugin, self.nerd),
                active: right
                    .as_ref()
                    .is_some_and(|a| a.as_deref() == Some(name.as_str())),
                badge: 0,
                action: Action::Plugin(name),
            });
            y += 2;
        }
        items.retain(|i| i.y < area.bottom());
        items
    }

    pub(crate) fn render_activity(&self, s: &mut Screen) {
        let Some(area) = self.layout().activity else {
            return;
        };
        s.shade(area.x, area.y, area.w, area.h, Surface::Bar);
        for item in self.activity_items() {
            let hovered = self
                .hover
                .is_some_and(|(x, y)| y == item.y && area.contains(x, y));
            if item.active {
                s.put(area.x, item.y, '▎', Style::ActivityMarker);
            }
            let style = if item.active || hovered {
                Style::ActivityActive
            } else {
                Style::Activity
            };
            s.put(area.x + 1, item.y, item.glyph, style);
            if item.badge > 0 {
                let digit = if item.badge < 10 {
                    char::from_digit(item.badge as u32, 10).unwrap_or('+')
                } else {
                    '+'
                };
                s.put(area.x + 3, item.y, digit, Style::ActivityBadge);
            }
        }
    }

    /// What a click on the activity bar at row `y` does.
    pub(crate) fn activity_click(&mut self, y: u16, fx: &mut Vec<Effect>) {
        let Some(item) = self.activity_items().into_iter().find(|i| i.y == y) else {
            return;
        };
        self.act(item.action, fx);
    }

    pub(crate) fn act(&mut self, action: Action, fx: &mut Vec<Effect>) {
        match action {
            Action::Run(command) => self.run(command, None, fx),
            Action::View(view) => {
                let shown = self.bar(Side::Left).visible
                    && self.active_panel_name(Side::Left).as_deref() == Some(view);
                if shown {
                    self.toggle_side_bar(fx);
                } else {
                    self.show_view(view, fx);
                }
            }
            Action::Plugin(name) => {
                let shown = self.bar(Side::Right).visible
                    && self.active_panel_name(Side::Right).as_deref() == Some(name.as_str());
                if shown {
                    self.bar_mut(Side::Right).visible = false;
                    if self.focus == Focus::Panel {
                        self.focus = Focus::Editor;
                    }
                } else {
                    self.bar_mut(Side::Right).active = Some(name);
                    self.focus_panel(Side::Right);
                }
            }
        }
    }

    /// The command center's start and end column in the title bar, if
    /// there's room for it.
    pub(crate) fn command_center(&self) -> Option<(u16, u16)> {
        // In the middle, or right of the menus; left of the layout buttons.
        let menus_end = self.menu_titles().last().map_or(0, |t| t.1);
        let w = (self.width / 3).clamp(20, 50);
        let x = (self.width.saturating_sub(w) / 2).max(menus_end + 3);
        let w = w.min(self.width.saturating_sub(x + 10));
        (self.layout().menu.h > 0 && w >= 16).then_some((x, x + w))
    }

    /// The layout buttons at the right of the title bar: column, glyph,
    /// whether what it toggles shows, and its command.
    pub(crate) fn layout_buttons(&self) -> Vec<(u16, char, bool, &'static str)> {
        let menus_end = self.menu_titles().last().map_or(0, |t| t.1);
        if self.layout().menu.h == 0 || self.width < menus_end + 9 {
            return Vec::new();
        }
        let x = self.width - 7;
        vec![
            (
                x,
                icon(Icon::LayoutLeft, self.nerd),
                self.bar(Side::Left).visible,
                "view.sidebar",
            ),
            (
                x + 2,
                icon(Icon::LayoutPanel, self.nerd),
                self.bar(Side::Bottom).visible,
                "view.panel",
            ),
            (
                x + 4,
                icon(Icon::LayoutRight, self.nerd),
                self.bar(Side::Right).visible,
                "view.secondary",
            ),
        ]
    }

    pub(crate) fn render_title_items(&self, s: &mut Screen) {
        if let Some((a, b)) = self.command_center() {
            s.fill(a, 0, b - a, Style::CommandCenter);
            let label = format!("{} {}", icon(Icon::Search, self.nerd), self.project);
            let w = str_width(&label) as u16;
            let x = a + (b - a).saturating_sub(w) / 2;
            s.print(x, 0, &label, Style::CommandCenter, b - x);
            if self
                .hover
                .is_some_and(|(hx, hy)| hy == 0 && (a..b).contains(&hx))
            {
                s.hover(a, 0, b - a);
            }
        }
        for (x, glyph, on, _) in self.layout_buttons() {
            let style = if on { Style::MenuBar } else { Style::MenuTitle };
            s.put(x, 0, glyph, style);
            if self.hover == Some((x, 0)) {
                s.hover(x, 0, 1);
            }
        }
    }

    /// What a click on the title bar outside the menus does.
    pub(crate) fn title_action(&self, x: u16) -> Option<Action> {
        if self
            .command_center()
            .is_some_and(|(a, b)| (a..b).contains(&x))
        {
            return Some(Action::Run("file.open"));
        }
        self.layout_buttons()
            .into_iter()
            .find(|(bx, ..)| *bx == x)
            .map(|(.., command)| Action::Run(command))
    }

    /// The status bar's items, laid out: what needs doing and the branch
    /// and problems on the left, the cursor and file on the right.
    pub(crate) fn status_items(&self) -> Vec<StatusItem> {
        let width = self.layout().status.w;
        let doc = self.doc();
        let mut left: Vec<(String, Style, Option<Action>)> = Vec::new();
        let reviews = self
            .docs
            .iter()
            .filter(|d| d.review.as_ref().is_some_and(|r| !r.plugin.is_empty()))
            .count();
        if reviews > 0 {
            let what = if reviews == 1 { "change" } else { "changes" };
            left.push((
                format!(
                    " {} {reviews} {what} to review ",
                    icon(Icon::Diff, self.nerd)
                ),
                Style::StatusAccent,
                Some(Action::Run("review.next")),
            ));
        }
        if let Some(branch) = &self.git.branch {
            left.push((
                format!(" {} {branch} ", icon(Icon::Branch, self.nerd)),
                Style::Status,
                Some(Action::View(CHANGES)),
            ));
        }
        let (errors, warnings) = self.problem_counts();
        left.push((
            format!(
                " {} {errors}  {} {warnings} ",
                icon(Icon::Error, self.nerd),
                icon(Icon::Warning, self.nerd)
            ),
            Style::Status,
            Some(Action::Run("view.problems")),
        ));
        if doc.review.is_some() {
            left.push((format!("  {}", self.review_hint()), Style::Status, None));
        }

        let (line, col) = doc.line_col(doc.cursor);
        let mut position = format!("Ln {}, Col {}", line + 1, col + 1);
        if !doc.extra.is_empty() {
            position.push_str(&format!(" ({} selections)", doc.extra.len() + 1));
        } else if let Some((a, b)) = doc.selection() {
            position.push_str(&format!(" ({} selected)", b - a));
        }
        let rope = doc.buffer.rope();
        let crlf = rope.len_lines() > 1 && rope.line(0).to_string().ends_with("\r\n");
        let tabs =
            (0..rope.len_lines().min(200)).any(|l| rope.line(l).chars().next() == Some('\t'));
        let indent = if tabs { "Tab Size: 4" } else { "Spaces: 4" };
        let mut right: Vec<(String, Option<Action>)> = vec![
            (position, Some(Action::Run("go.line"))),
            (indent.into(), None),
            ("UTF-8".into(), None),
            ((if crlf { "CRLF" } else { "LF" }).into(), None),
            (self.language(), None),
        ];
        let left_w: usize = left.iter().map(|(t, ..)| str_width(t)).sum();
        // A diagnostic under the cursor needs room more than the encoding.
        let wanted = |r: &[(String, Option<Action>)]| {
            r.iter().map(|(t, _)| str_width(t) + 3).sum::<usize>() + left_w + 1
        };
        let room =
            (width as usize).saturating_sub(if self.cursor_mark().is_some() { 40 } else { 0 });
        while right.len() > 2 && wanted(&right) > room {
            right.remove(1);
        }

        let mut items = Vec::new();
        let mut x = 0;
        for (text, style, action) in left {
            let w = str_width(&text) as u16;
            items.push(StatusItem {
                x,
                text,
                style,
                action,
            });
            x += w;
        }
        let right_w: u16 = right.iter().map(|(t, _)| str_width(t) as u16 + 3).sum();
        let mut x = width.saturating_sub(right_w).max(x + 1);
        for (text, action) in right {
            let w = str_width(&text) as u16 + 3;
            items.push(StatusItem {
                x,
                text: format!(" {text}  "),
                style: Style::Status,
                action,
            });
            x += w;
        }
        items
    }

    /// The diagnostic under the cursor, or else on its line.
    pub(crate) fn cursor_mark(&self) -> Option<&crate::lang::Mark> {
        let doc = self.doc();
        let line = doc.line_col(doc.cursor).0;
        doc.marks
            .iter()
            .filter(|m| m.start <= doc.cursor && doc.cursor <= m.end.max(m.start + 1))
            .chain(doc.marks.iter().filter(|m| doc.line_col(m.start).0 == line))
            .next()
    }

    pub(crate) fn render_status(&self, s: &mut Screen, area: Rect) {
        if area.h == 0 {
            return;
        }
        let row = area.y;
        s.fill(0, row, area.w, Style::Status);
        let items = self.status_items();
        let mut left_end = 0;
        let mut right_start = area.w;
        for item in &items {
            let w = s.print(
                item.x,
                row,
                &item.text,
                item.style,
                area.w.saturating_sub(item.x),
            );
            if item.x < area.w / 2 {
                left_end = left_end.max(item.x + w);
            } else {
                right_start = right_start.min(item.x);
            }
            let hovered = self
                .hover
                .is_some_and(|(hx, hy)| hy == row && (item.x..item.x + w).contains(&hx));
            if hovered && item.action.is_some() {
                s.hover(item.x, row, w);
            }
        }
        // A diagnostic under the cursor gets the middle.
        if let Some(mark) = self.cursor_mark() {
            let first = mark.message.lines().next().unwrap_or_default();
            let style = match mark.severity {
                Severity::Error => Style::StatusError,
                _ => Style::Status,
            };
            let room = right_start.saturating_sub(left_end + 2);
            s.print(left_end + 1, row, &format!(" {first} "), style, room);
        }
    }

    pub(crate) fn status_click(&mut self, x: u16, fx: &mut Vec<Effect>) {
        let hit = self.status_items().into_iter().find(|i| {
            let w = str_width(&i.text) as u16;
            (i.x..i.x + w).contains(&x)
        });
        if let Some(action) = hit.and_then(|i| i.action) {
            self.act(action, fx);
        }
    }

    /// Where the notice showing is, at the bottom right of the text.
    pub(crate) fn toast(&self) -> Option<Toast> {
        let message = self.message.as_ref()?;
        let text = self.layout().text;
        if text.h < 3 {
            return None;
        }
        let body = str_width(&message.text) as u16 + 6;
        let buttons_w: u16 = message
            .actions
            .iter()
            .map(|(l, _)| str_width(l) as u16 + 3)
            .sum();
        let h = if message.actions.is_empty() { 3 } else { 4 };
        let w = body.max(buttons_w + 4).min(text.w.saturating_sub(2));
        let area = Rect {
            x: text.right().saturating_sub(w + 2),
            y: text.bottom().saturating_sub(h),
            w,
            h,
        };
        let mut buttons = Vec::new();
        let mut x = area.right().saturating_sub(2);
        for (label, command) in message.actions.iter().rev() {
            let bw = str_width(label) as u16 + 2;
            x = x.saturating_sub(bw);
            buttons.push((x, bw, command.clone()));
            x = x.saturating_sub(1);
        }
        buttons.reverse();
        Some(Toast { area, buttons })
    }

    pub(crate) fn render_toast(&self, s: &mut Screen) {
        let (Some(message), Some(toast)) = (&self.message, self.toast()) else {
            return;
        };
        let a = toast.area;
        let (style, glyph) = if message.error {
            (Style::ToastError, Icon::Error)
        } else {
            (Style::Toast, Icon::Info)
        };
        let frame = if message.error {
            Style::ToastError
        } else {
            Style::WidgetFrame
        };
        s.frame(a.x, a.y, a.w, a.h, frame, style);
        let text = format!(" {}  {}", icon(glyph, self.nerd), message.text);
        s.print(a.x + 1, a.y + 1, &text, style, a.w.saturating_sub(2));
        for (i, (x, w, _)) in toast.buttons.iter().enumerate() {
            let button = if i + 1 == toast.buttons.len() {
                Style::ButtonFocused
            } else {
                Style::Button
            };
            let label = &message.actions[i].0;
            s.print(*x, a.y + 2, &format!(" {label} "), button, *w);
        }
    }

    /// The command of a notice's button at a cell, if there is one.
    pub(crate) fn toast_click(&self, x: u16, y: u16) -> Option<String> {
        let toast = self.toast()?;
        if y != toast.area.y + 2 {
            return None;
        }
        toast
            .buttons
            .into_iter()
            .find(|(bx, w, _)| (*bx..bx + w).contains(&x))
            .map(|(.., command)| command)
    }
}
