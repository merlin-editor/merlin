//! Timings for large files. Run with
//! `cargo test --release -p merlin-core --test large_probe -- --ignored --nocapture`.

use std::time::Instant;

use merlin_core::Event;
use merlin_core::script::Session;

fn time<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    eprintln!("  {what:<22} {:>9.1} ms", t.elapsed().as_secs_f64() * 1e3);
    out
}

/// The most a frame may take, in milliseconds, once the file is open: it
/// must cost what the screen shows, whatever the file's size.
const FRAME_LIMIT: f64 = 2.0;

/// A frame as the window draws one, the window's view and the text's, timed
/// and held to [`FRAME_LIMIT`] unless `over` says why it can't be yet.
fn frame(s: &Session, what: &str, over: Option<&str>, slow: &mut Vec<String>) {
    let t = Instant::now();
    std::hint::black_box((s.editor.window_view(), s.editor.text_views()));
    let ms = t.elapsed().as_secs_f64() * 1e3;
    eprintln!("  {what:<22} {ms:>9.1} ms");
    if ms > FRAME_LIMIT && over.is_none() {
        slow.push(format!("{what}: {ms:.1} ms"));
    }
}

fn probe(name: &str, path: &str, text: String) {
    if std::env::var("PROBE").is_ok_and(|p| !name.starts_with(&p)) {
        return;
    }
    eprintln!(
        "{name}: {} MB, {} lines",
        text.len() >> 20,
        text.lines().count()
    );
    let mut s = time("open", || {
        let mut s = Session::new("", Some("blank.txt"), 120, 40);
        // As the app starts: its default settings, breadcrumbs and all.
        s.send(Event::Settings(merlin_core::Settings::default()));
        s.send(Event::Opened {
            path: path.into(),
            text: text.clone(),
        });
        s.send(Event::GitBase {
            path: path.into(),
            text: Some(text),
        });
        s
    });
    let mut slow = Vec::new();
    time("first frame", || s.screen());
    frame(&s, "second frame", None, &mut slow);
    time("type a char", || s.run("type x"));
    frame(&s, "frame after typing", None, &mut slow);
    time("type 10 chars", || s.run("type abcdefghij"));
    time("go to end", || s.run("key ctrl+end"));
    frame(&s, "frame at end", None, &mut slow);
    time("undo", || s.run("key ctrl+z"));
    time("select all + copy", || s.run("key ctrl+a ctrl+c"));
    time("save", || s.run("key ctrl+s"));
    time("page down x5", || {
        s.run("key ctrl+home pagedown pagedown pagedown pagedown pagedown")
    });
    frame(&s, "frame", None, &mut slow);
    time("ctrl+d x3", || {
        s.run("key ctrl+home ctrl+right ctrl+d ctrl+d ctrl+d")
    });
    frame(&s, "frame", None, &mut slow);
    time("type over 4 carets", || s.run("type Z"));
    time("escape + undo", || s.run("key esc ctrl+z"));
    time("find open + type", || s.run("key ctrl+f; type quick"));
    frame(&s, "frame with find", None, &mut slow);
    time("find next", || s.run("key enter"));
    frame(&s, "frame after find next", None, &mut slow);
    time("find previous", || s.run("key up up"));
    time("find a rare word", || s.run("type zq"));
    time("find next, wrapping", || s.run("key enter"));
    time("find no word", || {
        s.run("key backspace backspace; type QQQ")
    });
    time("close find", || s.run("key esc"));
    time("paste 1MB", || {
        s.send(Event::Paste("pasted text\n".repeat(1 << 16)))
    });
    frame(&s, "frame", None, &mut slow);
    time("delete line", || s.run("key ctrl+shift+k"));
    time("minimap on", || s.run("cmd view.minimap"));
    frame(
        &s,
        "frame with minimap",
        Some("step 4: the minimap builds every line"),
        &mut slow,
    );
    time("type with minimap", || s.run("type m"));
    frame(
        &s,
        "frame with minimap",
        Some("step 4: the minimap builds every line"),
        &mut slow,
    );
    time("minimap off", || s.run("cmd view.minimap"));
    time("wrap on", || s.run("cmd view.wrap"));
    frame(&s, "frame wrapped", None, &mut slow);
    time("type wrapped", || s.run("type w"));
    frame(&s, "frame wrapped", None, &mut slow);
    time("go to end wrapped", || s.run("key ctrl+end"));
    frame(&s, "frame wrapped", None, &mut slow);
    time("wrap off", || s.run("cmd view.wrap"));
    assert!(
        slow.is_empty(),
        "{name}: frames over {FRAME_LIMIT} ms: {slow:?}"
    );
}

#[test]
#[ignore]
fn large_files() {
    let line = "the quick brown fox jumps over the lazy dog 0123456789\n";
    probe("plain 100MB", "big.txt", line.repeat(100 << 20 >> 6));
    let rust = "fn main() {\n    let x = vec![1, 2, 3];\n    println!(\"{x:?}\");\n}\n";
    probe("rust 20MB", "big.rs", rust.repeat(20 << 20 >> 6));
    let json = format!("[{}]", "{\"a\":1,\"b\":\"text\"},".repeat(20 << 20 >> 4));
    probe("json one line 20MB", "big.json", json);
    let log = "2026-11-17T10:00:00Z INFO request served path=/api/x status=200 took=3ms\n";
    probe("log 300MB", "big.log", log.repeat(300 << 20 >> 6));
}
