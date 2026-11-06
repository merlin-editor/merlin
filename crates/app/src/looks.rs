//! The look in use: which packs are stacked, and the pickers' lists. The
//! Settings tab's Appearance page chooses them (`prefs.rs`).
//!
//! From the bottom: merlin's own look, the colour theme, the icon theme,
//! then the packs in the settings' `packs` list, the first of them on top.

use std::path::PathBuf;
use std::time::SystemTime;

use merlin_core::{Event, ThemeKind};

use crate::packs::{self, Look, Pack};
use crate::{App, modified, theme};

/// What the settings choose, and what a picker is showing for now.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    pub colours: Option<String>,
    pub icons: Option<String>,
    /// Extra packs, the first on top.
    pub packs: Vec<String>,
    /// Shown while a picker is open, over the choice.
    pub preview: [Option<String>; 2],
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
        events
    }
}
