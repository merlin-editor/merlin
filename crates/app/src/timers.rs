//! The waits the editor asks for, as which-key's pause, timed here, and
//! what's due when the event loop wakes.

use super::*;

/// A wait the editor asks for by a tag and a length, as
/// [`Editor::which_key_wait`] does: timed from when it's first asked for,
/// and timed again when the tag changes.
#[derive(Default)]
pub(super) struct Wait(Option<(Instant, u64)>);

impl Wait {
    /// The editor asks for `ask`, a tag and how many milliseconds, now.
    pub(super) fn touch(&mut self, ask: Option<(u64, u64)>) {
        self.0 = match (ask, self.0) {
            (None, _) => None,
            (Some((tag, _)), Some((at, timed))) if timed == tag => Some((at, tag)),
            (Some((tag, ms)), _) => Some((Instant::now() + Duration::from_millis(ms), tag)),
        };
    }

    /// When it's due.
    pub(super) fn at(&self) -> Option<Instant> {
        self.0.map(|(at, _)| at)
    }

    /// Its tag, once it's due by `now`; then it's done.
    pub(super) fn take(&mut self, now: Instant) -> Option<u64> {
        let (at, tag) = self.0?;
        (at <= now).then(|| {
            self.0 = None;
            tag
        })
    }
}

impl App {
    /// Times the waits the editor asks for, as it asks for them now.
    pub(super) fn touch_waits(&mut self) {
        let format = self
            .editor
            .formatting()
            .map(|doc| (doc, merlin_core::FORMAT_WAIT_MS));
        self.format_wait.touch(format);
        self.which_key_wait.touch(self.editor.which_key_wait());
        self.palette_search_wait
            .touch(self.editor.palette_search_wait());
        self.link_wait.touch(self.editor.link_wait());
    }

    /// Sets when changed files save by themselves: the settings' delay
    /// after the last change to any of them.
    pub(super) fn touch_auto_save(&mut self) {
        let Some(delay) = self.editor.auto_save_delay() else {
            self.auto_save = None;
            return;
        };
        let unsaved: Vec<(u64, u64)> = self
            .editor
            .unsaved()
            .into_iter()
            .filter(|(_, path, _)| path.is_some())
            .map(|(id, _, version)| (id, version))
            .collect();
        if unsaved != self.auto_save_for {
            self.auto_save = (!unsaved.is_empty())
                .then(|| Instant::now() + std::time::Duration::from_millis(delay));
            self.auto_save_for = unsaved;
        }
    }

    /// When the app next has something to do, if ever.
    pub(crate) fn due(&self) -> Option<Instant> {
        [
            self.fetch_due.filter(|_| self.window_focused),
            self.blame_due.as_ref().map(|(at, _)| *at),
            self.settle,
            self.backups.due,
            self.auto_save,
            self.format_wait.at(),
            self.which_key_wait.at(),
            self.palette_search_wait.at(),
            self.link_wait.at(),
            self.editor.macro_playing().then(Instant::now),
            self.terminals.due(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Does what's due by now. Returns whether the editor heard anything.
    pub(crate) fn tick(&mut self) -> bool {
        let now = Instant::now();
        let mut changed = false;
        // Every few minutes while the window has focus, and as it comes
        // back after them.
        if self.blame_due.as_ref().is_some_and(|(at, _)| *at <= now)
            && let Some((_, ask)) = self.blame_due.take()
        {
            let git = self.git_info();
            self.jobs
                .run(Lane::Git, move || jobs::events(vec![git.blame(ask)]));
        }
        if self.window_focused && self.fetch_due.is_some_and(|at| at <= now) {
            self.fetch_due = Some(now + FETCH_EVERY);
            self.fetch_later();
        }
        if self.settle.is_some_and(|at| at <= now) {
            self.settle = None;
            let changes = std::mem::take(&mut self.changes);
            self.settled(changes);
            changed = true;
        }
        let fired = [
            self.which_key_wait.take(now).map(Event::WhichKey),
            self.palette_search_wait.take(now).map(Event::PaletteSearch),
            self.link_wait.take(now).map(Event::LinkDue),
        ];
        for event in fired.into_iter().flatten() {
            self.dispatch(event);
            changed = true;
        }
        if self.editor.macro_playing() {
            self.dispatch(Event::MacroStep);
            changed = true;
        }
        if self.format_wait.take(now).is_some() {
            self.dispatch(Event::FormatTimedOut);
            changed = true;
        }
        if self.auto_save.is_some_and(|at| at <= now) {
            self.auto_save = None;
            self.dispatch(Event::AutoSave(AutoSaveWhy::Delay));
            changed = true;
        }
        if self.backups.due.is_some_and(|at| at <= now) {
            let editor = &self.editor;
            let disk = &self.disk;
            let seen = |path: &Path| disk.get(path).copied().flatten();
            if let Some(plan) = self
                .backups
                .plan(editor.unsaved(), |id| editor.text_of(id), seen)
            {
                self.jobs.run(Lane::Disk, move || {
                    disk::write_backups(&plan);
                    jobs::events(Vec::new())
                });
            }
        }
        changed
    }
}
