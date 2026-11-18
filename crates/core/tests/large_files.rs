//! Files too large to load: a window at a time read from disk, numbered,
//! searched and gone through as one file; read-only, or edited in place.

use merlin_core::Event;
use merlin_core::script::Session;

/// A file of `lines` numbered lines, "line 0" first, open read-only as a
/// large file.
fn open(lines: usize) -> (tempfile::TempDir, Session, String) {
    open_as(lines, false)
}

/// The same, edited in place.
fn edit(lines: usize) -> (tempfile::TempDir, Session, String) {
    open_as(lines, true)
}

fn open_as(lines: usize, editable: bool) -> (tempfile::TempDir, Session, String) {
    let dir = tempfile::tempdir().unwrap();
    let full = dir.path().join("big.log");
    let text: String = (0..lines).map(|n| format!("line {n}\n")).collect();
    std::fs::write(&full, &text).unwrap();
    let mut s = Session::new("", None, 100, 20);
    s.send(Event::OpenedLarge {
        path: "big.log".into(),
        full,
        size: text.len() as u64,
        free: None,
        editable,
    });
    (dir, s, text)
}

fn status(s: &Session) -> String {
    s.editor
        .window_view()
        .status
        .iter()
        .map(|i| i.text.as_str())
        .collect()
}

#[test]
fn it_opens_read_only_at_the_top_with_its_lines_counted() {
    let (_dir, s, text) = open(600_000);
    assert!(s.screen().contains("line 0"), "{}", s.screen());
    let status = status(&s);
    assert!(status.contains("Read-only"), "{status}");
    assert!(status.contains("Ln 1, Col 1"), "{status}");
    // Only a window of it is held.
    assert!(s.text().len() < text.len() * 2 / 3);
}

#[test]
fn typing_asks_to_load_it_and_changes_nothing() {
    let (_dir, mut s, _) = open(600_000);
    let before = s.text();
    s.run("type hello");
    assert_eq!(s.text(), before);
    let toast = s.editor.window_view().toast.expect("a notice");
    assert!(toast.text.contains("read-only"), "{}", toast.text);
}

#[test]
fn ctrl_end_goes_to_the_files_end_with_its_numbers() {
    let (_dir, mut s, _) = open(600_000);
    s.run("key ctrl+end");
    let screen = s.screen();
    assert!(screen.contains("line 599999"), "{screen}");
    assert!(screen.contains("600000"), "numbered as the file: {screen}");
    s.run("key ctrl+home");
    assert!(s.screen().contains("line 0"), "{}", s.screen());
}

#[test]
fn go_to_line_reaches_any_line() {
    let (_dir, mut s, _) = open(600_000);
    s.run("cmd go.line; type 300000; key enter");
    assert!(s.screen().contains("line 299999"), "{}", s.screen());
    assert!(status(&s).contains("Ln 300,000"), "{}", status(&s));
}

#[test]
fn find_goes_to_a_match_past_the_window_and_counts_them_all() {
    let (_dir, mut s, _) = open(600_000);
    s.run("key ctrl+f; type line 477777");
    s.run("key enter");
    assert!(status(&s).contains("Ln 477,778"), "{}", status(&s));
    let find = s.editor.window_view().find.expect("find open");
    assert_eq!(find.total, 1);
    assert_eq!(find.counting, None);
    // Going on wraps round to the same one.
    s.run("key enter");
    assert!(status(&s).contains("Ln 477,778"), "{}", status(&s));
}

#[test]
fn dragging_the_scrollbar_reads_the_window_there() {
    let (_dir, mut s, _) = open(600_000);
    let group = 0;
    s.send(Event::ScrollTo {
        group,
        row: 500_000.0,
    });
    let screen = s.screen();
    assert!(screen.contains("line 500000"), "{screen}");
    let view = &s.editor.text_views()[0];
    assert!((view.top_row - 500_000.0).abs() < 1.0, "{}", view.top_row);
    assert_eq!(view.total_rows, 600_001);
}

#[test]
fn scrolling_on_reads_the_next_window_and_keeps_the_place() {
    let (_dir, mut s, _) = open(600_000);
    // A window is about 4 MB: some 350 000 of these lines. Scrolled
    // most of the way through it, the next is read around the view.
    for _ in 0..30 {
        s.send(Event::Scroll {
            group: 0,
            rows: 10_000.0,
            cols: 0.0,
        });
    }
    let screen = s.screen();
    assert!(screen.contains("line 300000"), "{screen}");
    assert!(screen.contains("300001"), "numbered: {screen}");
    let view = &s.editor.text_views()[0];
    assert!((view.top_row - 300_000.0).abs() < 1.0, "{}", view.top_row);
}

#[test]
fn saving_never_writes_the_window_over_the_file() {
    use merlin_core::Effect;
    let (_dir, mut s, _) = open(600_000);
    s.effects.clear();
    s.run("key ctrl+s");
    assert!(!s.effects.iter().any(|e| matches!(e, Effect::Save { .. })));
}

#[test]
fn a_log_that_grows_is_followed_at_the_end() {
    use std::io::Write;
    let (dir, mut s, text) = open(600_000);
    s.run("key ctrl+end");
    let full = dir.path().join("big.log");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&full)
        .unwrap();
    file.write_all(b"line 600000\nline 600001\n").unwrap();
    s.send(Event::LargeResized {
        path: "big.log".into(),
        size: text.len() as u64 + 24,
    });
    let screen = s.screen();
    assert!(screen.contains("line 600001"), "{screen}");
    assert!(screen.contains("600002"), "numbered: {screen}");
}

#[test]
fn a_log_that_grows_away_from_the_end_says_so() {
    use std::io::Write;
    let (dir, mut s, text) = open(600_000);
    let full = dir.path().join("big.log");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&full)
        .unwrap();
    file.write_all(b"line 600000\n").unwrap();
    s.send(Event::LargeResized {
        path: "big.log".into(),
        size: text.len() as u64 + 12,
    });
    assert!(s.screen().contains("line 0"), "stays put: {}", s.screen());
    let bar = &s.editor.window_view().groups[0].bar;
    let bar = bar.as_ref().expect("the bar");
    assert!(bar.text.contains("grew"), "{}", bar.text);
    // Go to End goes there, the new line and all.
    s.run("cmd large.end");
    assert!(s.screen().contains("line 600000"), "{}", s.screen());
}

fn tab_dirty(s: &Session) -> bool {
    s.editor
        .window_view()
        .tabs
        .iter()
        .any(|t| t.active && t.dirty)
}

#[test]
fn typing_edits_it_in_place() {
    let (_dir, mut s, _) = edit(600_000);
    s.run("type hello");
    assert!(s.screen().contains("helloline 0"), "{}", s.screen());
    assert!(s.editor.window_view().toast.is_none(), "no question");
    assert!(tab_dirty(&s));
    assert!(status(&s).contains("Edited from disk"), "{}", status(&s));
}

#[test]
fn an_edit_stays_when_the_view_goes_away_and_back() {
    let (_dir, mut s, _) = edit(600_000);
    s.run("type EDIT");
    s.run("key ctrl+end");
    assert!(s.screen().contains("line 599999"), "{}", s.screen());
    assert!(tab_dirty(&s), "still unsaved past the window");
    s.run("key ctrl+home");
    assert!(s.screen().contains("EDITline 0"), "{}", s.screen());
}

#[test]
fn lines_added_number_the_rest_of_the_file() {
    let (_dir, mut s, _) = edit(600_000);
    s.run("key enter; key enter; key enter");
    s.run("key ctrl+end");
    assert!(s.screen().contains("600003"), "{}", s.screen());
    s.run("cmd go.line; type 300004; key enter");
    assert!(s.screen().contains("line 300000"), "{}", s.screen());
    assert!(status(&s).contains("Ln 300,004"), "{}", status(&s));
}

#[test]
fn undo_goes_back_through_windows_in_turn() {
    let (_dir, mut s, _) = edit(600_000);
    s.run("type A");
    s.run("key ctrl+end; type Z");
    assert!(s.screen().contains("Z"), "{}", s.screen());
    // The last edit first, at the end, then the one at the top.
    s.run("cmd edit.undo");
    assert!(!s.screen().contains("Z"), "{}", s.screen());
    s.run("cmd edit.undo");
    assert!(s.screen().contains("line 0"), "{}", s.screen());
    assert!(!s.screen().contains("Aline 0"), "{}", s.screen());
    assert!(!tab_dirty(&s), "back as it was");
    s.run("cmd edit.redo");
    assert!(s.screen().contains("Aline 0"), "{}", s.screen());
    s.run("cmd edit.redo");
    s.run("key ctrl+end");
    assert!(s.screen().contains("Z"), "{}", s.screen());
}

#[test]
fn find_finds_what_was_typed_from_anywhere() {
    let (_dir, mut s, _) = edit(600_000);
    s.run("type needle ");
    s.run("key ctrl+end");
    s.run("key ctrl+f; type needle; key enter");
    assert!(status(&s).contains("Ln 1,"), "{}", status(&s));
    assert_eq!(s.editor.window_view().find.unwrap().total, 1);
}

#[test]
fn saving_writes_every_edit_and_reads_the_new_file() {
    use merlin_core::Effect;
    let (dir, mut s, text) = edit(600_000);
    s.run("type TOP");
    s.run("key ctrl+end; type END");
    s.effects.clear();
    s.run("key ctrl+s");
    assert!(
        !s.effects.iter().any(|e| matches!(e, Effect::Save { .. })),
        "never the window as a whole file"
    );
    let on_disk = std::fs::read_to_string(dir.path().join("big.log")).unwrap();
    assert!(on_disk == format!("TOP{text}END"), "both edits written");
    assert!(!tab_dirty(&s), "saved");
    // Numbered as before, read from the new file.
    assert!(s.screen().contains("600001 END"), "{}", s.screen());
    s.run("key ctrl+home");
    assert!(s.screen().contains("1 TOPline 0"), "{}", s.screen());
    // No temporary file left beside it.
    let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(names.len(), 1);
}

#[test]
fn undo_goes_on_past_a_save() {
    let (dir, mut s, text) = edit(600_000);
    s.run("type A");
    s.run("key ctrl+end; type Z");
    s.run("key ctrl+s");
    s.run("cmd edit.undo; cmd edit.undo");
    assert!(tab_dirty(&s), "changed from what was saved");
    assert!(
        s.screen().contains("line 0") && !s.screen().contains("Aline 0"),
        "{}",
        s.screen()
    );
    s.run("key ctrl+s");
    let on_disk = std::fs::read_to_string(dir.path().join("big.log")).unwrap();
    assert!(on_disk == text, "back as it was");
}

#[test]
fn a_file_with_nothing_changed_isnt_written() {
    use merlin_core::Effect;
    let (_dir, mut s, _) = edit(600_000);
    s.effects.clear();
    s.run("key ctrl+s");
    assert!(
        !s.effects
            .iter()
            .any(|e| matches!(e, Effect::SaveLarge { .. }))
    );
}

#[test]
fn find_counts_only_whats_new_after_an_edit() {
    use merlin_core::Effect;
    use merlin_core::bigfile::Part;
    use merlin_core::work::Kind;
    let (_dir, mut s, text) = edit(600_000);
    s.run("key ctrl+f; type line 4242");
    let find = s.editor.window_view().find.unwrap();
    let expected = text.matches("line 4242").count();
    assert_eq!(find.total, expected);
    // Back in the text, with Find still open: typing one more match.
    s.run("click 20 5");
    s.effects.clear();
    s.run("type line 4242");
    let parts: Vec<Part> = s
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Work(job) => match &job.kind {
                Kind::FileCount { parts, .. } => Some(parts.clone()),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .collect();
    assert!(!parts.is_empty(), "the new text counted");
    assert!(
        !parts.iter().any(|p| matches!(p, Part::Block(_))),
        "the file's blocks not again: {parts:?}"
    );
    let find = s.editor.window_view().find.unwrap();
    assert_eq!(find.total, expected + 1);
    assert_eq!(find.counting, None);
    // Away and back, and a jump past the window: still right.
    s.run("key ctrl+end");
    assert_eq!(s.editor.window_view().find.unwrap().total, expected + 1);
}
