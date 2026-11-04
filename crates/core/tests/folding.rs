//! Folding blocks by indentation.

use merlin_core::script::Session;

const CODE: &str =
    "fn a() {\n    one();\n    if x {\n        two();\n    }\n}\n\nfn b() {\n    three();\n}\n";

fn session() -> Session {
    Session::new(CODE, Some("a.rs"), 60, 14)
}

#[test]
fn folding_hides_the_block_and_shows_an_ellipsis() {
    let mut s = session();
    s.run("key down; key ctrl+shift+[");
    let screen = s.screen();
    assert!(screen.contains("fn a() { ⋯"), "{screen}");
    assert!(!screen.contains("one();"), "{screen}");
    assert!(screen.contains("  6 }"), "the closing line stays: {screen}");
    assert!(
        screen.contains("Ln 1,"),
        "the cursor moves to the first line: {screen}"
    );
    // Down skips the folded lines.
    s.run("key down");
    assert!(s.screen().contains("Ln 6,"), "{}", s.screen());
    s.run("key up; key ctrl+shift+]");
    assert!(s.screen().contains("one();"), "{}", s.screen());
}

#[test]
fn folding_again_folds_the_block_outside() {
    let mut s = session();
    s.run("key down; key down; key down; key ctrl+shift+[");
    let screen = s.screen();
    assert!(screen.contains("if x { ⋯"), "{screen}");
    assert!(screen.contains("one();"), "{screen}");
    s.run("key ctrl+shift+[");
    assert!(!s.screen().contains("one();"), "{}", s.screen());
}

#[test]
fn going_to_a_folded_line_unfolds_it() {
    let mut s = session();
    s.run("cmd edit.fold_all");
    let screen = s.screen();
    assert!(
        !screen.contains("three();") && !screen.contains("one();"),
        "{screen}"
    );
    s.run("key ctrl+g; type 9; key enter");
    assert!(s.screen().contains("three();"), "{}", s.screen());
    assert!(!s.screen().contains("one();"), "{}", s.screen());
}

#[test]
fn clicking_the_arrow_folds_and_unfolds() {
    let mut s = session();
    // The arrow is in the gutter's last column, just left of the text.
    let (x, y) = s.find("fn b()").unwrap();
    let x = x - 1;
    s.run(&format!("move {x} {y}"));
    let row = &s.editor.text_rows()[y as usize];
    assert!(row.contains("▾fn b()"), "{}", s.screen());
    s.run(&format!("click {x} {y}"));
    assert!(s.screen().contains("▸fn b() { ⋯"), "{}", s.screen());
    s.run(&format!("click {x} {y}"));
    assert!(s.screen().contains("three();"), "{}", s.screen());
}

#[test]
fn edits_above_a_fold_keep_it() {
    let mut s = session();
    s.run("key ctrl+end; key up; key up; key ctrl+shift+[; key ctrl+home; type // top\\n");
    let screen = s.screen();
    assert!(screen.contains("fn b() { ⋯"), "{screen}");
    assert!(!screen.contains("three();"), "{screen}");
}

#[test]
fn folds_work_with_word_wrap() {
    let mut s = session();
    s.run("cmd view.wrap; key down; key ctrl+shift+[; key down");
    assert!(s.screen().contains("Ln 6,"), "{}", s.screen());
}

#[test]
fn a_fold_in_one_group_leaves_the_others_alone() {
    let text: String = std::iter::once("fn a() {\n".to_string())
        .chain((1..=60).map(|i| format!("    line{i}();\n")))
        .chain(std::iter::once("}\n".to_string()))
        .collect();
    let mut s = Session::new(&text, Some("a.rs"), 100, 20);
    s.run("key ctrl+g; type 40; key enter; cmd view.split; key ctrl+home; cmd edit.fold_all");
    let screen = s.screen();
    // Each group has its own folds, as in VS Code: the right one's is
    // folded, and the left still shows the lines inside.
    assert_eq!(screen.matches("fn a() { ⋯").count(), 1, "{screen}");
    assert!(screen.contains("line40();"), "{screen}");
    // And keeps them when the other is focused.
    s.run("cmd view.group1");
    let screen = s.screen();
    assert_eq!(screen.matches("fn a() { ⋯").count(), 1, "{screen}");
    assert!(screen.contains("line40();"), "{screen}");
}
