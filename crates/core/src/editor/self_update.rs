//! Updating merlin itself: the title bar's pill, and the card it opens
//! (`docs/mockups/AppUpdate.dc.html`, and `AppUpdateStates.html` for the
//! card's other states). The app finds a release and does the work; this
//! says what there is to say, and asks the app for what's clicked.

use super::{Editor, Effect};

/// A newer release, as the app found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateOffer {
    /// Its tag, such as `v0.2.0`.
    pub version: String,
    /// This merlin's, such as `v0.1.0`.
    pub current: String,
    /// Its release notes, in Markdown; empty without any.
    pub notes: String,
    /// Its page, for Open on GitHub.
    pub page: String,
    /// Where merlin is, when it can't replace itself there, such as
    /// `in /usr/bin`: the card then gives [`UpdateOffer::command`].
    pub manual: Option<String>,
    /// The command that installs it.
    pub command: String,
}

/// A step of an update under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UpdateStep {
    Download,
    Check,
    Place,
    Restart,
}

/// What the app is asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateAction {
    /// Download, check and put in place this version.
    Install(String),
    /// Stop a download or check.
    Cancel,
    /// Start the new merlin once this one has quit.
    Restart,
}

/// What a click on the pill or in its card is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateHit {
    Pill,
    Notes,
    Back,
    /// Open on GitHub.
    Page,
    Install,
    Cancel,
    Later,
    Restart,
    Copy,
    /// The notes scrolled to this many pixels from their top.
    ScrollTo(i32),
}

/// How the update's going.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum UpdateState {
    #[default]
    Offered,
    Working {
        step: UpdateStep,
        /// Bytes downloaded, of how many; 0 when not known.
        done: u64,
        total: u64,
    },
    /// In place: the next start is the new version. The download's size.
    Ready(u64),
    Failed(String),
}

/// Which side of the card shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdatePage {
    Main,
    Notes,
}

/// The pill, and its card while it's open.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateView {
    pub pill: String,
    /// A turning ring before the label instead of the dot.
    pub busy: bool,
    /// The ↓ after the label.
    pub arrow: bool,
    pub card: Option<UpdateCardView>,
}

/// What the card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateCardView {
    pub title: String,
    /// The chip at the title's right: the version, or the percentage.
    pub badge: String,
    /// The release notes' width rather than the usual.
    pub wide: bool,
    /// The bar under the title, from 0 to 1.
    pub progress: Option<f32>,
    pub steps: Vec<UpdateStepView>,
    pub text: String,
    /// The text in the error colour.
    pub error: bool,
    /// A command line, with Copy.
    pub command: Option<String>,
    /// The release notes, and how far they're scrolled, in pixels.
    pub notes: Option<(String, i32)>,
    /// Open on GitHub ↗, under the notes.
    pub page_link: bool,
    pub buttons: Vec<UpdateButton>,
}

/// A line of the card's steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateStepView {
    pub label: String,
    /// At the line's right, such as `12 of 23 MB`.
    pub detail: String,
    pub mark: StepMark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepMark {
    Todo,
    Now,
    Done,
}

/// A button at the card's foot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateButton {
    pub label: String,
    pub hit: UpdateHit,
    pub style: ButtonStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Text only, in the muted colour.
    Ghost,
    Plain,
    Primary,
}

fn button(label: &str, hit: UpdateHit, style: ButtonStyle) -> UpdateButton {
    UpdateButton {
        label: label.into(),
        hit,
        style,
    }
}

/// Megabytes, whole, as the card says them.
fn mb(bytes: u64) -> String {
    format!("{}", (bytes + 512 * 1024) / (1024 * 1024))
}

impl Editor {
    /// A release the app found, or none: a check found nothing newer.
    pub(crate) fn update_offered(&mut self, offer: Option<UpdateOffer>) {
        match offer {
            Some(offer) => {
                // A check while an update is under way leaves it be.
                let same = self
                    .app_update
                    .as_ref()
                    .is_some_and(|(o, _)| o.version == offer.version);
                if same {
                    if let Some((o, _)) = &mut self.app_update {
                        *o = offer;
                    }
                } else {
                    self.app_update = Some((offer, UpdateState::Offered));
                }
            }
            None => {
                if matches!(
                    self.app_update,
                    Some((_, UpdateState::Offered | UpdateState::Failed(_)))
                ) {
                    self.app_update = None;
                    self.update_card = None;
                }
            }
        }
    }

    pub(crate) fn update_progress(&mut self, step: UpdateStep, done: u64, total: u64) {
        if let Some((_, state)) = &mut self.app_update {
            *state = UpdateState::Working { step, done, total };
        }
    }

    pub(crate) fn update_failed(&mut self, why: String) {
        if let Some((_, state)) = &mut self.app_update {
            *state = UpdateState::Failed(why);
        }
    }

    /// The new version is in place. With the card open, as it is after
    /// Restart & Update, merlin restarts; closed, it waits for a click.
    pub(crate) fn update_ready(&mut self, fx: &mut Vec<Effect>) {
        let Some((_, state)) = &mut self.app_update else {
            return;
        };
        let size = match state {
            UpdateState::Working { done, total, .. } => (*total).max(*done),
            _ => 0,
        };
        *state = UpdateState::Ready(size);
        if self.update_card.is_some() {
            self.restart_for_update(fx);
        }
    }

    /// Quits, keeping unsaved files for the next start whatever
    /// `files.hot_exit` says, and asks the app to start the new merlin.
    fn restart_for_update(&mut self, fx: &mut Vec<Effect>) {
        if let Some((_, state)) = &mut self.app_update {
            *state = UpdateState::Working {
                step: UpdateStep::Restart,
                done: 0,
                total: 0,
            };
        }
        self.keep_unsaved = self.docs.iter().any(|d| d.buffer.is_dirty());
        fx.push(Effect::AppUpdate(UpdateAction::Restart));
        self.quit(fx);
    }

    /// A click on the pill or in its card.
    pub(crate) fn update_click(&mut self, hit: UpdateHit, fx: &mut Vec<Effect>) {
        let Some((offer, state)) = &self.app_update else {
            return;
        };
        let (offer, state) = (offer.clone(), state.clone());
        match hit {
            UpdateHit::Pill => {
                self.update_card = match self.update_card {
                    Some(_) => None,
                    None => Some(UpdatePage::Main),
                };
            }
            UpdateHit::Notes if offer.notes.trim().is_empty() => {
                fx.push(Effect::OpenUrl(offer.page));
            }
            UpdateHit::Notes => {
                self.update_card = Some(UpdatePage::Notes);
                self.update_scroll = 0;
            }
            UpdateHit::Back => self.update_card = Some(UpdatePage::Main),
            UpdateHit::Page => fx.push(Effect::OpenUrl(offer.page)),
            UpdateHit::Install => {
                if offer.manual.is_some()
                    || matches!(state, UpdateState::Working { .. } | UpdateState::Ready(_))
                {
                    return;
                }
                self.update_card = Some(UpdatePage::Main);
                self.update_progress(UpdateStep::Download, 0, 0);
                fx.push(Effect::AppUpdate(UpdateAction::Install(offer.version)));
            }
            UpdateHit::Cancel => {
                if let UpdateState::Working { step, .. } = state
                    && step <= UpdateStep::Check
                {
                    if let Some((_, s)) = &mut self.app_update {
                        *s = UpdateState::Offered;
                    }
                    fx.push(Effect::AppUpdate(UpdateAction::Cancel));
                }
            }
            UpdateHit::Later => self.update_card = None,
            UpdateHit::Restart => {
                if matches!(state, UpdateState::Ready(_)) {
                    self.restart_for_update(fx);
                }
            }
            UpdateHit::Copy => {
                fx.push(Effect::Copy(offer.command));
                self.notify("Copied".into());
            }
            UpdateHit::ScrollTo(px) => self.update_scroll = px.max(0),
        }
    }

    /// The pill and card, while there's an update.
    pub(crate) fn update_view(&self) -> Option<UpdateView> {
        let (offer, state) = self.app_update.as_ref()?;
        let percent = |state: &UpdateState| -> u8 {
            match state {
                UpdateState::Working {
                    step: UpdateStep::Download,
                    done,
                    total,
                } if *total > 0 => ((*done).min(*total) * 90 / total) as u8,
                UpdateState::Working {
                    step: UpdateStep::Download,
                    ..
                } => 0,
                UpdateState::Working {
                    step: UpdateStep::Check,
                    ..
                } => 92,
                UpdateState::Working {
                    step: UpdateStep::Place,
                    ..
                } => 96,
                _ => 100,
            }
        };
        let (pill, busy, arrow) = match state {
            UpdateState::Working { .. } => (format!("Updating {}%", percent(state)), true, false),
            UpdateState::Ready(_) => ("Restart to update".to_string(), false, false),
            _ => ("Update available".to_string(), false, true),
        };
        let card = self.update_card.map(|page| {
            let v = &offer.version;
            let mut card = UpdateCardView {
                title: String::new(),
                badge: v.clone(),
                wide: false,
                progress: None,
                steps: Vec::new(),
                text: String::new(),
                error: false,
                command: None,
                notes: None,
                page_link: false,
                buttons: Vec::new(),
            };
            let notes = button("Release notes", UpdateHit::Notes, ButtonStyle::Ghost);
            let install = button("Restart & Update", UpdateHit::Install, ButtonStyle::Primary);
            match (page, state) {
                (UpdatePage::Notes, _) => {
                    card.title = format!("What's new in {v}");
                    card.wide = true;
                    card.notes = Some((offer.notes.clone(), self.update_scroll));
                    card.page_link = true;
                    card.buttons
                        .push(button("‹ Back", UpdateHit::Back, ButtonStyle::Ghost));
                    if offer.manual.is_none() && matches!(state, UpdateState::Offered) {
                        card.buttons.push(install);
                    }
                }
                (_, UpdateState::Offered) if offer.manual.is_some() => {
                    card.title = "Update Available".into();
                    card.text = format!(
                        "You have {}. merlin can't update itself {}. Update it the way you installed it, or run:",
                        offer.current,
                        offer.manual.as_deref().unwrap_or_default()
                    );
                    card.command = Some(offer.command.clone());
                    card.buttons.push(notes);
                }
                (_, UpdateState::Offered) => {
                    card.title = "Update Ready".into();
                    card.text = format!(
                        "You have {}. Restarting to update keeps your unsaved changes.",
                        offer.current
                    );
                    card.buttons.extend([notes, install]);
                }
                (_, UpdateState::Working { step, done, total }) => {
                    card.title = format!("Updating to {v}");
                    card.badge = format!("{}%", percent(state));
                    card.progress = Some(f32::from(percent(state)) / 100.0);
                    let size = match (*done, *total) {
                        (_, 0) if *done == 0 => String::new(),
                        (d, 0) => format!("{} MB", mb(d)),
                        (d, t) => format!("{} of {} MB", mb(d), mb(t)),
                    };
                    let mark = |s: UpdateStep| match s.cmp(step) {
                        std::cmp::Ordering::Less => StepMark::Done,
                        std::cmp::Ordering::Equal => StepMark::Now,
                        std::cmp::Ordering::Greater => StepMark::Todo,
                    };
                    for (s, label, detail) in [
                        (UpdateStep::Download, "Downloading", size),
                        (UpdateStep::Check, "Checking the signature", String::new()),
                        (UpdateStep::Place, "Putting it in place", String::new()),
                        (UpdateStep::Restart, "Restarting", String::new()),
                    ] {
                        card.steps.push(UpdateStepView {
                            label: label.into(),
                            detail,
                            mark: mark(s),
                        });
                    }
                    card.text = "Unsaved changes are kept.".into();
                    card.buttons.push(notes);
                    if *step <= UpdateStep::Check {
                        card.buttons
                            .push(button("Cancel", UpdateHit::Cancel, ButtonStyle::Plain));
                    }
                }
                (_, UpdateState::Ready(size)) => {
                    card.title = "Ready to Restart".into();
                    let size = if *size > 0 {
                        format!("{} MB", mb(*size))
                    } else {
                        String::new()
                    };
                    for (label, detail) in [
                        ("Downloaded", size),
                        ("Signature checked", String::new()),
                        ("In place", String::new()),
                    ] {
                        card.steps.push(UpdateStepView {
                            label: label.into(),
                            detail,
                            mark: StepMark::Done,
                        });
                    }
                    card.text = format!(
                        "{v} is installed and starts next time you open merlin."
                    );
                    card.buttons.extend([
                        button("Later", UpdateHit::Later, ButtonStyle::Ghost),
                        button("Restart Now", UpdateHit::Restart, ButtonStyle::Primary),
                    ]);
                }
                (_, UpdateState::Failed(why)) => {
                    card.title = "Update Failed".into();
                    card.text = why.clone();
                    card.error = true;
                    card.buttons.extend([
                        notes,
                        button("Try Again", UpdateHit::Install, ButtonStyle::Primary),
                    ]);
                }
            }
            card
        });
        Some(UpdateView {
            pill,
            busy,
            arrow,
            card,
        })
    }
}
