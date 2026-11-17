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
    time("first frame", || s.screen());
    time("second frame", || s.screen());
    time("type a char", || s.run("type x"));
    time("frame after typing", || s.screen());
    time("type 10 chars", || s.run("type abcdefghij"));
    time("go to end", || s.run("key ctrl+end"));
    time("frame at end", || s.screen());
    time("undo", || s.run("key ctrl+z"));
    time("select all + copy", || s.run("key ctrl+a ctrl+c"));
    time("save", || s.run("key ctrl+s"));
    time("page down x5", || {
        s.run("key ctrl+home pagedown pagedown pagedown pagedown pagedown")
    });
    time("frame", || s.screen());
    time("ctrl+d x3", || {
        s.run("key ctrl+home ctrl+right ctrl+d ctrl+d ctrl+d")
    });
    time("frame", || s.screen());
    time("type over 4 carets", || s.run("type Z"));
    time("escape + undo", || s.run("key esc ctrl+z"));
    time("find open + type", || s.run("key ctrl+f; type quick"));
    time("frame with find", || s.screen());
    time("find next", || s.run("key enter"));
    time("close find", || s.run("key esc"));
    time("paste 1MB", || {
        s.send(Event::Paste("pasted text\n".repeat(1 << 16)))
    });
    time("frame", || s.screen());
    time("delete line", || s.run("key ctrl+shift+k"));
    time("minimap on", || s.run("cmd view.minimap"));
    time("frame with minimap", || s.screen());
    time("type with minimap", || s.run("type m"));
    time("frame with minimap", || s.screen());
    time("minimap off", || s.run("cmd view.minimap"));
    time("wrap on", || s.run("cmd view.wrap"));
    time("frame wrapped", || s.screen());
    time("type wrapped", || s.run("type w"));
    time("frame wrapped", || s.screen());
    time("go to end wrapped", || s.run("key ctrl+end"));
    time("frame wrapped", || s.screen());
    time("wrap off", || s.run("cmd view.wrap"));
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
