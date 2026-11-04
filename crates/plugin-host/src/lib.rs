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

mod process;
mod secrets;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};
use merlin_core::{
    Button, Command, Completion, Diagnostic, Event, Node, Panel, PickItem, Severity, Tone,
};
use serde::{Deserialize, Serialize};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use process::{Output, Processes};
use secrets::Secrets;

mod bindings {
    wasmtime::component::bindgen!({ path: "../../wit", world: "plugin" });
}

use bindings::Plugin as Bindings;
use bindings::merlin::plugin::{host, types};

/// The API version this host implements. A manifest names the version it was
/// built for and loads only on an exact match while the API is 0.x.
pub const API_VERSION: &str = "0.8";

/// Roughly a few hundred million instructions per call.
const FUEL_PER_CALL: u64 = 500_000_000;

/// Process messages delivered per poll, so a chatty process can't starve
/// the editor of input.
const MESSAGES_PER_POLL: usize = 256;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub title: String,
    pub api: String,
    /// The component file, relative to the plugin folder. A plugin that
    /// only adds data, such as colour themes, has none.
    #[serde(default)]
    pub component: Option<PathBuf>,
    #[serde(default)]
    pub permissions: Permissions,
    /// Default settings, which the user's and the workspace's
    /// `[plugins.<name>]` override.
    #[serde(default)]
    pub settings: toml::Table,
    /// Languages the plugin adds, as a VS Code extension contributes them.
    #[serde(default)]
    pub languages: Vec<ManifestLanguage>,
    /// Colour themes the plugin adds.
    #[serde(default)]
    pub themes: Vec<ManifestTheme>,
    /// Theme packs the plugin adds: icons, layout, fonts and colours, any
    /// of them, layered over a theme. A theme is a pack too.
    #[serde(default)]
    pub packs: Vec<ManifestTheme>,
}

/// A theme or pack: its name in the pickers, and its file, relative to the
/// plugin folder.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestTheme {
    pub label: String,
    pub file: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestLanguage {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub line_comment: Option<String>,
    #[serde(default)]
    pub block_comment: Option<(String, String)>,
}

impl ManifestLanguage {
    fn to_core(&self) -> merlin_core::lang::Language {
        merlin_core::lang::Language {
            id: self.id.clone(),
            name: self.name.clone(),
            extensions: self
                .extensions
                .iter()
                .map(|e| e.trim_start_matches('.').to_string())
                .collect(),
            line_comment: self.line_comment.clone(),
            block_comment: self.block_comment.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Permissions {
    #[serde(default)]
    pub editor_read: bool,
    #[serde(default)]
    pub editor_write: bool,
    /// Programs the plugin may start, by name as found on `PATH`.
    #[serde(default)]
    pub processes: Vec<String>,
}

impl Permissions {
    fn is_empty(&self) -> bool {
        *self == Permissions::default()
    }

    /// Whether these allow everything `wanted` asks for.
    fn covers(&self, wanted: &Permissions) -> bool {
        (self.editor_read || !wanted.editor_read)
            && (self.editor_write || !wanted.editor_write)
            && wanted.processes.iter().all(|p| self.processes.contains(p))
    }
}

/// Permissions the user allowed, by plugin folder.
#[derive(Default, Deserialize, Serialize)]
struct Grants {
    #[serde(default)]
    plugins: BTreeMap<String, Permissions>,
}

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
    pub text: String,
    pub root: PathBuf,
    /// Line and UTF-16 column.
    pub cursor: (u32, u32),
    pub selection: Option<Selection>,
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

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    name: String,
    permissions: Permissions,
    snapshot: Snapshot,
    events: Vec<Event>,
    processes: Processes,
    secrets: Arc<Secrets>,
    /// The plugin's settings, defaults and all.
    settings: toml::Table,
}

impl State {
    fn need(&self, granted: bool, permission: &str) -> Result<(), String> {
        if granted {
            Ok(())
        } else {
            Err(format!("permission denied: {permission}"))
        }
    }
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl types::Host for State {}

impl host::Host for State {
    fn active_text(&mut self) -> Result<String, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        Ok(self.snapshot.text.clone())
    }

    fn active_path(&mut self) -> Result<Option<String>, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        Ok(self.snapshot.path.clone())
    }

    fn active_cursor(&mut self) -> Result<types::Position, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        let (line, column) = self.snapshot.cursor;
        Ok(types::Position { line, column })
    }

    fn active_selection(&mut self) -> Result<Option<types::Selection>, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        let at = |(line, column)| types::Position { line, column };
        Ok(self.snapshot.selection.clone().map(|s| types::Selection {
            start: at(s.start),
            end: at(s.end),
            text: s.text,
        }))
    }

    fn show_diff(&mut self, id: String, path: String, old: String, new: String) {
        self.events.push(Event::ShowReview {
            plugin: self.name.clone(),
            id,
            path: self.snapshot.local(&path),
            old,
            new,
        });
    }

    fn close_diff(&mut self, id: String) {
        self.events.push(Event::CloseReview {
            plugin: self.name.clone(),
            id,
        });
    }

    fn set_diagnostics(&mut self, path: String, diagnostics: Vec<types::Diagnostic>) {
        let at = |p: types::Position| (p.line, p.column);
        self.events.push(Event::SetDiagnostics {
            plugin: self.name.clone(),
            path: self.snapshot.local(&path),
            diagnostics: diagnostics
                .into_iter()
                .map(|d| Diagnostic {
                    start: at(d.start),
                    end: at(d.end),
                    severity: match d.severity {
                        types::Severity::Error => Severity::Error,
                        types::Severity::Warning => Severity::Warning,
                        types::Severity::Info => Severity::Info,
                        types::Severity::Hint => Severity::Hint,
                    },
                    message: d.message,
                })
                .collect(),
        });
    }

    fn set_tokens(
        &mut self,
        path: String,
        layer: types::TokenLayer,
        lines: Option<types::LineRange>,
        tokens: Vec<types::Token>,
    ) {
        self.events.push(Event::SetTokens {
            path: self.snapshot.local(&path),
            layer: match layer {
                types::TokenLayer::Base => merlin_core::layers::Layer::Base,
                types::TokenLayer::Semantic => merlin_core::layers::Layer::Semantic,
            },
            lines: lines.map(|r| (r.start, r.end)),
            tokens: tokens.into_iter().map(plugin_token).collect(),
        });
    }

    fn set_symbols(&mut self, path: String, symbols: Vec<types::DocSymbol>) {
        let at = |p: types::Position| (p.line, p.column);
        self.events.push(Event::SetSymbols {
            path: self.snapshot.local(&path),
            symbols: symbols
                .into_iter()
                .map(|s| merlin_core::lang::PluginSymbol {
                    name: s.name,
                    kind: s.kind,
                    detail: s.detail,
                    start: at(s.start),
                    end: at(s.end),
                    depth: s.depth,
                })
                .collect(),
        });
    }

    fn set_folds(&mut self, path: String, ranges: Vec<types::FoldRange>) {
        self.events.push(Event::SetFolds {
            path: self.snapshot.local(&path),
            ranges: ranges.into_iter().map(|r| (r.first, r.last)).collect(),
        });
    }

    fn show_hover(&mut self, text: String) {
        self.events.push(Event::ShowHover(text));
    }

    fn show_completions(&mut self, items: Vec<types::Completion>) {
        self.events.push(Event::ShowCompletions(
            items
                .into_iter()
                .map(|c| Completion {
                    label: c.label,
                    detail: c.detail,
                    insert: c.insert,
                })
                .collect(),
        ));
    }

    fn open_at(&mut self, path: String, at: types::Position) {
        self.events.push(Event::OpenAt {
            path: self.snapshot.local(&path),
            line: at.line,
            column: at.column,
        });
    }

    fn insert_text(&mut self, text: String) -> Result<(), String> {
        self.need(self.permissions.editor_write, "editor-write")?;
        self.events.push(Event::InsertText(text));
        Ok(())
    }

    fn workspace_root(&mut self) -> String {
        self.snapshot.root.display().to_string()
    }

    fn notify(&mut self, message: String) {
        self.events.push(Event::Notify(message));
    }

    fn focus_panel(&mut self) {
        self.events.push(Event::FocusPanel(self.name.clone()));
    }

    fn show_picker(&mut self, id: String, title: String, items: Vec<types::PickItem>) {
        self.events.push(Event::ShowPicker {
            plugin: self.name.clone(),
            id,
            title,
            items: items
                .into_iter()
                .map(|i| PickItem {
                    id: i.id,
                    label: i.label,
                })
                .collect(),
        });
    }

    fn spawn(
        &mut self,
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        framing: types::Framing,
    ) -> Result<u32, String> {
        if !self.permissions.processes.contains(&program) {
            return Err(format!("permission denied: processes ({program})"));
        }
        let root = self.snapshot.root.clone();
        self.processes.spawn(&program, &args, &env, &root, framing)
    }

    fn send(&mut self, process: u32, message: String) -> Result<(), String> {
        self.processes.send(process, &message)
    }

    fn kill(&mut self, process: u32) {
        self.processes.kill(process);
    }

    fn get_setting(&mut self, key: String) -> Option<types::Setting> {
        let mut value: &toml::Value = self.settings.get(key.split('.').next()?)?;
        for part in key.split('.').skip(1) {
            value = value.as_table()?.get(part)?;
        }
        Some(match value {
            toml::Value::String(s) => types::Setting::Text(s.clone()),
            toml::Value::Boolean(b) => types::Setting::Flag(*b),
            toml::Value::Integer(i) => types::Setting::Integer(*i),
            toml::Value::Float(f) => types::Setting::Float(*f),
            other => types::Setting::Other(other.to_string()),
        })
    }

    fn get_secret(&mut self, key: String) -> Option<String> {
        self.secrets.get(&self.name, &key)
    }

    fn set_secret(&mut self, key: String, value: String) -> Result<(), String> {
        self.secrets.set(&self.name, &key, &value)
    }

    fn delete_secret(&mut self, key: String) {
        self.secrets.delete(&self.name, &key);
    }
}

struct Instance {
    store: Store<State>,
    bindings: Bindings,
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
    config: Config,
    secrets: Arc<Secrets>,
    /// `[plugins]` from the user's and the workspace's settings.
    settings: toml::Table,
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
            config,
            secrets,
            settings: toml::Table::new(),
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
            let plugin = manifest.name.clone();
            let languages: Vec<_> = manifest
                .languages
                .iter()
                .map(ManifestLanguage::to_core)
                .collect();
            self.plugins.push(Loaded {
                manifest,
                dir,
                modified: None,
                wasm: PathBuf::new(),
                instance: None,
                generation: 0,
                failed: false,
            });
            return Ok((!languages.is_empty())
                .then_some(Event::RegisterLanguages { plugin, languages })
                .into_iter()
                .collect());
        };
        let wasm = folder.join(component);
        let ask = self.config.ask
            && !manifest.permissions.is_empty()
            && !self
                .grants()
                .plugins
                .get(&dir)
                .is_some_and(|g| g.covers(&manifest.permissions));
        let instance = if ask {
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
        });
        let index = self.plugins.len() - 1;
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
    /// editor, so a rebuilt plugin or editor compiles afresh.
    fn compile(&self, wasm: &Path) -> Result<Component> {
        use std::hash::{Hash, Hasher};
        let bytes = fs::read(wasm)?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        std::env::current_exe()
            .ok()
            .and_then(|e| fs::metadata(e).ok()?.modified().ok())
            .hash(&mut hasher);
        let dir = self.config.data_dir.join("compiled");
        let cached = dir.join(format!("{:016x}.cwasm", hasher.finish()));
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
            let aside = dir.join(format!(
                "{:016x}.{}.tmp",
                hasher.finish(),
                std::process::id()
            ));
            let _ = fs::create_dir_all(&dir)
                .and_then(|()| fs::write(&aside, compiled))
                .and_then(|()| fs::rename(&aside, &cached));
        }
        Ok(component)
    }

    fn instantiate(&self, manifest: &Manifest, wasm: &Path, generation: u64) -> Result<Instance> {
        let component = self
            .compile(wasm)
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
            ),
            secrets: self.secrets.clone(),
            settings: plugin_settings(manifest, &self.settings),
        };
        let mut store = Store::new(&self.engine, state);
        store.set_fuel(FUEL_PER_CALL).map_err(wt)?;
        let bindings = Bindings::instantiate(&mut store, &component, &self.linker)
            .map_err(wt)
            .with_context(|| format!("instantiating {}", manifest.name))?;
        Ok(Instance { store, bindings })
    }

    fn activate(&mut self, index: usize, state: Option<Vec<u8>>) -> Vec<Event> {
        let plugin = self.plugins[index].manifest.name.clone();
        let (commands, mut events) = self.call(index, Snapshot::default(), |b, s| {
            b.call_activate(s, state.as_deref())
        });
        if let Some(commands) = commands {
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
            let languages: Vec<_> = self.plugins[index]
                .manifest
                .languages
                .iter()
                .map(ManifestLanguage::to_core)
                .collect();
            if !languages.is_empty() {
                events.insert(
                    1,
                    Event::RegisterLanguages {
                        plugin: self.plugins[index].manifest.name.clone(),
                        languages,
                    },
                );
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
        let result = f(&instance.bindings, store);
        let mut events = std::mem::take(&mut store.data_mut().events);
        match result {
            Ok(value) => (Some(value), events),
            Err(err) => {
                plugin.failed = true;
                store.data_mut().processes.kill_all();
                events.push(Event::PluginFailed {
                    plugin: plugin.manifest.name.clone(),
                    message: format!("{err:#}"),
                });
                (None, events)
            }
        }
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.plugins.iter().position(|p| p.manifest.name == name)
    }

    /// Takes new settings for plugins: the `[plugins]` table of the user's
    /// settings with the workspace's over it.
    pub fn set_settings(&mut self, plugins: toml::Table) {
        self.settings = plugins;
        for plugin in &mut self.plugins {
            let settings = plugin_settings(&plugin.manifest, &self.settings);
            if let Some(instance) = &mut plugin.instance {
                instance.store.data_mut().settings = settings;
            }
        }
    }

    pub fn run_command(&mut self, plugin: &str, command: &str, snapshot: Snapshot) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_run_command(s, command))
            .1
    }

    pub fn click(&mut self, plugin: &str, button: &str, snapshot: Snapshot) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        // Data only: nothing to click, and no permission to answer.
        if self.plugins[i].manifest.component.is_none() {
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
        let i = self.plugins.iter().position(|p| {
            p.instance.is_some()
                && p.manifest
                    .languages
                    .iter()
                    .any(|l| l.id == name || l.extensions.contains(&name))
        })?;
        let (tokens, _) = self.call(i, Snapshot::default(), |b, s| {
            b.call_colour_text(s, &name, text)
        });
        Some(tokens?.into_iter().map(plugin_token).collect())
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
            // Data only: no panel, and nothing to allow.
            if self.plugins[i].manifest.component.is_none() {
                continue;
            }
            if self.plugins[i].instance.is_none() {
                events.push(Event::SetPanel {
                    plugin: self.plugins[i].manifest.name.clone(),
                    panel: Some(permission_card(&self.plugins[i].manifest)),
                });
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
        let pending: Vec<Output> = self.output_rx.try_iter().take(MESSAGES_PER_POLL).collect();
        if pending.is_empty() {
            return (Vec::new(), false);
        }
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
            let (_, mut more) = self.call(i, snapshot.clone(), |b, s| {
                b.call_on_process(s, output.process, &output.event)
            });
            events.append(&mut more);
        }
        (events, true)
    }

    /// Reloads plugins whose component file changed since it was loaded.
    /// A plugin that fails to reload keeps running its old code. One that
    /// saves its state gets it back, and keeps its processes.
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
            let name = self.plugins[i].manifest.name.clone();
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
                    events.extend(self.activate(i, state));
                    events.push(Event::Notify(format!("Reloaded {name}")));
                }
                Err(err) => events.push(Event::Notify(format!("Reload of {name} failed: {err:#}"))),
            }
        }
        events
    }
}

fn convert_panel(panel: types::Panel) -> Panel {
    let button = |b: types::Button| Button {
        id: b.id,
        label: b.label,
    };
    let tone = |tone| match tone {
        types::Tone::Normal => Tone::Normal,
        types::Tone::Muted => Tone::Muted,
        types::Tone::Strong => Tone::Strong,
        types::Tone::Warning => Tone::Warning,
        types::Tone::Error => Tone::Error,
    };
    Panel {
        title: panel.title,
        nodes: panel
            .nodes
            .into_iter()
            .map(|node| match node {
                types::Node::Heading(text) => Node::Heading(text),
                types::Node::Text(text) => Node::Text(text),
                types::Node::Styled(s) => Node::Styled(s.text, tone(s.tone)),
                types::Node::Item(i) => Node::Item {
                    id: i.id,
                    label: i.label,
                    tone: tone(i.tone),
                },
                types::Node::Markdown(text) => Node::Markdown(text),
                types::Node::Button(b) => Node::Button(button(b)),
                types::Node::Buttons(bs) => Node::Buttons(bs.into_iter().map(button).collect()),
                types::Node::Input(i) => Node::Input {
                    id: i.id,
                    placeholder: i.placeholder,
                    secret: i.secret,
                },
                types::Node::Divider => Node::Divider,
                types::Node::Footer => Node::Footer,
            })
            .collect(),
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
    for program in &p.processes {
        nodes.push(Node::Styled(
            format!("• run `{program}`, outside the sandbox, with your rights"),
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

fn wt(err: wasmtime::Error) -> anyhow::Error {
    anyhow!("{err:#}")
}

/// A plugin's settings: its manifest's defaults, with what the user set
/// over them.
fn plugin_settings(manifest: &Manifest, plugins: &toml::Table) -> toml::Table {
    let mut settings = manifest.settings.clone();
    if let Some(toml::Value::Table(set)) = plugins.get(&manifest.name) {
        merge(&mut settings, set);
    }
    settings
}

/// Puts `over`'s values into `base`, table by table.
pub fn merge(base: &mut toml::Table, over: &toml::Table) {
    for (key, value) in over {
        match (base.get_mut(key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

fn plugin_token(t: types::Token) -> merlin_core::lang::PluginToken {
    use merlin_core::Syntax;
    use types::TokenKind as K;
    let at = |p: types::Position| (p.line, p.column);
    merlin_core::lang::PluginToken {
        start: at(t.start),
        end: at(t.end),
        kind: match t.kind {
            K::Plain => Syntax::Plain,
            K::Keyword => Syntax::Keyword,
            K::String => Syntax::String,
            K::Comment => Syntax::Comment,
            K::Function => Syntax::Function,
            K::Type => Syntax::Type,
            K::Constant => Syntax::Constant,
            K::Attribute => Syntax::Attribute,
            K::Property => Syntax::Property,
            K::Operator => Syntax::Operator,
            K::Punctuation => Syntax::Punctuation,
            K::Escape => Syntax::Escape,
            K::Variable => Syntax::Variable,
            K::Macro => Syntax::Macro,
        },
    }
}

fn read_grants(data_dir: &Path) -> Grants {
    fs::read_to_string(data_dir.join("grants.toml"))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Remembers that the plugin in `dir` may do what `permissions` say.
fn save_grant(data_dir: &Path, dir: &str, permissions: &Permissions) -> Result<()> {
    let mut grants = read_grants(data_dir);
    grants.plugins.insert(dir.to_string(), permissions.clone());
    fs::create_dir_all(data_dir)?;
    fs::write(data_dir.join("grants.toml"), toml::to_string(&grants)?)?;
    Ok(())
}

/// Reads a plugin's manifest.
pub fn manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join("plugin.toml");
    toml::from_str(
        &fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))
}

/// Allows the plugin in `dir` what its manifest asks for, as installing it
/// does, and says what that is.
pub fn grant(config: &Config, dir: &Path) -> Result<(Manifest, Vec<String>)> {
    let manifest = manifest(dir)?;
    let dir = dir.canonicalize()?.display().to_string();
    save_grant(&config.data_dir, &dir, &manifest.permissions)?;
    let p = &manifest.permissions;
    let mut allowed = Vec::new();
    if p.editor_read {
        allowed.push("read the open file".to_string());
    }
    if p.editor_write {
        allowed.push("change the open file".to_string());
    }
    for program in &p.processes {
        allowed.push(format!("run `{program}`, outside the sandbox"));
    }
    Ok((manifest, allowed))
}
