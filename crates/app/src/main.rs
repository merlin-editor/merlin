mod clipboard;
mod disk;
mod editorconfig;
mod gui;
mod jobs;
mod looks;
mod packs;
mod settings;
mod template;
mod terminal;
mod theme;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};
use std::{env, fs, io};

use anyhow::{Context, Result, bail};
use disk::{Backups, Changes, Watch};
use jobs::{Jobs, Lane};
use merlin_core::script::{self, Step};
use merlin_core::{
    AutoSaveWhy, Editor, Effect, Event, FileOp, FilePick, GitAction, GitMove, MergeKind,
    MergeState, SearchHit, SyncKind, SyncOutcome, SyncState, Workspace,
};
use merlin_plugins::{BufferEvent, Config, PluginHost, Selection, Snapshot};

const USAGE: &str = "\
merlin - a code editor with sandboxed plugins

Usage: merlin [options] [file...]
       merlin plugin new <name>   Start a plugin in a new folder

Options:
  --plugin <dir>     Load the plugin in <dir> (repeatable). Plugins in
                     ~/.config/merlin/plugins load too. Plugins reload when
                     their component file changes.
  --exec <script>    Run a script headless and print what the window would
                     show, as text (see crates/core/src/script.rs for the
                     format). Scripts grant plugins their permissions
                     without asking, and remember nothing.
  --size <WxH>       The text's size for --exec, in columns and rows
                     (default 80x24)
  -h, --help         Show this help
";

struct Args {
    files: Vec<PathBuf>,
    plugins: Vec<PathBuf>,
    exec: Option<String>,
    size: (u16, u16),
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        files: Vec::new(),
        plugins: Vec::new(),
        exec: None,
        size: (80, 24),
    };
    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--plugin" => args
                .plugins
                .push(it.next().context("--plugin needs a folder")?.into()),
            "--exec" => args.exec = Some(it.next().context("--exec needs a script")?),
            "--size" => {
                let size = it.next().context("--size needs WxH")?;
                args.size = size
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .with_context(|| format!("bad size: {size}"))?;
            }
            flag if flag.starts_with('-') => bail!("unknown option: {flag}\n\n{USAGE}"),
            file => args.files.push(file.into()),
        }
    }
    Ok(args)
}

fn plugin_dirs(args: &Args) -> Vec<PathBuf> {
    let mut dirs = args.plugins.clone();
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    if let Some(config) = config
        && let Ok(entries) = fs::read_dir(config.join("merlin/plugins"))
    {
        let mut found: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        found.sort();
        // One given with --plugin as well loads once.
        let given: Vec<PathBuf> = dirs.iter().filter_map(|d| d.canonicalize().ok()).collect();
        dirs.extend(found.into_iter().filter(|d| {
            d.join("plugin.toml").exists() && !d.canonicalize().is_ok_and(|c| given.contains(&c))
        }));
    }
    dirs
}

/// `merlin plugin install <dir>...`: links each plugin into the user's
/// plugins folder, so it loads at every start, and allows what its manifest
/// asks for, since installing it is the user's say-so.
fn install_plugins(dirs: &[String]) -> Result<()> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .context("no home folder")?;
    let plugins = config.join("merlin/plugins");
    fs::create_dir_all(&plugins)?;
    for dir in dirs {
        let dir = Path::new(dir)
            .canonicalize()
            .with_context(|| format!("no plugin at {dir}"))?;
        let manifest = merlin_plugins::manifest(&dir)?;
        if let Some(component) = &manifest.component
            && !dir.join(component).exists()
        {
            bail!(
                "{} isn't built: run `cargo build --release` in {}",
                manifest.name,
                dir.display()
            );
        }
        let link = plugins.join(&manifest.name);
        if link.symlink_metadata().is_ok() {
            if link.canonicalize().ok().as_ref() != Some(&dir) {
                bail!("{} is already installed from elsewhere", link.display());
            }
        } else {
            std::os::unix::fs::symlink(&dir, &link)?;
        }
        let (_, allowed) = merlin_plugins::grant(&Config::standard(), &link)?;
        let may = if allowed.is_empty() {
            String::new()
        } else {
            format!(", allowed to {}", allowed.join(", "))
        };
        println!("Installed {}{may}", manifest.title);
    }
    Ok(())
}

/// Connects the editor to the outside world: it carries out the editor's
/// effects and feeds the results back in as events.
struct App {
    editor: Editor,
    /// Points the code's font is zoomed by, from Ctrl+= and Ctrl+-.
    zoom: f32,
    host: PluginHost,
    root: PathBuf,
    /// The file and buffer version the panels were last rendered for.
    panels_for: Option<(u64, u64)>,
    /// The file and buffer version plugins last heard about.
    buffer_seen: Option<(u64, u64)>,
    /// Paths saved since plugins last heard.
    saved: Vec<String>,
    /// The desktop's clipboards, once the window is open. Scripts have
    /// none, so they never touch the user's.
    pub(crate) clipboard: Option<clipboard::Clipboard>,
    /// The integrated terminal's shells.
    pub(crate) terminals: terminal::Terminals,
    /// When each open file last changed on disk, as far as the editor knows.
    disk: HashMap<PathBuf, Option<SystemTime>>,
    /// The repository's git folder, if the project is in one.
    git_dir: Option<PathBuf>,
    /// When git's index last changed, to notice commits.
    git_index: Option<SystemTime>,
    /// Where the project is in the repository, as git names paths: empty
    /// at its top, else ending in `/`.
    git_prefix: String,
    /// The push or pull running, by its process id, to cancel it.
    git_net: GitNet,
    /// When to ask the remote what's new next, while the window has focus.
    fetch_due: Option<Instant>,
    /// A blame asked for, run once the asking pauses.
    blame_due: Option<(Instant, Blame)>,
    pub(crate) window_focused: bool,
    /// The user's and the workspace's settings files.
    settings: settings::Files,
    /// Scripts run with default settings and don't change them.
    scripted: bool,
    /// How the window looks, from the packs stacked.
    look: packs::Look,
    /// What the settings say about the window: fonts, cursor, scrolling.
    window: settings::Window,
    /// Every pack there is, built in or from plugins.
    packs: Vec<packs::Pack>,
    /// The themes and packs chosen.
    choice: looks::Choice,
    /// The files of the packs stacked, and when each last changed, so a
    /// pack being written shows its changes on save.
    watched: Vec<(PathBuf, Option<SystemTime>)>,
    /// Disk, git and project walks, off the window's thread.
    jobs: Jobs,
    /// The folders whose files the editor cares about.
    watch: Watch,
    /// What changed on disk, waiting to settle until `settle`.
    changes: Changes,
    settle: Option<Instant>,
    /// The project's folders, from the last walk, to watch.
    tree_dirs: Vec<PathBuf>,
    /// A hash of the last walk, so one that found the same sends nothing.
    tree_hash: Option<u64>,
    /// What the watched folders were last worked out from.
    watch_for: Option<(Vec<Option<PathBuf>>, usize)>,
    backups: Backups,
    /// When changed files save by themselves next, and the changes that
    /// time is from.
    auto_save: Option<Instant>,
    auto_save_for: Vec<(u64, u64)>,
    /// When a save waiting for its formatting gives up and saves, and
    /// which document's it is.
    format_due: Option<(Instant, u64)>,
    quit: bool,
}

impl App {
    fn dispatch(&mut self, event: Event) {
        self.run_events(VecDeque::from([event]));
        // Once per dispatch, so a plugin that edits on every change can't
        // loop forever.
        let events = self.buffer_events();
        if !events.is_empty() {
            self.run_events(events.into());
        }
        self.refresh_panels();
        self.serve_colours();
        self.backups.touch(&self.editor.unsaved());
        self.touch_auto_save();
        self.touch_format();
        self.refresh_watch();
    }

    /// Times a save that waits for its formatting.
    fn touch_format(&mut self) {
        self.format_due = match (self.editor.formatting(), self.format_due) {
            (None, _) => None,
            (Some(doc), Some((at, timed))) if timed == doc => Some((at, doc)),
            (Some(doc), _) => Some((
                Instant::now() + std::time::Duration::from_millis(merlin_core::FORMAT_WAIT_MS),
                doc,
            )),
        };
    }

    /// Sets when changed files save by themselves: the settings' delay
    /// after the last change to any of them.
    fn touch_auto_save(&mut self) {
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

    /// Asks language plugins to colour what the editor wants coloured that
    /// isn't an open file: diffs, and code blocks in Markdown. Returns
    /// whether there was any.
    fn serve_colours(&mut self) -> bool {
        let wanted = self.editor.colour_requests();
        if wanted.is_empty() {
            return false;
        }
        let events = wanted
            .into_iter()
            .map(|(language, text)| {
                let tokens = self.host.colour_text(&language, &text).unwrap_or_default();
                Event::TextColours {
                    language,
                    text,
                    tokens,
                }
            })
            .collect();
        self.run_events(events);
        true
    }

    /// Tells plugins what happened to the open files since they last heard.
    fn buffer_events(&mut self) -> Vec<Event> {
        let now = (self.editor.doc_id(), self.editor.buffer().version());
        // Nothing seen yet, or a plugin just loaded: say which file is active.
        let mut changes = match self.buffer_seen {
            None => vec![BufferEvent::Switched],
            Some((doc, _)) if doc != now.0 => vec![BufferEvent::Switched],
            Some((_, version)) if version != now.1 => vec![BufferEvent::Changed],
            _ => Vec::new(),
        };
        self.buffer_seen = Some(now);
        changes.extend(self.saved.drain(..).map(BufferEvent::Saved));
        let snapshot = self.snapshot();
        let mut events = Vec::new();
        for change in changes {
            events.extend(self.host.buffer_event(&change, &snapshot));
        }
        events
    }

    fn run_events(&mut self, mut queue: VecDeque<Event>) {
        loop {
            let Some(event) = queue.pop_front() else {
                // Jobs done at once, as scripts have them, finish here.
                match self.jobs.take() {
                    Some(finish) => {
                        queue.extend(finish(self));
                        continue;
                    }
                    None => break,
                }
            };
            if matches!(event, Event::RegisterPlugin { .. }) {
                self.buffer_seen = None;
            }
            for effect in self.editor.update(event) {
                match effect {
                    Effect::Save {
                        doc,
                        path,
                        text,
                        revision,
                    } => {
                        let full = self.root.join(&path);
                        self.jobs.run(Lane::Disk, move || {
                            let written =
                                disk::write_atomic(&full, &text).map(|()| modified(&full));
                            Box::new(move |app: &mut App| match written {
                                Ok(when) => app.saved_file(doc, revision, path, when),
                                Err(err) => vec![Event::SaveFailed(err.to_string())],
                            })
                        });
                    }
                    Effect::Open(path) => self.open_later(&path),
                    Effect::ListFiles => {
                        let root = self.root.clone();
                        self.jobs.run(Lane::Scan, move || {
                            jobs::events(vec![Event::Files(list_files(&root))])
                        });
                    }
                    Effect::ListTree => self.list_tree_later(),
                    Effect::File(op) => {
                        let root = self.root.clone();
                        self.jobs.run(Lane::Disk, move || {
                            let done = file_op(&root, op);
                            Box::new(move |app: &mut App| {
                                for event in &done {
                                    match event {
                                        Event::Renamed { from, .. } => {
                                            app.disk.retain(|p, _| !p.starts_with(from))
                                        }
                                        Event::Trashed(path) => {
                                            app.disk.retain(|p, _| !p.starts_with(path))
                                        }
                                        Event::Opened { path, .. } => {
                                            let when = modified(&app.root.join(path));
                                            app.disk.insert(path.clone(), when);
                                        }
                                        _ => {}
                                    }
                                }
                                app.list_tree_later();
                                app.git_status_later();
                                done
                            })
                        });
                    }
                    Effect::Search(query) => {
                        let root = self.root.clone();
                        self.jobs.run(Lane::Scan, move || {
                            let hits = search(&root, &query);
                            jobs::events(vec![Event::SearchResults { query, hits }])
                        });
                    }
                    Effect::GitStatus => self.git_status_later(),
                    // Asked for as the text changes, it runs after a pause.
                    Effect::Blame {
                        path,
                        version,
                        text,
                        ignore,
                    } => {
                        let due = Instant::now() + BLAME_PAUSE;
                        self.blame_due = Some((
                            due,
                            Blame {
                                path,
                                version,
                                text,
                                ignore,
                            },
                        ));
                    }
                    Effect::MergeSides(path) => {
                        let git = self.git_info();
                        self.jobs
                            .run(Lane::Git, move || jobs::events(vec![git.merge_sides(path)]));
                    }
                    Effect::CommitInfo(hash) => {
                        let git = self.git_info();
                        self.jobs
                            .run(Lane::Git, move || jobs::events(vec![git.commit_info(hash)]));
                    }
                    Effect::OpenCommit { hash, path } => {
                        let git = self.git_info();
                        self.jobs.run(Lane::Git, move || {
                            jobs::events(vec![git.commit_diff(&hash, path)])
                        });
                    }
                    // Stopping a push or pull can't wait behind it.
                    Effect::Git(GitAction::CancelSync) => self.git_net.cancel(),
                    Effect::Git(action) => {
                        let commit = matches!(action, GitAction::Commit(_));
                        let git = self.git_info();
                        self.jobs.run(Lane::Git, move || {
                            let mut done = git.run(action);
                            if !commit && done.iter().any(|e| matches!(e, Event::GitStatus { .. }))
                            {
                                done.push(git.diffs());
                            }
                            Box::new(move |app: &mut App| {
                                if commit {
                                    // The new commit is the base for gutter
                                    // signs.
                                    app.git_index = None;
                                    app.check_disk();
                                }
                                done
                            })
                        });
                    }
                    Effect::OpenDiff(path) => {
                        let root = self.root.clone();
                        self.jobs.run(Lane::Git, move || {
                            jobs::events(vec![changes_of(&root, &path, path.clone())])
                        });
                    }
                    Effect::OpenMove { from, to } => {
                        let root = self.root.clone();
                        self.jobs.run(Lane::Git, move || {
                            jobs::events(vec![changes_of(&root, &from, to)])
                        });
                    }
                    Effect::ReadFiles(paths) => {
                        let root = self.root.clone();
                        self.jobs.run(Lane::Disk, move || {
                            let files = paths
                                .into_iter()
                                .map(|p| {
                                    let text = fs::read_to_string(root.join(&p)).ok();
                                    (p, text)
                                })
                                .collect();
                            jobs::events(vec![Event::FilesRead(files)])
                        });
                    }
                    Effect::OpenUrl(url) => open_url(&url),
                    Effect::Copy(text) => {
                        if let Some(clipboard) = &mut self.clipboard {
                            clipboard.copy(&text);
                        }
                    }
                    Effect::Terminal(op) => self.terminal(op),
                    Effect::ReadClipboard => {
                        let text = self.clipboard.as_mut().and_then(|c| c.paste());
                        queue.push_back(Event::Clipboard(text));
                    }
                    Effect::SetSelection(text) => {
                        if let Some(clipboard) = &mut self.clipboard {
                            clipboard.select(&text);
                        }
                    }
                    Effect::ReadSelection => {
                        if let Some(text) = self.clipboard.as_mut().and_then(|c| c.selection()) {
                            queue.push_back(Event::Paste(text));
                        }
                    }
                    Effect::Quit => self.quit = true,
                    // Scripts can't answer a dialog: it's as if cancelled.
                    Effect::PickFile(pick) if self.scripted => {
                        queue.push_back(Event::Picked { pick, path: None });
                    }
                    Effect::PickFile(pick) => {
                        // In the active file's folder, as VS Code starts.
                        let path = self.editor.buffer().path.clone();
                        let folder = path
                            .as_ref()
                            .and_then(|p| p.parent())
                            .map_or(self.root.clone(), |p| self.root.join(p));
                        let name = path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned());
                        self.jobs.run(Lane::Ask, move || {
                            let dialog = rfd::FileDialog::new().set_directory(&folder);
                            let path = match &pick {
                                FilePick::Open => dialog.set_title("Open File").pick_file(),
                                FilePick::OpenFolder => {
                                    dialog.set_title("Open Folder").pick_folder()
                                }
                                FilePick::SaveAs { .. } => {
                                    let dialog = dialog.set_title("Save As");
                                    match name {
                                        Some(name) => dialog.set_file_name(name),
                                        None => dialog,
                                    }
                                    .save_file()
                                }
                            };
                            jobs::events(vec![Event::Picked { pick, path }])
                        });
                    }
                    Effect::OpenFolder(folder) => {
                        if let Err(err) = open_window(&folder) {
                            queue.push_back(Event::Error(format!(
                                "Can't open {}: {err}",
                                folder.display()
                            )));
                        }
                    }
                    Effect::PluginCommand { plugin, command } if plugin == looks::PANEL => {
                        queue.extend(self.packs_command(&command));
                    }
                    Effect::PluginClick { plugin, button } if plugin == looks::PANEL => {
                        queue.extend(self.packs_click(&button));
                    }
                    Effect::PluginCommand { plugin, command } => {
                        let snapshot = self.snapshot();
                        queue.extend(self.host.run_command(&plugin, &command, snapshot));
                        self.panels_for = None;
                    }
                    Effect::PluginClick { plugin, button } => {
                        let snapshot = self.snapshot();
                        queue.extend(self.host.click(&plugin, &button, snapshot));
                        self.panels_for = None;
                    }
                    Effect::PluginSubmit {
                        plugin,
                        input,
                        text,
                    } => {
                        let snapshot = self.snapshot();
                        queue.extend(self.host.submit(&plugin, &input, &text, snapshot));
                        self.panels_for = None;
                    }
                    Effect::PluginPick {
                        plugin,
                        picker,
                        item,
                    } => {
                        let snapshot = self.snapshot();
                        queue.extend(self.host.pick(&plugin, &picker, &item, snapshot));
                        self.panels_for = None;
                    }
                    Effect::PluginReview {
                        plugin,
                        id,
                        accepted,
                    } => {
                        let snapshot = self.snapshot();
                        queue.extend(self.host.review(&plugin, &id, accepted, snapshot));
                        self.panels_for = None;
                    }
                    Effect::SetSetting { key, value } => {
                        if !self.scripted
                            && let Err(err) = self.settings.set(&key, &value)
                        {
                            queue.push_back(Event::Error(format!("Can't save {key}: {err}")));
                        }
                    }
                    Effect::PreviewTheme(kind, name) => {
                        queue.extend(self.preview(kind, name));
                    }
                    Effect::SetTheme(kind, name) => queue.extend(self.choose(kind, name)),
                    Effect::OpenSettings { workspace } => match self.settings.ensure(workspace) {
                        Ok(path) => self.open_later(&path),
                        Err(err) => {
                            queue.push_back(Event::Error(format!("Can't open settings: {err}")))
                        }
                    },
                    // Until the window draws the Settings window, the
                    // user's settings file stands in for it.
                    Effect::Zoom(step) => {
                        self.zoom = match step {
                            0 => 0.0,
                            n => (self.zoom + n as f32).clamp(-6.0, 18.0),
                        };
                    }
                    Effect::OpenSettingsWindow => match self.settings.ensure(false) {
                        Ok(path) => self.open_later(&path),
                        Err(err) => {
                            queue.push_back(Event::Error(format!("Can't open settings: {err}")))
                        }
                    },
                }
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        snapshot(&self.editor, &self.root)
    }

    fn is_settings(&self, path: &Path) -> bool {
        let full = self.root.join(path);
        let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        self.settings.is_settings(&full)
            || [&self.settings.user, &self.settings.workspace]
                .iter()
                .any(|s| canonical(s) == canonical(&full))
    }

    /// Reads the settings and hands them to the editor and the plugins. A
    /// script gets the defaults, so its output is the same everywhere.
    fn load_settings(&mut self) -> Vec<Event> {
        let (table, errors) = if self.scripted {
            (toml::Table::new(), Vec::new())
        } else {
            self.settings.load()
        };
        let mut editor = settings::editor(&table);
        self.window = settings::window(&table);
        // MERLIN_MINIMAP=on or off still decides, for trying it out.
        match env::var("MERLIN_MINIMAP").as_deref() {
            Ok("on" | "1") => editor.minimap = true,
            Ok("off" | "0") => editor.minimap = false,
            _ => {}
        }
        if let Some(icons) = settings::editor_text(&table, "icons")
            && env::var("MERLIN_ICONS").is_err()
        {
            self.editor.set_nerd_icons(icons != "plain");
        }
        self.host.set_settings(settings::plugins(&table));
        self.panels_for = None;
        self.choice.colours = settings::editor_text(&table, "theme").map(str::to_string);
        self.choice.icons = settings::editor_text(&table, "icon_theme").map(str::to_string);
        self.choice.packs = settings::editor_list(&table, "packs");
        let mut events = vec![Event::Settings(editor)];
        events.extend(self.refresh_looks());
        events.extend(
            errors
                .into_iter()
                .map(|e| Event::Error(format!("Settings: {e}"))),
        );
        events
    }

    /// A file saved: the editor hears, and saving a settings file applies
    /// it.
    fn saved_file(
        &mut self,
        doc: u64,
        revision: merlin_core::history::Revision,
        path: PathBuf,
        when: Option<SystemTime>,
    ) -> Vec<Event> {
        self.disk.insert(path.clone(), when);
        self.saved.push(path.display().to_string());
        let settings = self.is_settings(&path);
        let mut events = vec![Event::Saved {
            doc,
            revision,
            path,
        }];
        if settings {
            events.extend(self.load_settings());
        }
        events
    }

    /// A file dropped on the window, from anywhere.
    pub(crate) fn open_dropped(&mut self, path: &Path) {
        if path.is_dir() {
            let event = Event::Error(format!("{} is a folder", path.display()));
            self.dispatch(event);
        } else {
            self.open_later(path);
        }
    }

    /// Reads a file to open, at once, for files named at the start. A path
    /// that doesn't exist yet opens empty, and is created on save.
    fn open(&mut self, path: &Path) -> Vec<Event> {
        let path = relative_to(&self.root, path);
        let full = self.root.join(&path);
        self.disk.insert(path.clone(), modified(&full));
        match read_file(&full) {
            Ok(text) => vec![
                Event::Opened {
                    path: path.clone(),
                    text,
                },
                Event::FileConfig {
                    path,
                    config: editorconfig::for_file(&full),
                },
            ],
            Err(err) => vec![Event::Error(format!(
                "Can't open {}: {err}",
                path.display()
            ))],
        }
    }

    /// Reads a file to open, then its text as of the last commit.
    fn open_later(&mut self, path: &Path) {
        let path = relative_to(&self.root, path);
        let full = self.root.join(&path);
        self.jobs.run(Lane::Disk, move || {
            let when = modified(&full);
            let read = read_file(&full);
            let config = editorconfig::for_file(&full);
            Box::new(move |app: &mut App| match read {
                Ok(text) => {
                    app.disk.insert(path.clone(), when);
                    app.git_base_later(path.clone());
                    vec![
                        Event::Opened {
                            path: path.clone(),
                            text,
                        },
                        Event::FileConfig { path, config },
                    ]
                }
                Err(err) => vec![Event::Error(format!(
                    "Can't open {}: {err}",
                    path.display()
                ))],
            })
        });
    }

    /// Reads an open file again that changed on disk.
    fn reload_later(&mut self, path: PathBuf) {
        let full = self.root.join(&path);
        self.jobs.run(Lane::Disk, move || {
            let read = fs::read_to_string(&full);
            jobs::events(match read {
                Ok(text) => vec![Event::FileChanged { path, text }],
                Err(_) => Vec::new(),
            })
        });
    }

    fn git_base_later(&mut self, path: PathBuf) {
        let root = self.root.clone();
        self.jobs.run(Lane::Git, move || {
            let text = git_base(&root, &path);
            jobs::events(vec![Event::GitBase { path, text }])
        });
    }

    fn git_info(&self) -> Git {
        Git {
            root: self.root.clone(),
            repo: self.git_dir.is_some(),
            prefix: self.git_prefix.clone(),
            net: self.git_net.clone(),
            dir: self.git_dir.clone(),
        }
    }

    /// Asks the remote what's new, quietly, so the sync counts are right.
    fn fetch_later(&mut self) {
        let git = self.git_info();
        self.jobs.run(Lane::Git, move || {
            let mut done = git.run(GitAction::Sync(SyncKind::Fetch));
            done.push(git.diffs());
            jobs::events(done)
        });
    }

    fn git_status_later(&mut self) {
        let git = self.git_info();
        self.jobs.run(Lane::Git, move || {
            let mut status = git.status();
            status.push(git.diffs());
            Box::new(move |app: &mut App| {
                // git status can touch the index; that's not a commit.
                app.git_index = app
                    .git_dir
                    .as_ref()
                    .and_then(|d| modified(&d.join("index")));
                status
            })
        });
    }

    /// Walks the project for the Explorer, and to know which folders to
    /// watch.
    fn list_tree_later(&mut self) {
        let root = self.root.clone();
        self.jobs.run(Lane::Scan, move || {
            let tree = list_tree(&root);
            let hash = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                tree.hash(&mut h);
                h.finish()
            };
            Box::new(move |app: &mut App| {
                // The same as last time: nothing to tell the editor.
                if app.tree_hash == Some(hash) {
                    return Vec::new();
                }
                app.tree_hash = Some(hash);
                app.tree_dirs = tree
                    .iter()
                    .filter_map(|p| p.strip_suffix('/'))
                    .map(|d| app.root.join(d))
                    .collect();
                app.watch_for = None;
                vec![Event::Tree(tree)]
            })
        });
    }

    /// Reloads open files that changed on disk, such as ones another program
    /// edited, and refreshes git signs after a commit.
    fn check_disk(&mut self) {
        // A pack being written: show it as it's saved.
        for event in self.check_packs() {
            self.dispatch(event);
        }
        let index = self
            .git_dir
            .as_ref()
            .and_then(|dir| modified(&dir.join("index")));
        let committed = index != self.git_index;
        self.git_index = index;
        if committed {
            self.git_status_later();
        }
        let open: Vec<PathBuf> = self.editor.open_files().into_iter().flatten().collect();
        self.disk.retain(|path, _| open.contains(path));
        for path in open {
            let now = modified(&self.root.join(&path));
            let changed = self.disk.get(&path).is_some_and(|seen| *seen != now);
            if changed {
                self.disk.insert(path.clone(), now);
                self.reload_later(path.clone());
            }
            if committed {
                self.git_base_later(path);
            }
        }
        self.finish_jobs();
    }

    /// Applies the jobs that are done. Returns whether there were any.
    fn finish_jobs(&mut self) -> bool {
        let mut any = false;
        while let Some(finish) = self.jobs.take() {
            any = true;
            for event in finish(self) {
                self.dispatch(event);
            }
        }
        any
    }

    /// Waits for every job, as when quitting, so no save is lost.
    pub(crate) fn finish_all(&mut self) {
        while let Some(finish) = self.jobs.wait() {
            for event in finish(self) {
                self.dispatch(event);
            }
        }
    }

    /// Something woke the window: plugin output, a job done, or a change on
    /// disk. Returns whether the editor heard anything.
    pub(crate) fn wake(&mut self) -> bool {
        let mut changed = self.poll_terminals();
        changed |= self.poll_processes();
        changed |= self.finish_jobs();
        if self.watch.take(&mut self.changes) && self.settle.is_none() {
            self.settle = Some(Instant::now() + disk::SETTLE);
        }
        changed
    }

    /// The colours terminals are drawn in, from the look.
    pub(crate) fn terminal_colours(&self) -> terminal::Colours {
        let p = &self.look.palette;
        terminal::Colours::for_palette(p.text, p.island, theme::is_light(p))
    }

    /// Does what the editor asks of a terminal.
    fn terminal(&mut self, op: merlin_core::TermOp) {
        use merlin_core::TermOp;
        let t = &mut self.terminals;
        match op {
            TermOp::Spawn { id, command } => t.spawn(id, &self.root, command),
            TermOp::Key { id, key } => t.key(id, key),
            TermOp::Paste { id, text } => t.paste(id, &text),
            TermOp::Scroll { id, lines } => t.scroll(id, lines),
            TermOp::Page { id, up } => t.page(id, up),
            TermOp::Copy { id } => {
                if let (Some(text), Some(clipboard)) = (t.selected(id), &mut self.clipboard) {
                    clipboard.copy(&text);
                }
            }
            TermOp::Kill { id } => t.kill(id),
        }
    }

    /// Hands what the shells said to the editor. Returns whether a screen
    /// or the editor changed.
    fn poll_terminals(&mut self) -> bool {
        let colours = self.terminal_colours();
        let clipboard = &mut self.clipboard;
        let (events, changed) = self
            .terminals
            .take_events(&colours, &mut |text| match text {
                Some(text) => {
                    if let Some(c) = clipboard.as_mut() {
                        c.copy(&text);
                    }
                    String::new()
                }
                None => clipboard
                    .as_mut()
                    .and_then(|c| c.paste())
                    .unwrap_or_default(),
            });
        let any = !events.is_empty();
        for event in events {
            self.dispatch(event);
        }
        changed || any
    }

    /// When the app next has something to do, if ever.
    pub(crate) fn due(&self) -> Option<Instant> {
        [
            self.fetch_due.filter(|_| self.window_focused),
            self.blame_due.as_ref().map(|(at, _)| *at),
            self.settle,
            self.backups.due,
            self.auto_save,
            self.format_due.map(|(at, _)| at),
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
        if self.format_due.is_some_and(|(at, _)| at <= now) {
            self.format_due = None;
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
            if let Some(plan) = self.backups.plan(editor.unsaved(), |id| editor.text_of(id)) {
                self.jobs.run(Lane::Disk, move || {
                    disk::write_backups(&plan);
                    jobs::events(Vec::new())
                });
            }
        }
        changed
    }

    /// Files changed on disk, and have settled.
    fn settled(&mut self, changes: Changes) {
        self.reload_plugins();
        self.check_disk();
        let in_git = |p: &Path| self.git_dir.as_ref().is_some_and(|g| p.starts_with(g));
        let worktree: Vec<&PathBuf> = changes
            .paths
            .iter()
            .filter(|p| p.starts_with(&self.root) && !in_git(p))
            // Saving writes a file beside the one saved first.
            .filter(|p| !p.to_string_lossy().ends_with(".merlin-save"))
            .collect();
        if worktree.is_empty() {
            return;
        }
        if changes.structure {
            self.list_tree_later();
        }
        self.git_status_later();
    }

    /// Watches the folders of the project, the open files, git, the packs
    /// and the plugins, when any of those changed.
    fn refresh_watch(&mut self) {
        let open = self.editor.open_files();
        let key = (open, self.tree_dirs.len() + self.watched.len());
        if self.watch_for.as_ref() == Some(&key) {
            return;
        }
        let parent = |p: &Path| p.parent().map(Path::to_path_buf);
        let mut dirs: std::collections::HashSet<PathBuf> = self.tree_dirs.iter().cloned().collect();
        dirs.insert(self.root.clone());
        dirs.extend(
            key.0
                .iter()
                .flatten()
                .filter_map(|p| parent(&self.root.join(p))),
        );
        dirs.extend(self.git_dir.clone());
        dirs.extend(self.watched.iter().filter_map(|(p, _)| parent(p)));
        dirs.extend(self.host.components().iter().filter_map(|p| parent(p)));
        dirs.extend(
            [&self.settings.user, &self.settings.workspace]
                .into_iter()
                .filter_map(|p| parent(p)),
        );
        self.watch.set(dirs);
        self.watch_for = Some(key);
    }

    fn refresh_panels(&mut self) {
        if !self.editor.panel_visible() {
            self.panels_for = None;
            return;
        }
        let version = (self.editor.doc_id(), self.editor.buffer().version());
        if self.panels_for == Some(version) {
            return;
        }
        self.panels_for = Some(version);
        // Panel events only change what's drawn; they produce no effects.
        for event in self.host.render_panels(&self.snapshot()) {
            self.editor.update(event);
        }
    }

    /// Hands queued process output to plugins. Returns whether there was any.
    fn poll_processes(&mut self) -> bool {
        let (editor, root) = (&self.editor, &self.root);
        let (events, delivered) = self.host.poll_processes(|| snapshot(editor, root));
        if delivered {
            self.panels_for = None;
            for event in events {
                self.dispatch(event);
            }
            self.refresh_panels();
        }
        delivered
    }

    fn reload_plugins(&mut self) {
        let events = self.host.reload_changed();
        if !events.is_empty() {
            self.panels_for = None;
            for event in events {
                self.dispatch(event);
            }
        }
    }
}

/// A file to edit, read whole. One that isn't there yet opens empty.
fn read_file(path: &Path) -> io::Result<String> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "it isn't UTF-8 text, so editing it could change it",
            )
        }),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

/// Does a file operation from the Explorer, and says what changed.
fn file_op(root: &Path, op: FileOp) -> Vec<Event> {
    let at = |p: &Path| root.join(p);
    let fail = |what: &str, p: &Path, err: io::Error| {
        vec![Event::Error(format!("Can't {what} {}: {err}", p.display()))]
    };
    match op {
        FileOp::CreateFile(path) => {
            if at(&path).exists() {
                return vec![Event::Error(format!("{} already exists", path.display()))];
            }
            if let Some(dir) = at(&path).parent()
                && let Err(err) = fs::create_dir_all(dir)
            {
                return fail("create", &path, err);
            }
            if let Err(err) = fs::write(at(&path), "") {
                return fail("create", &path, err);
            }
            let config = editorconfig::for_file(&at(&path));
            vec![
                Event::Opened {
                    path: path.clone(),
                    text: String::new(),
                },
                Event::FileConfig { path, config },
            ]
        }
        FileOp::CreateFolder(path) => match fs::create_dir_all(at(&path)) {
            Ok(()) => Vec::new(),
            Err(err) => fail("create", &path, err),
        },
        FileOp::Rename { from, to } => {
            if at(&to).exists() {
                return vec![Event::Error(format!("{} already exists", to.display()))];
            }
            match fs::rename(at(&from), at(&to)) {
                Ok(()) => vec![Event::Renamed { from, to }],
                Err(err) => fail("rename", &from, err),
            }
        }
        FileOp::Trash(path) => match trash::delete(at(&path)) {
            Ok(()) => vec![Event::Trashed(path)],
            Err(err) => vec![Event::Error(format!(
                "Can't move {} to the trash: {err}",
                path.display()
            ))],
        },
    }
}

/// How often to ask the remote what's new.
const FETCH_EVERY: Duration = Duration::from_secs(5 * 60);

/// How long blame waits for the asking to stop, as typing does.
const BLAME_PAUSE: Duration = Duration::from_millis(400);

/// A file's blame to work out: its text as the editor has it.
struct Blame {
    path: PathBuf,
    version: u64,
    text: String,
    ignore: Vec<String>,
}

/// The push, pull or fetch running, shared with the window's thread so it
/// can be stopped.
#[derive(Clone, Default)]
struct GitNet {
    pid: std::sync::Arc<std::sync::Mutex<Option<u32>>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl GitNet {
    /// Stops git, and the ssh it started, as a group.
    fn cancel(&self) {
        let pid = *self.pid.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pid) = pid {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{pid}")])
                .status();
        }
    }
}

/// What git jobs need to know of the project, to run on their own thread.
struct Git {
    root: PathBuf,
    net: GitNet,
    /// Whether the project is in a repository at all.
    repo: bool,
    /// Where the project is in the repository, as git names paths.
    prefix: String,
    /// The repository's git folder.
    dir: Option<PathBuf>,
}

impl Git {
    /// The branch and changed files, with paths relative to the project,
    /// then where the branch stands with its upstream.
    fn status(&self) -> Vec<Event> {
        let output = self.repo.then(|| {
            Command::new("git")
                .args([
                    "status",
                    "--porcelain=v1",
                    "--branch",
                    "--untracked-files=all",
                    "--",
                    ".",
                ])
                .current_dir(&self.root)
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
        });
        let Some(output) = output.flatten() else {
            return vec![
                Event::GitStatus {
                    branch: None,
                    files: Vec::new(),
                    staged: Vec::new(),
                    changed: Vec::new(),
                },
                Event::GitSync(SyncState::default()),
                Event::GitMoves(Vec::new()),
                Event::GitMerge(None),
            ];
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let mut sync = SyncState::default();
        let mut moves = Vec::new();
        let mut branch = None;
        let mut files = Vec::new();
        let mut staged = Vec::new();
        let mut changed = Vec::new();
        for line in text.lines() {
            if let Some(head) = line.strip_prefix("## ") {
                sync = sync_state(head);
                let name = head.split("...").next().unwrap_or(head);
                let name = name.strip_prefix("No commits yet on ").unwrap_or(name);
                branch = Some(if name.starts_with("HEAD") {
                    "detached".into()
                } else {
                    name.to_string()
                });
                continue;
            }
            let (Some(status), Some(path)) = (line.get(..2), line.get(3..)) else {
                continue;
            };
            // A file with conflicts shows as `!`, in neither list.
            if matches!(status, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU") {
                let path = path.trim_matches('"');
                let path = path.strip_prefix(&self.prefix).unwrap_or(path);
                files.push((path.to_string(), '!'));
                continue;
            }
            let letter = match status {
                "??" => '?',
                s => s.chars().find(|c| *c != ' ').unwrap_or('M'),
            };
            // A rename shows as `old -> new`.
            if let Some((from, to)) = path.split_once(" -> ") {
                let strip = |p: &str| {
                    let p = p.trim_matches('"');
                    p.strip_prefix(&self.prefix).unwrap_or(p).to_string()
                };
                moves.push(GitMove {
                    from: strip(from),
                    to: strip(to),
                    staged: true,
                    similarity: None,
                });
            }
            let path = path.rsplit(" -> ").next().unwrap_or(path).trim_matches('"');
            let path = path.strip_prefix(&self.prefix).unwrap_or(path);
            files.push((path.to_string(), letter));
            // X is the index, Y the working tree.
            let mut xy = status.chars();
            let (x, y) = (xy.next().unwrap_or(' '), xy.next().unwrap_or(' '));
            if status == "??" {
                changed.push((path.to_string(), '?'));
                continue;
            }
            if x != ' ' {
                staged.push((path.to_string(), x));
            }
            if y != ' ' {
                changed.push((path.to_string(), y));
            }
        }
        // A branch with nowhere to push yet is published to the first
        // remote.
        if sync.upstream.is_none() {
            sync.remote = Command::new("git")
                .arg("remote")
                .current_dir(&self.root)
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .and_then(|o| {
                    let text = String::from_utf8_lossy(&o.stdout).into_owned();
                    text.lines().next().map(str::to_string)
                });
        }
        moves.extend(self.pair_moves(&changed));
        let conflicted: Vec<String> = files
            .iter()
            .filter(|(_, s)| *s == '!')
            .map(|(p, _)| p.clone())
            .collect();
        let merge = self.merge_state(&conflicted);
        vec![
            Event::GitStatus {
                branch,
                files,
                staged,
                changed,
            },
            Event::GitSync(sync),
            Event::GitMoves(moves),
            Event::GitMerge(merge),
        ]
    }

    /// The merge, rebase or cherry-pick git is in the middle of, from the
    /// files it keeps in its folder, and the conflicts left in each of
    /// `conflicted`, by their markers.
    fn merge_state(&self, conflicted: &[String]) -> Option<MergeState> {
        let dir = self.dir.as_ref()?;
        let read = |name: &str| fs::read_to_string(dir.join(name)).ok();
        let message = read("MERGE_MSG").unwrap_or_default();
        let first = message.lines().next().unwrap_or_default().to_string();
        let (kind, from) = if dir.join("MERGE_HEAD").exists() {
            // "Merge branch 'feature/themes'" names it between quotes.
            let from = first.split('\'').nth(1).unwrap_or(&first).to_string();
            (MergeKind::Merge, from)
        } else if dir.join("rebase-merge").exists() || dir.join("rebase-apply").exists() {
            let onto = read("rebase-merge/head-name")
                .or_else(|| read("rebase-apply/head-name"))
                .unwrap_or_default();
            let from = onto.trim().trim_start_matches("refs/heads/").to_string();
            (MergeKind::Rebase, from)
        } else if dir.join("CHERRY_PICK_HEAD").exists() {
            (MergeKind::CherryPick, first.clone())
        } else {
            return None;
        };
        let conflicts = conflicted
            .iter()
            .map(|p| {
                let text = fs::read_to_string(self.root.join(p)).unwrap_or_default();
                let n = text.lines().filter(|l| l.starts_with("<<<<<<< ")).count();
                (p.clone(), n)
            })
            .collect();
        // Comment lines git adds after the message aren't part of it.
        let message: String = message
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        Some(MergeState {
            kind,
            from,
            message,
            conflicts,
        })
    }

    /// Deleted and new files not staged that are alike enough to be one
    /// file moved, as git pairs them once both are staged: at least half
    /// the same. Only a few of each, and not big ones, are compared.
    fn pair_moves(&self, changed: &[(String, char)]) -> Vec<GitMove> {
        const MOST: usize = 20;
        const BIGGEST: u64 = 512 * 1024;
        let deleted: Vec<&String> = changed
            .iter()
            .filter(|(_, s)| *s == 'D')
            .map(|(p, _)| p)
            .take(MOST)
            .collect();
        if deleted.is_empty() {
            return Vec::new();
        }
        let added: Vec<&String> = changed
            .iter()
            .filter(|(_, s)| *s == '?')
            .map(|(p, _)| p)
            .filter(|p| fs::metadata(self.root.join(p)).is_ok_and(|m| m.len() <= BIGGEST))
            .take(MOST)
            .collect();
        let olds: Vec<(&String, String)> = deleted
            .into_iter()
            .filter_map(|p| Some((p, git_base(&self.root, Path::new(p))?)))
            .collect();
        let mut taken = Vec::new();
        let mut out = Vec::new();
        for new in added {
            let Ok(text) = fs::read_to_string(self.root.join(new)) else {
                continue;
            };
            let best = olds
                .iter()
                .filter(|(p, _)| !taken.contains(p))
                .map(|(p, old)| (*p, merlin_core::diff::similarity(old, &text)))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((from, alike)) = best.filter(|(_, a)| *a >= 0.5) {
                taken.push(from);
                out.push(GitMove {
                    from: from.clone(),
                    to: new.clone(),
                    staged: false,
                    similarity: Some((alike * 100.0).round() as u8),
                });
            }
        }
        out
    }

    /// Each changed file's hunks: in the index, and not yet staged.
    fn diffs(&self) -> Event {
        let diff = |cached: bool| {
            if !self.repo {
                return Vec::new();
            }
            let mut args = vec!["diff", "--no-color", "--no-ext-diff", "--relative"];
            if cached {
                args.push("--cached");
            }
            Command::new("git")
                .args(&args)
                .current_dir(&self.root)
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| merlin_core::git::parse(&String::from_utf8_lossy(&o.stdout)))
                .unwrap_or_default()
        };
        Event::GitDiffs {
            staged: diff(true),
            changed: diff(false),
        }
    }

    /// `git apply` of a patch from `git diff --relative`, whose paths are
    /// the project's: git takes them from the repository's top.
    fn apply(&self, patch: &str, cached: bool, reverse: bool) -> Result<(), String> {
        let directory = format!("--directory={}", self.prefix);
        let mut args = vec!["apply", "--recount", "--whitespace=nowarn"];
        if !self.prefix.is_empty() {
            args.push(&directory);
        }
        if cached {
            args.push("--cached");
        }
        if reverse {
            args.push("-R");
        }
        args.push("-");
        self.git(&args, Some(patch))
    }

    /// Runs git with `args`, and `input` on its standard input. Returns the
    /// first line of what it said when it failed.
    fn git(&self, args: &[&str], input: Option<&str>) -> Result<(), String> {
        use std::io::Write;
        let mut child = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .stdin(if input.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("git: {e}"))?;
        if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
            let _ = stdin.write_all(text.as_bytes());
        }
        let o = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
        if o.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&o.stderr);
        let out = String::from_utf8_lossy(&o.stdout);
        let text = if err.trim().is_empty() { out } else { err };
        let first = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("failed")
            .trim_start_matches("error: ")
            .to_string();
        Err(format!("git {}: {first}", args[0]))
    }

    /// Runs git to talk to a remote: never asking for a password, since
    /// there's no terminal to type it in, and in its own process group so
    /// a cancel stops what it started too. Returns how it went and what it
    /// said.
    fn net(&self, args: &[&str]) -> (SyncOutcome, String) {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(&self.root)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0);
        // ssh asks for passphrases itself: not when nothing set how it runs.
        let configured = Command::new("git")
            .args(["config", "--get", "core.sshCommand"])
            .current_dir(&self.root)
            .output()
            .is_ok_and(|o| !o.stdout.is_empty());
        if env::var_os("GIT_SSH_COMMAND").is_none() && !configured {
            command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => return (SyncOutcome::Failed(format!("git: {e}")), String::new()),
        };
        let lock = |net: &GitNet| net.pid.lock().unwrap_or_else(|e| e.into_inner()).take();
        *self.net.pid.lock().unwrap_or_else(|e| e.into_inner()) = Some(child.id());
        let output = child.wait_with_output();
        lock(&self.net);
        let cancelled = self
            .net
            .cancelled
            .swap(false, std::sync::atomic::Ordering::SeqCst);
        let Ok(o) = output else {
            return (SyncOutcome::Failed("git didn't run".into()), String::new());
        };
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        if cancelled {
            return (SyncOutcome::Cancelled, said);
        }
        if o.status.success() {
            return (SyncOutcome::Done, said);
        }
        let has = |words: &[&str]| words.iter().any(|w| said.contains(w));
        let outcome = if has(&["[rejected]", "non-fast-forward", "fetch first"]) {
            SyncOutcome::Rejected
        } else if has(&[
            "terminal prompts disabled",
            "could not read Username",
            "could not read Password",
            "Authentication failed",
            "Permission denied (publickey",
            "Host key verification failed",
        ]) {
            SyncOutcome::NeedsAuth {
                remote: self.remote().unwrap_or_else(|| "The remote".into()),
            }
        } else if has(&["CONFLICT", "Automatic merge failed", "could not apply"]) {
            SyncOutcome::Conflicts(self.conflicted())
        } else {
            let first = said
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with("hint:"))
                .unwrap_or("failed");
            let first = first
                .trim_start_matches("fatal: ")
                .trim_start_matches("error: ");
            SyncOutcome::Failed(first.to_string())
        };
        (outcome, said)
    }

    /// The remote the branch pushes to, else the first there is.
    fn remote(&self) -> Option<String> {
        let read = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&self.root)
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    let text = String::from_utf8_lossy(&o.stdout).into_owned();
                    text.lines().next().map(str::to_string)
                })
        };
        read(&["rev-parse", "--abbrev-ref", "@{upstream}"])
            .and_then(|u| u.split('/').next().map(str::to_string))
            .or_else(|| read(&["remote"]))
    }

    /// Files with conflicts, as the project names them.
    fn conflicted(&self) -> Vec<String> {
        Command::new("git")
            .args(["diff", "--name-only", "--relative", "--diff-filter=U"])
            .current_dir(&self.root)
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `git pull`, as git is set to: rebase or merge. Set to neither, it
    /// merges, as git did before it asked.
    fn pull(&self) -> (SyncOutcome, String) {
        let set = |key: &str| {
            Command::new("git")
                .args(["config", "--get", key])
                .current_dir(&self.root)
                .output()
                .is_ok_and(|o| !o.stdout.is_empty())
        };
        if set("pull.rebase") || set("pull.ff") {
            self.net(&["pull"])
        } else {
            self.net(&["pull", "--no-rebase"])
        }
    }

    /// A conflicted file's sides for the merge editor: the version both
    /// came from, for each conflict's base, and what each side is, as
    /// "themes · a41c9e2".
    fn merge_sides(&self, path: String) -> Event {
        let base = self
            .read(&["show", &format!(":1:./{path}")])
            .unwrap_or_default();
        let short = |rev: &str| {
            self.read(&["rev-parse", "--short", rev])
                .map(|h| h.trim().to_string())
                .unwrap_or_default()
        };
        let branch = self
            .read(&["branch", "--show-current"])
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "HEAD".into());
        let current = format!("{branch} · {}", short("HEAD"));
        let merge = self.merge_state(&[]);
        let head = ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REBASE_HEAD"]
            .into_iter()
            .find(|h| self.dir.as_ref().is_some_and(|d| d.join(h).exists()))
            .unwrap_or("MERGE_HEAD");
        let from = merge.map(|m| m.from).unwrap_or_else(|| "Incoming".into());
        let incoming = format!("{from} · {}", short(head));
        Event::MergeSides {
            path,
            base,
            incoming,
            current,
        }
    }

    /// Git's words for `args`, or nothing if it failed.
    fn read(&self, args: &[&str]) -> Option<String> {
        Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    }

    /// A commit's message past its summary, and its size.
    fn commit_info(&self, hash: String) -> Event {
        let message = self
            .read(&["show", "-s", "--format=%B", &hash])
            .unwrap_or_default();
        let body = message
            .split_once('\n')
            .map_or("", |(_, rest)| rest)
            .trim()
            .to_string();
        let mut info = merlin_core::editor::CommitInfo {
            body,
            ..Default::default()
        };
        let stat = self
            .read(&["show", "--numstat", "--format=", &hash])
            .unwrap_or_default();
        for line in stat.lines().filter(|l| !l.trim().is_empty()) {
            let mut parts = line.split('\t');
            info.added += parts
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0);
            info.removed += parts
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0);
            info.files += 1;
        }
        Event::CommitInfo { hash, info }
    }

    /// What a commit changed in a file: the file before it and after.
    fn commit_diff(&self, hash: &str, path: PathBuf) -> Event {
        let at = |rev: String| {
            self.read(&["show", &format!("{rev}:./{}", path.display())])
                .unwrap_or_default()
        };
        Event::ShowReview {
            plugin: "merlin.commit".into(),
            id: hash.chars().take(7).collect(),
            path: path.clone(),
            old: at(format!("{hash}^")),
            new: at(hash.to_string()),
        }
    }

    /// Who last changed each line of the editor's text of a file. A file
    /// git doesn't know has none.
    fn blame(&self, ask: Blame) -> Event {
        use std::io::Write;
        let mut args = vec![
            "blame".to_string(),
            "--porcelain".into(),
            "--contents".into(),
        ];
        args.push("-".into());
        for rev in &ask.ignore {
            args.push("--ignore-rev".into());
            args.push(rev.clone());
        }
        args.push("--".into());
        args.push(ask.path.display().to_string());
        let child = Command::new("git")
            .args(&args)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let text = child.ok().and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(ask.text.as_bytes());
            }
            let o = child.wait_with_output().ok()?;
            o.status
                .success()
                .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
        });
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        Event::Blame {
            path: ask.path,
            version: ask.version,
            blame: text
                .map(|t| merlin_core::git::parse_blame(&t))
                .unwrap_or_default(),
            now,
        }
    }

    /// A push, pull, sync, publish or fetch, then what git says now.
    fn sync(&self, kind: SyncKind) -> Vec<Event> {
        let (outcome, output) = match kind {
            SyncKind::Fetch => self.net(&["fetch", "--quiet", "--prune"]),
            SyncKind::Push => self.net(&["push"]),
            SyncKind::Pull => self.pull(),
            SyncKind::Sync => match self.pull() {
                (SyncOutcome::Done, pulled) => {
                    let (outcome, pushed) = self.net(&["push"]);
                    (outcome, pulled + &pushed)
                }
                other => other,
            },
            SyncKind::Publish => match self.remote() {
                Some(remote) => self.net(&["push", "--set-upstream", &remote, "HEAD"]),
                None => (
                    SyncOutcome::Failed("there's no remote to publish to".into()),
                    String::new(),
                ),
            },
        };
        // A rejected push learns how far behind it is, to say so.
        if outcome == SyncOutcome::Rejected {
            self.net(&["fetch", "--quiet"]);
        }
        let mut events = self.status();
        events.push(Event::Synced {
            kind,
            outcome,
            output,
        });
        events
    }

    /// Runs what Source Control asked for, and says how it went.
    fn run(&self, action: GitAction) -> Vec<Event> {
        let done = match &action {
            GitAction::Sync(kind) => return self.sync(*kind),
            GitAction::CancelSync => return Vec::new(),
            // A merge stopped part way, else a rebase.
            GitAction::AbortPull => self
                .git(&["merge", "--abort"], None)
                .or_else(|_| self.git(&["rebase", "--abort"], None))
                .or_else(|_| self.git(&["cherry-pick", "--abort"], None)),
            // No editor opens: git keeps the message it has.
            GitAction::Continue => {
                let kind = if self
                    .dir
                    .as_ref()
                    .is_some_and(|d| d.join("CHERRY_PICK_HEAD").exists())
                {
                    "cherry-pick"
                } else {
                    "rebase"
                };
                let out = Command::new("git")
                    .args([kind, "--continue"])
                    .env("GIT_EDITOR", "true")
                    .current_dir(&self.root)
                    .stdin(std::process::Stdio::null())
                    .output();
                match out {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(format!(
                        "git {kind}: {}",
                        String::from_utf8_lossy(&o.stderr)
                            .lines()
                            .find(|l| !l.trim().is_empty())
                            .unwrap_or("failed")
                    )),
                    Err(e) => Err(format!("git: {e}")),
                }
            }
            GitAction::Stage(path) => self.git(&["add", "--", path], None),
            GitAction::Unstage(path) => self.git(&["restore", "--staged", "--", path], None),
            GitAction::StageAll => self.git(&["add", "--all", "--", "."], None),
            GitAction::UnstageAll => self.git(&["restore", "--staged", "--", "."], None),
            GitAction::Commit(message) => self.git(&["commit", "--quiet", "-m", message], None),
            GitAction::Apply {
                patch,
                cached,
                reverse,
            } => self.apply(patch, *cached, *reverse),
            GitAction::StageMove { from, to } => self.git(&["add", "-A", "--", from, to], None),
            GitAction::Ignore(path) => {
                let file = self.root.join(".gitignore");
                let mut text = fs::read_to_string(&file).unwrap_or_default();
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&format!("/{path}\n"));
                fs::write(&file, text).map_err(|e| format!(".gitignore: {e}"))
            }
            // A deleted file comes back from the last commit, in the index
            // too; there's nothing to undo to.
            GitAction::Discard { path, patch: None } if !self.root.join(path).exists() => {
                let done = self.git(
                    &[
                        "restore",
                        "--source=HEAD",
                        "--staged",
                        "--worktree",
                        "--",
                        path,
                    ],
                    None,
                );
                return match done {
                    Ok(()) => {
                        let name = path.rsplit('/').next().unwrap_or(path);
                        std::iter::once(Event::Notify(format!("Restored {name}")))
                            .chain(self.status())
                            .collect()
                    }
                    Err(e) => vec![Event::Error(e)],
                };
            }
            GitAction::Discard { path, patch } => {
                let file = self.root.join(path);
                let before = fs::read_to_string(&file).unwrap_or_default();
                let done = match patch {
                    Some(patch) => self.apply(patch, false, true),
                    None => self.git(&["restore", "--worktree", "--", path], None),
                };
                return match done {
                    Ok(()) => std::iter::once(Event::Discarded {
                        path: path.clone(),
                        before,
                        all: patch.is_none(),
                    })
                    .chain(self.status())
                    .collect(),
                    Err(e) => vec![Event::Error(e)],
                };
            }
            GitAction::Undiscard { path, text } => {
                fs::write(self.root.join(path), text).map_err(|e| format!("{path}: {e}"))
            }
        };
        match (done, action) {
            (Ok(()), GitAction::Commit(message)) => {
                let first = message.lines().next().unwrap_or_default().to_string();
                vec![Event::Notify(format!("Committed: {first}"))]
            }
            (Ok(()), _) => self.status(),
            (Err(e), _) => vec![Event::Error(e)],
        }
    }
}

/// Where a branch stands, from `git status --branch`'s first line, as
/// `main...origin/main [ahead 1, behind 2]`.
fn sync_state(head: &str) -> SyncState {
    let mut sync = SyncState::default();
    let Some((_, rest)) = head.split_once("...") else {
        return sync;
    };
    let (upstream, counts) = match rest.split_once(" [") {
        Some((u, c)) => (u, c.trim_end_matches(']')),
        None => (rest, ""),
    };
    // An upstream that's gone is none.
    if counts == "gone" {
        return sync;
    }
    sync.upstream = Some(upstream.to_string());
    for part in counts.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            sync.ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            sync.behind = n.parse().unwrap_or(0);
        }
    }
    sync
}

/// Opens a web page in the desktop's browser. Only web pages: a link a
/// plugin sends mustn't open a program.
fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    let _ = Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Starts merlin again in `folder`, on its own, as Open Folder does.
fn open_window(folder: &Path) -> io::Result<()> {
    Command::new(env::current_exe()?)
        .current_dir(folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// `path` relative to the project folder when it's inside it, so the same
/// file always has the same name.
fn relative_to(root: &Path, path: &Path) -> PathBuf {
    let path = root.join(path);
    // A new file can't be canonicalized, but its folder can.
    let full = match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) if !path.exists() => dir
            .canonicalize()
            .map(|d| d.join(name))
            .unwrap_or_else(|_| path.clone()),
        _ => path.canonicalize().unwrap_or_else(|_| path.clone()),
    };
    full.strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or(full)
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The git folder of the repository `root` is in, if any.
fn git_dir(root: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let dir = String::from_utf8(output.stdout).ok()?;
    output.status.success().then(|| root.join(dir.trim()))
}

/// Where `root` is in its repository, as `git status` prefixes paths.
fn git_prefix(root: &Path) -> String {
    Command::new("git")
        .args(["rev-parse", "--show-prefix"])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Lines in the project's files containing `query`, ignoring case unless
/// it has capitals, as find does.
fn search(root: &Path, query: &str) -> Vec<SearchHit> {
    const MAX_HITS: usize = 2000;
    const MAX_FILE: u64 = 2 << 20;
    let fold = !query.chars().any(char::is_uppercase);
    let needle = if fold {
        query.to_lowercase()
    } else {
        query.to_string()
    };
    let mut hits = Vec::new();
    for path in list_files(root) {
        let full = root.join(&path);
        if fs::metadata(&full).map_or(true, |m| m.len() > MAX_FILE) {
            continue;
        }
        // Not text: skip it.
        let Ok(text) = fs::read_to_string(&full) else {
            continue;
        };
        for (line, content) in text.lines().enumerate() {
            let hay = if fold {
                content.to_lowercase()
            } else {
                content.to_string()
            };
            let Some(at) = hay.find(&needle) else {
                continue;
            };
            // Lowercasing can change byte offsets; count chars instead.
            let chars = hay[..at].chars().count();
            let column = content
                .chars()
                .take(chars)
                .map(char::len_utf16)
                .sum::<usize>();
            let shown: String = content.chars().take(200).collect();
            hits.push(SearchHit {
                path: path.clone(),
                line: line as u32,
                column: column as u32,
                text: shown,
            });
            if hits.len() >= MAX_HITS {
                return hits;
            }
        }
    }
    hits
}

/// A file's changes since the last commit, from `from` as it was then to
/// `to` as it is now: as text, or a binary file's sizes.
fn changes_of(root: &Path, from: &Path, to: PathBuf) -> Event {
    let base = Command::new("git")
        .arg("show")
        .arg(format!("HEAD:./{}", from.display()))
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout)
        .unwrap_or_default();
    let now = fs::read(root.join(&to)).unwrap_or_default();
    // A NUL near the start says it isn't text, as git judges.
    let binary = |b: &[u8]| b.iter().take(8000).any(|&c| c == 0);
    if binary(&base) || binary(&now) {
        return Event::ShowBinary {
            path: to,
            old: base.len() as u64,
            new: now.len() as u64,
        };
    }
    Event::ShowDiff {
        path: to,
        old: String::from_utf8_lossy(&base).into_owned(),
        new: String::from_utf8_lossy(&now).into_owned(),
    }
}

/// A file as of the last commit, or `None` if git doesn't track it.
fn git_base(root: &Path, path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("show")
        .arg(format!("HEAD:./{}", path.display()))
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The project's files for the open-file picker, skipping hidden and
/// git-ignored ones.
/// The project's files and folders for the Explorer, folders ending in
/// `/` so empty ones show.
fn list_tree(root: &Path) -> Vec<String> {
    const MAX: usize = 50_000;
    let mut paths: Vec<String> = ignore::WalkBuilder::new(root)
        .build()
        .flatten()
        .filter_map(|e| {
            let rel = e.path().strip_prefix(root).ok()?.display().to_string();
            if rel.is_empty() {
                return None;
            }
            let dir = e.file_type().is_some_and(|t| t.is_dir());
            Some(if dir { format!("{rel}/") } else { rel })
        })
        .take(MAX)
        .collect();
    paths.sort();
    paths
}

fn list_files(root: &Path) -> Vec<String> {
    const MAX: usize = 50_000;
    let mut files: Vec<String> = ignore::WalkBuilder::new(root)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| Some(e.path().strip_prefix(root).ok()?.display().to_string()))
        .take(MAX)
        .collect();
    files.sort();
    files
}

fn snapshot(editor: &Editor, root: &Path) -> Snapshot {
    let buffer = editor.buffer();
    Snapshot {
        path: buffer.path.as_ref().map(|p| p.display().to_string()),
        text: buffer.snapshot(),
        root: root.to_path_buf(),
        cursor: editor.cursor_utf16(),
        selection: editor
            .selection_utf16()
            .zip(editor.selection())
            .map(|((start, end), text)| Selection { start, end, text }),
        changed: editor.changed_lines(),
    }
}

/// glibc gives each thread that allocates a heap of its own, and keeps
/// what's freed in it: wasmtime's threads compiling plugins left about
/// 170 MB that way. Two are plenty for an editor.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn fewer_heaps() {
    unsafe extern "C" {
        fn mallopt(param: i32, value: i32) -> i32;
    }
    const M_ARENA_MAX: i32 = -8;
    // SAFETY: mallopt only changes the allocator's settings, before any
    // other thread starts.
    unsafe {
        mallopt(M_ARENA_MAX, 2);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn fewer_heaps() {}

fn main() -> Result<()> {
    fewer_heaps();
    let argv: Vec<String> = env::args().skip(1).collect();
    if let [plugin, new, name] = &argv[..]
        && plugin == "plugin"
        && new == "new"
    {
        return template::create(name);
    }
    if let [plugin, install, dirs @ ..] = &argv[..]
        && plugin == "plugin"
        && install == "install"
        && !dirs.is_empty()
    {
        return install_plugins(dirs);
    }
    let args = parse_args()?;
    let root = env::current_dir()
        .and_then(|d| d.canonicalize())
        .context("reading the current folder")?;
    let git_dir = git_dir(&root);
    let git_prefix = git_prefix(&root);
    let config = Config {
        ask: args.exec.is_none(),
        ..Config::standard()
    };
    let settings_files = settings::Files::new(&root);
    let data_dir = config.data_dir.clone();
    let gui = args.exec.is_none();
    // Scripts remember nothing, and so keep nothing.
    let backups_dir = gui.then(|| {
        data_dir
            .join("backups")
            .join(safe_name(&root.display().to_string()))
    });
    // The minimap and word wrap used to be remembered here; they're
    // settings now.
    let view_path = config.data_dir.join("view");
    if args.exec.is_none()
        && let Ok(old) = fs::read_to_string(&view_path)
    {
        if !settings_files.user.exists() {
            for (line, key) in [
                ("minimap = on", "editor.minimap"),
                ("wrap = on", "editor.word_wrap"),
            ] {
                if old.lines().any(|l| l.trim() == line) {
                    let _ = settings_files.ensure(false);
                    let _ = settings_files.set(key, "true");
                }
            }
        }
        let _ = fs::remove_file(&view_path);
    }
    let mut app = App {
        zoom: 0.0,
        editor: Editor::new("", None),
        host: PluginHost::new(config)?,
        root,
        panels_for: None,
        buffer_seen: None,
        saved: Vec::new(),
        clipboard: None,
        terminals: terminal::Terminals::new(gui::waker()),
        disk: HashMap::new(),
        fetch_due: git_dir.is_some().then(|| Instant::now() + FETCH_EVERY),
        blame_due: None,
        git_prefix,
        git_dir,
        git_index: None,
        git_net: GitNet::default(),
        window_focused: true,
        settings: settings_files,
        scripted: args.exec.is_some(),
        look: packs::Look::stack(&[]).expect("merlin's own look"),
        window: settings::Window::default(),
        packs: Vec::new(),
        choice: looks::Choice::default(),
        watched: Vec::new(),
        jobs: if gui {
            Jobs::threaded(gui::waker())
        } else {
            Jobs::inline()
        },
        watch: if gui {
            Watch::new(gui::waker())
        } else {
            Watch::none()
        },
        changes: Changes::default(),
        settle: None,
        tree_dirs: Vec::new(),
        tree_hash: None,
        watch_for: None,
        backups: Backups::new(backups_dir),
        auto_save: None,
        auto_save_for: Vec::new(),
        format_due: None,
        quit: false,
    };
    // Nerd Font icons, except in scripted runs, whose output should read
    // anywhere. MERLIN_ICONS=nerd or =plain decides either way.
    let nerd = match env::var("MERLIN_ICONS").as_deref() {
        Ok("nerd") => true,
        Ok("plain") => false,
        _ => args.exec.is_none(),
    };
    app.editor.set_nerd_icons(nerd);
    // Before plugins load, so they start with their settings.
    for event in app.load_settings() {
        app.dispatch(event);
    }
    if let Some(name) = app.root.file_name() {
        app.editor.set_project(name.to_string_lossy().to_string());
    }
    let root = app.root.clone();
    app.editor.set_root(root);
    for file in &args.files {
        for event in app.open(file) {
            if let Event::Error(err) = event {
                bail!(err);
            }
            app.dispatch(event);
        }
    }
    if !args.files.is_empty() {
        app.dispatch(Event::Command("file.next".into()));
    }
    // Started without files: the tabs and groups open when this folder was
    // last closed, as VS Code reopens a workspace.
    let workspace_path = workspace_file(view_path.parent().unwrap_or(Path::new(".")), &app.root);
    if args.files.is_empty() && args.exec.is_none() {
        let ws = Workspace::parse(&fs::read_to_string(&workspace_path).unwrap_or_default());
        for file in ws.files() {
            if app.root.join(&file).is_file() {
                for event in app.open(&file) {
                    app.dispatch(event);
                }
            }
        }
        app.dispatch(Event::Restore(ws));
    }
    // Unsaved changes a crash left behind open again, unsaved.
    let kept = app.backups.left_over();
    if !kept.is_empty() {
        let count = kept.len();
        for kept in kept {
            if let Some(path) = &kept.path
                && !app.editor.open_files().contains(&Some(path.clone()))
            {
                for event in app.open(path) {
                    app.dispatch(event);
                }
            }
            app.dispatch(Event::Recovered {
                path: kept.path,
                text: kept.text,
            });
        }
        let files = if count == 1 { "file" } else { "files" };
        app.dispatch(Event::Notify(format!(
            "Restored unsaved changes to {count} {files}"
        )));
        // The file asked for stays in front of the ones restored.
        if let Some(first) = args.files.first() {
            for event in app.open(first) {
                app.dispatch(event);
            }
        }
    }
    // Loads git signs for the files opened so far; later ones get theirs
    // when they open.
    app.check_disk();
    if gui {
        gui::set_waker(&mut app.host);
        // The Explorer's tree, and the folders to watch.
        app.list_tree_later();
    }
    for dir in plugin_dirs(&args) {
        match app.host.load(&dir) {
            Ok(events) => events.into_iter().for_each(|e| app.dispatch(e)),
            Err(err) => app.dispatch(Event::PluginFailed {
                plugin: dir.display().to_string(),
                message: format!("{err:#}"),
            }),
        }
    }
    // Plugins' packs are known now, so the ones the settings ask for can be
    // stacked.
    for event in app.refresh_looks() {
        app.dispatch(event);
    }

    if let Some(script) = &args.exec {
        app.dispatch(Event::Resize(args.size.0, args.size.1));
        let result = run_script(&mut app, script);
        // Drawing may ask for colours, such as a code block's.
        app.editor.dump();
        app.serve_colours();
        print!("{}", app.editor.dump());
        return result;
    }

    let result = gui::run(&mut app);
    app.finish_all();
    if result.is_ok() {
        app.backups.clear();
    }
    let _ = fs::create_dir_all(workspace_path.parent().unwrap_or(Path::new(".")))
        .and_then(|()| fs::write(&workspace_path, app.editor.workspace().to_text()));
    result
}

/// Where the open tabs and groups of the project at `root` are kept
/// between runs.
fn workspace_file(data_dir: &Path, root: &Path) -> PathBuf {
    data_dir
        .join("workspaces")
        .join(safe_name(&root.display().to_string()))
}

/// `text` as a file name: letters, digits, `.` and `-`, and `_` for the
/// rest.
pub(crate) fn safe_name(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn run_script(app: &mut App, script: &str) -> Result<()> {
    for step in script::parse(script).map_err(anyhow::Error::msg)? {
        match step {
            Step::Event(event) => app.dispatch(event),
            Step::Wait(ms) => {
                let until = Instant::now() + Duration::from_millis(ms);
                while Instant::now() < until {
                    if !app.poll_processes() {
                        app.check_disk();
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
            Step::WaitFor { text, timeout_ms } => {
                let until = Instant::now() + Duration::from_millis(timeout_ms);
                while !app.editor.dump().contains(&text) {
                    if Instant::now() >= until {
                        bail!("timed out waiting for {text:?}");
                    }
                    if !app.poll_processes() {
                        app.check_disk();
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        }
        app.poll_processes();
        if app.quit {
            break;
        }
    }
    Ok(())
}
