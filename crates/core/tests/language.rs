//! Milestone 3 in the core: syntax colours, diagnostics, hover, completion
//! and go-to.

use merlin_core::script::Session;
use merlin_core::{Completion, Diagnostic, Effect, Event, Severity};

fn rust(text: &str) -> Session {
    Session::new(text, Some("main.rs"), 60, 8)
}

fn diagnostic(start: (u32, u32), end: (u32, u32), message: &str) -> Event {
    Event::SetDiagnostics {
        plugin: "lsp".into(),
        path: "main.rs".into(),
        diagnostics: vec![Diagnostic {
            start,
            end,
            severity: Severity::Error,
            message: message.into(),
        }],
    }
}

/// The cell at a text position, past the gutter.
fn cell(s: &Session, col: u16, row: u16) -> merlin_core::style::Cell {
    let (x, y) = s.editor.text_origin();
    s.cell(x + col, y + row).unwrap()
}

#[test]
fn diagnostics_underline_colour_the_line_number_and_show_in_the_status() {
    let mut s = rust("let x = y;\n");
    s.send(diagnostic((0, 8), (0, 9), "cannot find value `y`"));
    assert_eq!(cell(&s, 8, 0).mark, Some(Severity::Error));
    assert_eq!(cell(&s, 4, 0).mark, None);
    assert_eq!(
        s.cell(s.editor.text_origin().0 - 2, s.editor.text_origin().1)
            .unwrap()
            .style,
        merlin_core::style::Style::GutterError
    );
    assert!(
        s.screen().contains("cannot find value `y`"),
        "{}",
        s.screen()
    );
}

#[test]
fn diagnostics_move_with_edits() {
    let mut s = rust("let x = y;\n");
    s.send(diagnostic((0, 8), (0, 9), "oops"));
    s.run("key enter");
    assert_eq!(
        cell(&s, 8, 1).mark,
        Some(Severity::Error),
        "moved down a line"
    );
    s.run("key ctrl+z");
    assert_eq!(cell(&s, 8, 0).mark, Some(Severity::Error), "and back");
}

#[test]
fn diagnostic_columns_count_utf16_like_lsp() {
    let mut s = rust("let s = \"😀\"; z\n");
    // The emoji is two UTF-16 units, so `z` is at unit 14 but char 13.
    s.send(diagnostic((0, 14), (0, 15), "z?"));
    let y = s.editor.text_origin().1;
    let x = (4..60).find(|x| s.cell(*x, y).unwrap().ch == 'z').unwrap();
    assert_eq!(s.cell(x, y).unwrap().mark, Some(Severity::Error));
}

#[test]
fn cursor_positions_for_plugins_count_utf16() {
    let mut s = rust("😀ab");
    s.run("key end");
    assert_eq!(s.editor.cursor_utf16(), (0, 4));
}

#[test]
fn hover_shows_until_the_next_key() {
    let mut s = rust("fn main() {}");
    s.send(Event::ShowHover("fn main()\n\nThe entry point.".into()));
    assert!(s.screen().contains("The entry point."), "{}", s.screen());
    s.run("key right");
    assert_eq!(s.editor.popup_text(), None);
    assert_eq!(s.editor.cursor(), 1, "the key still moved the cursor");
}

fn completions() -> Event {
    let item = |label: &str, detail: &str| Completion {
        label: label.into(),
        detail: detail.into(),
        insert: label.into(),
    };
    Event::ShowCompletions(vec![
        item("push", "fn(&mut self, T)"),
        item("pop", "fn(&mut self) -> Option<T>"),
        item("len", "fn(&self) -> usize"),
    ])
}

#[test]
fn completion_filters_as_you_type_and_replaces_the_word() {
    let mut s = rust("v.p");
    s.run("key end");
    s.send(completions());
    assert_eq!(s.editor.popup_text().as_deref(), Some("push\npop"));
    s.run("type o");
    assert_eq!(s.editor.popup_text().as_deref(), Some("pop"));
    s.run("key enter");
    assert_eq!(s.text(), "v.pop");
    assert_eq!(s.editor.popup_text(), None);
}

#[test]
fn completion_closes_when_nothing_matches_or_on_escape() {
    let mut s = rust("v.");
    s.run("key end");
    s.send(completions());
    assert!(s.screen().contains("fn(&self) -> usize"), "{}", s.screen());
    s.run("type q");
    assert_eq!(s.editor.popup_text(), None);
    s.send(completions());
    s.run("key esc; type x");
    assert_eq!(s.text(), "v.qx");
}

#[test]
fn completion_list_moves_with_the_arrows() {
    let mut s = rust("v.");
    s.run("key end");
    s.send(completions());
    s.run("key down down down; key tab");
    assert_eq!(s.text(), "v.len");
}

#[test]
fn go_to_opens_the_file_at_the_position_and_alt_left_comes_back() {
    let mut s = rust("use lib;\nfn main() {}");
    s.files
        .insert("src/lib.rs".into(), "// lib\npub fn helper() {}\n".into());
    s.run("key down end");
    s.send(Event::OpenAt {
        path: "src/lib.rs".into(),
        line: 1,
        column: 7,
    });
    assert!(s.effects.contains(&Effect::Open("src/lib.rs".into())));
    assert_eq!(s.editor.cursor_utf16(), (1, 7));
    assert_eq!(s.text(), "// lib\npub fn helper() {}\n");
    s.run("key alt+left");
    assert_eq!(s.text(), "use lib;\nfn main() {}");
    assert_eq!(s.editor.cursor_utf16(), (1, 12));
}

#[test]
fn hover_keeps_code_indentation() {
    let mut s = rust("fn main() {}");
    s.send(Event::ShowHover("struct A {\n    field: u32,\n}".into()));
    assert!(s.screen().contains("    field: u32,"), "{}", s.screen());
}

/// The note a line of the active group shows after its end.
fn note(s: &Session, line: usize) -> Option<(Severity, String)> {
    let views = s.editor.text_views();
    let view = views.last().unwrap();
    view.lines.iter().find(|l| l.line == line)?.note.clone()
}

#[test]
fn a_lines_worst_problem_is_written_after_it() {
    let mut s = rust("let x = y;\nlet z = 1;\n");
    s.send(Event::SetDiagnostics {
        plugin: "lsp".into(),
        path: "main.rs".into(),
        diagnostics: vec![
            Diagnostic {
                start: (0, 4),
                end: (0, 5),
                severity: Severity::Warning,
                message: "unused variable `x`".into(),
            },
            Diagnostic {
                start: (0, 8),
                end: (0, 9),
                severity: Severity::Error,
                message: "cannot find value `y`\nhelp: a local variable with a similar name exists"
                    .into(),
            },
            // The same error again, as a second checker reports it.
            Diagnostic {
                start: (0, 8),
                end: (0, 9),
                severity: Severity::Error,
                message: "cannot find value `y`".into(),
            },
            Diagnostic {
                start: (1, 4),
                end: (1, 5),
                severity: Severity::Hint,
                message: "a hint stays out of the way".into(),
            },
        ],
    });
    assert_eq!(
        note(&s, 0),
        Some((Severity::Error, "cannot find value `y`  (+1 more)".into())),
        "the error wins, without its help, and the warning is counted once"
    );
    assert_eq!(note(&s, 1), None, "hints aren't written out");
    s.send(diagnostic(
        (1, 4),
        (1, 5),
        "mismatched types\nexpected `u32`, found `usize`",
    ));
    assert_eq!(
        note(&s, 1),
        Some((
            Severity::Error,
            "mismatched types: expected `u32`, found `usize`".into()
        )),
        "a second line that says more is kept"
    );
    s.send(Event::Settings(merlin_core::Settings {
        inline_diagnostics: false,
        ..Default::default()
    }));
    assert_eq!(note(&s, 0), None, "the setting turns it off");
}

fn signature(active: (usize, usize)) -> Event {
    Event::ShowSignature(Some(merlin_core::Signature {
        label: "fn mix(a: u8, b: u8) -> u8".into(),
        active: Some(active),
        doc: "Mixes two values.".into(),
        index: 0,
        count: 1,
    }))
}

#[test]
fn signature_help_stays_while_the_call_is_typed() {
    let mut s = rust("fn main() {\n    mix(\n}\n");
    s.run("key down; key end");
    s.send(signature((7, 12)));
    let shown = s.editor.window_view().signature.expect("it shows");
    assert_eq!(shown.signature.active, Some((7, 12)));
    // Typing the arguments keeps it, and a new answer keeps its place.
    s.run("type 1,");
    s.send(signature((14, 19)));
    let again = s.editor.window_view().signature.unwrap();
    assert_eq!(again.anchor, shown.anchor, "it stays where the call is");
    assert_eq!(again.signature.active, Some((14, 19)));
    // Escape hides it, and leaves the text and cursor be.
    s.run("key esc");
    assert!(s.editor.window_view().signature.is_none());
    assert_eq!(s.text(), "fn main() {\n    mix(1,\n}\n");
}

#[test]
fn signature_help_goes_when_the_cursor_leaves_the_call() {
    let mut s = rust("fn main() {\n    mix(\n}\n");
    s.run("key down; key end");
    s.send(signature((7, 12)));
    s.run("key backspace");
    assert!(
        s.editor.window_view().signature.is_none(),
        "back before where it opened"
    );
    s.run("type (");
    s.send(signature((7, 12)));
    s.run("key down");
    assert!(s.editor.window_view().signature.is_none(), "another line");
    s.run("key up; key end");
    s.send(Event::ShowSignature(None));
    assert!(
        s.editor.window_view().signature.is_none(),
        "the server says so"
    );
}
