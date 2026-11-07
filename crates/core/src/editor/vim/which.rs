//! Which-key: after a pause on a prefix, a card along the bottom of the
//! text says what each next key does. For Vim's prefixes (`g`, `z`, `[`,
//! Ctrl+W, an operator waiting for its motion), its registers and marks,
//! the leader's keys with plugins' among them, and VS Code's Ctrl+K
//! chords.

use super::*;

/// A key under the leader, from merlin or a plugin's `[[keys]]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderKey {
    /// The keys after the leader, such as `f p`.
    pub keys: String,
    /// What it runs; none for a group, which only gathers the keys under
    /// it.
    pub command: Option<String>,
    pub label: String,
}

/// The card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhichKeyView {
    /// The keys so far, such as `g`, `Space f` or `Ctrl+K`.
    pub chord: String,
    pub hint: String,
    pub items: Vec<WhichKeyItem>,
    /// One column with what each holds, as registers have, or a grid.
    pub columns: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhichKeyItem {
    pub key: String,
    pub label: String,
    /// It opens more keys rather than running something.
    pub group: bool,
    /// The plugin it's from, for a chip.
    pub plugin: Option<String>,
    /// What a register or mark holds.
    pub preview: Option<String>,
}

/// merlin's own keys under the leader.
const LEADER: &[(&str, &str, &str)] = &[
    ("f", "", "+ files"),
    ("s", "", "+ search"),
    ("g", "", "+ git"),
    ("l", "", "+ language"),
    ("p", "file.open", "Go to File"),
    ("/", "view.search", "Search the project"),
    ("b", "git.blame", "Blame this line"),
    ("r", "edit.rename", "Rename symbol"),
    ("e", "view.explorer", "Explorer"),
    ("o", "view.outline", "Outline"),
    ("h", "vim.stage_change", "Stage this change"),
    ("a", "edit.quick_fix", "Code actions"),
    ("w", "file.save", "Save"),
    ("t", "terminal.toggle", "Terminal"),
    ("d", "vim.diff", "Diff with HEAD"),
    ("k", "view.hover", "Hover"),
    ("f f", "file.open", "Go to File"),
    ("f r", "file.reopen", "Reopen the last closed"),
    ("f n", "file.new", "New file"),
    ("f s", "file.save", "Save"),
    ("f S", "file.save_as", "Save as…"),
    ("f o", "file.open_folder", "Open a folder"),
    ("f c", "file.close", "Close"),
    ("s s", "view.search", "Search the project"),
    ("s f", "find.open", "Find in the file"),
    ("s r", "find.replace", "Replace in the file"),
    ("s o", "go.symbol", "A symbol in the file"),
    ("s l", "go.line", "Go to a line"),
    ("s c", "palette.open", "Commands"),
    ("g b", "git.blame", "Blame"),
    ("g h", "vim.stage_change", "Stage this change"),
    ("g d", "vim.diff", "Diff with HEAD"),
    ("g n", "go.next_change", "Next change"),
    ("g N", "go.prev_change", "Previous change"),
    ("g s", "view.changes", "Source Control"),
    ("g p", "scm.push", "Push"),
    ("g P", "scm.pull", "Pull"),
    ("l d", "go.definition", "Go to Definition"),
    ("l r", "go.references", "Find References"),
    ("l R", "edit.rename", "Rename symbol"),
    ("l a", "edit.quick_fix", "Code actions"),
    ("l f", "edit.format", "Format the file"),
    ("l k", "view.hover", "Hover"),
    ("l p", "view.problems", "Problems"),
];

/// Vim's keys after `g`, `z`, `[`, `]` and Ctrl+W.
const PREFIXES: &[(char, &[(&str, &str)])] = &[
    (
        'g',
        &[
            ("d", "Go to Definition"),
            ("r", "Find References"),
            ("h", "Hover"),
            ("a", "Code actions"),
            ("c", "+ comment"),
            ("u", "Lower case · motion"),
            ("U", "Upper case · motion"),
            ("~", "Swap case · motion"),
            ("g", "First line"),
            ("v", "Last selection"),
            ("i", "Insert where you left"),
            ("J", "Join, no spaces"),
            ("t", "Next tab"),
            ("T", "Previous tab"),
            ("e", "Back to a word's end"),
            ("_", "Last letter on the line"),
        ],
    ),
    (
        'z',
        &[
            ("z", "This line in the middle"),
            ("t", "This line at the top"),
            ("b", "This line at the bottom"),
            ("o", "Unfold"),
            ("c", "Fold"),
            ("a", "Fold or unfold"),
            ("R", "Unfold everything"),
            ("M", "Fold everything"),
        ],
    ),
    (']', &[("c", "Next change"), ("d", "Next problem")]),
    ('[', &[("c", "Previous change"), ("d", "Previous problem")]),
    (
        '\x17',
        &[
            ("w", "Next group"),
            ("W", "Group before"),
            ("v", "Split right"),
            ("s", "Split down"),
            ("c", "Close"),
            ("q", "Close"),
            ("o", "Close the others"),
            ("h", "Group before"),
            ("l", "Next group"),
        ],
    ),
];

/// What an operator takes next.
const MOTION_KEYS: &[(&str, &str)] = &[
    ("w", "To the next word"),
    ("e", "To the word's end"),
    ("b", "Back a word"),
    ("$", "To the line's end"),
    ("0", "From the line's start"),
    ("^", "From its first letter"),
    ("j", "This line and the next"),
    ("k", "This line and the one before"),
    ("G", "To the last line"),
    ("gg", "To the first line"),
    ("f", "To a letter · then it"),
    ("t", "Up to a letter · then it"),
    ("}", "To the paragraph's end"),
    ("%", "To the matching bracket"),
    ("i", "+ inside"),
    ("a", "+ around"),
];

/// Text objects, after `i` or `a`.
const OBJECT_KEYS: &[(&str, &str)] = &[
    ("w", "Word"),
    ("W", "Word with its punctuation"),
    ("s", "Sentence"),
    ("p", "Paragraph"),
    ("(", "Round brackets"),
    ("{", "Braces"),
    ("[", "Square brackets"),
    ("<", "Angle brackets"),
    ("\"", "Double quotes"),
    ("'", "Single quotes"),
    ("`", "Backticks"),
    ("f", "Function"),
    ("c", "Class"),
];

/// A key as the card shows it.
fn shown(c: char) -> String {
    match c {
        ' ' => "Space".into(),
        '\x17' => "Ctrl+W".into(),
        c if (c as u32) < 27 => format!("Ctrl+{}", ((c as u8) + b'A' - 1) as char),
        c => c.to_string(),
    }
}

fn preview(text: &str) -> String {
    let one: String = text
        .trim_end_matches('\n')
        .replace('\n', "⏎")
        .replace('\t', " ");
    let one = one.trim().to_string();
    if one.chars().count() > 60 {
        format!("{}…", one.chars().take(59).collect::<String>())
    } else {
        one
    }
}

impl Editor {
    /// The leader's keys: merlin's, then plugins'.
    pub(crate) fn leader_keys(&self) -> Vec<(LeaderKey, Option<String>)> {
        LEADER
            .iter()
            .map(|(keys, command, label)| {
                (
                    LeaderKey {
                        keys: keys.to_string(),
                        command: (!command.is_empty()).then(|| command.to_string()),
                        label: label.to_string(),
                    },
                    None,
                )
            })
            .chain(self.plugin_keys.iter().flat_map(|(plugin, keys)| {
                keys.iter().map(move |k| (k.clone(), Some(plugin.clone())))
            }))
            .collect()
    }

    /// A plugin's `[[keys]]`, from its manifest.
    pub(crate) fn register_keys(&mut self, plugin: String, keys: Vec<LeaderKey>) {
        self.plugin_keys.retain(|(p, _)| *p != plugin);
        if !keys.is_empty() {
            self.plugin_keys.push((plugin, keys));
        }
    }

    /// The leader's key, from `vim.leader`.
    pub(super) fn leader(&self) -> char {
        match self.vim_settings.leader.as_str() {
            "space" | "" => ' ',
            other => other.chars().next().unwrap_or(' '),
        }
    }

    /// The keys after the leader so far: a group waits, a key runs. Returns
    /// whether the keys are still the leader's.
    pub(super) fn vim_leader(&mut self, fx: &mut Vec<Effect>) -> bool {
        let leader = self.leader();
        let Some(vim) = self.vim.as_ref() else {
            return false;
        };
        if vim.pending.first() != Some(&leader) || vim.mode() != VimMode::Normal {
            return false;
        }
        let typed: String = vim.pending[1..]
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        if typed.is_empty() {
            return true;
        }
        let keys = self.leader_keys();
        let exact = keys.iter().rev().find(|(k, _)| k.keys == typed).cloned();
        let more = keys
            .iter()
            .any(|(k, _)| k.keys.starts_with(&format!("{typed} ")));
        let vim = self.vim.as_mut().expect("vim");
        match exact {
            Some((key, plugin)) if key.command.is_some() && !more => {
                vim.pending.clear();
                vim.pending_keys.clear();
                let command = key.command.unwrap_or_default();
                match plugin {
                    Some(plugin) => fx.push(Effect::PluginCommand { plugin, command }),
                    None => self.vim_leader_run(&command, fx),
                }
            }
            _ if more || exact.is_some() => {}
            _ => {
                vim.pending.clear();
                vim.pending_keys.clear();
                vim.note = Some(format!("Nothing under {} {typed}", shown(leader)));
            }
        }
        true
    }

    fn vim_leader_run(&mut self, command: &str, fx: &mut Vec<Effect>) {
        match command {
            "vim.stage_change" => {
                let line = self.doc().line_col(self.doc().cursor).0;
                if !self.change_peeking() {
                    self.toggle_change_peek(line);
                }
                if self.change_peeking() {
                    self.stage_change_peek(fx);
                } else {
                    self.vim_note("No change on this line".into());
                }
            }
            "vim.diff" => match self.doc().buffer.path.clone() {
                Some(path) => {
                    let path = path
                        .strip_prefix(&self.root)
                        .map(|p| p.to_path_buf())
                        .unwrap_or(path);
                    fx.push(Effect::OpenDiff(path));
                }
                None => self.vim_note("This file isn't saved yet".into()),
            },
            id => self.run(id, fx),
        }
    }

    /// Whether a prefix is waiting for its next key, so a card could
    /// show.
    fn which_key_prefix(&self) -> bool {
        if self.chord.is_some() {
            return true;
        }
        let Some(vim) = self.vim.as_ref() else {
            return false;
        };
        !vim.pending.is_empty() && vim.line.is_none()
    }

    /// The card shows after this many milliseconds of waiting on a
    /// prefix: which pause, and how long. `None` with nothing waiting.
    pub fn which_key_wait(&self) -> Option<(u64, u64)> {
        let delay = self.vim_settings.which_key_delay;
        (delay > 0 && !self.which_key_shown && self.which_key_prefix())
            .then_some((self.which_key_tick, delay))
    }

    /// The pause went on long enough.
    pub(crate) fn which_key_due(&mut self, tick: u64) {
        if tick == self.which_key_tick && self.which_key_prefix() {
            self.which_key_shown = true;
        }
    }

    /// Each key starts the wait again.
    pub(crate) fn which_key_key(&mut self) {
        self.which_key_tick += 1;
        self.which_key_shown = false;
    }

    pub(crate) fn which_key_view(&self) -> Option<WhichKeyView> {
        if !self.which_key_shown {
            return None;
        }
        let hint = format!(
            "{}Backspace goes up · Esc cancels",
            if self.vim_settings.which_key_delay > 0 {
                format!(
                    "After {:.1} s · ",
                    self.vim_settings.which_key_delay as f32 / 1000.0
                )
            } else {
                String::new()
            }
        );
        let item = |key: &str, label: &str| WhichKeyItem {
            key: key.to_string(),
            label: label.trim_start_matches("+ ").to_string(),
            group: label.starts_with("+ "),
            plugin: None,
            preview: None,
        };
        // VS Code's chords: what the second key does.
        if let Some(first) = self.chord {
            let titles: HashMap<String, String> = self.commands().into_iter().collect();
            let mut items: Vec<WhichKeyItem> = self
                .keymap
                .iter()
                .filter(|b| b.keys.len() == 2 && b.keys[0] == first && !b.command.is_empty())
                .map(|b| {
                    let title = titles.get(&b.command).cloned().unwrap_or(b.command.clone());
                    let label = title
                        .split_once(": ")
                        .map_or(title.as_str(), |(_, t)| t)
                        .to_string();
                    let second = b.keys[1];
                    let key = if second.ctrl && first.ctrl {
                        format!(
                            "Ctrl {}",
                            Key {
                                ctrl: false,
                                ..second
                            }
                        )
                    } else {
                        second.to_string()
                    };
                    item(&key, &label)
                })
                .collect();
            items.dedup_by(|a, b| a.key == b.key);
            return Some(WhichKeyView {
                chord: first.to_string(),
                hint,
                items,
                columns: 2,
            });
        }
        let vim = self.vim.as_ref()?;
        let pending = &vim.pending;
        let leader = self.leader();
        let chord: String = pending
            .iter()
            .map(|c| shown(*c))
            .collect::<Vec<_>>()
            .join(" ");
        // The leader's keys.
        if pending.first() == Some(&leader) && vim.mode() == VimMode::Normal {
            let typed: String = pending[1..]
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let depth = if typed.is_empty() {
                0
            } else {
                typed.split(' ').count()
            };
            let prefix = if typed.is_empty() {
                String::new()
            } else {
                format!("{typed} ")
            };
            let mut items: Vec<WhichKeyItem> = Vec::new();
            for (key, plugin) in self.leader_keys() {
                let Some(rest) = key.keys.strip_prefix(&prefix) else {
                    continue;
                };
                if rest.split(' ').count() != 1 || key.keys.split(' ').count() != depth + 1 {
                    continue;
                }
                let group = key.command.is_none();
                // A later one with the same key wins, as a plugin's does.
                items.retain(|i| i.key != rest);
                items.push(WhichKeyItem {
                    key: rest.to_string(),
                    label: key.label.trim_start_matches("+ ").to_string(),
                    group,
                    plugin,
                    preview: None,
                });
            }
            // Groups first, as the card lists them.
            items.sort_by_key(|i| !i.group);
            return Some(WhichKeyView {
                chord,
                hint,
                items,
                columns: 4,
            });
        }
        let mut i = 0;
        if pending.first() == Some(&'"') {
            i = 2;
        }
        while pending.get(i).is_some_and(|c| c.is_ascii_digit()) {
            i += 1;
        }
        let rest = &pending[i.min(pending.len())..];
        // Registers, after `"`.
        if pending.len() == 1 && pending[0] == '"' {
            let mut names: Vec<char> = vim.registers.keys().copied().collect();
            let order = |c: &char| {
                "\"0123456789-abcdefghijklmnopqrstuvwxyz+/:"
                    .find(*c)
                    .unwrap_or(99)
            };
            if vim.search.is_some() && !names.contains(&'/') {
                names.push('/');
            }
            if !vim.history.is_empty() && !names.contains(&':') {
                names.push(':');
            }
            names.sort_by_key(order);
            let items = names
                .into_iter()
                .map(|c| {
                    let text = match c {
                        '/' => vim.search.as_ref().map(|s| s.0.clone()).unwrap_or_default(),
                        ':' => vim.history.last().cloned().unwrap_or_default(),
                        c => vim
                            .registers
                            .get(&c)
                            .map(|r| r.text.clone())
                            .unwrap_or_default(),
                    };
                    let label = match c {
                        '"' => "last delete or yank",
                        '0' => "last yank",
                        '1' => "last line delete",
                        '-' => "small delete",
                        '+' | '*' => "clipboard",
                        '/' => "last search",
                        ':' => "last command",
                        'q' => "macro",
                        _ => "",
                    };
                    WhichKeyItem {
                        key: c.to_string(),
                        label: label.into(),
                        group: false,
                        plugin: None,
                        preview: Some(preview(&text)),
                    }
                })
                .collect();
            return Some(WhichKeyView {
                chord,
                hint: "' and ` list marks the same way".into(),
                items,
                columns: 1,
            });
        }
        // Macros to play, after `@`.
        if rest == ['@'] {
            let mut names: Vec<char> = vim
                .registers
                .keys()
                .copied()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect();
            names.sort();
            let mut items: Vec<WhichKeyItem> = names
                .into_iter()
                .map(|c| WhichKeyItem {
                    key: c.to_string(),
                    label: String::new(),
                    group: false,
                    plugin: None,
                    preview: Some(preview(&vim.registers[&c].text)),
                })
                .collect();
            if let Some(r) = self.last_macro_register {
                items.insert(
                    0,
                    WhichKeyItem {
                        key: "@".into(),
                        label: format!("@{r} again"),
                        group: false,
                        plugin: None,
                        preview: None,
                    },
                );
            }
            return Some(WhichKeyView {
                chord,
                hint: "q and a letter records one".into(),
                items,
                columns: 1,
            });
        }
        // Marks, after `'` and `` ` ``.
        if matches!(rest, ['\''] | ['`']) {
            let doc = self.doc();
            let mut marks: Vec<(char, usize)> = vim
                .marks
                .iter()
                .filter(|(_, (id, _))| *id == doc.id)
                .map(|(c, (_, at))| (*c, (*at).min(doc.buffer.len())))
                .collect();
            marks.sort();
            let items = marks
                .into_iter()
                .map(|(c, at)| {
                    let line = doc.line_col(at).0;
                    let text: String = doc.line_chars(line).into_iter().collect();
                    WhichKeyItem {
                        key: c.to_string(),
                        label: format!("line {}", line + 1),
                        group: false,
                        plugin: None,
                        preview: Some(preview(&text)),
                    }
                })
                .collect();
            return Some(WhichKeyView {
                chord,
                hint,
                items,
                columns: 1,
            });
        }
        // An operator waiting: its motions, or text objects after i or a.
        if let Ok(Some((_, n))) = operator(rest) {
            let mut j = n;
            while rest.get(j).is_some_and(|c| c.is_ascii_digit()) {
                j += 1;
            }
            let after = &rest[j.min(rest.len())..];
            let list: Vec<WhichKeyItem> = match after {
                [] => {
                    let doubled = rest[n - 1].to_string();
                    std::iter::once(item(&doubled, "The whole line"))
                        .chain(MOTION_KEYS.iter().map(|(k, l)| item(k, l)))
                        .collect()
                }
                ['i' | 'a'] => OBJECT_KEYS.iter().map(|(k, l)| item(k, l)).collect(),
                _ => return None,
            };
            return Some(WhichKeyView {
                chord,
                hint,
                items: list,
                columns: 4,
            });
        }
        if let ['i' | 'a'] = rest
            && vim.mode().visual()
        {
            return Some(WhichKeyView {
                chord,
                hint,
                items: OBJECT_KEYS.iter().map(|(k, l)| item(k, l)).collect(),
                columns: 4,
            });
        }
        let [first] = rest else { return None };
        let (_, keys) = PREFIXES.iter().find(|(c, _)| c == first)?;
        Some(WhichKeyView {
            chord,
            hint,
            items: keys.iter().map(|(k, l)| item(k, l)).collect(),
            columns: 3,
        })
    }
}
