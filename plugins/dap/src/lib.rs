//! The Debugger: speaks the Debug Adapter Protocol to whichever debugger a
//! plugin's manifest declares, as the lsp plugin does for language
//! servers, and reports the run to the editor.
//!
//! The editor runs `debug.start` here when the user starts a run, and the
//! plugin reads what to start with `debug-start`. It starts the debugger
//! through the process API, framed with `Content-Length` as DAP is, and
//! turns the debugger's events into the editor's: its state, threads,
//! stack, variables, breakpoints and output. The toolbar's buttons and
//! keys come back as commands.

use std::collections::HashMap;

use merlin_plugin::prelude::*;
use serde_json::{Value, json};

/// What a request was for, to know what to do with its answer.
#[derive(Clone, Debug)]
enum Asked {
    Initialize,
    Launch,
    Breakpoints { ids: Vec<u32> },
    Threads,
    Stack { thread: u32 },
    Scopes,
    Variables { parent: u32 },
    Evaluate,
    Disconnect,
    Other,
}

/// A run: the debugger's process and what's known of it.
#[derive(Default)]
struct Run {
    process: u32,
    start: Option<DebugLaunch>,
    seq: i64,
    asked: HashMap<i64, Asked>,
    caps: Value,
    /// The thread that stopped, and the frame shown.
    stopped: Option<u32>,
    frame: Option<u32>,
    /// Stop filters the debugger offers: id, label, and whether on.
    filters: Vec<(String, String, bool)>,
    /// Files breakpoints were sent for, so emptied ones are cleared.
    files: Vec<String>,
    exit: Option<i32>,
    ended: bool,
}

#[derive(Default)]
struct Debugger {
    run: Option<Run>,
}

impl Run {
    fn send(&mut self, command: &str, arguments: Value, asked: Asked) {
        self.seq += 1;
        let message = json!({
            "seq": self.seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        });
        self.asked.insert(self.seq, asked);
        let _ = host::send(self.process, &message.to_string());
    }

    fn respond(&mut self, request: &Value, success: bool) {
        self.seq += 1;
        let message = json!({
            "seq": self.seq,
            "type": "response",
            "request_seq": request["seq"],
            "command": request["command"],
            "success": success,
        });
        let _ = host::send(self.process, &message.to_string());
    }

    fn thread(&self) -> u32 {
        self.stopped.unwrap_or(1)
    }

    /// Sends every breakpoint, file by file, as DAP sets them.
    fn send_breakpoints(&mut self) {
        let all = host::breakpoints().unwrap_or_default();
        let mut files: Vec<String> = all.iter().map(|b| b.path.clone()).collect();
        files.sort();
        files.dedup();
        let mut sent = files.clone();
        for old in &self.files {
            if !files.contains(old) {
                sent.push(old.clone());
            }
        }
        for path in sent {
            let here: Vec<&Breakpoint> = all
                .iter()
                .filter(|b| b.path == path && b.enabled)
                .collect();
            let list: Vec<Value> = here
                .iter()
                .map(|b| {
                    let mut v = json!({ "line": b.line + 1 });
                    if !b.condition.is_empty() {
                        v["condition"] = json!(b.condition);
                    }
                    if !b.hit_condition.is_empty() {
                        v["hitCondition"] = json!(b.hit_condition);
                    }
                    if !b.log_message.is_empty() {
                        v["logMessage"] = json!(b.log_message);
                    }
                    v
                })
                .collect();
            let ids = here.iter().map(|b| b.id).collect();
            self.send(
                "setBreakpoints",
                json!({ "source": { "path": path }, "breakpoints": list }),
                Asked::Breakpoints { ids },
            );
        }
        self.files = files;
    }

    fn send_filters(&mut self) {
        let on: Vec<&str> = self
            .filters
            .iter()
            .filter(|(_, _, on)| *on)
            .map(|(id, _, _)| id.as_str())
            .collect();
        let filters = json!(on);
        self.send(
            "setExceptionBreakpoints",
            json!({ "filters": filters }),
            Asked::Other,
        );
        let list: Vec<StopFilter> = self
            .filters
            .iter()
            .map(|(id, label, on)| StopFilter {
                id: id.clone(),
                label: label.clone(),
                on: *on,
            })
            .collect();
        let _ = host::set_stop_filters(&list);
    }

    fn end(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        let _ = host::set_debug_state(&DebugState::Ended(self.exit));
    }
}

/// Whether a frame is in a library rather than the project: no source, a
/// source outside the project, or one the debugger says to play down.
fn is_library(frame: &Value, root: &str) -> bool {
    let path = frame["source"]["path"].as_str().unwrap_or_default();
    let hint = frame["presentationHint"].as_str().unwrap_or_default();
    let source_hint = frame["source"]["presentationHint"].as_str().unwrap_or_default();
    path.is_empty()
        || !path.starts_with(root)
        || hint == "subtle"
        || source_hint == "deemphasize"
}

impl Debugger {
    fn start(&mut self) {
        let Some(start) = host::debug_start().ok().flatten() else {
            return;
        };
        if let Some(old) = self.run.take()
            && !old.ended
        {
            host::kill(old.process);
        }
        let Some((program, args)) = start.command.split_first() else {
            let _ = host::debug_output(OutputKind::Stderr, "This debugger has no command to start it.\n");
            let _ = host::set_debug_state(&DebugState::Ended(None));
            return;
        };
        let _ = host::set_debug_state(&DebugState::Starting);
        let process = match host::spawn(program, args, &start.env, Framing::ContentLength) {
            Ok(p) => p,
            Err(e) => {
                let _ = host::debug_output(
                    OutputKind::Stderr,
                    &format!("Couldn't start {program}: {e}\n"),
                );
                let _ = host::set_debug_state(&DebugState::Ended(None));
                return;
            }
        };
        let mut run = Run {
            process,
            ..Default::default()
        };
        run.send(
            "initialize",
            json!({
                "clientID": "merlin",
                "clientName": "merlin",
                "adapterID": start.debugger,
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path",
                "supportsVariableType": true,
                "supportsRunInTerminalRequest": false,
                "locale": "en",
            }),
            Asked::Initialize,
        );
        run.start = Some(start);
        self.run = Some(run);
    }

    fn on_message(&mut self, message: Value) {
        let Some(run) = &mut self.run else {
            return;
        };
        match message["type"].as_str() {
            Some("response") => {
                let seq = message["request_seq"].as_i64().unwrap_or_default();
                let asked = run.asked.remove(&seq).unwrap_or(Asked::Other);
                self.on_response(asked, &message);
            }
            Some("event") => self.on_event(&message),
            Some("request") => {
                // Nothing the editor can do for the debugger, such as
                // starting a terminal: it's told no.
                run.respond(&message, false);
            }
            _ => {}
        }
    }

    fn on_response(&mut self, asked: Asked, message: &Value) {
        let Some(run) = &mut self.run else {
            return;
        };
        let ok = message["success"].as_bool().unwrap_or(false);
        let body = &message["body"];
        let error = || {
            message["body"]["error"]["format"]
                .as_str()
                .or(message["message"].as_str())
                .unwrap_or("it failed")
                .to_string()
        };
        match asked {
            Asked::Initialize => {
                if !ok {
                    let _ = host::debug_output(
                        OutputKind::Stderr,
                        &format!("The debugger didn't start: {}\n", error()),
                    );
                    host::kill(run.process);
                    return run.end();
                }
                run.caps = body.clone();
                let chosen = run
                    .start
                    .as_ref()
                    .map(|s| s.stop_on.clone())
                    .unwrap_or_default();
                run.filters = body["exceptionBreakpointFilters"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|f| {
                                let id = f["filter"].as_str()?.to_string();
                                let label = f["label"].as_str().unwrap_or(&id).to_string();
                                let on = chosen.contains(&id)
                                    || (chosen.is_empty() && f["default"].as_bool() == Some(true));
                                Some((id, label, on))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let launch = run
                    .start
                    .as_ref()
                    .and_then(|s| serde_json::from_str::<Value>(&s.launch).ok())
                    .unwrap_or_else(|| json!({}));
                let attach = launch.get("pid").is_some();
                run.send(
                    if attach { "attach" } else { "launch" },
                    launch,
                    Asked::Launch,
                );
            }
            Asked::Launch => {
                if !ok {
                    let _ = host::debug_output(
                        OutputKind::Stderr,
                        &format!("The program didn't start: {}\n", error()),
                    );
                    run.send("disconnect", json!({ "terminateDebuggee": true }), Asked::Disconnect);
                    return run.end();
                }
                let _ = host::set_debug_state(&DebugState::Running);
            }
            Asked::Breakpoints { ids } => {
                let list = body["breakpoints"].as_array().cloned().unwrap_or_default();
                let states: Vec<BreakpointState> = ids
                    .iter()
                    .zip(list.iter())
                    .map(|(id, b)| BreakpointState {
                        id: *id,
                        set: b["verified"].as_bool().unwrap_or(false),
                        line: b["line"].as_u64().map_or(0, |l| l.saturating_sub(1) as u32),
                        message: b["message"].as_str().unwrap_or_default().to_string(),
                    })
                    .collect();
                let _ = host::set_breakpoint_states(&states);
            }
            Asked::Threads => {
                let stopped = run.stopped;
                let all = run.caps["allThreadsStopped"].as_bool().unwrap_or(true);
                let threads: Vec<DebugThread> = body["threads"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|t| {
                                let id = t["id"].as_u64()? as u32;
                                Some(DebugThread {
                                    id,
                                    name: t["name"].as_str().unwrap_or("thread").to_string(),
                                    paused: stopped.is_some() && (all || stopped == Some(id)),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = host::set_threads(&threads);
            }
            Asked::Stack { thread } => {
                let root = run.start.as_ref().map(|s| s.cwd.clone()).unwrap_or_default();
                let list = body["stackFrames"].as_array().cloned().unwrap_or_default();
                let frames: Vec<StackFrame> = list
                    .iter()
                    .filter_map(|f| {
                        Some(StackFrame {
                            id: f["id"].as_u64()? as u32,
                            name: f["name"].as_str().unwrap_or("?").to_string(),
                            path: f["source"]["path"].as_str().unwrap_or_default().to_string(),
                            line: f["line"].as_u64().unwrap_or(1).saturating_sub(1) as u32,
                            column: f["column"].as_u64().unwrap_or(1).saturating_sub(1) as u32,
                            library: is_library(f, &root),
                        })
                    })
                    .collect();
                // The frame shown is the first in the project, as the
                // editor shows it.
                let shown = frames.iter().find(|f| !f.library).or(frames.first()).map(|f| f.id);
                let _ = host::set_stack(thread, &frames);
                if run.stopped == Some(thread)
                    && let Some(frame) = shown
                {
                    run.frame = Some(frame);
                    run.send("scopes", json!({ "frameId": frame }), Asked::Scopes);
                }
            }
            Asked::Scopes => {
                let scopes: Vec<DebugVariable> = body["scopes"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|s| DebugVariable {
                                name: s["name"].as_str().unwrap_or("Scope").to_string(),
                                value: String::new(),
                                type_name: String::new(),
                                children: s["variablesReference"].as_u64().unwrap_or(0) as u32,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = host::set_variables(0, &scopes);
                // The first scope, the locals, opens at once.
                if let Some(first) = scopes.first().filter(|s| s.children != 0) {
                    let parent = first.children;
                    run.send(
                        "variables",
                        json!({ "variablesReference": parent }),
                        Asked::Variables { parent },
                    );
                }
            }
            Asked::Variables { parent } => {
                let list: Vec<DebugVariable> = body["variables"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|v| DebugVariable {
                                name: v["name"].as_str().unwrap_or("?").to_string(),
                                value: v["value"].as_str().unwrap_or_default().to_string(),
                                type_name: v["type"].as_str().unwrap_or_default().to_string(),
                                children: v["variablesReference"].as_u64().unwrap_or(0) as u32,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = host::set_variables(parent, &list);
            }
            Asked::Evaluate => {
                let text = if ok {
                    body["result"].as_str().unwrap_or_default().to_string()
                } else {
                    error()
                };
                let kind = if ok { OutputKind::Stdout } else { OutputKind::Stderr };
                let _ = host::debug_output(kind, &format!("{text}\n"));
            }
            Asked::Disconnect => {
                host::kill(run.process);
                run.end();
            }
            Asked::Other => {}
        }
    }

    fn on_event(&mut self, message: &Value) {
        let Some(run) = &mut self.run else {
            return;
        };
        let body = &message["body"];
        match message["event"].as_str().unwrap_or_default() {
            "initialized" => {
                run.send_breakpoints();
                if !run.filters.is_empty() {
                    run.send_filters();
                }
                if run.caps["supportsConfigurationDoneRequest"].as_bool() == Some(true) {
                    run.send("configurationDone", json!({}), Asked::Other);
                }
            }
            "stopped" => {
                let thread = body["threadId"].as_u64().map_or(run.thread(), |t| t as u32);
                run.stopped = Some(thread);
                run.frame = None;
                let reason = body["reason"].as_str().unwrap_or("pause").to_string();
                let description = body["text"]
                    .as_str()
                    .or(body["description"].as_str())
                    .unwrap_or_default()
                    .to_string();
                let _ = host::set_debug_state(&DebugState::Paused(DebugPause {
                    reason,
                    description,
                    thread,
                }));
                run.send("threads", json!({}), Asked::Threads);
                run.send(
                    "stackTrace",
                    json!({ "threadId": thread, "levels": 60 }),
                    Asked::Stack { thread },
                );
            }
            "continued" => {
                run.stopped = None;
                run.frame = None;
                let _ = host::set_debug_state(&DebugState::Running);
            }
            "exited" => {
                run.exit = body["exitCode"].as_i64().map(|c| c as i32);
            }
            "terminated" => {
                run.send("disconnect", json!({}), Asked::Disconnect);
            }
            "output" => {
                let text = body["output"].as_str().unwrap_or_default();
                let kind = match body["category"].as_str().unwrap_or("console") {
                    "stdout" => OutputKind::Stdout,
                    "stderr" => OutputKind::Stderr,
                    "important" => OutputKind::Important,
                    // Telemetry isn't for people.
                    "telemetry" => return,
                    _ => OutputKind::Console,
                };
                let _ = host::debug_output(kind, text);
            }
            "thread" => {
                if run.stopped.is_some() {
                    run.send("threads", json!({}), Asked::Threads);
                }
            }
            "breakpoint" => {
                // Easier to send them all again than to match this one.
                run.send_breakpoints();
            }
            _ => {}
        }
    }

    fn step(&mut self, command: &str) {
        let Some(run) = &mut self.run else {
            return;
        };
        let thread = run.thread();
        run.send(command, json!({ "threadId": thread }), Asked::Other);
        if command != "pause" {
            // The debugger says `continued` only sometimes.
            run.stopped = None;
            let _ = host::set_debug_state(&DebugState::Running);
        }
    }
}

impl Plugin for Debugger {
    fn run_command(&mut self, id: &str) {
        let id = id.strip_prefix("dap.").unwrap_or(id);
        if id == "debug.start" {
            return self.start();
        }
        let Some(run) = &mut self.run else {
            return;
        };
        match id {
            "debug.continue" => self.step("continue"),
            "debug.pause" => self.step("pause"),
            "debug.step-over" => self.step("next"),
            "debug.step-into" => self.step("stepIn"),
            "debug.step-out" => self.step("stepOut"),
            "debug.stop" => {
                run.send(
                    "disconnect",
                    json!({ "terminateDebuggee": true }),
                    Asked::Disconnect,
                );
            }
            "debug.breakpoints" => run.send_breakpoints(),
            _ => {
                if let Some(r) = id.strip_prefix("debug.expand:").and_then(|r| r.parse::<u32>().ok()) {
                    run.send(
                        "variables",
                        json!({ "variablesReference": r }),
                        Asked::Variables { parent: r },
                    );
                } else if let Some(f) = id.strip_prefix("debug.frame:").and_then(|f| f.parse::<u32>().ok()) {
                    run.frame = Some(f);
                    run.send("scopes", json!({ "frameId": f }), Asked::Scopes);
                } else if let Some(f) = id.strip_prefix("debug.stop-on:") {
                    if let Some(filter) = run.filters.iter_mut().find(|x| x.0 == f) {
                        filter.2 = !filter.2;
                    }
                    run.send_filters();
                } else if let Some(t) = id.strip_prefix("debug.pause-thread:").and_then(|t| t.parse::<u32>().ok()) {
                    run.send("pause", json!({ "threadId": t }), Asked::Other);
                }
            }
        }
    }

    fn submit(&mut self, input: &str, text: &str) {
        if input != "debug-console" {
            return;
        }
        let Some(run) = &mut self.run else {
            return;
        };
        let mut args = json!({ "expression": text, "context": "repl" });
        if let Some(frame) = run.frame {
            args["frameId"] = json!(frame);
        }
        run.send("evaluate", args, Asked::Evaluate);
    }

    fn process(&mut self, process: u32, event: ProcessEvent) {
        if self.run.as_ref().map(|r| r.process) != Some(process) {
            return;
        }
        match event {
            ProcessEvent::Message(text) => {
                if let Ok(message) = serde_json::from_str(&text) {
                    self.on_message(message);
                }
            }
            ProcessEvent::Stderr(text) => {
                let _ = host::debug_output(OutputKind::Console, &format!("{text}\n"));
            }
            ProcessEvent::Exit(_) => {
                let Some(run) = &mut self.run else {
                    return;
                };
                run.end();
            }
        }
    }
}

merlin_plugin::export!(Debugger);
