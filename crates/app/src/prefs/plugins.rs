//! Plugins on the Settings tab: their summaries, a plugin's own page, and
//! what its buttons do.

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
                    ("failed", Tone::Error)
                } else if !p.enabled {
                    ("off", Tone::Muted)
                } else if p.waiting {
                    ("needs your OK", Tone::Warning)
                } else if p.data_only {
                    ("data only", Tone::Badge)
                } else if !p.permissions.processes.is_empty() {
                    ("runs programs", Tone::Warning)
                } else {
                    ("sandboxed", Tone::Success)
                };
                PluginSummary {
                    what: describe(&p),
                    name: p.name,
                    title: p.title,
                    tag: tag.into(),
                    tone,
                }
            })
            .collect();
        Event::PluginList(list)
    }

    pub(crate) fn page_view(&self, name: &str) -> Option<PageView> {
        let p = self.host.plugins().into_iter().find(|p| p.name == name)?;
        let changed = |k: &str| p.defaults.get(k) != p.settings.get(k);
        let mut settings: Vec<_> = p
            .settings
            .iter()
            .map(|(k, v)| (k.clone(), v.clone(), changed(k)))
            .collect();
        settings.sort_by(|a, b| a.0.cmp(&b.0));
        Some(PageView {
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
            processes: p.permissions.processes,
            memory: p.memory,
            busy: p.busy,
            languages: p.languages,
            looks: p.looks,
            settings,
        })
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
            PageAction::Output => events.push(Event::Command("view.output".into())),
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
            PageAction::Setting(plugin, key, value) => {
                if !self.scripted {
                    let path = format!("plugins.{plugin}.{key}");
                    if let Err(err) = self.settings.set(&path, &value.to_string()) {
                        events.push(Event::Error(format!("Can't save {path}: {err}")));
                    }
                    events.extend(self.load_settings());
                }
            }
        }
        events.push(self.plugin_summaries());
        events
    }
}
