//! Plugins on the Settings tab: their summaries, a plugin's own page, and
//! what its buttons do.

use super::view::shown;
use super::*;

impl App {
    /// The Plugins view's list, from what's loaded.
    pub(crate) fn plugin_summaries(&self) -> Event {
        let list = self
            .host
            .plugins()
            .into_iter()
            .map(|p| {
                let (tag, tone) = if p.failed {
                    ("failed".to_string(), Tone::Error)
                } else if !p.enabled {
                    ("disabled".to_string(), Tone::Muted)
                } else if p.waiting {
                    ("needs OK".to_string(), Tone::Warning)
                } else if !p.looks.is_empty() {
                    let n = p.looks.len();
                    (
                        format!("{n} pack{}", if n == 1 { "" } else { "s" }),
                        Tone::Badge,
                    )
                } else if !p.permissions.processes.is_empty() {
                    ("LSP".to_string(), Tone::Badge)
                } else {
                    (String::new(), Tone::Normal)
                };
                PluginSummary {
                    mark: mark(&p),
                    what: describe(&p),
                    name: p.name,
                    title: p.title,
                    tag,
                    tone,
                }
            })
            .collect();
        Event::PluginList(list)
    }

    pub(crate) fn page_view(&self, name: &str) -> Option<PageView> {
        let p = self.host.plugins().into_iter().find(|p| p.name == name)?;
        let rows = self.rows_of(&p);
        let off_here = self.off_here(&p.name);
        let pictures = self.editor.picture_bytes(&p.name);
        Some(PageView {
            mark: mark(&p),
            commands: self.editor.plugin_commands(&p.name),
            installed: self.install_link(&p).is_some(),
            uninstall_armed: self.uninstall_armed.as_deref() == Some(name),
            running: p
                .processes
                .iter()
                .map(|(prog, pid)| (prog.clone(), *pid, self.memory.of(*pid)))
                .collect(),
            name: p.name,
            title: p.title,
            dir: p.dir,
            api: p.api,
            data_only: p.data_only,
            enabled: p.enabled,
            waiting: p.waiting,
            dormant: p.dormant,
            failed: p.failed,
            read: p.permissions.editor_read,
            write: p.permissions.editor_write,
            workspace_read: p.permissions.workspace_read,
            terminal: p.permissions.terminal,
            processes: p.permissions.processes,
            memory: p.memory,
            pictures,
            busy: p.busy,
            languages: p.languages,
            looks: p.looks,
            off_here,
            rows,
        })
    }

    /// A plugin's settings as rows, by its name.
    pub(super) fn plugin_rows(&self, name: &str) -> Vec<Row> {
        self.host
            .plugins()
            .into_iter()
            .find(|p| p.name == name)
            .map_or_else(Vec::new, |p| self.rows_of(&p))
    }

    /// Every plugin's settings, a section each, for a search.
    pub(super) fn plugin_sections(&self) -> Vec<Section> {
        self.host
            .plugins()
            .into_iter()
            .map(|p| Section {
                category: Category::Plugins,
                rows: self.rows_of(&p),
                title: p.title,
            })
            .filter(|s| !s.rows.is_empty())
            .collect()
    }

    /// A plugin's settings as rows: each with its control, and what the
    /// user's file and the workspace's say.
    fn rows_of(&self, p: &PluginInfo) -> Vec<Row> {
        let Loaded {
            user, workspace, ..
        } = &self.loaded;
        rows_of(p, user, workspace, self.settings_ui.workspace)
    }

    /// The plugins turned off: in your settings, and in the workspace's.
    pub(crate) fn off_lists(&self) -> (Vec<String>, Vec<String>) {
        let Loaded {
            user, workspace, ..
        } = &self.loaded;
        (
            settings::editor_list(user, "disabled_plugins"),
            settings::editor_list(workspace, "disabled_plugins"),
        )
    }

    /// Whether a plugin is off in this workspace while on for you.
    pub(crate) fn off_here(&self, name: &str) -> bool {
        let (user, workspace) = self.off_lists();
        off_here(&user, &workspace, name)
    }

    /// Puts a plugin in or out of the list of those turned off, in your
    /// settings or the workspace's.
    fn list_off(&mut self, workspace: bool, name: &str, off: bool) -> Vec<Event> {
        if self.scripted {
            return Vec::new();
        }
        let (user, here) = self.off_lists();
        let Some(list) = with_off(if workspace { here } else { user }, name, off) else {
            return Vec::new();
        };
        let value = toml::Value::Array(list.into_iter().map(toml::Value::String).collect());
        let saved = if value.as_array().is_some_and(|a| a.is_empty()) {
            self.settings
                .remove_in(workspace, "editor.disabled_plugins")
        } else {
            self.settings
                .set_in(workspace, "editor.disabled_plugins", &value.to_string())
        };
        match saved {
            // Read again, so the lists and the pages agree.
            Ok(()) => self.load_settings(),
            Err(err) => vec![Event::Error(format!("Can't save that: {err}"))],
        }
    }

    /// The link `merlin plugin install` made for a plugin, if it has one.
    pub(super) fn install_link(&self, p: &PluginInfo) -> Option<PathBuf> {
        let link = settings::plugins_dir()?.join(&p.name);
        let target = link.read_link().ok()?;
        (target.canonicalize().ok()? == p.dir.canonicalize().ok()?).then_some(link)
    }

    pub(crate) fn page_action(&mut self, action: PageAction) -> Vec<Event> {
        let mut events = Vec::new();
        match action {
            // Off everywhere goes in your settings; on, it's on here too.
            PageAction::Enable(name, on) => {
                events.extend(self.host.set_enabled(&name, on));
                events.extend(self.list_off(false, &name, !on));
                if on {
                    events.extend(self.list_off(true, &name, false));
                }
            }
            PageAction::EnableHere(name, on) => {
                let off_everywhere = self.off_lists().0.contains(&name);
                events.extend(self.host.set_enabled(&name, on && !off_everywhere));
                events.extend(self.list_off(true, &name, !on));
            }
            PageAction::Folder(name) => {
                if let Some(p) = self.host.plugins().into_iter().find(|p| p.name == name) {
                    crate::open_folder(&p.dir);
                }
            }
            PageAction::Reload(name) => events.extend(self.host.reload(&name)),
            PageAction::Output(name) => events.push(Event::ShowOutput(name)),
            PageAction::Uninstall(name) => {
                if self.uninstall_armed.as_deref() != Some(name.as_str()) {
                    // Its page shows, asking again, as from its menu.
                    self.settings_ui.plugin = Some(name.clone());
                    self.uninstall_armed = Some(name);
                    return events;
                }
                self.uninstall_armed = None;
                let Some(info) = self.host.plugins().into_iter().find(|p| p.name == name) else {
                    return events;
                };
                let packaged = settings::plugins_dir()
                    .filter(|plugins| merlin_plugins::package::is_packaged(&info.dir, plugins));
                if packaged.is_some() {
                    // Installed from a package: its folder is the plugin's
                    // own, so it goes, to the bin where it can come back.
                    events.extend(self.host.unload(&name));
                    match trash::delete(&info.dir) {
                        Ok(()) => {
                            let data = merlin_plugins::Config::standard().data_dir;
                            let _ = merlin_plugins::package::record(&data, &name, None);
                            events.push(Event::Notify(format!(
                                "Uninstalled {}. Its folder is in the bin.",
                                info.title
                            )));
                            events.extend(self.refresh_looks());
                        }
                        Err(err) => {
                            events.push(Event::Error(format!("Can't uninstall: {err}")));
                        }
                    }
                    events.push(self.plugin_summaries());
                    return events;
                }
                match self.install_link(&info).map(std::fs::remove_file) {
                    Some(Ok(())) => {
                        events.extend(self.host.unload(&name));
                        events.push(Event::Notify(format!(
                            "Uninstalled {}. Its files are still in {}.",
                            info.title,
                            info.dir.display()
                        )));
                        events.extend(self.refresh_looks());
                    }
                    Some(Err(err)) => events.push(Event::Error(format!("Can't uninstall: {err}"))),
                    None => events.push(Event::Error(format!(
                        "{} wasn't installed with merlin plugin install, so there's nothing to remove",
                        info.title
                    ))),
                }
            }
        }
        events.push(self.plugin_summaries());
        events
    }
}

/// Whether a plugin is off in the workspace's list and not in yours.
fn off_here(user: &[String], workspace: &[String], name: &str) -> bool {
    workspace.iter().any(|n| n == name) && !user.iter().any(|n| n == name)
}

/// A list of plugins turned off, with `name` in or out; none when it's
/// already so.
fn with_off(mut list: Vec<String>, name: &str, off: bool) -> Option<Vec<String>> {
    if list.iter().any(|n| n == name) == off {
        return None;
    }
    list.retain(|n| n != name);
    if off {
        list.push(name.to_string());
    }
    Some(list)
}

/// A plugin's tile, as its manifest gives it.
pub(crate) fn mark(p: &merlin_plugins::PluginInfo) -> (String, merlin_core::style::Hue) {
    merlin_core::plugin_mark(
        &p.title,
        p.badge.as_deref(),
        p.hue.as_deref(),
        !p.looks.is_empty(),
    )
}

/// A plugin's settings as rows, from the user's file and the workspace's,
/// for the User tab or the Workspace tab.
pub(super) fn rows_of(
    p: &PluginInfo,
    user: &toml::Table,
    workspace: &toml::Table,
    on_workspace: bool,
) -> Vec<Row> {
    p.schema
        .iter()
        .map(|spec| {
            let key = format!("plugins.{}.{}", p.name, spec.key);
            let locked = spec.user_only && on_workspace;
            let scope = if on_workspace && !locked {
                workspace
            } else {
                user
            };
            let in_workspace = !spec.user_only && get(workspace, &key).is_some();
            // The file's value, or yours under the workspace's, or the
            // default.
            let value = get(scope, &key)
                .or_else(|| get(user, &key))
                .unwrap_or(&spec.default);
            let label_of = |v: &toml::Value| match &spec.kind {
                SettingKind::Choice(choices) => {
                    let v = shown(v);
                    choices
                        .iter()
                        .find(|(value, _)| *value == v)
                        .map_or(v, |(_, label)| label.clone())
                }
                _ => match v {
                    toml::Value::Array(items) if items.is_empty() => "none".into(),
                    toml::Value::Array(items) => {
                        items.iter().map(shown).collect::<Vec<_>>().join(", ")
                    }
                    toml::Value::String(s) if s.is_empty() => "not set".into(),
                    _ => shown(v),
                },
            };
            let yours = (on_workspace && in_workspace)
                .then(|| label_of(get(user, &key).unwrap_or(&spec.default)));
            let control = match &spec.kind {
                SettingKind::Flag => Control::Toggle(value.as_bool().unwrap_or(false)),
                SettingKind::Choice(choices) => Control::Choice(choices.clone(), shown(value)),
                SettingKind::List => Control::List(
                    value
                        .as_array()
                        .map(|a| a.iter().map(shown).collect())
                        .unwrap_or_default(),
                ),
                SettingKind::Text => Control::Field(shown(value), FieldKind::Text),
                SettingKind::Path => Control::Field(shown(value), FieldKind::Path),
                SettingKind::Secret => {
                    let set = p.secrets.contains(&spec.key);
                    Control::Field(if set { "set" } else { "" }.into(), FieldKind::Secret)
                }
                SettingKind::Integer { min, max } => {
                    Control::Field(shown(value), FieldKind::Integer(*min, *max))
                }
                SettingKind::Float { min, max } => {
                    Control::Field(shown(value), FieldKind::Float(*min, *max))
                }
            };
            let about = if spec.description.is_empty() && spec.user_only {
                "Set for you, not the workspace.".to_string()
            } else {
                spec.description.clone()
            };
            Row {
                label: spec.title.clone(),
                about,
                changed: !locked
                    && (get(scope, &key).is_some()
                        || (spec.kind == SettingKind::Secret && p.secrets.contains(&spec.key))),
                in_workspace,
                note: None,
                yours,
                user_only: spec.user_only,
                locked,
                control,
                key,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{off_here, with_off};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_plugin_is_off_here_only_when_the_workspace_says_and_you_dont() {
        assert!(off_here(&[], &names(&["python"]), "python"));
        assert!(!off_here(
            &names(&["python"]),
            &names(&["python"]),
            "python"
        ));
        assert!(!off_here(&[], &[], "python"));
    }

    #[test]
    fn turning_off_adds_once_and_turning_on_takes_out() {
        assert_eq!(
            with_off(names(&["go"]), "rust", true),
            Some(names(&["go", "rust"]))
        );
        assert_eq!(with_off(names(&["go", "rust"]), "rust", true), None);
        assert_eq!(
            with_off(names(&["go", "rust"]), "rust", false),
            Some(names(&["go"]))
        );
        assert_eq!(with_off(names(&["go"]), "rust", false), None);
    }
}
