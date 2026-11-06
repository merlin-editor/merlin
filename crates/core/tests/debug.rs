//! Debugging, driven by plugins: a manifest's debugger, a plugin's target,
//! breakpoints, F5 asking what to debug, the build first, the driver's
//! reports, and the end of a run.

use merlin_core::script::Session;
use merlin_core::{
    DebugState, DebugTarget, DebugUpdate, Debugger, Effect, Event, Key, KeyCode, OutputKind,
    StackFrame,
};

fn project() -> Session {
    let mut s = Session::new(
        "fn main() {\n    let n = 1;\n    println!(\"{n}\");\n}\n",
        Some("src/main.rs"),
        100,
        24,
    );
    s.send(Event::RegisterLanguages {
        plugin: "rust".into(),
        languages: vec![merlin_core::lang::Language {
            id: "rust".into(),
            name: "Rust".into(),
            extensions: vec!["rs".into()],
            line_comment: Some("//".into()),
            block_comment: None,
        }],
    });
    s.send(Event::RegisterDebuggers {
        plugin: "rust".into(),
        debuggers: vec![Debugger {
            id: "gdb".into(),
            name: "gdb".into(),
            plugin: "rust".into(),
            languages: vec!["rust".into()],
            command: vec!["gdb".into(), "--interpreter=dap".into()],
            install: "sudo apt install gdb".into(),
            launch: "{}".into(),
            driver: "dap".into(),
        }],
    });
    s.send(Event::DebuggersLocated(vec![("gdb".into(), true)]));
    s.send(Event::RegisterPlugin {
        plugin: "dap".into(),
        commands: Vec::new(),
    });
    s.send(Event::SetDebugTargets {
        plugin: "lsp".into(),
        targets: vec![DebugTarget {
            name: "demo".into(),
            language: "rust".into(),
            program: "target/debug/demo".into(),
            detail: "--bin demo".into(),
            ..Default::default()
        }],
    });
    s
}

fn key(s: &mut Session, code: KeyCode, shift: bool) {
    let mut k = Key::plain(code);
    k.shift = shift;
    s.send(Event::Key(k));
}

fn report(s: &mut Session, update: DebugUpdate) {
    s.send(Event::Debug {
        plugin: "dap".into(),
        update,
    });
}

/// The commands sent to the Debugger plugin, in order.
fn commands(s: &Session) -> Vec<String> {
    s.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Debug { command, .. } => Some(command.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn f5_asks_what_to_debug_then_starts_it_with_the_breakpoints() {
    let mut s = project();
    // F9 sets a breakpoint on the cursor's line.
    key(&mut s, KeyCode::Down, false);
    key(&mut s, KeyCode::F(9), false);
    key(&mut s, KeyCode::F(5), false);
    let palette = s.editor.window_view().palette.expect("asks what to debug");
    assert_eq!(palette.items[0].0, "demo");
    key(&mut s, KeyCode::Enter, false);
    let start = s.effects.iter().find_map(|e| match e {
        Effect::Debug {
            command,
            start: Some(start),
            breakpoints,
            ..
        } if command == "debug.start" => Some((start.clone(), breakpoints.clone())),
        _ => None,
    });
    let (start, breakpoints) = start.expect("starts the driver");
    assert_eq!(start.command, ["gdb", "--interpreter=dap"]);
    assert!(start.launch.contains("\"program\""), "{}", start.launch);
    assert_eq!(breakpoints.len(), 1);
    assert_eq!(breakpoints[0].line, 1);
    // The next F5 doesn't ask again.
    report(&mut s, DebugUpdate::State(DebugState::Ended(Some(0))));
    s.effects.clear();
    key(&mut s, KeyCode::F(5), false);
    assert!(s.editor.window_view().palette.is_none());
    assert_eq!(commands(&s), ["debug.start"]);
}

#[test]
fn a_pause_shows_the_line_and_the_keys_step() {
    let mut s = project();
    s.send(Event::Command("debug.toggle_breakpoint".into()));
    s.send(Event::Command("debug.pick".into()));
    key(&mut s, KeyCode::Enter, false);
    report(&mut s, DebugUpdate::State(DebugState::Running));
    assert!(s.editor.window_view().debug.is_some(), "the toolbar shows");
    report(
        &mut s,
        DebugUpdate::State(DebugState::Paused {
            reason: "breakpoint".into(),
            description: String::new(),
            thread: 1,
        }),
    );
    report(
        &mut s,
        DebugUpdate::Stack {
            thread: 1,
            frames: vec![StackFrame {
                id: 7,
                name: "demo::main".into(),
                path: Some("src/main.rs".into()),
                line: 2,
                column: 4,
                library: false,
            }],
        },
    );
    let view = &s.editor.text_views()[0];
    let line = view.lines.iter().find(|l| l.line == 2).unwrap();
    assert_eq!(line.paused, Some(false));
    let status: Vec<String> = s
        .editor
        .window_view()
        .status
        .into_iter()
        .map(|i| i.text)
        .collect();
    assert!(
        status.iter().any(|t| t.contains("Paused at main.rs:3")),
        "{status:?}"
    );
    s.effects.clear();
    key(&mut s, KeyCode::F(10), false);
    key(&mut s, KeyCode::F(11), true);
    key(&mut s, KeyCode::F(5), false);
    assert_eq!(
        commands(&s),
        ["debug.step-over", "debug.step-out", "debug.continue"]
    );
}

#[test]
fn the_end_says_how_it_went_and_the_toolbar_goes() {
    let mut s = project();
    s.send(Event::Command("debug.pick".into()));
    key(&mut s, KeyCode::Enter, false);
    report(&mut s, DebugUpdate::Output(OutputKind::Stdout, "hello\n".into()));
    report(&mut s, DebugUpdate::State(DebugState::Ended(Some(101))));
    assert!(s.editor.window_view().debug.is_none());
    s.send(Event::Command("view.debug_console".into()));
    let screen = s.screen();
    assert!(screen.contains("hello"), "{screen}");
    assert!(screen.contains("demo ended with code 101"), "{screen}");
}

#[test]
fn a_missing_debugger_says_how_to_install_it() {
    let mut s = project();
    s.send(Event::DebuggersLocated(vec![("gdb".into(), false)]));
    s.send(Event::Command("debug.pick".into()));
    key(&mut s, KeyCode::Enter, false);
    let toast = s.editor.window_view().toast.expect("a notice");
    assert!(toast.text.contains("needs gdb"), "{}", toast.text);
    assert!(toast.text.contains("sudo apt install gdb"), "{}", toast.text);
    assert!(commands(&s).is_empty());
}

#[test]
fn hovering_a_variable_while_paused_shows_its_value() {
    use merlin_core::{DebugVariable, MouseKind, Pointer, PopupKind};
    let mut s = project();
    s.send(Event::Command("debug.pick".into()));
    key(&mut s, KeyCode::Enter, false);
    report(
        &mut s,
        DebugUpdate::State(DebugState::Paused {
            reason: "breakpoint".into(),
            description: String::new(),
            thread: 1,
        }),
    );
    report(
        &mut s,
        DebugUpdate::Stack {
            thread: 1,
            frames: vec![StackFrame {
                id: 7,
                name: "demo::main".into(),
                path: Some("src/main.rs".into()),
                line: 2,
                column: 4,
                library: false,
            }],
        },
    );
    let var = |name: &str, value: &str, kind: &str, children| DebugVariable {
        name: name.into(),
        value: value.into(),
        kind: kind.into(),
        children,
    };
    report(
        &mut s,
        DebugUpdate::Variables {
            parent: 0,
            variables: vec![var("Locals", "", "", 5)],
        },
    );
    report(
        &mut s,
        DebugUpdate::Variables {
            parent: 5,
            variables: vec![var("n", "1", "i32", 0)],
        },
    );
    // `n` on line 2, `    let n = 1;`: the gutter, then 8 columns in.
    let (x, y) = s.find("let n").expect("the line shows");
    s.send(Event::Pointer(Pointer {
        group: 0,
        kind: MouseKind::Rest,
        row: y as f32,
        col: (x + 4) as f32,
        shift: false,
        alt: false,
        ctrl: false,
        clicks: 1,
    }));
    let popup = s.editor.window_view().popup.expect("a hover card");
    let PopupKind::Hover { lines, .. } = popup.kind else {
        panic!("not a hover");
    };
    assert_eq!(lines, ["```", "n: i32 = 1", "```"]);
}

#[test]
fn breakpoints_and_the_target_are_kept_with_the_workspace() {
    let mut s = project();
    key(&mut s, KeyCode::Down, false);
    key(&mut s, KeyCode::F(9), false);
    s.send(Event::Command("debug.pick".into()));
    key(&mut s, KeyCode::Enter, false);
    let mut ws = s.editor.workspace();
    ws.breakpoints[0].condition = "n > 0".into();
    let text = ws.to_text();
    assert!(text.contains("break 1 on src/main.rs\nbreak-if n > 0\n"), "{text}");
    assert!(text.contains("debug demo\n"), "{text}");
    let back = merlin_core::Workspace::parse(&text);
    assert_eq!(back.breakpoints[0].line, 1);
    assert_eq!(back.breakpoints[0].condition, "n > 0");
    assert_eq!(back.debug_target.as_deref(), Some("demo"));
    let mut fresh = project();
    fresh.send(Event::RestoreBreakpoints(back));
    let line = fresh.editor.text_views()[0]
        .lines
        .iter()
        .find(|l| l.line == 1)
        .and_then(|l| l.breakpoint);
    assert_eq!(line, Some(merlin_core::BreakMark::Conditional));
}
