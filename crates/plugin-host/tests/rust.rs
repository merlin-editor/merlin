//! The Rust language plugin on big files. Build it first:
//! `cd plugins/rust && cargo build --release`

use std::path::{Path, PathBuf};

use merlin_core::Event;
use merlin_plugins::{BufferEvent, PluginHost, Snapshot};

mod common;
use common::Host;

fn plugin_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/rust");
    assert!(
        dir.join("target/wasm32-wasip2/release/rust_language.wasm")
            .exists(),
        "build the rust plugin first (see the top of this file)"
    );
    dir
}

fn host() -> Host {
    let mut host = Host::new("merlin-rust-test-");
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

/// The lines a colouring covered, and its tokens' lines.
fn coloured(events: &[Event]) -> Option<((u32, u32), Vec<u32>)> {
    events.iter().find_map(|e| match e {
        Event::SetTokens { lines, tokens, .. } => {
            Some(((*lines)?, tokens.iter().map(|t| t.start.0).collect()))
        }
        _ => None,
    })
}

fn at(text: merlin_core::buffer::TextSnapshot, view: (u32, u32)) -> Snapshot {
    Snapshot {
        path: Some("big.rs".into()),
        text,
        root: std::env::temp_dir(),
        view,
        ..Snapshot::default()
    }
}

#[test]
fn a_bigger_file_is_coloured_around_the_view() {
    let mut host = host();
    let mut buffer = merlin_core::buffer::Buffer::new(&rust(20 << 20), None);
    let events = host.buffer_event(
        &BufferEvent::Switched,
        &at(buffer.snapshot(), (300_000, 300_040)),
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::PluginFailed { .. })),
        "{events:?}"
    );
    let ((a, b), lines) = coloured(&events).expect("coloured");
    assert!(a <= 300_000 && b >= 300_040 && b - a <= 2_100, "{a}..{b}");
    assert!(!lines.is_empty() && lines.iter().all(|l| (a..b).contains(l)));
    // A little further down, the window holds: nothing more.
    let events = host.buffer_event(
        &BufferEvent::Viewed,
        &at(buffer.snapshot(), (300_100, 300_140)),
    );
    assert_eq!(coloured(&events), None);
    // Typing in it colours the lines it touched.
    buffer.begin(0, merlin_core::history::Kind::Other);
    let pos = buffer.rope().line_to_char(300_010);
    buffer.apply(merlin_core::edit::Edit {
        at: pos,
        removed: String::new(),
        inserted: "// new\n".into(),
    });
    buffer.commit(pos);
    let events = host.buffer_event(
        &BufferEvent::Changed,
        &at(buffer.snapshot(), (300_100, 300_140)),
    );
    let ((a, b), _) = coloured(&events).expect("coloured again");
    assert!(a <= 300_010 && b <= 300_020, "{a}..{b}");
    // Far away, the lines there.
    let events = host.buffer_event(
        &BufferEvent::Viewed,
        &at(buffer.snapshot(), (600_000, 600_040)),
    );
    let ((a, b), lines) = coloured(&events).expect("coloured where it is");
    assert!(a <= 600_000 && b >= 600_040 && b - a <= 2_100, "{a}..{b}");
    assert!(!lines.is_empty());
    // It still colours a small file whole.
    let events = open(&mut host, &rust(1000));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::SetTokens { lines: None, .. }))
    );
}

#[test]
fn typing_in_a_small_file_colours_from_the_changes() {
    let mut host = host();
    let mut buffer = merlin_core::buffer::Buffer::new(&rust(100 << 10), None);
    let first = host.buffer_event(&BufferEvent::Switched, &at(buffer.snapshot(), (0, 40)));
    assert!(
        first
            .iter()
            .any(|e| matches!(e, Event::SetTokens { lines: None, .. }))
    );
    buffer.begin(0, merlin_core::history::Kind::Other);
    let pos = buffer.rope().line_to_char(500);
    buffer.apply(merlin_core::edit::Edit {
        at: pos,
        removed: String::new(),
        inserted: "/* a comment */\n".into(),
    });
    buffer.commit(pos);
    let events = host.buffer_event(&BufferEvent::Changed, &at(buffer.snapshot(), (0, 40)));
    let ((a, b), lines) = coloured(&events).expect("coloured the change");
    assert!(a <= 500 && b <= 510, "{a}..{b}");
    assert!(lines.contains(&500));
}
