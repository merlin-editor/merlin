//! Updating merlin itself: the title bar's pill and its card, through each
//! state the app reports.

use merlin_core::script::Session;
use merlin_core::{
    Effect, Event, Settings, UiEvent, UpdateAction, UpdateHit, UpdateOffer, UpdateStep,
};

fn offer() -> UpdateOffer {
    UpdateOffer {
        version: "v0.2.0".into(),
        current: "v0.1.0".into(),
        notes: "## What's in it\n\n- Updates from merlin itself.\n".into(),
        page: "https://github.com/merlin-editor/merlin/releases/tag/v0.2.0".into(),
        manual: None,
        command: "curl -fsSL https://example.org/install.sh | sh".into(),
    }
}

fn session() -> Session {
    let mut s = Session::new("fn main() {}\n", Some("src/main.rs"), 120, 30);
    s.files.insert("src/main.rs".into(), s.text());
    s
}

fn click(s: &mut Session, hit: UpdateHit) {
    s.send(Event::Ui(UiEvent::Update(hit)));
}

/// The dump's lines about the update, from the pill on.
fn update(s: &Session) -> String {
    let dump = s.editor.dump();
    let mut lines = dump
        .lines()
        .skip_while(|l| !l.starts_with("update:"))
        .take_while(|l| l.starts_with("update") || l.starts_with("  "));
    let mut out = String::new();
    for line in lines.by_ref() {
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn actions(s: &mut Session) -> Vec<UpdateAction> {
    s.effects
        .drain(..)
        .filter_map(|e| match e {
            Effect::AppUpdate(a) => Some(a),
            _ => None,
        })
        .collect()
}

#[test]
fn no_pill_until_theres_a_newer_release() {
    let mut s = session();
    assert_eq!(update(&s), "");
    assert!(s.editor.window_view().update.is_none());
    s.send(Event::UpdateOffer(Some(offer())));
    assert_eq!(update(&s), "update: • Update available ↓\n");
    s.send(Event::UpdateOffer(None));
    assert_eq!(update(&s), "", "a later check found nothing newer");
}

#[test]
fn the_pill_opens_the_card_and_a_click_elsewhere_closes_it() {
    let mut s = session();
    s.send(Event::UpdateOffer(Some(offer())));
    click(&mut s, UpdateHit::Pill);
    assert_eq!(
        update(&s),
        "update: • Update available ↓\n\
         update card: Update Ready [v0.2.0]\n  \
         You have v0.1.0. Restarting to update keeps your unsaved changes.\n  \
         [Release notes] [[Restart & Update]]\n"
    );
    s.run("key esc");
    assert!(s.editor.window_view().update.unwrap().card.is_none());
    click(&mut s, UpdateHit::Pill);
    s.send(Event::Ui(UiEvent::Crumb(0)));
    assert!(s.editor.window_view().update.unwrap().card.is_none());
}

#[test]
fn release_notes_show_in_the_card_and_back_returns() {
    let mut s = session();
    s.send(Event::UpdateOffer(Some(offer())));
    click(&mut s, UpdateHit::Pill);
    click(&mut s, UpdateHit::Notes);
    assert_eq!(
        update(&s),
        "update: • Update available ↓\n\
         update card: What's new in v0.2.0 [v0.2.0]\n  \
         | ## What's in it\n  \
         | - Updates from merlin itself.\n  \
         <Open on GitHub>\n  \
         [‹ Back] [[Restart & Update]]\n"
    );
    click(&mut s, UpdateHit::Page);
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::OpenUrl(u) if u.ends_with("/v0.2.0")))
    );
    click(&mut s, UpdateHit::Back);
    assert!(update(&s).contains("update card: Update Ready"));
}

#[test]
fn without_notes_release_notes_opens_the_page() {
    let mut s = session();
    s.send(Event::UpdateOffer(Some(UpdateOffer {
        notes: String::new(),
        ..offer()
    })));
    click(&mut s, UpdateHit::Pill);
    click(&mut s, UpdateHit::Notes);
    assert!(s.effects.iter().any(|e| matches!(e, Effect::OpenUrl(_))));
    assert!(update(&s).contains("update card: Update Ready"));
}

#[test]
fn updating_goes_step_by_step_then_restarts_keeping_unsaved_files() {
    let mut s = session();
    s.send(Event::Settings(Settings {
        hot_exit: false,
        ..Settings::default()
    }));
    s.run("type a");
    s.send(Event::UpdateOffer(Some(offer())));
    click(&mut s, UpdateHit::Pill);
    click(&mut s, UpdateHit::Install);
    assert_eq!(actions(&mut s), [UpdateAction::Install("v0.2.0".into())]);
    s.send(Event::UpdateProgress {
        step: UpdateStep::Download,
        done: 12 * 1024 * 1024,
        total: 23 * 1024 * 1024,
    });
    assert_eq!(
        update(&s),
        "update: ◌ Updating 46%\n\
         update card: Updating to v0.2.0 [46%]\n  \
         progress 46%\n  \
         ◌ Downloading · 12 of 23 MB\n  \
         ○ Checking the signature\n  \
         ○ Putting it in place\n  \
         ○ Restarting\n  \
         Unsaved changes are kept.\n  \
         [Release notes] [Cancel]\n"
    );
    s.send(Event::UpdateProgress {
        step: UpdateStep::Place,
        done: 23 * 1024 * 1024,
        total: 23 * 1024 * 1024,
    });
    let shown = update(&s);
    assert!(shown.contains("✓ Checking the signature"), "{shown}");
    assert!(shown.contains("◌ Putting it in place"), "{shown}");
    assert!(
        shown.contains("  [Release notes]\n"),
        "no Cancel once it's being put in place"
    );

    // The card is open, so it restarts by itself, keeping the unsaved file
    // although hot exit is off.
    s.send(Event::UpdateReady);
    assert_eq!(actions(&mut s), [UpdateAction::Restart]);
    assert!(s.quit && s.editor.keeps_unsaved());
    assert!(s.editor.window_view().dialog.is_none(), "no question");
}

#[test]
fn with_the_card_closed_it_waits_to_be_restarted() {
    let mut s = session();
    s.send(Event::UpdateOffer(Some(offer())));
    click(&mut s, UpdateHit::Pill);
    click(&mut s, UpdateHit::Install);
    s.send(Event::UpdateProgress {
        step: UpdateStep::Download,
        done: 23 * 1024 * 1024,
        total: 23 * 1024 * 1024,
    });
    s.run("key esc");
    s.send(Event::UpdateReady);
    assert!(!s.quit);
    assert_eq!(update(&s), "update: • Restart to update\n");
    click(&mut s, UpdateHit::Pill);
    assert_eq!(
        update(&s),
        "update: • Restart to update\n\
         update card: Ready to Restart [v0.2.0]\n  \
         ✓ Downloaded · 23 MB\n  \
         ✓ Signature checked\n  \
         ✓ In place\n  \
         v0.2.0 is installed and starts next time you open merlin.\n  \
         [Later] [[Restart Now]]\n"
    );
    click(&mut s, UpdateHit::Later);
    assert!(!s.quit && s.editor.window_view().update.unwrap().card.is_none());
    click(&mut s, UpdateHit::Pill);
    actions(&mut s);
    click(&mut s, UpdateHit::Restart);
    assert_eq!(actions(&mut s), [UpdateAction::Restart]);
    assert!(s.quit);
}

#[test]
fn cancel_and_a_failure_go_back_to_the_offer() {
    let mut s = session();
    s.send(Event::UpdateOffer(Some(offer())));
    click(&mut s, UpdateHit::Pill);
    click(&mut s, UpdateHit::Install);
    actions(&mut s);
    click(&mut s, UpdateHit::Cancel);
    assert_eq!(actions(&mut s), [UpdateAction::Cancel]);
    assert!(update(&s).contains("update card: Update Ready"));

    click(&mut s, UpdateHit::Install);
    actions(&mut s);
    s.send(Event::UpdateFailed(
        "The download failed its signature check. Nothing was changed.".into(),
    ));
    assert_eq!(
        update(&s),
        "update: • Update available ↓\n\
         update card: Update Failed [v0.2.0]\n  \
         error: The download failed its signature check. Nothing was changed.\n  \
         [Release notes] [[Try Again]]\n"
    );
    click(&mut s, UpdateHit::Install);
    assert_eq!(actions(&mut s), [UpdateAction::Install("v0.2.0".into())]);
}

#[test]
fn a_merlin_that_cant_replace_itself_gives_the_command() {
    let mut s = session();
    s.send(Event::UpdateOffer(Some(UpdateOffer {
        manual: Some("in /usr/bin".into()),
        ..offer()
    })));
    click(&mut s, UpdateHit::Pill);
    assert_eq!(
        update(&s),
        "update: • Update available ↓\n\
         update card: Update Available [v0.2.0]\n  \
         You have v0.1.0. merlin can't update itself in /usr/bin. Update it the way you installed it, or run:\n  \
         curl -fsSL https://example.org/install.sh | sh [Copy]\n  \
         [Release notes]\n"
    );
    click(&mut s, UpdateHit::Install);
    assert!(actions(&mut s).is_empty(), "nothing to install from here");
    click(&mut s, UpdateHit::Copy);
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::Copy(c) if c.starts_with("curl")))
    );
}

#[test]
fn a_check_during_an_update_leaves_it_going() {
    let mut s = session();
    s.send(Event::UpdateOffer(Some(offer())));
    click(&mut s, UpdateHit::Pill);
    click(&mut s, UpdateHit::Install);
    s.send(Event::UpdateOffer(Some(offer())));
    s.send(Event::UpdateOffer(None));
    assert!(update(&s).starts_with("update: ◌ Updating"));
}
