//! Example plugin: counts words, and shows a panel with a button.

use merlin_plugin::prelude::*;

#[derive(Default)]
struct WordCount;

impl Plugin for WordCount {
    fn commands(&self) -> Vec<Command> {
        vec![
            Command::new("show", "Word Count: Show").key("ctrl+alt+w"),
            Command::new("todo", "Word Count: Insert TODO"),
        ]
    }

    fn run_command(&mut self, id: &str) {
        let done = match id {
            "todo" => host::insert_text("TODO: "),
            _ => host::active_text().map(|text| {
                let words = text.split_whitespace().count();
                host::notify(&format!("{words} words, {} lines", text.lines().count()))
            }),
        };
        if let Err(err) = done {
            host::notify(&err);
        }
    }

    fn panel(&mut self) -> Option<Panel> {
        let text = host::active_text().ok()?;
        let words = format!("{} words", text.split_whitespace().count());
        let size = format!(
            "{} lines, {} chars",
            text.lines().count(),
            text.chars().count()
        );
        let panel = Panel::new("Word Count")
            .heading("This file")
            .text(words)
            .text(size);
        Some(panel.divider().button("todo", "Insert TODO"))
    }

    fn click(&mut self, button: &str) {
        self.run_command(button);
    }
}

merlin_plugin::export!(WordCount);
