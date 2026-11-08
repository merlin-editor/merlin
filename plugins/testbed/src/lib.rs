//! Exercises the plugin API for the host's tests: secrets, state kept
//! across a reload, processes, buffer events and diffs to review.

use merlin_plugin::prelude::*;

#[derive(Default)]
struct Testbed {
    clicks: u32,
    cat: Option<u32>,
    log: Vec<String>,
}

impl Plugin for Testbed {
    fn commands(&self) -> Vec<Command> {
        [
            "count",
            "spawn",
            "send",
            "set-secret",
            "show-secret",
            "delete-secret",
            "propose",
            "spawn-forbidden",
            "show-setting",
            "colour",
            "log",
            "warn",
            "fail",
        ]
        .map(|id| Command::new(id, format!("Testbed: {id}")))
        .into()
    }

    fn run_command(&mut self, id: &str) {
        match id {
            "count" => self.clicks += 1,
            "spawn" => self.cat = host::spawn("cat", &[], &[], Framing::Lines).ok(),
            "send" => {
                if let Some(cat) = self.cat {
                    let _ = host::send(cat, &format!("ping {}", self.clicks));
                }
            }
            "set-secret" => {
                if let Err(err) = host::set_secret("token", "hunter2") {
                    host::notify(&err);
                }
            }
            "show-secret" => host::notify(&format!("secret: {:?}", host::get_secret("token"))),
            "delete-secret" => host::delete_secret("token"),
            "show-setting" => host::notify(&format!(
                "greeting: {:?}, loud: {:?}",
                settings::text("greeting"),
                settings::flag("style.loud")
            )),
            "colour" => {
                let at = |line, column| Position { line, column };
                host::set_tokens(
                    "notes.txt",
                    TokenLayer::Base,
                    None,
                    &[Token {
                        start: at(0, 0),
                        end: at(0, 3),
                        kind: TokenKind::Keyword,
                    }],
                );
                host::set_folds("notes.txt", &[FoldRange { first: 0, last: 2 }]);
            }
            "log" => {
                host::log(LogLevel::Debug, "quiet detail");
                host::log(LogLevel::Warn, "something to see");
            }
            "warn" => host::notice(LogLevel::Warn, "careful", &[]),
            "fail" => host::notice(LogLevel::Error, "it broke", &[]),
            "propose" => host::show_diff("edit-1", "notes.txt", "one\n", "one\ntwo\n"),
            "spawn-forbidden" => {
                if let Err(err) = host::spawn("sh", &[], &[], Framing::Lines) {
                    host::notify(&err);
                }
            }
            _ => {}
        }
    }

    fn panel(&mut self) -> Option<Panel> {
        let log = format!("log: {}", self.log.join(", "));
        Some(
            Panel::new("Testbed")
                .text(format!("clicks: {}", self.clicks))
                .text(log),
        )
    }

    fn process(&mut self, _process: u32, event: ProcessEvent) {
        if let ProcessEvent::Message(line) = event {
            self.log.push(line);
        }
    }

    fn buffer(&mut self, event: BufferEvent) {
        self.log.push(match event {
            BufferEvent::Switched => "switched".into(),
            BufferEvent::Changed => "changed".into(),
            BufferEvent::Saved(path) => format!("saved {path}"),
        });
    }

    fn review(&mut self, id: &str, accepted: bool) {
        let answer = if accepted { "accepted" } else { "rejected" };
        self.log.push(format!("{id} {answer}"));
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        let cat = self.cat.map_or("-".into(), |c| c.to_string());
        Some(format!("{} {cat}", self.clicks).into_bytes())
    }

    fn restore_state(&mut self, state: Vec<u8>) {
        let state = String::from_utf8(state).unwrap_or_default();
        let mut parts = state.split(' ');
        self.clicks = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        self.cat = parts.next().and_then(|n| n.parse().ok());
    }
}

merlin_plugin::export!(Testbed);
