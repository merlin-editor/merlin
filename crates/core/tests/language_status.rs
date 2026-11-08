//! The status bar's language item and the card it opens, from what
//! plugins report.

use merlin_core::lang::{Language, StatusLine, StatusState};
use merlin_core::script::Session;
use merlin_core::{Effect, Event, LanguageCardHit, StatusMark, UiEvent};

fn rust() -> Session {
    let mut s = Session::new("fn main() {}\n", Some("src/main.rs"), 120, 12);
    s.send(Event::RegisterLanguages {
        plugin: "rust".into(),
        languages: vec![Language {
            id: "rust".into(),
            name: "Rust".into(),
            extensions: vec!["rs".into()],
            line_comment: Some("//".into()),
            block_comment: None,
            ..Default::default()
        }],
    });
    s
}

fn server(state: StatusState, detail: &str) -> StatusLine {
    StatusLine {
        name: "rust-analyzer".into(),
        state,
        detail: detail.into(),
        action: Some(("Restart".into(), "restart".into())),
        extensions: vec!["rs".into()],
        ..Default::default()
    }
}

fn report(s: &mut Session, line: StatusLine) {
    s.send(Event::SetStatus {
        plugin: "rust".into(),
        set: "rust-analyzer".into(),
        lines: vec![line],
    });
}

fn language(s: &Session) -> Option<StatusMark> {
    s.editor.window_view().status.last().unwrap().mark
}

#[test]
fn the_language_item_shows_how_its_servers_are_doing() {
    let mut s = rust();
    assert_eq!(language(&s), None, "nothing reported, no {{ }}");
    report(&mut s, server(StatusState::Ready, "Ready"));
    assert_eq!(language(&s), Some(StatusMark::Braces(StatusState::Ready)));
    report(
        &mut s,
        StatusLine {
            progress: Some(45),
            ..server(StatusState::Busy, "Indexing · 214/480 crates")
        },
    );
    let status = s.editor.window_view().status;
    let n = status.len();
    assert_eq!(status[n - 2].text, "rust-analyzer: Indexing 45%");
    assert_eq!(status[n - 2].mark, Some(StatusMark::Spinner));
    assert_eq!(language(&s), Some(StatusMark::Braces(StatusState::Busy)));
}

#[test]
fn a_click_opens_the_card_and_its_buttons_run_in_the_plugin() {
    let mut s = rust();
    report(
        &mut s,
        StatusLine {
            hint: Some("rustup component add rust-analyzer".into()),
            action: Some(("Try Again".into(), "restart".into())),
            ..server(StatusState::Warning, "isn't installed")
        },
    );
    let last = s.editor.window_view().status.len() - 1;
    s.send(Event::Ui(UiEvent::Status(last)));
    let card = s
        .editor
        .window_view()
        .language_card
        .expect("the card opens");
    assert_eq!(card.title, "Rust");
    assert_eq!(card.rows[0].name, "rust-analyzer");
    assert_eq!(card.rows[0].button, Some(("Try Again".into(), true)));
    assert_eq!(card.rows[1].name, "Problems");
    assert!(s.editor.window_view().status[last].open);

    s.effects.clear();
    s.send(Event::Ui(UiEvent::LanguageCard(LanguageCardHit::Row(0))));
    assert_eq!(
        s.effects,
        [Effect::PluginCommand {
            plugin: "rust".into(),
            command: "restart".into(),
        }]
    );
    s.effects.clear();
    s.send(Event::Ui(UiEvent::LanguageCard(LanguageCardHit::Copy(0))));
    assert!(
        s.effects
            .contains(&Effect::Copy("rustup component add rust-analyzer".into()))
    );

    s.run("key esc");
    assert!(
        s.editor.window_view().language_card.is_none(),
        "Escape closes it"
    );
    assert_eq!(s.text(), "fn main() {}\n", "and does nothing else");
}

#[test]
fn a_line_is_only_for_the_files_it_names() {
    let mut s = Session::new("package main\n", Some("main.go"), 120, 12);
    s.send(Event::SetStatus {
        plugin: "lsp".into(),
        set: "servers".into(),
        lines: vec![
            StatusLine {
                name: "gopls".into(),
                detail: "Ready".into(),
                extensions: vec!["go".into()],
                ..Default::default()
            },
            StatusLine {
                name: "clangd".into(),
                state: StatusState::Error,
                detail: "stopped".into(),
                extensions: vec!["cpp".into()],
                ..Default::default()
            },
        ],
    });
    assert_eq!(language(&s), Some(StatusMark::Braces(StatusState::Ready)));
    s.run("cmd language.status");
    let card = s.editor.window_view().language_card.unwrap();
    let names: Vec<&str> = card.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["gopls", "Problems"]);
    assert_eq!(card.from, "from the lsp plugin");
}
