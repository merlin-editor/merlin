//! Temp folders for the tests, removed when they drop, even if the test
//! fails, so test runs don't fill the temp dir.
#![allow(dead_code)]

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use merlin_plugins::{Config, PluginHost};
use tempfile::TempDir;

/// A new folder in the temp dir, named from `prefix`, removed when this
/// drops. Derefs to a path inside it: the folder itself, or one picked
/// with [`Folder::sub`].
pub struct Folder {
    temp: TempDir,
    path: PathBuf,
}

impl Folder {
    pub fn new(prefix: &str) -> Self {
        let temp = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
        let path = temp.path().to_path_buf();
        Self { temp, path }
    }

    /// The same folder, kept until this drops, but pointing at `name`
    /// inside it.
    pub fn sub(self, name: &str) -> Self {
        let path = self.temp.path().join(name);
        Self {
            temp: self.temp,
            path,
        }
    }

    /// The temp folder itself, whatever [`Folder::sub`] points at.
    pub fn root(&self) -> &Path {
        self.temp.path()
    }
}

impl Deref for Folder {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Folder {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// The config the tests use: grants without asking, no keyring, nothing in
/// the background, and data in `data`.
pub fn config(data: &Path) -> Config {
    Config {
        data_dir: data.to_path_buf(),
        ask: false,
        keyring: false,
        background: false,
    }
}

/// A host with a data folder of its own, named from `prefix`. The host
/// drops first, then the folder is removed.
pub struct Host {
    host: PluginHost,
    _data: Folder,
}

impl Host {
    pub fn new(prefix: &str) -> Self {
        let data = Folder::new(prefix);
        let host = PluginHost::new(config(&data)).unwrap();
        Self { host, _data: data }
    }
}

impl Deref for Host {
    type Target = PluginHost;
    fn deref(&self) -> &PluginHost {
        &self.host
    }
}

impl DerefMut for Host {
    fn deref_mut(&mut self) -> &mut PluginHost {
        &mut self.host
    }
}
