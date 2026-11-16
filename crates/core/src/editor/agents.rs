//! Agents in the chat (plugin-registry.md 12.6, `docs/mockups/Agents.html`).
//! A plugin talks to the agent; merlin owns the chat in the right side
//! bar, the permission cards, and the review of what a turn changed.
//!
//! Each agent the user opened is a tab in the right side bar, named
//! `merlin.agent:<plugin>/<id>`, with one conversation. Its panel is built
//! here from what the plugin said, and redrawn when that changes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{Editor, Effect, Side};
use crate::panel::{
    AskCard, Button, ChangedFile, ChangesBar, ChangesCard, Chip, ComposeBar, Message, Node, Panel,
    StepLine, StepState, Tone,
};

/// The prefix of an agent tab's panel name.
pub(crate) const AGENT: &str = "merlin.agent:";
/// The chat's text box.
const PROMPT: &str = "agent.prompt";

/// An agent a plugin adds, as `[[agents]]` declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSpec {
    pub plugin: String,
    pub id: String,
    pub name: String,
    pub icon: String,
    /// The program it runs, when that isn't installed.
    pub missing: Option<MissingProgram>,
    /// A view of its plugin's own that shows in place of merlin's chat,
    /// by its key (`plugin/view`).
    pub view: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingProgram {
    pub program: String,
    pub install: Option<String>,
    pub url: Option<String>,
}

impl AgentSpec {
    fn key(&self) -> String {
        format!("{}/{}", self.plugin, self.id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Edit,
    Run,
    Search,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanStep {
    pub text: String,
    pub state: StepState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentTool {
    pub id: String,
    pub kind: ToolKind,
    pub title: String,
    pub state: StepState,
    pub added: u32,
    pub removed: u32,
    pub run: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentPart {
    Text(String),
    Thought(String),
    Plan(Vec<PlanStep>),
    Tool(AgentTool),
    /// Pieces of the plugin's own, drawn in the message.
    Panel(Vec<Node>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRequest {
    Edit {
        path: String,
        diff: Option<String>,
    },
    Run(String),
    Read(String),
    Other(String),
    /// To leave its mode: the modes it can go on in, by id.
    Mode(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentAnswer {
    AllowOnce,
    AlwaysAllow,
    Deny,
    /// For a mode request: the mode picked.
    Mode(String),
}

/// How a mode of an agent goes about asking before it acts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentAsks {
    /// The mode doesn't say: merlin's own edit pill decides.
    #[default]
    Unsaid,
    ReadsOnly,
    AsksFirst,
    Edits,
    EditsAndSafeCommands,
    Never,
}

/// A mode of an agent, such as opencode's build and plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMode {
    pub id: String,
    pub name: String,
    /// A hue's name, `#rrggbb`, or empty for the accent.
    pub colour: String,
    pub asks: AgentAsks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentOutcome {
    Finished,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentContext {
    File(String),
    Lines(String, u32, u32),
}

/// What a plugin's agent said or asked, from its host calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentUpdate {
    Say(AgentPart),
    Ask {
        request: u32,
        what: AgentRequest,
    },
    Done {
        outcome: AgentOutcome,
        message: Option<String>,
    },
    Read {
        request: u32,
        path: String,
    },
    Write {
        request: u32,
        path: String,
        text: String,
    },
    Run {
        run: u32,
        command: String,
        cwd: Option<String>,
    },
    Stop {
        run: u32,
    },
    /// The chat's header: chips and a quiet line.
    Header {
        chips: Vec<Chip>,
        note: String,
    },
    /// The slash commands the agent offers: name and what it does.
    Commands(Vec<(String, String)>),
    /// The agent's modes, and the one it's in.
    Modes {
        modes: Vec<AgentMode>,
        current: String,
    },
    /// How full the context is (tokens used of its size), and a note.
    Usage {
        used: u64,
        size: u64,
        note: String,
    },
}

/// A call into the plugin behind a chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentCall {
    Prompt {
        chat: u32,
        agent: String,
        text: String,
        context: Vec<AgentContext>,
    },
    Cancel {
        chat: u32,
    },
    Close {
        chat: u32,
    },
    Answer {
        chat: u32,
        request: u32,
        answer: AgentAnswer,
    },
    File {
        chat: u32,
        request: u32,
        result: Result<String, String>,
    },
    Ran {
        chat: u32,
        run: u32,
        result: Result<(String, Option<i32>), String>,
    },
    /// The user picked one of the agent's modes.
    Mode {
        chat: u32,
        mode: String,
    },
}

/// What the agents' chats hold.
#[derive(Default)]
pub(crate) struct Agents {
    pub specs: Vec<AgentSpec>,
    pub tabs: Vec<Tab>,
    next_chat: u32,
    /// What's always allowed in this workspace: `<plugin>/<id>|edit` or
    /// `<plugin>/<id>|run:<command>`.
    pub allowed: Vec<String>,
    /// The tab whose "Allowed in This Workspace" list shows.
    pub showing_allowed: Option<String>,
    /// The agents the + menu lists, in its order.
    pub menu: Vec<String>,
    /// Writes waiting for their file's tab to open: plugin, chat, request,
    /// path and text.
    pending_writes: Vec<(String, u32, u32, PathBuf, String)>,
}

pub(crate) struct Tab {
    pub key: String,
    pub chat: u32,
    pub turns: Vec<Turn>,
    pub busy: bool,
    /// Permission cards waiting, first shown first.
    pub asks: Vec<Ask>,
    pub ended: Option<(AgentOutcome, Option<String>)>,
    /// Files changed this turn: each as it was before.
    pub changes: Vec<Change>,
    /// Paths allowed to be edited this turn, and commands allowed to run.
    allowed_paths: HashSet<String>,
    allowed_runs: Vec<String>,
    runs: Vec<RunRecord>,
    next_ask: u32,
    /// Whether the open file goes with the next message.
    pub attach: bool,
    /// When the turn's current thought started and ended, in ms.
    thought: Option<(u64, u64)>,
    /// The plugin's header, and the agent's slash commands.
    pub header: Option<(Vec<Chip>, String)>,
    pub commands: Vec<(String, String)>,
    /// The agent's modes, and the one it's in.
    pub modes: Vec<AgentMode>,
    pub mode: Option<String>,
    /// Tokens used of the context's size, and the plugin's note.
    pub usage: Option<(u64, u64, String)>,
    /// Whether the user let a mode that never asks run, in this chat.
    never_ok: bool,
    /// merlin's edit pill: edits go through without a card (or, in a
    /// read-only mode, read-only commands run without one).
    pub auto: bool,
    /// When each step was first and last heard of, by its id.
    tool_times: std::collections::HashMap<String, (u64, u64)>,
    /// Runs of steps opened, by turn and first item.
    open_folds: HashSet<(usize, usize)>,
    /// Messages sent while the agent worked, waiting their turn.
    pub queued: Vec<String>,
    /// Whether the plan above the box, and the changed files, are open.
    plan_open: bool,
    changes_open: bool,
}

pub(crate) enum Turn {
    You { text: String, chips: Vec<String> },
    Agent(Reply),
}

/// An agent's reply: what it said and did, the mode it was in (name and
/// colour), and when it started and ended, in ms.
pub(crate) struct Reply {
    items: Vec<Item>,
    mode: Option<(String, String)>,
    started: u64,
    ended: Option<u64>,
}

impl std::ops::Deref for Reply {
    type Target = Vec<Item>;
    fn deref(&self) -> &Vec<Item> {
        &self.items
    }
}

impl std::ops::DerefMut for Reply {
    fn deref_mut(&mut self) -> &mut Vec<Item> {
        &mut self.items
    }
}

pub(crate) enum Item {
    Text(String),
    /// Thinking, and how long it took in seconds once it's over.
    Thought(String, Option<u64>),
    Plan(Vec<PlanStep>),
    Tool(AgentTool),
    Panel(Vec<Node>),
}

pub(crate) struct Ask {
    /// The plugin's request, for a card from `agent-ask`.
    request: Option<u32>,
    what: AgentRequest,
    /// What merlin holds until the user answers, for a card of its own.
    held: Option<Held>,
}

enum Held {
    /// Switching to a mode that never asks: once per chat.
    Mode(String),
    Read {
        request: u32,
        path: PathBuf,
    },
    Write {
        request: u32,
        path: PathBuf,
        text: String,
    },
    Run {
        run: u32,
        command: String,
        cwd: Option<String>,
    },
}

pub(crate) struct Change {
    pub path: PathBuf,
    pub before: String,
    pub added: u32,
    pub removed: u32,
}

struct RunRecord {
    run: u32,
    term: u64,
}

/// Lines added and removed from `before` to `after`.
pub fn line_counts(before: &str, after: &str) -> (u32, u32) {
    let diff = similar::TextDiff::from_lines(before, after);
    let (mut added, mut removed) = (0, 0);
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    (added, removed)
}

/// What "Always Allow" covers for a command: its first two words, as
/// `cargo test`.
fn command_rule(command: &str) -> String {
    command
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

impl Editor {
    fn tab_index(&self, plugin: &str, chat: u32) -> Option<usize> {
        self.agents
            .tabs
            .iter()
            .position(|t| t.chat == chat && t.key.starts_with(&format!("{plugin}/")))
    }

    /// The agent whose chat shows on the right: its tab, or its view of
    /// its own.
    pub(crate) fn shown_agent(&self) -> Option<String> {
        let name = self.active_panel_name(Side::Right)?;
        if let Some(key) = name.strip_prefix(AGENT) {
            return Some(key.to_string());
        }
        self.agents
            .specs
            .iter()
            .find(|s| s.view.as_deref() == Some(name.as_str()))
            .map(AgentSpec::key)
    }

    /// An agent with a view of its own numbers its chats itself: the first
    /// word from one of them makes its tab.
    fn own_view_tab(&mut self, plugin: &str, chat: u32) -> Option<usize> {
        let key = self
            .agents
            .specs
            .iter()
            .find(|s| s.plugin == plugin && s.view.is_some())?
            .key();
        // It has one chat: what merlin opened takes the plugin's number.
        if let Some(i) = self.agents.tabs.iter().position(|t| t.key == key) {
            self.agents.tabs[i].chat = chat;
            return Some(i);
        }
        self.agents.tabs.push(Tab {
            key,
            chat,
            turns: Vec::new(),
            busy: true,
            asks: Vec::new(),
            ended: None,
            changes: Vec::new(),
            allowed_paths: HashSet::new(),
            allowed_runs: Vec::new(),
            runs: Vec::new(),
            next_ask: 0,
            attach: false,
            thought: None,
            header: None,
            commands: Vec::new(),
            modes: Vec::new(),
            mode: None,
            usage: None,
            never_ok: false,
            auto: false,
            tool_times: Default::default(),
            open_folds: HashSet::new(),
            queued: Vec::new(),
            plan_open: false,
            changes_open: false,
        });
        Some(self.agents.tabs.len() - 1)
    }

    fn spec(&self, key: &str) -> Option<&AgentSpec> {
        self.agents.specs.iter().find(|s| s.key() == key)
    }

    fn agent_name(&self, key: &str) -> String {
        self.spec(key)
            .map_or_else(|| key.to_string(), |s| s.name.clone())
    }

    /// A plugin's agents, as its manifest declares them. The first agent
    /// any plugin adds gets a tab of its own; the rest wait in +.
    pub(crate) fn register_agents(&mut self, plugin: &str, agents: Vec<AgentSpec>) {
        self.agents.specs.retain(|s| s.plugin != plugin);
        let none_open = self.agents.tabs.is_empty();
        let first = agents.first().map(AgentSpec::key);
        self.agents.specs.extend(agents);
        // Tabs for agents that are gone close.
        let gone: Vec<String> = self
            .agents
            .tabs
            .iter()
            .filter(|t| t.key.starts_with(&format!("{plugin}/")) && self.spec(&t.key).is_none())
            .map(|t| t.key.clone())
            .collect();
        for key in gone {
            self.close_agent(&key, &mut Vec::new());
        }
        if none_open && let Some(key) = first {
            self.open_agent(&key, false);
        }
        for key in self
            .agents
            .tabs
            .iter()
            .map(|t| t.key.clone())
            .collect::<Vec<_>>()
        {
            self.draw_agent(&key);
        }
    }

    /// Opens an agent's tab, or shows it if it's open.
    pub(crate) fn open_agent(&mut self, key: &str, show: bool) {
        if !self.agents.tabs.iter().any(|t| t.key == key) {
            self.agents.next_chat += 1;
            let chat = self.agents.next_chat;
            self.agents.tabs.push(Tab {
                key: key.to_string(),
                chat,
                turns: Vec::new(),
                busy: false,
                asks: Vec::new(),
                ended: None,
                changes: Vec::new(),
                allowed_paths: HashSet::new(),
                allowed_runs: Vec::new(),
                runs: Vec::new(),
                next_ask: 0,
                attach: true,
                thought: None,
                header: None,
                commands: Vec::new(),
                modes: Vec::new(),
                mode: None,
                usage: None,
                never_ok: false,
                auto: false,
                tool_times: Default::default(),
                open_folds: HashSet::new(),
                queued: Vec::new(),
                plan_open: false,
                changes_open: false,
            });
            // An agent with a view of its own shows that, not merlin's chat.
            if self.spec(key).and_then(|s| s.view.clone()).is_none() {
                let name = format!("{AGENT}{key}");
                let entry = self.plugin_mut(&name);
                entry.side = Side::Right;
                self.draw_agent(key);
            }
        }
        if show {
            let name = self
                .spec(key)
                .and_then(|s| s.view.clone())
                .unwrap_or_else(|| format!("{AGENT}{key}"));
            let bar = self.bar_mut(Side::Right);
            bar.visible = true;
            bar.active = Some(name);
            self.focus_panel(Side::Right);
        }
    }

    /// Closes an agent's tab, stopping its agent.
    fn close_agent(&mut self, key: &str, fx: &mut Vec<Effect>) {
        let Some(i) = self.agents.tabs.iter().position(|t| t.key == key) else {
            return;
        };
        let tab = self.agents.tabs.remove(i);
        if let Some(spec) = self.spec(key) {
            fx.push(Effect::Agent {
                plugin: spec.plugin.clone(),
                call: AgentCall::Close { chat: tab.chat },
            });
        }
        let name = format!("{AGENT}{key}");
        self.plugins.retain(|p| p.name != name);
        if self.bar(Side::Right).active.as_deref() == Some(name.as_str()) {
            let next = self.panel_names(Side::Right).first().cloned();
            self.bar_mut(Side::Right).active = next;
        }
    }

    /// What a plugin's agent said or asked in a chat.
    pub(crate) fn agent_update(
        &mut self,
        plugin: &str,
        chat: u32,
        update: AgentUpdate,
        at: u64,
        fx: &mut Vec<Effect>,
    ) {
        let found = self
            .tab_index(plugin, chat)
            .or_else(|| self.own_view_tab(plugin, chat));
        let Some(i) = found else {
            // A chat that's closed: answer what waits for an answer.
            let mut refuse = |call| {
                fx.push(Effect::Agent {
                    plugin: plugin.to_string(),
                    call,
                })
            };
            match update {
                AgentUpdate::Ask { request, .. } => refuse(AgentCall::Answer {
                    chat,
                    request,
                    answer: AgentAnswer::Deny,
                }),
                AgentUpdate::Read { request, .. } | AgentUpdate::Write { request, .. } => {
                    refuse(AgentCall::File {
                        chat,
                        request,
                        result: Err("the chat is closed".into()),
                    })
                }
                AgentUpdate::Run { run, .. } => refuse(AgentCall::Ran {
                    chat,
                    run,
                    result: Err("the chat is closed".into()),
                }),
                _ => {}
            }
            return;
        };
        let key = self.agents.tabs[i].key.clone();
        match update {
            AgentUpdate::Say(part) => self.agent_say(i, part, at),
            AgentUpdate::Ask { request, what } => {
                let rule = match &what {
                    AgentRequest::Edit { .. } => Some(format!("{key}|edit")),
                    AgentRequest::Run(c) => Some(format!("{key}|run:{}", command_rule(c))),
                    _ => None,
                };
                // Reading outside the workspace always asks.
                let outside = matches!(&what, AgentRequest::Read(p) if !self.in_workspace(p));
                let auto = self.auto_allows(i, &what);
                if !outside && (auto || rule.is_some_and(|r| self.agents.allowed.contains(&r))) {
                    self.allow_for_turn(i, &what);
                    fx.push(Effect::Agent {
                        plugin: plugin.to_string(),
                        call: AgentCall::Answer {
                            chat,
                            request,
                            answer: AgentAnswer::AlwaysAllow,
                        },
                    });
                } else {
                    self.push_ask(i, Some(request), what, None);
                }
            }
            AgentUpdate::Done { outcome, message } => {
                let tab = &mut self.agents.tabs[i];
                tab.busy = false;
                if let Some(Turn::Agent(reply)) = tab.turns.last_mut() {
                    reply.ended = Some(at);
                }
                tab.ended = (outcome != AgentOutcome::Finished || message.is_some())
                    .then_some((outcome, message));
                // Nothing can be answered once the turn's over.
                tab.asks.retain(|a| a.held.is_some());
                tab.allowed_paths.clear();
                tab.allowed_runs.clear();
                self.end_thought(i, at);
                // The next message waiting goes now, unless it failed.
                let tab = &mut self.agents.tabs[i];
                if outcome != AgentOutcome::Failed && !tab.queued.is_empty() {
                    let next = tab.queued.remove(0);
                    self.agent_send(&key, &next, fx);
                }
            }
            AgentUpdate::Read { request, path } => {
                let full = self.agent_path(&path);
                if self.in_workspace(&path) {
                    self.agent_read(plugin, chat, request, &full, fx);
                } else {
                    let what = AgentRequest::Read(path);
                    self.push_ask(
                        i,
                        None,
                        what,
                        Some(Held::Read {
                            request,
                            path: full,
                        }),
                    );
                }
            }
            AgentUpdate::Write {
                request,
                path,
                text,
            } => {
                let full = self.agent_path(&path);
                let shown = self.shown_path(&full);
                let edit = AgentRequest::Edit {
                    path: shown.clone(),
                    diff: None,
                };
                let allowed = self.in_workspace(&path)
                    && (self.agents.tabs[i].allowed_paths.contains(&shown)
                        || self.agents.allowed.contains(&format!("{key}|edit"))
                        || self.auto_allows(i, &edit));
                if allowed {
                    self.agent_write(plugin, chat, request, &full, text, fx);
                } else {
                    let before = self.open_text_at(&full).unwrap_or_default();
                    let diff = diff_text(&before, &text);
                    let what = AgentRequest::Edit {
                        path: shown,
                        diff: Some(diff),
                    };
                    let held = Held::Write {
                        request,
                        path: full,
                        text,
                    };
                    self.push_ask(i, None, what, Some(held));
                }
            }
            AgentUpdate::Run { run, command, cwd } => {
                let rule = command_rule(&command);
                let allowed = self.agents.tabs[i].allowed_runs.contains(&rule)
                    || self.agents.allowed.contains(&format!("{key}|run:{rule}"))
                    || self.auto_allows(i, &AgentRequest::Run(command.clone()));
                if allowed {
                    self.agent_run(i, run, command, cwd, fx);
                } else {
                    let what = AgentRequest::Run(command.clone());
                    self.push_ask(i, None, what, Some(Held::Run { run, command, cwd }));
                }
            }
            AgentUpdate::Header { chips, note } => {
                let chips = match plugin_pieces(vec![Node::Bar {
                    chips,
                    hint: String::new(),
                }])
                .pop()
                {
                    Some(Node::Bar { chips, .. }) => chips,
                    _ => Vec::new(),
                };
                self.agents.tabs[i].header =
                    (!chips.is_empty() || !note.is_empty()).then_some((chips, note));
            }
            AgentUpdate::Commands(commands) => self.agents.tabs[i].commands = commands,
            AgentUpdate::Modes { modes, current } => {
                let tab = &mut self.agents.tabs[i];
                tab.mode = modes.iter().any(|m| m.id == current).then_some(current);
                tab.modes = modes;
            }
            AgentUpdate::Usage { used, size, note } => {
                self.agents.tabs[i].usage = Some((used, size, note));
            }
            AgentUpdate::Stop { run } => {
                if let Some(r) = self.agents.tabs[i].runs.iter().find(|r| r.run == run) {
                    let term = r.term;
                    self.kill_terminal(term, fx);
                }
            }
        }
        self.draw_agent(&key);
    }

    fn agent_say(&mut self, i: usize, part: AgentPart, at: u64) {
        if !matches!(part, AgentPart::Thought(_)) {
            self.end_thought(i, at);
        }
        let tab = &mut self.agents.tabs[i];
        if !matches!(tab.turns.last(), Some(Turn::Agent(_))) {
            let mode = tab
                .mode
                .as_ref()
                .and_then(|m| tab.modes.iter().find(|x| &x.id == m))
                .map(|m| (m.name.clone(), m.colour.clone()));
            tab.turns.push(Turn::Agent(Reply {
                items: Vec::new(),
                mode,
                started: at,
                ended: None,
            }));
        }
        let Some(Turn::Agent(items)) = tab.turns.last_mut() else {
            return;
        };
        match part {
            AgentPart::Text(t) => match items.last_mut() {
                Some(Item::Text(text)) => text.push_str(&t),
                _ => items.push(Item::Text(t)),
            },
            AgentPart::Thought(t) => {
                if tab.thought.is_none() {
                    tab.thought = Some((at, at));
                }
                if let Some((_, end)) = &mut tab.thought {
                    *end = at;
                }
                match items.last_mut() {
                    Some(Item::Thought(text, None)) => text.push_str(&t),
                    _ => items.push(Item::Thought(t, None)),
                }
            }
            AgentPart::Plan(steps) => match items.iter_mut().find(|i| matches!(i, Item::Plan(_))) {
                Some(Item::Plan(old)) => *old = steps,
                _ => items.push(Item::Plan(steps)),
            },
            AgentPart::Panel(nodes) => items.push(Item::Panel(plugin_pieces(nodes))),
            AgentPart::Tool(tool) => {
                tab.tool_times
                    .entry(tool.id.clone())
                    .and_modify(|t| t.1 = at)
                    .or_insert((at, at));
                let found = items
                    .iter_mut()
                    .find(|i| matches!(i, Item::Tool(t) if t.id == tool.id));
                match found {
                    Some(Item::Tool(t)) => *t = tool,
                    _ => items.push(Item::Tool(tool)),
                }
            }
        }
    }

    /// A thought that's over gets how long it took.
    fn end_thought(&mut self, i: usize, at: u64) {
        let tab = &mut self.agents.tabs[i];
        let Some((start, _)) = tab.thought.take() else {
            return;
        };
        let secs = (at.saturating_sub(start) / 1000).max(1);
        if let Some(Turn::Agent(items)) = tab.turns.last_mut()
            && let Some(Item::Thought(_, took @ None)) = items
                .iter_mut()
                .rev()
                .find(|i| matches!(i, Item::Thought(..)))
        {
            *took = Some(secs);
        }
    }

    fn push_ask(&mut self, i: usize, request: Option<u32>, what: AgentRequest, held: Option<Held>) {
        let tab = &mut self.agents.tabs[i];
        tab.next_ask += 1;
        tab.asks.push(Ask {
            request,
            what,
            held,
        });
        let name = format!("{AGENT}{}", tab.key);
        // A question shows: its tab comes forward.
        let bar = self.bar_mut(Side::Right);
        bar.visible = true;
        bar.active = Some(name);
    }

    /// How the chat's current mode asks.
    fn mode_asks(tab: &Tab) -> AgentAsks {
        tab.mode
            .as_ref()
            .and_then(|m| tab.modes.iter().find(|x| &x.id == m))
            .map_or(AgentAsks::Unsaid, |m| m.asks)
    }

    /// Whether the chat's mode, or merlin's edit pill, lets this through
    /// without a card. Reading outside the workspace always asks.
    fn auto_allows(&self, i: usize, what: &AgentRequest) -> bool {
        let tab = &self.agents.tabs[i];
        let asks = Self::mode_asks(tab);
        match what {
            AgentRequest::Edit { .. } => match asks {
                AgentAsks::Unsaid => tab.auto,
                AgentAsks::Edits | AgentAsks::EditsAndSafeCommands | AgentAsks::Never => true,
                AgentAsks::ReadsOnly | AgentAsks::AsksFirst => false,
            },
            AgentRequest::Run(command) => match asks {
                AgentAsks::Never => true,
                AgentAsks::EditsAndSafeCommands => read_only_command(command),
                AgentAsks::ReadsOnly => tab.auto && read_only_command(command),
                _ => false,
            },
            _ => false,
        }
    }

    /// merlin's edit pill, by how the chat's mode asks: none when the
    /// mode says itself.
    fn edit_pill(tab: &Tab) -> Option<Chip> {
        let label = match (Self::mode_asks(tab), tab.auto) {
            (AgentAsks::Unsaid, false) => "⏸ Ask before edits ▾",
            (AgentAsks::Unsaid, true) => "⏵ Edit automatically ▾",
            (AgentAsks::ReadsOnly, false) => "⏸ Ask before commands ▾",
            (AgentAsks::ReadsOnly, true) => "⏵ Run read-only commands ▾",
            _ => return None,
        };
        Some(Chip {
            id: "agent:edit".into(),
            label: label.into(),
            tone: Tone::Normal,
        })
    }

    /// Switches a chat's agent to one of its modes, as Tab or the badge's
    /// menu does. A mode that never asks asks the user first, once a chat.
    pub(crate) fn set_agent_mode(&mut self, key: &str, mode: &str, fx: &mut Vec<Effect>) {
        let Some(i) = self.agents.tabs.iter().position(|t| t.key == key) else {
            return;
        };
        let Some(spec) = self.spec(key).cloned() else {
            return;
        };
        let tab = &self.agents.tabs[i];
        let Some(m) = tab.modes.iter().find(|m| m.id == mode).cloned() else {
            return;
        };
        if tab.mode.as_deref() == Some(mode) {
            return;
        }
        if m.asks == AgentAsks::Never && !tab.never_ok {
            self.push_ask(
                i,
                None,
                AgentRequest::Other(format!(
                    "to switch to {}, where it acts without asking",
                    m.name
                )),
                Some(Held::Mode(m.id)),
            );
            self.draw_agent(key);
            return;
        }
        let tab = &mut self.agents.tabs[i];
        tab.mode = Some(m.id.clone());
        fx.push(Effect::Agent {
            plugin: spec.plugin,
            call: AgentCall::Mode {
                chat: tab.chat,
                mode: m.id,
            },
        });
        self.draw_agent(key);
    }

    /// Tab: the chat's agent goes to its next mode.
    pub(crate) fn next_agent_mode(&mut self, key: &str, fx: &mut Vec<Effect>) -> bool {
        let Some(tab) = self.agents.tabs.iter().find(|t| t.key == key) else {
            return false;
        };
        if tab.modes.len() < 2 {
            return false;
        }
        let at = tab
            .mode
            .as_ref()
            .and_then(|m| tab.modes.iter().position(|x| &x.id == m))
            .unwrap_or(0);
        let next = tab.modes[(at + 1) % tab.modes.len()].id.clone();
        self.set_agent_mode(key, &next, fx);
        true
    }

    /// What an answer allows for the rest of the turn.
    fn allow_for_turn(&mut self, i: usize, what: &AgentRequest) {
        let shown = match what {
            AgentRequest::Edit { path, .. } => Some(self.shown_path(&self.agent_path(path))),
            _ => None,
        };
        let tab = &mut self.agents.tabs[i];
        match what {
            AgentRequest::Edit { .. } => {
                tab.allowed_paths.extend(shown);
            }
            AgentRequest::Run(c) => tab.allowed_runs.push(command_rule(c)),
            _ => {}
        }
    }

    /// The user answered the first card in a tab.
    fn agent_answer(&mut self, key: &str, answer: AgentAnswer, fx: &mut Vec<Effect>) {
        let Some(i) = self.agents.tabs.iter().position(|t| t.key == key) else {
            return;
        };
        let Some(ask) =
            (!self.agents.tabs[i].asks.is_empty()).then(|| self.agents.tabs[i].asks.remove(0))
        else {
            return;
        };
        let Some(spec) = self.spec(key).cloned() else {
            return;
        };
        let chat = self.agents.tabs[i].chat;
        let outside = matches!(&ask.what, AgentRequest::Read(p) if !self.in_workspace(p));
        if answer != AgentAnswer::Deny {
            self.allow_for_turn(i, &ask.what);
        }
        if answer == AgentAnswer::AlwaysAllow && !outside {
            let rule = match &ask.what {
                AgentRequest::Edit { .. } => Some(format!("{key}|edit")),
                AgentRequest::Run(c) => Some(format!("{key}|run:{}", command_rule(c))),
                _ => None,
            };
            if let Some(rule) = rule
                && !self.agents.allowed.contains(&rule)
            {
                self.agents.allowed.push(rule);
                fx.push(Effect::SaveAgentAllowed(self.agents.allowed.clone()));
            }
        }
        if let AgentAnswer::Mode(mode) = &answer {
            self.agents.tabs[i].mode = Some(mode.clone());
        }
        let plugin = spec.plugin.clone();
        match (ask.request, ask.held) {
            (Some(request), _) => fx.push(Effect::Agent {
                plugin,
                call: AgentCall::Answer {
                    chat,
                    request,
                    answer,
                },
            }),
            (None, Some(held)) => {
                let denied = |what: &str| Err(format!("the user didn't allow {what}"));
                match (held, answer) {
                    (Held::Read { request, .. }, AgentAnswer::Deny) => fx.push(Effect::Agent {
                        plugin,
                        call: AgentCall::File {
                            chat,
                            request,
                            result: denied("reading it"),
                        },
                    }),
                    (Held::Write { request, .. }, AgentAnswer::Deny) => fx.push(Effect::Agent {
                        plugin,
                        call: AgentCall::File {
                            chat,
                            request,
                            result: denied("the edit"),
                        },
                    }),
                    (Held::Mode(_), AgentAnswer::Deny) => {}
                    (Held::Mode(mode), _) => {
                        self.agents.tabs[i].never_ok = true;
                        self.set_agent_mode(key, &mode, fx);
                    }
                    (Held::Run { run, .. }, AgentAnswer::Deny) => fx.push(Effect::Agent {
                        plugin,
                        call: AgentCall::Ran {
                            chat,
                            run,
                            result: Err("the user didn't allow running it".into()),
                        },
                    }),
                    (Held::Read { request, path }, _) => {
                        self.agent_read(&plugin, chat, request, &path, fx)
                    }
                    (
                        Held::Write {
                            request,
                            path,
                            text,
                        },
                        _,
                    ) => self.agent_write(&plugin, chat, request, &path, text, fx),
                    (Held::Run { run, command, cwd }, _) => {
                        self.agent_run(i, run, command, cwd, fx)
                    }
                }
            }
            (None, None) => {}
        }
        self.draw_agent(key);
    }

    /// A path the agent named, made whole against the workspace.
    fn agent_full(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        }
    }

    /// A path the agent named, as open tabs have it: from the workspace's
    /// root when it's inside it.
    fn agent_path(&self, path: &str) -> PathBuf {
        let full = self.agent_full(path);
        match full.strip_prefix(&self.root) {
            Ok(rel) if !self.root.as_os_str().is_empty() => rel.to_path_buf(),
            _ => full,
        }
    }

    /// The path as the chat shows it: in the workspace, from its root.
    fn shown_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    /// Whether a path is inside the workspace's folder, without `..`.
    fn in_workspace(&self, path: &str) -> bool {
        let full = self.agent_full(path);
        // With no folder open, a relative path is as inside as it gets.
        let inside = if self.root.as_os_str().is_empty() {
            full.is_relative()
        } else {
            full.starts_with(&self.root)
        };
        inside
            && !full
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
    }

    /// A file's text as merlin has it: an open tab's, or none.
    fn open_text_at(&self, path: &Path) -> Option<String> {
        self.docs
            .iter()
            .find(|d| d.review.is_none() && d.buffer.path.as_deref() == Some(path))
            .map(|d| d.buffer.text())
    }

    fn agent_read(
        &mut self,
        plugin: &str,
        chat: u32,
        request: u32,
        path: &Path,
        fx: &mut Vec<Effect>,
    ) {
        match self.open_text_at(path) {
            Some(text) => fx.push(Effect::Agent {
                plugin: plugin.to_string(),
                call: AgentCall::File {
                    chat,
                    request,
                    result: Ok(text),
                },
            }),
            None => fx.push(Effect::AgentDisk {
                plugin: plugin.to_string(),
                chat,
                request,
                path: path.to_path_buf(),
                write: None,
            }),
        }
    }

    /// Writes a file for an agent: an open tab's text as one edit, then
    /// saved; a file that isn't open, on disk.
    fn agent_write(
        &mut self,
        plugin: &str,
        chat: u32,
        request: u32,
        path: &Path,
        text: String,
        fx: &mut Vec<Effect>,
    ) {
        // A file that isn't open opens in a tab, and takes the edit there,
        // so Undo can take it back as in one that was.
        let Some(doc) = self.doc_at(path) else {
            fx.push(Effect::Open(path.to_path_buf()));
            self.agents.pending_writes.push((
                plugin.to_string(),
                chat,
                request,
                path.to_path_buf(),
                text,
            ));
            return;
        };
        let before = doc.buffer.text();
        doc.buffer.begin(doc.cursor, crate::history::Kind::Other);
        doc.buffer.apply(crate::edit::Edit {
            at: 0,
            removed: before.clone(),
            inserted: text.clone(),
        });
        doc.cursor = doc.cursor.min(doc.buffer.len());
        doc.anchor = None;
        doc.buffer.commit(doc.cursor);
        fx.push(Effect::Save {
            doc: doc.id,
            path: path.to_path_buf(),
            text: text.clone(),
            revision: doc.buffer.revision(),
        });
        self.agent_changed(plugin, chat, path, before, &text);
        fx.push(Effect::Agent {
            plugin: plugin.to_string(),
            call: AgentCall::File {
                chat,
                request,
                result: Ok(String::new()),
            },
        });
    }

    /// A file opened: the writes waiting for it go in.
    pub(crate) fn agent_opened(&mut self, path: &Path, fx: &mut Vec<Effect>) {
        let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.agents.pending_writes)
            .into_iter()
            .partition(|(.., p, _)| p == path);
        self.agents.pending_writes = later;
        for (plugin, chat, request, path, text) in now {
            if self.doc_at(&path).is_some() {
                self.agent_write(&plugin, chat, request, &path, text, fx);
                if let Some(i) = self.tab_index(&plugin, chat) {
                    let key = self.agents.tabs[i].key.clone();
                    self.draw_agent(&key);
                }
            }
        }
    }

    /// A file changed on disk that an agent's last turn says it edits: the
    /// agent wrote it itself, as CLIs do. Its text before goes to the turn's
    /// changes. Says whether it was so. (The turn may have just ended: the
    /// disk is watched with a delay.)
    pub(crate) fn agent_wrote_itself(&mut self, path: &Path, before: String, after: &str) -> bool {
        let shown = self.shown_path(path);
        let tab = self.agents.tabs.iter().position(|t| {
            t.turns.last().is_some_and(|turn| match turn {
                Turn::Agent(items) => items.iter().any(|i| {
                    matches!(i, Item::Tool(tool) if tool.kind == ToolKind::Edit
                            && (tool.title == shown || shown.ends_with(&tool.title)))
                }),
                Turn::You { .. } => false,
            })
        });
        let Some(i) = tab else {
            return false;
        };
        let (key, chat) = (self.agents.tabs[i].key.clone(), self.agents.tabs[i].chat);
        let plugin = self
            .spec(&key)
            .map(|s| s.plugin.clone())
            .unwrap_or_default();
        self.agent_changed(&plugin, chat, path, before, after);
        self.draw_agent(&key);
        true
    }

    /// Notes a file an agent changed, keeping it as it first was.
    fn agent_changed(&mut self, plugin: &str, chat: u32, path: &Path, before: String, after: &str) {
        let Some(i) = self.tab_index(plugin, chat) else {
            return;
        };
        let tab = &mut self.agents.tabs[i];
        match tab.changes.iter_mut().find(|c| c.path == path) {
            Some(c) => {
                (c.added, c.removed) = line_counts(&c.before, after);
            }
            None => {
                let (added, removed) = line_counts(&before, after);
                tab.changes.push(Change {
                    path: path.to_path_buf(),
                    before,
                    added,
                    removed,
                });
            }
        }
    }

    /// The host read or wrote a file for an agent: its text, or for a
    /// write, what it was before.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn agent_disk(
        &mut self,
        plugin: &str,
        chat: u32,
        request: u32,
        path: PathBuf,
        wrote: Option<String>,
        result: Result<String, String>,
        fx: &mut Vec<Effect>,
    ) {
        let reply = match (&wrote, result) {
            (None, result) => result,
            (Some(text), Ok(before)) => {
                self.agent_changed(plugin, chat, &path, before, text);
                Ok(String::new())
            }
            (Some(_), Err(e)) => Err(e),
        };
        if request > 0 {
            fx.push(Effect::Agent {
                plugin: plugin.to_string(),
                call: AgentCall::File {
                    chat,
                    request,
                    result: reply,
                },
            });
        }
        if let Some(i) = self.tab_index(plugin, chat) {
            let key = self.agents.tabs[i].key.clone();
            self.draw_agent(&key);
        }
    }

    fn agent_run(
        &mut self,
        i: usize,
        run: u32,
        command: String,
        cwd: Option<String>,
        fx: &mut Vec<Effect>,
    ) {
        let key = self.agents.tabs[i].key.clone();
        let chat = self.agents.tabs[i].chat;
        let Some(spec) = self.spec(&key).cloned() else {
            return;
        };
        let line = match &cwd {
            Some(dir) if !dir.is_empty() => format!("cd {} && {command}", shell_quote(dir)),
            _ => command.clone(),
        };
        let name = format!("{}: {}", spec.name, command_rule(&command));
        let id = format!("agent:{chat}:{run}");
        let term = self.run_for_plugin(spec.plugin.clone(), id, &name, &line, fx);
        self.agents.tabs[i].runs.push(RunRecord { run, term });
    }

    /// A command an agent ran ended: what it printed goes back to it.
    /// Returns false for a run that isn't an agent's.
    pub(crate) fn agent_ran(
        &mut self,
        plugin: &str,
        id: &str,
        output: String,
        code: Option<i32>,
        fx: &mut Vec<Effect>,
    ) -> bool {
        let Some(rest) = id.strip_prefix("agent:") else {
            return false;
        };
        let Some((chat, run)) = rest
            .split_once(':')
            .and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)))
        else {
            return true;
        };
        fx.push(Effect::Agent {
            plugin: plugin.to_string(),
            call: AgentCall::Ran {
                chat,
                run,
                result: Ok((output, code)),
            },
        });
        true
    }

    /// The terminal a run of an agent's is in, to show it.
    fn agent_terminal(&self, key: &str, run: u32) -> Option<u64> {
        let tab = self.agents.tabs.iter().find(|t| t.key == key)?;
        tab.runs.iter().find(|r| r.run == run).map(|r| r.term)
    }

    /// Sends what's in the box to the tab's agent.
    pub(crate) fn agent_send(&mut self, key: &str, text: &str, fx: &mut Vec<Effect>) {
        let Some(spec) = self.spec(key).cloned() else {
            return;
        };
        let Some(i) = self.agents.tabs.iter().position(|t| t.key == key) else {
            return;
        };
        if spec.missing.is_some() || text.trim().is_empty() {
            return;
        }
        let name = format!("{AGENT}{key}");
        // While it works, a message waits its turn above the box.
        if self.agents.tabs[i].busy {
            self.agents.tabs[i].queued.push(text.trim().to_string());
            self.inputs.remove(&(name.clone(), PROMPT.to_string()));
            self.input_cursors.remove(&(name, PROMPT.to_string()));
            self.draw_agent(key);
            return;
        }
        let (context, chips) = self.agent_context(i);
        let tab = &mut self.agents.tabs[i];
        tab.turns.push(Turn::You {
            text: text.trim().to_string(),
            chips,
        });
        tab.busy = true;
        tab.ended = None;
        fx.push(Effect::Agent {
            plugin: spec.plugin.clone(),
            call: AgentCall::Prompt {
                chat: tab.chat,
                agent: spec.id.clone(),
                text: text.trim().to_string(),
                context,
            },
        });
        self.inputs.remove(&(name.clone(), PROMPT.to_string()));
        self.input_cursors.remove(&(name, PROMPT.to_string()));
        self.draw_agent(key);
    }

    /// What goes with a message: the open file, or its selected lines.
    fn agent_context(&self, i: usize) -> (Vec<AgentContext>, Vec<String>) {
        if !self.agents.tabs[i].attach {
            return (Vec::new(), Vec::new());
        }
        let doc = self.doc();
        let Some(path) = doc.buffer.path.as_deref() else {
            return (Vec::new(), Vec::new());
        };
        let shown = self.shown_path(path);
        let name = path
            .file_name()
            .map_or(shown.clone(), |n| n.to_string_lossy().into_owned());
        match doc.anchor.filter(|a| *a != doc.cursor) {
            Some(anchor) => {
                let a = doc.line_col(anchor).0 as u32 + 1;
                let b = doc.line_col(doc.cursor).0 as u32 + 1;
                let (first, last) = (a.min(b), a.max(b));
                (
                    vec![AgentContext::Lines(shown, first, last)],
                    vec![format!("{name} {first}–{last}")],
                )
            }
            None => (vec![AgentContext::File(shown)], vec![name]),
        }
    }

    /// A click in an agent's tab.
    pub(crate) fn agent_click(&mut self, id: &str, fx: &mut Vec<Effect>) -> bool {
        let key = self.shown_agent().unwrap_or_default();
        // A slash command picked: it goes in the box.
        if let Some(name) = id.strip_prefix("agent:cmd:") {
            let input = (format!("{AGENT}{key}"), PROMPT.to_string());
            let text = format!("/{name} ");
            self.input_cursors.insert(input.clone(), text.len());
            self.inputs.insert(input, text);
            return true;
        }
        let Some(rest) = id.strip_prefix("agent:") else {
            // The plugin's own pieces, in its agent's chat: its click.
            let in_chat = self
                .active_panel_name(Side::Right)
                .is_some_and(|n| n.starts_with(AGENT));
            if in_chat
                && self.focused_side() == Some(Side::Right)
                && let Some(spec) = self.spec(&key)
            {
                fx.push(Effect::PluginClick {
                    plugin: spec.plugin.clone(),
                    button: id.to_string(),
                });
                return true;
            }
            return false;
        };
        let (what, arg) = rest.split_once(':').unwrap_or((rest, ""));
        match what {
            "allow" => self.agent_answer(&key, AgentAnswer::AllowOnce, fx),
            "always" => self.agent_answer(&key, AgentAnswer::AlwaysAllow, fx),
            "deny" => self.agent_answer(&key, AgentAnswer::Deny, fx),
            "mode" => {
                let leaving = self
                    .agents
                    .tabs
                    .iter()
                    .find(|t| t.key == key)
                    .and_then(|t| t.asks.first())
                    .is_some_and(|a| matches!(a.what, AgentRequest::Mode(_)));
                if leaving {
                    self.agent_answer(&key, AgentAnswer::Mode(arg.to_string()), fx);
                } else {
                    self.set_agent_mode(&key, arg, fx);
                }
            }
            "stop" => {
                if let Some(spec) = self.spec(&key).cloned()
                    && let Some(tab) = self.agents.tabs.iter().find(|t| t.key == key)
                    && tab.busy
                {
                    fx.push(Effect::Agent {
                        plugin: spec.plugin,
                        call: AgentCall::Cancel { chat: tab.chat },
                    });
                }
            }
            "modes" => self.agent_modes_menu(&key),
            "edit" => {
                self.popup = None;
                self.context = Some(super::context::ContextMenu {
                    items: vec!["agent.auto.off", "agent.auto.on"],
                    selected: None,
                    title: None,
                });
                self.focus = super::Focus::Menu;
            }
            "ring" => {
                self.popup = None;
                self.context = Some(super::context::ContextMenu {
                    items: vec!["agent.usage", "agent.compact"],
                    selected: None,
                    title: None,
                });
                self.focus = super::Focus::Menu;
            }
            "send" => {
                let input = (format!("{AGENT}{key}"), PROMPT.to_string());
                let text = self.inputs.remove(&input).unwrap_or_default();
                self.input_cursors.remove(&input);
                if !text.trim().is_empty() {
                    self.agent_submit(&input.0, &text, fx);
                }
            }
            "attach" => {
                if let Some(tab) = self.agents.tabs.iter_mut().find(|t| t.key == key) {
                    tab.attach = !tab.attach;
                }
                self.draw_agent(&key);
            }
            "pick" => self.agent_menu(fx),
            "terminal" => {
                if let Ok(run) = arg.parse()
                    && let Some(term) = self.agent_terminal(&key, run)
                {
                    self.show_terminal_tab(term);
                }
            }
            "kill" => {
                if let Ok(run) = arg.parse()
                    && let Some(term) = self.agent_terminal(&key, run)
                {
                    self.kill_terminal(term, fx);
                }
            }
            "review" => {
                if let Some(c) = self.agent_change(&key, arg) {
                    let name = self.agent_name(&key);
                    let path = c.path.clone();
                    let before = c.before.clone();
                    self.review_against(path, before, &format!("Before {name}"), fx);
                }
            }
            "revert" => {
                self.agent_revert(&key, Some(arg), fx);
            }
            "revert-all" => self.agent_revert(&key, None, fx),
            "review-first" => {
                let first = self
                    .agents
                    .tabs
                    .iter()
                    .find(|t| t.key == key)
                    .and_then(|t| t.changes.first())
                    .map(|c| self.shown_path(&c.path));
                if let Some(path) = first {
                    return self.agent_click(&format!("agent:review:{path}"), fx);
                }
            }
            "copy" => fx.push(Effect::Copy(arg.to_string())),
            "copyturn" => {
                let text = self
                    .agents
                    .tabs
                    .iter()
                    .find(|t| t.key == key)
                    .and_then(|t| t.turns.get(arg.parse::<usize>().ok()?))
                    .and_then(|turn| match turn {
                        Turn::Agent(reply) => Some(
                            reply
                                .iter()
                                .filter_map(|i| match i {
                                    Item::Text(t) => Some(t.as_str()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n\n"),
                        ),
                        Turn::You { .. } => None,
                    });
                if let Some(text) = text {
                    fx.push(Effect::Copy(text));
                }
            }
            "plan" | "changes" => {
                if let Some(tab) = self.agents.tabs.iter_mut().find(|t| t.key == key) {
                    if what == "plan" {
                        tab.plan_open = !tab.plan_open;
                    } else {
                        tab.changes_open = !tab.changes_open;
                    }
                }
                self.draw_agent(&key);
            }
            "keep" | "keep-all" => {
                let root = self.root.clone();
                if let Some(tab) = self.agents.tabs.iter_mut().find(|t| t.key == key) {
                    tab.changes.retain(|c| {
                        what == "keep"
                            && c.path.strip_prefix(&root).unwrap_or(&c.path) != Path::new(arg)
                    });
                }
                self.draw_agent(&key);
            }
            "review-all" => {
                let all: Vec<(PathBuf, String)> = self
                    .agents
                    .tabs
                    .iter()
                    .find(|t| t.key == key)
                    .map(|t| {
                        t.changes
                            .iter()
                            .map(|c| (c.path.clone(), c.before.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let name = self.agent_name(&key);
                for (path, before) in all {
                    self.review_against(path, before, &format!("Before {name}"), fx);
                }
            }
            "queue-edit" | "queue-drop" | "queue-now" => {
                let n: usize = arg.parse().unwrap_or(usize::MAX);
                let Some(tab) = self.agents.tabs.iter_mut().find(|t| t.key == key) else {
                    return true;
                };
                if n >= tab.queued.len() {
                    return true;
                }
                match what {
                    "queue-edit" => {
                        let text = tab.queued.remove(n);
                        let input = (format!("{AGENT}{key}"), PROMPT.to_string());
                        self.input_cursors.insert(input.clone(), text.len());
                        self.inputs.insert(input, text);
                    }
                    "queue-drop" => {
                        tab.queued.remove(n);
                    }
                    _ => {
                        // Send Now: this one goes first, and the turn stops.
                        let text = tab.queued.remove(n);
                        tab.queued.insert(0, text);
                        if tab.busy
                            && let Some(spec) = self.spec(&key).cloned()
                        {
                            let chat = self
                                .agents
                                .tabs
                                .iter()
                                .find(|t| t.key == key)
                                .map_or(0, |t| t.chat);
                            fx.push(Effect::Agent {
                                plugin: spec.plugin,
                                call: AgentCall::Cancel { chat },
                            });
                        }
                    }
                }
                self.draw_agent(&key);
            }
            "fold" => {
                let mut parts = arg.split(':').filter_map(|p| p.parse::<usize>().ok());
                if let (Some(turn), Some(first)) = (parts.next(), parts.next())
                    && let Some(tab) = self.agents.tabs.iter_mut().find(|t| t.key == key)
                    && !tab.open_folds.remove(&(turn, first))
                {
                    tab.open_folds.insert((turn, first));
                }
                self.draw_agent(&key);
            }
            "url" => {
                if arg.starts_with("https://") {
                    fx.push(Effect::OpenUrl(arg.to_string()));
                }
            }
            "output" => {
                let plugin = self
                    .spec(&key)
                    .map(|s| s.plugin.clone())
                    .unwrap_or_default();
                self.show_output(&plugin);
            }
            "forget" => {
                if let Ok(n) = arg.parse::<usize>()
                    && n < self.agents.allowed.len()
                {
                    self.agents.allowed.remove(n);
                    fx.push(Effect::SaveAgentAllowed(self.agents.allowed.clone()));
                    self.draw_agent(&key);
                }
            }
            "back" => {
                self.agents.showing_allowed = None;
                self.draw_agent(&key);
            }
            _ => {}
        }
        true
    }

    fn agent_change(&self, key: &str, path: &str) -> Option<&Change> {
        let tab = self.agents.tabs.iter().find(|t| t.key == key)?;
        tab.changes
            .iter()
            .find(|c| self.shown_path(&c.path) == path)
    }

    /// Puts files back as they were before the turn: one, or all.
    fn agent_revert(&mut self, key: &str, path: Option<&str>, fx: &mut Vec<Effect>) {
        let Some(i) = self.agents.tabs.iter().position(|t| t.key == key) else {
            return;
        };
        let chat = self.agents.tabs[i].chat;
        let plugin = self.spec(key).map(|s| s.plugin.clone()).unwrap_or_default();
        let picked: Vec<Change> = {
            let root = self.root.clone();
            let tab = &mut self.agents.tabs[i];
            let (go, stay): (Vec<Change>, Vec<Change>) =
                std::mem::take(&mut tab.changes).into_iter().partition(|c| {
                    path.is_none_or(|p| {
                        c.path.strip_prefix(&root).unwrap_or(&c.path) == Path::new(p)
                    })
                });
            tab.changes = stay;
            go
        };
        for change in picked {
            match self.doc_at(&change.path) {
                Some(doc) => {
                    let now = doc.buffer.text();
                    doc.buffer.begin(doc.cursor, crate::history::Kind::Other);
                    doc.buffer.apply(crate::edit::Edit {
                        at: 0,
                        removed: now,
                        inserted: change.before.clone(),
                    });
                    doc.cursor = doc.cursor.min(doc.buffer.len());
                    doc.anchor = None;
                    doc.buffer.commit(doc.cursor);
                    fx.push(Effect::Save {
                        doc: doc.id,
                        path: change.path.clone(),
                        text: change.before.clone(),
                        revision: doc.buffer.revision(),
                    });
                }
                None => fx.push(Effect::AgentDisk {
                    plugin: plugin.clone(),
                    chat,
                    request: 0,
                    path: change.path.clone(),
                    write: Some(change.before.clone()),
                }),
            }
        }
        self.draw_agent(key);
    }

    /// The agent waiting for an answer, if one is.
    pub(crate) fn agent_waiting(&self) -> Option<String> {
        let tab = self.agents.tabs.iter().find(|t| !t.asks.is_empty())?;
        Some(self.agent_name(&tab.key))
    }

    /// Shows the agents' chat: the tab waiting for an answer, the first
    /// one open, or the first agent's.
    pub(crate) fn open_first_agent(&mut self, fx: &mut Vec<Effect>) {
        let key = self
            .agents
            .tabs
            .iter()
            .find(|t| !t.asks.is_empty())
            .or(self.agents.tabs.first())
            .map(|t| t.key.clone())
            .or_else(|| self.agents.specs.first().map(AgentSpec::key));
        match key {
            Some(key) => self.open_agent(&key, true),
            None => {
                self.open_page(super::SETTINGS_PAGE, "Settings");
                fx.push(Effect::OpenPluginPage(None));
            }
        }
    }

    /// The + menu: every agent, those not installed marked.
    pub(crate) fn agent_menu(&mut self, _fx: &mut Vec<Effect>) {
        self.agents.menu = self.agents.specs.iter().map(AgentSpec::key).collect();
        let mut items: Vec<&'static str> = AGENT_ITEMS
            .iter()
            .take(self.agents.menu.len())
            .copied()
            .collect();
        items.push("-");
        items.push("agent.setup");
        items.push("agent.find");
        self.popup = None;
        self.context = Some(super::context::ContextMenu {
            items,
            selected: None,
            title: Some("Agents".into()),
        });
        self.focus = super::Focus::Menu;
    }

    /// The mode badge's menu: the agent's modes.
    fn agent_modes_menu(&mut self, key: &str) {
        let Some(tab) = self.agents.tabs.iter().find(|t| t.key == key) else {
            return;
        };
        let items = MODE_ITEMS.iter().take(tab.modes.len()).copied().collect();
        self.popup = None;
        self.context = Some(super::context::ContextMenu {
            items,
            selected: None,
            title: Some("Mode · Tab".into()),
        });
        self.focus = super::Focus::Menu;
    }

    /// The ⋯ menu.
    pub(crate) fn agent_more_menu(&mut self) {
        self.popup = None;
        self.context = Some(super::context::ContextMenu {
            items: vec!["agent.allowed", "agent.new_chat", "-", "agent.close"],
            selected: None,
            title: None,
        });
        self.focus = super::Focus::Menu;
    }

    /// A menu item's label and what goes at its right end, with whether
    /// that's a warning.
    pub(crate) fn agent_menu_item(&self, id: &str) -> Option<(String, Option<String>, bool)> {
        if let Some(n) = AGENT_ITEMS.iter().position(|i| *i == id) {
            let spec = self.spec(self.agents.menu.get(n)?)?;
            let label = format!("{} {}", spec.icon, spec.name);
            return Some(match &spec.missing {
                Some(_) => (label, Some("not installed".into()), true),
                None => (label, Some(spec.plugin.clone()), false),
            });
        }
        let key = self.shown_agent().unwrap_or_default();
        let tab = self.agents.tabs.iter().find(|t| t.key == key);
        if let Some(n) = MODE_ITEMS.iter().position(|i| *i == id) {
            let tab = tab?;
            let m = tab.modes.get(n)?;
            let what = match m.asks {
                AgentAsks::Unsaid => "",
                AgentAsks::ReadsOnly => "Reads and plans; can't edit",
                AgentAsks::AsksFirst => "Asks before edits and commands",
                AgentAsks::Edits => "Edits without asking; you Keep or Undo after",
                AgentAsks::EditsAndSafeCommands => "Edits and safe commands; asks before the rest",
                AgentAsks::Never => "Never asks",
            };
            let on = tab.mode.as_deref() == Some(m.id.as_str());
            let right = if on {
                "✓".to_string()
            } else {
                what.to_string()
            };
            return Some((m.name.clone(), Some(right), m.asks == AgentAsks::Never));
        }
        let read_only = tab.is_some_and(|t| Self::mode_asks(t) == AgentAsks::ReadsOnly);
        let auto = tab.is_some_and(|t| t.auto);
        match id {
            "agent.auto.off" | "agent.auto.on" => {
                let on = id == "agent.auto.on";
                let label = match (read_only, on) {
                    (false, false) => "⏸ Ask before edits",
                    (false, true) => "⏵ Edit automatically",
                    (true, false) => "⏸ Ask before commands",
                    (true, true) => "⏵ Run read-only commands",
                };
                let right = if on == auto {
                    "✓"
                } else if on && read_only {
                    "ls, grep, git log…"
                } else if on {
                    "you review after"
                } else {
                    ""
                };
                return Some((
                    label.into(),
                    Some(right.into()).filter(|r: &String| !r.is_empty()),
                    false,
                ));
            }
            "agent.usage" => {
                let (used, size, note) = tab?.usage.clone()?;
                let label = if size > 0 {
                    format!(
                        "{:.0}% of {} tokens",
                        used as f64 * 100.0 / size as f64,
                        short_tokens(size)
                    )
                } else {
                    format!("{} tokens", short_tokens(used))
                };
                return Some((label, Some(note).filter(|n| !n.is_empty()), false));
            }
            "agent.compact" => return Some(("Compact Now".into(), Some("/compact".into()), false)),
            _ => {}
        }
        let label = match id {
            "agent.setup" => "Agent Settings…",
            "agent.find" => "Find Agent Plugins…",
            "agent.allowed" => "Allowed in This Workspace",
            "agent.new_chat" => "New Chat",
            "agent.close" => "Close This Agent",
            _ => return None,
        };
        // The agent's plugin, whose page Agent Settings opens.
        let right = (id == "agent.setup")
            .then(|| {
                let key = self.shown_agent();
                let spec = key
                    .and_then(|k| self.spec(&k))
                    .or(self.agents.specs.first())?;
                Some(format!("{}'s page", self.plugin_title(&spec.plugin)))
            })
            .flatten();
        Some((label.to_string(), right, false))
    }

    /// Runs one of the agents' menu items. Returns false if it isn't one.
    pub(crate) fn agent_menu_picked(&mut self, id: &str, fx: &mut Vec<Effect>) -> bool {
        let key = self.shown_agent().unwrap_or_default();
        if let Some(n) = AGENT_ITEMS.iter().position(|i| *i == id) {
            if let Some(k) = self.agents.menu.get(n).cloned() {
                self.open_agent(&k, true);
            }
            return true;
        }
        if let Some(n) = MODE_ITEMS.iter().position(|i| *i == id) {
            let mode = self
                .agents
                .tabs
                .iter()
                .find(|t| t.key == key)
                .and_then(|t| t.modes.get(n))
                .map(|m| m.id.clone());
            if let Some(mode) = mode {
                self.set_agent_mode(&key, &mode, fx);
            }
            return true;
        }
        match id {
            "agent.auto.off" | "agent.auto.on" => {
                if let Some(tab) = self.agents.tabs.iter_mut().find(|t| t.key == key) {
                    tab.auto = id == "agent.auto.on";
                }
                self.draw_agent(&key);
            }
            "agent.usage" => {}
            "agent.compact" => {
                if !key.is_empty() {
                    self.agent_send(&key, "/compact", fx);
                }
            }
            "agent.setup" => {
                let plugin = self
                    .spec(&key)
                    .or(self.agents.specs.first())
                    .map(|s| s.plugin.clone());
                self.open_page(super::SETTINGS_PAGE, "Settings");
                fx.push(Effect::OpenPluginPage(plugin));
            }
            "agent.find" => {
                self.open_page(super::SETTINGS_PAGE, "Settings");
                fx.push(Effect::OpenPluginPage(None));
            }
            "agent.allowed" => {
                self.agents.showing_allowed = Some(key.clone());
                self.draw_agent(&key);
            }
            "agent.new_chat" => {
                self.close_agent(&key, fx);
                self.open_agent(&key, true);
            }
            "agent.close" => self.close_agent(&key, fx),
            _ => return false,
        }
        true
    }

    /// Builds an agent tab's panel from its chat.
    pub(crate) fn draw_agent(&mut self, key: &str) {
        // An agent with a view of its own draws it; merlin adds its cards
        // as it shows (`agent_extras`).
        if self.spec(key).is_some_and(|s| s.view.is_some()) {
            return;
        }
        let name = format!("{AGENT}{key}");
        let panel = self.agent_panel(key);
        self.plugin_mut(&name).panel = Some(panel);
    }

    fn agent_panel(&self, key: &str) -> Panel {
        let spec = self.spec(key);
        let agent = spec.map_or_else(|| key.to_string(), |s| s.name.clone());
        let icon = spec.map_or("◆".to_string(), |s| s.icon.clone());
        let title = format!("{icon} {agent}");
        let mut nodes = Vec::new();
        let Some(tab) = self.agents.tabs.iter().find(|t| t.key == key) else {
            return Panel { title, nodes };
        };
        if self.agents.showing_allowed.as_deref() == Some(key) {
            return Panel {
                title,
                nodes: self.allowed_nodes(),
            };
        }
        if let Some(missing) = spec.and_then(|s| s.missing.as_ref()) {
            nodes.push(Node::Empty {
                title: format!("{} isn't installed", missing.program),
                text: "Install it, then come back to this tab.".into(),
                buttons: Vec::new(),
            });
            if let Some(install) = &missing.install {
                nodes.push(Node::Command {
                    text: install.clone(),
                    copy: button(&format!("agent:copy:{install}"), "Copy"),
                });
            }
            if let Some(url) = &missing.url {
                nodes.push(Node::Item {
                    id: format!("agent:url:{url}"),
                    label: format!("How to install {} ↗", missing.program),
                    tone: Tone::Badge,
                });
            }
            return Panel { title, nodes };
        }
        let last = tab.turns.len().saturating_sub(1);
        for (n, turn) in tab.turns.iter().enumerate() {
            match turn {
                Turn::You { text, chips } => nodes.push(Node::Message(Box::new(Message {
                    author: String::new(),
                    mine: true,
                    markdown: text.clone(),
                    chips: chips
                        .iter()
                        .map(|c| Chip {
                            id: String::new(),
                            label: c.clone(),
                            tone: Tone::Normal,
                        })
                        .collect(),
                    streaming: false,
                }))),
                Turn::Agent(reply) => {
                    let streaming = tab.busy && n == last;
                    self.agent_turn_nodes(&mut nodes, &title, n, reply, streaming, tab);
                }
            }
        }
        if let Some(ask) = tab.asks.first() {
            nodes.push(Node::Ask(Box::new(self.ask_card(&agent, ask, tab))));
        }
        match &tab.ended {
            Some((AgentOutcome::Cancelled, _)) => {
                let n = tab.changes.len();
                let files = if n == 1 { "file" } else { "files" };
                nodes.push(Node::Banner {
                    title: vec![(format!("Stopped. {n} {files} changed so far."), false)],
                    detail: String::new(),
                    tone: Tone::Normal,
                    buttons: if n > 0 {
                        vec![button("agent:review-first", "Review")]
                    } else {
                        Vec::new()
                    },
                });
            }
            Some((AgentOutcome::Failed, message)) => {
                let said = message
                    .as_deref()
                    .map_or(String::new(), |m| format!(": \"{m}\""));
                nodes.push(Node::Banner {
                    title: vec![(format!("{agent} stopped{said}."), false)],
                    detail: String::new(),
                    tone: Tone::Error,
                    buttons: vec![button("agent:output", "Open Output")],
                });
            }
            _ => {}
        }
        nodes.push(Node::Footer);
        // Above the box: the plan, messages waiting, and changed files.
        let plan = tab.turns.iter().rev().find_map(|t| match t {
            Turn::Agent(reply) => reply.iter().find_map(|i| match i {
                Item::Plan(p) => Some(p),
                _ => None,
            }),
            Turn::You { .. } => None,
        });
        if let Some(plan) = plan
            && (tab.busy || plan.iter().any(|s| s.state != StepState::Done))
            && !plan.is_empty()
        {
            nodes.push(Node::PlanDock {
                steps: plan.iter().map(|s| (s.text.clone(), s.state)).collect(),
                open: tab.plan_open,
            });
        }
        for (index, text) in tab.queued.iter().enumerate() {
            nodes.push(Node::Queued {
                index,
                text: text.clone(),
            });
        }
        if !tab.changes.is_empty() {
            nodes.push(Node::ChangesBar(Box::new(self.changes_bar(tab))));
        }
        let waiting = !tab.asks.is_empty();
        // What goes with the message, above its text.
        if let Some(path) = self.doc().buffer.path.as_deref() {
            let name = path
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
            nodes.push(Node::Chips {
                chips: vec![Chip {
                    id: "agent:attach".into(),
                    label: if tab.attach {
                        format!("{name} ✕")
                    } else {
                        format!("{name} left out")
                    },
                    tone: if tab.attach {
                        Tone::Normal
                    } else {
                        Tone::Muted
                    },
                }],
                hint: String::new(),
            });
        }
        nodes.push(Node::Input {
            id: PROMPT.into(),
            placeholder: if waiting {
                format!("{agent} is waiting for you to answer.")
            } else if tab.busy {
                format!("{agent} is working…")
            } else if tab.commands.is_empty() {
                format!("Ask {agent}")
            } else {
                format!("Ask {agent} · / for commands")
            },
            secret: false,
            multiline: true,
        });
        let mode = tab
            .mode
            .as_ref()
            .and_then(|m| tab.modes.iter().find(|x| &x.id == m))
            .map(|m| (m.name.clone(), m.colour.clone()));
        let mut chips: Vec<Chip> = Self::edit_pill(tab).into_iter().collect();
        if let Some((plugin_chips, _)) = &tab.header {
            chips.extend(plugin_chips.iter().cloned());
        }
        let ring = tab
            .usage
            .as_ref()
            .filter(|u| u.1 > 0)
            .map(|(used, size, note)| {
                let full = (*used as f64 * 100.0 / *size as f64)
                    .round()
                    .clamp(0.0, 100.0) as u8;
                let said = format!("{full}% of {} tokens", short_tokens(*size));
                (
                    full,
                    if note.is_empty() {
                        said
                    } else {
                        format!("{said} · {note}")
                    },
                )
            });
        nodes.push(Node::ComposeBar(Box::new(ComposeBar {
            mode,
            chips,
            ring,
            stop: tab.busy || waiting,
        })));
        Panel { title, nodes }
    }

    fn agent_turn_nodes(
        &self,
        nodes: &mut Vec<Node>,
        title: &str,
        turn: usize,
        reply: &Reply,
        streaming: bool,
        tab: &Tab,
    ) {
        // Who, in the mode it wrote in; then thoughts, the plan, the text
        // and runs of steps, folded, in the order they came.
        nodes.push(Node::Reply {
            author: title.to_string(),
            mode: reply.mode.clone(),
        });
        let items: &[Item] = reply;
        let last_text = items.iter().rposition(|i| matches!(i, Item::Text(_)));
        let mut n = 0;
        while n < items.len() {
            match &items[n] {
                Item::Thought(_, took) => nodes.push(Node::Styled(
                    match took {
                        Some(s) => format!("▸ Thought for {s} s"),
                        None => "▸ Thinking…".into(),
                    },
                    Tone::Muted,
                )),
                // The plan shows above the box, where it keeps its place.
                Item::Plan(_) => {}
                Item::Text(text) => nodes.push(Node::Message(Box::new(Message {
                    author: String::new(),
                    mine: false,
                    markdown: text.clone(),
                    chips: Vec::new(),
                    streaming: streaming && Some(n) == last_text,
                }))),
                Item::Panel(parts) => nodes.extend(parts.iter().cloned()),
                Item::Tool(_) => {
                    let first = n;
                    let mut tools = Vec::new();
                    while let Some(Item::Tool(t)) = items.get(n) {
                        tools.push(t);
                        n += 1;
                    }
                    self.fold_nodes(nodes, tab, turn, first, &tools);
                    continue;
                }
            }
            n += 1;
        }
        // Under a finished reply: model, time, and what it used.
        if let Some(ended) = reply.ended {
            let mut parts = Vec::new();
            if let Some(model) = tab
                .header
                .as_ref()
                .and_then(|(chips, _)| chips.first())
                .map(|c| c.label.trim_end_matches(" ▾").to_string())
            {
                parts.push(model);
            }
            parts.push(format!(
                "{} s",
                (ended.saturating_sub(reply.started) / 1000).max(1)
            ));
            if let Some((_, _, note)) = tab.usage.as_ref().filter(|u| !u.2.is_empty()) {
                parts.push(note.clone());
            }
            nodes.push(Node::TurnFoot {
                text: parts.join(" · "),
                copy: button(&format!("agent:copyturn:{turn}"), "Copy"),
            });
        }
        nodes.push(Node::ReplyEnd);
    }

    /// A run of steps: one line saying what they did, open to a line each.
    /// The one still running says what it's doing.
    fn fold_nodes(
        &self,
        nodes: &mut Vec<Node>,
        tab: &Tab,
        turn: usize,
        first: usize,
        tools: &[&AgentTool],
    ) {
        let count = |kind: ToolKind| tools.iter().filter(|t| t.kind == kind).count();
        let plural = |n: usize, one: &str, many: &str| {
            if n == 1 {
                one.to_string()
            } else {
                many.replace('#', &n.to_string())
            }
        };
        let mut did = Vec::new();
        let (read, edited, ran, searched) = (
            count(ToolKind::Read),
            count(ToolKind::Edit),
            count(ToolKind::Run),
            count(ToolKind::Search),
        );
        if read > 0 {
            did.push(plural(read, "read a file", "read # files"));
        }
        if edited > 0 {
            did.push(plural(edited, "edited a file", "edited #"));
        }
        if ran > 0 {
            did.push(plural(ran, "ran a command", "ran # commands"));
        }
        if searched > 0 {
            did.push(plural(searched, "searched once", "searched # times"));
        }
        let running = tools
            .iter()
            .rev()
            .find(|t| matches!(t.state, StepState::Running | StepState::Pending));
        let steps = plural(tools.len(), "1 step", "# steps");
        let (start, end) = tools.iter().fold((u64::MAX, 0), |(a, b), t| {
            let (s, e) = tab.tool_times.get(&t.id).copied().unwrap_or((a.min(b), b));
            (a.min(s), b.max(e))
        });
        let secs = (end.saturating_sub(start) / 1000).max(1);
        let open = tab.open_folds.contains(&(turn, first));
        nodes.push(Node::Fold {
            id: format!("agent:fold:{turn}:{first}"),
            label: match running {
                Some(t) => format!("{} {} · {steps}", running_verb(t.kind), t.title),
                None => steps,
            },
            detail: if running.is_some() {
                String::new()
            } else {
                format!("{}|{secs} s", did.join(", "))
            },
            open,
            running: running.is_some(),
        });
        if open {
            for tool in tools {
                nodes.push(Node::Step(Box::new(self.step_line(tool, tab))));
            }
        }
    }

    fn step_line(&self, tool: &AgentTool, tab: &Tab) -> StepLine {
        let done = tool.state == StepState::Done;
        let verb = match (tool.kind, tool.state) {
            (ToolKind::Read, StepState::Running) => "Reading",
            (ToolKind::Read, _) => "Read",
            (ToolKind::Edit, StepState::Running) => "Editing",
            (ToolKind::Edit, _) => "Edited",
            (ToolKind::Run, StepState::Running) => "Running",
            (ToolKind::Run, _) => "Ran",
            (ToolKind::Search, StepState::Running) => "Searching",
            (ToolKind::Search, _) => "Searched",
            (ToolKind::Other, _) => "",
        };
        let mut links = Vec::new();
        let mut buttons = Vec::new();
        if let Some(run) = tool.run
            && tab.runs.iter().any(|r| r.run == run)
        {
            links.push(button(&format!("agent:terminal:{run}"), "Terminal ↗"));
            if tool.state == StepState::Running {
                buttons.push(button(&format!("agent:kill:{run}"), "Stop"));
            }
        }
        let _ = done;
        StepLine {
            mark: tool.state,
            verb: verb.into(),
            name: tool.title.clone(),
            added: tool.added,
            removed: tool.removed,
            links,
            buttons,
        }
    }

    fn ask_card(&self, agent: &str, ask: &Ask, tab: &Tab) -> AskCard {
        let deny = Some(button("agent:deny", "Deny"));
        let unasked = ask.held.is_some();
        let mode_name = |id: &str| {
            tab.modes
                .iter()
                .find(|m| m.id == id)
                .map_or_else(|| id.to_string(), |m| m.name.clone())
        };
        match &ask.what {
            AgentRequest::Mode(to) => {
                let now = tab.mode.as_deref().unwrap_or_default();
                let planning = tab
                    .modes
                    .iter()
                    .any(|m| m.id == now && m.asks == AgentAsks::ReadsOnly);
                AskCard {
                    title: vec![(
                        if planning {
                            format!("{agent}'s plan is ready. Go on?")
                        } else {
                            format!("{agent} wants to switch modes")
                        },
                        false,
                    )],
                    warn: false,
                    code: None,
                    diff: Vec::new(),
                    note: String::new(),
                    buttons: to
                        .iter()
                        .map(|id| {
                            button(
                                &format!("agent:mode:{id}"),
                                &format!("Switch to {}", mode_name(id)),
                            )
                        })
                        .collect(),
                    quiet: Some(button(
                        "agent:deny",
                        &if now.is_empty() {
                            "Keep Going as It Is".to_string()
                        } else {
                            format!("Keep {}", mode_name(now))
                        },
                    )),
                    hint: String::new(),
                }
            }
            AgentRequest::Run(command) => AskCard {
                title: vec![(format!("{agent} wants to run a command"), false)],
                warn: false,
                code: Some(command.clone()),
                diff: Vec::new(),
                note: "In merlin/, in a terminal tab you can watch."
                    .replace("merlin", &self.project),
                buttons: vec![
                    button("agent:allow", "Allow Once"),
                    button(
                        "agent:always",
                        &format!("Always Allow `{}`", command_rule(command)),
                    ),
                ],
                quiet: deny,
                hint: "Enter allows once · Esc denies".into(),
            },
            AgentRequest::Edit { path, diff } => {
                let lines: Vec<String> = diff
                    .as_deref()
                    .unwrap_or("")
                    .lines()
                    .filter(|l| {
                        (l.starts_with('+') || l.starts_with('-'))
                            && !l.starts_with("+++")
                            && !l.starts_with("---")
                    })
                    .take(12)
                    .map(str::to_string)
                    .collect();
                if unasked {
                    AskCard {
                        title: vec![
                            (format!("{agent} is changing "), false),
                            (path.clone(), true),
                            (" without having asked".into(), false),
                        ],
                        warn: false,
                        code: None,
                        diff: lines,
                        note: String::new(),
                        buttons: vec![button("agent:allow", "Allow Once")],
                        quiet: deny,
                        hint: String::new(),
                    }
                } else {
                    AskCard {
                        title: vec![
                            (format!("{agent} wants to edit "), false),
                            (path.clone(), true),
                        ],
                        warn: false,
                        code: None,
                        diff: lines,
                        note: String::new(),
                        buttons: vec![
                            button("agent:allow", "Allow Once"),
                            button("agent:always", "Always Allow Edits"),
                        ],
                        quiet: deny,
                        hint: String::new(),
                    }
                }
            }
            AgentRequest::Read(path) => {
                let outside = !self.in_workspace(path);
                AskCard {
                    title: vec![(
                        if outside {
                            format!("{agent} wants to read a file outside this workspace")
                        } else {
                            format!("{agent} wants to read a file")
                        },
                        false,
                    )],
                    warn: outside,
                    code: Some(home_short(path)),
                    diff: Vec::new(),
                    note: String::new(),
                    buttons: vec![button("agent:allow", "Allow Once")],
                    quiet: deny,
                    hint: String::new(),
                }
            }
            AgentRequest::Other(what) => AskCard {
                title: vec![(format!("{agent} asks: {what}"), false)],
                warn: false,
                code: None,
                diff: Vec::new(),
                note: String::new(),
                buttons: vec![button("agent:allow", "Allow Once")],
                quiet: deny,
                hint: String::new(),
            },
        }
    }

    fn changes_bar(&self, tab: &Tab) -> ChangesBar {
        ChangesBar {
            files: tab
                .changes
                .iter()
                .map(|c| (self.shown_path(&c.path), c.added, c.removed))
                .collect(),
            open: tab.changes_open,
        }
    }

    #[allow(dead_code)]
    fn changes_card(&self, tab: &Tab) -> ChangesCard {
        let n = tab.changes.len();
        ChangesCard {
            title: format!("Changed {n} {}", if n == 1 { "file" } else { "files" }),
            revert_all: button("agent:revert-all", "Revert All"),
            files: tab
                .changes
                .iter()
                .map(|c| {
                    let shown = self.shown_path(&c.path);
                    ChangedFile {
                        path: shown.clone(),
                        added: c.added,
                        removed: c.removed,
                        review: button(&format!("agent:review:{shown}"), "Review"),
                        revert: button(&format!("agent:revert:{shown}"), "Revert"),
                    }
                })
                .collect(),
        }
    }

    fn allowed_nodes(&self) -> Vec<Node> {
        let mut nodes = vec![
            Node::Item {
                id: "agent:back".into(),
                label: "← Back to the chat".into(),
                tone: Tone::Muted,
            },
            Node::Heading(format!("Allowed in {}", self.project)),
        ];
        if self.agents.allowed.is_empty() {
            nodes.push(Node::Styled(
                "Nothing yet. Always Allow on a permission card adds to this list.".into(),
                Tone::Muted,
            ));
        }
        for (n, rule) in self.agents.allowed.iter().enumerate() {
            let (key, what) = rule.split_once('|').unwrap_or((rule, ""));
            let spec = self.spec(key);
            let agent = spec.map_or(key.to_string(), |s| s.name.clone());
            let icon = spec.map_or('◆', |s| s.icon.chars().next().unwrap_or('◆'));
            let what = match what.strip_prefix("run:") {
                Some(c) => format!("run {c}"),
                None => "edit files".into(),
            };
            nodes.push(Node::entry(crate::panel::Entry {
                id: format!("agent:noop:{n}"),
                label: format!("{agent} · {what}"),
                tone: Tone::Normal,
                icon: Some((icon, crate::style::Hue::Grey)),
                action: Some(('✕', format!("agent:forget:{n}"))),
                ..Default::default()
            }));
        }
        nodes
    }

    /// Whether [`Self::agent_extras`] can add to the panel `name`.
    pub(crate) fn agent_adds_to(&self, name: &str) -> bool {
        name.starts_with(AGENT)
            || self
                .agents
                .specs
                .iter()
                .any(|s| s.view.as_deref() == Some(name))
    }

    /// What merlin adds to a panel as it shows: the slash commands that
    /// match what's typed, in an agent's chat; and in an agent's view of
    /// its own, merlin's cards and the turn's changes.
    pub(crate) fn agent_extras(&self, name: &str, mut panel: Panel) -> Panel {
        if let Some(key) = name.strip_prefix(AGENT) {
            let Some(tab) = self.agents.tabs.iter().find(|t| t.key == key) else {
                return panel;
            };
            let typed = self
                .inputs
                .get(&(name.to_string(), PROMPT.to_string()))
                .map(String::as_str)
                .unwrap_or_default();
            let Some(word) = typed
                .strip_prefix('/')
                .filter(|w| !w.contains(char::is_whitespace))
            else {
                return panel;
            };
            let items: Vec<(String, String, String)> = tab
                .commands
                .iter()
                .filter(|(n, _)| n.starts_with(word))
                // A few at a time: typing more narrows them.
                .take(8)
                .map(|(n, d)| (format!("agent:cmd:{n}"), format!("/{n}"), d.clone()))
                .collect();
            if items.is_empty() {
                return panel;
            }
            if let Some(at) = panel
                .nodes
                .iter()
                .position(|n| matches!(n, Node::Input { .. }))
            {
                let agent = self.agent_name(key);
                panel.nodes.insert(
                    at,
                    Node::Menu {
                        title: format!("{agent}'s commands"),
                        items,
                    },
                );
            }
            return panel;
        }
        // A view an agent uses as its chat.
        let Some(spec) = self
            .agents
            .specs
            .iter()
            .find(|s| s.view.as_deref() == Some(name))
        else {
            return panel;
        };
        let key = spec.key();
        let Some(tab) = self.agents.tabs.iter().find(|t| t.key == key) else {
            return panel;
        };
        let mut extra = Vec::new();
        if let Some(ask) = tab.asks.first() {
            extra.push(Node::Ask(Box::new(self.ask_card(&spec.name, ask, tab))));
        }
        if !tab.changes.is_empty() {
            extra.push(Node::ChangesBar(Box::new(self.changes_bar(tab))));
        }
        if !extra.is_empty() {
            panel.nodes.push(Node::Footer);
            panel.nodes.extend(extra);
        }
        panel
    }

    /// The tab's text box, sent: from `view_submit`.
    pub(crate) fn agent_submit(&mut self, panel: &str, text: &str, fx: &mut Vec<Effect>) -> bool {
        let Some(key) = panel.strip_prefix(AGENT) else {
            return false;
        };
        let key = key.to_string();
        // A slash command half typed: Enter finishes it, as the menu shows.
        if let Some(word) = text
            .strip_prefix('/')
            .filter(|w| !w.contains(char::is_whitespace))
            && let Some(tab) = self.agents.tabs.iter().find(|t| t.key == key)
            && !tab.commands.iter().any(|(n, _)| n == word)
            && let Some((name, _)) = tab.commands.iter().find(|(n, _)| n.starts_with(word))
        {
            let input = (panel.to_string(), PROMPT.to_string());
            let text = format!("/{name} ");
            self.input_cursors.insert(input.clone(), text.len());
            self.inputs.insert(input, text);
            return true;
        }
        self.agent_send(&key, text, fx);
        true
    }

    /// Enter and Esc answer a permission card while one shows.
    pub(crate) fn agent_key(&mut self, key: crate::key::Key, fx: &mut Vec<Effect>) -> bool {
        let Some(tab_key) = self.shown_agent() else {
            return false;
        };
        // Shift Tab in the box: merlin's edit pill, when it shows.
        let back_tab = key.code == crate::key::KeyCode::BackTab
            || (key.code == crate::key::KeyCode::Tab && key.shift);
        if back_tab
            && self
                .focused_input(Side::Right)
                .is_some_and(|(_, id)| id == PROMPT)
        {
            let shown = self
                .agents
                .tabs
                .iter()
                .find(|t| t.key == tab_key)
                .is_some_and(|t| Self::edit_pill(t).is_some());
            if shown {
                if let Some(tab) = self.agents.tabs.iter_mut().find(|t| t.key == tab_key) {
                    tab.auto = !tab.auto;
                }
                self.draw_agent(&tab_key);
                return true;
            }
        }
        // Tab in the box: the agent's next mode.
        if key.code == crate::key::KeyCode::Tab
            && !key.shift
            && !key.ctrl
            && self
                .focused_input(Side::Right)
                .is_some_and(|(_, id)| id == PROMPT)
            && self.next_agent_mode(&tab_key, fx)
        {
            return true;
        }
        let asking = self
            .agents
            .tabs
            .iter()
            .any(|t| t.key == tab_key && !t.asks.is_empty());
        if !asking {
            return false;
        }
        match key.code {
            crate::key::KeyCode::Enter if !key.ctrl => {
                self.agent_answer(&tab_key, AgentAnswer::AllowOnce, fx);
                true
            }
            crate::key::KeyCode::Esc => {
                self.agent_answer(&tab_key, AgentAnswer::Deny, fx);
                true
            }
            _ => false,
        }
    }

    /// The rules loaded for this workspace.
    pub(crate) fn set_agent_allowed(&mut self, rules: Vec<String>) {
        self.agents.allowed = rules;
        for key in self
            .agents
            .tabs
            .iter()
            .map(|t| t.key.clone())
            .collect::<Vec<_>>()
        {
            self.draw_agent(&key);
        }
    }
}

/// The + menu's items, one per agent.
const MODE_ITEMS: &[&str] = &[
    "agent.mode.0",
    "agent.mode.1",
    "agent.mode.2",
    "agent.mode.3",
    "agent.mode.4",
    "agent.mode.5",
    "agent.mode.6",
    "agent.mode.7",
];

/// What a step that's running is doing, as "Reading".
fn running_verb(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "Reading",
        ToolKind::Edit => "Editing",
        ToolKind::Run => "Running",
        ToolKind::Search => "Searching",
        ToolKind::Other => "Working on",
    }
}

/// 950, 12k, 1M: a token count, short.
fn short_tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.0}k", n as f64 / 1_000.0),
        _ => format!("{}M", (n as f64 / 1_000_000.0 * 10.0).round() / 10.0),
    }
}

/// Whether a command line only reads: each part one of a few commands
/// that don't change anything, and nothing written to a file.
pub fn read_only_command(command: &str) -> bool {
    let cleaned = command.replace("2>/dev/null", "").replace("2>&1", "");
    if cleaned.contains('>') || cleaned.contains('`') || cleaned.contains("$(") {
        return false;
    }
    cleaned
        .split(['|', ';', '&'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .all(|part| {
            let mut words = part.split_whitespace();
            let first = words.next().unwrap_or_default();
            match first {
                "ls" | "cat" | "head" | "tail" | "grep" | "rg" | "wc" | "pwd" | "tree" | "file"
                | "stat" | "du" | "which" => true,
                "find" => !part.contains("-exec") && !part.contains("-delete"),
                "git" => matches!(
                    words.next(),
                    Some("log" | "status" | "diff" | "show" | "branch" | "blame" | "ls-files")
                ),
                _ => false,
            }
        })
}

const AGENT_ITEMS: &[&str] = &[
    "agent.menu.0",
    "agent.menu.1",
    "agent.menu.2",
    "agent.menu.3",
    "agent.menu.4",
    "agent.menu.5",
    "agent.menu.6",
    "agent.menu.7",
];

fn button(id: &str, label: &str) -> Button {
    Button {
        id: id.into(),
        label: label.into(),
    }
}

/// `before` to `after` as a unified diff, for the permission card.
fn diff_text(before: &str, after: &str) -> String {
    similar::TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(0)
        .to_string()
}

/// A path in the home folder as `~/…`.
fn home_short(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// A folder for `cd`, quoted for the shell.
fn shell_quote(text: &str) -> String {
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c))
    {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// A plugin's pieces in its agent's chat, which can't use merlin's ids:
/// one starting `agent:` would answer merlin's cards. It isn't clickable.
fn plugin_pieces(nodes: Vec<Node>) -> Vec<Node> {
    let mut panel = Panel {
        title: String::new(),
        nodes,
    };
    panel.ids_mut(|id| {
        if id.starts_with("agent:") {
            id.clear();
        }
    });
    panel.nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_added_and_removed_are_counted() {
        assert_eq!(line_counts("a\nb\nc\n", "a\nB\nc\nd\n"), (2, 1));
        assert_eq!(
            command_rule("cargo test -p merlin-app palette"),
            "cargo test"
        );
        assert_eq!(shell_quote("my dir"), "'my dir'");
    }
}
