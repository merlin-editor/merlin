//! Hot exit: quitting keeps unsaved files rather than asking, and they come
//! back at the next start, after a crash too.

use merlin_core::script::Session;
use merlin_core::{Event, UiEvent};

fn project() -> Session {
    let mut s = Session::new("fn main() {}\n", Some("src/main.rs"), 100, 24);
    s.files.insert("src/main.rs".into(), s.text());
    s
}

#[test]
fn quitting_with_unsaved_files_keeps_them_without_asking() {
    let mut s = project();
    s.run("type a; key ctrl+q");
    assert!(s.quit, "no question");
    assert!(s.editor.window_view().dialog.is_none());
    assert!(s.editor.keeps_unsaved());
    assert!(s.saved.is_empty(), "the file on disk isn't changed");
}

#[test]
fn quitting_with_nothing_unsaved_keeps_nothing() {
    let mut s = project();
    s.run("key ctrl+q");
    assert!(s.quit && !s.editor.keeps_unsaved());
}

#[test]
fn kept_files_come_back_unsaved_with_a_notice() {
    let mut s = project();
    s.send(Event::Recovered {
        path: Some("src/main.rs".into()),
        text: "fn main() { run(); }\n".into(),
        moved: false,
    });
    s.send(Event::Recovered {
        path: None,
        text: "scratch".into(),
        moved: false,
    });
    s.send(Event::RecoveredAll { crashed: false });
    assert_eq!(
        s.text(),
        "fn main() { run(); }\n",
        "the file in front stays"
    );
    let tabs = &s.editor.window_view().groups[0].tabs;
    assert_eq!(tabs.len(), 2);
    assert!(tabs.iter().all(|t| t.dirty));
    let toast = s.editor.window_view().toast.expect("a notice");
    assert_eq!(
        toast.text,
        "2 files with unsaved changes are back, as you left them."
    );
    assert_eq!(toast.buttons, ["Show"]);
    // Show lists them, with what changed.
    s.send(Event::Ui(UiEvent::ToastButton(0)));
    let palette = s.editor.window_view().palette.expect("the list");
    let names: Vec<&str> = palette.items.iter().map(|i| i.0.as_str()).collect();
    assert_eq!(names, ["main.rs", "[scratch]"]);
    assert_eq!(palette.extras[0].detail, "+1 −1");
    assert_eq!(palette.extras[1].detail, "untitled");
}

#[test]
fn after_a_crash_they_can_be_discarded() {
    let mut s = project();
    s.send(Event::Recovered {
        path: Some("src/main.rs".into()),
        text: "changed\n".into(),
        moved: false,
    });
    s.send(Event::Recovered {
        path: None,
        text: "scratch".into(),
        moved: false,
    });
    s.send(Event::RecoveredAll { crashed: true });
    let toast = s.editor.window_view().toast.expect("a notice");
    assert_eq!(
        toast.text,
        "merlin didn't close properly. 2 files with unsaved changes are back."
    );
    assert_eq!(toast.buttons, ["Discard Them", "Show"]);
    s.send(Event::Ui(UiEvent::ToastButton(0)));
    let dialog = s.editor.window_view().dialog.expect("asks first");
    assert_eq!(dialog.title, "Discard the unsaved changes to 2 files?");
    s.run("key enter");
    assert_eq!(s.text(), "fn main() {}\n", "back as it is on disk");
    let tabs = &s.editor.window_view().groups[0].tabs;
    assert_eq!(tabs.len(), 1, "the untitled file closed");
    assert!(!tabs[0].dirty);
}

#[test]
fn a_file_changed_on_disk_meanwhile_says_so() {
    let mut s = project();
    s.send(Event::Recovered {
        path: Some("src/main.rs".into()),
        text: "mine\n".into(),
        moved: true,
    });
    let away = s.editor.window_view().groups[0].away.clone();
    let (id, name) = away.expect("the bar");
    assert_eq!(name, "main.rs");
    // Keep Mine: the bar goes, the changes stay.
    s.send(Event::Ui(UiEvent::Press(format!("away:keep:{id}"))));
    assert!(s.editor.window_view().groups[0].away.is_none());
    assert_eq!(s.text(), "mine\n");

    let mut s = project();
    s.send(Event::Recovered {
        path: Some("src/main.rs".into()),
        text: "mine\n".into(),
        moved: true,
    });
    // Use the Disk's: the changes go.
    s.send(Event::Ui(UiEvent::Press(format!("away:disk:{id}"))));
    assert_eq!(s.text(), "fn main() {}\n");
    assert!(!s.editor.window_view().groups[0].tabs[0].dirty);
    assert!(s.editor.window_view().groups[0].away.is_none());
}
