//! Keys: the built-in bindings, VS Code's, and how a key finds its command.

use super::keymap::{Binding, From, When, spec_of};
use super::*;

/// Keys in a file's changes since the last commit.
pub(super) const DIFF_KEYS: &[(&str, &str)] = &[
    ("alt+f5", "diff.next"),
    ("alt+f3", "diff.prev"),
    ("alt+i", "diff.inline"),
    ("alt+s", "diff.stage"),
    ("alt+r", "diff.revert"),
];

/// Keys in the merge editor: the changes tab's, for conflicts.
pub(super) const MERGE_KEYS: &[(&str, &str)] = &[
    ("alt+f5", "merge.next"),
    ("alt+f3", "merge.prev"),
    ("alt+i", "diff.inline"),
];

/// Keys that work wherever focus is, as VS Code binds them on Linux.
pub(super) const GLOBAL_KEYS: &[(&str, &str)] = &[
    ("ctrl+s", "file.save"),
    ("ctrl+p", "file.open"),
    ("ctrl+o", "file.open_file"),
    ("ctrl+w", "file.close"),
    ("ctrl+q", "app.quit"),
    ("ctrl+shift+p", "palette.open"),
    ("ctrl+,", "settings.open"),
    ("f1", "palette.open"),
    ("f12", "go.definition"),
    ("shift+f12", "go.references"),
    ("ctrl+f", "find.open"),
    ("ctrl+h", "find.replace"),
    ("ctrl+n", "file.new"),
    ("ctrl+shift+s", "file.save_as"),
    ("ctrl+shift+t", "file.reopen"),
    ("ctrl+tab", "file.next"),
    ("ctrl+backtab", "file.prev"),
    ("ctrl+=", "view.zoom_in"),
    ("ctrl+shift+=", "view.zoom_in"),
    ("ctrl+-", "view.zoom_out"),
    ("ctrl+0", "view.zoom_reset"),
    ("ctrl+b", "view.sidebar"),
    ("ctrl+alt+b", "panel.toggle"),
    ("ctrl+shift+e", "view.explorer"),
    ("ctrl+e", "file.open"),
    ("ctrl+shift+f", "view.search"),
    ("ctrl+shift+o", "go.symbol"),
    ("ctrl+shift+g", "view.changes"),
    ("ctrl+j", "view.panel"),
    ("ctrl+`", "terminal.toggle"),
    ("ctrl+shift+`", "terminal.new"),
    ("ctrl+shift+m", "view.problems"),
    ("ctrl+shift+u", "view.output"),
    ("ctrl+g", "go.line"),
    ("ctrl+v", "edit.paste"),
    ("ctrl+pagedown", "file.next"),
    ("ctrl+pageup", "file.prev"),
    ("alt+left", "nav.back"),
    ("alt+z", "view.wrap"),
    ("ctrl+\\", "view.split"),
    ("ctrl+shift+[", "edit.fold"),
    ("ctrl+shift+]", "edit.unfold"),
    ("ctrl+1", "view.group1"),
    ("ctrl+2", "view.group2"),
    ("ctrl+3", "view.group3"),
    ("shift+alt+up", "edit.cursor_above"),
    ("shift+alt+down", "edit.cursor_below"),
    ("ctrl+alt+up", "edit.cursor_above"),
    ("ctrl+alt+down", "edit.cursor_below"),
    ("f10", "menu.open"),
    ("alt+f", "menu.file"),
    ("alt+e", "menu.edit"),
    ("alt+s", "menu.selection"),
    ("alt+v", "menu.view"),
    ("alt+g", "menu.go"),
    ("alt+h", "menu.help"),
];

/// Keys that work in the text.
pub(super) const EDITOR_KEYS: &[(&str, &str)] = &[
    ("ctrl+z", "edit.undo"),
    ("ctrl+y", "edit.redo"),
    ("ctrl+shift+z", "edit.redo"),
    ("ctrl+a", "edit.select_all"),
    ("ctrl+c", "edit.copy"),
    ("ctrl+x", "edit.cut"),
    ("ctrl+/", "edit.comment"),
    ("ctrl+d", "edit.add_next"),
    ("ctrl+shift+l", "edit.select_all_matches"),
    ("alt+up", "edit.move_line_up"),
    ("alt+f5", "go.next_change"),
    ("alt+f3", "go.prev_change"),
    ("alt+down", "edit.move_line_down"),
    // Shift+Alt+Up and Down add cursors on Linux, as in VS Code.
    ("ctrl+shift+alt+up", "edit.copy_line_up"),
    ("ctrl+shift+alt+down", "edit.copy_line_down"),
    ("ctrl+shift+k", "edit.delete_line"),
    ("ctrl+enter", "edit.insert_line_below"),
    ("ctrl+shift+enter", "edit.insert_line_above"),
    ("ctrl+l", "edit.select_line"),
    ("ctrl+]", "edit.indent"),
    ("ctrl+[", "edit.outdent"),
    ("ctrl+shift+\\", "edit.jump_to_bracket"),
    ("shift+alt+right", "edit.expand_selection"),
    ("shift+alt+left", "edit.shrink_selection"),
    ("ctrl+u", "edit.cursor_undo"),
    ("ctrl+.", "edit.quick_fix"),
    ("ctrl+shift+i", "edit.format"),
    ("f2", "edit.rename"),
];

/// Two keys in a row, as VS Code's Ctrl+K chords.
pub(super) const CHORDS: &[(&str, &str, &str)] = &[
    ("ctrl+k", "ctrl+d", "edit.skip_next"),
    ("ctrl+k", "ctrl+i", "view.hover"),
    ("ctrl+k", "ctrl+f", "edit.format_selection"),
    ("ctrl+k", "s", "file.save_without_formatting"),
    ("ctrl+k", "ctrl+0", "edit.fold_all"),
    ("ctrl+k", "ctrl+j", "edit.unfold_all"),
    ("ctrl+k", "ctrl+t", "theme.pick"),
    ("ctrl+k", "ctrl+s", "settings.open"),
    ("ctrl+k", "ctrl+w", "file.close_all"),
    ("ctrl+k", "ctrl+o", "file.open_folder"),
];

impl Editor {
    pub(crate) fn key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        // A terminal with the keys takes nearly all of them.
        if self.terminal_key(key, fx) {
            return;
        }
        // Typing in a file's changes since the last commit edits the file.
        if self.focus == Focus::Editor
            && self.chord.is_none()
            && (self.in_changes_tab() || self.in_compare())
            && self.edits_file(key)
        {
            return self.through_file(fx, |e, fx| e.key(key, fx));
        }
        // Esc closes a change opened in place, then the blame column.
        if self.focus == Focus::Editor && key.code == KeyCode::Esc && !key.ctrl && !key.shift {
            if self.change_peeking() {
                self.change_peek = None;
                return;
            }
            if self.blame_shown() && self.popup.is_none() && !self.has_carets() {
                self.blame_column = None;
                self.blame_card = None;
                return;
            }
        }
        // An open completion or code action list takes the keys it uses
        // first.
        if self.focus == Focus::Editor
            && (self.rename_box_key(key, fx)
                || self.peek_key(key, fx)
                || self.actions_key(key, fx)
                || self.popup_key(key))
        {
            return;
        }
        if let Some(first) = self.chord.take() {
            self.message = None;
            return match self.binding_for(&[first, key]).cloned() {
                Some(binding) => self.run_binding(&binding, fx),
                None => self.notify(format!(
                    "The key combination ({first}, {key}) is not a command"
                )),
            };
        }
        if let Some(binding) = self.binding_for(&[key]).cloned() {
            return self.run_binding(&binding, fx);
        }
        if self.starts_chord(key) {
            self.chord = Some(key);
            return self.notify(format!(
                "({key}) was pressed. Waiting for the second key of the chord…"
            ));
        }
        match self.focus {
            Focus::Editor => self.editor_key(key, fx),
            Focus::Find => self.find_key(key),
            Focus::Palette => self.palette_key(key, fx),
            Focus::Panel => self.panel_key(Side::Right, key, fx),
            Focus::SideBar => self.panel_key(Side::Left, key, fx),
            Focus::Bottom => self.panel_key(Side::Bottom, key, fx),
            Focus::Menu => self.menu_key(key, fx),
        }
    }

    /// Runs what a binding says; one to nothing does nothing.
    fn run_binding(&mut self, binding: &Binding, fx: &mut Vec<Effect>) {
        if binding.command.is_empty() {
            return;
        }
        match &binding.from {
            From::Plugin(plugin) => fx.push(Effect::PluginCommand {
                plugin: plugin.clone(),
                command: binding.command[plugin.len() + 1..].to_string(),
            }),
            _ => self.run(&binding.command, fx),
        }
    }

    /// Every command and the keys that run it, after the settings' changes.
    pub fn key_bindings(&self) -> Vec<KeyBinding> {
        // A binding counts unless one tried before it has the same keys
        // and applies at least as often.
        let live = |i: usize| {
            let b = &self.keymap[i];
            !self.keymap[..i]
                .iter()
                .any(|e| e.keys == b.keys && (e.when == b.when || e.when == When::default()))
        };
        let mut out = Vec::new();
        for (id, title) in self.commands() {
            let plugin = self.plugins.iter().find(|p| {
                id.strip_prefix(&p.name)
                    .is_some_and(|rest| rest.starts_with('.'))
            });
            let mine: Vec<usize> = (0..self.keymap.len())
                .filter(|&i| self.keymap[i].command == id)
                .collect();
            let keys: Vec<(String, String)> = mine
                .iter()
                .filter(|&&i| live(i))
                .map(|&i| spec_of(&self.keymap[i].keys))
                .collect();
            // The settings bind it, or take a key it had.
            let changed = mine.iter().any(|&i| {
                self.keymap[i].from == From::Settings
                    || (self.keymap[i].from != From::Settings && !live(i))
            });
            let defaults = mine
                .iter()
                .filter(|&&i| self.keymap[i].from != From::Settings)
                .map(|&i| spec_of(&self.keymap[i].keys).0)
                .collect();
            out.push(KeyBinding {
                command: id,
                title,
                defaults,
                keys,
                changed,
                plugin: plugin.map(|p| self.panel_title(Side::Right, &p.name).0),
            });
        }
        out
    }
}
