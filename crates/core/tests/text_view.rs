//! The text as data, for windows that draw it themselves: lines with their
//! colours and styles, and the cursors in columns.

use merlin_core::screen::Style;
use merlin_core::script::Session;

fn rust(text: &str) -> Session {
    Session::new(text, Some("main.rs"), 60, 10)
}

#[test]
fn lines_on_screen_with_their_numbers() {
    let s = rust("fn main() {\n    let x = 1;\n}\n");
    let views = s.editor.text_views();
    let view = views.last().unwrap();
    let texts: Vec<String> = view
        .lines
        .iter()
        .map(|l| l.chars.iter().collect())
        .collect();
    assert_eq!(texts, ["fn main() {", "    let x = 1;", "}", ""]);
    assert_eq!(view.lines[1].number.trim(), "2");
    assert_eq!(view.lines[1].row, 1);
    assert!(!view.beyond.is_empty(), "rows past the end");
}

#[test]
fn carets_are_in_lines_and_columns() {
    let mut s = rust("one\ntwo\nthree\n");
    s.run("key down right right; key shift+alt+down");
    let views = s.editor.text_views();
    let carets = &views.last().unwrap().carets;
    assert_eq!((carets[0].line, carets[0].col), (2, 2), "the primary first");
    assert_eq!((carets[1].line, carets[1].col), (1, 2));
    assert_eq!(carets[0].cell, Some((2, 2)));
}

#[test]
fn selections_and_the_current_line_are_styles() {
    let mut s = rust("abc def\nxyz\n");
    s.run("key shift+ctrl+right");
    let views = s.editor.text_views();
    let line = &views.last().unwrap().lines[0];
    assert_eq!(&line.styles[..3], [Style::Selection; 3]);
    assert_eq!(
        line.styles[3],
        Style::Normal,
        "no current line with a selection"
    );
    s.run("key down");
    let views = s.editor.text_views();
    let line = &views.last().unwrap().lines[1];
    assert_eq!(line.base, Style::CurrentLine);
}

#[test]
fn scrolled_and_folded_views_start_and_skip_where_the_screen_does() {
    let text: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let mut s = rust(&text);
    s.run("key ctrl+end");
    let views = s.editor.text_views();
    let view = views.last().unwrap();
    assert_eq!(view.lines[0].line, view.scroll);
    assert!(view.scroll > 0);

    let mut s = rust("fn a() {\n    x\n    y\n}\nz\n");
    s.run("key ctrl+shift+[");
    let views = s.editor.text_views();
    let view = views.last().unwrap();
    assert!(view.lines[0].folded);
    assert_eq!(
        view.lines[1].line, 3,
        "the folded lines are skipped to the closer"
    );
}

#[test]
fn a_review_side_by_side_is_two_views_old_then_new() {
    let mut s = Session::new("fn a() {}\nfn b() {}\n", Some("src/main.rs"), 150, 14);
    s.editor.set_windowed(true);
    s.editor.set_own_text(true);
    s.send(merlin_core::Event::Resize(150, 14));
    s.send(merlin_core::Event::ShowReview {
        plugin: "agent".into(),
        id: "e1".into(),
        path: "src/main.rs".into(),
        old: "fn a() {}\nfn b() {}\n".into(),
        new: "fn a() {}\nfn B() {}\n".into(),
    });
    let views = s.editor.text_views();
    assert_eq!(views.len(), 2, "two halves");
    let text = |v: &merlin_core::TextView| -> Vec<String> {
        v.lines.iter().map(|l| l.chars.iter().collect()).collect()
    };
    assert!(text(&views[0]).contains(&"fn b() {}".to_string()));
    assert!(text(&views[1]).contains(&"fn B() {}".to_string()));
    let removed = views[0]
        .lines
        .iter()
        .find(|l| l.chars.contains(&'b'))
        .unwrap();
    assert_eq!(removed.base, Style::DiffRemoved);
    assert!(
        views[0].area.right() < views[1].area.x + 1,
        "left, then right"
    );
    assert!(!s.screen().contains("fn B()"), "not in the cells too");
}

#[test]
fn a_touchpad_scrolls_by_exact_lines() {
    use merlin_core::{Event, Mouse, MouseKind};
    let text: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    let mut s = rust(&text);
    let area = s.editor.text_views().last().unwrap().area;
    let scroll = |s: &mut Session, lines: i16| {
        s.editor.update(Event::Mouse(Mouse {
            kind: MouseKind::ScrollLines(lines),
            x: area.x + 10,
            y: area.y + 2,
            shift: false,
            alt: false,
            ctrl: false,
            clicks: 1,
        }));
        s.editor.text_views().last().unwrap().scroll
    };
    assert_eq!(scroll(&mut s, 1), 1);
    assert_eq!(scroll(&mut s, 4), 5);
    assert_eq!(scroll(&mut s, -2), 3);
    // No further up than the top.
    assert_eq!(scroll(&mut s, -10), 0);
}

/// A window drawing the text itself, `rows` high, as the GUI has it.
fn window(text: &str, rows: u16) -> Session {
    use merlin_core::Event;
    let mut s = Session::new(text, Some("notes.txt"), 60, rows);
    s.editor.set_windowed(true);
    s.editor.set_own_text(true);
    s.send(Event::Viewports(vec![(60, rows)]));
    s
}

fn pointer(group: usize, kind: merlin_core::MouseKind, row: f32, col: f32) -> merlin_core::Event {
    merlin_core::Event::Pointer(merlin_core::Pointer {
        group,
        kind,
        row,
        col,
        shift: false,
        alt: false,
        ctrl: false,
        clicks: 1,
    })
}

#[test]
fn scrolling_goes_by_parts_of_a_row() {
    use merlin_core::Event;
    let text: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    let mut s = window(&text, 10);
    s.send(Event::Scroll {
        group: 0,
        rows: 2.5,
        cols: 0.0,
    });
    let view = s.editor.text_views().pop().unwrap();
    assert_eq!((view.scroll, view.top_frac, view.top_row), (2, 0.5, 2.5));
    // Rows above the view come too, to glide into.
    assert!(view.lines.iter().any(|l| l.row < 0));
    // No further up than the top.
    s.send(Event::Scroll {
        group: 0,
        rows: -10.0,
        cols: 0.0,
    });
    let view = s.editor.text_views().pop().unwrap();
    assert_eq!((view.scroll, view.top_frac), (0, 0.0));
    // The scrollbar and minimap put the top anywhere.
    s.send(Event::ScrollTo {
        group: 0,
        row: 40.25,
    });
    let view = s.editor.text_views().pop().unwrap();
    assert_eq!((view.scroll, view.top_frac), (40, 0.25));
}

#[test]
fn a_wrapped_line_scrolls_a_row_at_a_time() {
    use merlin_core::Event;
    let long = "word ".repeat(60);
    let mut s = window(&format!("{long}\nshort\n"), 6);
    s.run("cmd view.wrap");
    let rows = s.editor.text_views().pop().unwrap().lines[0].rows.len();
    assert!(rows > 3, "the line wraps");
    s.send(Event::Scroll {
        group: 0,
        rows: 2.0,
        cols: 0.0,
    });
    let view = s.editor.text_views().pop().unwrap();
    // Still the first line, two of its rows up.
    assert_eq!((view.scroll, view.top_row), (0, 2.0));
    assert_eq!(view.lines[0].row, -2);
}

#[test]
fn the_pointer_clicks_by_rows_and_columns_of_a_group() {
    use merlin_core::MouseKind;
    let mut s = window("alpha\nbravo\ncharlie\n", 10);
    let gutter = s.editor.text_views().pop().unwrap().gutter as f32;
    // Nearer the gap after "br" than before it.
    s.send(pointer(0, MouseKind::Down, 1.2, gutter + 1.6));
    s.send(pointer(0, MouseKind::Up, 1.2, gutter + 1.6));
    assert_eq!(s.editor.cursor_utf16(), (1, 2));
    // Half a row scrolled off: the same place is the next row down.
    s.send(merlin_core::Event::Scroll {
        group: 0,
        rows: 0.5,
        cols: 0.0,
    });
    s.send(pointer(0, MouseKind::Down, 1.7, gutter + 0.2));
    assert_eq!(s.editor.cursor_utf16(), (1, 0));
}

#[test]
fn a_press_in_another_group_focuses_it() {
    use merlin_core::{Event, MouseKind};
    let mut s = window("one\ntwo\n", 10);
    s.run("cmd view.split");
    s.send(Event::Viewports(vec![(30, 10), (30, 10)]));
    let gutter = s.editor.text_views().pop().unwrap().gutter as f32;
    assert!(s.editor.window_view().groups[1].focused);
    s.send(pointer(0, MouseKind::Down, 1.0, gutter + 1.0));
    s.send(pointer(0, MouseKind::Up, 1.0, gutter + 1.0));
    assert!(s.editor.window_view().groups[0].focused);
    assert_eq!(s.editor.cursor_utf16(), (1, 1));
    // The wheel scrolls a group without focusing it.
    s.send(Event::Scroll {
        group: 1,
        rows: 1.0,
        cols: 0.0,
    });
    assert!(s.editor.window_view().groups[0].focused);
    let right = s
        .editor
        .text_views()
        .into_iter()
        .find(|v| v.group == 1)
        .unwrap();
    assert_eq!(right.scroll, 1);
}
