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
        background: false,
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
    assert!(languages[1].extensions.contains(&"h".to_string()), "a header is C++");
    assert_eq!(languages[0].extensions, ["c"]);
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

#[test]
fn reads_gcc_clang_and_msvc_problems() {
    let (_host, events) = host();
    let matchers = events
        .into_iter()
        .find_map(|e| match e {
            Event::RegisterProblemMatchers { matchers, .. } => Some(matchers),
            _ => None,
        })
        .expect("the plugin brings problem matchers");
    // Each line, and the groups the matcher that reads it finds: file,
    // line, column, severity, code and message.
    let read = |text: &str| {
        matchers.iter().find_map(|m| {
            let l = &m.lines[0];
            let caps = l.regex.captures(text)?;
            let group = |i: Option<usize>| {
                i.and_then(|i| caps.get(i))
                    .map_or("", |m| m.as_str())
                    .to_string()
            };
            Some([l.file, l.line, l.column, l.severity, l.code, l.message].map(group))
        })
    };
    assert_eq!(
        read("src/a.cpp:3:5: warning: unused variable 'x' [-Wunused-variable]").unwrap(),
        ["src/a.cpp", "3", "5", "warning", "-Wunused-variable", "unused variable 'x'"]
    );
    assert_eq!(
        read("C:\\work\\a.cpp:10:2: fatal error: 'gone.h' file not found").unwrap(),
        ["C:\\work\\a.cpp", "10", "2", "fatal error", "", "'gone.h' file not found"]
    );
    assert_eq!(
        read("a.c:7: error: expected ';'").unwrap(),
        ["a.c", "7", "", "error", "", "expected ';'"]
    );
    assert_eq!(
        read("src\\a.cpp(3,5): error C2065: 'x': undeclared identifier").unwrap(),
        ["src\\a.cpp", "3", "5", "error", "C2065", "'x': undeclared identifier"]
    );
    // Notes say more about the problem above, and other lines aren't ones.
    assert!(read("src/a.cpp:4:1: note: declared here").is_none());
    assert!(read("[ 50%] Building CXX object a.cpp.o").is_none());
}
