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
fn cell(s: &Session, col: u16, row: u16) -> merlin_core::screen::Cell {
    let (x, y) = s.editor.text_origin();
    s.editor.render().cell(x + col, y + row)
}

#[test]
fn diagnostics_underline_colour_the_line_number_and_show_in_the_status() {
    let mut s = rust("let x = y;\n");
    s.send(diagnostic((0, 8), (0, 9), "cannot find value `y`"));
    assert_eq!(cell(&s, 8, 0).mark, Some(Severity::Error));
    assert_eq!(cell(&s, 4, 0).mark, None);
    assert_eq!(
        s.editor
            .render()
            .cell(s.editor.text_origin().0 - 2, s.editor.text_origin().1)
            .style,
        merlin_core::screen::Style::GutterError
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
    let screen = s.editor.render();
    let y = s.editor.text_origin().1;
    let x = (4..60).find(|x| screen.cell(*x, y).ch == 'z').unwrap();
    assert_eq!(screen.cell(x, y).mark, Some(Severity::Error));
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
