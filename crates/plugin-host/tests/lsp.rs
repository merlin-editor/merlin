//! Runs the Rust plugin's language server client against a fake
//! `rust-analyzer`. Build it first:
//! `cargo build --release --manifest-path plugins/rust/Cargo.toml`

use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, Instant};

use merlin_core::{Completion, Event, Severity};
use merlin_plugins::{BufferEvent, Config, PluginHost, Snapshot};

fn plugin_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/rust");
    let wasm = dir.join("target/wasm32-wasip2/release/rust_language.wasm");
    assert!(
        wasm.exists(),
        "build the rust plugin first (see the top of this file)"
    );
    dir
}

/// Puts the fake server first on PATH, so the plugin spawns it.
fn use_fake_server() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fake = plugin_dir().join("tests/fake");
        let path = format!(
            "{}:{}",
            fake.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        // Safe: set once, before any test in this file spawns a process.
        unsafe { std::env::set_var("PATH", path) };
    });
}

/// An empty project, so the plugin has no Cargo.toml files to read.
fn root() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("merlin-lsp-root-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn snapshot() -> Snapshot {
    Snapshot {
        path: Some("src/main.rs".into()),
        text: "fn main() {\n    let v = Vec::new();\n    v.\n}\n".into(),
        root: root(),
        cursor: (2, 6),
        selection: None,
        changed: None,
        menu_target: None,
    }
}

/// A host with the plugin loaded and the server told about main.rs.
fn started() -> PluginHost {
    use_fake_server();
    let config = Config {
        data_dir: std::env::temp_dir().join(format!("merlin-lsp-test-{}", std::process::id())),
        ask: false,
        keyring: false,
    };
    let mut host = PluginHost::new(config).unwrap();
    host.load(&plugin_dir()).unwrap();
    host.buffer_event(&BufferEvent::Switched, &snapshot());
    host
}

/// Polls process output until an event matches.
fn wait_for(host: &mut PluginHost, what: &str, matches: impl Fn(&Event) -> bool) -> Event {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let (events, _) = host.poll_processes(snapshot);
        if let Some(event) = events.into_iter().find(&matches) {
            return event;
        }
        assert!(Instant::now() < until, "never got {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn pushed_and_pulled_diagnostics_arrive_together() {
    let mut host = started();
    let both = |e: &Event| matches!(e, Event::SetDiagnostics { diagnostics, .. } if diagnostics.len() == 2);
    let event = wait_for(&mut host, "both kinds of diagnostics", both);
    let Event::SetDiagnostics {
        path, diagnostics, ..
    } = event
    else {
        unreachable!()
    };
    assert_eq!(path, Path::new("src/main.rs"), "made relative to the root");
    assert_eq!(diagnostics[0].start, (0, 3));
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(diagnostics[0].message, "fake error\nmore detail");
    assert_eq!(diagnostics[1].severity, Severity::Warning, "pulled");
}

#[test]
fn hover_keeps_its_markdown_for_the_card_to_draw() {
    let mut host = started();
    host.run_command("rust", "hover", snapshot());
    let event = wait_for(&mut host, "hover", |e| matches!(e, Event::ShowHover(_)));
    let Event::ShowHover(text) = event else {
        unreachable!()
    };
    assert_eq!(text, "```rust\nfn main()\n```\n\n---\n\nThe entry point.");
}

#[test]
fn definition_opens_the_location() {
    let mut host = started();
    host.run_command("rust", "definition", snapshot());
    let event = wait_for(&mut host, "definition", |e| {
        matches!(e, Event::OpenAt { .. })
    });
    let Event::OpenAt { path, line, column } = event else {
        unreachable!()
    };
    assert_eq!(
        (path.as_path(), line, column),
        (Path::new("src/main.rs"), 2, 4)
    );
}

#[test]
fn completions_come_sorted_with_their_insert_text() {
    let mut host = started();
    host.run_command("rust", "complete", snapshot());
    let event = wait_for(&mut host, "completions", |e| {
        matches!(e, Event::ShowCompletions(_))
    });
    let Event::ShowCompletions(items) = event else {
        unreachable!()
    };
    let pop = Completion {
        label: "pop".into(),
        detail: "() -> Option<T>".into(),
        insert: "pop()".into(),
    };
    assert_eq!(items[0], pop);
    assert_eq!(items[1].insert, "push");
}

#[test]
fn typing_a_dot_asks_for_completions() {
    let mut host = started();
    wait_for(&mut host, "diagnostics", |e| {
        matches!(e, Event::SetDiagnostics { .. })
    });
    // The snapshot's cursor is just after `v.`.
    host.buffer_event(&BufferEvent::Changed, &snapshot());
    wait_for(&mut host, "completions", |e| {
        matches!(e, Event::ShowCompletions(_))
    });
}

#[test]
fn semantic_tokens_symbols_and_folds_come_from_the_server() {
    use merlin_core::Syntax;
    use merlin_core::layers::Layer;
    let mut host = started();
    // All three answer each sync, so collect until each has come.
    let mut seen = Vec::new();
    let until = Instant::now() + Duration::from_secs(10);
    let has = |seen: &[Event], f: fn(&Event) -> bool| seen.iter().any(f);
    while !(has(&seen, |e| matches!(e, Event::SetTokens { .. }))
        && has(&seen, |e| matches!(e, Event::SetSymbols { .. }))
        && has(&seen, |e| matches!(e, Event::SetFolds { .. })))
    {
        assert!(Instant::now() < until, "only got {seen:?}");
        seen.extend(host.poll_processes(snapshot).0);
        std::thread::sleep(Duration::from_millis(10));
    }
    for event in seen {
        match event {
            Event::SetTokens {
                path,
                layer,
                tokens,
                ..
            } => {
                assert_eq!(path, std::path::PathBuf::from("src/main.rs"));
                assert_eq!(layer, Layer::Semantic);
                let found: Vec<_> = tokens.iter().map(|t| (t.start, t.end, t.kind)).collect();
                assert_eq!(
                    found,
                    [
                        ((0, 0), (0, 2), Syntax::Keyword),
                        ((0, 3), (0, 7), Syntax::Function),
                        ((1, 8), (1, 9), Syntax::Variable),
                    ]
                );
            }
            Event::SetSymbols { symbols, .. } => {
                let found: Vec<_> = symbols
                    .iter()
                    .map(|s| (s.name.as_str(), s.kind.as_str(), s.depth))
                    .collect();
                assert_eq!(found, [("main", "function", 0), ("v", "variable", 1)]);
            }
            Event::SetFolds { ranges, .. } => assert_eq!(ranges, [(0, 2)]),
            _ => {}
        }
    }
}

#[test]
fn code_actions_come_with_their_edits_and_one_chosen_is_made() {
    use merlin_core::lang::{ActionKind, FileEdit};
    let mut host = started();
    host.run_command("rust", "code-actions", snapshot());
    let event = wait_for(&mut host, "code actions", |e| {
        matches!(e, Event::ShowCodeActions(_))
    });
    let Event::ShowCodeActions(actions) = event else {
        unreachable!()
    };
    let listed: Vec<(&str, ActionKind, bool)> = actions
        .iter()
        .map(|a| (a.title.as_str(), a.kind, a.preferred))
        .collect();
    assert_eq!(
        listed,
        [
            ("Extract into a variable", ActionKind::Refactor, false),
            ("Rename to `_v`", ActionKind::QuickFix, true),
        ],
        "a bare command and a disabled action are left out"
    );
    let events = host.pick("rust", "code-actions", &actions[1].id, snapshot());
    let edits = events.into_iter().find_map(|e| match e {
        Event::ApplyEdits(edits) => Some(edits),
        _ => None,
    });
    assert_eq!(
        edits,
        Some(vec![FileEdit {
            path: "src/main.rs".into(),
            edits: vec![((1, 8), (1, 9), "_v".into())],
        }])
    );
}

/// The edits a format sent for the active file.
fn format_edits(
    host: &mut PluginHost,
    command: &str,
    snapshot: Snapshot,
) -> Vec<merlin_core::lang::TextEdit> {
    let mut events = host.run_command("rust", command, snapshot);
    let found = |events: &[Event]| {
        events.iter().find_map(|e| match e {
            Event::ApplyEdits(files) => Some(files[0].edits.clone()),
            _ => None,
        })
    };
    if let Some(edits) = found(&events) {
        return edits;
    }
    let event = wait_for(host, "formatting", |e| matches!(e, Event::ApplyEdits(_)));
    events = vec![event];
    found(&events).unwrap()
}

#[test]
fn formatting_sends_the_servers_edits_and_changed_lines_ask_for_their_ranges() {
    let mut host = started();
    assert_eq!(
        format_edits(&mut host, "format", snapshot()),
        [((1, 7), (1, 8), "  ".to_string())]
    );
    let changed = Snapshot {
        changed: Some(vec![(2, 2), (0, 0)]),
        menu_target: None,
        ..snapshot()
    };
    let mut edits = format_edits(&mut host, "format-changed", changed);
    edits.sort();
    assert_eq!(
        edits,
        [
            ((0, 0), (0, 0), "// formatted\n".to_string()),
            ((2, 0), (2, 0), "// formatted\n".to_string()),
        ],
        "a range for each, sent together"
    );
}

#[test]
fn a_file_not_in_git_saves_unformatted_and_says_so_once() {
    let mut host = started();
    let mut said = 0;
    for _ in 0..2 {
        let events = host.run_command("rust", "format-changed", snapshot());
        said += events
            .iter()
            .filter(
                |e| matches!(e, Event::PluginNotice { text, .. } if text.contains("isn't in git")),
            )
            .count();
        let empty = events
            .iter()
            .any(|e| matches!(e, Event::ApplyEdits(f) if f[0].edits.is_empty()));
        assert!(empty, "the save hears there's nothing to do");
    }
    assert_eq!(said, 1);
}

/// The snapshot with the cursor on `v`, in `let v`.
fn on_v() -> Snapshot {
    Snapshot {
        cursor: (1, 8),
        ..snapshot()
    }
}

/// Polls with the cursor on `v` until an event matches.
fn wait_on_v(host: &mut PluginHost, what: &str, matches: impl Fn(&Event) -> bool) -> Event {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let (events, _) = host.poll_processes(on_v);
        if let Some(event) = events.into_iter().find(&matches) {
            return event;
        }
        assert!(Instant::now() < until, "never got {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn rename_says_how_far_a_name_reaches_then_renames_or_sends_it_to_review() {
    use merlin_core::lang::{FileEdit, RenameInfo};
    let mut host = started();
    host.run_command("rust", "rename", on_v());
    let event = wait_on_v(&mut host, "the rename's reach", |e| {
        matches!(e, Event::ShowRename(_))
    });
    let Event::ShowRename(info) = event else {
        unreachable!()
    };
    assert_eq!(
        info,
        Ok(RenameInfo {
            start: (1, 8),
            end: (1, 9),
            places: 3,
            files: 2,
        })
    );
    let edits = vec![FileEdit {
        path: "src/main.rs".into(),
        edits: vec![((1, 8), (1, 9), "w".into()), ((2, 4), (2, 5), "w".into())],
    }];
    host.submit("rust", "rename-review", "w", on_v());
    let event = wait_on_v(&mut host, "the review", |e| {
        matches!(e, Event::ShowRenameReview(_))
    });
    let Event::ShowRenameReview(review) = event else {
        unreachable!()
    };
    assert_eq!(review, edits);
    // Each rename is prepared first.
    host.run_command("rust", "rename", on_v());
    wait_on_v(&mut host, "the reach", |e| {
        matches!(e, Event::ShowRename(_))
    });
    host.submit("rust", "rename", "w", on_v());
    let event = wait_on_v(&mut host, "the edits", |e| {
        matches!(e, Event::ApplyEdits(_))
    });
    let Event::ApplyEdits(made) = event else {
        unreachable!()
    };
    assert_eq!(made, edits);
}

#[test]
fn a_name_the_server_cant_rename_says_why() {
    let mut host = started();
    // Once it's ready, renames are prepared.
    host.run_command("rust", "hover", snapshot());
    wait_for(&mut host, "a hover", |e| matches!(e, Event::ShowHover(_)));
    host.run_command("rust", "rename", snapshot());
    let event = wait_for(&mut host, "why", |e| matches!(e, Event::ShowRename(_)));
    let Event::ShowRename(info) = event else {
        unreachable!()
    };
    assert_eq!(
        info,
        Err("Can't rename this: No references found at position".into())
    );
}

#[test]
fn references_come_for_the_peek_view() {
    use merlin_core::lang::Location;
    let mut host = started();
    host.run_command("rust", "references", on_v());
    let event = wait_on_v(&mut host, "references", |e| {
        matches!(e, Event::ShowReferences(_))
    });
    let Event::ShowReferences(places) = event else {
        unreachable!()
    };
    let at = |path: &str, line, a, b| Location {
        path: path.into(),
        start: (line, a),
        end: (line, b),
    };
    assert_eq!(
        places,
        [
            at("src/main.rs", 1, 8, 9),
            at("src/main.rs", 2, 4, 5),
            at("src/lib.rs", 0, 0, 1),
        ]
    );
}
