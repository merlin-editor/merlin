//! The Rust language plugin on big files. Build it first:
//! `cd plugins/rust && cargo build --release`

use std::path::{Path, PathBuf};

use merlin_core::Event;
use merlin_plugins::{BufferEvent, Config, PluginHost, Snapshot};

fn plugin_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/rust");
    assert!(
        dir.join("target/wasm32-wasip2/release/rust_language.wasm")
            .exists(),
        "build the rust plugin first (see the top of this file)"
    );
    dir
}

fn host() -> PluginHost {
    let config = Config {
        data_dir: std::env::temp_dir().join(format!("merlin-rust-test-{}", std::process::id())),
        ask: false,
        keyring: false,
        background: false,
    };
    let mut host = PluginHost::new(config).unwrap();
    host.load(&plugin_dir()).unwrap();
    host
}

/// About `bytes` of ordinary Rust.
fn rust(bytes: usize) -> String {
    let item = "/// Adds them.\npub fn add(a: u32, b: &str) -> Option<u32> {\n    \
                let n: u32 = b.parse().ok()?;\n    if n > 10 { return None; }\n    \
                Some(a + n) // done\n}\n\n";
    item.repeat(bytes / item.len() + 1)
}

fn open(host: &mut PluginHost, text: &str) -> Vec<Event> {
    let snapshot = Snapshot {
        path: Some("big.rs".into()),
        text: text.into(),
        root: std::env::temp_dir(),
        ..Snapshot::default()
    };
    host.buffer_event(&BufferEvent::Switched, &snapshot)
}

#[test]
fn colours_a_file_just_under_the_limit() {
    let mut host = host();
    let events = open(&mut host, &rust(500 * 1024));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::PluginFailed { .. })),
        "{events:?}"
    );
    assert!(events.iter().any(|e| matches!(e, Event::SetTokens { .. })));
}

#[test]
fn leaves_a_bigger_file_uncoloured_and_keeps_working() {
    let mut host = host();
    let events = open(&mut host, &rust(2_000_000));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::PluginFailed { .. })),
        "{events:?}"
    );
    assert!(!events.iter().any(|e| matches!(e, Event::SetTokens { .. })));
    // It still colours the next file.
    let events = open(&mut host, &rust(1000));
    assert!(events.iter().any(|e| matches!(e, Event::SetTokens { .. })));
}
