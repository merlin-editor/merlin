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
            failed: p.failed,
            read: p.permissions.editor_read,
            write: p.permissions.editor_write,
            workspace_read: p.permissions.workspace_read,
            terminal: p.permissions.terminal,
            processes: p.permissions.processes,
            memory: p.memory,
            busy: p.busy,
            languages: p.languages,
            looks: p.looks,
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

    /// The link `merlin plugin install` made for a plugin, if it has one.
    pub(super) fn install_link(&self, p: &PluginInfo) -> Option<PathBuf> {
        let link = settings::plugins_dir()?.join(&p.name);
        let target = link.read_link().ok()?;
        (target.canonicalize().ok()? == p.dir.canonicalize().ok()?).then_some(link)
    }

    pub(crate) fn page_action(&mut self, action: PageAction) -> Vec<Event> {
        let mut events = Vec::new();
        match action {
            PageAction::Enable(name, on) => {
                events.extend(self.host.set_enabled(&name, on));
                if !self.scripted {
                    let mut off =
                        settings::editor_list(&self.settings.load().0, "disabled_plugins");
                    off.retain(|n| *n != name);
                    if !on {
                        off.push(name.clone());
                    }
                    let list =
                        toml::Value::Array(off.into_iter().map(toml::Value::String).collect());
                    if let Err(err) = self
                        .settings
                        .set("editor.disabled_plugins", &list.to_string())
                    {
                        events.push(Event::Error(format!("Can't save that: {err}")));
                    }
                }
            }
            PageAction::Reload(name) => events.extend(self.host.reload(&name)),
            PageAction::Output(name) => events.push(Event::ShowOutput(name)),
            PageAction::Uninstall(name) => {
                if self.uninstall_armed.as_deref() != Some(name.as_str()) {
                    self.uninstall_armed = Some(name);
                    return events;
                }
                self.uninstall_armed = None;
                let Some(info) = self.host.plugins().into_iter().find(|p| p.name == name) else {
                    return events;
                };
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
