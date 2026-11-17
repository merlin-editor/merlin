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
    // Alt+O is the editor's own, which runs it.
    assert_eq!(key, Some(None));
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

/// clangd's call hierarchy, a level at a time, and the uses of a name, read
/// and written. Only where clangd is installed.
#[test]
fn calls_and_a_names_uses_come_from_clangd() {
    use merlin_core::HierarchyMode;
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
    let root = Folder::new("merlin-cpp-calls-");
    let text = "int area(int w, int h) { return w * h; }\n\
                int total() {\n    int t = 0;\n    t += area(1, 2);\n    return t;\n}\n\
                int main() { return total(); }\n";
    std::fs::write(root.join("a.cpp"), text).unwrap();
    let at = |cursor: (u32, u32)| Snapshot {
        root: root.to_path_buf(),
        cursor,
        ..snapshot("a.cpp", text)
    };
    host.buffer_event(&BufferEvent::Switched, &at((0, 5)));
    let wait = |host: &mut PluginHost, what: &str, find: &dyn Fn(&Event) -> bool| {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let (events, _) = host.poll_processes(|| at((0, 5)));
            if let Some(e) = events.into_iter().find(|e| find(e)) {
                return e;
            }
            assert!(Instant::now() < until, "no {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    // Callers of `area`, asked until clangd has read the file.
    let mut shown = None;
    for _ in 0..20 {
        host.run_command("cpp", "call-hierarchy", at((0, 5)));
        let e = wait(&mut host, "a hierarchy or a notice", &|e| {
            matches!(e, Event::ShowHierarchy { .. } | Event::PluginNotice { .. })
        });
        if let Event::ShowHierarchy {
            mode,
            root,
            children,
            ..
        } = e
        {
            shown = Some((mode, root, children));
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let (mode, root_item, children) = shown.expect("clangd's call hierarchy");
    assert_eq!(mode, HierarchyMode::Callers);
    assert_eq!(root_item.name, "area");
    let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["total"]);
    assert_eq!(children[0].location.start.0, 3, "where total calls it");
    // The level under total: main.
    let total = children[0].id;
    host.run_command("cpp", &format!("hierarchy-expand:{total}"), at((0, 5)));
    let Event::HierarchyChildren {
        parent, children, ..
    } = wait(&mut host, "total's callers", &|e| {
        matches!(e, Event::HierarchyChildren { .. })
    })
    else {
        unreachable!()
    };
    assert_eq!(parent, total);
    assert_eq!(children[0].name, "main");
    // The other way from area: it calls nothing.
    host.run_command("cpp", "hierarchy-mode:callees", at((0, 5)));
    let Event::ShowHierarchy { mode, children, .. } = wait(&mut host, "callees", &|e| {
        matches!(e, Event::ShowHierarchy { .. })
    }) else {
        unreachable!()
    };
    assert_eq!(mode, HierarchyMode::Callees);
    assert!(children.is_empty(), "{children:?}");

    // `t`'s uses: written where it's set and added to, read where returned.
    host.run_command("cpp", "highlight", at((2, 8)));
    let Event::SetHighlights { ranges, .. } = wait(&mut host, "highlights", &|e| {
        matches!(e, Event::SetHighlights { .. })
    }) else {
        unreachable!()
    };
    let lines: Vec<(u32, bool)> = ranges.iter().map(|(s, _, w)| (s.0, *w)).collect();
    assert!(lines.contains(&(4, false)), "{lines:?}");
    assert!(
        lines.iter().any(|&(l, w)| l == 2 && w) || lines.contains(&(2, false)),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 3, "{lines:?}");
}

/// A file clangd has no compile command for: a bar says it's guessing the
/// flags, with Set Include Folders. Only where clangd is installed.
#[test]
fn a_bar_says_when_clangd_guesses_the_flags() {
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
    let root = Folder::new("merlin-cpp-guess-");
    let text = "#include \"engine.h\"\nint main() { return 0; }\n";
    std::fs::write(root.join("a.cpp"), text).unwrap();
    let at = Snapshot {
        root: root.to_path_buf(),
        ..snapshot("a.cpp", text)
    };
    let mut events = host.buffer_event(&BufferEvent::Switched, &at);
    let until = Instant::now() + Duration::from_secs(30);
    let bar = loop {
        if let Some(bar) = events.iter().find_map(|e| match e {
            Event::ShowBar {
                id, text, buttons, ..
            } => Some((id.clone(), text.clone(), buttons.clone())),
            _ => None,
        }) {
            break bar;
        }
        assert!(Instant::now() < until, "no bar");
        std::thread::sleep(Duration::from_millis(50));
        events = host.poll_processes(|| at.clone()).0;
    };
    assert_eq!(bar.0, "no-compile-commands");
    assert!(bar.1.contains("guessing the flags"), "{}", bar.1);
    assert_eq!(
        bar.2.last().map(|b| b.id.as_str()),
        Some("run:settings.plugin:cpp")
    );
    // Hidden by hand, it stays hidden.
    host.click("cpp", "dismiss:no-compile-commands", at.clone());
    let events = host.buffer_event(&BufferEvent::Switched, &at);
    assert!(
        !events.iter().any(|e| matches!(e, Event::ShowBar { .. })),
        "{events:?}"
    );
}

/// Problems' Check Project: every file compile_commands.json lists,
/// checked by clangd, its errors in Problems. Only where clangd is
/// installed.
#[test]
fn check_project_puts_every_files_errors_in_problems() {
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
    let root = Folder::new("merlin-cpp-check-");
    let good = "int main() {\n  int unused;\n  return 0;\n}\n";
    std::fs::write(root.join("good.cpp"), good).unwrap();
    std::fs::write(root.join("bad.cpp"), "int f() {\n  return x;\n}\n").unwrap();
    let dir = root.display().to_string();
    std::fs::write(
        root.join("compile_commands.json"),
        format!(
            r#"[{{"directory": "{dir}", "file": "good.cpp", "arguments": ["c++", "-Wall", "-c", "good.cpp"]}},
                {{"directory": "{dir}", "file": "bad.cpp", "arguments": ["c++", "-c", "bad.cpp"]}}]"#
        ),
    )
    .unwrap();
    let at = Snapshot {
        root: root.to_path_buf(),
        ..snapshot("good.cpp", good)
    };
    let mut events = host.run_command("cpp", "check-project", at.clone());
    let mut warned = Vec::new();
    let mut last_bad: Option<usize> = None;
    let until = Instant::now() + Duration::from_secs(60);
    let mut found = Vec::new();
    loop {
        for e in &events {
            if let Event::SetDiagnostics {
                path, diagnostics, ..
            } = e
                && path.ends_with("bad.cpp")
            {
                last_bad = Some(diagnostics.len());
            }
            match e {
                Event::SetDiagnostics {
                    path, diagnostics, ..
                } if path.ends_with("bad.cpp") && !diagnostics.is_empty() => {
                    found = diagnostics.clone()
                }
                Event::SetDiagnostics {
                    path, diagnostics, ..
                } if path.ends_with("good.cpp") && !diagnostics.is_empty() => {
                    warned = diagnostics.clone()
                }
                _ => {}
            }
        }
        if events
            .iter()
            .any(|e| matches!(e, Event::ProgressEnd { .. }))
        {
            break;
        }
        assert!(Instant::now() < until, "the check didn't end");
        std::thread::sleep(Duration::from_millis(50));
        events = host.poll_processes(|| at.clone()).0;
    }
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].start.0, 1);
    assert!(
        found[0].message.contains("undeclared identifier 'x'"),
        "{}",
        found[0].message
    );
    // Closed again, its problems stay.
    std::thread::sleep(Duration::from_millis(500));
    for e in host.poll_processes(|| at.clone()).0 {
        if let Event::SetDiagnostics {
            path, diagnostics, ..
        } = e
            && path.ends_with("bad.cpp")
        {
            last_bad = Some(diagnostics.len());
        }
    }
    assert_eq!(last_bad, Some(1), "bad.cpp's problems were cleared");
    // Warnings too, from the running clangd, as for an open file.
    assert!(
        warned
            .iter()
            .any(|d| d.severity == merlin_core::Severity::Warning && d.message.contains("unused")),
        "{warned:?}"
    );
}

/// With no compile_commands.json, clangd knows only open files: the
/// palette's symbols come from a scan of the sources, marked, with a note
/// saying why, and the ones in files excluded from search held back.
#[test]
fn symbols_without_compile_commands_come_from_a_scan() {
    use merlin_core::exclude::{Exclusions, Matcher};
    let (mut host, _) = host();
    let root = Folder::new("merlin-cpp-scan-");
    for (path, text) in [
        (
            "src/circle.cpp",
            "#include \"shape.h\"\ndouble Circle::area() const {\n    return 3.14 * r * r;\n}\n",
        ),
        (
            "include/shape.h",
            "struct Shape {\n    virtual double area() const = 0;\n};\n",
        ),
        ("third_party/z.cpp", "int z_area(int a) { return a; }\n"),
        ("README.md", "double area() {}\n"),
    ] {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }
    let at = Snapshot {
        root: root.to_path_buf(),
        exclusions: std::sync::Arc::new(Matcher::new(Exclusions {
            search: vec!["third_party/".into()],
            ..Exclusions::default()
        })),
        ..Snapshot::default()
    };
    let events = host.search_symbols("cpp", "project-symbols", "area", at);
    let Some(Event::ProjectSymbols {
        symbols,
        excluded,
        note,
        ..
    }) = events
        .iter()
        .find(|e| matches!(e, Event::ProjectSymbols { .. }))
    else {
        panic!("{events:?}");
    };
    let found: Vec<(String, String, String, u32, bool)> = symbols
        .iter()
        .map(|s| {
            let path = s.path.display().to_string();
            (s.name.clone(), s.container.clone(), path, s.line, s.scan)
        })
        .collect();
    assert!(
        found.contains(&(
            "area".into(),
            "Circle".into(),
            "src/circle.cpp".into(),
            1,
            true
        )),
        "{found:?}"
    );
    assert!(
        found.contains(&(
            "area".into(),
            "Shape".into(),
            "include/shape.h".into(),
            1,
            true
        )),
        "{found:?}"
    );
    assert!(
        !found.iter().any(|f| f.2 == "README.md"),
        "only C and C++ files"
    );
    assert_eq!(excluded.len(), 1, "{excluded:?}");
    assert_eq!(excluded[0].name, "z_area");
    assert!(
        note.warning.contains("No compile_commands.json"),
        "{note:?}"
    );
}

/// Keep clangd out of excluded folders: a block in .clangd, the user's
/// own lines left alone, and taken out again when it's turned off.
#[test]
fn excluded_folders_are_kept_out_of_clangds_index() {
    use merlin_core::exclude::{Exclusions, Matcher};
    let (mut host, _) = host();
    let root = Folder::new("merlin-cpp-clangd-");
    let mine = "CompileFlags:\n  Add: [-Wall]\n";
    std::fs::write(root.join(".clangd"), mine).unwrap();
    let at = Snapshot {
        root: root.to_path_buf(),
        exclusions: std::sync::Arc::new(Matcher::new(Exclusions {
            files: vec!["third_party/".into()],
            ..Exclusions::default()
        })),
        ..Snapshot::default()
    };
    let on: toml::Table = toml::from_str("[cpp]\nindex-excluded = true\n").unwrap();
    host.set_settings(on, toml::Table::new(), &at);
    let text = std::fs::read_to_string(root.join(".clangd")).unwrap();
    assert!(text.starts_with(mine), "{text}");
    assert!(
        text.contains("PathMatch: ['(.*/)?third_party/.*']"),
        "{text}"
    );
    assert!(text.contains("Background: Skip"), "{text}");
    host.set_settings(toml::Table::new(), toml::Table::new(), &at);
    assert_eq!(std::fs::read_to_string(root.join(".clangd")).unwrap(), mine);
}
