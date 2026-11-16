//! Agents in the chat: a plugin's agent says things and asks; merlin draws
//! the chat, asks the user, makes edits through the editor, and reviews
//! what a turn changed (docs/mockups/Agents.html).

use std::path::PathBuf;

use merlin_core::script::Session;
use merlin_core::{
    AgentAnswer, AgentAsks, AgentCall, AgentContext, AgentMode, AgentOutcome, AgentPart,
    AgentRequest, AgentSpec, AgentTool, AgentUpdate, Button, Chip, Effect, Event, Node, Place,
    PlanStep, StepState, Tone, ToolKind, ViewSpec,
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
    assert!(shown.contains("plan 1 of 2: Fix it"), "{shown}");
    assert!(shown.contains("It clears the cache too early."), "{shown}");
    // Steps that are done fold to a line; a click opens them.
    assert!(shown.contains("▸ 1 step · edited a file · 1 s"), "{shown}");
    assert!(!shown.contains("Edited `src/main.rs`"), "{shown}");
    s.click_entry("1 step", 1);
    let shown = chat(&s);
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
        shown.contains("1 file changed +1 −3 [Review] [Undo All] [Keep All]"),
        "{shown}"
    );
    s.click_entry("1 file changed", 1);
    assert!(
        chat(&s).contains("| src/main.rs +1 −3 [Undo] [Keep]"),
        "{}",
        chat(&s)
    );
    s.click_entry("src/main.rs Undo", 1);
    assert_eq!(s.text(), TEXT);
    assert!(!chat(&s).contains("file changed"));
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
        shown.contains("1 file changed +1 −3 [Review] [Undo All] [Keep All]"),
        "{shown}"
    );
    s.click_entry("Undo All", 1);
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
        shown.contains("box: (⏸ Ask before edits ▾) (Gemini 3 Pro ▾) (Sneaky) [■]"),
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

fn mode(id: &str, asks: AgentAsks) -> AgentMode {
    AgentMode {
        id: id.into(),
        name: id[..1].to_uppercase() + &id[1..],
        colour: String::new(),
        asks,
    }
}

fn with_modes(s: &mut Session) {
    say(
        s,
        AgentUpdate::Modes {
            modes: vec![
                mode("build", AgentAsks::Unsaid),
                mode("plan", AgentAsks::ReadsOnly),
                mode("yolo", AgentAsks::Never),
            ],
            current: "build".into(),
        },
    );
}

#[test]
fn tab_in_the_box_moves_through_the_agents_modes() {
    let mut s = session();
    send(&mut s, "go");
    with_modes(&mut s);
    say(
        &mut s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Finished,
            message: None,
        },
    );
    calls(&mut s);
    s.run("key tab");
    let c = calls(&mut s);
    assert!(
        matches!(&c[..], [AgentCall::Mode { chat: 1, mode }] if mode == "plan"),
        "{c:?}"
    );
    // The next never asks: merlin asks first, once.
    s.run("key tab");
    assert!(calls(&mut s).is_empty());
    let shown = chat(&s);
    assert!(shown.contains("to switch to Yolo"), "{shown}");
    s.click_entry("Allow Once", 1);
    let c = calls(&mut s);
    assert!(
        matches!(&c[..], [AgentCall::Mode { mode, .. }] if mode == "yolo"),
        "{c:?}"
    );
}

#[test]
fn leaving_plan_asks_with_the_modes_to_go_on_in() {
    let mut s = session();
    send(&mut s, "plan it");
    with_modes(&mut s);
    say(
        &mut s,
        AgentUpdate::Modes {
            modes: vec![
                mode("build", AgentAsks::Unsaid),
                mode("plan", AgentAsks::ReadsOnly),
            ],
            current: "plan".into(),
        },
    );
    calls(&mut s);
    say(
        &mut s,
        AgentUpdate::Ask {
            request: 4,
            what: AgentRequest::Mode(vec!["build".into()]),
        },
    );
    let shown = chat(&s);
    assert!(
        shown.contains("ask: Gemini's plan is ready. Go on? [Switch to Build] [Keep Plan]"),
        "{shown}"
    );
    s.click_entry("Switch to Build", 1);
    let c = calls(&mut s);
    assert!(
        matches!(&c[..], [AgentCall::Answer { request: 4, answer: AgentAnswer::Mode(m), .. }] if m == "build"),
        "{c:?}"
    );
}

#[test]
fn the_box_shows_the_mode_the_edit_pill_and_how_full_the_context_is() {
    let mut s = session();
    send(&mut s, "go");
    with_modes(&mut s);
    say(
        &mut s,
        AgentUpdate::Usage {
            used: 380_000,
            size: 1_000_000,
            note: "12.4k tokens · $0.03".into(),
        },
    );
    say(
        &mut s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Finished,
            message: None,
        },
    );
    let shown = chat(&s);
    assert!(
        shown.contains("box: [BUILD] (⏸ Ask before edits ▾) ◔38% [↑]"),
        "{shown}"
    );
    // Shift Tab: edits go through without a card.
    s.run("key shift+tab");
    assert!(
        chat(&s).contains("(⏵ Edit automatically ▾)"),
        "{}",
        chat(&s)
    );
    calls(&mut s);
    send(&mut s, "again");
    calls(&mut s);
    say(
        &mut s,
        AgentUpdate::Ask {
            request: 9,
            what: AgentRequest::Edit {
                path: "src/main.rs".into(),
                diff: None,
            },
        },
    );
    let c = calls(&mut s);
    assert!(
        matches!(&c[..], [AgentCall::Answer { request: 9, .. }]),
        "{c:?}"
    );
    // In a read-only mode the pill is about commands.
    s.run("key tab");
    assert!(
        chat(&s).contains("[PLAN] (⏵ Run read-only commands ▾)"),
        "{}",
        chat(&s)
    );
}

#[test]
fn read_only_commands_are_told_apart() {
    use merlin_core::editor::read_only_command as ro;
    assert!(ro("ls -la src"));
    assert!(ro("git log --oneline -20 2>/dev/null | head"));
    assert!(ro("grep -rn foo . && wc -l x"));
    assert!(!ro("git commit -m x"));
    assert!(!ro("ls > out.txt"));
    assert!(!ro("find . -delete"));
    assert!(!ro("cat $(rm -rf x)"));
    assert!(!ro("cargo test"));
}

#[test]
fn a_message_sent_while_it_works_waits_and_goes_next() {
    let mut s = session();
    send(&mut s, "first");
    calls(&mut s);
    s.run("paste and then this; key enter");
    assert!(calls(&mut s).is_empty());
    let shown = chat(&s);
    assert!(shown.contains("next: and then this"), "{shown}");
    say(
        &mut s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Finished,
            message: None,
        },
    );
    let c = calls(&mut s);
    assert!(
        matches!(&c[..], [AgentCall::Prompt { text, .. }] if text == "and then this"),
        "{c:?}"
    );
    assert!(!chat(&s).contains("next:"));
}

#[test]
fn keys_answer_a_card_and_words_instead_deny_it_and_go_next() {
    let mut s = session();
    send(&mut s, "go");
    calls(&mut s);
    say(
        &mut s,
        AgentUpdate::Ask {
            request: 5,
            what: AgentRequest::Run("cargo test".into()),
        },
    );
    s.run("key 3");
    let c = calls(&mut s);
    assert!(
        matches!(
            &c[..],
            [AgentCall::Answer {
                request: 5,
                answer: AgentAnswer::Deny,
                ..
            }]
        ),
        "{c:?}"
    );
    say(
        &mut s,
        AgentUpdate::Ask {
            request: 6,
            what: AgentRequest::Run("cargo test".into()),
        },
    );
    assert!(
        chat(&s).contains("Or tell it what to do instead"),
        "{}",
        chat(&s)
    );
    s.click_entry("Allow Once", 1);
    calls(&mut s);
    // A mode that never asks says so above the box.
    with_modes(&mut s);
    say(
        &mut s,
        AgentUpdate::Modes {
            modes: vec![mode("yolo", AgentAsks::Never)],
            current: "yolo".into(),
        },
    );
    assert!(
        chat(&s).contains("acts without asking in Yolo"),
        "{}",
        chat(&s)
    );
}

/// The agent changes two places in the file: the first line, and adds one
/// at the end.
fn two_changes(s: &mut Session) {
    send(s, "go");
    say(
        s,
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
        s,
        AgentUpdate::Write {
            request: 2,
            path: "src/main.rs".into(),
            text: "fn start() {\n    println!(\"hi\");\n}\n// checked\n".into(),
        },
    );
    say(
        s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Finished,
            message: None,
        },
    );
}

#[test]
fn each_change_in_the_editor_has_undo_and_keep() {
    use merlin_core::UiEvent;
    let mut s = session();
    two_changes(&mut s);
    let view = s.editor.text_views().remove(0);
    let buttons: Vec<usize> = view
        .agent_hunks
        .iter()
        .filter_map(|h| h.buttons.as_ref().map(|b| b.0))
        .collect();
    assert_eq!(buttons, [0, 1], "{:?}", view.agent_hunks);
    assert!(
        view.agent_hunks
            .iter()
            .any(|h| h.removed == ["fn main() {"]),
        "{:?}",
        view.agent_hunks
    );
    assert_eq!(view.agent_bar.as_ref().map(|b| b.count), Some(2));
    // Undo the first: it's put back; the second stays to review.
    s.send(Event::Ui(UiEvent::AgentHunk {
        index: 0,
        keep: false,
    }));
    assert_eq!(
        s.text(),
        "fn main() {\n    println!(\"hi\");\n}\n// checked\n"
    );
    assert!(chat(&s).contains("1 file changed +1 −0"), "{}", chat(&s));
    // Keep the other: nothing's left to review.
    s.send(Event::Ui(UiEvent::AgentHunk {
        index: 0,
        keep: true,
    }));
    assert!(!chat(&s).contains("file changed"), "{}", chat(&s));
    assert!(s.editor.text_views()[0].agent_bar.is_none());
}

#[test]
fn editing_a_message_rewinds_the_files_and_starts_again_from_it() {
    let mut s = session();
    two_changes(&mut s);
    assert_ne!(s.text(), TEXT);
    calls(&mut s);
    s.click_entry("go Edit", 1);
    // The files are as they were before it, and the chat starts again.
    assert_eq!(s.text(), TEXT);
    let c = calls(&mut s);
    assert!(matches!(&c[..], [AgentCall::Close { chat: 1 }]), "{c:?}");
    let shown = chat(&s);
    assert!(!shown.contains("file changed"), "{shown}");
    assert!(shown.contains("[go]") || shown.contains("go"), "{shown}");
    // Sent again, it goes to a new chat.
    s.run("key enter");
    let c = calls(&mut s);
    assert!(
        matches!(&c[..], [AgentCall::Prompt { chat: 2, text, .. }] if text == "go"),
        "{c:?}"
    );
}

#[test]
fn at_lists_files_and_what_is_picked_goes_with_the_message() {
    let mut s = session();
    // @ asks for the project's files, which the session answers.
    s.run("cmd agent.open; paste look at; key space; key @; paste mai");
    let shown = chat(&s);
    assert!(
        shown.contains("menu: Add to the message | 📄 main.rs — src"),
        "{shown}"
    );
    s.run("key enter");
    s.run("paste please; key enter");
    let c = calls(&mut s);
    assert!(
        matches!(&c[..], [AgentCall::Prompt { text, context, .. }]
            if text == "look at @src/main.rs please"
                && context.contains(&AgentContext::File("src/main.rs".into()))),
        "{c:?}"
    );
}

#[test]
fn a_finished_chat_is_kept_and_listed_under_chats() {
    let mut s = session();
    send(&mut s, "Fix the flicker");
    s.effects.clear();
    say(
        &mut s,
        AgentUpdate::Done {
            outcome: AgentOutcome::Finished,
            message: None,
        },
    );
    let kept = s.effects.iter().find_map(|e| match e {
        Effect::SaveAgentChats(c) => Some(c.clone()),
        _ => None,
    });
    let kept = kept.expect("the chat is kept");
    assert_eq!(kept[0].title, "Fix the flicker");
    // Loaded in another window, it's listed, and opens again.
    let mut other = session();
    other.send(Event::AgentChats(kept));
    other.run("cmd agent.open; cmd agent.history");
    let dump = other.editor.dump();
    assert!(dump.contains("Fix the flicker"), "{dump}");
    other.run("key down; key enter");
    let shown = chat(&other);
    assert!(shown.contains("Fix the flicker"), "{shown}");
    assert!(shown.contains("doesn't see what's above"), "{shown}");
}
