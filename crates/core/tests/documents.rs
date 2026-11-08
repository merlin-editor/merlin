//! Documents a plugin draws, such as PDFs: they open as pages, their words
//! become the tab's text, and Find, Copy and the status bar work on them.

use std::path::PathBuf;

use merlin_core::script::Session;
use merlin_core::{
    DocEditor, Effect, Event, Key, KeyCode, PageSize, SurfaceAction, TextBox, UiEvent,
};

fn word(x: f32, y: f32, text: &str) -> TextBox {
    TextBox {
        x,
        y,
        width: 10.0 * text.len() as f32,
        height: 12.0,
        text: text.into(),
    }
}

fn key(code: KeyCode, ctrl: bool) -> Event {
    Event::Key(Key {
        code,
        ctrl,
        alt: false,
        shift: false,
    })
}

#[test]
fn a_pdf_opens_as_pages_with_its_words_to_find_and_copy() {
    let mut s = Session::new("", Some("notes.txt"), 60, 10);
    s.send(Event::RegisterEditors {
        plugin: "pdf".into(),
        editors: vec![DocEditor {
            plugin: "pdf".into(),
            id: "pdf".into(),
            title: "PDF".into(),
            extensions: vec!["pdf".into()],
        }],
    });
    s.send(Event::Opened {
        path: PathBuf::from("docs/a.pdf"),
        text: String::new(),
    });
    let surface = s
        .effects
        .iter()
        .find_map(|e| match e {
            Effect::OpenDocument { surface, path, .. } if path.ends_with("a.pdf") => Some(*surface),
            _ => None,
        })
        .expect("asked its plugin to read it");
    let page = PageSize {
        width: 600.0,
        height: 800.0,
    };
    s.effects.clear();
    s.send(Event::DocumentOpened {
        surface,
        pages: Ok(vec![page, page]),
    });
    // Each page's words are asked for, in order.
    let asked: Vec<u32> = s
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::PageText { page, .. } => Some(*page),
            _ => None,
        })
        .collect();
    assert_eq!(asked, [0, 1]);
    s.send(Event::DocumentText {
        surface,
        page: 0,
        boxes: vec![
            word(10.0, 10.0, "One"),
            word(60.0, 10.0, "atlas"),
            word(10.0, 40.0, "two"),
        ],
    });
    s.send(Event::DocumentText {
        surface,
        page: 1,
        boxes: vec![word(10.0, 10.0, "atlas")],
    });
    assert_eq!(s.text(), "One atlas\ntwo\n\natlas");

    s.send(Event::Ui(UiEvent::SurfaceArea {
        id: surface,
        w: 400.0,
        h: 300.0,
    }));
    let view = s.editor.window_view();
    let (_, sv) = &view.surfaces[0];
    assert_eq!(sv.current, 0);
    let status: Vec<String> = view
        .status
        .iter()
        .map(|i| i.text.trim().to_string())
        .collect();
    assert!(status.contains(&"Page 1 of 2".to_string()), "{status:?}");
    assert!(status.contains(&"PDF".to_string()), "{status:?}");

    // Find: two matches, on both pages, as boxes.
    s.send(key(KeyCode::Char('f'), true));
    for c in "atlas".chars() {
        s.send(key(KeyCode::Char(c), false));
    }
    let view = s.editor.window_view();
    let (_, sv) = &view.surfaces[0];
    assert_eq!(sv.hits.len(), 2);
    assert_eq!(sv.hits[1].0, 1, "the second on page 2");
    assert_eq!(sv.hit, Some(0));
    s.send(key(KeyCode::Esc, false));

    // Selecting the first line by its words, then Copy.
    s.send(Event::Ui(UiEvent::SurfacePoint {
        id: surface,
        page: 0,
        x: 10.0,
        y: 15.0,
        drag: false,
    }));
    s.send(Event::Ui(UiEvent::SurfacePoint {
        id: surface,
        page: 0,
        x: 110.0,
        y: 15.0,
        drag: true,
    }));
    s.effects.clear();
    s.send(key(KeyCode::Char('c'), true));
    assert!(
        s.effects.contains(&Effect::Copy("One atlas".into())),
        "{:?}",
        s.effects
    );
    // One band across both words.
    let view = s.editor.window_view();
    assert_eq!(view.surfaces[0].1.selected.len(), 1);

    // Next page, and typing changes nothing.
    s.send(Event::Ui(UiEvent::Surface {
        id: surface,
        action: SurfaceAction::Next,
    }));
    s.send(key(KeyCode::Char('x'), false));
    assert_eq!(s.text(), "One atlas\ntwo\n\natlas");
    let view = s.editor.window_view();
    assert_eq!(view.surfaces[0].1.current, 1);
}
