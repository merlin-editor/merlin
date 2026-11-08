//! `.merlin` packages: a plugin packed into one file, the same bytes every
//! time from the same source, and installed from one into the plugins
//! folder by unpacking it to the side and swapping it in.
//!
//! See `docs/plugin-registry.md`, section 4 for the format and 7.2 for
//! installing.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::read::GzDecoder;
use globset::{GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{API_VERSION, Manifest, manifest};

/// A package's file extension.
pub const EXTENSION: &str = "merlin";
/// The component's name inside a package.
pub const COMPONENT: &str = "plugin.wasm";
/// The largest package `pack` makes.
pub const MAX_PACKED: u64 = 25 * 1024 * 1024;
/// The most a package may unpack to, so a small file can't fill the disk.
pub const MAX_UNPACKED: u64 = 200 * 1024 * 1024;

/// Left out of every package: what builds the component, not what runs.
/// Top-level names only.
const BUILD_FILES: &[&str] = &[
    "target",
    "src",
    "tests",
    ".cargo",
    ".git",
    "Cargo.toml",
    "Cargo.lock",
];

/// A package `pack` made.
#[derive(Debug)]
pub struct Packed {
    pub path: PathBuf,
    pub name: String,
    pub version: String,
    /// Lowercase hex.
    pub sha256: String,
    pub size: u64,
    pub optimised: Optimised,
}

/// What became of `wasm-opt -Oz` on the component.
#[derive(Debug, PartialEq, Eq)]
pub enum Optimised {
    /// The plugin has no component: it's data.
    NoComponent,
    /// `wasm-opt` isn't on `PATH`; the component went in as built.
    Missing,
    /// It ran but failed (it reads core modules, and may not read a
    /// component); the component went in as built.
    Failed(String),
    Done {
        before: u64,
        after: u64,
    },
}

/// Packs the plugin in `dir` into `<name>-<version>.merlin` in `out_dir`.
///
/// `epoch` is every file's modification time, in seconds: CI gives the
/// pinned commit's time through `SOURCE_DATE_EPOCH` (see [`source_epoch`]),
/// so the same commit always packs to the same bytes.
pub fn pack(dir: &Path, out_dir: &Path, epoch: u64) -> Result<Packed> {
    let (bytes, manifest, optimised) = pack_bytes(dir, epoch)?;
    let size = bytes.len() as u64;
    if size > MAX_PACKED {
        bail!(
            "{} packs to {} MB; a package can be at most {} MB",
            manifest.name,
            size / (1024 * 1024),
            MAX_PACKED / (1024 * 1024)
        );
    }
    fs::create_dir_all(out_dir)?;
    let path = out_dir.join(format!(
        "{}-{}.{EXTENSION}",
        manifest.name, manifest.version
    ));
    let part = path.with_extension(format!("{EXTENSION}.part"));
    fs::write(&part, &bytes).with_context(|| format!("writing {}", part.display()))?;
    fs::rename(&part, &path)?;
    Ok(Packed {
        path,
        name: manifest.name,
        version: manifest.version,
        sha256: sha256_hex(&bytes),
        size,
        optimised,
    })
}

/// `SOURCE_DATE_EPOCH` if it's set, else 0.
pub fn source_epoch() -> u64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The package's bytes, the manifest, and how the component was optimised.
fn pack_bytes(dir: &Path, epoch: u64) -> Result<(Vec<u8>, Manifest, Optimised)> {
    let manifest = manifest(dir)?;
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    // The manifest, pointing at the component by its packed name.
    let text = fs::read_to_string(dir.join("plugin.toml"))?;
    let optimised = match &manifest.component {
        None => {
            files.insert("plugin.toml".into(), text.into_bytes());
            Optimised::NoComponent
        }
        Some(component) => {
            let path = dir.join(component);
            let built = fs::read(&path).with_context(|| {
                format!(
                    "{} isn't built: run `cargo build --release` in {}",
                    manifest.name,
                    dir.display()
                )
            })?;
            let (wasm, optimised) = optimise(built);
            files.insert(COMPONENT.into(), wasm);
            let mut doc: toml_edit::DocumentMut = text.parse().context("reading plugin.toml")?;
            doc["component"] = toml_edit::value(COMPONENT);
            files.insert("plugin.toml".into(), doc.to_string().into_bytes());
            optimised
        }
    };

    // Everything else in the folder that isn't the build's.
    let exclude = excludes(&manifest.package.exclude)?;
    let skip = manifest.component.as_ref().map(|c| slash_path(c));
    for rel in walk(dir)? {
        let top = rel.split('/').next().unwrap_or("");
        if BUILD_FILES.contains(&top)
            || rel == "plugin.toml"
            || rel == COMPONENT
            || Some(&rel) == skip.as_ref()
            || rel.ends_with(&format!(".{EXTENSION}"))
            || excluded(&exclude, &rel)
        {
            continue;
        }
        files.insert(rel.clone(), fs::read(dir.join(&rel))?);
    }

    // Every file the manifest names has to be there.
    for named in named_files(&manifest) {
        if !files.contains_key(&named) {
            bail!(
                "plugin.toml names {named}, but it isn't in the package: it's missing, or [package] exclude leaves it out"
            );
        }
    }

    Ok((archive(&files, epoch)?, manifest, optimised))
}

/// Files the manifest names: themes' and packs' files, and SVG icons.
fn named_files(manifest: &Manifest) -> Vec<String> {
    let mut named: Vec<String> = manifest
        .themes
        .iter()
        .chain(&manifest.packs)
        .map(|t| slash_path(&t.file))
        .collect();
    let icons = manifest.views.iter().flat_map(|v| {
        v.icon
            .iter()
            .chain(v.toolbar.iter().filter_map(|t| t.icon.as_ref()))
    });
    named.extend(icons.filter(|i| i.ends_with(".svg")).cloned());
    named
}

/// `exclude` patterns as globs. A pattern names a file or a folder, from
/// the plugin's folder; `*` stays within a folder and `**` crosses them.
fn excludes(patterns: &[String]) -> Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        let trimmed = pattern.trim_start_matches("./").trim_end_matches('/');
        let glob = globset::GlobBuilder::new(trimmed)
            .literal_separator(true)
            .build()
            .with_context(|| format!("[package] exclude: bad pattern {pattern:?}"))?;
        set.add(glob);
    }
    Ok(set.build()?)
}

/// Whether `rel`, or a folder it's in, matches an exclude.
fn excluded(set: &GlobSet, rel: &str) -> bool {
    let mut prefix = String::new();
    rel.split('/').any(|part| {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        set.is_match(&prefix)
    })
}

/// Every file under `dir`, as `/`-separated paths relative to it. A
/// symlink is an error: a package holds files only.
fn walk(dir: &Path) -> Result<Vec<String>> {
    fn go(root: &Path, at: &Path, out: &mut Vec<String>) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(at)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let rel = path.strip_prefix(root)?;
            let rel_s = slash_path(rel);
            let kind = entry.file_type()?;
            let top = rel_s.split('/').next().unwrap_or("");
            if BUILD_FILES.contains(&top) {
                continue;
            }
            if kind.is_symlink() {
                bail!("{rel_s} is a symlink; a package can hold only files");
            } else if kind.is_dir() {
                go(root, &path, out)?;
            } else if kind.is_file() {
                out.push(rel_s);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    go(dir, dir, &mut out)?;
    Ok(out)
}

fn slash_path(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Runs `wasm-opt -Oz` on the component if it can.
fn optimise(wasm: Vec<u8>) -> (Vec<u8>, Optimised) {
    let before = wasm.len() as u64;
    let tmp = std::env::temp_dir().join(format!("merlin-pack-{}-{}", std::process::id(), nanos()));
    if fs::create_dir_all(&tmp).is_err() {
        return (
            wasm,
            Optimised::Failed("can't make a temporary folder".into()),
        );
    }
    let (input, output) = (tmp.join("in.wasm"), tmp.join("out.wasm"));
    let result = fs::write(&input, &wasm)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            Command::new("wasm-opt")
                .args(["-Oz", "--all-features"])
                .arg(&input)
                .arg("-o")
                .arg(&output)
                .output()
                .map_err(|e| e.to_string())
        });
    let outcome = match result {
        Err(_) if which_missing() => (wasm, Optimised::Missing),
        Err(err) => (wasm, Optimised::Failed(err)),
        Ok(out) if !out.status.success() => {
            let err = String::from_utf8_lossy(&out.stderr);
            let first = err
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("it failed");
            (wasm, Optimised::Failed(first.trim().to_string()))
        }
        Ok(_) => match fs::read(&output) {
            Ok(smaller) => {
                let after = smaller.len() as u64;
                (smaller, Optimised::Done { before, after })
            }
            Err(err) => (wasm, Optimised::Failed(err.to_string())),
        },
    };
    let _ = fs::remove_dir_all(&tmp);
    outcome
}

/// Whether `wasm-opt` isn't there at all, as opposed to failing.
fn which_missing() -> bool {
    Command::new("wasm-opt")
        .arg("--version")
        .output()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

/// A gzipped ustar archive of `files`, the same bytes for the same input:
/// sorted, one modification time, no owners, no folders of their own.
fn archive(files: &BTreeMap<String, Vec<u8>>, epoch: u64) -> Result<Vec<u8>> {
    let gz = flate2::GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(Vec::new(), Compression::best());
    let mut tar = tar::Builder::new(gz);
    tar.mode(tar::HeaderMode::Deterministic);
    for (path, bytes) in files {
        let mut header = tar::Header::new_ustar();
        header
            .set_path(path)
            .with_context(|| format!("{path}: the path is too long for a package"))?;
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(epoch);
        header.set_uid(0);
        header.set_gid(0);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        tar.append(&header, bytes.as_slice())?;
    }
    let gz = tar.into_inner()?;
    Ok(gz.finish()?)
}

/// Where an installed plugin came from, as `installed.toml` in the data
/// folder records it, by plugin name.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Installed {
    /// `file` for a package installed from disk; later, a source's name.
    pub source: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Default, Deserialize, Serialize)]
struct InstalledFile {
    #[serde(default)]
    plugins: BTreeMap<String, Installed>,
}

/// What `installed.toml` in `data_dir` records.
pub fn installed(data_dir: &Path) -> BTreeMap<String, Installed> {
    fs::read_to_string(data_dir.join("installed.toml"))
        .ok()
        .and_then(|s| toml::from_str::<InstalledFile>(&s).ok())
        .unwrap_or_default()
        .plugins
}

/// Records `name` in `installed.toml`, or forgets it with `None`.
pub fn record(data_dir: &Path, name: &str, entry: Option<Installed>) -> Result<()> {
    let mut file = InstalledFile {
        plugins: installed(data_dir),
    };
    match entry {
        Some(entry) => file.plugins.insert(name.to_string(), entry),
        None => file.plugins.remove(name),
    };
    fs::create_dir_all(data_dir)?;
    let path = data_dir.join("installed.toml");
    let part = data_dir.join("installed.toml.part");
    fs::write(&part, toml::to_string(&file)?)?;
    fs::rename(part, path)?;
    Ok(())
}

/// Installs the package at `archive` into `plugins_dir`, replacing the
/// version there if any, and records it in `data_dir`. It doesn't grant
/// anything: the caller does, once the user has seen what it asks for.
///
/// It unpacks into `.staging/`, checks what it got, then moves the old
/// folder to `.trash/` and the new one into place. [`recover`] tidies up
/// after one that stopped halfway.
pub fn install(archive: &Path, plugins_dir: &Path, data_dir: &Path) -> Result<Manifest> {
    let bytes = fs::read(archive).with_context(|| format!("reading {}", archive.display()))?;
    install_bytes(&bytes, "file", plugins_dir, data_dir)
}

/// [`install`], from a package's bytes and the name of where they came from.
pub fn install_bytes(
    bytes: &[u8],
    source: &str,
    plugins_dir: &Path,
    data_dir: &Path,
) -> Result<Manifest> {
    recover(plugins_dir);
    let staging = plugins_dir.join(".staging").join(unique());
    fs::create_dir_all(&staging)?;
    let result = (|| {
        unpack(bytes, &staging)?;
        let manifest = manifest(&staging).context("in the package")?;
        if manifest.api != API_VERSION {
            bail!(
                "{} {} is built for plugin API {}, and this merlin has {API_VERSION}",
                manifest.title,
                manifest.version,
                manifest.api
            );
        }
        if let Some(component) = &manifest.component
            && !staging.join(component).is_file()
        {
            bail!("the package has no {}", component.display());
        }
        let target = plugins_dir.join(&manifest.name);
        if target
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            let from = target.read_link().unwrap_or_default();
            bail!(
                "{} is linked from {}; uninstall it first to install a package in its place",
                manifest.title,
                from.display()
            );
        }
        swap(&staging, &target, plugins_dir)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    // Only empty ones go, so another install under way keeps its own.
    let _ = fs::remove_dir(plugins_dir.join(".staging"));
    let _ = fs::remove_dir(plugins_dir.join(".trash"));
    let manifest = result?;
    record(
        data_dir,
        &manifest.name,
        Some(Installed {
            source: source.to_string(),
            version: manifest.version.clone(),
            sha256: sha256_hex(bytes),
        }),
    )?;
    Ok(manifest)
}

/// Unpacks a package into `into`, refusing anything but plain files at
/// plain relative paths.
fn unpack(bytes: &[u8], into: &Path) -> Result<()> {
    let mut tar = tar::Archive::new(GzDecoder::new(bytes));
    let mut total = 0u64;
    for entry in tar.entries().context("not a .merlin package")? {
        let mut entry = entry.context("not a .merlin package")?;
        let path = entry.path()?.into_owned();
        let shown = path.display().to_string();
        let plain = path.components().all(|c| matches!(c, Component::Normal(_)));
        if !plain || path.as_os_str().is_empty() {
            bail!("the package has {shown:?}, which points outside it");
        }
        match entry.header().entry_type() {
            tar::EntryType::Regular => {}
            tar::EntryType::Directory => continue,
            other => bail!("the package has {shown:?}, which is a {other:?}, not a file"),
        }
        total += entry.size();
        if total > MAX_UNPACKED {
            bail!(
                "the package unpacks to more than {} MB",
                MAX_UNPACKED / (1024 * 1024)
            );
        }
        let out = into.join(&path);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data)?;
        let mut file = fs::File::create(&out)?;
        file.write_all(&data)?;
    }
    Ok(())
}

/// Moves `staging` to `target`, putting what was there in `.trash/` first
/// and deleting it after.
fn swap(staging: &Path, target: &Path, plugins_dir: &Path) -> Result<()> {
    let trash = plugins_dir.join(".trash").join(unique());
    if target.exists() {
        fs::create_dir_all(&trash)?;
        let name = target.file_name().context("no plugin name")?;
        fs::rename(target, trash.join(name))
            .with_context(|| format!("moving {} aside", target.display()))?;
    }
    fs::rename(staging, target).with_context(|| format!("moving into {}", target.display()))?;
    let _ = fs::remove_dir_all(&trash);
    Ok(())
}

/// Tidies up after an install that stopped halfway: clears `.staging/`,
/// and puts back a plugin that went to `.trash/` with nothing in its place.
pub fn recover(plugins_dir: &Path) {
    let _ = fs::remove_dir_all(plugins_dir.join(".staging"));
    let trash = plugins_dir.join(".trash");
    let Ok(batches) = fs::read_dir(&trash) else {
        return;
    };
    for batch in batches.flatten() {
        for old in fs::read_dir(batch.path()).into_iter().flatten().flatten() {
            let target = plugins_dir.join(old.file_name());
            if target.symlink_metadata().is_err() {
                let _ = fs::rename(old.path(), &target);
            }
        }
    }
    let _ = fs::remove_dir_all(&trash);
}

/// Whether `dir` is a plugin installed from a package: a real folder,
/// not a link, right in `plugins_dir`.
pub fn is_packaged(dir: &Path, plugins_dir: &Path) -> bool {
    let (Ok(meta), Some(parent)) = (dir.symlink_metadata(), dir.parent()) else {
        return false;
    };
    meta.is_dir() && parent.canonicalize().ok() == plugins_dir.canonicalize().ok()
}

/// Links the plugin folder `dir` into `plugins_dir` as `name`. Where the
/// system won't make links (Windows without Developer Mode), it packs the
/// folder and installs that instead, and says so with `Ok(false)`.
pub fn link(dir: &Path, name: &str, plugins_dir: &Path, data_dir: &Path) -> Result<bool> {
    let link = plugins_dir.join(name);
    if link.symlink_metadata().is_ok() {
        if is_packaged(&link, plugins_dir) {
            bail!(
                "{name} is installed from a package; uninstall it in Settings › Plugins to link this folder in its place"
            );
        }
        if link.canonicalize().ok().as_deref() != Some(dir) {
            bail!("{} is already installed from elsewhere", link.display());
        }
        return Ok(true);
    }
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(dir, &link);
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(dir, &link);
    #[cfg(not(any(unix, windows)))]
    let linked: std::io::Result<()> = Err(std::io::ErrorKind::Unsupported.into());
    match linked {
        Ok(()) => Ok(true),
        Err(_) if cfg!(windows) => {
            let (bytes, _, _) = pack_bytes(dir, 0)?;
            install_bytes(&bytes, "folder", plugins_dir, data_dir)?;
            Ok(false)
        }
        Err(err) => Err(err.into()),
    }
}

fn unique() -> String {
    format!("{}-{}", std::process::id(), nanos())
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new(what: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("merlin-package-{what}-{}", unique()));
            fs::create_dir_all(&dir).unwrap();
            Temp(dir)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A plugin folder with a fake component, a theme, build files and an
    /// excluded folder.
    fn plugin(root: &Path, version: &str) -> PathBuf {
        let dir = root.join("src-plugin");
        let write = |rel: &str, text: &str| {
            let path = dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "plugin.toml",
            &format!(
                "# A test plugin.\nname = \"demo\"\ntitle = \"Demo\"\nversion = \"{version}\"\n\
                 api = \"{API_VERSION}\"\ncomponent = \"target/wasm32-wasip2/release/demo.wasm\"\n\n\
                 [package]\nexclude = [\"tools/\"]\n\n[[themes]]\nlabel = \"Dusk\"\nfile = \"themes/dusk.toml\"\n"
            ),
        );
        write("target/wasm32-wasip2/release/demo.wasm", "\0asm not really");
        write("themes/dusk.toml", "[colors]\n");
        write("README.md", "Demo\n");
        write("src/lib.rs", "// source\n");
        write("Cargo.toml", "[package]\n");
        write("tools/make-theme.py", "print()\n");
        dir
    }

    fn names(bytes: &[u8]) -> Vec<String> {
        let mut tar = tar::Archive::new(GzDecoder::new(bytes));
        tar.entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().display().to_string())
            .collect()
    }

    #[test]
    fn packing_twice_gives_the_same_bytes() {
        let tmp = Temp::new("same");
        let dir = plugin(&tmp.0, "0.1.0");
        let (a, _, _) = pack_bytes(&dir, 1_700_000_000).unwrap();
        // A file touched later changes nothing.
        fs::write(dir.join("README.md"), "Demo\n").unwrap();
        let (b, _, _) = pack_bytes(&dir, 1_700_000_000).unwrap();
        assert_eq!(sha256_hex(&a), sha256_hex(&b));
        let (c, _, _) = pack_bytes(&dir, 1_700_000_001).unwrap();
        assert_ne!(sha256_hex(&a), sha256_hex(&c), "the epoch is in the bytes");
    }

    #[test]
    fn a_package_holds_what_runs_and_not_what_builds() {
        let tmp = Temp::new("contents");
        let dir = plugin(&tmp.0, "0.1.0");
        let packed = pack(&dir, &tmp.0.join("out"), 0).unwrap();
        assert_eq!(packed.path.file_name().unwrap(), "demo-0.1.0.merlin");
        let bytes = fs::read(&packed.path).unwrap();
        assert_eq!(packed.sha256, sha256_hex(&bytes));
        assert_eq!(
            names(&bytes),
            [
                "README.md",
                "plugin.toml",
                "plugin.wasm",
                "themes/dusk.toml"
            ]
        );
        // The manifest points at the packed component and keeps its comments.
        let mut tar = tar::Archive::new(GzDecoder::new(bytes.as_slice()));
        let mut text = String::new();
        for entry in tar.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.path().unwrap().to_str() == Some("plugin.toml") {
                entry.read_to_string(&mut text).unwrap();
            }
        }
        assert!(text.contains("component = \"plugin.wasm\""), "{text}");
        assert!(text.starts_with("# A test plugin."), "{text}");
    }

    #[test]
    fn a_missing_named_file_stops_packing() {
        let tmp = Temp::new("missing");
        let dir = plugin(&tmp.0, "0.1.0");
        fs::remove_file(dir.join("themes/dusk.toml")).unwrap();
        let err = format!("{:#}", pack_bytes(&dir, 0).unwrap_err());
        assert!(err.contains("themes/dusk.toml"), "{err}");
    }

    #[test]
    fn installing_swaps_in_the_new_version() {
        let tmp = Temp::new("install");
        let (plugins, data) = (tmp.0.join("plugins"), tmp.0.join("data"));
        let dir = plugin(&tmp.0, "0.1.0");
        let first = pack(&dir, &tmp.0.join("out"), 0).unwrap();
        let manifest = install(&first.path, &plugins, &data).unwrap();
        assert_eq!(manifest.version, "0.1.0");
        let installed_dir = plugins.join("demo");
        assert!(installed_dir.join("plugin.wasm").is_file());
        assert!(is_packaged(&installed_dir, &plugins));
        // The installed folder loads as a plugin.
        assert_eq!(crate::manifest(&installed_dir).unwrap().name, "demo");

        let dir = plugin(&tmp.0, "0.2.0");
        let second = pack(&dir, &tmp.0.join("out"), 0).unwrap();
        install(&second.path, &plugins, &data).unwrap();
        assert_eq!(crate::manifest(&installed_dir).unwrap().version, "0.2.0");
        assert!(!plugins.join(".staging").exists() && !plugins.join(".trash").exists());
        let record = &installed(&data)["demo"];
        assert_eq!(record.version, "0.2.0");
        assert_eq!(record.sha256, second.sha256);
        assert_eq!(record.source, "file");
    }

    #[test]
    fn a_package_for_another_api_is_refused() {
        let tmp = Temp::new("api");
        let dir = plugin(&tmp.0, "0.1.0");
        let toml = dir.join("plugin.toml");
        let text = fs::read_to_string(&toml).unwrap();
        fs::write(
            &toml,
            text.replace(&format!("api = \"{API_VERSION}\""), "api = \"0.1\""),
        )
        .unwrap();
        let (bytes, _, _) = pack_bytes(&dir, 0).unwrap();
        let plugins = tmp.0.join("plugins");
        let err = format!(
            "{:#}",
            install_bytes(&bytes, "file", &plugins, &tmp.0).unwrap_err()
        );
        assert!(err.contains("built for plugin API 0.1"), "{err}");
        assert!(!plugins.join("demo").exists());
        assert!(!plugins.join(".staging").exists());
    }

    #[test]
    fn paths_out_of_the_package_are_refused() {
        let tmp = Temp::new("escape");
        let mut tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        let body = b"x";
        header.set_size(1);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        // set_path refuses `..`, so write the name by hand.
        header.as_old_mut().name[..9].copy_from_slice(b"../escape");
        header.set_cksum();
        tar.append(&header, body.as_slice()).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(&tar.into_inner().unwrap()).unwrap();
        let bytes = gz.finish().unwrap();
        let into = tmp.0.join("into");
        fs::create_dir_all(&into).unwrap();
        let err = format!("{:#}", unpack(&bytes, &into).unwrap_err());
        assert!(err.contains("outside"), "{err}");
        assert!(!tmp.0.join("escape").exists());
    }

    #[test]
    fn recovering_puts_back_a_half_swapped_plugin() {
        let tmp = Temp::new("recover");
        let plugins = tmp.0.join("plugins");
        fs::create_dir_all(plugins.join(".trash/1/demo")).unwrap();
        fs::write(plugins.join(".trash/1/demo/plugin.toml"), "x").unwrap();
        fs::create_dir_all(plugins.join(".staging/2")).unwrap();
        recover(&plugins);
        assert!(plugins.join("demo/plugin.toml").is_file());
        assert!(!plugins.join(".trash").exists() && !plugins.join(".staging").exists());
    }
}
