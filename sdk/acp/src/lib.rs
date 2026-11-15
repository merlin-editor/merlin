//! Agent Client Protocol client for merlin agent plugins.
//!
//! An agent plugin names its agents ([`Agent`]: an id from its manifest's
//! `[[agents]]`, the program, and its arguments) and exports [`Acp`]:
//!
//! ```ignore
//! const OPENCODE: Agent = Agent { id: "opencode", program: "opencode", args: &["acp"] };
//! merlin_plugin::export!(Acp => Acp::new(&[OPENCODE]));
//! ```
//!
//! Each chat starts its own copy of the agent, speaks ACP to it over its
//! stdin and stdout, and turns what it says into merlin's chat: text,
//! thoughts, its plan and its steps, permission cards, files read and
//! written through the editor, and commands run in terminal tabs. See
//! <https://agentclientprotocol.com>.

use std::collections::BTreeMap;

use merlin_plugin::{
    AgentAnswer, AgentCommand, AgentContext, AgentOutcome, AgentPart, AgentRequest, AgentTool,
    Chip, Framing, LogLevel, PickItem, PlanStep, Plugin, ProcessEvent, StepState, Tone,
    ToolKind, host,
};
use serde_json::{Value, json};

/// The ACP version spoken.
const PROTOCOL: u64 = 1;

/// An agent the plugin can start: its id in the manifest's `[[agents]]`,
/// and the program and arguments that start it in ACP mode.
#[derive(Clone, Copy, Debug)]
pub struct Agent {
    pub id: &'static str,
    pub program: &'static str,
    pub args: &'static [&'static str],
}

/// The plugin: every chat's agent, by chat.
pub struct Acp {
    agents: &'static [Agent],
    chats: BTreeMap<u32, Chat>,
}

/// One chat's agent.
struct Chat {
    process: u32,
    next_id: u64,
    session: Option<String>,
    /// Messages waiting for the session: text and what goes with it.
    queued: Vec<Value>,
    /// Our requests waiting for their answers, by id.
    waiting: BTreeMap<u64, Waiting>,
    /// The agent's permission requests on a card: merlin's request id to
    /// the JSON-RPC id and the options offered.
    asks: BTreeMap<u32, (Value, Vec<(String, String)>)>,
    /// Files read or written for the agent: merlin's request id to the
    /// JSON-RPC id, and the lines asked for, for a read.
    files: BTreeMap<u32, (Value, FileOp)>,
    /// Commands run for it, by merlin's run id.
    runs: BTreeMap<u32, Run>,
    /// Its steps as last said, by tool call id.
    tools: BTreeMap<String, AgentTool>,
    /// The last few lines it printed on stderr, to say why it stopped.
    stderr: Vec<String>,
    busy: bool,
    /// What it lets you choose in the chat's header: chips that open
    /// their pickers.
    settings: Vec<Setting>,
    /// What the chat has used, as the header's note says it.
    usage: String,
}

/// A choice in a chat's header: one of the session's config options
/// (`configOptions`), or, from agents without those, its model or mode.
struct Setting {
    id: String,
    title: String,
    kind: SetKind,
    value: String,
    /// Its options, by value and name.
    options: Vec<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetKind {
    Config,
    Model,
    Mode,
}

/// The session's `configOptions` that are lists to pick from.
fn config_settings(options: Option<&Value>) -> Vec<Setting> {
    fn flat(options: &[Value], out: &mut Vec<(String, String)>) {
        for o in options {
            // A group of options, under its name.
            if let Some(group) = o.get("options").and_then(Value::as_array) {
                flat(group, out);
            } else if let Some(value) = o.get("value").and_then(Value::as_str) {
                let name = o.get("name").and_then(Value::as_str).unwrap_or(value);
                out.push((value.to_string(), name.to_string()));
            }
        }
    }
    options
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|o| o.get("type").and_then(Value::as_str) == Some("select"))
        .filter_map(|o| {
            let id = o.get("id")?.as_str()?.to_string();
            let mut options = Vec::new();
            flat(o.get("options")?.as_array()?, &mut options);
            Some(Setting {
                title: o.get("name").and_then(Value::as_str).unwrap_or(&id).to_string(),
                value: o
                    .get("currentValue")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                id,
                kind: SetKind::Config,
                options,
            })
        })
        .collect()
}

/// An older agent's `models` and `modes`, as settings.
fn model_and_mode(result: &Value) -> Vec<Setting> {
    let list = |v: Option<&Value>, id: &str| -> Vec<(String, String)> {
        v.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let id = m.get(id)?.as_str()?.to_string();
                let name = m.get("name").and_then(Value::as_str).unwrap_or(&id).to_string();
                Some((id, name))
            })
            .collect()
    };
    let mut out = Vec::new();
    for (key, kind, title, all, id, current) in [
        ("models", SetKind::Model, "Model", "availableModels", "modelId", "currentModelId"),
        ("modes", SetKind::Mode, "Mode", "availableModes", "id", "currentModeId"),
    ] {
        let Some(v) = result.get(key) else { continue };
        out.push(Setting {
            id: key.into(),
            title: title.into(),
            kind,
            value: v
                .get(current)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            options: list(v.get(all), id),
        });
    }
    out
}

/// A file read, with the lines asked for (from, how many), or a write.
enum FileOp {
    Read(Option<(usize, Option<usize>)>),
    Write,
}

enum Waiting {
    Initialize,
    NewSession,
    Prompt,
    /// A setting changed: which, and what it was before.
    Set(String, String),
}

#[derive(Default)]
struct Run {
    output: String,
    /// Its exit code, once it ended; `Some(None)` for a signal.
    exit: Option<Option<i32>>,
    /// `terminal/wait_for_exit` requests waiting for it.
    waiters: Vec<Value>,
}

impl Acp {
    pub fn new(agents: &'static [Agent]) -> Self {
        Self {
            agents,
            chats: BTreeMap::new(),
        }
    }

    /// The chat whose agent is `process`.
    fn chat_of(&self, process: u32) -> Option<u32> {
        self.chats
            .iter()
            .find(|(_, c)| c.process == process)
            .map(|(id, _)| *id)
    }

    fn send(&mut self, chat: u32, message: Value) {
        if let Some(c) = self.chats.get(&chat)
            && let Err(e) = host::send(c.process, &message.to_string())
        {
            host::log(LogLevel::Warn, &format!("couldn't send to the agent: {e}"));
        }
    }

    fn request(&mut self, chat: u32, method: &str, params: Value, waiting: Waiting) {
        let Some(c) = self.chats.get_mut(&chat) else {
            return;
        };
        let id = c.next_id;
        c.next_id += 1;
        c.waiting.insert(id, waiting);
        self.send(
            chat,
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        );
    }

    fn reply(&mut self, chat: u32, id: Value, result: Value) {
        self.send(chat, json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn reply_error(&mut self, chat: u32, id: Value, message: &str) {
        self.send(
            chat,
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": message}}),
        );
    }

    /// Starts the agent for a chat, and says hello.
    fn start(&mut self, chat: u32, agent: &str) -> Result<(), String> {
        let spec = self
            .agents
            .iter()
            .find(|a| a.id == agent)
            .ok_or_else(|| format!("this plugin has no agent {agent:?}"))?;
        let args: Vec<String> = spec.args.iter().map(|a| a.to_string()).collect();
        let process = host::spawn(spec.program, &args, &[], Framing::Lines)?;
        self.chats.insert(
            chat,
            Chat {
                process,
                next_id: 0,
                session: None,
                queued: Vec::new(),
                waiting: BTreeMap::new(),
                asks: BTreeMap::new(),
                files: BTreeMap::new(),
                runs: BTreeMap::new(),
                tools: BTreeMap::new(),
                stderr: Vec::new(),
                busy: false,
                settings: Vec::new(),
                usage: String::new(),
            },
        );
        self.request(
            chat,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL,
                "clientCapabilities": {
                    "fs": {"readTextFile": true, "writeTextFile": true},
                    "terminal": true,
                },
                "clientInfo": {"name": "merlin", "title": "merlin", "version": env!("CARGO_PKG_VERSION")},
            }),
            Waiting::Initialize,
        );
        Ok(())
    }

    /// Sends the messages waiting, once there's a session.
    fn flush(&mut self, chat: u32) {
        let Some(c) = self.chats.get_mut(&chat) else {
            return;
        };
        let Some(session) = c.session.clone() else {
            return;
        };
        for prompt in std::mem::take(&mut c.queued) {
            self.request(
                chat,
                "session/prompt",
                json!({"sessionId": session, "prompt": prompt}),
                Waiting::Prompt,
            );
        }
    }

    /// The agent's turn is over.
    fn done(&mut self, chat: u32, outcome: AgentOutcome, message: Option<&str>) {
        if let Some(c) = self.chats.get_mut(&chat) {
            c.busy = false;
        }
        // The whole of it goes to the plugin's Output, for Open Output.
        if outcome == AgentOutcome::Failed {
            host::log(LogLevel::Error, message.unwrap_or("the agent stopped"));
        }
        host::agent_done(chat, outcome, message);
    }

    fn message(&mut self, chat: u32, text: &str) {
        let Ok(msg) = serde_json::from_str::<Value>(text) else {
            host::log(LogLevel::Debug, &format!("not JSON from the agent: {text}"));
            return;
        };
        let id = msg.get("id").cloned();
        match (msg.get("method").and_then(Value::as_str), id) {
            // A request from the agent.
            (Some(method), Some(id)) => {
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                self.agent_request(chat, method, id, &params);
            }
            // A notification.
            (Some(method), None) => {
                if method == "session/update"
                    && let Some(update) = msg.get("params").and_then(|p| p.get("update"))
                {
                    self.update(chat, update);
                }
            }
            // An answer to ours.
            (None, Some(id)) => {
                let Some(n) = id.as_u64() else { return };
                let waiting = self.chats.get_mut(&chat).and_then(|c| c.waiting.remove(&n));
                let error = msg
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let result = msg.get("result").cloned().unwrap_or(Value::Null);
                match (waiting, error) {
                    (Some(Waiting::Initialize), None) => {
                        let root = host::workspace_root();
                        self.request(
                            chat,
                            "session/new",
                            json!({"cwd": root, "mcpServers": []}),
                            Waiting::NewSession,
                        );
                    }
                    (Some(Waiting::NewSession), None) => {
                        if let Some(c) = self.chats.get_mut(&chat) {
                            c.session = result
                                .get("sessionId")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            let config = config_settings(result.get("configOptions"));
                            c.settings = if config.is_empty() {
                                model_and_mode(&result)
                            } else {
                                config
                            };
                        }
                        self.header(chat);
                        self.flush(chat);
                    }
                    (Some(Waiting::Set(..)), None) => {
                        // Changing one can change the others.
                        let config = config_settings(result.get("configOptions"));
                        if let (false, Some(c)) = (config.is_empty(), self.chats.get_mut(&chat)) {
                            c.settings = config;
                            self.header(chat);
                        }
                    }
                    // Not changed after all: back to what was on.
                    (Some(Waiting::Set(id, was)), Some(error)) => {
                        host::log(LogLevel::Warn, &format!("couldn't change {id}: {error}"));
                        if let Some(set) = self
                            .chats
                            .get_mut(&chat)
                            .and_then(|c| c.settings.iter_mut().find(|s| s.id == id))
                        {
                            set.value = was;
                        }
                        self.header(chat);
                    }
                    (Some(Waiting::Prompt), None) => {
                        if let Some(usage) = result.get("usage") {
                            self.used(chat, usage);
                        }
                        let stop = result.get("stopReason").and_then(Value::as_str);
                        match stop {
                            Some("cancelled") => self.done(chat, AgentOutcome::Cancelled, None),
                            Some("refusal") => {
                                self.done(chat, AgentOutcome::Failed, Some("it refused"))
                            }
                            Some("max_tokens") | Some("max_turn_requests") => self.done(
                                chat,
                                AgentOutcome::Failed,
                                Some("it reached its limit for one turn"),
                            ),
                            _ => self.done(chat, AgentOutcome::Finished, None),
                        }
                    }
                    (Some(_), Some(error)) => self.done(chat, AgentOutcome::Failed, Some(&error)),
                    (None, _) => {}
                }
            }
            (None, None) => {}
        }
    }

    /// What the agent says as it works.
    fn update(&mut self, chat: u32, update: &Value) {
        let say = |part: AgentPart| {
            let _ = host::agent_say(chat, &part);
        };
        let text_of = |u: &Value| {
            u.get("content")
                .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|c| c.get("text"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("agent_message_chunk") => {
                if let Some(t) = text_of(update) {
                    say(AgentPart::Text(t));
                }
            }
            Some("agent_thought_chunk") => {
                if let Some(t) = text_of(update) {
                    say(AgentPart::Thought(t));
                }
            }
            Some("plan") => {
                let steps = update
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|entries| {
                        entries
                            .iter()
                            .map(|e| PlanStep {
                                text: e
                                    .get("content")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                state: state_of(e.get("status")),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                say(AgentPart::Plan(steps));
            }
            Some("tool_call") | Some("tool_call_update") => {
                if let Some(tool) = self.tool(chat, update) {
                    say(AgentPart::Tool(tool));
                }
            }
            Some("available_commands_update") => {
                let commands: Vec<AgentCommand> = update
                    .get("availableCommands")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                Some(AgentCommand {
                                    name: c.get("name")?.as_str()?.to_string(),
                                    description: c
                                        .get("description")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = host::agent_commands(chat, &commands);
            }
            Some("current_mode_update") => {
                let mode = update.get("currentModeId").and_then(Value::as_str);
                if let (Some(mode), Some(set)) = (
                    mode,
                    self.chats
                        .get_mut(&chat)
                        .and_then(|c| c.settings.iter_mut().find(|s| s.kind == SetKind::Mode)),
                ) {
                    set.value = mode.to_string();
                }
                self.header(chat);
            }
            Some("config_option_update") => {
                let config = config_settings(update.get("configOptions"));
                if let Some(c) = self.chats.get_mut(&chat) {
                    c.settings = config;
                }
                self.header(chat);
            }
            Some("usage_update") => self.used(chat, update),
            _ => {}
        }
    }

    /// The chat's header: what it lets you choose, such as its model,
    /// each a chip that opens its picker; and what it has used.
    fn header(&mut self, chat: u32) {
        let Some(c) = self.chats.get(&chat) else {
            return;
        };
        let chips: Vec<Chip> = c
            .settings
            .iter()
            .enumerate()
            .filter(|(_, s)| s.options.len() > 1)
            .map(|(n, s)| {
                let name = s
                    .options
                    .iter()
                    .find(|(v, _)| *v == s.value)
                    .map_or(s.value.as_str(), |(_, name)| name.as_str());
                Chip {
                    id: format!("acp:set:{chat}:{n}"),
                    label: format!("{name} ▾"),
                    tone: Tone::Normal,
                }
            })
            .collect();
        let _ = host::agent_header(chat, &chips, &c.usage);
    }

    /// What the chat has used, from a `usage_update` or a turn's `usage`:
    /// "12.4k tokens · $0.03".
    fn used(&mut self, chat: u32, usage: &Value) {
        let tokens = ["used", "totalTokens", "total_tokens"]
            .iter()
            .find_map(|k| usage.get(*k).and_then(Value::as_u64));
        let cost = usage.get("cost").and_then(|c| {
            let amount = c.get("amount").and_then(Value::as_f64)?;
            let currency = c.get("currency").and_then(Value::as_str).unwrap_or("USD");
            Some(match currency {
                "USD" => format!("${amount:.2}"),
                other => format!("{amount:.2} {other}"),
            })
        });
        let mut parts = Vec::new();
        if let Some(t) = tokens.filter(|t| *t > 0) {
            parts.push(format!("{} tokens", short_count(t)));
        }
        parts.extend(cost);
        if parts.is_empty() {
            return;
        }
        if let Some(c) = self.chats.get_mut(&chat) {
            c.usage = parts.join(" · ");
        }
        self.header(chat);
    }

    /// A step, as it stands after `update`.
    fn tool(&mut self, chat: u32, update: &Value) -> Option<AgentTool> {
        let id = update.get("toolCallId")?.as_str()?.to_string();
        let root = host::workspace_root();
        let c = self.chats.get_mut(&chat)?;
        let tool = c.tools.entry(id.clone()).or_insert_with(|| AgentTool {
            id: id.clone(),
            kind: ToolKind::Other,
            title: String::new(),
            state: StepState::Pending,
            added: 0,
            removed: 0,
            run: None,
        });
        if let Some(kind) = update.get("kind").and_then(Value::as_str) {
            tool.kind = match kind {
                "read" => ToolKind::Read,
                "edit" | "delete" | "move" => ToolKind::Edit,
                "search" | "fetch" => ToolKind::Search,
                "execute" => ToolKind::Run,
                _ => ToolKind::Other,
            };
        }
        if update.get("status").is_some() {
            tool.state = state_of(update.get("status"));
        }
        // A file's path is the best name for a step about one.
        let path = update
            .get("locations")
            .and_then(Value::as_array)
            .and_then(|l| l.first())
            .and_then(|l| l.get("path"))
            .and_then(Value::as_str)
            .map(|p| shown(&root, p));
        if let Some(p) = path {
            tool.title = p;
        } else if let Some(t) = update.get("title").and_then(Value::as_str)
            && (tool.title.is_empty() || tool.kind != ToolKind::Edit)
        {
            tool.title = t.to_string();
        }
        for item in update
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match item.get("type").and_then(Value::as_str) {
                Some("diff") => {
                    let old = item.get("oldText").and_then(Value::as_str).unwrap_or("");
                    let new = item.get("newText").and_then(Value::as_str).unwrap_or("");
                    let (a, r) = counts(old, new);
                    tool.added = a;
                    tool.removed = r;
                    if let Some(p) = item.get("path").and_then(Value::as_str) {
                        tool.title = shown(&root, p);
                    }
                }
                Some("terminal") => {
                    tool.run = item
                        .get("terminalId")
                        .and_then(Value::as_str)
                        .and_then(|t| t.strip_prefix('t'))
                        .and_then(|n| n.parse().ok());
                }
                _ => {}
            }
        }
        Some(tool.clone())
    }

    /// A request from the agent: permission, a file, or a terminal.
    fn agent_request(&mut self, chat: u32, method: &str, id: Value, params: &Value) {
        let text = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        match method {
            "session/request_permission" => self.permission(chat, id, params),
            "fs/read_text_file" => {
                let path = text("path");
                let lines = params.get("line").and_then(Value::as_u64).map(|l| {
                    let limit = params.get("limit").and_then(Value::as_u64);
                    (l as usize, limit.map(|n| n as usize))
                });
                let lines = lines.or_else(|| {
                    params
                        .get("limit")
                        .and_then(Value::as_u64)
                        .map(|n| (1, Some(n as usize)))
                });
                match host::agent_read(chat, &path) {
                    Ok(request) => {
                        if let Some(c) = self.chats.get_mut(&chat) {
                            c.files.insert(request, (id, FileOp::Read(lines)));
                        }
                    }
                    Err(e) => self.reply_error(chat, id, &e),
                }
            }
            "fs/write_text_file" => {
                let (path, content) = (text("path"), text("content"));
                match host::agent_write(chat, &path, &content) {
                    Ok(request) => {
                        if let Some(c) = self.chats.get_mut(&chat) {
                            c.files.insert(request, (id, FileOp::Write));
                        }
                    }
                    Err(e) => self.reply_error(chat, id, &e),
                }
            }
            "terminal/create" => {
                let mut line = quote(&text("command"));
                for arg in params
                    .get("args")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    line.push(' ');
                    line.push_str(&quote(arg));
                }
                let cwd = params.get("cwd").and_then(Value::as_str);
                match host::agent_run(chat, &line, cwd) {
                    Ok(run) => {
                        if let Some(c) = self.chats.get_mut(&chat) {
                            c.runs.insert(run, Run::default());
                        }
                        self.reply(chat, id, json!({"terminalId": format!("t{run}")}));
                    }
                    Err(e) => self.reply_error(chat, id, &e),
                }
            }
            "terminal/output" | "terminal/wait_for_exit" | "terminal/kill" | "terminal/release" => {
                let run: Option<u32> = params
                    .get("terminalId")
                    .and_then(Value::as_str)
                    .and_then(|t| t.strip_prefix('t'))
                    .and_then(|n| n.parse().ok());
                let Some(run) = run.filter(|r| {
                    self.chats.get(&chat).is_some_and(|c| c.runs.contains_key(r))
                }) else {
                    return self.reply_error(chat, id, "no such terminal");
                };
                self.terminal(chat, method, id, run);
            }
            _ => {
                self.send(
                    chat,
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("merlin doesn't know {method}")}}),
                );
            }
        }
    }

    fn terminal(&mut self, chat: u32, method: &str, id: Value, run: u32) {
        let Some(c) = self.chats.get_mut(&chat) else {
            return;
        };
        let r = c.runs.get_mut(&run).expect("checked");
        let status = |exit: Option<i32>| json!({"exitCode": exit, "signal": Value::Null});
        let answer = match method {
            "terminal/output" => Some(json!({
                // merlin hands a run's output over when it ends.
                "output": r.output,
                "truncated": false,
                "exitStatus": r.exit.map(status),
            })),
            "terminal/wait_for_exit" => match r.exit {
                Some(code) => Some(status(code)),
                None => {
                    r.waiters.push(id.clone());
                    None
                }
            },
            "terminal/kill" => {
                if r.exit.is_none() {
                    host::agent_stop(run);
                }
                Some(json!({}))
            }
            _ => {
                if r.exit.is_none() {
                    host::agent_stop(run);
                }
                c.runs.remove(&run);
                Some(json!({}))
            }
        };
        if let Some(a) = answer {
            self.reply(chat, id, a);
        }
    }

    /// A permission card for what the agent wants to do.
    fn permission(&mut self, chat: u32, id: Value, params: &Value) {
        let root = host::workspace_root();
        let call = params.get("toolCall").cloned().unwrap_or(Value::Null);
        // What's known of the step from before, under what's said now.
        let known = call
            .get("toolCallId")
            .and_then(Value::as_str)
            .and_then(|t| self.chats.get(&chat)?.tools.get(t).cloned());
        let kind = call
            .get("kind")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                known.as_ref().map(|k| match k.kind {
                    ToolKind::Edit => "edit".to_string(),
                    ToolKind::Run => "execute".to_string(),
                    ToolKind::Read => "read".to_string(),
                    _ => "other".to_string(),
                })
            })
            .unwrap_or_else(|| "other".into());
        let title = call
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| known.as_ref().map(|k| k.title.clone()))
            .unwrap_or_else(|| "do something".into());
        let diff = call
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|c| c.get("type").and_then(Value::as_str) == Some("diff"));
        let path = diff
            .and_then(|d| d.get("path"))
            .or_else(|| {
                call.get("locations")
                    .and_then(Value::as_array)
                    .and_then(|l| l.first())
                    .and_then(|l| l.get("path"))
            })
            .and_then(Value::as_str)
            .map(|p| shown(&root, p));
        let command = call
            .get("rawInput")
            .and_then(|i| i.get("command"))
            .and_then(|c| match c {
                Value::String(s) => Some(s.clone()),
                Value::Array(a) => Some(
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                _ => None,
            });
        let request = match (kind.as_str(), path, command) {
            ("edit" | "delete" | "move", Some(path), _) => {
                let text = diff.map(|d| {
                    let old = d.get("oldText").and_then(Value::as_str).unwrap_or("");
                    let new = d.get("newText").and_then(Value::as_str).unwrap_or("");
                    similar::TextDiff::from_lines(old, new)
                        .unified_diff()
                        .context_radius(0)
                        .to_string()
                });
                AgentRequest::Edit((path, text))
            }
            ("execute", _, Some(command)) => AgentRequest::Run(command),
            ("read", Some(path), _) => AgentRequest::Read(path),
            _ => AgentRequest::Other(lower_first(&title)),
        };
        let options: Vec<(String, String)> = params
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| {
                Some((
                    o.get("optionId")?.as_str()?.to_string(),
                    o.get("kind")?.as_str()?.to_string(),
                ))
            })
            .collect();
        match host::agent_ask(chat, &request) {
            Ok(ask) => {
                if let Some(c) = self.chats.get_mut(&chat) {
                    c.asks.insert(ask, (id, options));
                }
            }
            Err(e) => self.reply_error(chat, id, &e),
        }
    }
}

impl Plugin for Acp {
    fn agent_prompt(&mut self, chat: u32, agent: &str, text: &str, context: Vec<AgentContext>) {
        if !self.chats.contains_key(&chat)
            && let Err(e) = self.start(chat, agent)
        {
            host::agent_done(chat, AgentOutcome::Failed, Some(&e));
            return;
        }
        let root = host::workspace_root();
        let mut prompt = vec![json!({"type": "text", "text": text})];
        for c in context {
            let (path, lines) = match c {
                AgentContext::File(p) => (p, None),
                AgentContext::Lines((p, a, b)) => (p, Some((a, b))),
            };
            let full = absolute(&root, &path);
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            if let Some((a, b)) = lines {
                prompt.push(json!({"type": "text", "text": format!("(About lines {a} to {b} of {path}.)")}));
            }
            prompt.push(json!({"type": "resource_link", "name": name, "uri": format!("file://{full}")}));
        }
        if let Some(c) = self.chats.get_mut(&chat) {
            c.busy = true;
            c.queued.push(Value::Array(prompt));
        }
        self.flush(chat);
    }

    fn agent_cancel(&mut self, chat: u32) {
        let Some(c) = self.chats.get_mut(&chat) else {
            return;
        };
        let session = c.session.clone();
        // Every card still open is answered: cancelled.
        let asks: Vec<Value> = std::mem::take(&mut c.asks).into_values().map(|(id, _)| id).collect();
        for id in asks {
            self.reply(chat, id, json!({"outcome": {"outcome": "cancelled"}}));
        }
        if let Some(session) = session {
            self.send(
                chat,
                json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}}),
            );
        }
    }

    fn click(&mut self, button: &str) {
        let Some((chat, n)) = setting_of(button) else {
            return;
        };
        let Some(set) = self.chats.get(&chat).and_then(|c| c.settings.get(n)) else {
            return;
        };
        let items: Vec<PickItem> = set
            .options
            .iter()
            .map(|(value, name)| PickItem {
                id: value.clone(),
                label: if *value == set.value {
                    format!("{name} ✓")
                } else {
                    name.clone()
                },
            })
            .collect();
        host::show_picker(button, &set.title, &items);
    }

    fn pick(&mut self, picker: &str, item: &str) {
        let Some((chat, n)) = setting_of(picker) else {
            return;
        };
        let Some(c) = self.chats.get_mut(&chat) else {
            return;
        };
        let (Some(session), Some(set)) = (c.session.clone(), c.settings.get_mut(n)) else {
            return;
        };
        if set.value == item {
            return;
        }
        let was = std::mem::replace(&mut set.value, item.to_string());
        let id = set.id.clone();
        let (method, params) = match set.kind {
            SetKind::Config => (
                "session/set_config_option",
                json!({"sessionId": session, "configId": id, "value": item}),
            ),
            SetKind::Model => (
                "session/set_model",
                json!({"sessionId": session, "modelId": item}),
            ),
            SetKind::Mode => (
                "session/set_mode",
                json!({"sessionId": session, "modeId": item}),
            ),
        };
        self.request(chat, method, params, Waiting::Set(id, was));
        self.header(chat);
    }

    fn agent_close(&mut self, chat: u32) {
        if let Some(c) = self.chats.remove(&chat) {
            host::kill(c.process);
        }
    }

    fn agent_answer(&mut self, chat: u32, request: u32, answer: AgentAnswer) {
        let Some((id, options)) = self.chats.get_mut(&chat).and_then(|c| c.asks.remove(&request))
        else {
            return;
        };
        let wanted: &[&str] = match answer {
            AgentAnswer::AllowOnce => &["allow_once", "allow_always"],
            AgentAnswer::AlwaysAllow => &["allow_always", "allow_once"],
            AgentAnswer::Deny => &["reject_once", "reject_always"],
        };
        let picked = wanted
            .iter()
            .find_map(|k| options.iter().find(|(_, kind)| kind == k))
            .map(|(id, _)| id.clone());
        let outcome = match picked {
            Some(option) => json!({"outcome": {"outcome": "selected", "optionId": option}}),
            None => json!({"outcome": {"outcome": "cancelled"}}),
        };
        self.reply(chat, id, outcome);
    }

    fn agent_file(&mut self, chat: u32, request: u32, got: Result<String, String>) {
        let Some((id, op)) = self.chats.get_mut(&chat).and_then(|c| c.files.remove(&request))
        else {
            return;
        };
        match (got, op) {
            (Ok(_), FileOp::Write) => self.reply(chat, id, json!({})),
            (Ok(text), FileOp::Read(lines)) => {
                let text = match lines {
                    Some((from, limit)) => {
                        let picked = text.lines().skip(from.saturating_sub(1));
                        let picked: Vec<&str> = match limit {
                            Some(n) => picked.take(n).collect(),
                            None => picked.collect(),
                        };
                        picked.join("\n")
                    }
                    None => text,
                };
                self.reply(chat, id, json!({"content": text}));
            }
            (Err(e), _) => self.reply_error(chat, id, &e),
        }
    }

    fn agent_run(&mut self, chat: u32, run: u32, ended: Result<(String, Option<i32>), String>) {
        let Some(r) = self.chats.get_mut(&chat).and_then(|c| c.runs.get_mut(&run)) else {
            return;
        };
        let (output, code) = match ended {
            Ok((output, code)) => (output, code),
            Err(e) => (e, Some(1)),
        };
        r.output = output;
        r.exit = Some(code);
        let waiters = std::mem::take(&mut r.waiters);
        for id in waiters {
            self.reply(chat, id, json!({"exitCode": code, "signal": Value::Null}));
        }
    }

    fn process(&mut self, process: u32, event: ProcessEvent) {
        let Some(chat) = self.chat_of(process) else {
            return;
        };
        match event {
            ProcessEvent::Message(text) => self.message(chat, &text),
            ProcessEvent::Stderr(line) => {
                if let Some(c) = self.chats.get_mut(&chat) {
                    c.stderr.push(line);
                    if c.stderr.len() > 5 {
                        c.stderr.remove(0);
                    }
                }
            }
            ProcessEvent::Exit(code) => {
                let Some(c) = self.chats.remove(&chat) else {
                    return;
                };
                if c.busy || c.session.is_none() {
                    for line in &c.stderr {
                        host::log(LogLevel::Error, line);
                    }
                    let last = c
                        .stderr
                        .iter()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| match code {
                            Some(n) => format!("it exited with code {n}"),
                            None => "it was stopped".into(),
                        });
                    host::agent_done(chat, AgentOutcome::Failed, Some(&last));
                }
            }
        }
    }
}

/// A header chip's or picker's id, `acp:set:<chat>:<n>`: its chat and
/// which setting.
fn setting_of(id: &str) -> Option<(u32, usize)> {
    let rest = id.strip_prefix("acp:set:")?;
    let (chat, n) = rest.split_once(':')?;
    Some((chat.parse().ok()?, n.parse().ok()?))
}

/// 950, 12.4k, 1.2M.
fn short_count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

fn state_of(status: Option<&Value>) -> StepState {
    match status.and_then(Value::as_str) {
        Some("in_progress") => StepState::Running,
        Some("completed") => StepState::Done,
        Some("failed") => StepState::Failed,
        _ => StepState::Pending,
    }
}

/// Lines added and removed from `old` to `new`.
fn counts(old: &str, new: &str) -> (u32, u32) {
    let (mut a, mut r) = (0, 0);
    for change in similar::TextDiff::from_lines(old, new).iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => a += 1,
            similar::ChangeTag::Delete => r += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    (a, r)
}

/// A path as the chat shows it: from the workspace's root when inside it.
fn shown(root: &str, path: &str) -> String {
    let root = root.trim_end_matches('/');
    path.strip_prefix(root)
        .and_then(|p| p.strip_prefix('/'))
        .unwrap_or(path)
        .to_string()
}

/// A workspace path made whole, as ACP wants every path.
fn absolute(root: &str, path: &str) -> String {
    if path.starts_with('/') || path.get(1..3) == Some(":\\") {
        path.to_string()
    } else {
        format!("{}/{path}", root.trim_end_matches('/'))
    }
}

/// An argument quoted for the shell, when it needs it.
fn quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-=:,+@%".contains(c))
    {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}
