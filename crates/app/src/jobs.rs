//! Work that waits on the disk, git or a walk of the project, off the
//! window's thread, so a slow disk or a big repository never freezes it.
//!
//! Each lane is one thread doing its jobs in order, so a save never
//! overtakes the one before it, and a long search doesn't hold up a save.
//! A job does its IO and hands back a [`Finish`], which runs on the main
//! thread with the app to apply what it found.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use merlin_core::Event;

use crate::App;

/// What a job found, applied on the main thread.
pub(crate) type Finish = Box<dyn FnOnce(&mut App) -> Vec<Event> + Send>;

type Job = Box<dyn FnOnce() -> Finish + Send>;

/// Wakes the window when a job is done.
pub(crate) type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy)]
pub(crate) enum Lane {
    /// Saves, opens and Explorer file operations: short, and in order.
    Disk,
    /// Walks of the project: the file list, the Explorer tree, search.
    Scan,
    /// git, which can take a while in a big repository.
    Git,
    /// The system's file dialogs, which wait on the user.
    Ask,
}

pub(crate) struct Jobs {
    /// One sender per lane; none when jobs run as they're asked for.
    lanes: Option<[Sender<Job>; 4]>,
    done: Receiver<Finish>,
    /// Jobs done at once, for scripts, whose output must be the same
    /// every run.
    inline: VecDeque<Finish>,
    /// Jobs asked for and not yet finished.
    pending: usize,
}

impl Jobs {
    /// Jobs on threads of their own, waking the window with `wake`.
    pub(crate) fn threaded(wake: Waker) -> Self {
        let (done_tx, done) = channel();
        let lane = |name: &str| {
            let (tx, rx) = channel::<Job>();
            let done_tx: Sender<Finish> = done_tx.clone();
            let wake = wake.clone();
            thread::Builder::new()
                .name(format!("merlin-{name}"))
                .spawn(move || {
                    for job in rx {
                        if done_tx.send(job()).is_err() {
                            break;
                        }
                        wake();
                    }
                })
                .expect("starting a job thread");
            tx
        };
        Self {
            lanes: Some([lane("disk"), lane("scan"), lane("git"), lane("ask")]),
            done,
            inline: VecDeque::new(),
            pending: 0,
        }
    }

    /// Jobs done at once, on the caller's thread.
    pub(crate) fn inline() -> Self {
        Self {
            lanes: None,
            done: channel().1,
            inline: VecDeque::new(),
            pending: 0,
        }
    }

    pub(crate) fn run(&mut self, lane: Lane, work: impl FnOnce() -> Finish + Send + 'static) {
        self.pending += 1;
        match &self.lanes {
            Some(lanes) => {
                let _ = lanes[lane as usize].send(Box::new(work));
            }
            None => self.inline.push_back(work()),
        }
    }

    /// A finished job, if there's one waiting.
    pub(crate) fn take(&mut self) -> Option<Finish> {
        let done = self
            .inline
            .pop_front()
            .or_else(|| self.done.try_recv().ok());
        if done.is_some() {
            self.pending -= 1;
        }
        done
    }

    /// The next job to finish, waiting for it; `None` once there are none.
    pub(crate) fn wait(&mut self) -> Option<Finish> {
        if self.pending == 0 {
            return None;
        }
        let done = self.inline.pop_front().or_else(|| self.done.recv().ok());
        self.pending -= 1;
        done
    }
}

/// A [`Finish`] that only hands the editor events.
pub(crate) fn events(events: Vec<Event>) -> Finish {
    Box::new(move |_| events)
}
