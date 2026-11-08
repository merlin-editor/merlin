//! Loads plugins as WebAssembly components and calls them.
//!
//! A plugin can only call the functions in `wit/merlin.wit`, plus a WASI
//! context with no files, environment or network. It never touches editor
//! state: it reads a [`Snapshot`] taken before the call, and what it asks for
//! comes back as [`Event`]s for the editor to apply. Each call has a fuel
//! budget, and a plugin that traps or runs out is disabled, not the editor.
//!
//! The one way out of the sandbox is spawning a program the manifest lists.
//! Its output is read on background threads and delivered to the plugin by
//! [`PluginHost::poll_processes`], between editor events.
//!
//! The first time a plugin asks for permissions, it shows a card in its
//! panel instead of loading, and loads once the user allows it. The grant
//! is remembered until the plugin asks for more.

mod manifest;
mod process;
mod secrets;
mod state;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use merlin_core::{
    Button, Command, Completion, Diagnostic, Event, LogLevel, Node, Panel, PickItem, Severity, Tone,
};
use serde::{Deserialize, Serialize};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use process::{Output, Processes};
use secrets::Secrets;
use state::*;

pub use manifest::*;

mod bindings {
    wasmtime::component::bindgen!({ path: "../../wit", world: "plugin" });
}

use bindings::Plugin as Bindings;
use bindings::merlin::plugin::{host, types};

/// The API version this host implements. A manifest names the version it was
/// built for and loads only on an exact match while the API is 0.x.
pub const API_VERSION: &str = "0.9";

/// Roughly six billion instructions per call, about a second. It stops a
/// plugin stuck in a loop, and leaves room for a language plugin to colour
/// a whole 512 KB file the first time it opens, which takes up to 3.5
/// billion (about half a second) for dense code.
const FUEL_PER_CALL: u64 = 6_000_000_000;

/// Process messages delivered per poll, so a chatty process can't starve
/// the editor of input.
const MESSAGES_PER_POLL: usize = 256;

pub struct Config {
    /// Where remembered permissions and file-backed secrets are kept.
    pub data_dir: PathBuf,
    /// Ask before granting a plugin's permissions the first time. When off,
    /// they're granted as the manifest says and nothing is remembered.
    pub ask: bool,
    /// Keep secrets in the system keyring when `secret-tool` is installed.
    pub keyring: bool,
}

impl Config {
    /// `$XDG_DATA_HOME/merlin` (`~/.local/share/merlin`), asking, with
    /// the keyring.
    pub fn standard() -> Self {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir);
        Self {
            data_dir: data.join("merlin"),
            ask: true,
            keyring: true,
        }
    }
}

/// A change to the open files, for plugins' `on-buffer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BufferEvent {
    Switched,
    Changed,
    Saved(String),
}

/// What a plugin call can see of the editor.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub path: Option<String>,
    /// Shared with the editor, not copied: a call that doesn't read it
    /// costs nothing.
    pub text: merlin_core::buffer::TextSnapshot,
    pub root: PathBuf,
    /// Line and UTF-16 column.
    pub cursor: (u32, u32),
    pub selection: Option<Selection>,
    /// The lines changed since git's copy, as first and last line, or
    /// none when the file isn't in git.
    pub changed: Option<Vec<(u32, u32)>>,
}

/// Selected text, and where it starts and ends as lines and UTF-16 columns.
#[derive(Clone, Debug)]
pub struct Selection {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub text: String,
}

impl Snapshot {
    /// A path a plugin gave, relative to the root when it's inside it, as
    /// the editor names files.
    fn local(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        path.strip_prefix(&self.root).unwrap_or(path).to_path_buf()
    }
}

struct Instance {
    store: Store<State>,
    bindings: Bindings,
    /// `on-setting-changed`, which plugins built before it don't export.
    setting_changed: Option<wasmtime::component::TypedFunc<(String,), ()>>,
}

struct Loaded {
    manifest: Manifest,
    /// The plugin's folder, which its permissions are remembered by.
    dir: String,
    wasm: PathBuf,
    modified: Option<SystemTime>,
    /// None while waiting for the user to allow its permissions.
    instance: Option<Instance>,
    /// Bumped on every reload, so output from an old instance's processes
    /// isn't delivered to the new one.
    generation: u64,
    failed: bool,
    /// Its last reload failed, so the old code runs on.
    failed_reload: bool,
    /// Turned off by the user: loaded, but not running.
    disabled: bool,
    /// Time spent in the plugin's calls since it loaded.
    busy: Duration,
    /// Adds languages only: no commands, processes, debugger or symbol
    /// search, so it has no panel and only cares about files of its own
    /// languages.
    pure_language: bool,
}

impl Loaded {
    fn handles_path(&self, path: Option<&str>) -> bool {
        let Some((_, ext)) = path.and_then(|p| p.rsplit_once('.')) else {
            return false;
        };
        self.manifest
            .languages
            .iter()
            .any(|l| l.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    }
}

/// What a plugin's page shows about it.
#[derive(Clone, Debug, Default)]
pub struct PluginInfo {
    pub name: String,
    pub title: String,
    pub description: Option<String>,
    /// Its tile's letters and colour, as its manifest gives them.
    pub badge: Option<String>,
    pub hue: Option<String>,
    /// Where it was loaded from.
    pub dir: PathBuf,
    pub api: String,
    /// It adds data only, and runs nothing.
    pub data_only: bool,
    pub enabled: bool,
    /// Waiting for the user to allow its permissions.
    pub waiting: bool,
    pub failed: bool,
    pub permissions: Permissions,
    pub languages: Vec<String>,
    /// Themes and packs, by name.
    pub looks: Vec<String>,
    /// Its settings with the user's over its defaults.
    pub settings: toml::Table,
    /// Its manifest's defaults, to tell what the user changed.
    pub defaults: toml::Table,
    /// Its settings as the Settings page shows them: the manifest's, then
    /// the ones every plugin that runs has.
    pub schema: Vec<SettingSpec>,
    /// Its secret settings that are stored, by key.
    pub secrets: Vec<String>,
    /// Bytes of memory its instance has.
    pub memory: usize,
    pub busy: Duration,
    /// Programs it's running, with their process ids.
    pub processes: Vec<(String, u32)>,
}

pub struct PluginHost {
    engine: Engine,
    linker: Linker<State>,
    plugins: Vec<Loaded>,
    output_tx: Sender<Output>,
    output_rx: Receiver<Output>,
    /// Called from background threads when process output is queued, so a
    /// frontend that sleeps between events knows to poll.
    waker: Option<Waker>,
    /// Coalesces wakeups across process reader threads until the next poll.
    woken: Arc<AtomicBool>,
    config: Config,
    secrets: Arc<Secrets>,
    /// `[plugins]` from the user's settings, and from the workspace's.
    user_settings: toml::Table,
    workspace_settings: toml::Table,
    /// Plugins the user turned off, by name, which load without running.
    disabled: Vec<String>,
}

const ALLOW: &str = "merlin:allow";
const DENY: &str = "merlin:deny";

/// Wakes the frontend from another thread.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

impl PluginHost {
    /// Wakes the frontend when processes of plugins loaded after this have
    /// output waiting.
    pub fn set_waker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.waker = Some(Arc::new(wake));
    }

    pub fn new(config: Config) -> Result<Self> {
        let mut wasm_config = wasmtime::Config::new();
        wasm_config.consume_fuel(true);
        let engine = Engine::new(&wasm_config).map_err(wt)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(wt)?;
        Bindings::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s).map_err(wt)?;
        let (output_tx, output_rx) = channel();
        let secrets = Arc::new(Secrets::new(
            config.data_dir.join("secrets"),
            config.keyring,
        ));
        Ok(Self {
            engine,
            linker,
            plugins: Vec::new(),
            output_tx,
            output_rx,
            waker: None,
            woken: Arc::new(AtomicBool::new(false)),
            config,
            secrets,
            user_settings: toml::Table::new(),
            workspace_settings: toml::Table::new(),
            disabled: Vec::new(),
        })
    }

    /// Loads the plugin in `dir` and returns the events that register it,
    /// or that show the permission card if the user hasn't allowed it yet.
    pub fn load(&mut self, dir: &Path) -> Result<Vec<Event>> {
        let manifest = manifest(dir)?;
        let valid = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if manifest.name.is_empty() || !manifest.name.chars().all(valid) {
            bail!(
                "plugin name {:?} must be lowercase letters, digits, - and _",
                manifest.name
            );
        }
        if manifest.api != API_VERSION {
            bail!(
                "{} needs plugin API {}, this editor has {API_VERSION}",
                manifest.name,
                manifest.api
            );
        }
        if self
            .plugins
            .iter()
            .any(|p| p.manifest.name == manifest.name)
        {
            bail!("a plugin named {} is already loaded", manifest.name);
        }
        let folder = dir.to_path_buf();
        let dir = dir
            .canonicalize()
            .unwrap_or_else(|_| dir.to_path_buf())
            .display()
            .to_string();
        // Data only: nothing runs, so there's nothing to allow.
        let Some(component) = &manifest.component else {
            if !manifest.permissions.is_empty() {
                bail!(
                    "{} asks for permissions but has no component to use them",
                    manifest.name
                );
            }
            let events = manifest_events(&manifest);
            self.plugins.push(Loaded {
                manifest,
                dir,
                modified: None,
                wasm: PathBuf::new(),
                instance: None,
                generation: 0,
                failed: false,
                failed_reload: false,
                disabled: false,
                busy: Duration::ZERO,
                pure_language: false,
            });
            return Ok(events);
        };
        let wasm = folder.join(component);
        let ask = self.config.ask
            && !manifest.permissions.is_empty()
            && !self.grants().covers(&dir, &manifest.permissions);
        let off = self.disabled.contains(&manifest.name);
        let instance = if ask || off {
            None
        } else {
            Some(self.instantiate(&manifest, &wasm, 0)?)
        };
        self.plugins.push(Loaded {
            manifest,
            dir,
            modified: modified(&wasm),
            wasm,
            instance,
            generation: 0,
            failed: false,
            failed_reload: false,
            disabled: off,
            busy: Duration::ZERO,
            pure_language: false,
        });
        let index = self.plugins.len() - 1;
        if off {
            return Ok(Vec::new());
        }
        if ask {
            let plugin = &self.plugins[index].manifest;
            return Ok(vec![
                Event::SetPanel {
                    plugin: plugin.name.clone(),
                    panel: Some(permission_card(plugin)),
                },
                Event::FocusPanel(plugin.name.clone()),
            ]);
        }
        Ok(self.activate(index, None))
    }

    /// Every loaded plugin's themes and packs: name and file.
    pub fn packs(&self) -> Vec<(String, PathBuf)> {
        self.plugins
            .iter()
            .flat_map(|p| {
                p.manifest
                    .themes
                    .iter()
                    .chain(&p.manifest.packs)
                    .map(|t| (t.label.clone(), Path::new(&p.dir).join(&t.file)))
            })
            .collect()
    }

    fn grants(&self) -> Grants {
        read_grants(&self.config.data_dir)
    }

    /// Answers a permission card: loads the plugin and remembers the grant,
    /// or drops it.
    fn answer(&mut self, index: usize, allow: bool) -> Vec<Event> {
        let name = self.plugins[index].manifest.name.clone();
        if !allow {
            let plugin = self.plugins.remove(index);
            return vec![
                Event::SetPanel {
                    plugin: name,
                    panel: None,
                },
                Event::Notify(format!("{} was not loaded", plugin.manifest.title)),
            ];
        }
        let plugin = &self.plugins[index];
        let instance = match self.instantiate(&plugin.manifest, &plugin.wasm, 0) {
            Ok(instance) => instance,
            Err(err) => {
                self.plugins.remove(index);
                return vec![Event::PluginFailed {
                    plugin: name,
                    message: format!("{err:#}"),
                }];
            }
        };
        let saved = save_grant(
            &self.config.data_dir,
            &plugin.dir,
            &plugin.manifest.permissions,
        );
        self.plugins[index].instance = Some(instance);
        let mut events = self.activate(index, None);
        if let Err(err) = saved {
            events.push(Event::Notify(format!(
                "Couldn't remember the permission: {err:#}"
            )));
        }
        events
    }

    /// Compiles a component, or loads it compiled from the cache in the
    /// data folder, since compiling every plugin at every start is slow.
    /// The cache is keyed by the component's bytes and this build of the
    /// editor, so a rebuilt plugin or editor compiles afresh and drops its
    /// older cached builds.
    fn compile(&self, name: &str, wasm: &Path) -> Result<Component> {
        use std::hash::{Hash, Hasher};
        let bytes = fs::read(wasm)?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        std::env::current_exe()
            .ok()
            .and_then(|e| fs::metadata(e).ok()?.modified().ok())
            .hash(&mut hasher);
        let hash = hasher.finish();
        let dir = self.config.data_dir.join("compiled");
        let prefix = format!("{name}-");
        let cached_name = format!("{prefix}{hash:016x}.cwasm");
        let cached = dir.join(&cached_name);
        if cached.exists() {
            // Safe as long as the file is what `serialize` wrote for this
            // engine: it's in the user's own data folder, named by the
            // component and the editor that compiled it.
            if let Ok(component) = unsafe { Component::deserialize_file(&self.engine, &cached) } {
                return Ok(component);
            }
        }
        let component = Component::new(&self.engine, &bytes).map_err(wt)?;
        // Written aside and renamed into place: another editor may have the
        // file mapped, and changing it under them would crash them.
        if let Ok(compiled) = component.serialize() {
            let aside = dir.join(format!("{prefix}{hash:016x}.{}.tmp", std::process::id()));
            if fs::create_dir_all(&dir)
                .and_then(|()| fs::write(&aside, compiled))
                .and_then(|()| fs::rename(&aside, &cached))
                .is_ok()
                && let Ok(entries) = fs::read_dir(&dir)
            {
                for entry in entries.flatten() {
                    let file = entry.file_name();
                    let file = file.to_string_lossy();
                    if file != cached_name && stale_build(&file, name) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
        Ok(component)
    }

    fn instantiate(&self, manifest: &Manifest, wasm: &Path, generation: u64) -> Result<Instance> {
        let component = self
            .compile(&manifest.name, wasm)
            .with_context(|| format!("loading {}", wasm.display()))?;
        let state = State {
            wasi: WasiCtx::builder().build(),
            table: ResourceTable::new(),
            name: manifest.name.clone(),
            permissions: manifest.permissions.clone(),
            snapshot: Snapshot::default(),
            events: Vec::new(),
            processes: Processes::new(
                manifest.name.clone(),
                generation,
                self.output_tx.clone(),
                self.waker.clone(),
                self.woken.clone(),
            ),
            secrets: self.secrets.clone(),
            settings: plugin_settings(manifest, &self.user_settings, &self.workspace_settings),
            limits: Limits::default(),
            debug_start: None,
            breakpoints: Vec::new(),
            symbol_query: String::new(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(FUEL_PER_CALL).map_err(wt)?;
        let instance = self
            .linker
            .instantiate(&mut store, &component)
            .map_err(wt)
            .with_context(|| format!("instantiating {}", manifest.name))?;
        let bindings = Bindings::new(&mut store, &instance)
            .map_err(wt)
            .with_context(|| format!("instantiating {}", manifest.name))?;
        let setting_changed = instance
            .get_typed_func::<(String,), ()>(&mut store, "on-setting-changed")
            .ok();
        Ok(Instance {
            store,
            bindings,
            setting_changed,
        })
    }

    fn activate(&mut self, index: usize, state: Option<Vec<u8>>) -> Vec<Event> {
        let plugin = self.plugins[index].manifest.name.clone();
        let (commands, mut events) = self.call(index, Snapshot::default(), |b, s| {
            b.call_activate(s, state.as_deref())
        });
        if let Some(commands) = commands {
            let m = &self.plugins[index].manifest;
            self.plugins[index].pure_language = !m.languages.is_empty()
                && commands.is_empty()
                && m.permissions.processes.is_empty()
                && !m.permissions.debugger
                && m.symbols.is_none();
            events.insert(
                0,
                Event::RegisterPlugin {
                    plugin,
                    commands: commands
                        .into_iter()
                        .map(|c| Command {
                            id: c.id,
                            title: c.title,
                            key: c.key,
                        })
                        .collect(),
                },
            );
            for (i, event) in manifest_events(&self.plugins[index].manifest)
                .into_iter()
                .enumerate()
            {
                events.insert(1 + i, event);
            }
        }
        events
    }

    /// Calls into a plugin and collects the events it asked for. A trap
    /// disables the plugin and stops its processes until it is reloaded.
    fn call<R>(
        &mut self,
        index: usize,
        snapshot: Snapshot,
        f: impl FnOnce(&Bindings, &mut Store<State>) -> wasmtime::Result<R>,
    ) -> (Option<R>, Vec<Event>) {
        let plugin = &mut self.plugins[index];
        let Some(instance) = plugin.instance.as_mut().filter(|_| !plugin.failed) else {
            return (None, Vec::new());
        };
        let store = &mut instance.store;
        store.data_mut().snapshot = snapshot;
        let _ = store.set_fuel(FUEL_PER_CALL);
        let started = std::time::Instant::now();
        let result = f(&instance.bindings, store);
        plugin.busy += started.elapsed();
        let mut events = std::mem::take(&mut store.data_mut().events);
        match result {
            Ok(value) => (Some(value), events),
            Err(err) => {
                plugin.failed = true;
                store.data_mut().processes.kill_all();
                // Out of fuel says nothing a backtrace would add.
                let message =
                    if err.downcast_ref::<wasmtime::Trap>() == Some(&wasmtime::Trap::OutOfFuel) {
                        "it ran too long and was stopped. Turn it off and on in Settings › \
                     Plugins to start it again."
                            .to_string()
                    } else {
                        format!("{err:#}")
                    };
                events.push(Event::PluginFailed {
                    plugin: plugin.manifest.name.clone(),
                    message,
                });
                (None, events)
            }
        }
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.plugins.iter().position(|p| p.manifest.name == name)
    }

    /// Takes new settings for plugins: the `[plugins]` table of the user's
    /// settings, and the workspace's, which wins except for settings only
    /// the user sets. A running plugin hears which of its own changed.
    pub fn set_settings(
        &mut self,
        user: toml::Table,
        workspace: toml::Table,
        snapshot: &Snapshot,
    ) -> Vec<Event> {
        self.user_settings = user;
        self.workspace_settings = workspace;
        let mut events = Vec::new();
        for i in 0..self.plugins.len() {
            let plugin = &mut self.plugins[i];
            let settings = plugin_settings(
                &plugin.manifest,
                &self.user_settings,
                &self.workspace_settings,
            );
            let Some(instance) = &mut plugin.instance else {
                continue;
            };
            let old = std::mem::replace(&mut instance.store.data_mut().settings, settings);
            let Some(func) = instance.setting_changed else {
                continue;
            };
            let new = &instance.store.data().settings;
            let mut changed: Vec<String> = Vec::new();
            changed_keys(&old, new, "", &mut changed);
            for key in changed {
                let (_, mut more) =
                    self.call(i, snapshot.clone(), |_, store| func.call(store, (key,)));
                events.append(&mut more);
            }
        }
        events
    }

    /// Stores one of a plugin's secret settings, or forgets it given none,
    /// as its row in Settings does. The plugin hears it changed.
    pub fn set_secret(
        &mut self,
        plugin: &str,
        key: &str,
        value: Option<&str>,
        snapshot: &Snapshot,
    ) -> Result<Vec<Event>, String> {
        let i = self.index(plugin).ok_or("no such plugin")?;
        match value {
            Some(value) => self.secrets.set(plugin, key, value)?,
            None => self.secrets.delete(plugin, key),
        }
        let Some(func) = self.plugins[i]
            .instance
            .as_ref()
            .and_then(|i| i.setting_changed)
        else {
            return Ok(Vec::new());
        };
        let key = key.to_string();
        Ok(self
            .call(i, snapshot.clone(), |_, store| func.call(store, (key,)))
            .1)
    }

    /// Gives a debugger plugin what `debug-start` and `breakpoints` read,
    /// before a debug command. `start` replaces what it had when given.
    pub fn set_debug(
        &mut self,
        plugin: &str,
        start: Option<merlin_core::DebugStart>,
        breakpoints: Vec<merlin_core::Breakpoint>,
    ) {
        let Some(i) = self.index(plugin) else {
            return;
        };
        if let Some(instance) = &mut self.plugins[i].instance {
            let state = instance.store.data_mut();
            if start.is_some() {
                state.debug_start = start;
            }
            state.breakpoints = breakpoints;
        }
    }

    pub fn run_command(&mut self, plugin: &str, command: &str, snapshot: Snapshot) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_run_command(s, command))
            .1
    }

    /// Runs a plugin's symbol search, with `query` for `symbol-query`.
    pub fn search_symbols(
        &mut self,
        plugin: &str,
        command: &str,
        query: &str,
        snapshot: Snapshot,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        if let Some(instance) = &mut self.plugins[i].instance {
            instance.store.data_mut().symbol_query = query.to_string();
        }
        self.call(i, snapshot, |b, s| b.call_run_command(s, command))
            .1
    }

    pub fn click(&mut self, plugin: &str, button: &str, snapshot: Snapshot) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        // Data only or turned off: nothing to click or answer.
        if self.plugins[i].manifest.component.is_none() || self.plugins[i].disabled {
            return Vec::new();
        }
        if self.plugins[i].instance.is_none() {
            return match button {
                ALLOW => self.answer(i, true),
                DENY => self.answer(i, false),
                _ => Vec::new(),
            };
        }
        self.call(i, snapshot, |b, s| b.call_on_click(s, button)).1
    }

    /// Tells every working plugin that the open files changed.
    pub fn buffer_event(&mut self, event: &BufferEvent, snapshot: &Snapshot) -> Vec<Event> {
        let event = match event {
            BufferEvent::Switched => types::BufferEvent::Switched,
            BufferEvent::Changed => types::BufferEvent::Changed,
            BufferEvent::Saved(path) => types::BufferEvent::Saved(path.clone()),
        };
        let mut events = Vec::new();
        for i in 0..self.plugins.len() {
            if self.plugins[i].pure_language
                && (matches!(event, types::BufferEvent::Saved(_))
                    || !self.plugins[i].handles_path(snapshot.path.as_deref()))
            {
                continue;
            }
            if !self.plugins[i].manifest.languages.is_empty()
                && snapshot.path.is_some()
                && !self.plugins[i].handles_path(snapshot.path.as_deref())
            {
                continue;
            }
            let (_, mut more) = self.call(i, snapshot.clone(), |b, s| b.call_on_buffer(s, &event));
            events.append(&mut more);
        }
        events
    }

    pub fn submit(
        &mut self,
        plugin: &str,
        input: &str,
        text: &str,
        snapshot: Snapshot,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_on_submit(s, input, text))
            .1
    }

    /// A command line the plugin ran in a terminal, or one of its tests',
    /// ended: all it printed, and its exit code.
    pub fn terminal(
        &mut self,
        plugin: &str,
        id: &str,
        output: &str,
        code: Option<i32>,
        snapshot: Snapshot,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_on_terminal(s, id, output, code))
            .1
    }

    pub fn review(
        &mut self,
        plugin: &str,
        id: &str,
        accepted: bool,
        snapshot: Snapshot,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_on_review(s, id, accepted))
            .1
    }

    /// Colours `text` as a language, named by its id or an extension, with
    /// the plugin that added it. `None` if no plugin did.
    pub fn colour_text(
        &mut self,
        language: &str,
        text: &str,
    ) -> Option<Vec<merlin_core::lang::PluginToken>> {
        let name = language.trim_start_matches('.').to_ascii_lowercase();
        let mut fallback = None;
        for i in 0..self.plugins.len() {
            let matches = self.plugins[i].instance.is_some()
                && self.plugins[i]
                    .manifest
                    .languages
                    .iter()
                    .any(|l| l.id == name || l.extensions.contains(&name));
            if !matches {
                continue;
            }
            let (tokens, _) = self.call(i, Snapshot::default(), |b, s| {
                b.call_colour_text(s, &name, text)
            });
            if let Some(tokens) = tokens {
                let converted: Vec<_> = tokens.into_iter().map(plugin_token).collect();
                if !converted.is_empty() {
                    return Some(converted);
                }
                fallback = Some(converted);
            }
        }
        fallback
    }

    pub fn pick(
        &mut self,
        plugin: &str,
        picker: &str,
        item: &str,
        snapshot: Snapshot,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_on_pick(s, picker, item))
            .1
    }

    /// Asks every working plugin for its panel.
    pub fn render_panels(&mut self, snapshot: &Snapshot) -> Vec<Event> {
        let mut events = Vec::new();
        for i in 0..self.plugins.len() {
            // Data only or turned off: no panel, and nothing to allow.
            if self.plugins[i].manifest.component.is_none() || self.plugins[i].disabled {
                continue;
            }
            if self.plugins[i].instance.is_none() {
                events.push(Event::SetPanel {
                    plugin: self.plugins[i].manifest.name.clone(),
                    panel: Some(permission_card(&self.plugins[i].manifest)),
                });
                continue;
            }
            if self.plugins[i].pure_language {
                continue;
            }
            let (panel, mut more) = self.call(i, snapshot.clone(), |b, s| b.call_render_panel(s));
            events.append(&mut more);
            if let Some(panel) = panel {
                events.push(Event::SetPanel {
                    plugin: self.plugins[i].manifest.name.clone(),
                    panel: panel.map(convert_panel),
                });
            }
        }
        events
    }

    /// Delivers process output that arrived since the last poll. `snapshot`
    /// is only called if there is output to deliver. Returns the events the
    /// plugins asked for, and whether anything was delivered.
    pub fn poll_processes(&mut self, snapshot: impl Fn() -> Snapshot) -> (Vec<Event>, bool) {
        self.woken.store(false, Ordering::Release);
        let pending: Vec<Output> = self.output_rx.try_iter().take(MESSAGES_PER_POLL).collect();
        if pending.is_empty() {
            return (Vec::new(), false);
        }
        let more_waiting = pending.len() == MESSAGES_PER_POLL;
        let snapshot = snapshot();
        let mut events = Vec::new();
        for output in pending {
            let Some(i) = self.index(&output.plugin) else {
                continue;
            };
            if self.plugins[i].generation != output.generation {
                continue;
            }
            if let (types::ProcessEvent::Exit(_), Some(instance)) =
                (&output.event, &mut self.plugins[i].instance)
            {
                instance.store.data_mut().processes.forget(output.process);
            }
            // What a plugin's programs print on stderr goes to its channel,
            // as well as to the plugin.
            if let types::ProcessEvent::Stderr(line) = &output.event {
                events.push(Event::PluginLog {
                    plugin: output.plugin.clone(),
                    level: LogLevel::Stderr,
                    text: line.clone(),
                });
            }
            let (_, mut more) = self.call(i, snapshot.clone(), |b, s| {
                b.call_on_process(s, output.process, &output.event)
            });
            events.append(&mut more);
        }
        if more_waiting
            && !self.woken.swap(true, Ordering::AcqRel)
            && let Some(wake) = &self.waker
        {
            wake();
        }
        (events, true)
    }

    /// Reloads plugins whose component file changed since it was loaded.
    /// A plugin that fails to reload keeps running its old code. One that
    /// saves its state gets it back, and keeps its processes.
    /// The component files of the plugins loaded, which reload when they
    /// change.
    pub fn components(&self) -> Vec<PathBuf> {
        self.plugins
            .iter()
            .filter(|p| !p.wasm.as_os_str().is_empty())
            .map(|p| p.wasm.clone())
            .collect()
    }

    pub fn reload_changed(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for i in 0..self.plugins.len() {
            let now = modified(&self.plugins[i].wasm);
            if now.is_none()
                || now == self.plugins[i].modified
                || self.plugins[i].instance.is_none()
            {
                continue;
            }
            self.plugins[i].modified = now;
            events.extend(self.reload_at(i));
        }
        events
    }

    /// Starts a plugin again from its component, keeping its state if it
    /// saves it.
    pub fn reload(&mut self, name: &str) -> Vec<Event> {
        match self.index(name) {
            Some(i) if self.plugins[i].instance.is_some() => {
                let mut events = self.reload_at(i);
                // Asked for, so said; a reload because its file changed is
                // only logged.
                if !self.plugins[i].failed_reload {
                    let title = &self.plugins[i].manifest.title;
                    events.push(Event::Notify(format!("Reloaded {title}")));
                }
                events
            }
            _ => Vec::new(),
        }
    }

    fn reload_at(&mut self, i: usize) -> Vec<Event> {
        let mut events = Vec::new();
        let name = self.plugins[i].manifest.title.clone();
        let plugin_name = self.plugins[i].manifest.name.clone();
        self.plugins[i].failed_reload = false;
        let (state, mut more) = self.call(i, Snapshot::default(), |b, s| b.call_save_state(s));
        events.append(&mut more);
        let state = state.flatten();
        let plugin = &self.plugins[i];
        // Output from the old instance's processes goes to the new one
        // only if they're handed over.
        let generation = plugin.generation + state.is_none() as u64;
        match self.instantiate(&plugin.manifest, &plugin.wasm, generation) {
            Ok(mut instance) => {
                let plugin = &mut self.plugins[i];
                if let Some(old) = &mut plugin.instance
                    && state.is_some()
                {
                    std::mem::swap(
                        &mut old.store.data_mut().processes,
                        &mut instance.store.data_mut().processes,
                    );
                }
                // Dropping the old store stops any processes it still has.
                plugin.instance = Some(instance);
                plugin.generation = generation;
                plugin.failed = false;
                let kept = state.is_some();
                events.extend(self.activate(i, state));
                events.push(Event::PluginLog {
                    plugin: plugin_name,
                    level: LogLevel::Info,
                    text: format!(
                        "Reloaded the plugin{}",
                        if kept { ", keeping its state" } else { "" }
                    ),
                });
            }
            Err(err) => {
                self.plugins[i].failed_reload = true;
                events.push(Event::PluginNotice {
                    plugin: plugin_name,
                    level: LogLevel::Error,
                    text: format!("{name} didn't reload, so its old code keeps running: {err:#}"),
                    buttons: Vec::new(),
                });
            }
        }
        events
    }

    /// Plugins to load turned off, by name, from the settings.
    pub fn set_disabled(&mut self, names: Vec<String>) {
        self.disabled = names;
    }

    /// Turns a plugin on or off. Off, it stops: its processes end, and its
    /// panel and commands go.
    pub fn set_enabled(&mut self, name: &str, on: bool) -> Vec<Event> {
        let Some(i) = self.index(name) else {
            return Vec::new();
        };
        self.disabled.retain(|n| n != name);
        if !on {
            self.disabled.push(name.to_string());
        }
        let plugin = &mut self.plugins[i];
        if plugin.manifest.component.is_none() || plugin.disabled == !on {
            return Vec::new();
        }
        plugin.disabled = !on;
        if !on {
            // Dropping the store stops its processes.
            plugin.instance = None;
            return vec![
                Event::SetPanel {
                    plugin: name.to_string(),
                    panel: None,
                },
                Event::RegisterPlugin {
                    plugin: name.to_string(),
                    commands: Vec::new(),
                },
            ];
        }
        let plugin = &self.plugins[i];
        match self.instantiate(&plugin.manifest, &plugin.wasm, plugin.generation + 1) {
            Ok(instance) => {
                let plugin = &mut self.plugins[i];
                plugin.instance = Some(instance);
                plugin.generation += 1;
                plugin.failed = false;
                self.activate(i, None)
            }
            Err(err) => vec![Event::PluginFailed {
                plugin: name.to_string(),
                message: format!("{err:#}"),
            }],
        }
    }

    /// Stops a plugin and forgets it, as uninstalling does.
    pub fn unload(&mut self, name: &str) -> Vec<Event> {
        let Some(i) = self.index(name) else {
            return Vec::new();
        };
        self.plugins.remove(i);
        vec![
            Event::SetPanel {
                plugin: name.to_string(),
                panel: None,
            },
            Event::RegisterPlugin {
                plugin: name.to_string(),
                commands: Vec::new(),
            },
        ]
    }

    /// What each plugin's page shows.
    pub fn plugins(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|p| {
                let m = &p.manifest;
                let (memory, processes) = match &p.instance {
                    Some(i) => (
                        i.store.data().limits.memory,
                        i.store.data().processes.running(),
                    ),
                    None => (0, Vec::new()),
                };
                PluginInfo {
                    name: m.name.clone(),
                    title: m.title.clone(),
                    description: m.description.clone(),
                    badge: m.badge.clone(),
                    hue: m.hue.clone(),
                    dir: PathBuf::from(&p.dir),
                    api: m.api.clone(),
                    data_only: m.component.is_none(),
                    enabled: !p.disabled,
                    waiting: m.component.is_some() && !p.disabled && p.instance.is_none(),
                    failed: p.failed,
                    permissions: m.permissions.clone(),
                    languages: m.languages.iter().map(|l| l.name.clone()).collect(),
                    looks: m
                        .themes
                        .iter()
                        .chain(&m.packs)
                        .map(|t| t.label.clone())
                        .collect(),
                    settings: plugin_settings(m, &self.user_settings, &self.workspace_settings),
                    defaults: m.settings.clone(),
                    schema: m
                        .schema
                        .iter()
                        .cloned()
                        .chain(
                            m.component
                                .is_some()
                                .then(common_settings)
                                .into_iter()
                                .flatten(),
                        )
                        .collect(),
                    secrets: m
                        .schema
                        .iter()
                        .filter(|s| s.kind == SettingKind::Secret)
                        .filter(|s| self.secrets.is_set(&m.name, &s.key))
                        .map(|s| s.key.clone())
                        .collect(),
                    memory,
                    busy: p.busy,
                    processes,
                }
            })
            .collect()
    }
}

/// What a plugin shows until the user allows its permissions.
fn permission_card(manifest: &Manifest) -> Panel {
    let p = &manifest.permissions;
    let mut nodes = vec![
        Node::Heading(format!("Allow {}?", manifest.title)),
        Node::Text("This plugin asks to:".into()),
    ];
    if p.editor_read {
        nodes.push(Node::Text("• read the open file".into()));
    }
    if p.editor_write {
        nodes.push(Node::Text("• change the open file".into()));
    }
    if p.workspace_read {
        nodes.push(Node::Text("• read the project's files".into()));
    }
    if p.terminal {
        nodes.push(Node::Styled(
            "• run command lines in a terminal, with your rights".into(),
            Tone::Warning,
        ));
    }
    for program in &p.processes {
        nodes.push(Node::Styled(
            format!("• run `{program}`, outside the sandbox, with your rights"),
            Tone::Warning,
        ));
    }
    if p.debugger {
        nodes.push(Node::Styled(
            "• start the debuggers plugins name when you start a run, and drive them".into(),
            Tone::Warning,
        ));
    }
    nodes.push(Node::Text(String::new()));
    nodes.push(Node::Styled(
        "You'll be asked again only if it wants more.".into(),
        Tone::Muted,
    ));
    nodes.push(Node::Buttons(vec![
        Button {
            id: ALLOW.into(),
            label: "Allow".into(),
        },
        Button {
            id: DENY.into(),
            label: "Don't load".into(),
        },
    ]));
    Panel {
        title: manifest.title.clone(),
        nodes,
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Whether `file` in the compile cache is a build of the plugin `name`,
/// `{name}-{hash}.cwasm`, or of a plugin before builds were named, and so
/// replaced by the one just compiled. Exactly, so `rust` doesn't take
/// `rust-tools`'s.
fn stale_build(file: &str, name: &str) -> bool {
    let Some(hash) = file.strip_suffix(".cwasm") else {
        return false;
    };
    let hash = hash
        .strip_prefix(name)
        .and_then(|h| h.strip_prefix('-'))
        .unwrap_or(hash);
    hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The dotted keys whose values differ between two settings tables.
fn changed_keys(old: &toml::Table, new: &toml::Table, prefix: &str, out: &mut Vec<String>) {
    let keys: std::collections::BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    for key in keys {
        match (old.get(key), new.get(key)) {
            (Some(toml::Value::Table(a)), Some(toml::Value::Table(b))) => {
                changed_keys(a, b, &format!("{prefix}{key}."), out);
            }
            (a, b) if a != b => out.push(format!("{prefix}{key}")),
            _ => {}
        }
    }
}

fn wt(err: wasmtime::Error) -> anyhow::Error {
    anyhow!("{err:#}")
}
