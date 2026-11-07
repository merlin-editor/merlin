//! The Keyboard Shortcuts page: the rows, their filters, and recording,
//! removing and resetting a command's keys.

use super::*;

impl App {
    /// The keys rows the filter and search leave, and how many each filter
    /// would list.
    pub(super) fn key_rows(&self, search: &str) -> (Vec<KeyRow>, [usize; 4]) {
        let ui = &self.settings_ui;
        let bindings = self.editor.key_bindings();
        // Keys two commands have, where the settings or a plugin gave one
        // of them: merlin's own share keys only where they can't clash.
        let mut owners: HashMap<&str, Vec<&KeyBinding>> = HashMap::new();
        for b in &bindings {
            for (spec, _) in &b.keys {
                owners.entry(spec).or_default().push(b);
            }
        }
        let clashes = |b: &KeyBinding| {
            b.keys.iter().any(|(spec, _)| {
                let o = &owners[spec.as_str()];
                o.len() > 1 && o.iter().any(|b| b.changed || b.plugin.is_some())
            })
        };
        let mut rows: Vec<KeyRow> = bindings
            .iter()
            .map(|b| KeyRow {
                conflict: clashes(b),
                binding: b.clone(),
            })
            .collect();
        let found = |r: &KeyRow| {
            let b = &r.binding;
            if search.is_empty() {
                true
            } else if ui.record {
                // A key caught: what it runs, alone or as a chord's start.
                b.keys.iter().any(|(_, shown)| {
                    let shown = shown.to_lowercase();
                    shown == search || shown.starts_with(&format!("{search} "))
                })
            } else {
                b.title.to_lowercase().contains(search)
                    || b.command.contains(search)
                    || b.keys
                        .iter()
                        .any(|(_, shown)| shown.to_lowercase().contains(search))
            }
        };
        rows.retain(found);
        let count = |f: KeysFilter| rows.iter().filter(|r| keys_filter(f, r)).count();
        let counts = KeysFilter::ALL.map(count);
        rows.retain(|r| keys_filter(ui.keys, r));
        rows.sort_by(|a, b| {
            let (a, b) = (&a.binding, &b.binding);
            a.keys
                .is_empty()
                .cmp(&b.keys.is_empty())
                .then(a.title.cmp(&b.title))
        });
        (rows, counts)
    }

    /// Starts catching keys for the keys row picked.
    pub(super) fn capture_row(&mut self) -> Vec<Event> {
        let (rows, _) = self.key_rows(&self.settings_ui.search.to_lowercase());
        if let Some(row) = self.settings_ui.key_row.and_then(|i| rows.get(i)) {
            self.settings_ui.capture = Some(Capture {
                command: row.binding.command.clone(),
                keys: Vec::new(),
            });
        }
        Vec::new()
    }

    /// The keys caught go to the command, with its others or instead.
    pub(super) fn capture_done(&mut self, replace: bool) -> Vec<Event> {
        let Some(c) = self.settings_ui.capture.take() else {
            return Vec::new();
        };
        if c.keys.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();
        if replace {
            events.extend(self.unbind(&c.command));
        }
        let spec: Vec<String> = c.keys.iter().map(Key::spec).collect();
        let value = toml::Value::String(c.command).to_string();
        events.extend(self.write(&format!("keys.{}", spec.join(" ")), Some(value)));
        events
    }

    /// Takes a command's keys off it: the settings' own come out of the
    /// file, and merlin's or a plugin's are bound to nothing.
    pub(super) fn unbind(&mut self, command: &str) -> Vec<Event> {
        let Some(binding) = self
            .editor
            .key_bindings()
            .into_iter()
            .find(|k| k.command == command)
        else {
            return Vec::new();
        };
        let scope = if self.settings_ui.workspace {
            &self.loaded.workspace
        } else {
            &self.loaded.user
        };
        let mine: Vec<String> = scope
            .get("keys")
            .and_then(|t| t.as_table())
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();
        let mut events = Vec::new();
        for (spec, _) in binding.keys {
            let value = if mine.contains(&spec) {
                None
            } else {
                Some("\"\"".to_string())
            };
            events.extend(self.write(&format!("keys.{spec}"), value));
        }
        events
    }

    /// A command's keys back to merlin's: what the settings say about it
    /// comes out of the file.
    pub(super) fn reset_key(&mut self, command: &str) -> Vec<Event> {
        let binding = self
            .editor
            .key_bindings()
            .into_iter()
            .find(|k| k.command == command);
        let scope = if self.settings_ui.workspace {
            &self.loaded.workspace
        } else {
            &self.loaded.user
        };
        let specs: Vec<String> = scope
            .get("keys")
            .and_then(|t| t.as_table())
            .map(|keys| {
                keys.iter()
                    .filter(|(spec, value)| {
                        let to = match value {
                            toml::Value::Table(t) => t.get("command").and_then(|c| c.as_str()),
                            other => other.as_str(),
                        };
                        // Its own, and its default keys, which another may
                        // have been given or which may be bound to nothing.
                        to == Some(command)
                            || binding.as_ref().is_some_and(|b| b.defaults.contains(spec))
                    })
                    .map(|(spec, _)| spec.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut events = Vec::new();
        for spec in specs {
            events.extend(self.write(&format!("keys.{spec}"), None));
        }
        events
    }
}

/// Whether a keys row is one a filter lists.
pub(super) fn keys_filter(f: KeysFilter, r: &KeyRow) -> bool {
    match f {
        KeysFilter::All => true,
        KeysFilter::Changed => r.binding.changed,
        KeysFilter::Plugins => r.binding.plugin.is_some(),
        KeysFilter::Conflicts => r.conflict,
    }
}
