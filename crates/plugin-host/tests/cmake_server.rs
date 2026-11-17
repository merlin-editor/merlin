//! The CMake plugin with the real neocmakelsp, where it's installed (on
//! PATH or in ~/.cargo/bin). Its own file, as it changes PATH for the
//! process. Build the plugin first: `cd plugins/cmake && cargo build
//! --release`

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use merlin_core::Event;
use merlin_plugins::{BufferEvent, Config, PluginHost, Snapshot};

mod common;
use common::Folder;

/// Puts ~/.cargo/bin on PATH, and says whether neocmakelsp runs.
fn server_here() -> bool {
    if let Some(home) = std::env::var_os("HOME") {
        let bin = PathBuf::from(home).join(".cargo/bin");
        let rest = std::env::var_os("PATH").unwrap_or_default();
        let path =
            std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&rest))).unwrap();
        // Safe: the only test in this process, before anything is spawned.
        unsafe { std::env::set_var("PATH", path) };
    }
    std::process::Command::new("neocmakelsp")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn wait(host: &mut PluginHost, at: &Snapshot, what: &str, find: impl Fn(&Event) -> bool) -> Event {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let (events, _) = host.poll_processes(|| at.clone());
        if let Some(e) = events.into_iter().find(&find) {
            return e;
        }
        assert!(Instant::now() < until, "no {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn neocmakelsp_completes_and_explains_cmake() {
    if !server_here() {
        eprintln!("neocmakelsp isn't installed; skipping");
        return;
    }
    let root = Folder::new("merlin-cmake-server-");
    let text = "cmake_minimum_required(VERSION 3.20)\nproject(demo CXX)\nadd_exec\n";
    std::fs::write(root.join("CMakeLists.txt"), text).unwrap();
    let mut host = PluginHost::new(Config {
        data_dir: root.join(".data"),
        ask: false,
        keyring: false,
        background: false,
    })
    .unwrap();
    host.load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/cmake"))
        .unwrap();
    let at = |cursor: (u32, u32)| Snapshot {
        path: Some("CMakeLists.txt".into()),
        text: text.into(),
        root: root.to_path_buf(),
        cursor,
        ..Snapshot::default()
    };
    host.buffer_event(&BufferEvent::Switched, &at((2, 8)));

    // Asked until the server has read the file.
    let mut found = Vec::new();
    for _ in 0..20 {
        host.run_command("cmake", "complete", at((2, 8)));
        if let Event::ShowCompletions(items) = wait(&mut host, &at((2, 8)), "completions", |e| {
            matches!(e, Event::ShowCompletions(_))
        }) && !items.is_empty()
        {
            found = items;
            break;
        }
    }
    let add = found
        .iter()
        .find(|c| c.label == "add_executable")
        .unwrap_or_else(|| panic!("{found:?}"));
    assert!(
        !add.insert.contains('$'),
        "a snippet goes in as plain text: {:?}",
        add.insert
    );

    host.run_command("cmake", "hover", at((1, 2)));
    let Event::ShowHover(hover) = wait(&mut host, &at((1, 2)), "hover", |e| {
        matches!(e, Event::ShowHover(_))
    }) else {
        unreachable!()
    };
    assert!(hover.to_lowercase().contains("project"), "{hover}");
}
