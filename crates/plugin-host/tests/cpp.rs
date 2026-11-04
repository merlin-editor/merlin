//! The C/C++ language plugin. Build it first:
//! `cd plugins/cpp && cargo build --release`

use std::path::{Path, PathBuf};

use merlin_core::layers::Layer;
use merlin_core::{Event, Syntax};
use merlin_plugins::{BufferEvent, Config, PluginHost, Snapshot};

fn plugin_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/cpp");
    assert!(
        dir.join("target/wasm32-wasip2/release/cpp.wasm").exists(),
        "build the cpp plugin first (see the top of this file)"
    );
    dir
}

fn host() -> (PluginHost, Vec<Event>) {
    let config = Config {
        data_dir: std::env::temp_dir().join(format!("merlin-cpp-test-{}", std::process::id())),
        ask: false,
        keyring: false,
    };
    let mut host = PluginHost::new(config).unwrap();
    let events = host.load(&plugin_dir()).unwrap();
    (host, events)
}

fn snapshot(path: &str, text: &str) -> Snapshot {
    Snapshot {
        path: Some(path.into()),
        text: text.into(),
        root: std::env::temp_dir(),
        ..Snapshot::default()
    }
}

#[test]
fn adds_c_and_cpp() {
    let (_, events) = host();
    let Some(Event::RegisterLanguages { languages, .. }) = events
        .iter()
        .find(|e| matches!(e, Event::RegisterLanguages { .. }))
    else {
        panic!("{events:?}");
    };
    let names: Vec<&str> = languages.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["C", "C++"]);
    assert!(languages[1].extensions.contains(&"hpp".to_string()));
}

#[test]
fn colours_cpp_files_and_leaves_others() {
    let (mut host, _) = host();
    let events = host.buffer_event(
        &BufferEvent::Switched,
        &snapshot("a.cpp", "int main() {\n    return 0; // done\n}\n"),
    );
    let Some(Event::SetTokens { layer, tokens, .. }) =
        events.iter().find(|e| matches!(e, Event::SetTokens { .. }))
    else {
        panic!("{events:?}");
    };
    assert_eq!(*layer, Layer::Base);
    let found: Vec<_> = tokens.iter().map(|t| (t.start, t.kind)).collect();
    assert_eq!(
        found,
        [
            ((0, 0), Syntax::Keyword),
            ((0, 4), Syntax::Function),
            ((1, 4), Syntax::Keyword),
            ((1, 11), Syntax::Constant),
            ((1, 14), Syntax::Comment),
        ]
    );
    let events = host.buffer_event(&BufferEvent::Switched, &snapshot("a.rs", "fn main() {}"));
    assert!(!events.iter().any(|e| matches!(e, Event::SetTokens { .. })));
}
