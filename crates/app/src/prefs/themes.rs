//! Colour and icon themes: the pair in use, following the system's light
//! or dark, and the cards' previews.

use super::*;

impl App {
    /// The colour themes for a dark desktop and a light one.
    pub(super) fn theme_pair(&self) -> (String, String) {
        let merged = &self.loaded.merged;
        let dark = settings::editor_text(merged, "theme")
            .unwrap_or(theme::DEFAULT_NAME)
            .to_string();
        let light = settings::editor_text(merged, "theme_light")
            .map(str::to_string)
            .or_else(|| {
                self.previews
                    .iter()
                    .find(|(_, p)| theme::is_light(p))
                    .map(|(l, _)| l.clone())
            })
            .unwrap_or_else(|| dark.clone());
        (dark, light)
    }

    /// The colour theme to use now: with Follow the system, the one for
    /// the desktop's light or dark.
    pub(crate) fn chosen_colours(&self) -> Option<String> {
        let merged = &self.loaded.merged;
        let follow = get(merged, "theme_follows_system")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if follow && self.system_light {
            Some(self.theme_pair().1)
        } else {
            settings::editor_text(merged, "theme").map(str::to_string)
        }
    }

    /// Asks the desktop whether it wants light colours, off the window's
    /// thread, when a theme follows it; and changes the theme if that
    /// changed.
    pub(crate) fn check_system_theme(&mut self) {
        let follow = get(&self.loaded.merged, "theme_follows_system")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !follow || self.scripted {
            return;
        }
        self.jobs.run(crate::jobs::Lane::Ask, move || {
            let light = system_light();
            Box::new(move |app: &mut App| match light {
                Some(light) if light != app.system_light => {
                    app.system_light = light;
                    app.load_settings()
                }
                _ => Vec::new(),
            })
        });
    }

    /// The window previews the theme the arrows or the pointer are on, or
    /// nothing.
    pub(crate) fn preview_card(&mut self, name: Option<String>) -> Vec<Event> {
        if self.choice.preview[ThemeKind::Colours as usize] == name {
            return Vec::new();
        }
        match name {
            Some(name) => self.preview(ThemeKind::Colours, name).into_iter().collect(),
            None => {
                self.choice.preview[ThemeKind::Colours as usize] = None;
                self.restack().err().map(Event::Error).into_iter().collect()
            }
        }
    }

    /// Every colour theme's palette and icon theme's icons, for the
    /// Settings tab's cards.
    pub(crate) fn make_previews(&mut self) {
        let base = self
            .packs
            .iter()
            .find(|p| p.label == theme::DEFAULT_NAME)
            .cloned();
        let stack = |pack: &Pack| -> Option<Look> {
            match &base {
                Some(b) => Look::stack(&[b, pack]).ok(),
                None => Look::stack(&[pack]).ok(),
            }
        };
        self.previews = self
            .packs
            .iter()
            .filter(|p| p.parts().colours || p.label == theme::DEFAULT_NAME)
            .filter_map(|p| Some((p.label.clone(), stack(p)?.palette)))
            .collect();
        let from = |p: &Pack| {
            p.file
                .as_ref()
                .and_then(|f| {
                    self.host
                        .plugins()
                        .into_iter()
                        .find(|i| f.starts_with(&i.dir))
                })
                .map_or_else(|| "built in".to_string(), |i| i.title)
        };
        self.icon_previews = self
            .packs
            .iter()
            .filter(|p| p.parts().icons && !p.parts().colours && !is_ui_only(p))
            .filter_map(|p| Some((p.label.clone(), stack(p)?.icons, from(p))))
            .collect();
    }
}

/// Whether the desktop asks for light colours, as the desktop portal says
/// (`color-scheme` 2), or GNOME's settings do; `None` if neither says.
pub(super) fn system_light() -> Option<bool> {
    use std::process::Command;
    let portal = Command::new("busctl")
        .args([
            "--user",
            "call",
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            "ReadOne",
            "ss",
            "org.freedesktop.appearance",
            "color-scheme",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success());
    // `v u 1`: dark; `v u 2`: light; `v u 0`: no preference.
    if let Some(out) = portal {
        match String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .last()
        {
            Some("1") => return Some(false),
            Some("2") => return Some(true),
            _ => {}
        }
    }
    let gnome = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let scheme = String::from_utf8_lossy(&gnome.stdout);
    Some(scheme.contains("light"))
}
