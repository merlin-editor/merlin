//! Secrets plugins store, such as API keys. Each plugin sees only its own.
//!
//! They go to the system keyring through `secret-tool` (libsecret) when it's
//! installed. Otherwise they go to a file per plugin that only the user can
//! read, which is no worse than the plain-text config files plugins used
//! before, but not encrypted.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

pub(crate) struct Secrets {
    dir: PathBuf,
    keyring: bool,
    /// Whether each plugin's secret is set, as last seen, so the Settings
    /// page doesn't ask the keyring on every frame.
    known: Mutex<HashMap<(String, String), bool>>,
}

impl Secrets {
    /// Uses the keyring if asked and `secret-tool` is on `PATH`, else files
    /// in `dir`.
    pub fn new(dir: PathBuf, keyring: bool) -> Self {
        Self {
            dir,
            keyring: keyring && on_path("secret-tool"),
            known: Mutex::new(HashMap::new()),
        }
    }

    /// Whether a plugin has stored this secret, asking the store once.
    pub fn is_set(&self, plugin: &str, key: &str) -> bool {
        let id = (plugin.to_string(), key.to_string());
        if let Some(set) = self.known.lock().ok().and_then(|k| k.get(&id).copied()) {
            return set;
        }
        self.get(plugin, key).is_some()
    }

    fn remember(&self, plugin: &str, key: &str, set: bool) {
        if let Ok(mut known) = self.known.lock() {
            known.insert((plugin.to_string(), key.to_string()), set);
        }
    }

    pub fn get(&self, plugin: &str, key: &str) -> Option<String> {
        let value = self.read(plugin, key);
        self.remember(plugin, key, value.is_some());
        value
    }

    fn read(&self, plugin: &str, key: &str) -> Option<String> {
        if self.keyring {
            let output = secret_tool(&["lookup"], plugin, key)
                .stdin(Stdio::null())
                .output()
                .ok()?;
            return output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).into_owned());
        }
        self.load(plugin).remove(key)
    }

    pub fn set(&self, plugin: &str, key: &str, value: &str) -> Result<(), String> {
        let done = self.write(plugin, key, value);
        if done.is_ok() {
            self.remember(plugin, key, true);
        }
        done
    }

    fn write(&self, plugin: &str, key: &str, value: &str) -> Result<(), String> {
        if self.keyring {
            let label = format!("--label=merlin {plugin} {key}");
            let mut child = secret_tool(&["store", &label], plugin, key)
                .stdin(Stdio::piped())
                .spawn()
                .map_err(|e| format!("secret-tool: {e}"))?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(value.as_bytes())
                    .map_err(|e| e.to_string())?;
            }
            let status = child.wait().map_err(|e| e.to_string())?;
            return if status.success() {
                Ok(())
            } else {
                Err("secret-tool couldn't store the secret".into())
            };
        }
        let mut secrets = self.load(plugin);
        secrets.insert(key.into(), value.into());
        self.save(plugin, &secrets)
    }

    pub fn delete(&self, plugin: &str, key: &str) {
        self.remember(plugin, key, false);
        if self.keyring {
            let _ = secret_tool(&["clear"], plugin, key).status();
            return;
        }
        let mut secrets = self.load(plugin);
        if secrets.remove(key).is_some() {
            let _ = self.save(plugin, &secrets);
        }
    }

    fn file(&self, plugin: &str) -> PathBuf {
        self.dir.join(format!("{plugin}.toml"))
    }

    fn load(&self, plugin: &str) -> BTreeMap<String, String> {
        fs::read_to_string(self.file(plugin))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, plugin: &str, secrets: &BTreeMap<String, String>) -> Result<(), String> {
        let text = toml::to_string(secrets).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.file(plugin);
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(0o600);
            // In case the file was made some other way.
            if path.exists() {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
        }
        let mut file = options.open(&path).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())
    }
}

fn secret_tool(args: &[&str], plugin: &str, key: &str) -> Command {
    let mut command = Command::new("secret-tool");
    command
        .args(args)
        .args(["application", "merlin", "plugin", plugin, "key", key])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}
