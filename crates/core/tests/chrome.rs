//! Milestone 6: the parts around the text as VS Code has them, the activity
//! bar, title bar, panel, notices and status bar, and what's drawn over
//! code.

use merlin_core::screen::{Hue, Style, Surface};
use merlin_core::script::Session;
use merlin_core::{Diagnostic, Effect, Event, Focus, Severity};

fn project() -> Session {
    let mut s = Session::new("fn a() {\n    b(1);\n}\n", Some("src/main.rs"), 100, 24);
    s.files.insert("src/main.rs".into(), s.text());
    s.files
        .insert("src/lib.rs".into(), "pub fn b() {}\n".into());
    s
}

/// Where `needle` first appears on screen, in cells.
fn find(s: &Session, needle: &str) -> (u16, u16) {
    let screen = s.screen();
    for (y, line) in screen.lines().enumerate() {
        if let Some(i) = line.find(needle) {
            return (line[..i].chars().count() as u16, y as u16);
        }
    }
    panic!("{needle:?} isn't on screen:\n{screen}");
}

fn problem(s: &mut Session) {
    s.send(Event::SetDiagnostics {
        plugin: "lsp".into(),
        path: "src/lib.rs".into(),
        diagnostics: vec![Diagnostic {
            start: (0, 7),
            end: (0, 9),
            severity: Severity::Error,
            message: "something is wrong".into(),
        }],
    });
}

#[test]
fn the_activity_bar_switches_views_and_hides_the_one_showing() {
    let mut s = project();
    // The search icon in the activity bar, not the command center's.
    let (x, y) = (1, find(&s, " ⌕    3").1);
    s.run(&format!("click {x} {y}"));
    assert!(s.screen().contains("SEARCH"), "{}", s.screen());
    s.run(&format!("click {x} {y}"));
    assert!(!s.editor.side_bar_visible(), "a second click hides it");
}

#[test]
fn the_source_control_icon_counts_changes() {
    let mut s = project();
    s.send(Event::GitStatus {
        branch: Some("main".into()),
        files: vec![("src/main.rs".into(), 'M'), ("x".into(), '?')],
        staged: vec![],
        changed: vec![("src/main.rs".into(), 'M'), ("x".into(), '?')],
    });
    assert!(s.screen().contains("⑂ 2"), "{}", s.screen());
}

#[test]
fn problems_list_by_file_and_open_at_their_line() {
    let mut s = project();
    problem(&mut s);
    s.run("key ctrl+shift+m");
    let screen = s.screen();
    assert!(screen.contains("PROBLEMS  1"), "{screen}");
    assert!(screen.contains("◆ lib.rs src"), "{screen}");
    assert!(
        screen.contains("⊗ something is wrong [Ln 1, Col 8]"),
        "{screen}"
    );
    assert_eq!(s.editor.focus(), Focus::Bottom);
    let (x, y) = find(&s, "something is wrong");
    s.run(&format!("click {x} {y}"));
    assert_eq!(s.text(), "pub fn b() {}\n");
    assert_eq!(s.editor.cursor_utf16(), (0, 7));
}

#[test]
fn ctrl_j_toggles_the_panel() {
    let mut s = project();
    s.run("key ctrl+j");
    assert!(s.editor.bottom_visible());
    assert!(
        s.screen().contains("No problems have been detected"),
        "{}",
        s.screen()
    );
    s.run("key ctrl+j");
    assert!(!s.editor.bottom_visible());
}

#[test]
fn messages_collect_in_output() {
    let mut s = project();
    s.run("key ctrl+s");
    s.run("cmd view.output");
    assert!(
        s.screen().contains("[info] Saved src/main.rs"),
        "{}",
        s.screen()
    );
}

#[test]
fn a_failed_plugin_offers_its_output() {
    let mut s = project();
    s.send(Event::PluginFailed {
        plugin: "lsp".into(),
        message: "exited".into(),
    });
    let (x, y) = find(&s, "Show Output");
    s.run(&format!("click {x} {y}"));
    assert!(s.editor.bottom_visible());
    assert!(
        s.screen().contains("[error] Plugin lsp failed: exited"),
        "{}",
        s.screen()
    );
}

#[test]
fn the_status_bar_opens_problems_and_go_to_line() {
    let mut s = project();
    let (x, y) = find(&s, "⊗ 0");
    s.run(&format!("click {x} {y}"));
    assert!(s.editor.bottom_visible());
    let (x, y) = find(&s, "Ln 1, Col 1");
    s.run(&format!("click {x} {y}"));
    assert_eq!(s.editor.focus(), Focus::Palette);
}

#[test]
fn the_command_center_opens_go_to_file() {
    let mut s = project();
    let (x, y) = find(&s, "⌕ merlin");
    s.run(&format!("click {} {y}", x + 2));
    assert!(s.effects.contains(&Effect::ListFiles));
}

#[test]
fn layout_buttons_toggle_the_side_bar_and_panel() {
    let mut s = project();
    let (x, y) = find(&s, "⬓");
    s.run(&format!("click {x} {y}"));
    assert!(s.editor.bottom_visible());
    let (x, y) = find(&s, "◧");
    s.run(&format!("click {x} {y}"));
    assert!(s.editor.side_bar_visible());
}

#[test]
fn the_explorer_counts_problems_and_colours_names() {
    let mut s = project();
    problem(&mut s);
    s.run("key ctrl+shift+e");
    let screen = s.screen();
    assert!(screen.contains("◆ lib.rs"), "{screen}");
    let (x, y) = find(&s, "lib.rs");
    let render = s.editor.render();
    assert_eq!(render.cell(x, y).style, Style::Error);
    assert_eq!(
        render.cell(x - 2, y).tint,
        Some(Hue::Orange),
        "the Rust icon"
    );
    let row: String = s.screen().lines().nth(y as usize).unwrap().to_string();
    let after = &row[row.find("lib.rs").unwrap() + 6..];
    assert!(
        after.trim_start().starts_with("1 "),
        "a count at the right: {row}"
    );
}

#[test]
fn rows_and_tabs_light_up_under_the_mouse() {
    let mut s = project();
    s.run("key ctrl+shift+e; key esc");
    let (x, y) = find(&s, "lib.rs");
    s.run(&format!("move {x} {y}"));
    assert_eq!(s.editor.render().cell(x, y).surface, Surface::Hover);
    s.run("move 60 10");
    assert_eq!(s.editor.render().cell(x, y).surface, Surface::Bar);
}

#[test]
fn only_the_active_or_hovered_tab_shows_its_close_button() {
    let mut s = project();
    s.run("key ctrl+p; type lib; key enter");
    assert!(
        s.screen().contains("◆ main.rs    ◆ lib.rs ×"),
        "{}",
        s.screen()
    );
    let (x, y) = find(&s, "main.rs");
    s.run(&format!("move {x} {y}"));
    assert!(
        s.screen().contains("◆ main.rs ×  ◆ lib.rs ×"),
        "{}",
        s.screen()
    );
}

#[test]
fn guides_brackets_and_the_word_under_the_cursor() {
    let mut s = Session::new(
        "fn a() {\n    if b(1) {\n        c();\n    }\n    c();\n}\n",
        Some("main.rs"),
        80,
        16,
    );
    let (x, y) = s.editor.text_origin();
    let render = s.editor.render();
    // Line 3, indented twice: a guide at each level, the inner one the
    // cursor's... not yet: the cursor is on line 1, whose block is the fn.
    assert_eq!(render.cell(x, y + 2).ch, '│');
    assert_eq!(render.cell(x, y + 2).tint, Some(Hue::GuideActive));
    assert_eq!(render.cell(x + 4, y + 2).tint, Some(Hue::Guide));
    // `{` at depth 0 and `(` inside `if b(` at depth 1.
    assert_eq!(render.cell(x + 7, y).tint, Some(Hue::Bracket1));
    assert_eq!(render.cell(x + 8, y + 1).tint, Some(Hue::Bracket2));
    s.run("key down down; key end; key left left left");
    let render = s.editor.render();
    // The cursor is on `c`; the other `c` is marked.
    assert_eq!(render.cell(x + 4, y + 4).style, Style::WordMatch);
    assert_eq!(render.cell(x + 4, y + 1).style, Style::Normal);
}

fn is_braille(c: char) -> bool {
    ('\u{2801}'..='\u{28ff}').contains(&c)
}

#[test]
fn the_minimap_is_an_option_drawn_in_braille() {
    let code: String = (0..200)
        .map(|i| format!("fn f{i}() {{ let x = {i}; }}\n"))
        .collect();
    let mut s = Session::new(&code, Some("main.rs"), 100, 24);
    assert!(!s.screen().chars().any(is_braille), "off by default");
    s.run("cmd view.minimap");
    let render = s.editor.render();
    let (_, y) = s.editor.text_origin();
    // Left of the scrollbar, in the last ten columns before it.
    let row: Vec<char> = (89..99).map(|x| render.cell(x, y).ch).collect();
    assert!(row.iter().any(|&c| is_braille(c)), "{row:?}");
    assert_eq!(render.cell(89, y).style, Style::MinimapView);
    assert_eq!(render.cell(89, y + 10).style, Style::Minimap);
    // Code no longer runs under it.
    assert!(render.cell(87, y).ch == ' ' || !is_braille(render.cell(87, y).ch));
    // A click on row 15 brings its lines, 60 to 63, into the middle.
    s.run(&format!("click 92 {}", y + 15));
    assert!(s.screen().contains("fn f62()"), "{}", s.screen());
    assert!(!s.screen().contains("fn f0()"), "{}", s.screen());
    // Too narrow, and it folds away.
    s.run("resize 60x24");
    assert!(!s.screen().chars().any(is_braille));
}

#[test]
fn the_minimap_and_scrollbar_mark_matches_problems_and_changes() {
    let code: String = (0..200)
        .map(|i| format!("fn f{i}() {{ let x = {i}; }}\n"))
        .collect();
    let mut s = Session::new(&code, Some("main.rs"), 100, 24);
    s.run("cmd view.minimap");
    let (_, y) = s.editor.text_origin();
    let base = code.replacen("fn f1()", "fn g1()", 1);
    s.send(Event::GitBase {
        path: "main.rs".into(),
        text: Some(base),
    });
    s.send(Event::SetDiagnostics {
        plugin: "lsp".into(),
        path: "main.rs".into(),
        diagnostics: vec![Diagnostic {
            start: (5, 0),
            end: (5, 2),
            severity: Severity::Error,
            message: "bad".into(),
        }],
    });
    let render = s.editor.render();
    // Line 1 changed: a sign in the gap left of the dots.
    assert_eq!(render.cell(88, y).style, Style::GitChanged);
    // Line 5's error colours the dots of row 1, columns 0-3.
    assert_eq!(render.cell(89, y + 1).tint, Some(Hue::Problem));
    // The match is on line 150, below the find box; the scrollbar marks it.
    s.run("key ctrl+f; type f150()");
    let render = s.editor.render();
    let bar = 99;
    let marked: Vec<u16> = (y..y + 20)
        .filter(|&r| render.cell(bar, r).style == Style::ScrollMatch)
        .collect();
    assert_eq!(marked.len(), 1, "{marked:?}");
    // The view jumped to it, and the minimap shows it on its own row.
    let found =
        (y..y + 20).any(|r| (89..99).any(|x| render.cell(x, r).style == Style::MinimapMatch));
    assert!(found, "{}", s.screen());
}
