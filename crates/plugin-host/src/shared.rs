//! Values plugins share with each other, such as where the build put
//! `compile_commands.json`. Every plugin's thread holds the same board: a
//! plugin writes its own values and reads the others', and the host tells
//! the plugins that watch a key when it changes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// The most a shared value may be, as JSON.
pub(crate) const MAX_SHARED: usize = 64 * 1024;

/// What each plugin shares, by key, in the order they first shared it.
#[derive(Default)]
pub(crate) struct Board {
    values: BTreeMap<String, Vec<(String, String)>>,
}

pub(crate) type SharedBoard = Arc<Mutex<Board>>;

impl Board {
    /// Sets or takes back `plugin`'s value under `key`. Whether it changed.
    pub fn set(&mut self, key: &str, plugin: &str, value: Option<String>) -> bool {
        let list = self.values.entry(key.to_string()).or_default();
        let at = list.iter().position(|(p, _)| p == plugin);
        let changed = match (at, value) {
            (Some(i), Some(value)) if list[i].1 == value => false,
            (Some(i), Some(value)) => {
                list[i].1 = value;
                true
            }
            (None, Some(value)) => {
                list.push((plugin.to_string(), value));
                true
            }
            (Some(i), None) => {
                list.remove(i);
                true
            }
            (None, None) => false,
        };
        if list.is_empty() {
            self.values.remove(key);
        }
        changed
    }

    /// What plugins other than `except` share under `key`.
    pub fn get(&self, key: &str, except: &str) -> Vec<(String, String)> {
        self.values
            .get(key)
            .map(|list| list.iter().filter(|(p, _)| p != except).cloned().collect())
            .unwrap_or_default()
    }

    /// Takes back everything `plugin` shares, as when it stops, and returns
    /// the keys that changed.
    pub fn withdraw(&mut self, plugin: &str) -> Vec<String> {
        let keys: Vec<String> = self
            .values
            .iter()
            .filter(|(_, list)| list.iter().any(|(p, _)| p == plugin))
            .map(|(k, _)| k.clone())
            .collect();
        for key in &keys {
            self.set(key, plugin, None);
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_kept_per_plugin_in_order_and_withdrawn() {
        let mut board = Board::default();
        assert!(board.set("compile-commands", "meson", Some("1".into())));
        assert!(board.set("compile-commands", "cmake", Some("2".into())));
        // The same value again is no change.
        assert!(!board.set("compile-commands", "cmake", Some("2".into())));
        assert_eq!(
            board.get("compile-commands", "cpp"),
            [("meson".into(), "1".into()), ("cmake".into(), "2".into())]
        );
        // A plugin doesn't read its own.
        assert_eq!(
            board.get("compile-commands", "cmake"),
            [("meson".into(), "1".into())]
        );
        assert!(board.set("cmake.targets", "cmake", Some("[]".into())));
        assert_eq!(
            board.withdraw("cmake"),
            ["cmake.targets", "compile-commands"]
        );
        assert_eq!(
            board.get("compile-commands", "cpp"),
            [("meson".into(), "1".into())]
        );
        assert!(board.get("cmake.targets", "cpp").is_empty());
        assert!(!board.set("nothing", "cmake", None));
    }
}
