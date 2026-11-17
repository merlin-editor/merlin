//! The C/C++ language plugin. Build it first:
//! `cd plugins/cpp && cargo build --release`

use std::path::{Path, PathBuf};

use merlin_core::layers::Layer;
use merlin_core::{Event, Syntax};
use merlin_plugins::{BufferEvent, PluginHost, Snapshot};

mod common;
use common::{Folder, Host};

fn plugin_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/cpp");
    assert!(
        dir.join("target/wasm32-wasip2/release/cpp.wasm").exists(),
        "build the cpp plugin first (see the top of this file)"
    );
    dir
}

fn host() -> (Host, Vec<Event>) {
    let mut host = Host::new("merlin-cpp-test-");
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
    assert!(
        languages[1].extensions.contains(&"h".to_string()),
        "a header is C++"
    );
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
        [
            "src/a.cpp",
            "3",
            "5",
            "warning",
            "-Wunused-variable",
            "unused variable 'x'"
        ]
    );
    assert_eq!(
        read("C:\\work\\a.cpp:10:2: fatal error: 'gone.h' file not found").unwrap(),
        [
            "C:\\work\\a.cpp",
            "10",
            "2",
            "fatal error",
            "",
            "'gone.h' file not found"
        ]
    );
    assert_eq!(
        read("a.c:7: error: expected ';'").unwrap(),
        ["a.c", "7", "", "error", "", "expected ';'"]
    );
    assert_eq!(
        read("src\\a.cpp(3,5): error C2065: 'x': undeclared identifier").unwrap(),
        [
            "src\\a.cpp",
            "3",
            "5",
            "error",
            "C2065",
            "'x': undeclared identifier"
        ]
    );
    // Notes say more about the problem above, and other lines aren't ones.
    assert!(read("src/a.cpp:4:1: note: declared here").is_none());
    assert!(read("[ 50%] Building CXX object a.cpp.o").is_none());
}

/// clangd's refactors come as commands that clangd runs, sending the edit
/// back: "Replace with deduced type" on `auto` turns it into `int`. Only
/// where clangd is installed.
#[test]
fn clangds_refactors_run_and_make_their_edit() {
    use std::time::{Duration, Instant};
    if std::process::Command::new("clangd")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("clangd isn't installed; skipping");
        return;
    }
    let (mut host, _) = host();
    let root = Folder::new("merlin-cpp-clangd-");
    let text = "int g() {\n    auto x = 1;\n    return x;\n}\n";
    std::fs::write(root.join("a.cpp"), text).unwrap();
    let at = Snapshot {
        root: root.to_path_buf(),
        cursor: (1, 5),
        ..snapshot("a.cpp", text)
    };
    host.buffer_event(&BufferEvent::Switched, &at);
    let wait = |host: &mut PluginHost, what: &str, find: &dyn Fn(&Event) -> bool| {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let (events, _) = host.poll_processes(|| at.clone());
            if let Some(e) = events.into_iter().find(|e| find(e)) {
                return e;
            }
            assert!(Instant::now() < until, "no {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    // Asked until clangd has read the file.
    let mut actions = Vec::new();
    for _ in 0..20 {
        host.run_command("cpp", "code-actions", at.clone());
        if let Event::ShowCodeActions(list) = wait(&mut host, "code actions", &|e| {
            matches!(e, Event::ShowCodeActions(_))
        }) && !list.is_empty()
        {
            actions = list;
            break;
        }
    }
    let action = actions
        .iter()
        .find(|a| a.title.contains("deduced type"))
        .unwrap_or_else(|| panic!("{actions:?}"));
    host.pick("cpp", "code-actions", &action.id, at.clone());
    let Event::ApplyEdits(edits) = wait(&mut host, "the refactor's edit", &|e| {
        matches!(e, Event::ApplyEdits(_))
    }) else {
        unreachable!()
    };
    assert_eq!(edits.len(), 1, "{edits:?}");
    // clangd lays the lines out in its own style as it goes, so what's
    // checked is the text after.
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let mut changes = edits[0].edits.clone();
    changes.sort_by_key(|c| std::cmp::Reverse(c.0));
    for ((l0, c0), (l1, c1), new) in changes {
        let (l0, c0, l1, c1) = (l0 as usize, c0 as usize, l1 as usize, c1 as usize);
        let joined = format!("{}{new}{}", &lines[l0][..c0], &lines[l1][c1..]);
        lines.splice(
            l0..=l1,
            joined.split('\n').map(String::from).collect::<Vec<_>>(),
        );
    }
    let after = lines.join("\n");
    assert!(
        after.contains("int x = 1;") && !after.contains("auto"),
        "{after}"
    );
}

/// Alt+O goes between a source file and its header, as clangd finds them.
/// Only where clangd is installed.
#[test]
fn switches_between_header_and_source() {
    use std::time::{Duration, Instant};
    if std::process::Command::new("clangd")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("clangd isn't installed; skipping");
        return;
    }
    let (mut host, events) = host();
    let key = events.iter().find_map(|e| match e {
        Event::RegisterPlugin { commands, .. } => commands
            .iter()
            .find(|c| c.id == "switch-source-header")
            .map(|c| c.key.clone()),
        _ => None,
    });
    assert_eq!(key, Some(Some("alt+o".to_string())));
    let root = Folder::new("merlin-cpp-switch-");
    std::fs::write(root.join("shape.h"), "int area();\n").unwrap();
    let text = "#include \"shape.h\"\nint area() { return 1; }\n";
    std::fs::write(root.join("shape.cpp"), text).unwrap();
    let at = Snapshot {
        root: root.to_path_buf(),
        ..snapshot("shape.cpp", text)
    };
    host.buffer_event(&BufferEvent::Switched, &at);
    host.run_command("cpp", "switch-source-header", at.clone());
    let until = Instant::now() + Duration::from_secs(30);
    let path = loop {
        let (events, _) = host.poll_processes(|| at.clone());
        if let Some(path) = events.into_iter().find_map(|e| match e {
            Event::OpenAt { path, .. } => Some(path),
            _ => None,
        }) {
            break path;
        }
        assert!(Instant::now() < until, "no header opened");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(path, Path::new("shape.h"));
}
