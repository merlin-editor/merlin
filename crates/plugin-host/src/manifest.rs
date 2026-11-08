//! A plugin's `plugin.toml`: what it adds, the permissions it asks for, and
//! the grants the user gave, remembered by its folder.

use super::*;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
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
    /// Debuggers the plugin brings, for languages it adds or others.
    #[serde(default)]
    pub debuggers: Vec<ManifestDebugger>,
    /// Keys under Vim's leader: a group with `group` and its label, or
    /// `keys` (after the leader, such as `x n`) and the command they run.
    #[serde(default)]
    pub keys: Vec<ManifestKey>,
    /// A command that searches the project's symbols for the palette. It
    /// reads the query with `symbol-query` and answers with
    /// `show-project-symbols`.
    #[serde(default)]
    pub symbols: Option<String>,
    /// Steps aside for files a language plugin with features of its own
    /// adds: a client for every language server, say, beside plugins that
    /// bring their own.
    #[serde(default)]
    pub fallback: bool,
}

/// A debugger, as `[[debuggers]]` in a manifest declares it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestKey {
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub keys: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    pub label: String,
}

impl ManifestKey {
    pub(super) fn to_core(&self, plugin: &str) -> Option<merlin_core::LeaderKey> {
        let keys = self
            .group
            .as_ref()
            .or(self.keys.as_ref())?
            .trim()
            .to_string();
        if keys.is_empty() {
            return None;
        }
        // The plugin's own command, with or without its name before it.
        let command = match (&self.group, &self.command) {
            (Some(_), _) => None,
            (None, Some(c)) => Some(
                c.strip_prefix(&format!("{plugin}."))
                    .unwrap_or(c)
                    .to_string(),
            ),
            (None, None) => return None,
        };
        Some(merlin_core::LeaderKey {
            keys,
            command,
            label: self.label.clone(),
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestDebugger {
    pub id: String,
    pub name: String,
    pub languages: Vec<String>,
    /// The program and its arguments. Empty for one the plugin drives
    /// itself.
    #[serde(default)]
    pub command: Vec<String>,
    /// How to install it, as a command to copy.
    #[serde(default)]
    pub install: String,
    /// Launch settings sent with every start.
    #[serde(default)]
    pub launch: toml::Table,
    /// The plugin that speaks to it: `dap` unless it says.
    #[serde(default)]
    pub driver: Option<String>,
    /// More to stop on, as functions to break in.
    #[serde(default)]
    pub stop_on: Vec<ManifestStop>,
}

/// Something more a debugger can stop on: a function to break in, such as
/// `rust_panic` for Rust's panics.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestStop {
    pub id: String,
    pub label: String,
    /// Every name it may have, as versions of the language differ.
    pub functions: Vec<String>,
    #[serde(default)]
    pub default: bool,
}

impl ManifestDebugger {
    pub(super) fn to_core(&self, plugin: &str) -> merlin_core::Debugger {
        merlin_core::Debugger {
            id: self.id.clone(),
            name: self.name.clone(),
            plugin: plugin.to_string(),
            languages: self.languages.clone(),
            command: self.command.clone(),
            install: self.install.clone(),
            launch: serde_json::to_string(&self.launch).unwrap_or_else(|_| "{}".into()),
            stops: self
                .stop_on
                .iter()
                .map(|s| merlin_core::FunctionStop {
                    id: s.id.clone(),
                    label: s.label.clone(),
                    functions: s.functions.clone(),
                    on: s.default,
                })
                .collect(),
            driver: self.driver.clone().unwrap_or_else(|| {
                if self.command.is_empty() {
                    plugin.to_string()
                } else {
                    merlin_core::DAP.to_string()
                }
            }),
        }
    }
}

/// The events that say what a manifest adds: its languages and its
/// debuggers.
pub(super) fn manifest_events(manifest: &Manifest) -> Vec<Event> {
    let mut events = Vec::new();
    let plugin = manifest.name.clone();
    let languages: Vec<_> = manifest
        .languages
        .iter()
        .map(ManifestLanguage::to_core)
        .collect();
    if !languages.is_empty() {
        events.push(Event::RegisterLanguages {
            plugin: plugin.clone(),
            languages,
        });
    }
    let keys: Vec<_> = manifest
        .keys
        .iter()
        .filter_map(|k| k.to_core(&plugin))
        .collect();
    if !keys.is_empty() {
        events.push(Event::RegisterKeys {
            plugin: plugin.clone(),
            keys,
        });
    }
    if let Some(command) = &manifest.symbols {
        events.push(Event::RegisterSymbolSearch {
            plugin: plugin.clone(),
            command: command.clone(),
        });
    }
    if !manifest.debuggers.is_empty() {
        events.push(Event::RegisterDebuggers {
            debuggers: manifest
                .debuggers
                .iter()
                .map(|d| d.to_core(&plugin))
                .collect(),
            plugin,
        });
    }
    events
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
    pub(super) fn to_core(&self) -> merlin_core::lang::Language {
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
    /// Drives debuggers: starts the ones plugins declare when the user
    /// starts a run, and reports the run.
    #[serde(default)]
    pub debugger: bool,
    /// Reads the project's files from disk, not only the open one.
    #[serde(default)]
    pub workspace_read: bool,
    /// Runs command lines in terminal tabs, as tasks do, when it likes.
    #[serde(default)]
    pub terminal: bool,
}

impl Permissions {
    pub(super) fn is_empty(&self) -> bool {
        *self == Permissions::default()
    }

    /// Whether these allow everything `wanted` asks for.
    pub(super) fn covers(&self, wanted: &Permissions) -> bool {
        (self.editor_read || !wanted.editor_read)
            && (self.editor_write || !wanted.editor_write)
            && (self.debugger || !wanted.debugger)
            && (self.workspace_read || !wanted.workspace_read)
            && (self.terminal || !wanted.terminal)
            && wanted.processes.iter().all(|p| self.processes.contains(p))
    }
}

/// Permissions the user allowed, by plugin folder.
#[derive(Default, Deserialize, Serialize)]
pub(super) struct Grants {
    #[serde(default)]
    pub(super) plugins: BTreeMap<String, Permissions>,
}

impl Grants {
    /// Whether the user allowed the plugin in `dir` everything `wanted`
    /// asks for. Each plugin asks for itself: one never borrows another's.
    pub(super) fn covers(&self, dir: &str, wanted: &Permissions) -> bool {
        self.plugins.get(dir).is_some_and(|g| g.covers(wanted))
    }
}

/// A plugin's settings: its manifest's defaults, with what the user set
/// over them.
pub(super) fn plugin_settings(manifest: &Manifest, plugins: &toml::Table) -> toml::Table {
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

pub(super) fn read_grants(data_dir: &Path) -> Grants {
    fs::read_to_string(data_dir.join("grants.toml"))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Remembers that the plugin in `dir` may do what `permissions` say.
pub(super) fn save_grant(data_dir: &Path, dir: &str, permissions: &Permissions) -> Result<()> {
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
    if p.debugger {
        allowed.push("start the debuggers plugins name, and drive them".to_string());
    }
    Ok((manifest, allowed))
}
