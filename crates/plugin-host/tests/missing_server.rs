//! A language server that isn't installed: the language card's hint is
//! the manifest's install command for this system. Its own file, as it
//! empties PATH for the whole test process. Build the C/C++ plugin first:
//! `cd plugins/cpp && cargo build --release`

use std::path::Path;
use std::time::{Duration, Instant};

use merlin_core::Event;
use merlin_plugins::{BufferEvent, Config, PluginHost, Snapshot};

mod common;
use common::Folder;

#[test]
fn a_missing_server_hints_the_manifests_install_command() {
    let empty = Folder::new("merlin-missing-path-");
    // Safe: the only test in this process, before anything is spawned.
    unsafe { std::env::set_var("PATH", &*empty) };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/cpp");
    let mut host = PluginHost::new(Config {
        data_dir: empty.join("data"),
        ask: false,
        keyring: false,
        background: false,
    })
    .unwrap();
    host.load(&dir).unwrap();
    let at = Snapshot {
        path: Some("a.cpp".into()),
        text: "int main() {}\n".into(),
        root: empty.to_path_buf(),
        ..Snapshot::default()
    };
    let mut events = host.buffer_event(&BufferEvent::Switched, &at);
    let until = Instant::now() + Duration::from_secs(10);
    let hint = loop {
        let found = events.iter().find_map(|e| match e {
            Event::SetStatus { lines, .. } => lines
                .iter()
                .find(|l| l.name == "clangd" && l.hint.is_some())
                .and_then(|l| l.hint.clone()),
            _ => None,
        });
        if let Some(hint) = found {
            break hint;
        }
        assert!(Instant::now() < until, "no hint");
        events = host.poll_processes(|| at.clone()).0;
        std::thread::sleep(Duration::from_millis(20));
    };
    let expected = match std::env::consts::OS {
        "linux" => "sudo apt install clangd",
        "macos" => "brew install llvm",
        _ => "winget install LLVM.LLVM",
    };
    assert_eq!(hint, expected);
}
