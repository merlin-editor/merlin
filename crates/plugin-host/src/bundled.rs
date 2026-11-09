//! The plugins that come with merlin. A release archive's `plugins/`
//! folder holds the registry's own packages, byte for byte, and
//! `bundled.toml` listing them. At start merlin installs each one that
//! isn't there yet, or is older from the same registry, so a fresh install
//! works offline and an editor update brings plugins built for its API.
//! One that was uninstalled stays uninstalled. See section 9, A3, of
//! `docs/distribution.md`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::Manifest;
use crate::package::{self, sha256_hex};
use crate::registry::{self, DEFAULT_SOURCE, Fetch, Loaded};

/// The list in a bundle's folder.
pub const LIST: &str = "bundled.toml";

/// A package in the bundle.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Bundled {
    pub name: String,
    pub version: String,
    pub sha256: String,
    /// The package's file name in the folder.
    pub file: String,
}

#[derive(Default, Deserialize, Serialize)]
struct ListFile {
    #[serde(default)]
    plugins: Vec<Bundled>,
}

/// The bundle in `dir`; empty if there's none.
pub fn read(dir: &Path) -> Result<Vec<Bundled>> {
    let path = dir.join(LIST);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(toml::from_str::<ListFile>(&text)
            .with_context(|| format!("reading {}", path.display()))?
            .plugins),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

/// Downloads each plugin's newest version for this merlin from `loaded`
/// into `dir`, checked against the index, and lists them in
/// `bundled.toml`. Every plugin the index has when `names` is empty.
pub fn fetch(loaded: &Loaded, names: &[String], dir: &Path, fetch: Fetch) -> Result<Vec<Bundled>> {
    let names: Vec<String> = if names.is_empty() {
        loaded.index.plugins.keys().cloned().collect()
    } else {
        names.to_vec()
    };
    fs::create_dir_all(dir)?;
    let mut list = Vec::new();
    for name in &names {
        let Some(entry) = loaded.index.plugins.get(name) else {
            bail!("{} has no plugin named {name}", loaded.source.name);
        };
        let Some(version) = entry.newest() else {
            bail!(
                "{} has no version of {name} to bundle for plugin API {}: \
                 release one to the registry first",
                loaded.source.name,
                crate::API_VERSION
            );
        };
        let bytes = registry::download(loaded, name, version, fetch)?;
        let file = format!("{name}-{}.{}", version.version, package::EXTENSION);
        fs::write(dir.join(&file), &bytes)?;
        list.push(Bundled {
            name: name.clone(),
            version: version.version.clone(),
            sha256: version.sha256.to_ascii_lowercase(),
            file,
        });
    }
    let text = toml::to_string(&ListFile {
        plugins: list.clone(),
    })?;
    fs::write(dir.join(LIST), text)?;
    Ok(list)
}

/// What [`install`] did.
#[derive(Debug, Default)]
pub struct Done {
    /// Each plugin installed, with the version it replaced.
    pub installed: Vec<(Manifest, Option<String>)>,
    /// Each one that couldn't be, and why.
    pub failed: Vec<(String, String)>,
}

/// Installs the bundle in `dir`: each plugin not installed and never
/// uninstalled, or installed from merlin's registry at an older version.
/// A linked plugin, one from another source, and a folder of its own are
/// left alone. The caller grants what each asks for.
pub fn install(dir: &Path, plugins_dir: &Path, data_dir: &Path) -> Done {
    let mut done = Done::default();
    let list = match read(dir) {
        Ok(list) => list,
        Err(err) => {
            done.failed.push((LIST.into(), format!("{err:#}")));
            return done;
        }
    };
    let installed = package::installed(data_dir);
    let removed = package::removed(data_dir);
    for b in list {
        let target = plugins_dir.join(&b.name);
        let previous = match installed.get(&b.name) {
            _ if removed.contains(&b.name) => continue,
            _ if target
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink()) =>
            {
                continue;
            }
            Some(mine) if mine.source != DEFAULT_SOURCE => continue,
            Some(mine)
                if registry::compare_versions(&b.version, &mine.version)
                    != std::cmp::Ordering::Greater =>
            {
                continue;
            }
            Some(mine) => Some(mine.version.clone()),
            None if target.exists() => continue,
            None => None,
        };
        match install_one(dir, &b, plugins_dir, data_dir) {
            Ok(manifest) => done.installed.push((manifest, previous)),
            Err(err) => done.failed.push((b.name.clone(), format!("{err:#}"))),
        }
    }
    done
}

fn install_one(dir: &Path, b: &Bundled, plugins_dir: &Path, data_dir: &Path) -> Result<Manifest> {
    let plain = Path::new(&b.file)
        .file_name()
        .is_some_and(|f| f == b.file.as_str());
    if !plain {
        bail!("{} isn't a file name", b.file);
    }
    let bytes = fs::read(dir.join(&b.file)).with_context(|| format!("reading {}", b.file))?;
    if sha256_hex(&bytes) != b.sha256.to_ascii_lowercase() {
        bail!("{} doesn't match {LIST}", b.file);
    }
    fs::create_dir_all(plugins_dir)?;
    package::install_bytes(
        &bytes,
        DEFAULT_SOURCE,
        Some((&b.name, &b.version)),
        plugins_dir,
        data_dir,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("merlin-bundled-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A bundle of `demo` at `version` in `root/bundle-<version>`.
    fn bundle(root: &Path, version: &str) -> PathBuf {
        let src = root.join(format!("src-{version}"));
        fs::create_dir_all(src.join("target")).unwrap();
        fs::write(
            src.join("plugin.toml"),
            format!(
                "name = \"demo\"\ntitle = \"Demo\"\nversion = \"{version}\"\napi = \"{}\"\n\
                 component = \"target/demo.wasm\"\n",
                crate::API_VERSION
            ),
        )
        .unwrap();
        fs::write(src.join("target/demo.wasm"), "\0asm").unwrap();
        let dir = root.join(format!("bundle-{version}"));
        let packed = package::pack(&src, &dir, 0).unwrap();
        let file = packed
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let list = ListFile {
            plugins: vec![Bundled {
                name: "demo".into(),
                version: version.into(),
                sha256: packed.sha256,
                file,
            }],
        };
        fs::write(dir.join(LIST), toml::to_string(&list).unwrap()).unwrap();
        dir
    }

    #[test]
    fn a_bundle_installs_once_updates_and_respects_an_uninstall() {
        let tmp = Temp::new("first-run");
        let (plugins, data) = (tmp.0.join("plugins"), tmp.0.join("data"));
        let v1 = bundle(&tmp.0, "0.1.0");

        let done = install(&v1, &plugins, &data);
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert_eq!(done.installed.len(), 1);
        assert_eq!(package::installed(&data)["demo"].source, DEFAULT_SOURCE);

        // Already there: nothing to do at the next start.
        assert!(install(&v1, &plugins, &data).installed.is_empty());

        // A newer merlin's bundle updates it, saying from what.
        let v2 = bundle(&tmp.0, "0.2.0");
        let done = install(&v2, &plugins, &data);
        assert_eq!(done.installed[0].1.as_deref(), Some("0.1.0"));
        // An older bundle never takes it back.
        assert!(install(&v1, &plugins, &data).installed.is_empty());

        // Uninstalled, it stays gone; installed again, it's back in the
        // running for updates.
        fs::remove_dir_all(plugins.join("demo")).unwrap();
        package::record(&data, "demo", None).unwrap();
        assert!(install(&v2, &plugins, &data).installed.is_empty());
        assert!(!plugins.join("demo").exists());
    }

    #[test]
    fn a_package_that_doesnt_match_its_list_is_refused() {
        let tmp = Temp::new("tampered");
        let (plugins, data) = (tmp.0.join("plugins"), tmp.0.join("data"));
        let dir = bundle(&tmp.0, "0.1.0");
        let file = read(&dir).unwrap()[0].file.clone();
        let mut bytes = fs::read(dir.join(&file)).unwrap();
        bytes[100] ^= 1;
        fs::write(dir.join(&file), bytes).unwrap();
        let done = install(&dir, &plugins, &data);
        assert!(done.installed.is_empty());
        assert!(
            done.failed[0].1.contains("doesn't match"),
            "{:?}",
            done.failed
        );
    }

    #[test]
    fn plugins_from_elsewhere_are_left_alone() {
        let tmp = Temp::new("elsewhere");
        let (plugins, data) = (tmp.0.join("plugins"), tmp.0.join("data"));
        let dir = bundle(&tmp.0, "0.1.0");
        // A folder of its own, as a copy or a build.
        fs::create_dir_all(plugins.join("demo")).unwrap();
        assert!(install(&dir, &plugins, &data).installed.is_empty());
        fs::remove_dir_all(plugins.join("demo")).unwrap();
        // One from another source.
        package::record(
            &data,
            "demo",
            Some(package::Installed {
                source: "acme".into(),
                version: "0.0.1".into(),
                sha256: String::new(),
            }),
        )
        .unwrap();
        assert!(install(&dir, &plugins, &data).installed.is_empty());
    }
}
