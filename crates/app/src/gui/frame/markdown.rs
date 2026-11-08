//! Markdown in panels: headings, lists, quotes, bold, italics, inline
//! code, links, and fenced code blocks coloured as their language.

use super::*;

/// How a run of a line looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Run {
    Plain,
    Bold,
    Italic,
    Code,
    Link,
}

/// A laid-out piece of Markdown, at its offset from the top.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum MdOp {
    /// A wrapped line of runs, indented by `x`, in a heading's face or not.
    Line {
        runs: Vec<(String, Run)>,
        x: f32,
        heading: bool,
        quote: bool,
    },
    /// A list item's bullet or number.
    Bullet { text: String, x: f32 },
    /// A line of a code block, with its source line for colours.
    Code { text: String, line: usize },
    /// The back of a code block, from its first line to its last.
    CodeBack { h: f32 },
}

const BODY: f32 = 12.5;
const LINE_H: f32 = 19.0;
const CODE_H: f32 = 18.0;

fn face(run: Run, heading: bool) -> Face {
    let size = if heading { 13.5 } else { BODY };
    match run {
        Run::Code => Face::mono(11.5),
        _ if heading => Face::ui(size).bold(),
        Run::Bold => Face::ui(size).bold(),
        Run::Italic => Face::ui(size).italic(),
        Run::Plain | Run::Link => Face::ui(size),
    }
}

/// A line's runs: `code`, **bold**, *italic* or _italic_, and
/// [links](url), whose text shows.
pub(super) fn runs(line: &str) -> Vec<(String, Run)> {
    let mut out: Vec<(String, Run)> = Vec::new();
    let push = |out: &mut Vec<(String, Run)>, text: &str, run: Run| {
        if text.is_empty() {
            return;
        }
        match out.last_mut() {
            Some((t, r)) if *r == run => t.push_str(text),
            _ => out.push((text.to_string(), run)),
        }
    };
    let mut rest = line;
    while !rest.is_empty() {
        let next = rest
            .char_indices()
            .find(|(_, c)| matches!(c, '`' | '*' | '_' | '['))
            .map(|(i, _)| i);
        let Some(i) = next else {
            push(&mut out, rest, Run::Plain);
            break;
        };
        push(&mut out, &rest[..i], Run::Plain);
        rest = &rest[i..];
        let (marker, run) = if rest.starts_with('`') {
            ("`", Run::Code)
        } else if rest.starts_with("**") {
            ("**", Run::Bold)
        } else if rest.starts_with("__") {
            ("__", Run::Bold)
        } else if rest.starts_with('*') {
            ("*", Run::Italic)
        } else if rest.starts_with('_') {
            ("_", Run::Italic)
        } else {
            ("[", Run::Link)
        };
        if run == Run::Link {
            // [text](url): the text shows, as a link.
            if let Some(close) = rest.find("](")
                && let Some(end) = rest[close..].find(')')
            {
                push(&mut out, &rest[1..close], Run::Link);
                rest = &rest[close + end + 1..];
            } else {
                push(&mut out, "[", Run::Plain);
                rest = &rest[1..];
            }
            continue;
        }
        let body = &rest[marker.len()..];
        // An underscore inside a word, or a marker with no end, is text.
        let inside_word = marker.starts_with('_')
            && out
                .last()
                .and_then(|(t, _)| t.chars().last())
                .is_some_and(char::is_alphanumeric);
        match body.find(marker) {
            Some(end) if end > 0 && !inside_word => {
                push(&mut out, &body[..end], run);
                rest = &body[end + marker.len()..];
            }
            _ => {
                push(&mut out, marker, Run::Plain);
                rest = body;
            }
        }
    }
    out
}

impl Frame<'_> {
    /// Lays Markdown out at width `w`: what to draw where, and the height.
    pub(super) fn md_layout(&mut self, text: &str, w: f32) -> (Vec<(f32, MdOp)>, f32) {
        let mut ops = Vec::new();
        let mut y = 0.0;
        let mut code: Option<(f32, usize)> = None;
        let mut after_gap = true;
        for (i, raw) in text.lines().enumerate() {
            let trimmed = raw.trim_start();
            if trimmed.starts_with("```") {
                match code.take() {
                    Some((top, at)) => {
                        let h = y - top + 6.0;
                        ops.insert(at, (top - 3.0, MdOp::CodeBack { h }));
                        y += 6.0;
                    }
                    None => {
                        y += 3.0;
                        code = Some((y, ops.len()));
                    }
                }
                continue;
            }
            if code.is_some() {
                ops.push((
                    y,
                    MdOp::Code {
                        text: raw.replace('\t', "    "),
                        line: i,
                    },
                ));
                y += CODE_H;
                continue;
            }
            if trimmed.is_empty() {
                if !after_gap {
                    y += 6.0;
                    after_gap = true;
                }
                continue;
            }
            after_gap = false;
            let (heading, quote, bullet, body) = block(trimmed);
            let indent = if bullet.is_some() { 16.0 } else { 0.0 } + if quote { 12.0 } else { 0.0 };
            if let Some(b) = &bullet {
                ops.push((
                    y,
                    MdOp::Bullet {
                        text: b.clone(),
                        x: if quote { 12.0 } else { 0.0 },
                    },
                ));
            }
            let h = if heading { 23.0 } else { LINE_H };
            for line in self.wrap_runs(&runs(body), heading, w - indent) {
                ops.push((
                    y,
                    MdOp::Line {
                        runs: line,
                        x: indent,
                        heading,
                        quote,
                    },
                ));
                y += h;
            }
        }
        // A block still open at the end is drawn as it is, as one streams.
        if let Some((top, at)) = code {
            let h = y - top + 6.0;
            ops.insert(at, (top - 3.0, MdOp::CodeBack { h }));
            y += 6.0;
        }
        (ops, y)
    }

    /// Plain text wrapped at `w`, word by word, a line at a time.
    pub(super) fn wrap_words(&mut self, text: &str, face: Face, w: f32) -> Vec<String> {
        let mut out = vec![String::new()];
        for word in text.split_inclusive(' ') {
            let line = out.last_mut().unwrap();
            if !line.is_empty() && self.text.width(&format!("{line}{word}"), face) > w {
                out.push(word.to_string());
            } else {
                line.push_str(word);
            }
        }
        out.retain(|l| !l.trim().is_empty());
        out
    }

    /// Runs wrapped at `w`, word by word.
    fn wrap_runs(
        &mut self,
        runs: &[(String, Run)],
        heading: bool,
        w: f32,
    ) -> Vec<Vec<(String, Run)>> {
        let mut out: Vec<Vec<(String, Run)>> = vec![Vec::new()];
        let mut width = 0.0;
        for (text, run) in runs {
            for word in text.split_inclusive(' ') {
                let ww = self.text.width(word, face(*run, heading));
                if width + ww > w && !out.last().unwrap().is_empty() {
                    out.push(Vec::new());
                    width = 0.0;
                }
                let line = out.last_mut().unwrap();
                match line.last_mut() {
                    Some((t, r)) if r == run => t.push_str(word),
                    _ => line.push((word.to_string(), *run)),
                }
                width += ww;
            }
        }
        out.retain(|l| !l.is_empty());
        out
    }

    /// Draws laid-out Markdown from (`x`, `y`). `colours` has each source
    /// line's code colours. Returns where its last line ends, for a caret.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn md_draw(
        &mut self,
        ops: &[(f32, MdOp)],
        x: f32,
        y: f32,
        w: f32,
        colours: &[Spans],
        colour: u32,
        clip: Rect,
    ) -> (f32, f32) {
        let p = self.p;
        let mut end = (x, y);
        for (dy, op) in ops {
            let top = y + dy;
            match op {
                MdOp::CodeBack { h } => {
                    let r = Rect::new(x, top, w, *h);
                    self.canvas
                        .outline(r, 6.0, 1.0, p.border, mix(p.ground, p.island, 0.5));
                }
                MdOp::Code { text, line } => {
                    let empty = Vec::new();
                    let spans = colours.get(*line).unwrap_or(&empty);
                    let inner = Rect::new(x, top, w, CODE_H).intersect(clip);
                    self.code_line_in(text, spans, x + 10.0, top, CODE_H, inner, Face::mono(11.5));
                    end = (x + 10.0 + self.text.width(text, Face::mono(11.5)), top);
                }
                MdOp::Bullet { text, x: bx } => {
                    self.text.draw_in_row(
                        &mut self.canvas,
                        text,
                        Face::ui(BODY),
                        x + bx + 2.0,
                        top,
                        LINE_H,
                        p.muted,
                        clip,
                    );
                }
                MdOp::Line {
                    runs,
                    x: lx,
                    heading,
                    quote,
                } => {
                    let h = if *heading { 23.0 } else { LINE_H };
                    if *quote {
                        self.canvas.round(
                            Rect::new(x + 2.0, top + 2.0, 2.0, h - 4.0),
                            1.0,
                            p.border,
                        );
                    }
                    let mut rx = x + lx;
                    for (text, run) in runs {
                        let f = face(*run, *heading);
                        let c = match run {
                            Run::Link => p.accent,
                            Run::Code => p.bright,
                            _ if *heading => p.bright,
                            _ if *quote => p.muted,
                            Run::Bold => p.bright,
                            _ => colour,
                        };
                        let tw = self.text.width(text, f);
                        if *run == Run::Code {
                            let back = Rect::new(rx - 1.0, top + 2.0, tw + 2.0, h - 4.0);
                            self.canvas.round(back, 3.0, p.chip);
                        }
                        self.text
                            .draw_in_row(&mut self.canvas, text, f, rx, top, h, c, clip);
                        rx += tw;
                    }
                    end = (rx, top);
                }
            }
        }
        end
    }
}

/// What a line of Markdown is: a heading, in a quote, a list item's
/// bullet, and the text after them.
fn block(line: &str) -> (bool, bool, Option<String>, &str) {
    let (quote, line) = match line.strip_prefix('>') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, line),
    };
    if let Some(rest) = line.strip_prefix('#') {
        return (true, quote, None, rest.trim_start_matches('#').trim_start());
    }
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return (false, quote, Some("•".into()), rest);
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = line[digits..].strip_prefix(". ")
    {
        return (false, quote, Some(format!("{}.", &line[..digits])), rest);
    }
    (false, quote, None, line)
}

#[cfg(test)]
mod tests {
    use super::{Run, block, runs};

    #[test]
    fn runs_find_code_bold_italics_and_links() {
        assert_eq!(
            runs("a `b` **c** *d* [e](http://x) f_g"),
            [
                ("a ".into(), Run::Plain),
                ("b".into(), Run::Code),
                (" ".into(), Run::Plain),
                ("c".into(), Run::Bold),
                (" ".into(), Run::Plain),
                ("d".into(), Run::Italic),
                (" ".into(), Run::Plain),
                ("e".into(), Run::Link),
                (" f_g".into(), Run::Plain),
            ]
        );
        // A marker without its end is text.
        assert_eq!(runs("2 * 3"), [("2 * 3".into(), Run::Plain)]);
    }

    #[test]
    fn blocks_are_headings_quotes_and_list_items() {
        assert_eq!(block("## Title"), (true, false, None, "Title"));
        assert_eq!(block("> said"), (false, true, None, "said"));
        assert_eq!(block("- item"), (false, false, Some("•".into()), "item"));
        assert_eq!(
            block("12. item"),
            (false, false, Some("12.".into()), "item")
        );
        assert_eq!(block("plain"), (false, false, None, "plain"));
    }
}
