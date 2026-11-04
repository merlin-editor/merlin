//! The files under the editor: saving without ever leaving one half
//! written, noticing when they change instead of asking every half
//! second, and keeping unsaved changes safe from a crash.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher as _};

use crate::jobs::Waker;

/// Writes `text` to `path` through a file beside it that then takes its
/// place, so a crash or a full disk leaves the old file whole. The new
/// file keeps the old one's permissions, and a symlink stays a symlink.
pub(crate) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return fs::write(&path, text);
    };
    let temp = dir.join(format!(".{}.merlin-save", name.to_string_lossy()));
    let written = (|| {
        let mut file = File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Ok(meta) = fs::metadata(&path) {
            fs::set_permissions(&temp, meta.permissions())?;
        }
        fs::rename(&temp, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
        // A folder that can't take a new file, but whose file can be
        // written: write it in place, as before.
        if path.exists() {
            return fs::write(&path, text);
        }
    }
    written
}

/// Folders watched for changes, each on its own so ignored ones such as
/// `target/` cost nothing.
pub(crate) struct Watch {
    watcher: Option<notify::RecommendedWatcher>,
    events: Receiver<notify::Result<notify::Event>>,
    dirs: HashSet<PathBuf>,
}

/// What changed on disk since it was last asked.
#[derive(Default)]
pub(crate) struct Changes {
    pub(crate) paths: HashSet<PathBuf>,
    /// Something was made, removed or renamed, not only written.
    pub(crate) structure: bool,
}

impl Watch {
    pub(crate) fn new(wake: Waker) -> Self {
        let (tx, events) = channel();
        let watcher = notify::recommended_watcher(move |event| {
            if tx.send(event).is_ok() {
                wake();
            }
        })
        .ok();
        Self {
            watcher,
            events,
            dirs: HashSet::new(),
        }
    }

    /// Watches nothing, for scripts.
    pub(crate) fn none() -> Self {
        Self {
            watcher: None,
            events: channel().1,
            dirs: HashSet::new(),
        }
    }

    /// Watches exactly `dirs`.
    pub(crate) fn set(&mut self, dirs: HashSet<PathBuf>) {
        let Some(watcher) = &mut self.watcher else {
            return;
        };
        if dirs == self.dirs {
            return;
        }
        for gone in self.dirs.difference(&dirs) {
            let _ = watcher.unwatch(gone);
        }
        let mut watched = HashSet::new();
        for dir in dirs {
            if self.dirs.contains(&dir) || watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok()
            {
                watched.insert(dir);
            }
        }
        self.dirs = watched;
    }

    /// What changed since last asked.
    pub(crate) fn take(&mut self, into: &mut Changes) -> bool {
        let mut any = false;
        while let Ok(event) = self.events.try_recv() {
            let Ok(event) = event else { continue };
            match event.kind {
                EventKind::Access(_) | EventKind::Other => continue,
                EventKind::Create(_) | EventKind::Remove(_) => into.structure = true,
                EventKind::Modify(notify::event::ModifyKind::Name(_)) => into.structure = true,
                _ => {}
            }
            any = true;
            into.paths.extend(event.paths);
        }
        any
    }
}

/// How long changes on disk settle before the editor looks, so a save
/// that writes a file in steps is seen once.
pub(crate) const SETTLE: Duration = Duration::from_millis(80);

/// How long after the last edit unsaved changes are kept.
const BACKUP_AFTER: Duration = Duration::from_millis(1000);

/// Unsaved changes, kept on disk while the editor runs, so a crash loses
/// none of them. A clean exit clears them.
pub(crate) struct Backups {
    dir: Option<PathBuf>,
    /// The buffer version kept of each file, by backup name.
    kept: HashMap<String, u64>,
    pub(crate) due: Option<Instant>,
}

/// What to do to keep the unsaved files as they are now.
pub(crate) struct Plan {
    pub(crate) dir: PathBuf,
    /// Each backup to write, by name, with its contents.
    pub(crate) write: Vec<(String, String)>,
    /// Backups of files that are saved now, or closed.
    pub(crate) remove: Vec<String>,
}

/// One file's kept changes: where it is, if it's not untitled, and its text.
pub(crate) struct Kept {
    pub(crate) path: Option<PathBuf>,
    pub(crate) text: String,
}

impl Backups {
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            kept: HashMap::new(),
            due: None,
        }
    }

    /// What a run that didn't exit cleanly left behind. Those backups stay
    /// until the files are kept again or the editor exits cleanly.
    pub(crate) fn left_over(&mut self) -> Vec<Kept> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let Ok(saved) = fs::read_to_string(entry.path()) else {
                continue;
            };
            let Some((path, text)) = saved.split_once('\n') else {
                continue;
            };
            self.kept
                .insert(entry.file_name().to_string_lossy().into_owned(), u64::MAX);
            found.push(Kept {
                path: (!path.is_empty()).then(|| PathBuf::from(path)),
                text: text.to_string(),
            });
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found
    }

    /// The editor's unsaved files changed: keep them a moment from now.
    pub(crate) fn touch(&mut self, unsaved: &[(u64, Option<PathBuf>, u64)]) {
        if self.dir.is_none() || self.due.is_some() {
            return;
        }
        let now: HashMap<String, u64> = unsaved
            .iter()
            .map(|(id, path, version)| (name(*id, path.as_deref()), *version))
            .collect();
        if now != self.kept {
            self.due = Some(Instant::now() + BACKUP_AFTER);
        }
    }

    /// What to write and remove to keep `unsaved` as it is now.
    pub(crate) fn plan(
        &mut self,
        unsaved: Vec<(u64, Option<PathBuf>, u64)>,
        text_of: impl Fn(u64) -> Option<String>,
    ) -> Option<Plan> {
        self.due = None;
        let dir = self.dir.clone()?;
        let mut write = Vec::new();
        let mut keep = HashMap::new();
        for (id, path, version) in unsaved {
            let name = name(id, path.as_deref());
            if self.kept.get(&name) != Some(&version)
                && let Some(text) = text_of(id)
            {
                let head = path.map(|p| p.display().to_string()).unwrap_or_default();
                write.push((name.clone(), format!("{head}\n{text}")));
            }
            keep.insert(name, version);
        }
        let remove = self
            .kept
            .keys()
            .filter(|n| !keep.contains_key(*n))
            .cloned()
            .collect();
        self.kept = keep;
        Some(Plan { dir, write, remove })
    }

    /// The editor exited cleanly: nothing to keep.
    pub(crate) fn clear(&self) {
        if let Some(dir) = &self.dir {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// A backup's file name: the file's path made safe, or the untitled
/// file's id.
fn name(id: u64, path: Option<&Path>) -> String {
    match path {
        Some(path) => crate::safe_name(&path.display().to_string()),
        None => format!("untitled-{id}"),
    }
}

/// Writes and removes backups, as [`Backups::plan`] said.
pub(crate) fn write_backups(plan: &Plan) {
    if !plan.write.is_empty() {
        let _ = fs::create_dir_all(&plan.dir);
    }
    for (name, text) in &plan.write {
        let _ = write_atomic(&plan.dir.join(name), text);
    }
    for name in &plan.remove {
        let _ = fs::remove_file(plan.dir.join(name));
    }
}
