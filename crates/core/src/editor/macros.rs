//! Macros: keys and commands recorded and played back, with Vim's `q` and
//! `@` or from the palette, and the ones saved in the settings.
//!
//! A macro is written as Vim writes keys, such as `0wcwkept<Esc>j`, so a
//! Vim register holds it as text and `[[macros]]` in the settings can too.
//! A command run from the palette is `<Cmd>id<CR>`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::*;

/// How long a run plays in one go before the window gets to draw.
const SLICE: Duration = Duration::from_millis(20);

/// A macro saved in the settings, as `[[macros]]` has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Macro {
    pub name: String,
    /// What it does, as Vim writes keys.
    pub keys: String,
}

impl Macro {
    /// Its command's id, such as `macro.saved.quote_the_word`.
    pub fn command(&self) -> String {
        let slug: String = self
            .name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        format!("macro.saved.{slug}")
    }
}

/// Recording or playing, for the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroView {
    /// The Vim register, as `@q`, or empty from the palette.
    pub name: String,
    /// Recording: the keys so far, as Vim writes them.
    pub keys: String,
    /// Playing: how many times it has run, and of how many.
    pub progress: Option<(usize, usize)>,
    /// What stops it, such as "q stops".
    pub hint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Key(Key),
    Command(String),
}

#[derive(Debug, Default)]
pub(crate) struct Recording {
    register: Option<char>,
    steps: Vec<Step>,
    /// The key being handled, which a key that stops recording takes out.
    current: Option<usize>,
}

#[derive(Debug)]
pub(crate) struct MacroRun {
    name: String,
    steps: Vec<Step>,
    times: usize,
    done: usize,
    queue: VecDeque<Step>,
    /// The cursors still to play at, the next last.
    carets: Vec<usize>,
    /// Where each cursor's play ended, and the text's length then.
    ends: Vec<usize>,
    len: usize,
    doc: u64,
    undo_from: usize,
    /// The first key that failed: its line, what went wrong, and how many
    /// times the macro had run through before it.
    failed: Option<(usize, String, usize)>,
    /// A key failed, so this cursor's play is over.
    halted: bool,
}

impl Editor {
    /// A key from the user, which a recording takes down and which stops a
    /// run that's playing.
    pub(crate) fn macro_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        if self.macro_run.is_some() {
            if key.code == KeyCode::Esc {
                self.macro_finish(true);
            }
            return;
        }
        let palette = self.focus == Focus::Palette;
        let from = match &mut self.recording {
            Some(rec) if !palette => {
                rec.steps.push(Step::Key(key));
                rec.current = Some(rec.steps.len() - 1);
                Some(rec.steps.len() - 1)
            }
            _ => None,
        };
        self.key(key, fx);
        if let Some(rec) = &mut self.recording {
            rec.current = None;
            // The key that opened the palette isn't kept: what it runs is.
            if let Some(from) = from
                && self.focus == Focus::Palette
            {
                rec.steps.truncate(from);
            }
        }
    }

    /// A command the palette ran, which a recording takes down.
    pub(crate) fn macro_command(&mut self, id: &str) {
        if let Some(rec) = &mut self.recording
            && id != "macro.record"
            && id != "macro.save"
        {
            rec.steps.push(Step::Command(id.to_string()));
        }
    }

    pub(crate) fn macro_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Starts recording, into a Vim register or for the palette's Play.
    pub(crate) fn macro_record(&mut self, register: Option<char>) {
        // `qA` adds to what `a` holds.
        let mut steps = Vec::new();
        let register = register.map(|r| {
            if r.is_ascii_uppercase() {
                let r = r.to_ascii_lowercase();
                steps = parse_steps(&self.vim_register_text(r).unwrap_or_default());
                r
            } else {
                r
            }
        });
        self.recording = Some(Recording {
            register,
            steps,
            current: None,
        });
    }

    /// Stops recording, without the key that stopped it.
    pub(crate) fn macro_stop(&mut self) {
        let Some(mut rec) = self.recording.take() else {
            return;
        };
        if let Some(at) = rec.current {
            rec.steps.truncate(at);
        }
        let text = steps_text(&rec.steps);
        if let Some(r) = rec.register {
            self.vim_set_register(r, &text);
            self.last_macro_register = Some(r);
        }
        self.last_macro = Some(text);
    }

    /// Plays a macro `times` times, at each cursor. One already playing
    /// plays it next, which is how a macro that ends with its own `@q`
    /// goes on until a key fails.
    pub(crate) fn macro_play(&mut self, name: String, text: &str, times: usize) {
        let steps = parse_steps(text);
        if steps.is_empty() {
            return self.notify(format!("{name} is empty"));
        }
        self.last_macro = Some(text.to_string());
        if let Some(run) = &mut self.macro_run {
            for _ in 0..times {
                for step in steps.iter().rev() {
                    run.queue.push_front(step.clone());
                }
            }
            return;
        }
        let mut carets: Vec<usize> = self.carets().iter().map(|c| c.cursor).collect();
        carets.sort_unstable();
        carets.dedup();
        self.clear_carets();
        self.doc_mut().anchor = None;
        let doc = self.doc();
        self.macro_run = Some(MacroRun {
            name,
            steps,
            times: times.max(1),
            done: 0,
            queue: VecDeque::new(),
            carets,
            ends: Vec::new(),
            len: doc.buffer.len(),
            doc: doc.id,
            undo_from: doc.buffer.undo_steps(),
            failed: None,
            halted: false,
        });
        self.macro_next_caret();
    }

    /// Plays the macro saved as `name`'s command, or the last one.
    pub(crate) fn macro_command_run(&mut self, id: &str) {
        if id == "macro.play" {
            return match self.last_macro.clone() {
                Some(text) => self.macro_play("the macro".into(), &text, 1),
                None => self.notify("No macro yet: Record a Macro first".into()),
            };
        }
        if let Some(m) = self.macros.iter().find(|m| m.command() == id).cloned() {
            self.macro_play(m.name.clone(), &m.keys, 1);
        }
    }

    /// Whether a run is playing, which the window steps on as soon as it
    /// can.
    pub fn macro_playing(&self) -> bool {
        self.macro_run.is_some()
    }

    /// Plays on for a slice of time.
    pub(crate) fn macro_step(&mut self, fx: &mut Vec<Effect>) {
        let start = Instant::now();
        while start.elapsed() < SLICE {
            let Some(run) = &mut self.macro_run else {
                return;
            };
            if let Some(step) = run.queue.pop_front() {
                match step {
                    Step::Key(key) => self.key(key, fx),
                    Step::Command(id) => self.run_command_id(&id, fx),
                }
                if let Some(why) = self.macro_failed.take() {
                    let line = self.doc().line_col(self.doc().cursor).0 + 1;
                    let run = self.macro_run.as_mut().expect("run");
                    let done = run.done.saturating_sub(1);
                    run.failed.get_or_insert((line, why, done));
                    run.queue.clear();
                    run.halted = true;
                }
                continue;
            }
            if run.done < run.times && !run.halted {
                run.done += 1;
                run.queue.extend(run.steps.iter().cloned());
                continue;
            }
            if !self.macro_next_caret() {
                return self.macro_finish(false);
            }
        }
    }

    /// Ends the run at a cursor and starts it at the next. Returns whether
    /// there was one.
    fn macro_next_caret(&mut self) -> bool {
        let leave = self.vim_mode().is_some_and(|m| m != VimMode::Normal);
        if leave {
            self.vim_reset();
        }
        let cursor = self.doc().cursor;
        let len = self.doc().buffer.len();
        let run = self.macro_run.as_mut().expect("run");
        let first = run.ends.is_empty() && run.done == 0;
        if !first {
            // What this one did moved the ones played below it.
            let delta = len as isize - run.len as isize;
            for end in &mut run.ends {
                *end = end.saturating_add_signed(delta);
            }
            run.ends.push(cursor);
        }
        let Some(next) = run.carets.pop() else {
            return false;
        };
        run.len = len;
        run.done = 0;
        run.halted = false;
        let doc = self.doc_mut();
        doc.cursor = next.min(doc.buffer.len());
        true
    }

    /// Stops the run and says how it went.
    fn macro_finish(&mut self, stopped: bool) {
        let Some(mut run) = self.macro_run.take() else {
            return;
        };
        if stopped {
            let delta = self.doc().buffer.len() as isize - run.len as isize;
            for end in &mut run.ends {
                *end = end.saturating_add_signed(delta);
            }
            run.ends.push(self.doc().cursor);
            run.ends.append(&mut run.carets);
        }
        if self.vim_mode().is_some_and(|m| m != VimMode::Normal) {
            self.vim_reset();
        }
        if self.doc().id == run.doc {
            self.doc_mut().buffer.join_undo(run.undo_from);
            // Every cursor stays where its play ended.
            run.ends.sort_unstable();
            run.ends.dedup();
            if let Some(&last) = run.ends.first() {
                let doc = self.doc_mut();
                let len = doc.buffer.len();
                doc.cursor = last.min(len);
                doc.extra = run.ends[1..]
                    .iter()
                    .map(|&c| crate::doc::Caret {
                        cursor: c.min(len),
                        anchor: None,
                    })
                    .collect();
            }
        }
        let times = |n: usize| match n {
            0 => "0 times".to_string(),
            1 => "once".to_string(),
            n => format!("{n} times"),
        };
        let note = match (&run.failed, stopped) {
            (_, true) => format!("Stopped {} after {} of {}", run.name, run.done, run.times),
            (Some((line, why, done)), _) => format!(
                "Ran {} {} · stopped early at line {line}, where {why}",
                run.name,
                times(*done)
            ),
            (None, _) => format!("Ran {} {}", run.name, times(run.times)),
        };
        match &mut self.vim {
            Some(vim) => vim.note = Some(note),
            None => self.notify(note),
        }
    }

    /// A key a macro played didn't work, which stops it.
    pub(crate) fn macro_fail(&mut self, why: String) {
        if self.macro_run.is_some() {
            self.macro_failed = Some(why);
        }
    }

    pub(crate) fn macro_view(&self) -> Option<MacroView> {
        let vim = self.vim.is_some();
        if let Some(run) = &self.macro_run {
            return Some(MacroView {
                name: run.name.clone(),
                keys: String::new(),
                progress: Some((run.done, run.times)),
                hint: "Esc stops".into(),
            });
        }
        let rec = self.recording.as_ref()?;
        Some(MacroView {
            name: rec.register.map(|r| format!("@{r}")).unwrap_or_default(),
            keys: steps_text(&rec.steps)
                .replace("<Esc>", "⎋")
                .replace("<CR>", "↵"),
            progress: None,
            hint: match (rec.register, vim) {
                (Some(_), _) => "q stops".into(),
                _ => self
                    .key_hint("macro.record")
                    .map_or("Record a Macro stops".into(), |k| format!("{k} stops")),
            },
        })
    }

    /// "Save the Last Macro As…": the palette asks for its name.
    pub(crate) fn macro_ask_name(&mut self) {
        if self.last_macro.is_none() {
            return self.notify("No macro yet: Record a Macro first".into());
        }
        self.palette = Some(Palette::new(
            Source::MacroName,
            Some("Save the last macro as".into()),
            Vec::new(),
        ));
        self.focus = Focus::Palette;
    }

    /// The name the palette was given: the macro's saved under it.
    pub(crate) fn macro_save(&mut self, name: &str, fx: &mut Vec<Effect>) {
        let (name, Some(keys)) = (name.trim(), self.last_macro.clone()) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let saved = Macro {
            name: name.to_string(),
            keys,
        };
        self.macros.retain(|m| m.command() != saved.command());
        self.macros.push(saved.clone());
        fx.push(Effect::SaveMacro(saved));
        self.notify_done(format!(
            "Saved Macro: {name}. Keyboard Shortcuts gives it a key"
        ));
    }
}

/// A key as Vim writes it: `a`, `<lt>`, `<Esc>`, `<C-r>`, `<S-Left>`.
fn key_text(key: &Key) -> String {
    let name = match key.code {
        KeyCode::Char('<') if !key.ctrl && !key.alt => return "<lt>".into(),
        KeyCode::Char(c) if !key.ctrl && !key.alt => return c.to_string(),
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "CR".into(),
        KeyCode::Backspace => "BS".into(),
        KeyCode::Delete => "Del".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "S-Tab".into(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}"),
    };
    let mut s = String::from("<");
    for (on, m) in [(key.ctrl, "C-"), (key.alt, "A-"), (key.shift, "S-")] {
        if on {
            s.push_str(m);
        }
    }
    s.push_str(&name);
    s.push('>');
    s
}

fn steps_text(steps: &[Step]) -> String {
    steps
        .iter()
        .map(|s| match s {
            Step::Key(k) => key_text(k),
            Step::Command(id) => format!("<Cmd>{id}<CR>"),
        })
        .collect()
}

/// A key in angle brackets, such as `C-r` or `S-Left`.
fn named_key(inner: &str) -> Option<Key> {
    let mut key = Key::plain(KeyCode::Esc);
    let mut rest = inner;
    loop {
        let lower = rest.to_ascii_lowercase();
        if rest.len() > 2 && lower.starts_with("c-") {
            key.ctrl = true;
        } else if rest.len() > 2 && (lower.starts_with("a-") || lower.starts_with("m-")) {
            key.alt = true;
        } else if rest.len() > 2 && lower.starts_with("s-") {
            key.shift = true;
        } else {
            break;
        }
        rest = &rest[2..];
    }
    key.code = match rest.to_ascii_lowercase().as_str() {
        "lt" => KeyCode::Char('<'),
        "cr" | "enter" | "return" => KeyCode::Enter,
        "bs" => KeyCode::Backspace,
        "del" => KeyCode::Delete,
        "esc" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "tab" if key.shift => {
            key.shift = false;
            KeyCode::BackTab
        }
        "tab" => KeyCode::Tab,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            KeyCode::F(f[1..].parse().ok()?)
        }
        _ => {
            let mut chars = rest.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if key.ctrl => KeyCode::Char(c.to_ascii_lowercase()),
                (Some(c), None) => KeyCode::Char(c),
                _ => return None,
            }
        }
    };
    Some(key)
}

/// A macro's text as its steps. Text in angle brackets that isn't a key
/// is typed as it is, as Vim types it.
fn parse_steps(text: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(close) = rest.find('>')
        {
            let inner = &rest[1..close];
            if inner == "Cmd"
                && let Some(end) = rest[close..].find("<CR>")
            {
                let id = &rest[close + 1..close + end];
                steps.push(Step::Command(id.to_string()));
                rest = &rest[close + end + 4..];
                continue;
            }
            if let Some(key) = named_key(inner) {
                steps.push(Step::Key(key));
                rest = &rest[close + 1..];
                continue;
            }
        }
        steps.push(Step::Key(match c {
            '\n' | '\r' => Key::plain(KeyCode::Enter),
            '\t' => Key::plain(KeyCode::Tab),
            c => Key::plain(KeyCode::Char(c)),
        }));
        rest = &rest[c.len_utf8()..];
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips() {
        for text in [
            "0wcwkept<Esc>j",
            "i<lt>b><Esc>",
            "<C-r><S-Left><A-x> x<CR>",
            "ab<Cmd>edit.comment<CR>c",
            "<F5><S-Tab><BS><Del>",
        ] {
            let steps = parse_steps(text);
            assert_eq!(steps_text(&steps), text);
        }
        // Not a key: typed as it is.
        assert_eq!(steps_text(&parse_steps("a<b")), "a<lt>b");
        assert_eq!(parse_steps("<nope>").len(), 6);
    }

    #[test]
    fn saved_command_ids() {
        let m = Macro {
            name: "Quote the word".into(),
            keys: String::new(),
        };
        assert_eq!(m.command(), "macro.saved.quote_the_word");
    }
}
