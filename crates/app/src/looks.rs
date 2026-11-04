//! The look in use: which packs are stacked, the pickers' lists, and the
//! Theme Packs panel, where packs are switched on and off and put in order
//! as resource packs are.
//!
//! From the bottom: merlin's own look, the colour theme, the icon theme,
//! then the packs in the settings' `packs` list, the first of them on top.

use std::path::PathBuf;
use std::time::SystemTime;

use merlin_core::panel::{Button, Entry, Node, Panel, Tone};
use merlin_core::{Command, Event, ThemeKind};

use crate::packs::{self, Look, Pack};
use crate::{App, modified, theme};

/// The packs panel, which the app shows as if it were a plugin's.
pub const PANEL: &str = "merlin.packs";

/// What the settings choose, and what a picker is showing for now.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    pub colours: Option<String>,
    pub icons: Option<String>,
    /// Extra packs, the first on top.
    pub packs: Vec<String>,
    /// Shown while a picker is open, over the choice.
    pub preview: [Option<String>; 2],
    /// The pack picked in the panel, by its place in `packs`.
    pub selected: Option<usize>,
}

impl App {
    /// The packs there are: merlin's own, then plugins'. Ones that don't
    /// read are left out, with what's wrong.
    pub(crate) fn find_packs(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut found = packs::built_in();
        for (label, file) in self.host.packs() {
            match Pack::read(&label, &file) {
                Ok(pack) => found.push(pack),
                Err(err) => errors.push(format!("{label}: {err:#}")),
            }
        }
        self.packs = found;
        errors
    }

    fn pack(&self, label: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.label == label)
    }

    /// Puts the look together from the choice and any preview.
    pub(crate) fn restack(&mut self) -> Result<(), String> {
        let c = &self.choice;
        let colours = c.preview[0]
            .clone()
            .or_else(|| c.colours.clone())
            .unwrap_or_else(|| theme::DEFAULT_NAME.into());
        let icons = c.preview[1]
            .clone()
            .or_else(|| c.icons.clone())
            .unwrap_or_else(|| packs::LETTERS.into());
        let mut names = vec![theme::DEFAULT_NAME.to_string(), colours, icons];
        names.extend(c.packs.iter().rev().cloned());
        let stack: Vec<&Pack> = names.iter().filter_map(|n| self.pack(n)).collect();
        let look = Look::stack(&stack).map_err(|e| format!("{e:#}"))?;
        self.watched = stack
            .iter()
            .filter_map(|p| p.file.clone())
            .map(|f| {
                let when = modified(&f);
                (f, when)
            })
            .collect();
        self.look = look;
        Ok(())
    }

    /// Reads packs again when one in use changes on disk, so a pack being
    /// written shows its changes on save.
    pub(crate) fn check_packs(&mut self) -> Vec<Event> {
        let changed = self
            .watched
            .iter()
            .any(|(f, when): &(PathBuf, Option<SystemTime>)| modified(f) != *when);
        if !changed {
            return Vec::new();
        }
        let mut events: Vec<Event> = self.find_packs().into_iter().map(Event::Error).collect();
        if let Err(err) = self.restack() {
            events.push(Event::Error(err));
        }
        events
    }

    /// Tells the editor which colour and icon themes there are, and uses
    /// what the settings ask for.
    pub(crate) fn refresh_looks(&mut self) -> Vec<Event> {
        let mut events: Vec<Event> = self.find_packs().into_iter().map(Event::Error).collect();
        if let Err(err) = self.restack() {
            events.push(Event::Error(err));
        }
        for (kind, chosen, fallback) in [
            (
                ThemeKind::Colours,
                &self.choice.colours,
                theme::DEFAULT_NAME,
            ),
            (ThemeKind::Icons, &self.choice.icons, packs::LETTERS),
        ] {
            let themes = self
                .packs
                .iter()
                .filter(|p| match kind {
                    ThemeKind::Colours => p.parts().colours || p.label == theme::DEFAULT_NAME,
                    ThemeKind::Icons => p.parts().icons,
                })
                .map(|p| p.label.clone())
                .collect();
            events.push(Event::Themes {
                kind,
                themes,
                current: chosen.clone().unwrap_or_else(|| fallback.into()),
            });
        }
        events.push(Event::RegisterPlugin {
            plugin: PANEL.into(),
            commands: vec![Command {
                id: "open".into(),
                title: "Preferences: Theme Packs".into(),
                key: None,
            }],
        });
        events
    }

    pub(crate) fn preview(&mut self, kind: ThemeKind, name: String) -> Option<Event> {
        self.choice.preview[kind as usize] = Some(name);
        self.restack().err().map(Event::Error)
    }

    pub(crate) fn choose(&mut self, kind: ThemeKind, name: String) -> Vec<Event> {
        self.choice.preview[kind as usize] = None;
        let key = match kind {
            ThemeKind::Colours => {
                self.choice.colours = Some(name.clone());
                "editor.theme"
            }
            ThemeKind::Icons => {
                self.choice.icons = Some(name.clone());
                "editor.icon_theme"
            }
        };
        let mut events = Vec::new();
        if let Err(err) = self.restack() {
            events.push(Event::Error(err));
        }
        if !self.scripted {
            let value = toml::Value::String(name).to_string();
            if let Err(err) = self.settings.set(key, &value) {
                events.push(Event::Error(format!("Can't save {key}: {err}")));
            }
        }
        events.extend(self.packs_panel(true));
        events
    }

    fn save_packs(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        if let Err(err) = self.restack() {
            events.push(Event::Error(err));
        }
        if !self.scripted {
            let list = toml::Value::Array(
                self.choice
                    .packs
                    .iter()
                    .map(|p| toml::Value::String(p.clone()))
                    .collect(),
            );
            if let Err(err) = self.settings.set("editor.packs", &list.to_string()) {
                events.push(Event::Error(format!("Can't save the packs: {err}")));
            }
        }
        events
    }

    /// A command from the packs panel's own palette entry.
    pub(crate) fn packs_command(&mut self, command: &str) -> Vec<Event> {
        if command != "open" {
            return Vec::new();
        }
        let mut events = self.packs_panel(false);
        events.push(Event::FocusPanel(PANEL.into()));
        events
    }

    /// A click in the packs panel.
    pub(crate) fn packs_click(&mut self, button: &str) -> Vec<Event> {
        let (what, arg) = button.split_once(':').unwrap_or((button, ""));
        let index = arg.parse::<usize>().ok();
        let list = &mut self.choice.packs;
        let mut events = Vec::new();
        match (what, index) {
            ("colours", _) => return vec![Event::Command("theme.pick".into())],
            ("icons", _) => return vec![Event::Command("icons.pick".into())],
            ("done", _) => {
                self.choice.selected = None;
                return vec![Event::SetPanel {
                    plugin: PANEL.into(),
                    panel: None,
                }];
            }
            ("pick", Some(i)) => {
                self.choice.selected = (self.choice.selected != Some(i)).then_some(i);
            }
            ("up", Some(i)) if i > 0 && i < list.len() => {
                list.swap(i, i - 1);
                self.choice.selected = Some(i - 1);
                events = self.save_packs();
            }
            ("down", Some(i)) if i + 1 < list.len() => {
                list.swap(i, i + 1);
                self.choice.selected = Some(i + 1);
                events = self.save_packs();
            }
            ("off", Some(i)) if i < list.len() => {
                list.remove(i);
                self.choice.selected = None;
                events = self.save_packs();
            }
            ("on", _) if !arg.is_empty() && !list.iter().any(|p| p == arg) => {
                // New packs go on top, as they do in a resource pack list.
                list.insert(0, arg.to_string());
                self.choice.selected = Some(0);
                events = self.save_packs();
            }
            _ => {}
        }
        events.extend(self.packs_panel(true));
        events
    }

    /// The packs panel's contents, if it's showing or `force`d.
    pub(crate) fn packs_panel(&self, only_if_open: bool) -> Vec<Event> {
        if only_if_open && !self.editor.has_panel(PANEL) {
            return Vec::new();
        }
        let c = &self.choice;
        let mut nodes = vec![Node::Heading("Colour theme".into())];
        let colours = c
            .colours
            .clone()
            .unwrap_or_else(|| theme::DEFAULT_NAME.into());
        nodes.push(Node::Entry(Entry {
            id: "colours".into(),
            label: colours,
            detail: "change".into(),
            ..Entry::default()
        }));
        nodes.push(Node::Heading("Icon theme".into()));
        let icons = c.icons.clone().unwrap_or_else(|| packs::LETTERS.into());
        nodes.push(Node::Entry(Entry {
            id: "icons".into(),
            label: icons,
            detail: "change".into(),
            ..Entry::default()
        }));
        nodes.push(Node::Heading("Packs in use".into()));
        if c.packs.is_empty() {
            nodes.push(Node::Styled(
                "None yet. Packs stack over the themes; the top one wins where they overlap."
                    .into(),
                Tone::Muted,
            ));
        } else {
            nodes.push(Node::Styled(
                "The top pack wins where packs overlap.".into(),
                Tone::Muted,
            ));
        }
        for (i, label) in c.packs.iter().enumerate() {
            let parts = self.pack(label).map(|p| p.parts().describe());
            nodes.push(Node::Entry(Entry {
                id: format!("pick:{i}"),
                label: label.clone(),
                detail: parts.unwrap_or_else(|| "not installed".into()),
                tone: if c.selected == Some(i) {
                    Tone::Strong
                } else {
                    Tone::Normal
                },
                badge: (i + 1).to_string(),
                badge_tone: Tone::Badge,
                action: Some(('×', format!("off:{i}"))),
                ..Entry::default()
            }));
            if c.selected == Some(i) {
                nodes.push(Node::Buttons(vec![
                    Button {
                        id: format!("up:{i}"),
                        label: "Move up".into(),
                    },
                    Button {
                        id: format!("down:{i}"),
                        label: "Move down".into(),
                    },
                ]));
            }
        }
        let available: Vec<&Pack> = self
            .packs
            .iter()
            .filter(|p| p.file.is_some() && !c.packs.contains(&p.label))
            .collect();
        if !available.is_empty() {
            nodes.push(Node::Heading("Available".into()));
            for p in available {
                nodes.push(Node::Entry(Entry {
                    id: format!("on:{}", p.label),
                    label: p.label.clone(),
                    detail: p.parts().describe(),
                    action: Some(('+', format!("on:{}", p.label))),
                    ..Entry::default()
                }));
            }
        }
        nodes.push(Node::Divider);
        nodes.push(Node::Button(Button {
            id: "done".into(),
            label: "Done".into(),
        }));
        vec![Event::SetPanel {
            plugin: PANEL.into(),
            panel: Some(Panel {
                title: "Theme Packs".into(),
                nodes,
            }),
        }]
    }
}
