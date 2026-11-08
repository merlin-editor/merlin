//! Exercises the plugin API for the host's tests: secrets, state kept
//! across a reload, processes, buffer events and diffs to review.

use merlin_plugin::prelude::*;

#[derive(Default)]
struct Testbed {
    clicks: u32,
    cat: Option<u32>,
    log: Vec<String>,
    asked: Vec<String>,
    /// Whether the Counters row is open.
    open: bool,
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
            "show-path",
            "show-chat",
            "about",
            "slow",
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
            "count" => {
                self.clicks += 1;
                host::set_view_badge("activity", self.clicks);
            }
            "show-chat" => host::focus_view("chat"),
            // Busy for a while, as rendering a page of a PDF can be.
            "slow" => {
                let mut x = 0u64;
                for i in 0..std::hint::black_box(40_000_000u64) {
                    x = x.wrapping_mul(31).wrapping_add(i);
                }
                self.clicks += 1;
                host::notify(&format!("slow done {}", std::hint::black_box(x) > 0));
            }
            "about" => {
                let target = host::menu_target().unwrap_or_else(|| "nothing".into());
                self.asked.push(format!("about {target}"));
                host::focus_view("chat");
            }
            "spawn" => {
                self.cat = host::spawn("cat", &[], &[], Framing::Lines).ok();
                if self.cat.is_some() {
                    host::start_progress("cat", "Running cat", true);
                    host::report_progress("cat", "waiting for input", None);
                }
            }
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
            "show-path" => host::notify(&format!(
                "mode: {:?}, path: {:?}",
                settings::text("mode"),
                settings::text("server-path")
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

    /// Activity, Chat and Runs each show the nodes a view can have.
    fn view(&mut self, id: &str) -> Option<Panel> {
        match id {
            "activity" => Some(self.activity()),
            "chat" => Some(self.chat()),
            "runs" if self.cat.is_some() => Some(
                Panel::new("Runs")
                    .node(Node::progress("cat is running", None))
                    .node(Node::progress("Counted", Some(self.clicks.min(10) as f32 / 10.0))),
            ),
            "runs" => Some(Panel::new("Runs").node(Node::empty(
                "No runs yet",
                "Start cat to see one here.",
                vec![Button::new("spawn", "Start cat")],
            ))),
            _ => None,
        }
    }

    /// Rows, chips and buttons: what they do, or a line in the log.
    fn click(&mut self, button: &str) {
        match button {
            "toggle" => self.open = !self.open,
            "count" | "spawn" => self.run_command(button),
            "reset" => self.clicks = 0,
            other => self.log.push(format!("clicked {other}")),
        }
    }

    fn submit(&mut self, input: &str, text: &str) {
        if input == "ask" {
            self.asked.push(text.to_string());
        }
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

    /// Cancel on cat's progress: it stops.
    fn cancel(&mut self, id: &str) {
        if id == "cat"
            && let Some(cat) = self.cat.take()
        {
            host::kill(cat);
            host::end_progress("cat");
            self.log.push("cat cancelled".into());
        }
    }

    fn setting_changed(&mut self, key: &str) {
        self.log.push(format!("{key} changed"));
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

impl Testbed {
    /// Counts and the log, then a tree with chips and a right-click menu.
    fn activity(&self) -> Panel {
        let log = format!("log: {}", self.log.join(", "));
        let mut panel = Panel::new("Testbed")
            .text(format!("clicks: {}", self.clicks))
            .text(log)
            .row(
                Row::new("toggle", "Counters")
                    .expanded(self.open)
                    .count(self.clicks.to_string()),
            );
        if self.open {
            panel = panel
                .row(
                    Row::new("count", "clicks")
                        .depth(1)
                        .icon("#")
                        .detail(self.clicks.to_string())
                        .chip(Chip::label("→ more", Tone::Warning))
                        .menu(Button::new("count", "Count Again"))
                        .menu(Button::new("reset", "Reset")),
                )
                .row(
                    Row::new("log-row", "log lines")
                        .depth(1)
                        .detail(self.log.len().to_string())
                        .chip(Chip::label("live", Tone::Muted)),
                );
        }
        panel
    }

    /// The conversation: each question and its answer, an edit to review,
    /// then a box to ask in.
    fn chat(&self) -> Panel {
        let mut panel = Panel::new("Chat").text(format!("asked: {}", self.asked.join(", ")));
        for (i, q) in self.asked.iter().enumerate() {
            let last = i + 1 == self.asked.len();
            panel = panel
                .message(
                    Node::message("You", q.clone())
                        .mine()
                        .chip(Chip::new("file", "notes.txt 1–3")),
                )
                .message(
                    Node::message(
                        "Testbed",
                        format!("You asked **{q}**. Here's some code:\n\n```rust\nfn main() {{\n    println!(\"{q}\");\n}}\n```"),
                    )
                    .streaming(last && self.clicks % 2 == 1),
                );
        }
        if !self.asked.is_empty() {
            panel = panel.node(Node::edit(
                "notes.txt",
                1,
                0,
                vec![Button::new("review", "Review"), Button::new("apply", "Apply")],
            ));
        }
        panel
            .footer()
            .node(Node::multiline_input("ask", "Ask something"))
            .node(Node::chips(
                vec![Chip::new("selection", "+ selection"), Chip::new("model", "fast ▾")],
                "Ctrl Enter to send",
            ))
    }
}

merlin_plugin::export!(Testbed);
