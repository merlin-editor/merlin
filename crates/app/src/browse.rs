//! Browse: the plugin registries in Settings › Plugins. merlin checks every
//! source off the window's thread at start and every few hours, installs
//! and updates there too, and works out what each plugin's row and page
//! say: installed, an update, withdrawn, blocked, or not built for this
//! merlin. See `docs/plugin-registry.md` section 7 and
//! `docs/mockups/PluginBrowse.dc.html`.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use merlin_core::Event;
use merlin_core::style::Hue;
use merlin_plugins::package::{self, Installed};
use merlin_plugins::registry::{self, Entry, Loaded, Source, StaleKind, Version};
use merlin_plugins::{API_VERSION, Config, Permissions};

use crate::jobs::{Finish, Lane};
use crate::{App, plugin_cli, settings};

/// How often merlin checks the registries while it runs.
pub(crate) const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The first check, a little after the window opens.
const FIRST_CHECK: Duration = Duration::from_secs(3);
/// Opening Browse checks again if the last check is older than this.
const STALE_AFTER: Duration = Duration::from_secs(10 * 60);
/// Status-bar progress is under this plugin name and id.
const PROGRESS: (&str, &str) = ("merlin", "registry");
/// The registry whose issues Report opens.
const REPORT_URL: &str = "https://github.com/merlin-editor/plugins/issues/new";

/// A loaded plugin's title and tile, by its name, to name updates as
/// the Installed list does.
pub(crate) type Titles<'a> = dyn Fn(&str) -> Option<(String, (String, Hue))> + 'a;

/// What the app keeps of the registries.
#[derive(Default)]
pub(crate) struct Browse {
    /// Each source's index as last checked: fresh, or its last good one.
    pub sources: Vec<Arc<Loaded>>,
    /// Sources with no index at all, and settings that name one wrongly:
    /// each said as a banner.
    pub failed: Vec<String>,
    pub checking: bool,
    pub checked: Option<SystemTime>,
    /// When to check next; none in scripts.
    pub due: Option<Instant>,
    /// The plugin installing now, and what waits after it.
    pub busy: Option<String>,
    pub queue: VecDeque<Job>,
    /// The Install? or Replace? card, while it's open.
    pub confirm: Option<Confirm>,
    /// The plugin whose page Browse shows: its source and name.
    pub picked: Option<(String, String)>,
    /// Updates passed over with Skip This Version: name to version.
    pub skipped: BTreeMap<String, String>,
    /// What `installed.toml` says, read again after each change.
    pub installed: BTreeMap<String, Installed>,
    /// Blocked plugins already told of, so it's said once.
    pub told: HashSet<String>,
    data_dir: PathBuf,
}

/// An install or update waiting its turn.
#[derive(Clone, Debug)]
pub(crate) struct Job {
    pub source: String,
    pub name: String,
    pub version: String,
}

/// A card asking before an install.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Confirm {
    pub source: String,
    pub name: String,
    pub version: String,
    pub title: String,
    pub kind: ConfirmKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ConfirmKind {
    /// It will run programs outside the sandbox: these.
    Outside(Vec<String>),
    /// It replaces the same name from another source.
    Replace { from: String },
}

/// A click in Browse, or on a registry plugin's banners.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BrowseAction {
    /// Check every source now.
    Check,
    /// Show a plugin's page: its source and name.
    Pick(String, String),
    /// Install the newest version; `confirmed` once the card said yes.
    Install {
        source: String,
        name: String,
        confirmed: bool,
    },
    /// Update one, allowing what it newly asks for.
    Update(String),
    /// Every update that asks for nothing new.
    UpdateAll,
    /// Hides an update until a newer one: name and version.
    Skip(String, String),
    /// Closes the card.
    Cancel,
    /// Opens a page in the browser: a repository, or Report.
    Open(String),
}

/// How a registry plugin's row reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    /// Not installed, with a version to install.
    Available,
    Installed,
    Update,
    /// Installed some other way, as a link or a folder of its own.
    Linked,
    /// Nothing built for this merlin's API.
    NotForThis,
}

/// A plugin in Browse's list.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowseRow {
    pub source: String,
    pub name: String,
    pub title: String,
    pub what: String,
    pub mark: (String, Hue),
    pub version: String,
    pub state: RowState,
    /// The API its newest version needs, when it isn't this one.
    pub api: Option<String>,
}

/// A version in a Versions table.
#[derive(Clone, Debug, PartialEq)]
pub struct VersionRow {
    pub version: String,
    pub api: String,
    /// The day it was published, as people read it.
    pub published: String,
    pub size: u64,
    pub note: VersionNote,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VersionNote {
    /// The one Install picks.
    Installs,
    Installed,
    Withdrawn(String),
    Blocked(String),
    /// For another API.
    Older,
    None,
}

/// What Browse's page says about a plugin.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowsePage {
    pub source: String,
    pub name: String,
    pub title: String,
    pub mark: (String, Hue),
    pub description: String,
    /// The version Install picks, if there's one.
    pub version: Option<String>,
    pub license: Option<String>,
    pub owners: Vec<String>,
    pub repository: Option<String>,
    /// What installing lets it do, each with whether it's outside the
    /// sandbox. Every permission it could have, so the ones it doesn't ask
    /// for read as not.
    pub permissions: Vec<(String, bool)>,
    pub not_asked: Vec<String>,
    pub versions: Vec<VersionRow>,
    pub state: RowState,
    /// Why there's nothing to install, when there isn't.
    pub none: Option<String>,
    /// The commit its newest version was built from.
    pub commit: Option<String>,
    pub report: Option<String>,
}

/// An update waiting in Installed.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateRow {
    pub name: String,
    pub title: String,
    pub mark: (String, Hue),
    pub from: String,
    pub to: String,
    /// What the new version asks for beyond what's allowed.
    pub more: Vec<(String, bool)>,
    /// The installed version was withdrawn, and why.
    pub withdrawn: Option<String>,
}

/// A banner over Browse's list.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceBanner {
    pub kind: StaleKind,
    pub text: String,
}

/// What a registry plugin's Installed page adds.
#[derive(Clone, Debug, PartialEq)]
pub struct RegistryPage {
    pub source: String,
    pub version: String,
    pub update: Option<UpdateRow>,
    pub withdrawn: Option<String>,
    pub blocked: Option<String>,
    pub versions: Vec<VersionRow>,
    pub repository: Option<String>,
    pub report: Option<String>,
}

/// Browse as the window draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrowseView {
    pub rows: Vec<BrowseRow>,
    pub page: Option<BrowsePage>,
    pub banners: Vec<SourceBanner>,
    /// "merlin registry · checked 2 min ago", or that it's checking.
    pub status: String,
    /// More than one source, so rows say which.
    pub many: bool,
    pub busy: Option<String>,
    pub confirm: Option<ConfirmView>,
    pub updates: Vec<UpdateRow>,
}

/// The card, as the window draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfirmView {
    pub title: String,
    pub text: String,
    pub items: Vec<String>,
    pub button: String,
    pub action: BrowseAction,
}

impl Browse {
    /// The registries as merlin last saw them, from the data folder,
    /// without the network: enough to know what's blocked at start.
    pub(crate) fn new(data_dir: PathBuf, user: &toml::Table, gui: bool) -> Self {
        let mut browse = Self {
            due: gui.then(|| Instant::now() + FIRST_CHECK),
            skipped: read_skipped(&data_dir),
            installed: package::installed(&data_dir),
            data_dir,
            ..Self::default()
        };
        if gui {
            let (sources, _) = registry::sources(user);
            browse.sources = sources
                .into_iter()
                .filter_map(|source| {
                    let index = registry::cached(&source, &browse.data_dir).ok()?;
                    Some(Arc::new(Loaded {
                        source,
                        index,
                        stale: None,
                    }))
                })
                .collect();
        }
        browse
    }

    fn loaded(&self, source: &str) -> Option<&Arc<Loaded>> {
        self.sources.iter().find(|l| l.source.name == source)
    }

    /// An installed plugin's registry entry, from the source it came from.
    fn entry_of(&self, name: &str) -> Option<(&Installed, &Arc<Loaded>, &Entry)> {
        let mine = self.installed.get(name)?;
        let loaded = self.loaded(&mine.source)?;
        let entry = loaded.index.plugins.get(name)?;
        Some((mine, loaded, entry))
    }

    /// Installed plugins a registry blocked, and why.
    pub(crate) fn blocked(&self) -> Vec<(String, String)> {
        self.installed
            .keys()
            .filter_map(|name| {
                let (mine, _, entry) = self.entry_of(name)?;
                let why = entry.blocked(&mine.version)?;
                Some((name.clone(), why.to_string()))
            })
            .collect()
    }

    pub(crate) fn is_blocked(&self, name: &str) -> Option<String> {
        self.blocked()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, why)| why)
    }

    /// Updates for installed plugins: withdrawn ones first, then by name.
    /// A skipped version shows only once a newer one is out.
    pub(crate) fn updates(&self, titles: &Titles) -> Vec<UpdateRow> {
        let config = Config::standard();
        let plugins = settings::plugins_dir();
        let mut rows: Vec<UpdateRow> = self
            .installed
            .keys()
            .filter_map(|name| {
                let (mine, _, entry) = self.entry_of(name)?;
                if entry.blocked(&mine.version).is_some() {
                    return None;
                }
                let newest = entry.newest()?;
                if registry::compare_versions(&newest.version, &mine.version)
                    != std::cmp::Ordering::Greater
                {
                    return None;
                }
                let withdrawn = entry.version(&mine.version).and_then(|v| v.yanked.clone());
                if withdrawn.is_none() && self.skipped.get(name) == Some(&newest.version) {
                    return None;
                }
                let allowed = plugins
                    .as_ref()
                    .and_then(|p| merlin_plugins::granted(&config, &p.join(name)))
                    .unwrap_or_default();
                let (title, mark) =
                    titles(name).unwrap_or_else(|| (entry.title.clone(), entry_mark(entry)));
                Some(UpdateRow {
                    name: name.clone(),
                    title,
                    mark,
                    from: mine.version.clone(),
                    to: newest.version.clone(),
                    more: newest.permissions.beyond(&allowed),
                    withdrawn,
                })
            })
            .collect();
        rows.sort_by(|a, b| {
            b.withdrawn
                .is_some()
                .cmp(&a.withdrawn.is_some())
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        rows
    }

    /// Every plugin from every source, A to Z, those blocked left out
    /// unless the filter names them.
    pub(crate) fn rows(&self, filter: &str, loaded: &dyn Fn(&str) -> bool) -> Vec<BrowseRow> {
        let filter = filter.trim().to_lowercase();
        let mut rows: Vec<BrowseRow> = self
            .sources
            .iter()
            .flat_map(|l| {
                l.index
                    .plugins
                    .iter()
                    .map(move |(name, entry)| (l, name, entry))
            })
            .filter(|(_, name, entry)| {
                if entry.blocked.is_some() {
                    return !filter.is_empty() && name.as_str() == filter;
                }
                filter.is_empty()
                    || [
                        name.as_str(),
                        &entry.title,
                        entry.description.as_deref().unwrap_or(""),
                    ]
                    .iter()
                    .any(|t| t.to_lowercase().contains(&filter))
            })
            .map(|(l, name, entry)| {
                let newest = entry.newest();
                let state = self.state(&l.source.name, name, entry, loaded(name));
                let api = newest
                    .is_none()
                    .then(|| newest_any(entry))
                    .flatten()
                    .map(|v| v.api.clone());
                BrowseRow {
                    source: l.source.name.clone(),
                    name: name.clone(),
                    title: entry.title.clone(),
                    what: entry.description.clone().unwrap_or_default(),
                    mark: entry_mark(entry),
                    version: newest
                        .or(newest_any(entry))
                        .map_or_else(String::new, |v| v.version.clone()),
                    state,
                    api,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then_with(|| a.source.cmp(&b.source))
        });
        rows
    }

    fn state(&self, source: &str, name: &str, entry: &Entry, loaded: bool) -> RowState {
        match self.installed.get(name) {
            Some(mine) if mine.source == source => match entry.newest() {
                Some(v)
                    if registry::compare_versions(&v.version, &mine.version)
                        == std::cmp::Ordering::Greater =>
                {
                    RowState::Update
                }
                _ => RowState::Installed,
            },
            // From another source, or from a file: Install replaces it.
            Some(_) if entry.newest().is_some() => RowState::Available,
            None if loaded => RowState::Linked,
            _ if entry.newest().is_none() => RowState::NotForThis,
            _ => RowState::Available,
        }
    }

    /// The page for the plugin picked, or the first row's.
    pub(crate) fn page(
        &self,
        rows: &[BrowseRow],
        loaded: &dyn Fn(&str) -> bool,
    ) -> Option<BrowsePage> {
        let (source, name) = self
            .picked
            .clone()
            .filter(|(s, n)| rows.iter().any(|r| r.source == *s && r.name == *n))
            .or_else(|| rows.first().map(|r| (r.source.clone(), r.name.clone())))?;
        let l = self.loaded(&source)?;
        let entry = l.index.plugins.get(&name)?;
        let newest = entry.newest();
        let shown = newest.or(newest_any(entry));
        let wants = shown.map(|v| v.permissions.clone()).unwrap_or_default();
        let permissions = wants.described();
        let not_asked = not_asked(&wants);
        let state = self.state(&source, &name, entry, loaded(&name));
        let mine = self
            .installed
            .get(&name)
            .filter(|m| m.source == source)
            .map(|m| m.version.as_str());
        let none = if newest.is_some() {
            None
        } else if let Some(why) = &entry.blocked {
            Some(format!(
                "Blocked by the {source} registry's maintainers: {why}"
            ))
        } else if let Some(v) = newest_any(entry).filter(|v| v.api != API_VERSION) {
            Some(format!(
                "Not yet updated for this version of merlin. It needs plugin API {API_VERSION}, and its newest, {}, is for {}.",
                v.version, v.api
            ))
        } else {
            Some("Every version is withdrawn or blocked, so there's nothing to install.".into())
        };
        Some(BrowsePage {
            report: report_url(&source, &name, shown.map(|v| v.version.as_str())),
            source,
            title: entry.title.clone(),
            mark: entry_mark(entry),
            description: entry.description.clone().unwrap_or_default(),
            version: newest.map(|v| v.version.clone()),
            license: entry.license.clone(),
            owners: entry.owners.clone(),
            repository: entry.repository.clone(),
            permissions,
            not_asked,
            versions: versions(entry, mine),
            state,
            none,
            commit: shown.and_then(|v| v.commit.clone()),
            name,
        })
    }

    /// What an installed plugin's page adds, if a registry installed it.
    pub(crate) fn registry_page(
        &self,
        name: &str,
        update: Option<UpdateRow>,
    ) -> Option<RegistryPage> {
        let (mine, l, entry) = self.entry_of(name)?;
        let version = entry.version(&mine.version);
        Some(RegistryPage {
            source: mine.source.clone(),
            version: mine.version.clone(),
            update,
            withdrawn: version.and_then(|v| v.yanked.clone()),
            blocked: entry.blocked(&mine.version).map(str::to_string),
            versions: versions(entry, Some(&mine.version)),
            repository: entry.repository.clone(),
            report: report_url(&l.source.name, name, Some(&mine.version)),
        })
    }

    /// The banners over Browse: sources it couldn't use.
    pub(crate) fn banners(&self) -> Vec<SourceBanner> {
        let mut banners: Vec<SourceBanner> = self
            .sources
            .iter()
            .filter_map(|l| {
                let stale = l.stale.as_ref()?;
                let name = &l.source.name;
                let since = l.index.generated.get(..10).map(day).unwrap_or_default();
                let text = match stale.kind {
                    StaleKind::Unreachable => format!(
                        "Couldn't reach the {name} registry. Showing its list from {since}. Installing needs it."
                    ),
                    StaleKind::Expired => format!(
                        "The {name} registry's list ran out on {} and hasn't been renewed. Showing it, but nothing installs from it.",
                        l.index.expires.get(..10).map(day).unwrap_or_default()
                    ),
                    StaleKind::Rejected => format!(
                        "The {name} registry's list didn't pass its checks, so merlin isn't using it: {}. Showing the last good one, from {since}.",
                        stale.why
                    ),
                };
                Some(SourceBanner {
                    kind: stale.kind,
                    text,
                })
            })
            .collect();
        for text in &self.failed {
            banners.push(SourceBanner {
                kind: StaleKind::Unreachable,
                text: text.clone(),
            });
        }
        banners
    }

    /// The line over Browse's list.
    pub(crate) fn status(&self) -> String {
        let names: Vec<&str> = self
            .sources
            .iter()
            .map(|l| l.source.name.as_str())
            .collect();
        let what = match names.as_slice() {
            [] => "No registry".to_string(),
            [one] => format!("{one} registry"),
            many => many.join(" · "),
        };
        if self.checking {
            return format!("{what} · checking…");
        }
        match self.checked.and_then(|t| t.elapsed().ok()) {
            Some(ago) => format!("{what} · checked {}", ago_text(ago)),
            None => what,
        }
    }

    /// Whether opening Browse should check again.
    pub(crate) fn stale(&self) -> bool {
        !self.checking
            && self
                .checked
                .and_then(|t| t.elapsed().ok())
                .is_none_or(|ago| ago > STALE_AFTER)
    }

    fn skip(&mut self, name: String, version: String) {
        self.skipped.insert(name, version);
        let text = toml::to_string(&self.skipped).unwrap_or_default();
        let _ = fs::create_dir_all(&self.data_dir);
        let _ = fs::write(self.data_dir.join("skipped-updates.toml"), text);
    }
}

fn read_skipped(data_dir: &std::path::Path) -> BTreeMap<String, String> {
    fs::read_to_string(data_dir.join("skipped-updates.toml"))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// The newest version of any API that isn't blocked or withdrawn, to say
/// what a plugin not built for this merlin is at.
fn newest_any(entry: &Entry) -> Option<&Version> {
    entry
        .versions
        .iter()
        .filter(|v| v.yanked.is_none() && v.blocked.is_none())
        .max_by(|a, b| registry::compare_versions(&a.version, &b.version))
}

/// A registry entry's tile.
fn entry_mark(entry: &Entry) -> (String, Hue) {
    merlin_core::plugin_mark(
        &entry.title,
        entry.badge.as_deref(),
        entry.hue.as_deref(),
        false,
    )
}

/// What a plugin doesn't ask for, of what most ask for, to show as not.
fn not_asked(wants: &Permissions) -> Vec<String> {
    let mut not = Vec::new();
    if !wants.editor_write {
        not.push("Change files".to_string());
    }
    if wants.processes.is_empty() && !wants.terminal {
        not.push("Run programs".to_string());
    }
    not
}

/// A plugin's versions, newest first, each with its note.
fn versions(entry: &Entry, mine: Option<&str>) -> Vec<VersionRow> {
    let picks = entry.newest().map(|v| v.version.clone());
    let mut rows: Vec<VersionRow> = entry
        .versions
        .iter()
        .map(|v| {
            let note = if let Some(why) = entry.blocked(&v.version) {
                VersionNote::Blocked(why.to_string())
            } else if let Some(why) = &v.yanked {
                VersionNote::Withdrawn(why.clone())
            } else if mine == Some(v.version.as_str()) {
                VersionNote::Installed
            } else if picks.as_deref() == Some(v.version.as_str()) {
                VersionNote::Installs
            } else if v.api != API_VERSION {
                VersionNote::Older
            } else {
                VersionNote::None
            };
            VersionRow {
                version: v.version.clone(),
                api: v.api.clone(),
                published: v
                    .published
                    .as_deref()
                    .and_then(|p| p.get(..10))
                    .map(day)
                    .unwrap_or_default(),
                size: v.size,
                note,
            }
        })
        .collect();
    rows.sort_by(|a, b| registry::compare_versions(&b.version, &a.version));
    rows
}

/// `2026-11-09` as "9 Nov 2026".
fn day(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = date.split('-');
    let (Some(y), Some(m), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
        return date.to_string();
    };
    match (m.parse::<usize>(), d.parse::<u32>()) {
        (Ok(m @ 1..=12), Ok(d)) => format!("{d} {} {y}", MONTHS[m - 1]),
        _ => date.to_string(),
    }
}

fn ago_text(ago: Duration) -> String {
    let mins = ago.as_secs() / 60;
    match mins {
        0 => "just now".into(),
        1 => "a minute ago".into(),
        2..=59 => format!("{mins} min ago"),
        60..=119 => "an hour ago".into(),
        _ => format!("{} hours ago", mins / 60),
    }
}

/// Report's link: an issue on merlin's own registry. Other sources have
/// their own ways, which merlin doesn't know.
fn report_url(source: &str, name: &str, version: Option<&str>) -> Option<String> {
    (source == registry::DEFAULT_SOURCE).then(|| {
        let what = match version {
            Some(v) => format!("{name}+{v}"),
            None => name.to_string(),
        };
        format!("{REPORT_URL}?title=Report%3A+{what}")
    })
}

impl App {
    /// The registries' state, worked out for the Settings tab.
    pub(crate) fn browse_view(&self) -> BrowseView {
        let host = self.host.plugins();
        let loaded = |name: &str| host.iter().any(|p| p.name == name);
        let titles = |name: &str| {
            host.iter()
                .find(|p| p.name == name)
                .map(|p| (p.title.clone(), crate::prefs::plugins::mark(p)))
        };
        let filter = self.settings_ui.filter_text().to_string();
        let rows = self.browse.rows(&filter, &loaded);
        let page = self.browse.page(&rows, &loaded);
        let confirm = self.browse.confirm.as_ref().map(confirm_view);
        BrowseView {
            many: self.browse.sources.len() > 1,
            page,
            rows,
            banners: self.browse.banners(),
            status: self.browse.status(),
            busy: self.browse.busy.clone(),
            confirm,
            updates: self.browse.updates(&titles),
        }
    }

    /// Checks every source, off the window's thread.
    pub(crate) fn check_registry_later(&mut self) {
        if self.scripted || self.browse.checking {
            return;
        }
        self.browse.checking = true;
        self.browse.due = Some(Instant::now() + CHECK_EVERY);
        let (sources, problems) = registry::sources(&self.loaded.user);
        let data = self.browse.data_dir.clone();
        self.jobs.run(Lane::Net, move || {
            let now = registry::now();
            let results: Vec<(Source, Result<Loaded, String>)> = sources
                .into_iter()
                .map(|source| {
                    let got = registry::load(&source, &data, &plugin_cli::fetch, now)
                        .map_err(|e| format!("{e:#}"));
                    (source, got)
                })
                .collect();
            Box::new(move |app: &mut App| app.registry_checked(results, problems)) as Finish
        });
    }

    fn registry_checked(
        &mut self,
        results: Vec<(Source, Result<Loaded, String>)>,
        problems: Vec<String>,
    ) -> Vec<Event> {
        let b = &mut self.browse;
        b.checking = false;
        b.checked = Some(SystemTime::now());
        b.sources.clear();
        b.failed.clear();
        for (source, got) in results {
            match got {
                Ok(loaded) => b.sources.push(Arc::new(loaded)),
                Err(why) => b.failed.push(format!(
                    "Couldn't reach the {} registry, and there's no list from before: {why}",
                    source.name
                )),
            }
        }
        b.failed.extend(
            problems
                .into_iter()
                .map(|p| format!("Skipping a plugin source: {p}")),
        );
        b.installed = package::installed(&b.data_dir);
        self.turn_off_blocked()
    }

    /// Turns off installed plugins a registry blocked, saying so once.
    pub(crate) fn turn_off_blocked(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for (name, why) in self.browse.blocked() {
            let Some(p) = self.host.plugins().into_iter().find(|p| p.name == name) else {
                continue;
            };
            if p.enabled {
                events.extend(self.host.set_enabled(&name, false));
            }
            if self.browse.told.insert(name.clone()) {
                let source = self
                    .browse
                    .installed
                    .get(&name)
                    .map_or("a", |m| m.source.as_str());
                events.push(Event::Error(format!(
                    "{} was turned off. The {source} registry blocked it: {why}",
                    p.title
                )));
            }
        }
        if !events.is_empty() {
            events.push(self.plugin_summaries());
        }
        events
    }

    pub(crate) fn browse_action(&mut self, action: BrowseAction) -> Vec<Event> {
        if !matches!(action, BrowseAction::Install { .. }) {
            self.browse.confirm = None;
        }
        match action {
            BrowseAction::Check => {
                self.check_registry_later();
                Vec::new()
            }
            BrowseAction::Pick(source, name) => {
                self.browse.picked = Some((source, name));
                self.browse.confirm = None;
                self.settings_ui.scroll = 0.0;
                Vec::new()
            }
            BrowseAction::Install {
                source,
                name,
                confirmed,
            } => self.install_from(source, name, confirmed),
            BrowseAction::Update(name) => {
                let Some((mine, l, entry)) = self.browse.entry_of(&name) else {
                    return Vec::new();
                };
                let Some(v) = entry.newest() else {
                    return Vec::new();
                };
                let _ = mine;
                let job = Job {
                    source: l.source.name.clone(),
                    name: name.clone(),
                    version: v.version.clone(),
                };
                self.queue_install(vec![job])
            }
            BrowseAction::UpdateAll => {
                let jobs: Vec<Job> = self
                    .browse
                    .updates(&|_| None)
                    .into_iter()
                    .filter(|u| u.more.is_empty())
                    .filter_map(|u| {
                        let (_, l, _) = self.browse.entry_of(&u.name)?;
                        Some(Job {
                            source: l.source.name.clone(),
                            name: u.name,
                            version: u.to,
                        })
                    })
                    .collect();
                self.queue_install(jobs)
            }
            BrowseAction::Skip(name, version) => {
                self.browse.skip(name, version);
                Vec::new()
            }
            BrowseAction::Cancel => Vec::new(),
            // Only web pages: an index names them, and a source could be
            // anyone's.
            BrowseAction::Open(url) if url.starts_with("https://") => {
                crate::open_folder(std::path::Path::new(&url));
                Vec::new()
            }
            BrowseAction::Open(_) => Vec::new(),
        }
    }

    /// Install from Browse: asks first if it runs programs outside the
    /// sandbox, or replaces the same name from another source.
    fn install_from(&mut self, source: String, name: String, confirmed: bool) -> Vec<Event> {
        self.browse.confirm = None;
        // Its page, so it becomes its Installed page once it's in.
        self.browse.picked = Some((source.clone(), name.clone()));
        let Some(l) = self.browse.loaded(&source) else {
            return Vec::new();
        };
        let Some(entry) = l.index.plugins.get(&name) else {
            return Vec::new();
        };
        let Some(v) = entry.newest() else {
            return Vec::new();
        };
        if !confirmed {
            let card = |kind| Confirm {
                source: source.clone(),
                name: name.clone(),
                version: v.version.clone(),
                title: entry.title.clone(),
                kind,
            };
            if let Some(mine) = self
                .browse
                .installed
                .get(&name)
                .filter(|m| m.source != source)
            {
                self.browse.confirm = Some(card(ConfirmKind::Replace {
                    from: mine.source.clone(),
                }));
                return Vec::new();
            }
            let outside: Vec<String> = v
                .permissions
                .described()
                .into_iter()
                .filter(|(_, out)| *out)
                .map(|(what, _)| what)
                .collect();
            if !outside.is_empty() {
                self.browse.confirm = Some(card(ConfirmKind::Outside(outside)));
                return Vec::new();
            }
        }
        let job = Job {
            source,
            name,
            version: v.version.clone(),
        };
        self.queue_install(vec![job])
    }

    fn queue_install(&mut self, jobs: Vec<Job>) -> Vec<Event> {
        for job in jobs {
            let waiting = self.browse.busy.as_deref() == Some(job.name.as_str())
                || self.browse.queue.iter().any(|q| q.name == job.name);
            if !waiting {
                self.browse.queue.push_back(job);
            }
        }
        self.next_install()
    }

    /// Starts the next install, if none is running.
    fn next_install(&mut self) -> Vec<Event> {
        if self.browse.busy.is_some() {
            return Vec::new();
        }
        let Some(job) = self.browse.queue.pop_front() else {
            return Vec::new();
        };
        let Some(l) = self.browse.loaded(&job.source).cloned() else {
            return self.next_install();
        };
        let Some(entry) = l.index.plugins.get(&job.name) else {
            return self.next_install();
        };
        let Some(version) = entry.version(&job.version).cloned() else {
            return self.next_install();
        };
        let Some(plugins) = settings::plugins_dir() else {
            return vec![Event::Error("Can't install: there's no home folder".into())];
        };
        let title = entry.title.clone();
        let updating = self
            .browse
            .installed
            .get(&job.name)
            .map(|m| m.version.clone());
        self.browse.busy = Some(job.name.clone());
        let data = self.browse.data_dir.clone();
        let name = job.name.clone();
        let what = if updating.is_some() {
            "Updating"
        } else {
            "Installing"
        };
        let start = Event::ProgressStart {
            plugin: PROGRESS.0.into(),
            id: PROGRESS.1.into(),
            title: format!("{what} {title}"),
            cancellable: false,
        };
        self.jobs.run(Lane::Net, move || {
            let result = (|| {
                fs::create_dir_all(&plugins)?;
                let manifest =
                    registry::install(&l, &name, &version, &plugins, &data, &plugin_cli::fetch)?;
                // Installing is the user's say-so, as on the command line.
                merlin_plugins::grant(&Config::standard(), &plugins.join(&name))?;
                anyhow::Ok(manifest)
            })()
            .map_err(|e| format!("{e:#}"));
            Box::new(move |app: &mut App| app.install_done(name, title, updating, result)) as Finish
        });
        vec![start]
    }

    fn install_done(
        &mut self,
        name: String,
        title: String,
        updating: Option<String>,
        result: Result<merlin_plugins::Manifest, String>,
    ) -> Vec<Event> {
        self.browse.busy = None;
        let mut events = vec![Event::ProgressEnd {
            plugin: PROGRESS.0.into(),
            id: Some(PROGRESS.1.into()),
        }];
        match result {
            Err(why) => {
                let what = if updating.is_some() {
                    "update"
                } else {
                    "install"
                };
                events.push(Event::Error(format!("Can't {what} {title}: {why}")));
            }
            Ok(manifest) => {
                self.browse.installed = package::installed(&self.browse.data_dir);
                // The new version starts in place of the old.
                events.extend(self.host.unload(&name));
                if let Some(dir) = settings::plugins_dir().map(|p| p.join(&name)) {
                    match self.host.load(&dir) {
                        Ok(more) => events.extend(more),
                        Err(err) => events.push(Event::Error(format!(
                            "{} installed, but didn't start: {err:#}",
                            manifest.title
                        ))),
                    }
                }
                events.extend(self.refresh_looks());
                events.push(Event::Notify(match updating {
                    Some(from) => format!(
                        "Updated {} from {from} to {}",
                        manifest.title, manifest.version
                    ),
                    None => format!("Installed {} {}", manifest.title, manifest.version),
                }));
                // Installed from Browse, its page is now its Installed one.
                if self.browse.picked.as_ref().is_some_and(|(_, n)| *n == name)
                    && self.settings_ui.list_tab == crate::prefs::ListTab::Browse
                {
                    self.settings_ui.list_tab = crate::prefs::ListTab::Installed;
                    self.settings_ui.plugin = Some(name);
                }
                events.push(self.plugin_summaries());
            }
        }
        events.extend(self.next_install());
        events
    }
}

fn confirm_view(c: &Confirm) -> ConfirmView {
    let action = BrowseAction::Install {
        source: c.source.clone(),
        name: c.name.clone(),
        confirmed: true,
    };
    match &c.kind {
        ConfirmKind::Outside(items) => ConfirmView {
            title: format!("Install {}?", c.title),
            text: "It will be able to:".into(),
            items: items.clone(),
            button: "Install".into(),
            action,
        },
        ConfirmKind::Replace { from } => ConfirmView {
            title: format!("Replace {} from {from} with {}'s?", c.title, c.source),
            text: format!(
                "Its settings stay. It will update only from {}, and is allowed what {} {} asks for.",
                c.source, c.source, c.version
            ),
            items: Vec::new(),
            button: "Replace".into(),
            action,
        },
    }
}


/// Where the plugins that come with merlin are: `plugins/` beside the
/// executable, as the release archive has it, or the copy `install.sh`
/// keeps in the data folder. `MERLIN_BUNDLE` names another.
fn bundle_dir(data_dir: &std::path::Path) -> Option<PathBuf> {
    use merlin_plugins::bundled::LIST;
    if let Some(dir) = std::env::var_os("MERLIN_BUNDLE") {
        return Some(dir.into());
    }
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let beside = exe.parent()?.join("plugins");
    if beside.join(LIST).is_file() {
        return Some(beside);
    }
    let kept = data_dir.join("bundled");
    kept.join(LIST).is_file().then_some(kept)
}

/// Installs the plugins that come with merlin that aren't installed yet,
/// or are older, granting what they ask for: they're as trusted as merlin
/// itself. Says so in one notice, naming what runs outside the sandbox.
/// Runs before plugins load.
pub(crate) fn install_bundled(config: &Config) -> Vec<Event> {
    let (Some(dir), Some(plugins)) = (bundle_dir(&config.data_dir), settings::plugins_dir()) else {
        return Vec::new();
    };
    let done = merlin_plugins::bundled::install(&dir, &plugins, &config.data_dir);
    let mut events: Vec<Event> = done
        .failed
        .iter()
        .map(|(name, why)| {
            Event::Error(format!(
                "Couldn't install {name}, which comes with merlin: {why}"
            ))
        })
        .collect();
    if done.installed.is_empty() {
        return events;
    }
    // What runs outside the sandbox: programs by name, and which plugins
    // run command lines in a terminal.
    let mut programs: Vec<String> = Vec::new();
    let mut terminals: Vec<String> = Vec::new();
    for (manifest, _) in &done.installed {
        if let Err(err) = merlin_plugins::grant(config, &plugins.join(&manifest.name)) {
            events.push(Event::Error(format!(
                "{} installed, but what it may do couldn't be saved: {err:#}",
                manifest.title
            )));
        }
        let p = &manifest.permissions;
        programs.extend(
            p.processes
                .iter()
                .map(|prog| prog.rsplit(['/', '\\']).next().unwrap_or(prog).to_string()),
        );
        if p.terminal {
            terminals.push(manifest.title.clone());
        }
    }
    programs.sort();
    programs.dedup();
    let n = done.installed.len();
    let updated = done
        .installed
        .iter()
        .filter(|(_, from)| from.is_some())
        .count();
    let plugins_word = if n == 1 { "plugin" } else { "plugins" };
    let mut text = if updated == n {
        format!("Updated {n} {plugins_word} that come with merlin.")
    } else {
        format!("Installed {n} {plugins_word} that come with merlin.")
    };
    if !programs.is_empty() {
        text += &format!(
            " Outside the sandbox, they may run {}.",
            list_words(&programs)
        );
    }
    if !terminals.is_empty() {
        text += &format!(
            " {} may run commands in a terminal.",
            list_words(&terminals)
        );
    }
    events.push(Event::NotifyWith {
        text,
        actions: vec![("Review".into(), "plugins.manage".into())],
    });
    events
}

/// "a, b and c".
fn list_words(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_read_as_people_write_them() {
        assert_eq!(day("2026-11-09"), "9 Nov 2026");
        assert_eq!(day("soon"), "soon");
    }

    #[test]
    fn only_merlins_registry_has_report() {
        assert_eq!(
            report_url("merlin", "go", Some("0.1.0")).unwrap(),
            "https://github.com/merlin-editor/plugins/issues/new?title=Report%3A+go+0.1.0"
        );
        assert!(report_url("acme", "go", None).is_none());
    }
}
