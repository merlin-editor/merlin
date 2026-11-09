//! Each running plugin's instance lives on a thread of its own, so a slow
//! call can't stall the window. The host hands it jobs in order; a job's
//! events come back over a channel, or straight away when the caller waits.
//! A request that a newer one of the same kind replaces (a view drawn
//! again, a file changed again) is dropped if it hasn't started.

use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use merlin_core::Event;
use wasmtime::Store;

use crate::Bindings;
use crate::state::State;

pub(crate) struct Instance {
    pub store: Store<State>,
    pub bindings: Bindings,
    /// It trapped or ran out of fuel: it's stopped until it's started
    /// again, and later jobs do nothing.
    pub failed: bool,
}

/// What a plugin's page shows about its instance, kept by its thread.
#[derive(Clone, Debug, Default)]
pub(crate) struct Stats {
    pub memory: usize,
    pub processes: Vec<(String, u32)>,
    pub busy: Duration,
}

/// A call's events, on their way back to the window.
pub(crate) struct Finished {
    pub plugin: String,
    pub generation: u64,
    pub events: Vec<Event>,
    pub failed: bool,
    /// Its events only draw (a view, colours), so views needn't be asked
    /// for again because of them.
    pub quiet: bool,
}

/// A job, which may hand back a reply to send once its time is counted.
type Job = Box<dyn FnOnce(&mut Instance) -> Option<Reply> + Send>;
type Reply = Box<dyn FnOnce() + Send>;

/// A kind of request and its number: see [`Worker::ticket`].
pub(crate) type Ticket = (String, u64);

pub(crate) struct Worker {
    jobs: Sender<Job>,
    stats: Arc<Mutex<Stats>>,
    /// The newest ticket of each kind of request that a newer one replaces.
    latest: Arc<Mutex<HashMap<String, u64>>>,
}

impl Worker {
    /// Starts a thread for `instance`. Dropping the worker ends it once its
    /// job in hand is done, and dropping the instance stops its processes.
    pub fn start(name: &str, mut instance: Instance) -> std::io::Result<Self> {
        let (jobs, rx) = channel::<Job>();
        let stats = Arc::new(Mutex::new(Stats::default()));
        let kept = stats.clone();
        thread::Builder::new()
            .name(format!("plugin {name}"))
            .spawn(move || {
                for job in rx {
                    let started = Instant::now();
                    let reply = job(&mut instance);
                    {
                        let data = instance.store.data();
                        let mut stats = kept.lock().unwrap();
                        stats.busy += started.elapsed();
                        stats.memory = data.limits.memory;
                        stats.processes = data.processes.running();
                    }
                    // After the stats, so a caller that waited sees them
                    // with this job in.
                    if let Some(reply) = reply {
                        reply();
                    }
                }
            })?;
        Ok(Self {
            jobs,
            stats,
            latest: Arc::default(),
        })
    }

    /// Runs `f` on the plugin's thread and waits for it. None if the
    /// thread has gone.
    pub fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Instance) -> R + Send + 'static,
    ) -> Option<R> {
        let (tx, rx) = channel();
        self.jobs
            .send(Box::new(move |i| {
                let result = f(i);
                Some(Box::new(move || {
                    let _ = tx.send(result);
                }) as Reply)
            }))
            .ok()?;
        rx.recv().ok()
    }

    /// A new ticket for requests of `kind`: jobs posted with an older one
    /// do nothing once they come up. Requests posted together, as the
    /// tiles a view shows, share one.
    pub fn ticket(&self, kind: String) -> Ticket {
        let mut latest = self.latest.lock().unwrap();
        let n = latest.entry(kind.clone()).or_default();
        *n += 1;
        (kind, *n)
    }

    /// Queues `f` without waiting. Given a ticket, a newer one of the same
    /// kind makes this do nothing if it hasn't started.
    pub fn post(&self, ticket: Option<Ticket>, f: impl FnOnce(&mut Instance) + Send + 'static) {
        let latest = self.latest.clone();
        let _ = self.jobs.send(Box::new(move |i| {
            if let Some((kind, n)) = &ticket
                && latest.lock().unwrap().get(kind) != Some(n)
            {
                return None;
            }
            f(i);
            None
        }));
    }

    pub fn stats(&self) -> Stats {
        self.stats.lock().unwrap().clone()
    }
}
