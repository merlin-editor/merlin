//! A plugin's `plugin.toml`: what it adds, the permissions it asks for, and
//! the grants the user gave, remembered by its folder.

use super::*;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Two letters for its tile in the plugin lists, such as `RS`; the
    /// title's first letters without.
    #[serde(default)]
    pub badge: Option<String>,
    /// The tile's colour: blue, green, yellow, orange, red, purple, cyan
    /// or grey.
    #[serde(default)]
    pub hue: Option<String>,
    pub api: String,
    /// The component file, relative to the plugin folder. A plugin that
    /// only adds data, such as colour themes, has none.
    #[serde(default)]
    pub component: Option<PathBuf>,
    #[serde(default)]
    pub permissions: Permissions,
    /// Default settings, which the user's and the workspace's
    /// `[plugins.<name>]` override. A setting may be a table describing
    /// it, with `type` and `default`: see [`SettingSpec`]. Once read, this
    /// has the defaults only.
    #[serde(default)]
    pub settings: toml::Table,
    /// Its settings as the Settings page shows them, described or not.
    #[serde(skip)]
    pub schema: Vec<SettingSpec>,
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
    /// How to read problems in tasks' output, as VS Code's problem
    /// matchers do: rustc's, say, or Python's tracebacks.
    #[serde(default, rename = "problem-matchers")]
    pub problem_matchers: Vec<ManifestMatcher>,
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
    /// Views the plugin draws, each where it goes: see [`ManifestView`].
    #[serde(default)]
    pub views: Vec<ManifestView>,
}

/// A view, as `[[views]]` in a manifest declares it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestView {
    pub id: String,
    pub title: String,
    /// An SVG file in the plugin's folder, a glyph's name such as `plus`,
    /// or a character or two.
    #[serde(default)]
    pub icon: Option<String>,
    /// `left`, `right` (the default), `bottom` or `tab`.
    #[serde(default, rename = "where")]
    pub place: Option<String>,
    /// What it says while the plugin gives it nothing to show.
    #[serde(default)]
    pub empty: String,
    #[serde(default)]
    pub toolbar: Vec<ManifestTool>,
}

/// A button in a view's header, as `[[views.toolbar]]` declares it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestTool {
    /// One of the plugin's commands, with or without its name before it.
    pub command: String,
    #[serde(default)]
    pub icon: Option<String>,
}

impl ManifestView {
    pub(super) fn to_core(&self, plugin: &str, dir: &Path) -> merlin_core::ViewSpec {
        let icon = |text: &Option<String>| {
            text.as_deref()
                .and_then(|t| merlin_core::ViewIcon::parse(t, dir))
        };
        merlin_core::ViewSpec {
            id: self.id.clone(),
            title: self.title.clone(),
            icon: icon(&self.icon),
            place: self
                .place
                .as_deref()
                .and_then(merlin_core::Place::parse)
                .unwrap_or_default(),
            empty: self.empty.clone(),
            toolbar: self
                .toolbar
                .iter()
                .map(|t| merlin_core::ToolButton {
                    command: t
                        .command
                        .strip_prefix(&format!("{plugin}."))
                        .unwrap_or(&t.command)
                        .to_string(),
                    icon: icon(&t.icon),
                })
                .collect(),
        }
    }
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
    let matchers: Vec<_> = manifest
        .problem_matchers
        .iter()
        .filter_map(|m| m.to_core().ok())
        .collect();
    if !matchers.is_empty() {
        events.push(Event::RegisterProblemMatchers {
            plugin: plugin.clone(),
            matchers,
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

/// A problem matcher, as `[[problem-matchers]]` declares it: `lines` one
/// after another, each a `regex` and the groups with the problem's
/// `file`, `line`, `column`, `severity`, `code` and `message`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestMatcher {
    pub lines: Vec<ManifestMatcherLine>,
    /// A pattern for lines that may come between them.
    #[serde(default)]
    pub between: Option<String>,
    /// A page about a problem's code, with `{code}` where it goes.
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestMatcherLine {
    pub regex: String,
    #[serde(default)]
    pub file: Option<usize>,
    #[serde(default)]
    pub line: Option<usize>,
    #[serde(default)]
    pub column: Option<usize>,
    #[serde(default)]
    pub severity: Option<usize>,
    #[serde(default)]
    pub code: Option<usize>,
    #[serde(default)]
    pub message: Option<usize>,
}

impl ManifestMatcher {
    pub(super) fn to_core(&self) -> Result<merlin_core::lang::ProblemMatcher> {
        if self.lines.is_empty() {
            bail!("a problem matcher has no lines");
        }
        let regex = |r: &str| {
            regex::Regex::new(r).with_context(|| format!("problem matcher pattern {r:?}"))
        };
        Ok(merlin_core::lang::ProblemMatcher {
            lines: self
                .lines
                .iter()
                .map(|l| {
                    Ok(merlin_core::lang::MatcherLine {
                        regex: regex(&l.regex)?,
                        file: l.file,
                        line: l.line,
                        column: l.column,
                        severity: l.severity,
                        code: l.code,
                        message: l.message,
                    })
                })
                .collect::<Result<_>>()?,
            between: self.between.as_deref().map(regex).transpose()?,
            link: self.link.clone(),
        })
    }
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
    #[serde(default)]
    pub indent_after: Vec<String>,
    #[serde(default)]
    pub exception: Option<String>,
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
            indent_after: self.indent_after.clone(),
            exception: self.exception.clone(),
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

/// A plugin's settings: its manifest's defaults, with the user's over
/// them, then the workspace's, except for settings only the user sets.
pub(super) fn plugin_settings(
    manifest: &Manifest,
    user: &toml::Table,
    workspace: &toml::Table,
) -> toml::Table {
    let mut settings = manifest.settings.clone();
    if let Some(toml::Value::Table(set)) = user.get(&manifest.name) {
        merge(&mut settings, set);
    }
    if let Some(toml::Value::Table(set)) = workspace.get(&manifest.name) {
        let mut set = set.clone();
        for spec in manifest.schema.iter().filter(|s| s.user_only) {
            remove_path(&mut set, &spec.key);
        }
        merge(&mut settings, &set);
    }
    settings
}

/// What a setting holds, as the manifest's `type` says.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingKind {
    Flag,
    Text,
    /// A file or program on this computer.
    Path,
    Integer {
        min: Option<i64>,
        max: Option<i64>,
    },
    Float {
        min: Option<f64>,
        max: Option<f64>,
    },
    /// One of these values, each with its label.
    Choice(Vec<(String, String)>),
    /// A list of strings.
    List,
    /// Kept in the system keyring, never in a settings file, and shown as
    /// dots. Plugins read it with `get-secret`.
    Secret,
}

/// A setting as the Settings page shows it:
///
/// ```toml
/// [settings.check-command]
/// type = "choice"           # flag, string, path, integer, float, choice, list, secret
/// default = "check"
/// title = "Check on save"
/// choices = [{ value = "check", label = "cargo check" }, "clippy"]
/// description = "What rust-analyzer runs when you save."
/// scope = "workspace"       # or user-only
/// ```
///
/// A plain value under `[settings]` is a setting too, its type taken from
/// its default.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingSpec {
    /// Dotted for a setting inside a table, as `style.loud`.
    pub key: String,
    pub title: String,
    pub kind: SettingKind,
    pub default: toml::Value,
    pub description: String,
    /// Set for the user only, never by a workspace: a program's path, say.
    pub user_only: bool,
}

/// The settings every plugin that runs has without declaring them.
pub fn common_settings() -> Vec<SettingSpec> {
    let choice = |pairs: &[(&str, &str)]| {
        SettingKind::Choice(
            pairs
                .iter()
                .map(|(v, l)| (v.to_string(), l.to_string()))
                .collect(),
        )
    };
    vec![
        SettingSpec {
            key: "notices".into(),
            title: "Notices".into(),
            kind: choice(&[("all", "All"), ("errors", "Errors only"), ("none", "None")]),
            default: "all".into(),
            description: "The rest go to its Output channel.".into(),
            user_only: false,
        },
        SettingSpec {
            key: "log-level".into(),
            title: "Log level".into(),
            kind: choice(&[
                ("trace", "Trace"),
                ("debug", "Debug"),
                ("info", "Info"),
                ("warn", "Warn"),
                ("error", "Error"),
            ]),
            default: "info".into(),
            description: "How much it writes to Output.".into(),
            user_only: false,
        },
    ]
}

/// The described settings in `[settings]`, and every setting's default.
fn read_schema(table: &toml::Table) -> Result<(Vec<SettingSpec>, toml::Table)> {
    let mut schema = Vec::new();
    let mut defaults = toml::Table::new();
    read_level(table, "", &mut schema, &mut defaults)?;
    Ok((schema, defaults))
}

fn read_level(
    table: &toml::Table,
    prefix: &str,
    schema: &mut Vec<SettingSpec>,
    defaults: &mut toml::Table,
) -> Result<()> {
    for (name, value) in table {
        let key = format!("{prefix}{name}");
        let described = value
            .as_table()
            .filter(|t| t.get("type").is_some_and(toml::Value::is_str));
        if let Some(t) = described {
            let spec = described_setting(&key, t).with_context(|| format!("setting {key}"))?;
            if spec.kind != SettingKind::Secret {
                defaults.insert(name.clone(), spec.default.clone());
            }
            schema.push(spec);
        } else if let toml::Value::Table(inner) = value {
            let mut sub = toml::Table::new();
            read_level(inner, &format!("{key}."), schema, &mut sub)?;
            defaults.insert(name.clone(), toml::Value::Table(sub));
        } else {
            defaults.insert(name.clone(), value.clone());
            schema.push(SettingSpec {
                title: title_of(name),
                kind: kind_of(value),
                default: value.clone(),
                description: String::new(),
                user_only: false,
                key,
            });
        }
    }
    Ok(())
}

fn described_setting(key: &str, t: &toml::Table) -> Result<SettingSpec> {
    let text = |k: &str| t.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let int = |k: &str| t.get(k).and_then(toml::Value::as_integer);
    let float = |k: &str| {
        t.get(k)
            .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
    };
    let ty = text("type").unwrap_or_default();
    let kind = match ty.as_str() {
        "flag" | "bool" | "boolean" => SettingKind::Flag,
        "string" | "text" => SettingKind::Text,
        "path" => SettingKind::Path,
        "integer" => SettingKind::Integer {
            min: int("min"),
            max: int("max"),
        },
        "float" | "number" => SettingKind::Float {
            min: float("min"),
            max: float("max"),
        },
        "choice" => {
            let list = t
                .get("choices")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow!("a choice needs `choices`"))?;
            let choices = list
                .iter()
                .map(|c| match c {
                    toml::Value::String(v) => Some((v.clone(), title_of(v))),
                    toml::Value::Table(c) => {
                        let value = c.get("value")?.as_str()?.to_string();
                        let label = c
                            .get("label")
                            .and_then(|l| l.as_str())
                            .map_or_else(|| title_of(&value), str::to_string);
                        Some((value, label))
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| anyhow!("each choice is a string or {{ value, label }}"))?;
            SettingKind::Choice(choices)
        }
        "list" => SettingKind::List,
        "secret" => SettingKind::Secret,
        other => bail!(
            "unknown type `{other}`: flag, string, path, integer, float, choice, list or secret"
        ),
    };
    let default = match t.get("default") {
        Some(v) => v.clone(),
        None => match &kind {
            SettingKind::Flag => false.into(),
            SettingKind::Text | SettingKind::Path | SettingKind::Secret => "".into(),
            SettingKind::Integer { min, .. } => min.unwrap_or(0).into(),
            SettingKind::Float { min, .. } => min.unwrap_or(0.0).into(),
            SettingKind::Choice(c) => c.first().map_or("", |(v, _)| v.as_str()).into(),
            SettingKind::List => toml::Value::Array(Vec::new()),
        },
    };
    let user_only = match text("scope").as_deref() {
        // A secret is in your keyring, which no workspace sets.
        _ if kind == SettingKind::Secret => true,
        None | Some("workspace") => false,
        Some("user-only" | "user") => true,
        Some(other) => bail!("unknown scope `{other}`: workspace or user-only"),
    };
    Ok(SettingSpec {
        key: key.to_string(),
        title: text("title").unwrap_or_else(|| title_of(key.rsplit('.').next().unwrap_or(key))),
        kind,
        default,
        description: text("description").unwrap_or_default(),
        user_only,
    })
}

/// A setting's type from its default.
fn kind_of(value: &toml::Value) -> SettingKind {
    match value {
        toml::Value::Boolean(_) => SettingKind::Flag,
        toml::Value::Integer(_) => SettingKind::Integer {
            min: None,
            max: None,
        },
        toml::Value::Float(_) => SettingKind::Float {
            min: None,
            max: None,
        },
        toml::Value::Array(_) => SettingKind::List,
        _ => SettingKind::Text,
    }
}

/// `check-command` reads "Check command".
fn title_of(key: &str) -> String {
    let s = key.replace(['-', '_'], " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => s,
    }
}

/// Takes a dotted key out of a table.
fn remove_path(table: &mut toml::Table, key: &str) {
    match key.split_once('.') {
        Some((head, rest)) => {
            if let Some(toml::Value::Table(inner)) = table.get_mut(head) {
                remove_path(inner, rest);
            }
        }
        None => {
            table.remove(key);
        }
    }
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
    let manifest: Manifest = toml::from_str(
        &fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    let mut manifest = manifest;
    let (schema, defaults) =
        read_schema(&manifest.settings).with_context(|| format!("in {}", path.display()))?;
    manifest.schema = schema;
    manifest.settings = defaults;
    for matcher in &manifest.problem_matchers {
        matcher
            .to_core()
            .with_context(|| format!("in {}", path.display()))?;
    }
    check_views(&manifest).with_context(|| format!("in {}", path.display()))?;
    Ok(manifest)
}

/// Views need a component to draw them, an id of their own, and a place
/// merlin knows.
fn check_views(manifest: &Manifest) -> Result<()> {
    if !manifest.views.is_empty() && manifest.component.is_none() {
        bail!("views need a component to draw them");
    }
    let mut ids = Vec::new();
    for view in &manifest.views {
        let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if view.id.is_empty() || !view.id.chars().all(ok) {
            bail!(
                "view id {:?} must be lowercase letters, digits, - and _",
                view.id
            );
        }
        if ids.contains(&&view.id) {
            bail!("two views have the id {:?}", view.id);
        }
        ids.push(&view.id);
        if let Some(place) = &view.place
            && merlin_core::Place::parse(place).is_none()
        {
            bail!(
                "view {:?} has where = {place:?}; it can be left, right, bottom or tab",
                view.id
            );
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use merlin_core::Severity;
    use merlin_core::editor::{Found, problems};

    use super::*;

    /// What a plugin's own matchers find in `out`.
    fn found(plugin: &str, out: &str) -> Vec<Found> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../plugins")
            .join(plugin);
        let manifest = super::manifest(&dir).unwrap();
        let matchers: Vec<_> = manifest
            .problem_matchers
            .iter()
            .map(|m| m.to_core().unwrap())
            .collect();
        problems(out, &matchers.iter().collect::<Vec<_>>())
    }

    #[test]
    fn rust_reads_rustcs_problems() {
        let out = "error[E0308]: mismatched types\n \
 --> src/main.rs:5:23\n\
warning: unused variable: `x`\n \
 --> src/lib.rs:3:9\n\
error: could not compile `app` (bin \"app\") due to 1 previous error\n";
        let found = found("rust", out);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].path, PathBuf::from("src/main.rs"));
        assert_eq!((found[0].line, found[0].column), (4, 22));
        assert_eq!(found[0].code, "E0308");
        assert_eq!(found[0].message, "mismatched types");
        assert_eq!(
            found[0].link,
            "https://doc.rust-lang.org/error_codes/E0308.html"
        );
        assert_eq!(found[1].severity, Severity::Warning);
    }

    #[test]
    fn javascript_reads_typescripts_problems() {
        let found = found(
            "javascript",
            "src/a.ts(3,5): error TS2322: Type 'string' is not 'number'.\n",
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].line, found[0].column), (2, 4));
        assert_eq!(found[0].code, "TS2322");
        assert_eq!(found[0].message, "Type 'string' is not 'number'.");
    }

    #[test]
    fn python_reads_a_traceback_at_the_projects_last_frame() {
        let out = "Traceback (most recent call last):\n  \
File \"app/main.py\", line 9, in <module>\n    main()\n  \
File \"app/main.py\", line 4, in main\n    print(nme)\n  \
File \"/usr/lib/python3/x.py\", line 1, in f\n\
NameError: name 'nme' is not defined\n";
        let found = found("python", out);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].path, PathBuf::from("app/main.py"));
        assert_eq!(found[0].line, 3);
        assert_eq!(found[0].message, "NameError: name 'nme' is not defined");
    }

    #[test]
    fn a_bad_pattern_stops_the_plugin_loading() {
        let dir = std::env::temp_dir().join(format!("merlin-matcher-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("plugin.toml"),
            "name = \"bad\"\ntitle = \"Bad\"\napi = \"0.10\"\n\
             [[problem-matchers]]\nlines = [{ regex = '(', file = 1 }]\n",
        )
        .unwrap();
        let err = format!("{:#}", super::manifest(&dir).unwrap_err());
        assert!(err.contains("problem matcher pattern"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }
}
