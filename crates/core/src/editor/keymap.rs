//! Key bindings as VS Code keeps them: one list of keys, the command each
//! runs and when it applies, so the settings, plugins and built-in keys
//! all work the same way, and menus show whatever key really runs a
//! command.
//!
//! The settings' bindings come first, then plugins', then the built-in
//! ones, and the first whose keys match and whose `when` holds wins. A key
//! is one key or two in a row, as Ctrl+K Ctrl+D.

use super::keys::{CHORDS, EDITOR_KEYS, GLOBAL_KEYS};
use super::{Editor, Focus, Popup};
use crate::key::Key;

/// When a binding applies: context keys joined with `&&` and `||`, each
/// maybe with `!`, or compared with `==` and `!=`, as VS Code writes them.
/// Empty holds always.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct When {
    /// Any of these, each all of its terms.
    any: Vec<Vec<Term>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Term {
    /// A context key that's true, or with `!` false.
    Is { key: String, not: bool },
    /// A context key with a value, such as `editorLangId == rust`.
    Equals {
        key: String,
        value: String,
        not: bool,
    },
}

impl When {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut any = Vec::new();
        for alternative in text.split("||") {
            let mut all = Vec::new();
            for term in alternative.split("&&") {
                let term = term.trim();
                if term.is_empty() {
                    if text.trim().is_empty() {
                        continue;
                    }
                    return Err(format!("`{text}` has an empty part"));
                }
                let compare = term
                    .split_once("!=")
                    .map(|(k, v)| (k, v, true))
                    .or_else(|| term.split_once("==").map(|(k, v)| (k, v, false)));
                all.push(match compare {
                    Some((key, value, not)) => Term::Equals {
                        key: name(key.trim(), text)?,
                        value: value.trim().trim_matches('\'').to_string(),
                        not,
                    },
                    None => {
                        let (not, key) = match term.strip_prefix('!') {
                            Some(rest) => (true, rest.trim()),
                            None => (false, term),
                        };
                        Term::Is {
                            key: name(key, text)?,
                            not,
                        }
                    }
                });
            }
            if !all.is_empty() {
                any.push(all);
            }
        }
        Ok(Self { any })
    }

    fn of(text: &str) -> Self {
        Self::parse(text).expect("a built-in when")
    }

    /// Whether it holds, given each context key's value: `None` for one
    /// that isn't set, which counts as false.
    pub(crate) fn holds(&self, value: impl Fn(&str) -> Option<String>) -> bool {
        self.any.is_empty()
            || self.any.iter().any(|all| {
                all.iter().all(|term| match term {
                    Term::Is { key, not } => {
                        let on = value(key).is_some_and(|v| v != "false" && !v.is_empty());
                        on != *not
                    }
                    Term::Equals {
                        key,
                        value: want,
                        not,
                    } => (value(key).as_deref() == Some(want.as_str())) != *not,
                })
            })
    }
}

fn name(key: &str, text: &str) -> Result<String, String> {
    let ok = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '_');
    if ok {
        Ok(key.to_string())
    } else {
        Err(format!("`{text}` isn't a condition merlin understands"))
    }
}

/// Where a binding is from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum From {
    Builtin,
    /// A plugin, by name.
    Plugin(String),
    Settings,
}

#[derive(Clone, Debug)]
pub(crate) struct Binding {
    /// One key, or two in a row.
    pub keys: Vec<Key>,
    /// The command's id; empty unbinds the keys.
    pub command: String,
    pub when: When,
    pub from: From,
}

/// A binding from the settings, as `[keys]` writes it: keys such as
/// `ctrl+k ctrl+m`, the command, and when it applies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserKey {
    pub keys: String,
    pub command: String,
    pub when: Option<String>,
}

impl UserKey {
    pub fn new(keys: &str, command: &str) -> Self {
        Self {
            keys: keys.into(),
            command: command.into(),
            when: None,
        }
    }
}

/// `ctrl+k ctrl+d` as keys.
pub(crate) fn parse_keys(spec: &str) -> Option<Vec<Key>> {
    let keys: Option<Vec<Key>> = spec.split_whitespace().map(Key::parse).collect();
    keys.filter(|k| (1..=2).contains(&k.len()))
}

/// Keys as settings write them, and as people read them.
pub(crate) fn spec_of(keys: &[Key]) -> (String, String) {
    let spec: Vec<String> = keys.iter().map(Key::spec).collect();
    let shown: Vec<String> = keys.iter().map(Key::to_string).collect();
    (spec.join(" "), shown.join(" "))
}

/// The built-in bindings, in the order they're tried.
pub(crate) fn builtin() -> Vec<Binding> {
    let one = |table: &[(&str, &str)], when: &str| -> Vec<Binding> {
        table
            .iter()
            .filter_map(|(spec, id)| {
                Some(Binding {
                    keys: vec![Key::parse(spec)?],
                    command: id.to_string(),
                    when: When::of(when),
                    from: From::Builtin,
                })
            })
            .collect()
    };
    // The changes tab's keys come first: Alt+S stages there, and opens the
    // Selection menu elsewhere.
    let mut all = one(super::keys::DIFF_KEYS, "isInDiffEditor");
    all.extend(one(super::keys::MERGE_KEYS, "isInMergeEditor"));
    all.extend(one(GLOBAL_KEYS, ""));
    all.extend(one(EDITOR_KEYS, "editorTextFocus"));
    all.extend(CHORDS.iter().filter_map(|(a, b, id)| {
        Some(Binding {
            keys: vec![Key::parse(a)?, Key::parse(b)?],
            command: id.to_string(),
            when: When::default(),
            from: From::Builtin,
        })
    }));
    all
}

impl Editor {
    /// What a context key is now, for `when`: `None` when it isn't set.
    pub(crate) fn context_value(&self, key: &str) -> Option<String> {
        let flag = |on: bool| on.then(|| "true".to_string());
        let text = self.focus == Focus::Editor && self.doc().page.is_none();
        match key {
            "editorTextFocus" | "editorFocus" => flag(text),
            "textInputFocus" => flag(text || matches!(self.focus, Focus::Find | Focus::Palette)),
            "inputFocus" => flag(matches!(self.focus, Focus::Find | Focus::Palette)),
            "editorHasSelection" => flag(self.doc().selection().is_some()),
            "editorHasMultipleSelections" => flag(self.has_carets()),
            "editorReadonly" => flag(self.doc().review.is_some() || self.doc().page.is_some()),
            "editorLangId" => Some(
                self.plugin_language()
                    .map(|l| l.id.clone())
                    .unwrap_or_else(|| self.extension().to_string()),
            ),
            "findWidgetVisible" => flag(self.find.is_some()),
            "findInputFocussed" | "findInputFocused" => flag(self.focus == Focus::Find),
            "inQuickOpen" => flag(self.focus == Focus::Palette),
            "suggestWidgetVisible" => flag(matches!(self.popup, Some(Popup::Completion { .. }))),
            "sideBarFocus" => flag(self.focus == Focus::SideBar),
            "panelFocus" => flag(self.focus == Focus::Bottom),
            "terminalFocus" => flag(self.terminal_focused()),
            "auxiliaryBarFocus" => flag(self.focus == Focus::Panel),
            "menuFocus" => flag(self.focus == Focus::Menu),
            "multipleEditorGroups" => flag(self.groups.len() > 1),
            "isInDiffEditor" => flag(self.focus == Focus::Editor && self.in_changes_tab()),
            "isInMergeEditor" => flag(self.focus == Focus::Editor && self.merging().is_some()),
            _ => None,
        }
    }

    /// Brings the bindings up to date with the settings and plugins.
    pub(crate) fn rebuild_keymap(&mut self) {
        let mut all = self.user_keys.clone();
        for plugin in &self.plugins {
            all.extend(plugin.keys.iter().map(|(key, command)| Binding {
                keys: vec![*key],
                command: format!("{}.{command}", plugin.name),
                // Plugins' keys work in the text and the panels, not
                // while typing in the palette or find.
                when: When::of("!inputFocus && !menuFocus"),
                from: From::Plugin(plugin.name.clone()),
            }));
        }
        all.extend(builtin());
        self.keymap = all;
    }

    /// The first binding for `keys` that applies now.
    pub(crate) fn binding_for(&self, keys: &[Key]) -> Option<&Binding> {
        let value = |k: &str| self.context_value(k);
        self.keymap
            .iter()
            .find(|b| b.keys == keys && b.when.holds(value))
    }

    /// Whether `key` starts a chord that applies now.
    pub(crate) fn starts_chord(&self, key: Key) -> bool {
        let value = |k: &str| self.context_value(k);
        self.keymap
            .iter()
            .any(|b| b.keys.len() == 2 && b.keys[0] == key && b.when.holds(value))
    }

    /// The keys that run `command`, as people read them, for menus: the
    /// first binding to it that nothing ahead of it takes.
    pub(crate) fn key_hint(&self, command: &str) -> Option<String> {
        self.keymap
            .iter()
            .enumerate()
            .find(|(i, b)| {
                b.command == command
                    && !self.keymap[..*i]
                        .iter()
                        .any(|e| e.keys == b.keys && e.when == b.when)
            })
            .map(|(_, b)| spec_of(&b.keys).1)
    }
}
