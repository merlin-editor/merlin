//! A few low-priority threads for whole-file work on large files, as the
//! core asks with `Effect::Work`: a search's count, a wrap. Each job's
//! pieces are taken in order by whichever thread is free, so the work
//! fills in from the top of the file, and each piece's result is sent to
//! the editor as soon as it's done. A newer job of the same kind stops the
//! one before it.
//!
//! Four threads: on the study's measure (`docs/buffer-redesign.md`,
//! prototype 2), 70% less time than one for 10% more CPU in all.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use merlin_core::Event;
use merlin_core::work::{Job, Kind};

use crate::jobs::Waker;

const THREADS: usize = 4;

/// A job under way.
struct Task {
    job: Job,
    pieces: usize,
    /// The next piece to take.
    next: AtomicUsize,
    stop: AtomicBool,
}

impl Task {
    fn finished(&self) -> bool {
        self.stop.load(Ordering::Relaxed) || self.next.load(Ordering::Relaxed) >= self.pieces
    }
}

type Queue = Arc<(Mutex<VecDeque<Arc<Task>>>, Condvar)>;

pub(crate) struct Workers {
    /// None when work is done as it's asked for, for scripts.
    queue: Option<Queue>,
    done: Receiver<Event>,
    inline: VecDeque<Event>,
    /// The latest job of each kind, to stop when a newer one comes.
    latest: Vec<Arc<Task>>,
}

impl Workers {
    /// Threads of their own, at low priority, waking the window with `wake`.
    pub(crate) fn threaded(wake: Waker) -> Self {
        let queue: Queue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let (tx, done) = channel::<Event>();
        for n in 0..THREADS {
            let queue = queue.clone();
            let tx: Sender<Event> = tx.clone();
            let wake = wake.clone();
            thread::Builder::new()
                .name(format!("merlin-work-{n}"))
                .spawn(move || {
                    lower_priority();
                    work(&queue, &tx, &wake);
                })
                .expect("starting a work thread");
        }
        Self {
            queue: Some(queue),
            done,
            inline: VecDeque::new(),
            latest: Vec::new(),
        }
    }

    /// Work done at once, on the caller's thread.
    pub(crate) fn inline() -> Self {
        Self {
            queue: None,
            done: channel().1,
            inline: VecDeque::new(),
            latest: Vec::new(),
        }
    }

    pub(crate) fn run(&mut self, job: Job) {
        let Some(queue) = &self.queue else {
            for done in job.run_all() {
                self.inline.push_back(Event::Worked { id: job.id, done });
            }
            return;
        };
        let same = |a: &Kind, b: &Kind| std::mem::discriminant(a) == std::mem::discriminant(b);
        self.latest.retain(|t| {
            let older = same(&t.job.kind, &job.kind);
            if older {
                t.stop.store(true, Ordering::Relaxed);
            }
            !older && !t.finished()
        });
        let task = Arc::new(Task {
            pieces: job.pieces(),
            job,
            next: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
        });
        self.latest.push(task.clone());
        let (lock, ready) = &**queue;
        lock.lock().expect("work queue").push_back(task);
        ready.notify_all();
    }

    /// A finished piece, if one is waiting.
    pub(crate) fn take(&mut self) -> Option<Event> {
        self.inline
            .pop_front()
            .or_else(|| self.done.try_recv().ok())
    }
}

/// A work thread: takes the next piece of the first job not finished.
fn work(queue: &Queue, tx: &Sender<Event>, wake: &Waker) {
    let (lock, ready) = &**queue;
    loop {
        let task = {
            let mut q = lock.lock().expect("work queue");
            loop {
                while q.front().is_some_and(|t| t.finished()) {
                    q.pop_front();
                }
                if let Some(t) = q.front() {
                    break t.clone();
                }
                q = ready.wait(q).expect("work queue");
            }
        };
        let piece = task.next.fetch_add(1, Ordering::Relaxed);
        if piece >= task.pieces || task.stop.load(Ordering::Relaxed) {
            continue;
        }
        let done = task.job.run(piece);
        if task.stop.load(Ordering::Relaxed) {
            continue;
        }
        if tx
            .send(Event::Worked {
                id: task.job.id,
                done,
            })
            .is_err()
        {
            return;
        }
        wake();
    }
}

/// This thread's work gives way to the window's.
fn lower_priority() {
    #[cfg(unix)]
    // SAFETY: setpriority only changes this thread's scheduling: on Linux,
    // `who` 0 with PRIO_PROCESS is the calling thread.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
}
