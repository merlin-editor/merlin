//! Commands: the built-in ones by id and title, and running one.

use super::*;

/// Commands, titled as VS Code titles them. Menus show the part after the
/// colon.
pub(super) const BUILTIN: &[(&str, &str)] = &[
    ("file.open", "Go: Go to File…"),
    ("file.save", "File: Save"),
    ("file.close", "File: Close Editor"),
    ("file.next", "View: Next Editor"),
    ("file.keep", "View: Keep Editor Open"),
    ("view.breadcrumbs", "View: Toggle Breadcrumbs"),
    ("view.sticky_scroll", "View: Toggle Sticky Scroll"),
    ("edit.fold", "Fold"),
    ("edit.unfold", "Unfold"),
    ("edit.fold_all", "Fold All"),
    ("edit.unfold_all", "Unfold All"),
    ("file.prev", "View: Previous Editor"),
    ("edit.undo", "Edit: Undo"),
    ("edit.redo", "Edit: Redo"),
    ("edit.select_all", "Selection: Select All"),
    ("edit.copy", "Edit: Copy"),
    ("edit.cut", "Edit: Cut"),
    ("edit.paste", "Edit: Paste"),
    ("edit.comment", "Edit: Toggle Line Comment"),
    ("edit.indent", "Selection: Indent Lines"),
    ("edit.outdent", "Selection: Outdent Lines"),
    ("edit.move_line_up", "Edit: Move Line Up"),
    ("edit.move_line_down", "Edit: Move Line Down"),
    ("edit.copy_line_up", "Edit: Copy Line Up"),
    ("edit.copy_line_down", "Edit: Copy Line Down"),
    ("edit.delete_line", "Edit: Delete Line"),
    ("edit.insert_line_below", "Edit: Insert Line Below"),
    ("edit.insert_line_above", "Edit: Insert Line Above"),
    ("edit.select_line", "Selection: Expand Line Selection"),
    ("edit.jump_to_bracket", "Go: Go to Bracket"),
    ("file.close_others", "View: Close Other Editors"),
    ("file.new", "File: New Untitled File"),
    ("explorer.new_file", "Explorer: New File…"),
    ("explorer.new_folder", "Explorer: New Folder…"),
    ("explorer.rename", "Explorer: Rename…"),
    ("explorer.delete", "Explorer: Delete"),
    ("file.save_as", "File: Save As…"),
    ("file.reopen", "View: Reopen Closed Editor"),
    ("view.zoom_in", "View: Zoom In"),
    ("view.zoom_out", "View: Zoom Out"),
    ("view.zoom_reset", "View: Reset Zoom"),
    ("file.close_right", "View: Close Editors to the Right"),
    ("file.close_all", "View: Close All Editors"),
    ("file.copy_path", "File: Copy Path of Active File"),
    ("edit.expand_selection", "Selection: Expand Selection"),
    ("edit.shrink_selection", "Selection: Shrink Selection"),
    ("edit.cursor_undo", "Selection: Cursor Undo"),
    (
        "edit.skip_next",
        "Selection: Move Last Selection to Next Find Match",
    ),
    ("find.open", "Edit: Find"),
    ("find.replace", "Edit: Replace"),
    ("palette.open", "View: Command Palette…"),
    ("view.sidebar", "View: Toggle Side Bar"),
    ("view.explorer", "View: Explorer"),
    ("view.search", "View: Search"),
    ("view.outline", "View: Outline"),
    ("settings.user", "Preferences: Open User Settings"),
    ("settings.workspace", "Preferences: Open Workspace Settings"),
    ("settings.open", "Preferences: Open Settings"),
    ("view.plugins", "View: Plugins"),
    ("theme.pick", "Preferences: Color Theme"),
    ("icons.pick", "Preferences: File Icon Theme"),
    ("go.symbol", "Go to Symbol in Editor"),
    ("view.changes", "View: Source Control"),
    ("panel.toggle", "View: Focus Secondary Side Bar"),
    ("view.secondary", "View: Toggle Secondary Side Bar"),
    ("view.panel", "View: Toggle Panel"),
    ("view.minimap", "View: Toggle Minimap"),
    ("view.wrap", "View: Toggle Word Wrap"),
    ("view.split", "View: Split Editor Right"),
    ("view.split_down", "View: Split Editor Down"),
    ("edit.add_next", "Selection: Add Next Occurrence"),
    (
        "edit.select_all_matches",
        "Selection: Select All Occurrences",
    ),
    ("edit.cursor_above", "Selection: Add Cursor Above"),
    ("edit.cursor_below", "Selection: Add Cursor Below"),
    ("view.group1", "View: Focus First Editor Group"),
    ("view.group2", "View: Focus Second Editor Group"),
    ("view.group3", "View: Focus Third Editor Group"),
    ("view.problems", "View: Problems"),
    ("view.output", "View: Output"),
    ("go.line", "Go: Go to Line…"),
    ("review.next", "Go: Next Change to Review"),
    ("nav.back", "Go: Back"),
    ("app.about", "Help: About"),
    ("app.quit", "File: Exit"),
];

pub(super) fn set_flag(key: &str, on: bool) -> Effect {
    Effect::SetSetting {
        key: key.into(),
        value: on.to_string(),
    }
}

impl Editor {
    pub(crate) fn run(&mut self, id: &str, armed: Option<&'static str>, fx: &mut Vec<Effect>) {
        if id.starts_with("explorer.") && self.explorer_command(id, fx) {
            return;
        }
        match id {
            "file.save" => {
                let doc = self.doc();
                match &doc.buffer.path {
                    Some(path) => fx.push(Effect::Save {
                        doc: doc.id,
                        path: path.clone(),
                        text: doc.buffer.text(),
                        revision: doc.buffer.revision(),
                    }),
                    None => self.run("file.save_as", None, fx),
                }
            }
            "file.open" => fx.push(Effect::ListFiles),
            "file.close" => self.close(armed == Some("file.close")),
            "file.next" => self.next_tab(true),
            "file.keep" => self.keep_open(),
            "view.breadcrumbs" => {
                self.breadcrumbs = !self.breadcrumbs;
                fx.push(set_flag("editor.breadcrumbs", self.breadcrumbs));
            }
            "view.sticky_scroll" => {
                self.sticky_scroll = !self.sticky_scroll;
                fx.push(set_flag("editor.sticky_scroll", self.sticky_scroll));
            }
            "settings.user" => fx.push(Effect::OpenSettings { workspace: false }),
            "settings.workspace" => fx.push(Effect::OpenSettings { workspace: true }),
            "edit.fold" => self.fold(),
            "edit.unfold" => self.unfold(),
            "edit.fold_all" => self.fold_all(),
            "edit.unfold_all" => self.unfold_all(),
            "file.prev" => self.next_tab(false),
            "view.split" => self.split(false),
            "view.split_down" => self.split(true),
            "edit.add_next" => self.add_next_occurrence(),
            "edit.select_all_matches" => self.select_all_occurrences(),
            "edit.cursor_above" => self.add_caret_vertical(false),
            "edit.cursor_below" => self.add_caret_vertical(true),
            "view.group1" => self.focus_group(0),
            "view.group2" => self.focus_group(1),
            "view.group3" => self.focus_group(2),
            "nav.back" => self.go_back(),
            "view.sidebar" => self.toggle_side_bar(fx),
            "view.explorer" => self.show_view(views::EXPLORER, fx),
            "view.search" => self.show_view(views::SEARCH, fx),
            "view.outline" => self.show_view(views::OUTLINE, fx),
            "go.symbol" => {
                let items = crate::symbols::symbols(self.doc())
                    .into_iter()
                    .map(|s| {
                        let label = if s.kind == "#" || s.kind == "impl" {
                            s.name
                        } else {
                            format!("{} {}", s.kind, s.name)
                        };
                        (
                            s.line.to_string(),
                            format!("{}{label}", "  ".repeat(s.depth)),
                        )
                    })
                    .collect();
                self.palette = Some(Palette::new(
                    Source::Symbols,
                    Some("Go to symbol".into()),
                    items,
                ));
                self.focus = Focus::Palette;
            }
            "view.changes" => self.show_view(views::CHANGES, fx),
            "view.panel" => self.toggle_bottom(),
            "view.minimap" => {
                self.minimap = !self.minimap;
                fx.push(set_flag("editor.minimap", self.minimap));
            }
            "view.wrap" => {
                self.wrap = !self.wrap;
                fx.push(set_flag("editor.word_wrap", self.wrap));
                self.doc_mut().hscroll = 0;
                self.scroll_to_cursor();
            }
            "view.problems" => self.show_bottom(views::PROBLEMS),
            "view.output" => self.show_bottom(views::OUTPUT),
            "view.secondary" => {
                let bar = self.bar_mut(Side::Right);
                bar.visible = !bar.visible;
                if !bar.visible && self.focus == Focus::Panel {
                    self.focus = Focus::Editor;
                }
            }
            "review.next" => {
                let next = (1..=self.docs.len())
                    .map(|k| (self.current + k) % self.docs.len())
                    .find(|&i| {
                        self.docs[i]
                            .review
                            .as_ref()
                            .is_some_and(|r| !r.plugin.is_empty())
                    });
                match next {
                    Some(i) => {
                        self.current = i;
                        self.focus = Focus::Editor;
                    }
                    None => self.notify("No changes to review".into()),
                }
            }
            "go.line" => {
                self.palette = Some(Palette::new(
                    Source::Line,
                    Some("Go to line".into()),
                    Vec::new(),
                ));
                self.focus = Focus::Palette;
            }
            "edit.comment" => self.toggle_comment(),
            "edit.indent" => self.indent_lines(true),
            "edit.outdent" => self.indent_lines(false),
            "edit.move_line_up" => self.move_lines_by(false),
            "edit.move_line_down" => self.move_lines_by(true),
            "edit.copy_line_up" => self.copy_lines(false),
            "edit.copy_line_down" => self.copy_lines(true),
            "edit.delete_line" => self.each_caret(Kind::Other, |e, _| e.delete_lines()),
            "edit.insert_line_below" => self.each_caret(Kind::Other, |e, _| e.insert_line(true)),
            "edit.insert_line_above" => self.each_caret(Kind::Other, |e, _| e.insert_line(false)),
            "edit.select_line" => self.each_caret(Kind::Other, |e, _| e.select_line()),
            "edit.jump_to_bracket" => self.jump_to_bracket(),
            "file.close_others" => self.close_others(),
            "file.new" => {
                let id = self.new_id();
                self.docs.push(Doc::new(id, "", None));
                self.current = self.docs.len() - 1;
                self.focus = Focus::Editor;
                self.sync_groups();
            }
            "file.save_as" => {
                let now = self
                    .doc()
                    .buffer
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let mut palette = Palette::new(Source::SaveAs, Some("Save as".into()), Vec::new());
                palette.query = now;
                self.palette = Some(palette);
                self.focus = Focus::Palette;
            }
            "file.reopen" => match self.closed.pop() {
                Some(path) => self.open(path, fx),
                None => self.notify("No closed files to reopen".into()),
            },
            "view.zoom_in" => fx.push(Effect::Zoom(1)),
            "view.zoom_out" => fx.push(Effect::Zoom(-1)),
            "view.zoom_reset" => fx.push(Effect::Zoom(0)),
            "file.close_right" => self.close_right(),
            "file.close_all" => self.close_all(),
            "file.copy_path" => self.copy_path(fx),
            "edit.expand_selection" => self.expand_selection(),
            "edit.shrink_selection" => self.shrink_selection(),
            "edit.cursor_undo" => self.undo_cursor(),
            "edit.skip_next" => self.skip_occurrence(),
            "app.about" => self.notify(format!("merlin {}", env!("CARGO_PKG_VERSION"))),
            menu if menu.starts_with("menu.") => {
                let title = menu.strip_prefix("menu.").filter(|t| *t != "open");
                self.open_menu(title);
            }
            "edit.undo" => match self.doc_mut().buffer.undo() {
                Some(cursor) => self.set_cursor(cursor),
                None => self.notify("Nothing to undo".into()),
            },
            "edit.redo" => match self.doc_mut().buffer.redo() {
                Some(cursor) => self.set_cursor(cursor),
                None => self.notify("Nothing to redo".into()),
            },
            "edit.select_all" => {
                let doc = self.doc_mut();
                doc.anchor = Some(0);
                doc.cursor = doc.buffer.len();
                self.focus = Focus::Editor;
            }
            "edit.copy" => self.copy(false, fx),
            "edit.cut" => self.copy(true, fx),
            "edit.paste" => fx.push(Effect::ReadClipboard),
            "find.open" => self.open_find(false),
            "find.replace" => self.open_find(true),
            "palette.open" => {
                self.palette = Some(Palette::new(Source::Commands, None, Vec::new()));
                self.focus = Focus::Palette;
            }
            "settings.open" => fx.push(Effect::OpenSettingsWindow),
            "view.plugins" => self.show_view(views::PLUGINS, fx),
            "theme.pick" => self.pick_theme(ThemeKind::Colours),
            "icons.pick" => self.pick_theme(ThemeKind::Icons),
            // Show and focus, then step through the panels, then hide.
            "panel.toggle" => self.toggle_plugin_panels(),
            "app.quit" => {
                let dirty = self.docs.iter().filter(|d| d.buffer.is_dirty()).count();
                if dirty > 0 && armed != Some("app.quit") {
                    self.armed = Some("app.quit");
                    let what = match dirty {
                        1 => "Unsaved changes".to_string(),
                        n => format!("{n} files have unsaved changes"),
                    };
                    self.error(format!(
                        "{what}. Press Ctrl+Q again to quit without saving."
                    ));
                } else {
                    fx.push(Effect::Quit);
                }
            }
            other => {
                let found = self.plugins.iter().find_map(|p| {
                    let local = other.strip_prefix(&p.name)?.strip_prefix('.')?;
                    p.commands
                        .iter()
                        .any(|c| c.id == local)
                        .then(|| (p.name.clone(), local.to_string()))
                });
                match found {
                    Some((plugin, command)) => fx.push(Effect::PluginCommand { plugin, command }),
                    None => self.error(format!("Unknown command: {other}")),
                }
            }
        }
    }
}
