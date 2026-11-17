//! What the project leaves out, from the editor's side: the Explorer's
//! Exclude and ⊘, Search's boxes, and the palette's symbols held back.

use merlin_core::exclude::{Exclusions, Why};
use merlin_core::lang::{ProjectSymbol, SymbolsNote};
use merlin_core::script::Session;
use merlin_core::{Effect, Event, Node, SearchAsk, Settings, Side, StateTone, UiEvent};

fn windowed() -> Session {
    let mut s = Session::new("fn a() {}\n", Some("src/main.rs"), 100, 24);
    s.files.insert("src/main.rs".into(), s.text());
    s.send(Event::Resize(100, 24));
    s
}

/// The Explorer showing `files`, and the target index of row `label`.
fn explorer(s: &mut Session, files: &[&str], label: &str) -> usize {
    s.send(Event::Ui(UiEvent::Rail(0)));
    s.send(Event::Tree(files.iter().map(|f| f.to_string()).collect()));
    row(s, label)
}

fn row(s: &Session, label: &str) -> usize {
    let left = s.editor.window_view().left.expect("the Explorer shows");
    let i = left
        .nodes
        .iter()
        .position(|n| matches!(n, Node::Entry(e) if e.label == label))
        .unwrap_or_else(|| panic!("{label} is listed"));
    left.targets[i].unwrap()
}

fn menu(s: &mut Session, index: usize) -> Vec<String> {
    s.send(Event::Ui(UiEvent::TargetMenu {
        side: Side::Left,
        index,
    }));
    let menu = s.editor.window_view().context.expect("a menu");
    menu.items.iter().map(|i| i.label.clone()).collect()
}

fn run_item(s: &mut Session, label: &str) {
    let menu = s.editor.window_view().context.expect("a menu");
    let i = menu.items.iter().position(|i| i.label == label).unwrap();
    s.send(Event::Ui(UiEvent::ContextItem(i)));
}

#[test]
fn a_folder_is_excluded_from_its_menu_and_the_notice_undoes_it() {
    let mut s = windowed();
    let index = explorer(&mut s, &["src/main.rs", "third_party/z.c"], "third_party");
    let items = menu(&mut s, index);
    for item in [
        "Find in Folder…",
        "Exclude from Project",
        "Exclude from Search Only",
        "Add to .gitignore",
    ] {
        assert!(items.contains(&item.to_string()), "{items:?}");
    }
    assert!(!items.contains(&"Include in Project".to_string()));
    s.effects.clear();
    run_item(&mut s, "Exclude from Project");
    assert!(s.effects.contains(&Effect::Exclude {
        pattern: "third_party/".into(),
        search: false,
    }));
    let toast = s.editor.window_view().toast.expect("a notice");
    assert!(
        toast.text.starts_with("Excluded third_party/"),
        "{}",
        toast.text
    );
    assert_eq!(toast.buttons, ["Open Exclusions", "Undo"]);
    s.effects.clear();
    s.run("cmd explorer.undo_exclude");
    assert_eq!(
        s.effects,
        [Effect::Include(vec![("third_party/".into(), false)])]
    );
}

#[test]
fn what_is_left_out_shows_dimmed_with_why_and_can_come_back() {
    let mut s = windowed();
    s.send(Event::Settings(Settings {
        exclusions: Exclusions {
            files: vec!["third_party/".into()],
            ..Exclusions::default()
        },
        ..Settings::default()
    }));
    explorer(&mut s, &["src/main.rs"], "src");
    s.effects.clear();
    s.run("cmd explorer.toggle_excluded");
    assert!(s.effects.contains(&Effect::ShowLeftOut(true)));
    let left = s.editor.window_view().left.unwrap();
    assert!(
        left.toolbar
            .iter()
            .any(|t| t.command == "explorer.toggle_excluded" && t.on)
    );
    s.send(Event::LeftOut {
        folder: None,
        entries: vec![
            ("build/".into(), Why::GitIgnore),
            (".cache/".into(), Why::Hidden),
            ("third_party/".into(), Why::Excluded),
        ],
    });
    let left = s.editor.window_view().left.unwrap();
    let mark = |label: &str| {
        left.nodes.iter().find_map(|n| match n {
            Node::Entry(e) if e.label == label => Some(e.badge.clone()),
            _ => None,
        })
    };
    assert_eq!(mark("build").as_deref(), Some(".gitignore"));
    assert_eq!(mark(".cache").as_deref(), Some("hidden"));
    assert_eq!(mark("third_party").as_deref(), Some("excluded"));
    // Opening a left-out folder lists it, as the walk didn't.
    let index = row(&s, "third_party");
    s.effects.clear();
    s.send(Event::Ui(UiEvent::Target {
        side: Side::Left,
        index,
        clicks: 1,
    }));
    assert!(
        s.effects
            .contains(&Effect::ListLeftOut("third_party".into(), Why::Excluded))
    );
    s.send(Event::LeftOut {
        folder: Some("third_party".into()),
        entries: vec![("third_party/z.c".into(), Why::Excluded)],
    });
    assert_eq!(mark_of(&s, "z.c").as_deref(), Some("excluded"));
    // Its menu puts it back, by the rule that leaves it out.
    let index = row(&s, "third_party");
    let items = menu(&mut s, index);
    assert!(
        items.contains(&"Include in Project".to_string()),
        "{items:?}"
    );
    assert!(!items.contains(&"Exclude from Project".to_string()));
    s.effects.clear();
    run_item(&mut s, "Include in Project");
    assert_eq!(
        s.effects,
        [Effect::Include(vec![("third_party/".into(), false)])]
    );
    // And one git ignores offers .gitignore.
    let index = row(&s, "build");
    assert!(menu(&mut s, index).contains(&"Open .gitignore".to_string()));
}

fn mark_of(s: &Session, label: &str) -> Option<String> {
    let left = s.editor.window_view().left.unwrap();
    left.nodes.iter().find_map(|n| match n {
        Node::Entry(e) if e.label == label => Some(e.badge.clone()),
        _ => None,
    })
}

#[test]
fn search_takes_files_to_include_and_exclude() {
    let mut s = windowed();
    s.run(
        "key ctrl+shift+f; type radius; key down; type src/, include/; key down; type *_test.cpp",
    );
    s.effects.clear();
    s.run("key enter");
    let ask = SearchAsk {
        query: "radius".into(),
        include: "src/, include/".into(),
        exclude: "*_test.cpp".into(),
        use_exclusions: true,
    };
    assert_eq!(s.effects, [Effect::Search(ask.clone())]);
    s.send(Event::SearchResults {
        ask,
        hits: Vec::new(),
        skipped: 2,
    });
    assert!(
        s.screen().contains("No results · 2 excluded paths skipped"),
        "{}",
        s.screen()
    );
    // The switch searches everything, ignored files too.
    s.effects.clear();
    s.send(Event::Ui(UiEvent::Press("search:exclusions".into())));
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::Search(a) if !a.use_exclusions))
    );
}

fn symbol(name: &str, path: &str, scan: bool) -> ProjectSymbol {
    ProjectSymbol {
        name: name.into(),
        kind: "function".into(),
        container: String::new(),
        path: path.into(),
        line: 3,
        column: 4,
        scan,
    }
}

#[test]
fn the_palette_says_what_the_symbols_lack_and_shows_the_excluded() {
    let mut s = Session::new("", None, 100, 24);
    s.send(Event::RegisterSymbolSearch {
        plugin: "cpp".into(),
        command: "project-symbols".into(),
    });
    s.run("key ctrl+shift+p; key backspace; type @area");
    let (tick, _) = s.editor.palette_search_wait().expect("it waits");
    s.send(Event::PaletteSearch(tick));
    let came = |s: &mut Session, symbols, excluded, note| {
        s.send(Event::ProjectSymbols {
            plugin: "cpp".into(),
            query: "area".into(),
            symbols,
            excluded,
            note,
        })
    };
    // Still indexing: a spinner and Refresh, which asks again.
    came(
        &mut s,
        vec![symbol("area", "src/circle.cpp", false)],
        vec![symbol("z_area", "third_party/z.c", false)],
        SymbolsNote {
            indexing: "clangd is still indexing: 412 of 1,030 files".into(),
            ..SymbolsNote::default()
        },
    );
    let p = s.editor.window_view().palette.unwrap();
    let state = p.state.expect("a line under the results");
    assert_eq!(state.tone, StateTone::Busy);
    assert!(
        state
            .text
            .starts_with("clangd is still indexing: 412 of 1,030 files")
    );
    s.effects.clear();
    s.send(Event::Ui(UiEvent::Press("palette.state".into())));
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::SearchSymbols { .. }))
    );
    // Done, with symbols held back: counted, then shown dimmed.
    came(
        &mut s,
        vec![symbol("area", "src/circle.cpp", true)],
        vec![symbol("z_area", "third_party/z.c", false)],
        SymbolsNote::default(),
    );
    let p = s.editor.window_view().palette.unwrap();
    let state = p.state.expect("the excluded counted");
    assert_eq!(state.tone, StateTone::Quiet);
    assert_eq!(state.text, "1 more in excluded files: third_party/");
    assert_eq!(state.link.as_deref(), Some("Show Them"));
    assert_eq!(p.extras[0].chip.as_deref(), Some("SCAN"));
    s.send(Event::Ui(UiEvent::Press("palette.state".into())));
    let p = s.editor.window_view().palette.unwrap();
    assert!(p.state.is_none());
    let z = p
        .items
        .iter()
        .position(|(l, _)| l.contains("z_area"))
        .expect("shown");
    assert_eq!(p.extras[z].chip.as_deref(), Some("excluded"));
    assert!(p.extras[z].muted);
    // A warning's button runs its command.
    s.run("key backspace");
    let (tick, _) = s.editor.palette_search_wait().expect("it waits");
    s.send(Event::PaletteSearch(tick));
    s.send(Event::ProjectSymbols {
        plugin: "cpp".into(),
        query: "are".into(),
        symbols: Vec::new(),
        excluded: Vec::new(),
        note: SymbolsNote {
            warning: "No compile_commands.json".into(),
            action: Some(("Configure with CMake".into(), "run:cmake.configure".into())),
            ..SymbolsNote::default()
        },
    });
    let p = s.editor.window_view().palette.unwrap();
    let state = p.state.expect("the warning");
    assert_eq!(state.tone, StateTone::Warning);
    assert_eq!(state.link.as_deref(), Some("Configure with CMake"));
}
