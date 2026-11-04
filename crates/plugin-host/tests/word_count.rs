//! Runs the example plugin. Build it first:
//! `cargo build --release --manifest-path plugins/word-count/Cargo.toml`

use std::fs;
use std::path::{Path, PathBuf};

use merlin_core::Event;
use merlin_plugins::{Config, PluginHost, Snapshot};

fn plugin_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/word-count");
    let wasm = dir.join("target/wasm32-wasip2/release/word_count.wasm");
    assert!(
        wasm.exists(),
        "build the word-count plugin first (see the top of this file)"
    );
    dir
}

/// A copy of the plugin with its own manifest, in a fresh folder.
fn copy_with_manifest(name: &str, permissions: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("merlin-test-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::copy(
        plugin_dir().join("target/wasm32-wasip2/release/word_count.wasm"),
        dir.join("plugin.wasm"),
    )
    .unwrap();
    fs::write(
        dir.join("plugin.toml"),
        format!(
            "name = \"{name}\"\ntitle = \"Test\"\napi = \"0.8\"\ncomponent = \"plugin.wasm\"\n\n[permissions]\n{permissions}\n"
        ),
    )
    .unwrap();
    dir
}

/// Grants permissions without asking, and keeps nothing outside a temp
/// folder.
fn config() -> Config {
    Config {
        data_dir: std::env::temp_dir().join(format!("merlin-test-data-{}", std::process::id())),
        ask: false,
        keyring: false,
    }
}

fn snapshot(text: &str) -> Snapshot {
    Snapshot {
        path: None,
        text: text.into(),
        root: std::env::temp_dir(),
        cursor: (0, 0),
        selection: None,
    }
}

#[test]
fn loads_and_registers_commands() {
    let mut host = PluginHost::new(config()).unwrap();
    let events = host.load(&plugin_dir()).unwrap();
    let Some(Event::RegisterPlugin { plugin, commands }) = events.first() else {
        panic!("expected registration, got {events:?}");
    };
    assert_eq!(plugin, "word-count");
    assert_eq!(commands.len(), 2);
}

#[test]
fn commands_read_the_snapshot() {
    let mut host = PluginHost::new(config()).unwrap();
    host.load(&plugin_dir()).unwrap();
    let events = host.run_command("word-count", "show", snapshot("one two three"));
    assert!(
        matches!(&events[..], [Event::Notify(m)] if m == "3 words, 1 lines"),
        "{events:?}"
    );
}

#[test]
fn ungranted_permissions_are_denied() {
    let dir = copy_with_manifest("readonly", "editor-read = true");
    let mut host = PluginHost::new(config()).unwrap();
    host.load(&dir).unwrap();
    let events = host.run_command("readonly", "todo", snapshot(""));
    assert!(
        matches!(&events[..], [Event::Notify(m)] if m == "permission denied: editor-write"),
        "{events:?}"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reloads_when_the_component_changes() {
    let dir = copy_with_manifest("reloading", "editor-read = true");
    let mut host = PluginHost::new(config()).unwrap();
    host.load(&dir).unwrap();
    assert!(host.reload_changed().is_empty());

    let wasm = dir.join("plugin.wasm");
    let later =
        fs::metadata(&wasm).unwrap().modified().unwrap() + std::time::Duration::from_secs(5);
    fs::File::options()
        .write(true)
        .open(&wasm)
        .unwrap()
        .set_modified(later)
        .unwrap();

    let events = host.reload_changed();
    assert!(
        matches!(events.first(), Some(Event::RegisterPlugin { .. })),
        "{events:?}"
    );
    assert!(matches!(events.last(), Some(Event::Notify(m)) if m == "Reloaded reloading"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_other_api_versions() {
    let dir = copy_with_manifest("future", "");
    let manifest = fs::read_to_string(dir.join("plugin.toml")).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        manifest.replace("api = \"0.8\"", "api = \"0.9\""),
    )
    .unwrap();
    let err = PluginHost::new(config()).unwrap().load(&dir).unwrap_err();
    assert!(err.to_string().contains("needs plugin API 0.9"), "{err}");
    fs::remove_dir_all(dir).unwrap();
}
