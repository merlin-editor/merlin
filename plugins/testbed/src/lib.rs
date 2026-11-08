//! Exercises the plugin API for the host's tests: secrets, state kept
//! across a reload, processes, buffer events and diffs to review.

use merlin_plugin::prelude::*;
use merlin_plugin::{
    Colour, DrawCurve, DrawItem, DrawLine, DrawPoint, DrawRect, DrawText, Drawing, Shape,
};

#[derive(Default)]
struct Testbed {
    clicks: u32,
    cat: Option<u32>,
    log: Vec<String>,
    asked: Vec<String>,
    /// Whether the Counters row is open.
    open: bool,
    /// The count its chart was last drawn for.
    plotted: Option<u32>,
    /// Whether its graph's dot moves, and how far it has.
    animating: bool,
    frame: u32,
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
            "bad-image",
            "animate",
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
            "animate" => self.animating = !self.animating,
            "bad-image" => {
                if let Err(err) = host::set_image("bad", Some(&Picture::Png(vec![1, 2, 3]))) {
                    host::notify(&err);
                }
            }
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
                    .node(Node::progress("Counted", Some(self.clicks.min(10) as f32 / 10.0)))
                    // Never handed over: it waits, saying what it'll be.
                    .node(Node::image("last-run", "Plot of the last run")),
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
    fn activity(&mut self) -> Panel {
        if self.plotted != Some(self.clicks) {
            let _ = host::set_image("chart", Some(&chart(self.clicks)));
            self.plotted = Some(self.clicks);
        }
        let log = format!("log: {}", self.log.join(", "));
        let mut panel = Panel::new("Testbed")
            .text(format!("clicks: {}", self.clicks))
            .text(log)
            .node(Node::image("chart", "Clicks so far"))
            .node(Node::Drawing(self.graph()))
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

impl Testbed {
    /// A small commit graph: a line of commits, a branch off it on a
    /// curve, their names, and, while animating, a dot along the line.
    fn graph(&mut self) -> Drawing {
        let theme = |name: &str| Colour::Theme(name.into());
        let point = |x, y| DrawPoint { x, y };
        let mut items = vec![
            DrawItem {
                id: String::new(),
                shape: Shape::Line(DrawLine {
                    start: point(20.0, 20.0),
                    end: point(20.0, 100.0),
                    width: 2.0,
                    colour: theme("accent"),
                }),
            },
            DrawItem {
                id: String::new(),
                shape: Shape::Curve(DrawCurve {
                    points: vec![
                        point(20.0, 40.0),
                        point(20.0, 60.0),
                        point(60.0, 50.0),
                        point(60.0, 80.0),
                    ],
                    width: 2.0,
                    colour: theme("success"),
                }),
            },
        ];
        for (i, (x, y, name)) in [(20.0, 20.0, "main"), (20.0, 100.0, "first"), (60.0, 80.0, "branch")]
            .into_iter()
            .enumerate()
        {
            items.push(DrawItem {
                id: format!("commit-{i}"),
                shape: Shape::Rect(DrawRect {
                    x: x - 6.0,
                    y: y - 6.0,
                    width: 12.0,
                    height: 12.0,
                    radius: 6.0,
                    fill: Some(theme("island")),
                    stroke: Some(theme(if i == 2 { "success" } else { "accent" })),
                }),
            });
            items.push(DrawItem {
                id: String::new(),
                shape: Shape::Text(DrawText {
                    x: x + 12.0,
                    y: y - 8.0,
                    text: name.into(),
                    size: 12.0,
                    colour: theme("text"),
                    mono: i == 2,
                }),
            });
        }
        if self.animating {
            self.frame += 1;
            let y = 20.0 + (self.frame % 80) as f32;
            items.push(DrawItem {
                id: String::new(),
                shape: Shape::Rect(DrawRect {
                    x: 16.0,
                    y: y - 4.0,
                    width: 8.0,
                    height: 8.0,
                    radius: 4.0,
                    fill: Some(Colour::Rgb(0xe6b45c)),
                    stroke: None,
                }),
            });
            host::request_frame();
        }
        Drawing {
            width: 240.0,
            height: 120.0,
            items,
        }
    }
}

/// A bar for each of the last eight counts up to `clicks`, as RGBA: blue
/// bars on nothing, so the view's own colour shows between them.
fn chart(clicks: u32) -> Picture {
    let (w, h) = (240u32, 96u32);
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for bar in 0..8u32 {
        let value = (clicks + bar) % 9 + 1;
        let top = h - value * h / 10;
        let x0 = 6 + bar * 29;
        for y in top..h {
            for x in x0..x0 + 21 {
                let i = ((y * w + x) * 4) as usize;
                pixels[i..i + 4].copy_from_slice(&[0x7c, 0x9c, 0xff, 0xff]);
            }
        }
    }
    Picture::rgba(w, h, pixels)
}

merlin_plugin::export!(Testbed);
