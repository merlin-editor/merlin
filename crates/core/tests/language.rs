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
            ..Default::default()
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
                ..Default::default()
            },
            Diagnostic {
                start: (0, 8),
                end: (0, 9),
                severity: Severity::Error,
                message: "cannot find value `y`\nhelp: a local variable with a similar name exists"
                    .into(),
                ..Default::default()
            },
            // The same error again, as a second checker reports it.
            Diagnostic {
                start: (0, 8),
                end: (0, 9),
                severity: Severity::Error,
                message: "cannot find value `y`".into(),
                ..Default::default()
            },
            Diagnostic {
                start: (1, 4),
                end: (1, 5),
                severity: Severity::Hint,
                message: "a hint stays out of the way".into(),
                ..Default::default()
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

/// A window drawing the code itself, as the GUI has it, with a language
/// server that answers hovers.
fn hovering(text: &str) -> Session {
    let mut s = rust(text);
    s.send(Event::Viewports(vec![(60, 8)]));
    s.send(Event::RegisterPlugin {
        plugin: "lsp".into(),
        commands: vec![merlin_core::Command {
            id: "hover".into(),
            title: "Language: Show Hover".into(),
            key: None,
        }],
    });
    s
}

/// The mouse at text column `col` of `row`, moving or resting.
fn mouse(s: &mut Session, kind: merlin_core::MouseKind, row: u16, col: u16) {
    let (x, y) = s.editor.text_origin();
    s.send(Event::Pointer(merlin_core::Pointer {
        group: 0,
        kind,
        row: (y + row) as f32,
        col: (x + col) as f32 + 0.2,
        shift: false,
        alt: false,
        ctrl: false,
        clicks: 1,
    }));
}

fn hover(s: &Session) -> Option<(Vec<merlin_core::ProblemView>, Vec<String>)> {
    match s.editor.window_view().popup?.kind {
        merlin_core::PopupKind::Hover {
            problems, lines, ..
        } => Some((problems, lines)),
        _ => None,
    }
}

fn mismatched() -> Event {
    Event::SetDiagnostics {
        plugin: "lsp".into(),
        path: "main.rs".into(),
        diagnostics: vec![
            Diagnostic {
                start: (0, 15),
                end: (0, 26),
                severity: Severity::Error,
                message: "mismatched types\nexpected `u32`, found `usize`".into(),
                source: "rustc".into(),
                code: "E0308".into(),
                link: String::new(),
            },
            // rust-analyzer's copy of it, worded and placed differently.
            Diagnostic {
                start: (0, 17),
                end: (0, 26),
                severity: Severity::Error,
                message: "expected u32, found usize".into(),
                source: "rust-analyzer".into(),
                code: "E0308".into(),
                link: "https://doc.rust-lang.org/error_codes/E0308.html".into(),
            },
            Diagnostic {
                start: (0, 4),
                end: (0, 9),
                severity: Severity::Warning,
                message: "unused variable: `count`".into(),
                source: "rustc".into(),
                code: "unused_variables".into(),
                link: String::new(),
            },
        ],
    }
}

#[test]
fn resting_on_a_problem_shows_it_first_and_the_servers_hover_joins_it() {
    use merlin_core::MouseKind::Rest;
    let mut s = hovering("let count: u32 = steps.len();\n");
    s.send(mismatched());
    s.effects.clear();
    // On `len`, inside the error.
    mouse(&mut s, Rest, 0, 23);
    let (problems, lines) = hover(&s).expect("a card shows at once");
    assert_eq!(problems.len(), 1, "the same problem twice shows once");
    assert_eq!(
        problems[0].message, "mismatched types\nexpected `u32`, found `usize`",
        "the fuller copy is kept"
    );
    assert_eq!(
        (&*problems[0].source, &*problems[0].code),
        ("rustc", "E0308")
    );
    assert_eq!(
        problems[0].also,
        ["rust-analyzer"],
        "and who else reports it"
    );
    assert!(lines.is_empty());
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::PluginCommand { command, .. } if command == "hover")),
        "the server is asked about the word too"
    );
    s.send(Event::ShowHover(
        "```rust\npub const fn len(&self) -> usize\n```".into(),
    ));
    let (problems, lines) = hover(&s).unwrap();
    assert_eq!(problems.len(), 1, "the problem stays first");
    assert_eq!(lines[1], "pub const fn len(&self) -> usize");
}

#[test]
fn a_problem_off_a_word_still_gets_a_card() {
    use merlin_core::MouseKind::Rest;
    // The error covers `( )`, where there's no word to ask the server about.
    let mut s = hovering("call( );\n");
    s.send(diagnostic((0, 4), (0, 7), "this function takes 1 argument"));
    s.effects.clear();
    mouse(&mut s, Rest, 0, 5);
    let (problems, _) = hover(&s).expect("a card of the problem alone");
    assert_eq!(problems[0].message, "this function takes 1 argument");
    assert!(s.effects.is_empty(), "nothing to ask the server");
}

#[test]
fn the_card_stays_while_the_mouse_is_on_its_problem() {
    use merlin_core::MouseKind::{Move, Rest};
    let mut s = hovering("let count: u32 = steps.len();\n");
    s.send(mismatched());
    mouse(&mut s, Rest, 0, 17);
    assert!(hover(&s).is_some());
    // Along the error from `steps` to `len`, it stays.
    mouse(&mut s, Move, 0, 24);
    assert!(hover(&s).is_some());
    // Off it, it goes.
    mouse(&mut s, Move, 0, 2);
    assert!(hover(&s).is_none());
}

#[test]
fn ctrl_k_ctrl_i_shows_the_hover_at_the_cursor() {
    let mut s = hovering("let count: u32 = steps.len();\n");
    s.send(mismatched());
    s.run("key right right right right right");
    s.run("key ctrl+k\nkey ctrl+i");
    let (problems, _) = hover(&s).expect("the cursor's problem shows");
    assert_eq!(problems[0].message, "unused variable: `count`");
    assert_eq!(problems[0].severity, Severity::Warning);
}

#[test]
fn the_cards_buttons_copy_the_problem_and_open_problems() {
    use merlin_core::{HoverButton, MouseKind::Rest, UiEvent};
    let mut s = hovering("let count: u32 = steps.len();\n");
    s.send(mismatched());
    mouse(&mut s, Rest, 0, 6);
    s.effects.clear();
    s.send(Event::Ui(UiEvent::Hover(HoverButton::Copy)));
    assert!(
        s.effects
            .contains(&Effect::Copy("unused variable: `count`".into()))
    );
    s.send(Event::Ui(UiEvent::Hover(HoverButton::Problems)));
    assert!(hover(&s).is_none(), "the card goes");
    let bottom = s.editor.window_view().bottom.expect("the panel shows");
    assert_eq!(bottom.title, "Problems");
}

#[test]
fn a_hint_that_goes_with_a_problem_is_a_note_under_it() {
    use merlin_core::MouseKind::Rest;
    let mut s = hovering("let hex = 5;\n");
    let unused = |severity, message: &str| Diagnostic {
        start: (0, 4),
        end: (0, 7),
        severity,
        message: message.into(),
        source: "rustc".into(),
        code: "unused_variables".into(),
        link: String::new(),
    };
    s.send(Event::SetDiagnostics {
        plugin: "lsp".into(),
        path: "main.rs".into(),
        diagnostics: vec![
            unused(Severity::Warning, "unused variable: `hex`"),
            unused(
                Severity::Hint,
                "if this is intentional, prefix it with an underscore: `_hex`",
            ),
        ],
    });
    mouse(&mut s, Rest, 0, 5);
    let (problems, _) = hover(&s).unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(
        problems[0].message,
        "unused variable: `hex`\nhelp: if this is intentional, prefix it with an underscore: `_hex`"
    );
}

#[test]
fn the_cards_code_opens_its_page_and_go_to_definition_goes_from_the_word() {
    use merlin_core::{HoverButton, MouseKind::Rest, UiEvent};
    let mut s = hovering("let count: u32 = steps.len();\n");
    s.send(Event::RegisterPlugin {
        plugin: "lsp".into(),
        commands: ["hover", "definition"]
            .map(|id| merlin_core::Command {
                id: id.into(),
                title: id.into(),
                key: None,
            })
            .into(),
    });
    s.send(mismatched());
    mouse(&mut s, Rest, 0, 23);
    s.send(Event::ShowHover(
        "```rust\npub const fn len(&self) -> usize\n```".into(),
    ));
    let Some(merlin_core::PopupKind::Hover { definition, .. }) =
        s.editor.window_view().popup.map(|p| p.kind)
    else {
        panic!("a hover shows");
    };
    assert!(definition, "it offers to go to the definition");
    s.effects.clear();
    s.send(Event::Ui(UiEvent::Hover(HoverButton::Link(0))));
    assert_eq!(
        s.effects,
        [Effect::OpenUrl(
            "https://doc.rust-lang.org/error_codes/E0308.html".into()
        )],
        "rust-analyzer's link serves rustc's copy"
    );
    s.effects.clear();
    s.send(Event::Ui(UiEvent::Hover(HoverButton::Definition)));
    assert_eq!(
        s.editor.cursor_utf16(),
        (0, 23),
        "the cursor goes to the word"
    );
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::PluginCommand { command, .. } if command == "definition"))
    );
    assert!(hover(&s).is_none());
}
