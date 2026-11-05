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
    AutoSaveWhy, Editor, Effect, Event, FileOp, FilePick, GitAction, SearchHit, Workspace,
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
                    Effect::Git(action) => {
                        let commit = matches!(action, GitAction::Commit(_));
                        let git = self.git_info();
                        self.jobs.run(Lane::Git, move || {
                            let done = git.run(action);
                            Box::new(move |app: &mut App| {
                                if commit {
                                    // The new commit is the base for gutter
                                    // signs.
                                    app.git_index = None;
                                    app.check_disk();
                                }
                                vec![done]
                            })
                        });
                    }
                    Effect::OpenDiff(path) => {
                        let root = self.root.clone();
                        self.jobs.run(Lane::Git, move || {
                            let old = git_base(&root, &path).unwrap_or_default();
                            let new = fs::read_to_string(root.join(&path)).unwrap_or_default();
                            jobs::events(vec![Event::ShowDiff { path, old, new }])
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
        }
    }

    fn git_status_later(&mut self) {
        let git = self.git_info();
        self.jobs.run(Lane::Git, move || {
            let status = git.status();
            Box::new(move |app: &mut App| {
                // git status can touch the index; that's not a commit.
                app.git_index = app
                    .git_dir
                    .as_ref()
                    .and_then(|d| modified(&d.join("index")));
                vec![status]
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
            TermOp::Spawn { id } => t.spawn(id, &self.root),
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
            self.settle,
            self.backups.due,
            self.auto_save,
            self.format_due.map(|(at, _)| at),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Does what's due by now. Returns whether the editor heard anything.
    pub(crate) fn tick(&mut self) -> bool {
        let now = Instant::now();
        let mut changed = false;
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

/// What git jobs need to know of the project, to run on their own thread.
struct Git {
    root: PathBuf,
    /// Whether the project is in a repository at all.
    repo: bool,
    /// Where the project is in the repository, as git names paths.
    prefix: String,
}

impl Git {
    /// The branch and changed files, with paths relative to the project.
    fn status(&self) -> Event {
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
            return Event::GitStatus {
                branch: None,
                files: Vec::new(),
                staged: Vec::new(),
                changed: Vec::new(),
            };
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let mut branch = None;
        let mut files = Vec::new();
        let mut staged = Vec::new();
        let mut changed = Vec::new();
        for line in text.lines() {
            if let Some(head) = line.strip_prefix("## ") {
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
            let letter = match status {
                "??" => '?',
                s => s.chars().find(|c| *c != ' ').unwrap_or('M'),
            };
            // A rename shows as `old -> new`.
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
        Event::GitStatus {
            branch,
            files,
            staged,
            changed,
        }
    }

    /// Runs what Source Control asked for, and says how it went.
    fn run(&self, action: GitAction) -> Event {
        let args: Vec<&str> = match &action {
            GitAction::Stage(path) => vec!["add", "--", path],
            GitAction::Unstage(path) => vec!["restore", "--staged", "--", path],
            GitAction::StageAll => vec!["add", "--all", "--", "."],
            GitAction::UnstageAll => vec!["restore", "--staged", "--", "."],
            GitAction::Commit(message) => vec!["commit", "--quiet", "-m", message],
        };
        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::null())
            .output();
        match output {
            Ok(o) if o.status.success() => match action {
                GitAction::Commit(message) => {
                    let first = message.lines().next().unwrap_or_default().to_string();
                    Event::Notify(format!("Committed: {first}"))
                }
                _ => self.status(),
            },
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                let out = String::from_utf8_lossy(&o.stdout);
                let text = if err.trim().is_empty() { out } else { err };
                let first = text
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("failed");
                Event::Error(format!("git {}: {first}", args[0]))
            }
            Err(err) => Event::Error(format!("git: {err}")),
        }
    }
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
        git_prefix,
        git_dir,
        git_index: None,
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
