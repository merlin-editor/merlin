//! Updating merlin itself.
//!
//! Each release carries `SHA256SUMS`, the SHA-256 of each of its archives,
//! and `SHA256SUMS.minisig`, a minisign signature over it by the release
//! key, whose trusted comment is `merlin <tag>`. An archive counts as that
//! release's only when the list is signed by the key merlin was built with,
//! the comment names the release asked for, and the archive's hash is on
//! the list. A checksum beside the archive on the same page would prove
//! nothing against someone who can change the page.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use merlin_core::{Event, UpdateAction, UpdateOffer, UpdateStep};

use crate::App;
use crate::jobs::{Finish, Lane};

/// Where merlin asks for its newest release.
const LATEST: &str = "https://api.github.com/repos/merlin-editor/merlin/releases/latest";
/// What installs merlin, for a copy that can't replace itself.
pub(crate) const INSTALL: &str = "curl -fsSL https://raw.githubusercontent.com/merlin-editor/merlin/main/scripts/install.sh | sh";
/// How often merlin looks while it runs, and its first look after it starts.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const FIRST_CHECK: Duration = Duration::from_secs(10);
/// How often the window reads an update's progress while one is under way.
const POLL: Duration = Duration::from_millis(200);
/// The largest archive, list and release description merlin reads.
const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
const MAX_SMALL: u64 = 1024 * 1024;
/// Left beside the executable while updating, and removed at the next start.
const STAGING: &str = ".merlin-update";
const OLD: &str = ".merlin-old";

/// The release keys' public halves, as `minisign -G` wrote them: the one
/// the release workflow signs with (`keys/release.pub`, which it checks
/// with too), and the one kept offline to rotate to if that one must go
/// (`keys/release-next.pub`). A release signed by either installs.
const RELEASE_PUBS: [&str; 2] = [
    include_str!("../../../keys/release.pub"),
    include_str!("../../../keys/release-next.pub"),
];

/// The base64 public keys from [`RELEASE_PUBS`]: each file's line after
/// its comment.
pub fn release_keys() -> [&'static str; 2] {
    RELEASE_PUBS.map(|file| {
        file.lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
            .unwrap_or_default()
    })
}

/// Checks `sums` against `signature` with any of `keys` and that it was
/// signed for `tag`, and reads it: each archive's file name to its
/// SHA-256, in hex.
pub fn check_sums(
    keys: &[&str],
    sums: &[u8],
    signature: &[u8],
    tag: &str,
) -> Result<BTreeMap<String, String>> {
    let sig_text = std::str::from_utf8(signature).context("the signature isn't text")?;
    let sig = minisign_verify::Signature::decode(sig_text)
        .map_err(|e| anyhow::anyhow!("the signature can't be read: {e}"))?;
    let signed = keys.iter().any(|k| {
        minisign_verify::PublicKey::from_base64(k)
            .is_ok_and(|key| key.verify(sums, &sig, false).is_ok())
    });
    if !signed {
        bail!("{tag}'s SHA256SUMS isn't signed by merlin's release key");
    }
    let want = format!("merlin {tag}");
    if sig.trusted_comment() != want {
        bail!(
            "{tag}'s SHA256SUMS was signed for {:?}, not {want:?}",
            sig.trusted_comment()
        );
    }
    let text = std::str::from_utf8(sums).context("SHA256SUMS isn't text")?;
    let mut out = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        // `sha256sum` writes `<hex>  <name>`, or `<hex> *<name>` in binary mode.
        let (hex, name) = line
            .split_once(' ')
            .with_context(|| format!("SHA256SUMS has a line it can't read: {line:?}"))?;
        let name = name.trim_start_matches([' ', '*']);
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) || name.is_empty() {
            bail!("SHA256SUMS has a line it can't read: {line:?}");
        }
        out.insert(name.to_string(), hex.to_ascii_lowercase());
    }
    Ok(out)
}

/// Checks that `bytes` are the archive `name` from a list [`check_sums`]
/// read.
pub fn check_archive(sums: &BTreeMap<String, String>, name: &str, bytes: &[u8]) -> Result<()> {
    let want = sums
        .get(name)
        .with_context(|| format!("the release's SHA256SUMS doesn't list {name}"))?;
    let got = merlin_plugins::package::sha256_hex(bytes);
    if &got != want {
        bail!("{name} doesn't match the release's SHA256SUMS: expected {want}, got {got}");
    }
    Ok(())
}

/// This merlin's version, as a tag: `v0.1.0`.
pub(crate) fn current() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// A tag's numbers, `v0.10.2` as (0, 10, 2); a pre-release counts as its
/// numbers.
fn numbers(tag: &str) -> Option<(u64, u64, u64)> {
    let core = tag.trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|n| n.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

/// Whether `tag` is newer than `than`.
pub(crate) fn newer(tag: &str, than: &str) -> bool {
    matches!((numbers(tag), numbers(than)), (Some(a), Some(b)) if a > b)
}

/// This platform's archive in a release, as `ci.yml` names them.
pub(crate) fn archive_name() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("merlin-linux-x86_64.tar.gz")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("merlin-linux-aarch64.tar.gz")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("merlin-macos-aarch64.tar.gz")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("merlin-macos-x86_64.tar.gz")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some("merlin-windows-x86_64.zip")
    } else {
        None
    }
}

/// A published release, as GitHub describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Release {
    pub tag: String,
    pub notes: String,
    pub page: String,
    /// Each file's name and where to download it.
    pub files: BTreeMap<String, String>,
}

/// Reads GitHub's answer for a release.
pub(crate) fn read_release(json: &[u8]) -> Result<Release> {
    let v: serde_json::Value = serde_json::from_slice(json).context("the release can't be read")?;
    let text = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let tag = text("tag_name");
    if tag.is_empty() {
        bail!("the release has no tag");
    }
    let files = v
        .get("assets")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some((
                a.get("name")?.as_str()?.to_string(),
                a.get("browser_download_url")?.as_str()?.to_string(),
            ))
        })
        .collect();
    Ok(Release {
        tag,
        notes: text("body").replace("\r\n", "\n"),
        page: text("html_url"),
        files,
    })
}

/// Why merlin, at `exe`, can't replace itself there: said as "in /usr/bin"
/// or "here". None when it can.
pub(crate) fn cant_replace(exe: &Path) -> Option<String> {
    let dir = exe.parent()?;
    let place = format!("in {}", dir.display());
    if std::env::var_os("MERLIN_NO_SELF_UPDATE").is_some() {
        return Some("here".into());
    }
    if archive_name().is_none() {
        return Some("on this system".into());
    }
    // A package manager's copy is the package manager's to update.
    let managed = [
        "/usr/", "/opt/", "/nix/", "/snap/", "/app/", "/bin/", "/sbin/",
    ];
    let shown = dir.to_string_lossy().replace('\\', "/");
    if managed.iter().any(|m| shown.starts_with(m))
        || shown.contains("/Cellar/")
        || shown.contains("/homebrew/")
    {
        return Some(place);
    }
    let probe = dir.join(format!("{STAGING}-probe"));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            None
        }
        Err(_) => Some(place),
    }
}

/// What's left beside `exe` by an update: removed at the next start.
pub(crate) fn tidy(exe: &Path) {
    if let Some(dir) = exe.parent() {
        let _ = std::fs::remove_dir_all(dir.join(STAGING));
        let _ = std::fs::remove_file(dir.join(OLD));
        let _ = std::fs::remove_file(dir.join(format!("{OLD}.exe")));
        let _ = std::fs::remove_dir_all(dir.join(format!("{OLD}-plugins")));
    }
}

/// How an update under way is going, shared with the thread doing it.
#[derive(Default)]
pub(crate) struct Progress {
    step: AtomicU8,
    done: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
}

impl Progress {
    fn step(&self) -> UpdateStep {
        match self.step.load(Ordering::Relaxed) {
            0 => UpdateStep::Download,
            1 => UpdateStep::Check,
            _ => UpdateStep::Place,
        }
    }

    fn set_step(&self, step: UpdateStep) {
        self.step.store(step as u8, Ordering::Relaxed);
    }

    fn cancelled(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            bail!(Cancelled);
        }
        Ok(())
    }
}

/// An update stopped by Cancel, said by nothing.
#[derive(Debug)]
struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

/// What the app keeps of updates.
#[derive(Default)]
pub(crate) struct Updater {
    /// When to check next; none in scripts or with checks off.
    pub due: Option<Instant>,
    checking: bool,
    /// The newest release, when it's newer than this merlin.
    found: Option<Release>,
    /// An update under way, and when to read its progress next.
    working: Option<Arc<Progress>>,
    pub poll: Option<Instant>,
    /// Where merlin's executable is, read before it's replaced: what
    /// starts after quitting, when that's asked for.
    exe: Option<PathBuf>,
    pub restart: bool,
}

impl Updater {
    pub(crate) fn new(gui: bool) -> Self {
        Updater {
            due: gui.then(|| Instant::now() + FIRST_CHECK),
            exe: std::env::current_exe()
                .ok()
                .and_then(|e| e.canonicalize().ok()),
            ..Self::default()
        }
    }

    /// The new merlin, after quitting, when Restart asked for it: with the
    /// same arguments, in the same folder.
    pub(crate) fn start_again(&self, root: &Path) {
        if !self.restart {
            return;
        }
        let Some(exe) = &self.exe else { return };
        let _ = std::process::Command::new(exe)
            .args(std::env::args_os().skip(1))
            .current_dir(root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// Fetches `url` into memory, at most `limit` bytes, counting into
/// `progress` as it goes and stopping at Cancel.
fn download(url: &str, limit: u64, progress: &Progress) -> Result<Vec<u8>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", concat!("merlin/", env!("CARGO_PKG_VERSION")))
        .call()
        .with_context(|| format!("fetching {url}"))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    if total > limit {
        bail!("{url} is larger than {limit} bytes");
    }
    progress.total.store(total, Ordering::Relaxed);
    let mut reader = response.body_mut().with_config().limit(limit + 1).reader();
    let mut out = Vec::with_capacity(total as usize);
    let mut buf = vec![0; 64 * 1024];
    loop {
        progress.cancelled()?;
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("reading {url}"))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() as u64 > limit {
            bail!("{url} is larger than {limit} bytes");
        }
        progress.done.store(out.len() as u64, Ordering::Relaxed);
    }
    Ok(out)
}

/// Downloads `release`'s archive for this platform, checks it against the
/// release's signed list, and puts it in place of `exe`, with its plugins
/// in `bundle`'s place.
/// Fetches a URL, at most so many bytes, counting into the progress.
type Get<'a> = &'a dyn Fn(&str, u64, &Progress) -> Result<Vec<u8>>;

/// The network's [`Get`]: the archive counted as it comes, the rest whole.
fn get(url: &str, limit: u64, progress: &Progress) -> Result<Vec<u8>> {
    if limit == MAX_ARCHIVE {
        download(url, limit, progress)
    } else {
        crate::plugin_cli::fetch(url, limit)
    }
}

fn install(
    release: &Release,
    exe: &Path,
    bundle: &Path,
    keys: &[&str],
    get: Get,
    progress: &Progress,
) -> Result<()> {
    let name = archive_name().context("there's no release for this system")?;
    let url = |file: &str| {
        release
            .files
            .get(file)
            .cloned()
            .with_context(|| format!("{} has no {file}", release.tag))
    };
    let (archive_url, sums_url, sig_url) =
        (url(name)?, url("SHA256SUMS")?, url("SHA256SUMS.minisig")?);
    progress.set_step(UpdateStep::Download);
    let archive = get(&archive_url, MAX_ARCHIVE, progress)?;
    let sums = get(&sums_url, MAX_SMALL, progress)?;
    let sig = get(&sig_url, MAX_SMALL, progress)?;
    progress.cancelled()?;

    progress.set_step(UpdateStep::Check);
    check_sums(keys, &sums, &sig, &release.tag)
        .and_then(|list| check_archive(&list, name, &archive))
        .map_err(|e| anyhow::anyhow!(Mismatch(format!("{e:#}"))))?;
    let dir = exe.parent().context("merlin's folder can't be found")?;
    let staging = dir.join(STAGING);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    unpack(name, &archive, &staging)?;
    let binary = if cfg!(windows) {
        "merlin.exe"
    } else {
        "merlin"
    };
    let new_exe = staging.join(binary);
    if !new_exe.is_file() {
        bail!("{name} has no {binary}");
    }
    progress.cancelled()?;

    // From here it goes through: no Cancel.
    progress.set_step(UpdateStep::Place);
    place(exe, &new_exe, &staging.join("plugins"), bundle)?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// A download that doesn't match the signed list.
#[derive(Debug)]
struct Mismatch(String);

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unpacks a release archive into `into`.
fn unpack(name: &str, archive: &[u8], into: &Path) -> Result<()> {
    if name.ends_with(".tar.gz") {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        for entry in tar.entries().context("the archive can't be read")? {
            let mut entry = entry.context("the archive can't be read")?;
            // Nothing outside the folder: unpack_in refuses such paths.
            if !entry.unpack_in(into).context("unpacking the archive")? {
                bail!("the archive has a file outside its folder");
            }
        }
        Ok(())
    } else {
        // A zip, on Windows: its tar (bsdtar) reads zips.
        let file = into.join("archive.zip");
        std::fs::write(&file, archive)?;
        let ok = std::process::Command::new("tar")
            .arg("-xf")
            .arg(&file)
            .arg("-C")
            .arg(into)
            .status()
            .context("running tar")?
            .success();
        let _ = std::fs::remove_file(&file);
        if !ok {
            bail!("the archive can't be unpacked");
        }
        // 7-Zip may have kept the folder the files were packed from.
        let nested = into.join("staging");
        if nested.is_dir() {
            for entry in std::fs::read_dir(&nested)? {
                let entry = entry?;
                std::fs::rename(entry.path(), into.join(entry.file_name()))?;
            }
        }
        Ok(())
    }
}

/// Puts `new_exe` where `exe` is, and `plugins` where `bundle` is.
fn place(exe: &Path, new_exe: &Path, plugins: &Path, bundle: &Path) -> Result<()> {
    let dir = exe.parent().context("merlin's folder can't be found")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new_exe, std::fs::Permissions::from_mode(0o755))?;
        // The running program keeps its file; the name gets the new one.
        std::fs::rename(new_exe, exe).context("replacing merlin")?;
    }
    #[cfg(windows)]
    {
        // A running .exe can't be overwritten, but it can be renamed.
        let old = dir.join(format!("{OLD}.exe"));
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old).context("moving the running merlin aside")?;
        if let Err(err) = std::fs::rename(new_exe, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(err).context("replacing merlin");
        }
    }
    if plugins.join(merlin_plugins::bundled::LIST).is_file() {
        let old = dir.join(format!("{OLD}-plugins"));
        let _ = std::fs::remove_dir_all(&old);
        if let Some(parent) = bundle.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if bundle.exists() {
            std::fs::rename(bundle, &old).context("moving the old plugins aside")?;
        }
        if std::fs::rename(plugins, bundle).is_err() {
            // Another disk: copy instead.
            copy_dir(plugins, bundle).context("putting the new plugins in place")?;
        }
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

impl App {
    /// Whether the user's settings let merlin look for updates.
    /// Installed registry plugins with no version for the plugin API a
    /// release's notes name ("Plugin API 0.16"), by title. None when the
    /// notes don't say, or it's this merlin's.
    fn plugins_off_after(&self, notes: &str) -> Vec<String> {
        let api = notes.lines().find_map(|l| {
            let at = l.find("Plugin API ")?;
            let rest = &l[at + "Plugin API ".len()..];
            let v: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            Some(v.trim_end_matches('.').to_string())
        });
        let Some(api) = api.filter(|a| !a.is_empty() && a != merlin_plugins::API_VERSION) else {
            return Vec::new();
        };
        self.browse
            .installed
            .iter()
            .filter_map(|(name, mine)| {
                let l = self
                    .browse
                    .sources
                    .iter()
                    .find(|l| l.source.name == mine.source)?;
                let entry = l.index.plugins.get(name)?;
                let ready = entry
                    .versions
                    .iter()
                    .any(|v| v.api == api && v.yanked.is_none() && v.blocked.is_none());
                (!ready).then(|| entry.title.clone())
            })
            .collect()
    }

    fn checks_for_updates(&self) -> bool {
        self.loaded
            .user
            .get("editor")
            .and_then(|e| e.get("check_for_updates"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    /// Asks GitHub for the newest release, off the window's thread.
    pub(crate) fn check_for_update_later(&mut self) {
        self.updater.due = Some(Instant::now() + CHECK_EVERY);
        if self.scripted || self.updater.checking || !self.checks_for_updates() {
            return;
        }
        self.updater.checking = true;
        // MERLIN_UPDATE_URL asks elsewhere, to try a release before it's out.
        let url = std::env::var("MERLIN_UPDATE_URL").unwrap_or_else(|_| LATEST.into());
        self.jobs.run(Lane::Net, move || {
            let got = crate::plugin_cli::fetch(&url, MAX_SMALL)
                .and_then(|json| read_release(&json))
                .map_err(|e| format!("{e:#}"));
            Box::new(move |app: &mut App| app.update_checked(got)) as Finish
        });
    }

    fn update_checked(&mut self, got: Result<Release, String>) -> Vec<Event> {
        self.updater.checking = false;
        let release = match got {
            Ok(release) => release,
            Err(why) => {
                eprintln!("merlin: can't check for updates: {why}");
                return Vec::new();
            }
        };
        let current = current();
        if !newer(&release.tag, &current) {
            self.updater.found = None;
            return vec![Event::UpdateOffer(None)];
        }
        let exe = self.updater.exe.clone().unwrap_or_default();
        let offer = UpdateOffer {
            version: release.tag.clone(),
            current,
            notes: release.notes.clone(),
            page: release.page.clone(),
            manual: cant_replace(&exe),
            command: INSTALL.into(),
            plugins_off: self.plugins_off_after(&release.notes),
        };
        self.updater.found = Some(release);
        vec![Event::UpdateOffer(Some(offer))]
    }

    /// What the update card asked for.
    pub(crate) fn app_update(&mut self, action: UpdateAction) -> Vec<Event> {
        match action {
            UpdateAction::Install(tag) => self.install_update(tag),
            UpdateAction::Cancel => {
                if let Some(p) = &self.updater.working {
                    p.cancel.store(true, Ordering::Relaxed);
                }
                Vec::new()
            }
            UpdateAction::Restart => {
                self.updater.restart = true;
                Vec::new()
            }
        }
    }

    fn install_update(&mut self, tag: String) -> Vec<Event> {
        let release = match &self.updater.found {
            Some(r) if r.tag == tag => r.clone(),
            _ => {
                return vec![Event::UpdateFailed(format!(
                    "{tag} isn't the newest release any more."
                ))];
            }
        };
        let Some(exe) = self.updater.exe.clone() else {
            return vec![Event::UpdateFailed(
                "merlin's own file can't be found.".into(),
            )];
        };
        if self.updater.working.is_some() {
            return Vec::new();
        }
        let bundle = crate::browse::bundle_dir(&self.browse.data_dir)
            .unwrap_or_else(|| self.browse.data_dir.join("bundled"));
        let progress = Arc::new(Progress::default());
        self.updater.working = Some(progress.clone());
        self.updater.poll = Some(Instant::now() + POLL);
        self.jobs.run(Lane::Net, move || {
            let result = install(&release, &exe, &bundle, &release_keys(), &get, &progress);
            Box::new(move |app: &mut App| app.update_done(&release.tag, result)) as Finish
        });
        Vec::new()
    }

    /// Reads an update's progress for the card, while one is under way.
    pub(crate) fn poll_update(&mut self) {
        let Some(p) = &self.updater.working else {
            self.updater.poll = None;
            return;
        };
        self.updater.poll = Some(Instant::now() + POLL);
        let (step, done, total) = (
            p.step(),
            p.done.load(Ordering::Relaxed),
            p.total.load(Ordering::Relaxed),
        );
        if p.cancel.load(Ordering::Relaxed) {
            return;
        }
        self.dispatch(Event::UpdateProgress { step, done, total });
    }

    fn update_done(&mut self, tag: &str, result: Result<()>) -> Vec<Event> {
        let progress = self.updater.working.take();
        self.updater.poll = None;
        match result {
            Ok(()) => {
                if let Some(p) = progress {
                    let (done, total) = (
                        p.done.load(Ordering::Relaxed),
                        p.total.load(Ordering::Relaxed),
                    );
                    self.dispatch(Event::UpdateProgress {
                        step: UpdateStep::Place,
                        done,
                        total,
                    });
                }
                vec![Event::UpdateReady]
            }
            Err(err) if err.downcast_ref::<Cancelled>().is_some() => Vec::new(),
            Err(err) if err.downcast_ref::<Mismatch>().is_some() => {
                eprintln!("merlin: {tag}: {err:#}");
                vec![Event::UpdateFailed(
                    "The download failed its signature check. Nothing was changed.".into(),
                )]
            }
            Err(err) => vec![Event::UpdateFailed(format!(
                "Can't update to {tag}: {err:#}"
            ))],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Signer {
        keys: minisign::KeyPair,
    }

    impl Signer {
        fn new() -> Self {
            Signer {
                keys: minisign::KeyPair::generate_unencrypted_keypair().unwrap(),
            }
        }
        fn key(&self) -> String {
            self.keys.pk.to_base64()
        }
        fn sign(&self, bytes: &[u8], comment: &str) -> Vec<u8> {
            let sig = minisign::sign(
                Some(&self.keys.pk),
                &self.keys.sk,
                std::io::Cursor::new(bytes),
                Some(comment),
                None,
            )
            .unwrap();
            sig.to_string().into_bytes()
        }
    }

    fn sums_for(archive: &[u8]) -> Vec<u8> {
        let hex = merlin_plugins::package::sha256_hex(archive);
        format!(
            "{hex}  merlin-linux-x86_64.tar.gz\n{} *merlin-windows-x86_64.zip\n",
            "a".repeat(64)
        )
        .into_bytes()
    }

    #[test]
    fn the_embedded_keys_are_the_repositorys() {
        let keys = release_keys();
        assert_ne!(keys[0], keys[1]);
        for key in keys {
            assert!(key.starts_with("RW"), "{key}");
            minisign_verify::PublicKey::from_base64(key).unwrap();
        }
    }

    #[test]
    fn a_list_signed_by_the_rotation_key_is_accepted_too() {
        let (now, next) = (Signer::new(), Signer::new());
        let sums = sums_for(b"x");
        let sig = next.sign(&sums, "merlin v0.2.0");
        check_sums(&[&now.key(), &next.key()], &sums, &sig, "v0.2.0").unwrap();
    }

    #[test]
    fn a_signed_list_for_the_tag_reads_and_vouches_for_its_archives() {
        let s = Signer::new();
        let archive = b"an archive".as_slice();
        let sums = sums_for(archive);
        let sig = s.sign(&sums, "merlin v0.2.0");
        let read = check_sums(&[&s.key()], &sums, &sig, "v0.2.0").unwrap();
        assert_eq!(read.len(), 2);
        assert!(read.contains_key("merlin-windows-x86_64.zip"));
        check_archive(&read, "merlin-linux-x86_64.tar.gz", archive).unwrap();
        let err = check_archive(&read, "merlin-linux-x86_64.tar.gz", b"changed").unwrap_err();
        assert!(err.to_string().contains("doesn't match"), "{err}");
        let err = check_archive(&read, "merlin-macos-aarch64.tar.gz", archive).unwrap_err();
        assert!(err.to_string().contains("doesn't list"), "{err}");
    }

    #[test]
    fn another_key_a_changed_list_or_another_tag_is_refused() {
        let s = Signer::new();
        let sums = sums_for(b"x");
        let sig = s.sign(&sums, "merlin v0.2.0");

        let other = Signer::new();
        let err = check_sums(&[&other.key()], &sums, &sig, "v0.2.0").unwrap_err();
        assert!(err.to_string().contains("isn't signed"), "{err}");

        let mut changed = sums.clone();
        changed[0] = if changed[0] == b'0' { b'1' } else { b'0' };
        let err = check_sums(&[&s.key()], &changed, &sig, "v0.2.0").unwrap_err();
        assert!(err.to_string().contains("isn't signed"), "{err}");

        // An older release's signed list served as the newest one.
        let err = check_sums(&[&s.key()], &sums, &sig, "v0.3.0").unwrap_err();
        assert!(err.to_string().contains("signed for"), "{err}");
    }

    #[test]
    fn a_list_it_cant_read_is_refused() {
        let s = Signer::new();
        let sums = b"not a checksum line\n".to_vec();
        let sig = s.sign(&sums, "merlin v0.2.0");
        let err = check_sums(&[&s.key()], &sums, &sig, "v0.2.0").unwrap_err();
        assert!(err.to_string().contains("can't read"), "{err}");
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(newer("v0.2.0", "v0.1.0"));
        assert!(newer("v0.10.0", "v0.9.3"), "not as text");
        assert!(newer("v1.0", "v0.99.99"));
        assert!(!newer("v0.1.0", "v0.1.0"));
        assert!(!newer("v0.1.0", "v0.2.0"));
        assert!(!newer("nightly", "v0.1.0"), "a tag that isn't a version");
        assert_eq!(current(), format!("v{}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn githubs_answer_reads_as_a_release() {
        let json = br###"{"tag_name":"v0.2.0","html_url":"https://github.com/x/releases/tag/v0.2.0",
            "body":"## New\r\n\r\n- this\r\n",
            "assets":[{"name":"SHA256SUMS","browser_download_url":"https://dl/SHA256SUMS"},
                      {"name":"merlin-linux-x86_64.tar.gz","browser_download_url":"https://dl/a.tar.gz"}]}"###;
        let r = read_release(json).unwrap();
        assert_eq!(r.tag, "v0.2.0");
        assert_eq!(r.notes, "## New\n\n- this\n");
        assert_eq!(r.files["SHA256SUMS"], "https://dl/SHA256SUMS");
        assert_eq!(r.files.len(), 2);
        assert!(read_release(b"{}").is_err(), "no tag");
    }

    #[test]
    fn a_package_managers_copy_isnt_replaced() {
        if archive_name().is_none() {
            return;
        }
        assert_eq!(
            cant_replace(Path::new("/usr/bin/merlin")).as_deref(),
            Some("in /usr/bin")
        );
        let scratch = scratch("writable");
        let dir = scratch.path();
        assert_eq!(cant_replace(&dir.join("merlin")), None);
        assert!(!dir.join(format!("{STAGING}-probe")).exists());
    }

    /// A folder of the test's own, removed when it drops, even if the
    /// test fails.
    fn scratch(name: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("merlin-update-{name}-"))
            .tempdir()
            .unwrap()
    }

    /// A release archive as `ci.yml` packs one, holding `binary` and a
    /// plugins folder.
    fn archive(binary: &[u8]) -> Vec<u8> {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut add = |path: &str, bytes: &[u8], mode: u32| {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(mode);
            header.set_cksum();
            tar.append_data(&mut header, path, bytes).unwrap();
        };
        add("merlin", binary, 0o755);
        add("README.md", b"readme", 0o644);
        add(
            &format!("plugins/{}", merlin_plugins::bundled::LIST),
            b"# new\n",
            0o644,
        );
        add("plugins/rust-0.2.0.merlin", b"package", 0o644);
        tar.into_inner().unwrap().finish().unwrap()
    }

    /// A release whose files are served from memory by the [`Get`] it
    /// comes with, signed by `signer`.
    fn release(
        signer: &Signer,
        archive: Vec<u8>,
        sign_for: &str,
    ) -> (Release, BTreeMap<String, Vec<u8>>) {
        let name = archive_name().unwrap();
        let sums = format!(
            "{}  {name}\n",
            merlin_plugins::package::sha256_hex(&archive)
        )
        .into_bytes();
        let sig = signer.sign(&sums, &format!("merlin {sign_for}"));
        let mut served = BTreeMap::new();
        served.insert(format!("mem://{name}"), archive);
        served.insert("mem://SHA256SUMS".to_string(), sums);
        served.insert("mem://SHA256SUMS.minisig".to_string(), sig);
        let files = served
            .keys()
            .map(|url| (url.trim_start_matches("mem://").to_string(), url.clone()))
            .collect();
        (
            Release {
                tag: "v0.2.0".into(),
                notes: String::new(),
                page: String::new(),
                files,
            },
            served,
        )
    }

    #[test]
    fn an_update_replaces_merlin_and_its_plugins_once_checked() {
        if archive_name().is_none_or(|n| !n.ends_with(".tar.gz")) {
            return;
        }
        let scratch = scratch("install");
        let dir = scratch.path();
        let exe = dir.join("merlin");
        std::fs::write(&exe, b"old merlin").unwrap();
        let bundle = dir.join("plugins");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("rust-0.1.0.merlin"), b"old").unwrap();

        let signer = Signer::new();
        let (rel, served) = release(&signer, archive(b"new merlin"), "v0.2.0");
        let get = |url: &str, _: u64, p: &Progress| -> Result<Vec<u8>> {
            p.done.store(1, Ordering::Relaxed);
            served.get(url).cloned().context("not served")
        };
        let progress = Progress::default();
        install(&rel, &exe, &bundle, &[&signer.key()], &get, &progress).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new merlin");
        assert!(bundle.join("rust-0.2.0.merlin").is_file());
        assert!(
            !bundle.join("rust-0.1.0.merlin").exists(),
            "the old plugins went"
        );
        assert_eq!(progress.step(), UpdateStep::Place);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "it runs");
        }
        tidy(&exe);
        assert!(!dir.join(STAGING).exists());
    }

    #[test]
    fn a_tampered_or_mislabelled_release_changes_nothing() {
        if archive_name().is_none_or(|n| !n.ends_with(".tar.gz")) {
            return;
        }
        let scratch = scratch("refused");
        let dir = scratch.path();
        let exe = dir.join("merlin");
        std::fs::write(&exe, b"old merlin").unwrap();
        let signer = Signer::new();

        // The archive changed after signing.
        let (rel, mut served) = release(&signer, archive(b"new merlin"), "v0.2.0");
        let name = format!("mem://{}", archive_name().unwrap());
        served.insert(name.clone(), archive(b"evil merlin"));
        let get = |url: &str, _: u64, _: &Progress| -> Result<Vec<u8>> {
            served.get(url).cloned().context("not served")
        };
        let err = install(
            &rel,
            &exe,
            &dir.join("plugins"),
            &[&signer.key()],
            &get,
            &Progress::default(),
        )
        .unwrap_err();
        assert!(err.downcast_ref::<Mismatch>().is_some(), "{err:#}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old merlin");

        // An older release's signed list, passed off as this one.
        let (rel, served) = release(&signer, archive(b"new merlin"), "v0.1.0");
        let get = |url: &str, _: u64, _: &Progress| -> Result<Vec<u8>> {
            served.get(url).cloned().context("not served")
        };
        let err = install(
            &rel,
            &exe,
            &dir.join("plugins"),
            &[&signer.key()],
            &get,
            &Progress::default(),
        )
        .unwrap_err();
        assert!(err.downcast_ref::<Mismatch>().is_some(), "{err:#}");

        // Another key's signature.
        let (rel, served) = release(&Signer::new(), archive(b"new merlin"), "v0.2.0");
        let get = |url: &str, _: u64, _: &Progress| -> Result<Vec<u8>> {
            served.get(url).cloned().context("not served")
        };
        let err = install(
            &rel,
            &exe,
            &dir.join("plugins"),
            &[&signer.key()],
            &get,
            &Progress::default(),
        )
        .unwrap_err();
        assert!(err.downcast_ref::<Mismatch>().is_some(), "{err:#}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old merlin");
    }

    #[test]
    fn cancel_stops_before_anything_changes() {
        if archive_name().is_none() {
            return;
        }
        let scratch = scratch("cancel");
        let dir = scratch.path();
        let exe = dir.join("merlin");
        std::fs::write(&exe, b"old merlin").unwrap();
        let signer = Signer::new();
        let (rel, served) = release(&signer, archive(b"new merlin"), "v0.2.0");
        let get = |url: &str, _: u64, p: &Progress| -> Result<Vec<u8>> {
            p.cancel.store(true, Ordering::Relaxed);
            served.get(url).cloned().context("not served")
        };
        let err = install(
            &rel,
            &exe,
            &dir.join("plugins"),
            &[&signer.key()],
            &get,
            &Progress::default(),
        )
        .unwrap_err();
        assert!(err.downcast_ref::<Cancelled>().is_some(), "{err:#}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old merlin");
    }
}
