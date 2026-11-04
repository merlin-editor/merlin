//! Keys: the built-in bindings, VS Code's, and how a key finds its command.

use super::*;

/// Keys that work wherever focus is, as VS Code binds them where the
/// terminal can tell them apart. Ctrl+Shift needs a terminal with the kitty
/// keyboard protocol; elsewhere the menus and F1 reach everything.
pub(super) const GLOBAL_KEYS: &[(&str, &str)] = &[
    ("ctrl+s", "file.save"),
    ("ctrl+p", "file.open"),
    ("ctrl+o", "file.open"),
    ("ctrl+w", "file.close"),
    ("ctrl+q", "app.quit"),
    ("ctrl+shift+p", "palette.open"),
    ("ctrl+,", "settings.open"),
    ("f1", "palette.open"),
    ("ctrl+f", "find.open"),
    ("ctrl+h", "find.replace"),
    ("ctrl+b", "view.sidebar"),
    ("ctrl+alt+b", "panel.toggle"),
    ("ctrl+shift+e", "view.explorer"),
    ("ctrl+e", "view.explorer"),
    ("ctrl+shift+f", "view.search"),
    ("ctrl+shift+o", "go.symbol"),
    ("ctrl+shift+g", "view.changes"),
    ("ctrl+j", "view.panel"),
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
    // kitty keeps Ctrl+Shift+[ and ] for its own windows.
    ("ctrl+alt+[", "edit.fold"),
    ("ctrl+alt+]", "edit.unfold"),
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
];

/// Two keys in a row, as VS Code's Ctrl+K chords.
pub(super) const CHORDS: &[(&str, &str, &str)] = &[
    ("ctrl+k", "ctrl+d", "edit.skip_next"),
    ("ctrl+k", "ctrl+0", "edit.fold_all"),
    ("ctrl+k", "ctrl+j", "edit.unfold_all"),
    ("ctrl+k", "ctrl+t", "theme.pick"),
    ("ctrl+k", "ctrl+s", "settings.open"),
];

pub(super) fn bound(table: &[(&str, &'static str)], key: Key) -> Option<&'static str> {
    table
        .iter()
        .find(|(spec, _)| Key::parse(spec) == Some(key))
        .map(|(_, id)| *id)
}

impl Editor {
    pub(crate) fn key(&mut self, key: Key, armed: Option<&'static str>, fx: &mut Vec<Effect>) {
        // Keys from the settings come first; one bound to nothing does
        // nothing.
        if let Some((_, command)) = self.user_keys.iter().find(|(k, _)| *k == key) {
            if command.is_empty() {
                return;
            }
            let command = command.clone();
            return self.run(&command, armed, fx);
        }
        if let Some(first) = self.chord.take() {
            let parse = |spec: &str| Key::parse(spec);
            let id = CHORDS
                .iter()
                .find(|(a, b, _)| parse(a) == Some(first) && parse(b) == Some(key))
                .map(|(.., id)| *id);
            self.message = None;
            return match id {
                Some(id) => self.run(id, armed, fx),
                None => self.notify(format!(
                    "The key combination ({}, {}) is not a command",
                    first, key
                )),
            };
        }
        if CHORDS.iter().any(|(a, ..)| Key::parse(a) == Some(key)) {
            self.chord = Some(key);
            return self.notify(format!(
                "({}) was pressed. Waiting for the second key of the chord…",
                key
            ));
        }
        if let Some(id) = bound(GLOBAL_KEYS, key) {
            return self.run(id, armed, fx);
        }
        if matches!(
            self.focus,
            Focus::Editor | Focus::Panel | Focus::SideBar | Focus::Bottom
        ) {
            let plugin_key = self.plugins.iter().find_map(|p| {
                let (_, command) = p.keys.iter().find(|(k, _)| *k == key)?;
                Some((p.name.clone(), command.clone()))
            });
            if let Some((plugin, command)) = plugin_key {
                return fx.push(Effect::PluginCommand { plugin, command });
            }
        }
        match self.focus {
            Focus::Editor => self.editor_key(key, fx),
            Focus::Find => self.find_key(key),
            Focus::Palette => self.palette_key(key, armed, fx),
            Focus::Panel => self.panel_key(Side::Right, key, fx),
            Focus::SideBar => self.panel_key(Side::Left, key, fx),
            Focus::Bottom => self.panel_key(Side::Bottom, key, fx),
            Focus::Menu => self.menu_key(key, fx),
        }
    }

    /// Every command and the keys that run it, after the settings' changes.
    pub fn key_bindings(&self) -> Vec<KeyBinding> {
        let defaults: Vec<(Key, &str)> = GLOBAL_KEYS
            .iter()
            .chain(EDITOR_KEYS)
            .filter_map(|(spec, id)| Some((Key::parse(spec)?, *id)))
            .collect();
        let user = |key: Key| self.user_keys.iter().find(|(k, _)| *k == key);
        let mut out = Vec::new();
        for (id, title) in self.commands() {
            let mut keys = Vec::new();
            let mut changed = false;
            let plugin = self.plugins.iter().find(|p| {
                id.strip_prefix(&p.name)
                    .is_some_and(|rest| rest.starts_with('.'))
            });
            let from_plugin = plugin.map(|p| {
                let local = &id[p.name.len() + 1..];
                p.keys
                    .iter()
                    .filter(|(_, c)| c == local)
                    .map(|(k, _)| *k)
                    .collect::<Vec<_>>()
            });
            let own = from_plugin.unwrap_or_else(|| {
                defaults
                    .iter()
                    .filter(|(_, c)| *c == id)
                    .map(|(k, _)| *k)
                    .collect()
            });
            for key in own {
                // A key the settings bind elsewhere, or to nothing, is gone.
                match user(key) {
                    Some((_, c)) if *c != id => changed = true,
                    _ => keys.push(key),
                }
            }
            for (key, c) in &self.user_keys {
                if *c == id && !keys.contains(key) {
                    keys.push(*key);
                    changed = true;
                }
            }
            let defaults = defaults
                .iter()
                .filter(|(_, c)| *c == id)
                .map(|(k, _)| k.spec())
                .collect();
            out.push(KeyBinding {
                command: id,
                title,
                defaults,
                keys: keys.iter().map(|k| (k.spec(), k.to_string())).collect(),
                changed,
                plugin: plugin.map(|p| self.panel_title(Side::Right, &p.name).0),
            });
        }
        out
    }
}
