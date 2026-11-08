//! The PDF plugin through the host: a file of its type opens as pages, a
//! tile draws, and the page's words come with where they are. Build it
//! first: `cd plugins/pdf && cargo build --release`

use std::path::Path;

use merlin_core::{Event, TileKey};
use merlin_plugins::{Config, PluginHost};

#[test]
fn a_pdf_opens_draws_and_has_its_words() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/pdf");
    assert!(
        dir.join("target/wasm32-wasip2/release/pdf.wasm").exists(),
        "build the pdf plugin first (see the top of this file)"
    );
    let mut host = PluginHost::new(Config {
        data_dir: std::env::temp_dir().join(format!("merlin-pdf-test-{}", std::process::id())),
        ask: false,
        keyring: false,
        background: false,
    })
    .unwrap();
    let events = host.load(&dir).unwrap();
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::RegisterEditors { editors, .. } if editors[0].extensions == ["pdf"]
        )),
        "{events:?}"
    );

    let data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/page.pdf"))
        .unwrap();
    let pages = match &host.open_document("pdf", "pdf", "page.pdf", data, 7)[..] {
        [
            Event::DocumentOpened {
                surface: 7,
                pages: Ok(pages),
            },
        ] => pages.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(pages.len(), 1);
    assert!((pages[0].width - 595.3).abs() < 1.0, "A4: {:?}", pages[0]);

    // The top left corner, at half size: white paper, opaque.
    let tile = TileKey {
        page: 0,
        x: 0,
        y: 0,
        width: 64,
        height: 32,
        scale: 500,
    };
    match &host.render_tiles("pdf", 7, vec![tile])[..] {
        [
            Event::DocumentTile {
                image: Ok(image),
                tile: t,
                ..
            },
        ] => {
            assert_eq!(*t, tile);
            assert_eq!((image.width, image.height), (64, 32));
            assert_eq!(&image.pixels[..4], &[255, 255, 255, 255]);
        }
        other => panic!("{other:?}"),
    }

    let words = match &host.page_text("pdf", 7, 0)[..] {
        [Event::DocumentText { boxes, .. }] => boxes.clone(),
        other => panic!("{other:?}"),
    };
    let text: Vec<&str> = words.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(&text[..3], ["Drawing", "a", "frame"], "{text:?}");
    // Words on a line share it, left to right, inside the page.
    assert!((words[0].y - words[2].y).abs() < 1.0);
    assert!(words[0].x < words[1].x && words[1].x < words[2].x);
    assert!(
        words
            .iter()
            .all(|b| b.x >= 0.0 && b.x + b.width <= pages[0].width)
    );
    assert!(text.contains(&"atlas"), "{text:?}");

    // A garbled file says why.
    match &host.open_document("pdf", "pdf", "bad.pdf", b"not a pdf".to_vec(), 8)[..] {
        [
            Event::DocumentOpened {
                pages: Err(why), ..
            },
        ] => assert!(why.contains("PDF"), "{why}"),
        other => panic!("{other:?}"),
    }
    host.close_document("pdf", 7);
}
