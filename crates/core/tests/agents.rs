//! Agents in the chat: a plugin's agent says things and asks; merlin draws
//! the chat, asks the user, makes edits through the editor, and reviews
//! what a turn changed (docs/mockups/Agents.html).

use std::path::PathBuf;

use merlin_core::script::Session;
use merlin_core::{
    AgentAnswer, AgentCall, AgentContext, AgentOutcome, AgentPart, AgentRequest, AgentSpec,
    AgentTool, AgentUpdate, Button, Chip, Effect, Event, Node, Place, PlanStep, StepState, Tone,
    ToolKind, ViewSpec,
};

const TEXT: &str = "fn main() {\n    println!(\"hi\");\n}\n";

fn session() -> Session {
    let mut s = Session::new(TEXT, Some("src/main.rs"), 120, 40);
    s.files.insert("src/main.rs".into(), s.text());
    s.send(Event::RegisterAgents {
        plugin: "acp".into(),
        agents: vec![AgentSpec {
            plugin: "acp".into(),
            id: "gemini".into(),
            name: "Gemini".into(),
            icon: "◆".into(),
            missing: None,
            view: None,
        }],
    });
    s
}

fn say(s: &mut Session, update: AgentUpdate) {
    s.send(Event::Agent {
        plugin: "acp".into(),
        chat: 1,
        update,
        at: 0,
    });
}

/// The calls into the plugin since last asked.
fn calls(s: &mut Session) -> Vec<AgentCall> {
    s.effects
        .drain(..)
        .filter_map(|e| match e {
            Effect::Agent { call, .. } => Some(call),
            _ => None,
        })
        .collect()
}

/// The right side bar's lines in the dump.
fn chat(s: &Session) -> String {
    let dump = s.editor.dump();
    dump.lines()
        .skip_while(|l| !l.contains("right:"))
        .take_while(|l| !l.starts_with("── ") || l.contains("right:"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Opens the chat and sends `text`.
fn send(s: &mut Session, text: &str) {
    s.run(&format!("cmd agent.open; paste {text}; key ctrl+enter"));
}

#[test]
fn the_first_agent_gets_a_tab_and_a_message_goes_to_it() {
    let mut s = session();
    send(&mut s, "Fix the flicker");
    let c = calls(&mut s);
    assert!(
        matches!(&c[..], [AgentCall::Prompt { chat: 1, agent, text, context }]
            if agent == "gemini" && text == "Fix the flicker"
                && context == &[AgentContext::File("src/main.rs".into())]),
        "{c:?}"
    );
    let shown = chat(&s);
    assert!(shown.contains("◆ Gemini"), "{shown}");
    assert!(shown.contains("Fix the flicker"), "{shown}");
}

#[test]
fn a_turn_shows_its_plan_its_steps_and_its_text() {
    let mut s = session();
    send(&mut s, "go");
    say(
        &mut s,
        AgentUpdate::Say(AgentPart::Plan(vec![
            PlanStep {
                text: "Read it".into(),
                state: StepState::Done,
            },
            PlanStep {
                text: "Fix it".into(),
                state: StepState::Running,
            },
        ])),
    );
    say(
        &mut s,
        AgentUpdate::Say(AgentPart::Text("It clears the cache ".into())),
    );
    say(
        &mut s,
        AgentUpdate::Say(AgentPart::Text("too early.".into())),
    );
    say(
        &mut s,
        AgentUpdate::Say(AgentPart::Tool(AgentTool {
            id: "1".into(),
            kind: ToolKind::Edit,
            title: "src/main.rs".into(),
            state: StepState::Done,
            added: 5,
            removed: 6,
            run: None,
        })),
    );
    let shown = chat(&s);
    assert!(shown.contains("✓ Read it · ◌ Fix it"), "{shown}");
    assert!(shown.contains("It clears the cache too early."), "{shown}");
    assert!(shown.contains("✓ Edited `src/main.rs` +5 −6"), "{shown}");
    assert!(shown.contains("Gemini is working…"), "{shown}");
}

#[test]
fn a_permission_card_answers_the_plugin() {
    let mut s = session();
    s.run("cmd agent.open");
    say(
        &mut s,
        AgentUpdate::Ask {
            request: 7,
            what: AgentRequest::Run("cargo test -p app palette".into()),
        },
    );
    let shown = chat(&s);
    assert!(
        shown.contains("ask: Gemini wants to run a command | cargo test -p app palette"),
        "{shown}"
    );
    assert!(
        shown.contains("[Allow Once] [Always Allow `cargo test`] [Deny]"),
        "{shown}"
    );
    s.click_entry("Always Allow", 1);
    let c = calls(&mut s);
    assert!(
        matches!(
            &c[..],
            [AgentCall::Answer {
                request: 7,
                answer: AgentAnswer::AlwaysAllow,
                ..
            }]
        ),
        "{c:?}"
    );
    // The same command is allowed from now on without a card.
    say(
        &mut s,
        AgentUpdate::Ask {
            request: 8,
            what: AgentRequest::Run("cargo test --all".into()),
        },
    );
    let c = calls(&mut s);
    assert!(
        matches!(
            &c[..],
            [AgentCall::Answer {
                request: 8,
                answer: AgentAnswer::AlwaysAllow,
                ..
            }]
        ),
        "{c:?}"
    );
    assert!(!chat(&s).contains("ask:"));
}

#[test]
fn an_edit_it_didnt_ask_about_waits_and_then_goes_through_the_editor() {
    let mut s = session();
    s.run("cmd agent.open");
    let new = "fn main() {\n    println!(\"hello\");\n}\n";
    say(
        &mut s,
        AgentUpdate::Write {
            request: 3,
            path: "src/main.rs".into(),
            text: new.into(),
        },
    );
    let shown = chat(&s);
    assert!(
        shown.contains("ask: Gemini is changing `src/main.rs` without having asked"),
        "{shown}"
    );
    assert!(shown.contains("| +    println!(\"hello\");"), "{shown}");
    assert_eq!(s.text(), TEXT, "nothing changes before the answer");
    s.click_entry("Allow Once", 1);
    assert_eq!(s.text(), new);
    let c = calls(&mut s);
    assert!(
        matches!(
            &c[..],
            [AgentCall::File {
                request: 3,
                result: Ok(_),
                ..
            }]
        ),
        "{c:?}"
    );
    // It's one edit, which Undo takes back.
    s.run("cmd edit.undo");
    assert_eq!(s.text(), TEXT);
}

#[test]
fn reading_outside_the_workspace_asks_and_deny_says_so() {
    let mut s = session();
    s.editor.set_root(PathBuf::from("/work/proj"));
    s.run("cmd agent.open");
    say(
        &mut s,
        AgentUpdate::Read {
            request: 4,
            path: "/home/x/.cargo/config.toml".into(),
        },
    );
    let shown = chat(&s);
    assert!(
        shown.contains("! ask: Gemini wants to read a file outside this workspace"),
        "{shown}"
    );
    s.click_entry("Deny", 1);
    let c = calls(&mut s);
    assert!(
        matches!(
            &c[..],
            [AgentCall::File {
                request: 4,
                result: Err(_),
                ..
            }]
        ),
        "{c:?}"
    );
}

#[test]
fn a_finished_turn_lists_its_changes_and_revert_puts_them_back() {
    let mut s = session();
    send(&mut s, "go");
    say(
        &mut s,
        AgentUpdate::Ask {
            request: 1,
            what: AgentRequest::Edit {
                path: "src/main.rs".into(),
                diff: None,
            },
        },
    );
    s.click_entry("Allow Once", 1);
    say(
        &mut s,
        AgentUpdate::Write {
            request: 2,
            path: "src/main.rs".into(),
            text: "fn main() {}\n".into(),
        },
    );
    assert_eq!(s.text(), "fn main() {}\n", "allowed, so no second card");
    say(
        &mut s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Finished,
            message: None,
        },
    );
    let shown = chat(&s);
    assert!(
        shown.contains("Changed 1 file [Revert All] | src/main.rs +1 −3"),
        "{shown}"
    );
    s.click_entry("src/main.rs Revert", 1);
    assert_eq!(s.text(), TEXT);
    assert!(!chat(&s).contains("Changed 1 file"));
}

#[test]
fn stopped_and_failed_say_so() {
    let mut s = session();
    send(&mut s, "go");
    calls(&mut s);
    s.click_entry("Stop", 1);
    let c = calls(&mut s);
    assert!(matches!(&c[..], [AgentCall::Cancel { chat: 1 }]), "{c:?}");
    say(
        &mut s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Cancelled,
            message: None,
        },
    );
    assert!(chat(&s).contains("Stopped. 0 files changed so far."));
    say(
        &mut s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Failed,
            message: Some("API key not valid".into()),
        },
    );
    assert!(chat(&s).contains("Gemini stopped: \"API key not valid\"."));
}

#[test]
fn an_edit_to_a_file_that_isnt_open_opens_it_and_goes_through_the_editor() {
    let mut s = session();
    s.run("cmd agent.open");
    say(
        &mut s,
        AgentUpdate::Ask {
            request: 1,
            what: AgentRequest::Edit {
                path: "src/lib.rs".into(),
                diff: None,
            },
        },
    );
    s.click_entry("Allow Once", 1);
    calls(&mut s);
    say(
        &mut s,
        AgentUpdate::Write {
            request: 2,
            path: "src/lib.rs".into(),
            text: "pub fn two() -> u8 { 2 }\n".into(),
        },
    );
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::Open(p) if p == &PathBuf::from("src/lib.rs")))
    );
    assert!(calls(&mut s).is_empty(), "it waits for the tab");
    s.send(Event::Opened {
        path: "src/lib.rs".into(),
        text: "pub fn one() -> u8 { 1 }\n".into(),
    });
    assert_eq!(s.text(), "pub fn two() -> u8 { 2 }\n");
    let c = calls(&mut s);
    assert!(
        matches!(
            &c[..],
            [AgentCall::File {
                request: 2,
                result: Ok(_),
                ..
            }]
        ),
        "{c:?}"
    );
    s.run("cmd edit.undo");
    assert_eq!(s.text(), "pub fn one() -> u8 { 1 }\n");
}

#[test]
fn a_file_the_agent_writes_itself_is_still_reviewed() {
    let mut s = session();
    send(&mut s, "go");
    say(
        &mut s,
        AgentUpdate::Say(AgentPart::Tool(AgentTool {
            id: "e".into(),
            kind: ToolKind::Edit,
            title: "src/main.rs".into(),
            state: StepState::Running,
            added: 0,
            removed: 0,
            run: None,
        })),
    );
    s.send(Event::FileChanged {
        path: "src/main.rs".into(),
        text: "fn main() {}\n".into(),
    });
    say(
        &mut s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Finished,
            message: None,
        },
    );
    let shown = chat(&s);
    assert!(
        shown.contains("Changed 1 file [Revert All] | src/main.rs +1 −3"),
        "{shown}"
    );
    s.click_entry("src/main.rs Revert", 1);
    assert_eq!(s.text(), TEXT);
}

fn chip(id: &str, label: &str) -> Chip {
    Chip {
        id: id.into(),
        label: label.into(),
        tone: Tone::Normal,
    }
}

#[test]
fn the_agents_header_and_its_own_pieces_show_and_their_clicks_go_to_it() {
    let mut s = session();
    send(&mut s, "go");
    say(
        &mut s,
        AgentUpdate::Header {
            chips: vec![
                chip("acp:model:1", "Gemini 3 Pro ▾"),
                chip("agent:allow", "Sneaky"),
            ],
            note: "12.4k tokens · $0.03".into(),
        },
    );
    say(
        &mut s,
        AgentUpdate::Say(AgentPart::Panel(vec![Node::Buttons(vec![
            Button {
                id: "update-all".into(),
                label: "Update All 3".into(),
            },
            Button {
                id: "agent:allow".into(),
                label: "Fake Allow".into(),
            },
        ])])),
    );
    let shown = chat(&s);
    assert!(
        shown.contains("bar: (Gemini 3 Pro ▾) (Sneaky) 12.4k tokens · $0.03"),
        "{shown}"
    );
    assert!(shown.contains("Update All 3"), "{shown}");
    calls(&mut s);
    s.effects.clear();
    s.click_entry("Update All 3", 1);
    assert!(
        s.effects.iter().any(|e| matches!(e,
            Effect::PluginClick { plugin, button } if plugin == "acp" && button == "update-all")),
        "{:?}",
        s.effects
    );
    // merlin's ids are kept out of the plugin's pieces.
    s.effects.clear();
    s.click_entry("Fake Allow", 1);
    assert!(calls(&mut s).is_empty());
}

#[test]
fn typing_a_slash_lists_the_agents_commands_and_enter_finishes_one() {
    let mut s = session();
    send(&mut s, "go");
    say(
        &mut s,
        AgentUpdate::Commands(vec![
            ("review".into(), "Review the changes on this branch".into()),
            ("plan".into(), "Plan before changing anything".into()),
        ]),
    );
    say(
        &mut s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Finished,
            message: None,
        },
    );
    calls(&mut s);
    s.run("paste /re");
    let shown = chat(&s);
    assert!(
        shown.contains("menu: Gemini's commands | /review — Review the changes on this branch"),
        "{shown}"
    );
    assert!(!shown.contains("/plan"), "{shown}");
    // Enter finishes the command rather than sending "/re".
    s.run("key ctrl+enter");
    assert!(calls(&mut s).is_empty());
    s.run("paste the palette; key ctrl+enter");
    let c = calls(&mut s);
    assert!(
        matches!(&c[..], [AgentCall::Prompt { text, .. }] if text == "/review the palette"),
        "{c:?}"
    );
}

#[test]
fn an_agent_with_its_own_view_still_gets_merlins_cards() {
    let mut s = Session::new(TEXT, Some("src/main.rs"), 120, 40);
    s.files.insert("src/main.rs".into(), s.text());
    s.send(Event::RegisterViews {
        plugin: "acp".into(),
        views: vec![ViewSpec {
            id: "manager".into(),
            title: "Gemini Manager".into(),
            icon: None,
            place: Place::Right,
            empty: String::new(),
            toolbar: Vec::new(),
        }],
    });
    s.send(Event::RegisterAgents {
        plugin: "acp".into(),
        agents: vec![AgentSpec {
            plugin: "acp".into(),
            id: "gemini".into(),
            name: "Gemini".into(),
            icon: "◆".into(),
            missing: None,
            view: Some("acp/manager".into()),
        }],
    });
    s.send(Event::SetPanel {
        plugin: "acp/manager".into(),
        panel: Some(merlin_core::Panel {
            title: "Gemini Manager".into(),
            nodes: vec![
                Node::Text("Fix the palette flicker".into()),
                Node::Buttons(vec![Button {
                    id: "agent:allow".into(),
                    label: "Fake Allow".into(),
                }]),
            ],
        }),
    });
    s.run("cmd agent.open");
    say(
        &mut s,
        AgentUpdate::Ask {
            request: 3,
            what: AgentRequest::Run("cargo test".into()),
        },
    );
    let dump = s.editor.dump();
    assert!(dump.contains("Fix the palette flicker"), "{dump}");
    assert!(
        dump.contains("ask: Gemini wants to run a command | cargo test"),
        "{dump}"
    );
    // The plugin's own button named as merlin's is the plugin's click.
    s.effects.clear();
    s.click_entry("Fake Allow", 1);
    assert!(calls(&mut s).is_empty(), "{:?}", s.effects);
    s.click_entry("Allow Once", 1);
    let c = calls(&mut s);
    assert!(
        matches!(
            &c[..],
            [AgentCall::Answer {
                request: 3,
                answer: AgentAnswer::AllowOnce,
                ..
            }]
        ),
        "{c:?}"
    );
}
