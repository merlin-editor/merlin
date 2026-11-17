//! The debugger plugin with the real gdb: disassembly around the pause,
//! the memory of an expression, and stepping by instruction. Build the dap
//! plugin first: `cd plugins/dap && cargo build --release`.

use std::path::Path;
use std::time::{Duration, Instant};

use merlin_core::{Breakpoint, DebugStart, DebugState, DebugUpdate, Event};
use merlin_plugins::{PluginHost, Snapshot};

mod common;
use common::{Folder, Host};

fn installed(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .output()
        .is_ok()
}

/// Runs `command` in the plugin, then polls until `want` picks an update.
fn wait<T>(
    host: &mut PluginHost,
    at: &Snapshot,
    command: Option<&str>,
    what: &str,
    want: &dyn Fn(&DebugUpdate) -> Option<T>,
) -> T {
    let mut events = match command {
        Some(c) => host.run_command("dap", c, at.clone()),
        None => Vec::new(),
    };
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        for e in &events {
            if let Event::Debug { update, .. } = e
                && let Some(found) = want(update)
            {
                return found;
            }
        }
        assert!(Instant::now() < until, "no {what}");
        std::thread::sleep(Duration::from_millis(30));
        events = host.poll_processes(|| at.clone()).0;
    }
}

#[test]
fn disassembly_memory_and_instruction_steps_with_gdb() {
    if !installed("gdb") || !installed("cc") {
        eprintln!("gdb or cc isn't installed; skipping");
        return;
    }
    let root = Folder::new("merlin-dap-gdb-");
    let source = "int total;\n\
                  int add(int a, int b) { return a + b; }\n\
                  int main(void) {\n\
                  \x20   int t = add(3, 4);\n\
                  \x20   total = t;\n\
                  \x20   return 0;\n\
                  }\n";
    std::fs::write(root.join("a.c"), source).unwrap();
    let built = std::process::Command::new("cc")
        .args(["-g", "-O0", "-o", "a.out", "a.c"])
        .current_dir(&*root)
        .status()
        .unwrap();
    assert!(built.success());
    let mut host = Host::new("merlin-dap-data-");
    host.load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/dap"))
        .unwrap();
    let at = Snapshot {
        root: root.to_path_buf(),
        ..Snapshot::default()
    };
    let program = root.join("a.out").display().to_string();
    host.set_debug(
        "dap",
        Some(DebugStart {
            name: "a.out".into(),
            debugger: "gdb".into(),
            command: vec![
                "gdb".into(),
                "--interpreter=dap".into(),
                "-q".into(),
                "-nx".into(),
            ],
            program: program.clone(),
            cwd: root.display().to_string(),
            launch: serde_json::json!({ "program": program, "cwd": root.to_path_buf() })
                .to_string(),
            ..Default::default()
        }),
        vec![Breakpoint {
            id: 1,
            path: root.join("a.c"),
            line: 4,
            condition: String::new(),
            hit: String::new(),
            log: String::new(),
            enabled: true,
        }],
    );
    for setting in ["debug.disassembly:on", "debug.memory:&total"] {
        host.run_command("dap", setting, at.clone());
    }
    let paused = |u: &DebugUpdate| match u {
        DebugUpdate::State(s @ DebugState::Paused { .. }) => Some(s.clone()),
        _ => None,
    };
    wait(
        &mut host,
        &at,
        Some("debug.start"),
        "the breakpoint",
        &paused,
    );
    // The instructions around `total = t`, the paused one marked, under
    // their source line.
    let lines = wait(&mut host, &at, None, "the disassembly", &|u| match u {
        DebugUpdate::Disassembly(l) if !l.is_empty() => Some(l.clone()),
        _ => None,
    });
    let current: Vec<_> = lines.iter().filter(|l| l.current).collect();
    assert_eq!(current.len(), 1, "{lines:#?}");
    assert!(
        lines
            .iter()
            .any(|l| l.source.as_ref().is_some_and(|(_, n)| *n == 4)),
        "{lines:#?}"
    );
    // total's four bytes, 0 before the line runs.
    let memory = wait(&mut host, &at, None, "the memory", &|u| match u {
        DebugUpdate::Memory(m) => Some(m.clone()),
        _ => None,
    });
    assert!(memory.error.is_empty(), "{}", memory.error);
    let (from, len) = memory.mark;
    assert_eq!(len, 4);
    assert_eq!(&memory.bytes[from..from + 4], [0, 0, 0, 0]);
    // A step over the line: total is 7 now, and those bytes changed.
    host.run_command("dap", "debug.step-over", at.clone());
    let memory = wait(
        &mut host,
        &at,
        None,
        "the memory after a step",
        &|u| match u {
            DebugUpdate::Memory(m) if !m.changed.is_empty() => Some(m.clone()),
            _ => None,
        },
    );
    let (from, _) = memory.mark;
    assert_eq!(&memory.bytes[from..from + 4], [7, 0, 0, 0]);
    assert!(memory.changed.contains(&from), "{:?}", memory.changed);
    // By instruction, a step moves the paused instruction on.
    let before = lines.iter().find(|l| l.current).unwrap().address.clone();
    host.run_command("dap", "debug.by-instruction:on", at.clone());
    let now = wait(
        &mut host,
        &at,
        Some("debug.step-over"),
        "an instruction step",
        &|u| match u {
            DebugUpdate::Disassembly(l) => l.iter().find(|l| l.current).map(|l| l.address.clone()),
            _ => None,
        },
    );
    assert_ne!(now, before);
    host.run_command("dap", "debug.stop", at.clone());
}

/// A debugger plugin that waits to start, as one does once its commands
/// are known from before: F5 wakes it with the run, and it starts at once.
#[test]
fn a_waiting_debugger_plugin_starts_the_first_run() {
    if !installed("gdb") || !installed("cc") {
        eprintln!("gdb or cc isn't installed; skipping");
        return;
    }
    let root = Folder::new("merlin-dap-wake-");
    std::fs::write(root.join("a.c"), "int main(void) {\n    return 0;\n}\n").unwrap();
    let built = std::process::Command::new("cc")
        .args(["-g", "-O0", "-o", "a.out", "a.c"])
        .current_dir(&*root)
        .status()
        .unwrap();
    assert!(built.success());
    let data = Folder::new("merlin-dap-wake-data-");
    let config = || merlin_plugins::Config {
        data_dir: data.to_path_buf(),
        ask: false,
        keyring: false,
        background: false,
    };
    let dap = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/dap");
    // Once, so its commands are kept; the second time it waits.
    PluginHost::new(config()).unwrap().load(&dap).unwrap();
    let mut host = PluginHost::new(config()).unwrap();
    host.load(&dap).unwrap();
    let at = Snapshot {
        root: root.to_path_buf(),
        ..Snapshot::default()
    };
    let program = root.join("a.out").display().to_string();
    let mut events = host.set_debug(
        "dap",
        Some(DebugStart {
            name: "a.out".into(),
            debugger: "gdb".into(),
            command: vec![
                "gdb".into(),
                "--interpreter=dap".into(),
                "-q".into(),
                "-nx".into(),
            ],
            program: program.clone(),
            cwd: root.display().to_string(),
            launch: serde_json::json!({ "program": program, "cwd": root.to_path_buf() })
                .to_string(),
            ..Default::default()
        }),
        vec![Breakpoint {
            id: 1,
            path: root.join("a.c"),
            line: 1,
            condition: String::new(),
            hit: String::new(),
            log: String::new(),
            enabled: true,
        }],
    );
    events.extend(host.run_command("dap", "debug.start", at.clone()));
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if events.iter().any(|e| {
            matches!(
                e,
                Event::Debug {
                    update: DebugUpdate::State(DebugState::Paused { .. }),
                    ..
                }
            )
        }) {
            break;
        }
        assert!(Instant::now() < until, "the first run didn't start");
        std::thread::sleep(Duration::from_millis(30));
        events = host.poll_processes(|| at.clone()).0;
    }
    host.run_command("dap", "debug.stop", at.clone());
}

/// A local's address in Variables, and a breakpoint on an instruction,
/// with the real gdb.
#[test]
fn variables_have_addresses_and_an_instruction_can_stop_it() {
    if !installed("gdb") || !installed("cc") {
        eprintln!("gdb or cc isn't installed; skipping");
        return;
    }
    let root = Folder::new("merlin-dap-addr-");
    let source =
        "int main(void) {\n    int t = 3;\n    t = t * 2;\n    t = t + 1;\n    return t - 7;\n}\n";
    std::fs::write(root.join("a.c"), source).unwrap();
    let built = std::process::Command::new("cc")
        .args(["-g", "-O0", "-o", "a.out", "a.c"])
        .current_dir(&*root)
        .status()
        .unwrap();
    assert!(built.success());
    let mut host = Host::new("merlin-dap-addr-data-");
    host.load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/dap"))
        .unwrap();
    let at = Snapshot {
        root: root.to_path_buf(),
        ..Snapshot::default()
    };
    let program = root.join("a.out").display().to_string();
    host.set_debug(
        "dap",
        Some(DebugStart {
            name: "a.out".into(),
            debugger: "gdb".into(),
            command: vec![
                "gdb".into(),
                "--interpreter=dap".into(),
                "-q".into(),
                "-nx".into(),
            ],
            program: program.clone(),
            cwd: root.display().to_string(),
            launch: serde_json::json!({ "program": program, "cwd": root.to_path_buf() })
                .to_string(),
            ..Default::default()
        }),
        vec![Breakpoint {
            id: 1,
            path: root.join("a.c"),
            line: 2,
            condition: String::new(),
            hit: String::new(),
            log: String::new(),
            enabled: true,
        }],
    );
    host.run_command("dap", "debug.disassembly:on", at.clone());
    let address = wait(
        &mut host,
        &at,
        Some("debug.start"),
        "t's address",
        &|u| match u {
            DebugUpdate::Variables { variables, .. } => variables
                .iter()
                .find(|v| v.name == "t" && !v.address.is_empty())
                .map(|v| v.address.clone()),
            _ => None,
        },
    );
    assert!(address.starts_with("0x7f"), "{address}");
    // A breakpoint a few instructions on: Continue stops there. (Asked
    // again, as it came while waiting for the address.)
    let lines = wait(
        &mut host,
        &at,
        Some("debug.disassembly:on"),
        "the disassembly",
        &|u| match u {
            DebugUpdate::Disassembly(l) if l.iter().any(|l| l.current) => Some(l.clone()),
            _ => None,
        },
    );
    let here = lines.iter().position(|l| l.current).unwrap();
    let target = lines[here + 3].address.clone();
    host.run_command(
        "dap",
        &format!("debug.instruction-breakpoints:{target}"),
        at.clone(),
    );
    let now = wait(
        &mut host,
        &at,
        Some("debug.continue"),
        "the instruction stop",
        &|u| match u {
            DebugUpdate::Disassembly(l) => l.iter().find(|l| l.current).map(|l| l.address.clone()),
            _ => None,
        },
    );
    assert_eq!(now, target);
    host.run_command("dap", "debug.stop", at.clone());
}
