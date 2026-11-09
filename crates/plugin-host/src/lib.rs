//! Loads plugins as WebAssembly components and calls them.
//!
//! A plugin can only call the functions in `sdk/rust/wit/merlin.wit`, plus a WASI
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

pub mod bundled;
mod manifest;
pub mod package;
mod process;
pub mod registry;
mod secrets;
mod state;
mod worker;

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
use worker::{Finished, Instance, Ticket, Worker};

pub use manifest::*;

mod bindings {
    wasmtime::component::bindgen!({ path: "../../sdk/rust/wit", world: "plugin" });
}

use bindings::Plugin as Bindings;
use bindings::merlin::plugin::{host, types};

/// The API version this host implements. A manifest names the version it was
/// built for and loads only on an exact match while the API is 0.x.
pub const API_VERSION: &str = "0.14";

/// Roughly six billion instructions per call, about a second. It stops a
/// plugin stuck in a loop, and leaves room for a language plugin to colour
/// a whole 512 KB file the first time it opens, which takes up to 3.5
/// billion (about half a second) for dense code.
const FUEL_PER_CALL: u64 = 6_000_000_000;

/// Process messages delivered per poll, so a chatty process can't starve
/// the editor of input.
const MESSAGES_PER_POLL: usize = 256;

#[derive(Clone)]
pub struct Config {
    /// Where remembered permissions and file-backed secrets are kept.
    pub data_dir: PathBuf,
    /// Ask before granting a plugin's permissions the first time. When off,
    /// they're granted as the manifest says and nothing is remembered.
    pub ask: bool,
    /// Keep secrets in the system keyring when `secret-tool` is installed.
    pub keyring: bool,
    /// Calls into plugins don't wait: each plugin runs on a thread of its
    /// own and what it asks for comes from [`PluginHost::poll_processes`].
    /// Off, each call waits and returns its events, as scripts and tests
    /// want.
    pub background: bool,
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
            background: false,
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
    /// The file the last right-click menu was opened on.
    pub menu_target: Option<String>,
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

struct Loaded {
    manifest: Manifest,
    /// The plugin's folder, which its permissions are remembered by.
    dir: String,
    wasm: PathBuf,
    modified: Option<SystemTime>,
    /// Its thread, which has its instance. None while waiting for the
    /// user to allow its permissions, or to start.
    worker: Option<Worker>,
    /// Bumped on every reload, so output from an old instance's processes
    /// isn't delivered to the new one.
    generation: u64,
    failed: bool,
    /// Its last reload failed, so the old code runs on.
    failed_reload: bool,
    /// Turned off by the user: loaded, but not running.
    disabled: bool,
    /// Adds languages only: no commands, processes, debugger or symbol
    /// search, so it has no panel and only cares about files of its own
    /// languages.
    pure_language: bool,
    /// Not started yet: it waits for what its `activate-on` names, or
    /// for one of its commands or views to be used.
    dormant: bool,
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
    /// Waits to start until its file, language, commands or views are
    /// needed.
    pub dormant: bool,
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
    /// Calls that didn't wait, done.
    done_tx: Sender<Finished>,
    done_rx: Receiver<Finished>,
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
        let (done_tx, done_rx) = channel();
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
            done_tx,
            done_rx,
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
                worker: None,
                generation: 0,
                failed: false,
                failed_reload: false,
                disabled: false,
                pure_language: false,
                dormant: false,
            });
            return Ok(events);
        };
        let wasm = folder.join(component);
        let ask = self.config.ask
            && !manifest.permissions.is_empty()
            && !self.grants().covers(&dir, &manifest.permissions);
        let off = self.disabled.contains(&manifest.name);
        // One that starts when needed waits, if the commands it had last
        // time are known for the palette meanwhile.
        let cached = (!manifest.activate_on.is_empty() && !ask && !off)
            .then(|| self.cached_commands(&manifest.name))
            .flatten();
        let worker = if ask || off || cached.is_some() {
            None
        } else {
            Some(self.instantiate(&manifest, &wasm, 0)?)
        };
        self.plugins.push(Loaded {
            manifest,
            dir,
            modified: modified(&wasm),
            wasm,
            worker,
            generation: 0,
            failed: false,
            failed_reload: false,
            disabled: off,
            pure_language: false,
            dormant: cached.is_some(),
        });
        let index = self.plugins.len() - 1;
        if off {
            return Ok(Vec::new());
        }
        if let Some(commands) = cached {
            let mut events = vec![Event::RegisterPlugin {
                plugin: self.plugins[index].manifest.name.clone(),
                commands,
            }];
            events.extend(self.declared_events(index));
            return Ok(events);
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

    /// Stops each plugin that needs one that isn't loaded or is off, and
    /// says which. Call once every plugin has loaded.
    pub fn check_depends(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for i in 0..self.plugins.len() {
            let missing: Vec<String> = self.plugins[i]
                .manifest
                .depends
                .iter()
                .filter(|d| {
                    !self
                        .plugins
                        .iter()
                        .any(|p| p.manifest.name == **d && !p.disabled && !p.failed)
                })
                .cloned()
                .collect();
            if missing.is_empty() || self.plugins[i].failed {
                continue;
            }
            let plugin = &mut self.plugins[i];
            plugin.worker = None;
            plugin.dormant = false;
            plugin.failed = true;
            let which = missing.join(", ");
            let s = if missing.len() == 1 { "" } else { "s" };
            events.push(Event::PluginFailed {
                plugin: plugin.manifest.name.clone(),
                message: format!("it needs the {which} plugin{s}, which isn't installed or is off"),
            });
        }
        events
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
        let worker = match self.instantiate(&plugin.manifest, &plugin.wasm, 0) {
            Ok(worker) => worker,
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
        self.plugins[index].worker = Some(worker);
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

    /// Starts a plugin's instance on a thread of its own.
    fn instantiate(&self, manifest: &Manifest, wasm: &Path, generation: u64) -> Result<Worker> {
        let instance = self.instance(manifest, wasm, generation)?;
        Worker::start(&manifest.name, instance).context("starting the plugin's thread")
    }

    fn instance(&self, manifest: &Manifest, wasm: &Path, generation: u64) -> Result<Instance> {
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
            images: Default::default(),
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
        Ok(Instance {
            store,
            bindings,
            failed: false,
        })
    }

    fn activate(&mut self, index: usize, state: Option<Vec<u8>>) -> Vec<Event> {
        let plugin = self.plugins[index].manifest.name.clone();
        let (commands, mut events) = self.call(index, Snapshot::default(), move |b, s| {
            b.call_activate(s, state.as_deref())
        });
        if let Some(commands) = commands {
            self.cache_commands(index, &commands);
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
            for (i, event) in self.declared_events(index).into_iter().enumerate() {
                events.insert(1 + i, event);
            }
        }
        events
    }

    /// What a plugin's manifest adds: its languages, keys and the rest,
    /// then its views.
    fn declared_events(&self, index: usize) -> Vec<Event> {
        let loaded = &self.plugins[index];
        let views = loaded
            .manifest
            .views
            .iter()
            .map(|v| v.to_core(&loaded.manifest.name, Path::new(&loaded.dir)))
            .collect();
        let mut declared = manifest_events(&loaded.manifest);
        declared.push(Event::RegisterViews {
            plugin: loaded.manifest.name.clone(),
            views,
        });
        declared
    }

    /// Where a plugin's commands are kept between runs, for the palette
    /// while it waits to start.
    fn commands_file(&self, name: &str) -> PathBuf {
        self.config
            .data_dir
            .join("commands")
            .join(format!("{name}.toml"))
    }

    fn cached_commands(&self, name: &str) -> Option<Vec<Command>> {
        #[derive(Deserialize)]
        struct Cached {
            commands: Vec<CachedCommand>,
        }
        #[derive(Deserialize)]
        struct CachedCommand {
            id: String,
            title: String,
            key: Option<String>,
        }
        let text = fs::read_to_string(self.commands_file(name)).ok()?;
        let cached: Cached = toml::from_str(&text).ok()?;
        Some(
            cached
                .commands
                .into_iter()
                .map(|c| Command {
                    id: c.id,
                    title: c.title,
                    key: c.key,
                })
                .collect(),
        )
    }

    /// Remembers a plugin's commands, if it waits to start next time.
    fn cache_commands(&self, index: usize, commands: &[types::Command]) {
        let m = &self.plugins[index].manifest;
        if m.activate_on.is_empty() {
            return;
        }
        #[derive(Serialize)]
        struct Cached<'a> {
            commands: Vec<CachedCommand<'a>>,
        }
        #[derive(Serialize)]
        struct CachedCommand<'a> {
            id: &'a str,
            title: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            key: Option<&'a str>,
        }
        let cached = Cached {
            commands: commands
                .iter()
                .map(|c| CachedCommand {
                    id: &c.id,
                    title: &c.title,
                    key: c.key.as_deref(),
                })
                .collect(),
        };
        let path = self.commands_file(&m.name);
        if let (Some(dir), Ok(text)) = (path.parent(), toml::to_string(&cached)) {
            let _ = fs::create_dir_all(dir);
            let _ = fs::write(path, text);
        }
    }

    /// Starts a plugin that was waiting to.
    fn wake(&mut self, index: usize) -> Vec<Event> {
        let plugin = &self.plugins[index];
        let name = plugin.manifest.name.clone();
        match self.instantiate(&plugin.manifest, &plugin.wasm, plugin.generation) {
            Ok(worker) => {
                let plugin = &mut self.plugins[index];
                plugin.worker = Some(worker);
                plugin.dormant = false;
                self.activate(index, None)
            }
            Err(err) => {
                let plugin = &mut self.plugins[index];
                plugin.dormant = false;
                plugin.failed = true;
                vec![Event::PluginFailed {
                    plugin: name,
                    message: format!("{err:#}"),
                }]
            }
        }
    }

    /// Whether a waiting plugin's `activate-on` matches the open file or
    /// the workspace.
    fn wants_start(&self, index: usize, snapshot: &Snapshot) -> bool {
        let ext = snapshot
            .path
            .as_deref()
            .and_then(|p| p.rsplit_once('.'))
            .map(|(_, e)| e.to_ascii_lowercase());
        self.plugins[index].manifest.activate_on.iter().any(|t| {
            if let Some(id) = t.strip_prefix("language:") {
                // The language's extensions, from whichever plugin adds it.
                return ext.as_ref().is_some_and(|ext| {
                    self.plugins.iter().any(|p| {
                        p.manifest.languages.iter().any(|l| {
                            l.id == id && l.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
                        })
                    })
                });
            }
            t.strip_prefix("file:").is_some_and(|f| {
                !snapshot.root.as_os_str().is_empty() && snapshot.root.join(f).exists()
            })
        })
    }

    /// Calls into a plugin, waits, and collects the events it asked for.
    /// A trap disables the plugin and stops its processes until it is
    /// reloaded. One waiting to start starts.
    fn call<R: Send + 'static>(
        &mut self,
        index: usize,
        snapshot: Snapshot,
        f: impl FnOnce(&Bindings, &mut Store<State>) -> wasmtime::Result<R> + Send + 'static,
    ) -> (Option<R>, Vec<Event>) {
        let mut events = if self.plugins[index].dormant {
            self.wake(index)
        } else {
            Vec::new()
        };
        let plugin = &mut self.plugins[index];
        let Some(worker) = plugin.worker.as_ref().filter(|_| !plugin.failed) else {
            return (None, events);
        };
        let name = plugin.manifest.name.clone();
        let Some((result, more, failed)) = worker.run(move |i| run_call(i, &name, snapshot, f))
        else {
            return (None, events);
        };
        plugin.failed |= failed;
        events.extend(more);
        (result, events)
    }

    /// Calls into a plugin without waiting, when the host runs in the
    /// background: `then` turns what it returns into events, and they come
    /// from [`Self::poll_processes`] with the ones it asked for. A newer
    /// call of the same `kind` replaces this one if it hasn't started.
    /// `quiet` calls only draw, so their events don't ask for views again.
    /// Not in the background, it waits and returns them.
    fn post<R: Send + 'static>(
        &mut self,
        index: usize,
        snapshot: Snapshot,
        kind: Option<String>,
        quiet: bool,
        f: impl FnOnce(&Bindings, &mut Store<State>) -> wasmtime::Result<R> + Send + 'static,
        then: impl FnOnce(R) -> Vec<Event> + Send + 'static,
    ) -> Vec<Event> {
        let ticket = match (&self.plugins[index].worker, kind) {
            (Some(worker), Some(kind)) if self.config.background => Some(worker.ticket(kind)),
            _ => None,
        };
        self.post_ticketed(index, snapshot, ticket, quiet, f, then)
    }

    /// [`Self::post`] with a ticket already taken, which several calls can
    /// share.
    fn post_ticketed<R: Send + 'static>(
        &mut self,
        index: usize,
        snapshot: Snapshot,
        ticket: Option<Ticket>,
        quiet: bool,
        f: impl FnOnce(&Bindings, &mut Store<State>) -> wasmtime::Result<R> + Send + 'static,
        then: impl FnOnce(R) -> Vec<Event> + Send + 'static,
    ) -> Vec<Event> {
        if !self.config.background {
            let (result, mut events) = self.call(index, snapshot, f);
            events.extend(result.map(then).unwrap_or_default());
            return events;
        }
        let events = if self.plugins[index].dormant {
            self.wake(index)
        } else {
            Vec::new()
        };
        let plugin = &self.plugins[index];
        let Some(worker) = plugin.worker.as_ref().filter(|_| !plugin.failed) else {
            return events;
        };
        let name = plugin.manifest.name.clone();
        let generation = plugin.generation;
        let done = self.done_tx.clone();
        let waker = self.waker.clone();
        let woken = self.woken.clone();
        worker.post(ticket, move |i| {
            let (result, mut events, failed) = run_call(i, &name, snapshot, f);
            events.extend(result.map(then).unwrap_or_default());
            if events.is_empty() && !failed {
                return;
            }
            let _ = done.send(Finished {
                plugin: name,
                generation,
                events,
                failed,
                quiet,
            });
            if !woken.swap(true, Ordering::AcqRel)
                && let Some(wake) = &waker
            {
                wake();
            }
        });
        events
    }

    /// [`Self::post`] for a call whose result is only its events.
    fn post_call<R: Send + 'static>(
        &mut self,
        index: usize,
        snapshot: Snapshot,
        f: impl FnOnce(&Bindings, &mut Store<State>) -> wasmtime::Result<R> + Send + 'static,
    ) -> Vec<Event> {
        self.post(index, snapshot, None, false, f, |_| Vec::new())
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
            let plugin = &self.plugins[i];
            if plugin.worker.is_none() {
                continue;
            }
            let settings = plugin_settings(
                &plugin.manifest,
                &self.user_settings,
                &self.workspace_settings,
            );
            let call = move |b: &Bindings, s: &mut Store<State>| {
                let old = std::mem::replace(&mut s.data_mut().settings, settings);
                let mut changed: Vec<String> = Vec::new();
                changed_keys(&old, &s.data().settings, "", &mut changed);
                for key in changed {
                    b.call_on_setting_changed(&mut *s, &key)?;
                }
                Ok(())
            };
            events.extend(self.post_call(i, snapshot.clone(), call));
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
        let key = key.to_string();
        Ok(self.post_call(i, snapshot.clone(), move |b, s| {
            b.call_on_setting_changed(s, &key)
        }))
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
        if let Some(worker) = &self.plugins[i].worker {
            worker.post(None, move |i| {
                let state = i.store.data_mut();
                if start.is_some() {
                    state.debug_start = start;
                }
                state.breakpoints = breakpoints;
            });
        }
    }

    pub fn run_command(&mut self, plugin: &str, command: &str, snapshot: Snapshot) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        let command = command.to_string();
        self.post_call(i, snapshot, move |b, s| b.call_run_command(s, &command))
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
        let (command, query) = (command.to_string(), query.to_string());
        self.post_call(i, snapshot, move |b, s| {
            s.data_mut().symbol_query = query;
            b.call_run_command(s, &command)
        })
    }

    pub fn click(&mut self, plugin: &str, button: &str, snapshot: Snapshot) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        // Data only or turned off: nothing to click or answer.
        if self.plugins[i].manifest.component.is_none() || self.plugins[i].disabled {
            return Vec::new();
        }
        if self.plugins[i].worker.is_none() && !self.plugins[i].dormant {
            return match button {
                ALLOW => self.answer(i, true),
                DENY => self.answer(i, false),
                _ => Vec::new(),
            };
        }
        let button = button.to_string();
        self.post_call(i, snapshot, move |b, s| b.call_on_click(s, &button))
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
            // Waiting to start: only its own files or languages start it.
            if self.plugins[i].dormant && !self.wants_start(i, snapshot) {
                continue;
            }
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
            // Typing on: a change not yet heard is replaced by the newer one.
            let kind = matches!(event, types::BufferEvent::Changed).then(|| "changed".to_string());
            let event = event.clone();
            events.extend(self.post(
                i,
                snapshot.clone(),
                kind,
                false,
                move |b, s| b.call_on_buffer(s, &event),
                |()| Vec::new(),
            ));
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
        let (input, text) = (input.to_string(), text.to_string());
        self.post_call(i, snapshot, move |b, s| b.call_on_submit(s, &input, &text))
    }

    /// The user cancelled a plugin's work under way.
    pub fn cancel(&mut self, plugin: &str, id: &str, snapshot: Snapshot) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        let id = id.to_string();
        self.post_call(i, snapshot, move |b, s| b.call_on_cancel(s, &id))
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
        let (id, output) = (id.to_string(), output.to_string());
        self.post_call(i, snapshot, move |b, s| {
            b.call_on_terminal(s, &id, &output, code)
        })
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
        let id = id.to_string();
        self.post_call(i, snapshot, move |b, s| b.call_on_review(s, &id, accepted))
    }

    /// Colours `text` as a language, named by its id or an extension, with
    /// the plugin that added it: a [`Event::TextColours`], now or from
    /// [`Self::poll_processes`]. With no plugin for it, no colours.
    pub fn colour_text(&mut self, language: &str, text: &str) -> Vec<Event> {
        let name = language.trim_start_matches('.').to_ascii_lowercase();
        let found = (0..self.plugins.len()).find(|&i| {
            (self.plugins[i].worker.is_some() || self.plugins[i].dormant)
                && !self.plugins[i].failed
                && self.plugins[i]
                    .manifest
                    .languages
                    .iter()
                    .any(|l| l.id == name || l.extensions.contains(&name))
        });
        let (language, text) = (language.to_string(), text.to_string());
        let Some(i) = found else {
            return vec![Event::TextColours {
                language,
                text,
                tokens: Vec::new(),
            }];
        };
        let asked = text.clone();
        self.post(
            i,
            Snapshot::default(),
            None,
            true,
            move |b, s| b.call_colour_text(s, &name, &asked),
            move |tokens| {
                vec![Event::TextColours {
                    language,
                    text,
                    tokens: tokens.into_iter().map(plugin_token).collect(),
                }]
            },
        )
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
        let (picker, item) = (picker.to_string(), item.to_string());
        self.post_call(i, snapshot, move |b, s| b.call_on_pick(s, &picker, &item))
    }

    /// Opens a file in a plugin's `[[editors]]` as document `surface`:
    /// a [`Event::DocumentOpened`] with its pages, or why it can't.
    pub fn open_document(
        &mut self,
        plugin: &str,
        editor: &str,
        path: &str,
        data: Vec<u8>,
        surface: u32,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return vec![Event::DocumentOpened {
                surface,
                pages: Err(format!("the {plugin} plugin isn't loaded")),
            }];
        };
        // Off, stopped, or waiting to be allowed: it can't draw it.
        let loaded = &self.plugins[i];
        let why = if loaded.disabled {
            Some("is turned off in Settings › Plugins")
        } else if loaded.failed {
            Some("stopped; its Output says why")
        } else if loaded.worker.is_none() && !loaded.dormant {
            Some("is waiting for you to allow it")
        } else {
            None
        };
        if let Some(why) = why {
            return vec![Event::DocumentOpened {
                surface,
                pages: Err(format!("The {} plugin {why}.", loaded.manifest.title)),
            }];
        }
        let (editor, path) = (editor.to_string(), path.to_string());
        self.post(
            i,
            Snapshot::default(),
            None,
            true,
            move |b, s| b.call_open_document(s, surface, &editor, &path, &data),
            move |pages| {
                vec![Event::DocumentOpened {
                    surface,
                    pages: pages.map(|pages| {
                        pages
                            .into_iter()
                            .map(|p| merlin_core::PageSize {
                                width: p.width,
                                height: p.height,
                            })
                            .collect()
                    }),
                }]
            },
        )
    }

    /// Asks for the parts of a document's pages that show, each a
    /// [`Event::DocumentTile`]. Ones asked for before that haven't started
    /// are dropped: the view moved on.
    pub fn render_tiles(
        &mut self,
        plugin: &str,
        surface: u32,
        tiles: Vec<merlin_core::TileKey>,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        let ticket = match &self.plugins[i].worker {
            Some(worker) if self.config.background => {
                Some(worker.ticket(format!("tiles {surface}")))
            }
            _ => None,
        };
        let mut events = Vec::new();
        for tile in tiles {
            let asked = types::Tile {
                page: tile.page,
                x: tile.x,
                y: tile.y,
                width: tile.width,
                height: tile.height,
                scale: tile.scale(),
            };
            events.extend(self.post_ticketed(
                i,
                Snapshot::default(),
                ticket.clone(),
                true,
                move |b, s| b.call_render_tile(s, surface, asked),
                move |picture| {
                    let image = picture.and_then(|p| {
                        let (width, height, pixels) = state::decode(types::Picture::Rgba(p))?;
                        Ok(Arc::new(merlin_core::PluginImage::new(
                            width, height, pixels,
                        )))
                    });
                    vec![Event::DocumentTile {
                        surface,
                        tile,
                        image,
                    }]
                },
            ));
        }
        events
    }

    /// A page drawn whole, to copy: a [`Event::PageImage`].
    pub fn page_image(
        &mut self,
        plugin: &str,
        surface: u32,
        tile: merlin_core::TileKey,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        let asked = types::Tile {
            page: tile.page,
            x: 0,
            y: 0,
            width: tile.width,
            height: tile.height,
            scale: tile.scale(),
        };
        self.post(
            i,
            Snapshot::default(),
            None,
            true,
            move |b, s| b.call_render_tile(s, surface, asked),
            move |picture| {
                let image = picture.and_then(|p| {
                    let (width, height, pixels) = state::decode(types::Picture::Rgba(p))?;
                    Ok(Arc::new(merlin_core::PluginImage::new(
                        width, height, pixels,
                    )))
                });
                vec![Event::PageImage { surface, image }]
            },
        )
    }

    /// Asks where a page's text is: a [`Event::DocumentText`].
    pub fn page_text(&mut self, plugin: &str, surface: u32, page: u32) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.post(
            i,
            Snapshot::default(),
            None,
            true,
            move |b, s| b.call_page_text(s, surface, page),
            move |boxes| {
                vec![Event::DocumentText {
                    surface,
                    page,
                    boxes: boxes
                        .into_iter()
                        .map(|b| merlin_core::TextBox {
                            x: b.x,
                            y: b.y,
                            width: b.width,
                            height: b.height,
                            text: b.text,
                        })
                        .collect(),
                }]
            },
        )
    }

    /// A document's tab closed.
    pub fn close_document(&mut self, plugin: &str, surface: u32) {
        if let Some(i) = self.index(plugin)
            && self.plugins[i].worker.is_some()
        {
            self.post_call(i, Snapshot::default(), move |b, s| {
                b.call_close_document(s, surface)
            });
        }
    }

    /// Asks plugins to draw the views in `shown`, by name. With `all`,
    /// a plugin waiting for its permissions shows what it asks for, on the
    /// right.
    pub fn render_panels(
        &mut self,
        snapshot: &Snapshot,
        shown: &[String],
        all: bool,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        for i in 0..self.plugins.len() {
            // Data only or turned off: no views, and nothing to allow.
            if self.plugins[i].manifest.component.is_none() || self.plugins[i].disabled {
                continue;
            }
            let name = self.plugins[i].manifest.name.clone();
            if self.plugins[i].worker.is_none() && !self.plugins[i].dormant {
                if all {
                    events.push(Event::SetPanel {
                        plugin: name,
                        panel: Some(permission_card(&self.plugins[i].manifest)),
                    });
                }
                continue;
            }
            let views: Vec<String> = self.plugins[i]
                .manifest
                .views
                .iter()
                .map(|v| v.id.clone())
                .filter(|id| shown.contains(&merlin_core::view_key(&name, id)))
                .collect();
            for id in views {
                let key = merlin_core::view_key(&name, &id);
                events.extend(self.post(
                    i,
                    snapshot.clone(),
                    Some(format!("view {id}")),
                    true,
                    move |b, s| b.call_render_view(s, &id),
                    move |panel| {
                        vec![Event::SetPanel {
                            plugin: key,
                            panel: panel.map(convert_panel),
                        }]
                    },
                ));
            }
        }
        events
    }

    /// Delivers process output that arrived since the last poll, and
    /// collects what calls that didn't wait asked for. `snapshot` is only
    /// called if there is output to deliver. Returns the events, and
    /// whether anything happened that may change what plugins show:
    /// events from drawing a view or colouring text alone don't.
    pub fn poll_processes(&mut self, snapshot: impl Fn() -> Snapshot) -> (Vec<Event>, bool) {
        self.woken.store(false, Ordering::Release);
        let mut events = Vec::new();
        let mut changed = false;
        let finished: Vec<Finished> = self.done_rx.try_iter().collect();
        for done in finished {
            let Some(i) = self.index(&done.plugin) else {
                continue;
            };
            if self.plugins[i].generation != done.generation {
                continue;
            }
            self.plugins[i].failed |= done.failed;
            changed |= !done.quiet;
            events.extend(done.events);
        }
        let pending: Vec<Output> = self.output_rx.try_iter().take(MESSAGES_PER_POLL).collect();
        if pending.is_empty() {
            return (events, changed);
        }
        let more_waiting = pending.len() == MESSAGES_PER_POLL;
        let snapshot = snapshot();
        for output in pending {
            let Some(i) = self.index(&output.plugin) else {
                continue;
            };
            if self.plugins[i].generation != output.generation {
                continue;
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
            let (process, event) = (output.process, output.event);
            events.extend(self.post_call(i, snapshot.clone(), move |b, s| {
                if let types::ProcessEvent::Exit(_) = &event {
                    s.data_mut().processes.forget(process);
                }
                b.call_on_process(s, process, &event)
            }));
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
            if now.is_none() || now == self.plugins[i].modified || self.plugins[i].worker.is_none()
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
            Some(i) if self.plugins[i].worker.is_some() => {
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
        let started = self
            .instance(&plugin.manifest, &plugin.wasm, generation)
            .and_then(|mut instance| {
                // The old instance's programs go to the new one, on the
                // old one's thread.
                if let Some(old) = &plugin.worker
                    && state.is_some()
                {
                    instance = old
                        .run(move |old| {
                            old.store
                                .data_mut()
                                .processes
                                .hand_over(&mut instance.store.data_mut().processes);
                            instance
                        })
                        .ok_or_else(|| anyhow!("the plugin's thread ended"))?;
                }
                let name = &plugin.manifest.name;
                Worker::start(name, instance).context("starting the plugin's thread")
            });
        match started {
            Ok(worker) => {
                let plugin = &mut self.plugins[i];
                // Dropping the old instance stops any programs it still has.
                plugin.worker = Some(worker);
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
            // Dropping the instance stops its processes.
            plugin.worker = None;
            return vec![
                Event::SetPanel {
                    plugin: name.to_string(),
                    panel: None,
                },
                Event::RegisterPlugin {
                    plugin: name.to_string(),
                    commands: Vec::new(),
                },
                Event::RegisterViews {
                    plugin: name.to_string(),
                    views: Vec::new(),
                },
                Event::ProgressEnd {
                    plugin: name.to_string(),
                    id: None,
                },
            ];
        }
        let plugin = &self.plugins[i];
        match self.instantiate(&plugin.manifest, &plugin.wasm, plugin.generation + 1) {
            Ok(worker) => {
                let plugin = &mut self.plugins[i];
                plugin.worker = Some(worker);
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
            Event::RegisterViews {
                plugin: name.to_string(),
                views: Vec::new(),
            },
            Event::ProgressEnd {
                plugin: name.to_string(),
                id: None,
            },
        ]
    }

    /// What each plugin's page shows.
    pub fn plugins(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|p| {
                let m = &p.manifest;
                let stats = p.worker.as_ref().map(Worker::stats).unwrap_or_default();
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
                    waiting: m.component.is_some()
                        && !p.disabled
                        && !p.dormant
                        && p.worker.is_none(),
                    dormant: p.dormant,
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
                    memory: stats.memory,
                    busy: stats.busy,
                    processes: stats.processes,
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
    for (what, outside) in p.described() {
        let line = format!("• {what}");
        nodes.push(if outside {
            Node::Styled(line, Tone::Warning)
        } else {
            Node::Text(line)
        });
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

/// One call into a plugin's instance, on its thread: what it returned,
/// the events it asked for, and whether it failed and is now stopped.
fn run_call<R>(
    instance: &mut Instance,
    plugin: &str,
    snapshot: Snapshot,
    f: impl FnOnce(&Bindings, &mut Store<State>) -> wasmtime::Result<R>,
) -> (Option<R>, Vec<Event>, bool) {
    if instance.failed {
        return (None, Vec::new(), false);
    }
    let store = &mut instance.store;
    store.data_mut().snapshot = snapshot;
    let _ = store.set_fuel(FUEL_PER_CALL);
    let result = f(&instance.bindings, store);
    let mut events = std::mem::take(&mut store.data_mut().events);
    match result {
        Ok(value) => (Some(value), events, false),
        Err(err) => {
            instance.failed = true;
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
                plugin: plugin.to_string(),
                message,
            });
            (None, events, true)
        }
    }
}

fn wt(err: wasmtime::Error) -> anyhow::Error {
    anyhow!("{err:#}")
}
