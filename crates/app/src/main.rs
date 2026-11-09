// A window, not a console program, on Windows: no console opens beside
// it. `attach_console` gives the command line its output back.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod browse;
mod clipboard;
mod disk;
mod editorconfig;
mod effects;
mod git;
mod gui;
mod jobs;
mod looks;
mod packs;
mod plugin_cli;
mod prefs;
mod settings;
mod template;
mod terminal;
mod theme;
mod timers;

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

use git::*;

const USAGE: &str = "\
merlin - a code editor with sandboxed plugins

Usage: merlin [options] [file...]
       merlin plugin new <name>   Start a plugin in a new folder
       merlin plugin install <name | dir | file.merlin>...
                                  Install plugins: by name from a registry
                                  (name@1.2.0 for a version, source:name for
                                  a source), a folder linked so rebuilding
                                  it updates it, or a package unpacked
       merlin plugin search [text]
                                  Search the registries
       merlin plugin update [name...]
                                  Update plugins from their registries
       merlin plugin uninstall <name>...
                                  Remove installed plugins
       merlin plugin fetch <dir> [name...]
                                  Download merlin's registry's plugins for
                                  this merlin into a folder, as a release's
                                  plugins/ holds them
       merlin plugin pack [dir] [--out <dir>]
                                  Pack a built plugin into
                                  <name>-<version>.merlin

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
  -v, --version      Show version
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
            "-v" | "--version" => {
                println!("merlin v{}", env!("CARGO_PKG_VERSION"));
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
    let installed = settings::plugins_dir();
    if let Some(installed) = &installed {
        // An install that stopped halfway is put right before anything loads.
        merlin_plugins::package::recover(installed);
    }
    if let Some(installed) = installed
        && let Ok(entries) = fs::read_dir(installed)
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

/// Connects the editor to the outside world: it carries out the editor's
/// effects and feeds the results back in as events.
struct App {
    editor: Editor,
    /// Points the code's font is zoomed by, from Ctrl+= and Ctrl+-.
    zoom: f32,
    host: PluginHost,
    root: PathBuf,
    /// The file and buffer version the panels were last rendered for.
    /// What panels were last drawn for: the file and its version, the
    /// views that showed, and whether the right side bar did.
    panels_for: Option<((u64, u64), Vec<String>, bool)>,
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
    /// A save waiting for its formatting gives up and saves; which-key
    /// shows after a pause on a prefix; the palette searches the project's
    /// text; the language server is asked about the name under Ctrl.
    format_wait: timers::Wait,
    which_key_wait: timers::Wait,
    palette_search_wait: timers::Wait,
    link_wait: timers::Wait,
    quit: bool,
    /// The Settings tab's state.
    settings_ui: prefs::SettingsUi,
    /// The settings files as last read.
    loaded: prefs::Loaded,
    /// Each colour theme's palette, and each icon theme's icons and where
    /// it's from, for the Settings tab's cards.
    previews: Vec<(String, theme::Palette)>,
    icon_previews: Vec<(String, packs::Icons, String)>,
    /// The plugin whose Uninstall was pressed once.
    uninstall_armed: Option<String>,
    /// When plugins' views that asked for a frame are drawn again, and
    /// when they last were.
    frame_due: Option<Instant>,
    last_frame: Instant,
    /// Whether plugins have loaded, and documents opened before then, to
    /// hand them once they have.
    plugins_loaded: bool,
    documents_waiting: Vec<merlin_core::Effect>,
    /// The tiles last asked of each document's plugin, still to come.
    tiles_asked: Vec<(u32, std::collections::HashSet<merlin_core::TileKey>)>,
    /// Programs' memory, for the Plugins page.
    memory: prefs::Memory,
    /// The desktop asks for light colours, for Follow the system.
    system_light: bool,
    /// The plugin registries, for Browse and updates.
    browse: browse::Browse,
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
        self.touch_waits();
        self.refresh_watch();
    }

    /// Asks language plugins to colour what the editor wants coloured that
    /// isn't an open file: diffs, and code blocks in Markdown. Returns
    /// whether there was any.
    fn serve_colours(&mut self) -> bool {
        let wanted = self.editor.colour_requests();
        if wanted.is_empty() {
            return false;
        }
        // Coloured now, or later on the plugin's thread.
        let events = wanted
            .into_iter()
            .flat_map(|(language, text)| self.host.colour_text(&language, &text))
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
        let (user, workspace) = if self.scripted {
            Default::default()
        } else {
            self.settings.each()
        };
        self.loaded = prefs::Loaded {
            merged: table.clone(),
            user,
            workspace,
            errors: errors.clone(),
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
        // Plugins turned off, for you or in this workspace: off now, and
        // left off when they load.
        let mut toggled = Vec::new();
        if !self.scripted {
            let (mut off, here) = self.off_lists();
            off.extend(here);
            // And what a registry blocked, whatever the settings say.
            off.extend(self.browse.blocked().into_iter().map(|(name, _)| name));
            for p in self.host.plugins() {
                let on = !off.contains(&p.name);
                if !p.data_only && p.enabled != on {
                    toggled.extend(self.host.set_enabled(&p.name, on));
                }
            }
            self.host.set_disabled(off);
        }
        let snapshot = self.snapshot();
        let heard = self.host.set_settings(
            settings::plugins(&self.loaded.user),
            settings::plugins(&self.loaded.workspace),
            &snapshot,
        );
        self.panels_for = None;
        self.choice.colours = self.chosen_colours();
        self.choice.icons = settings::editor_text(&table, "icon_theme").map(str::to_string);
        self.choice.packs = settings::editor_list(&table, "packs");
        let mut events = vec![Event::Settings(editor)];
        events.extend(toggled);
        events.extend(heard);
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
        // A document a plugin draws isn't read as text: its plugin reads it.
        if self.editor.opens_as_document(&path) {
            return vec![Event::Opened {
                path,
                text: String::new(),
            }];
        }
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
        // Plugin views that show, and, while the right side bar does, every
        // plugin's panel, so it knows which have one.
        let shown = self.editor.visible_views();
        let all = self.editor.panel_visible();
        if shown.is_empty() && !all {
            self.panels_for = None;
            return;
        }
        let version = (self.editor.doc_id(), self.editor.buffer().version());
        let key = (version, shown, all);
        if self.panels_for.as_ref() == Some(&key) {
            return;
        }
        // Panel events only change what's drawn; they produce no effects.
        for event in self.host.render_panels(&self.snapshot(), &key.1, all) {
            self.editor.update(event);
        }
        self.panels_for = Some(key);
    }

    /// The tiles a document shows and doesn't have. Asks its plugin for
    /// them when any weren't asked for already, which drops the ones
    /// asked for before that the view has moved on from.
    pub(crate) fn want_tiles(
        &mut self,
        surface: u32,
        plugin: &str,
        wanted: Vec<merlin_core::TileKey>,
    ) {
        let asked = match self.tiles_asked.iter_mut().find(|(s, _)| *s == surface) {
            Some((_, asked)) => asked,
            None => {
                self.tiles_asked.push((surface, Default::default()));
                &mut self.tiles_asked.last_mut().unwrap().1
            }
        };
        if wanted.iter().all(|t| asked.contains(t)) {
            return;
        }
        *asked = wanted.iter().copied().collect();
        let events = self.host.render_tiles(plugin, surface, wanted);
        for event in events {
            self.dispatch(event);
        }
    }

    /// Hands queued process output to plugins, and applies what their
    /// calls asked for. Returns whether there was any.
    fn poll_processes(&mut self) -> bool {
        let (editor, root) = (&self.editor, &self.root);
        let (events, changed) = self.host.poll_processes(|| snapshot(editor, root));
        let any = changed || !events.is_empty();
        // A view drawn or text coloured changes no plugin, so the views
        // aren't asked for again; anything else may have.
        if changed {
            self.panels_for = None;
        }
        for event in events {
            self.dispatch(event);
        }
        if changed {
            self.refresh_panels();
        }
        any
    }

    fn reload_plugins(&mut self) {
        let events = self.host.reload_changed();
        if !events.is_empty() {
            self.panels_for = None;
            for event in events {
                self.dispatch(event);
            }
            let summaries = self.plugin_summaries();
            self.dispatch(summaries);
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
/// On Windows, writes to the terminal merlin was started from, if any, so
/// `merlin --version` and `merlin plugin …` print there although merlin is
/// a window program. Started from the Start menu, there's none, and
/// nothing changes.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: a plain call with no pointers; it fails harmlessly when
    // there's no parent console or one is attached already.
    unsafe {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Opens a folder in the desktop's file manager, or a web page in the
/// browser.
pub(crate) fn open_folder(dir: &Path) {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(windows)]
    let mut command = Command::new("explorer");
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut command = Command::new("xdg-open");
    let _ = command
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
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
        menu_target: editor.menu_target(),
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
    attach_console();
    fewer_heaps();
    let argv: Vec<String> = env::args().skip(1).collect();
    if let [plugin, new, name] = &argv[..]
        && plugin == "plugin"
        && new == "new"
    {
        return template::create(name);
    }
    if let [plugin, command, rest @ ..] = &argv[..]
        && plugin == "plugin"
        && let Some(run) = plugin_cli::command(command)
    {
        return run(rest, USAGE);
    }
    let args = parse_args()?;
    let root = env::current_dir()
        .and_then(|d| d.canonicalize())
        .context("reading the current folder")?;
    let git_dir = git_dir(&root);
    let git_prefix = git_prefix(&root);
    // The window never waits for a plugin; a script does, so its steps
    // happen in order.
    let config = Config {
        ask: args.exec.is_none(),
        background: args.exec.is_none(),
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
    // The registries as last seen, to know what's blocked before plugins
    // load.
    let settings_user = settings_files.each().0;
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
        format_wait: timers::Wait::default(),
        which_key_wait: timers::Wait::default(),
        palette_search_wait: timers::Wait::default(),
        link_wait: timers::Wait::default(),
        quit: false,
        settings_ui: prefs::SettingsUi::default(),
        loaded: prefs::Loaded::default(),
        previews: Vec::new(),
        icon_previews: Vec::new(),
        uninstall_armed: None,
        tiles_asked: Vec::new(),
        plugins_loaded: false,
        frame_due: None,
        last_frame: Instant::now(),
        documents_waiting: Vec::new(),
        memory: prefs::Memory::default(),
        system_light: false,
        browse: browse::Browse::new(data_dir.clone(), &settings_user, gui),
    };
    app.editor.set_utc_offset(local_utc_offset());
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
    // The plugins that come with merlin, on its first run or after an
    // update, before any plugin loads. Scripts leave the user's alone.
    let bundled = if args.exec.is_none() {
        browse::install_bundled(&Config::standard())
    } else {
        Vec::new()
    };
    // The file types plugins open, such as PDFs, known before any file
    // opens, so those open as their documents once the plugin loads.
    for dir in plugin_dirs(&args) {
        if let Ok(manifest) = merlin_plugins::manifest(&dir)
            && let Some(event) = merlin_plugins::editors_event(&manifest)
        {
            app.dispatch(event);
        }
    }
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
    } else if args.exec.is_none() {
        let ws = Workspace::parse(&fs::read_to_string(&workspace_path).unwrap_or_default());
        app.dispatch(Event::RestoreBreakpoints(ws));
    }
    // Unsaved changes the last run kept, with a hot exit or a crash, open
    // again, unsaved.
    let (kept, hot) = app.backups.left_over();
    if !kept.is_empty() {
        for kept in kept {
            if let Some(path) = &kept.path
                && !app.editor.open_files().contains(&Some(path.clone()))
            {
                for event in app.open(path) {
                    app.dispatch(event);
                }
            }
            // Changed on disk since the changes were made to it.
            let moved = kept.path.as_ref().is_some_and(|path| {
                let now = modified(&app.root.join(path));
                kept.seen.is_some() && now.is_some() && now != kept.seen
            });
            app.dispatch(Event::Recovered {
                path: kept.path,
                text: kept.text,
                moved,
            });
        }
        app.dispatch(Event::RecoveredAll { crashed: !hot });
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
    // Every plugin loads before any hears of the open file, so a fallback
    // already sees the language plugins it leaves their files to.
    let mut loaded = Vec::new();
    for dir in plugin_dirs(&args) {
        match app.host.load(&dir) {
            Ok(events) => loaded.extend(events),
            Err(err) => loaded.push(Event::PluginFailed {
                plugin: dir.display().to_string(),
                message: format!("{err:#}"),
            }),
        }
    }
    loaded.extend(app.host.check_depends());
    loaded.into_iter().for_each(|e| app.dispatch(e));
    // Documents that opened before their plugin had loaded.
    app.plugins_loaded = true;
    for effect in std::mem::take(&mut app.documents_waiting) {
        let mut queue = VecDeque::new();
        app.run_effect(effect, &mut queue);
        app.run_events(queue);
    }
    let summaries = app.plugin_summaries();
    app.dispatch(summaries);
    bundled.into_iter().for_each(|e| app.dispatch(e));
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
    if result.is_ok() && app.editor.keeps_unsaved() {
        let (editor, disk) = (&app.editor, &app.disk);
        let seen = |path: &Path| disk.get(path).copied().flatten();
        let plan = app
            .backups
            .plan(editor.unsaved(), |id| editor.text_of(id), seen);
        app.backups.keep(plan);
    } else if result.is_ok() {
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

/// Whether a program can be started by name: a path that exists, or a
/// file of that name in a folder on `PATH`.
fn on_path(program: &str) -> bool {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return path.is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|dirs| std::env::split_paths(&dirs).any(|dir| dir.join(program).is_file()))
}

/// `.merlin/launch.toml`'s `[[debug]]` tables as things to debug. The keys
/// merlin knows are its own; the rest go to the debugger as they are, and
/// `attach = <pid>` attaches.
fn launch_configs(text: &str) -> Result<Vec<merlin_core::DebugTarget>, String> {
    let table: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| e.message().to_string())?;
    let Some(list) = table.get("debug").and_then(|d| d.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let Some(t) = item.as_table() else { continue };
        let text = |key: &str| {
            t.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let mut launch = serde_json::Map::new();
        for (key, value) in t {
            match key.as_str() {
                "name" | "program" | "args" | "build" | "debugger" | "language" => {}
                "attach" => {
                    if let Some(pid) = value.as_integer() {
                        launch.insert("pid".into(), serde_json::json!(pid));
                    }
                }
                _ => {
                    if let Ok(v) = serde_json::to_value(value) {
                        launch.insert(key.clone(), v);
                    }
                }
            }
        }
        let name = match text("name") {
            n if n.is_empty() => format!("Configuration {}", i + 1),
            n => n,
        };
        out.push(merlin_core::DebugTarget {
            name,
            debugger: text("debugger"),
            language: text("language"),
            program: text("program"),
            args: t
                .get("args")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            build: text("build"),
            detail: String::new(),
            launch: serde_json::Value::Object(launch).to_string(),
            plugin: None,
        });
    }
    Ok(out)
}

/// The user's own processes, newest first, as pid, name and command line,
/// leaving out merlin and what it started; and Linux's `ptrace_scope`, 0
/// when there's none.
fn processes() -> (Vec<(u32, String, String)>, u8) {
    let scope = fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let me = std::process::id();
    let uid = |pid: &str| -> Option<String> {
        let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let line = status.lines().find(|l| l.starts_with("Uid:"))?;
        line.split_whitespace().nth(1).map(String::from)
    };
    let mine = uid("self");
    let mut list: Vec<(u64, u32, String, String)> = Vec::new();
    let Ok(dir) = fs::read_dir("/proc") else {
        return (Vec::new(), scope);
    };
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == me || uid(&name) != mine {
            continue;
        }
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        // The parent, and when it started, are after the name's brackets.
        let after = stat.rsplit_once(") ").map_or("", |(_, r)| r);
        let fields: Vec<&str> = after.split_whitespace().collect();
        if fields.get(1).and_then(|p| p.parse::<u32>().ok()) == Some(me) {
            continue;
        }
        let started = fields.get(19).and_then(|s| s.parse().ok()).unwrap_or(0);
        let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let command = fs::read(format!("/proc/{pid}/cmdline"))
            .map(|b| {
                String::from_utf8_lossy(&b)
                    .replace('\0', " ")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        if command.is_empty() {
            continue;
        }
        list.push((started, pid, comm.trim().to_string(), command));
    }
    list.sort_by_key(|p| std::cmp::Reverse(p.0));
    (
        list.into_iter()
            .map(|(_, pid, name, cmd)| (pid, name, cmd))
            .collect(),
        scope,
    )
}

/// The local time's offset from UTC now, in seconds.
#[cfg(unix)]
fn local_utc_offset() -> i64 {
    // SAFETY: localtime_r writes only to `tm`, which lives here.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i64
    }
}

#[cfg(not(unix))]
fn local_utc_offset() -> i64 {
    0
}

#[cfg(test)]
mod launch_tests {
    #[test]
    fn launch_toml_keeps_the_debuggers_own_keys() {
        let configs = super::launch_configs(
            "[[debug]]\nname = \"notes\"\nprogram = \"target/debug/m\"\nargs = [\"~/notes\"]\n\
             build = \"cargo build\"\nenv = { A = \"1\" }\njustMyCode = false\n\n\
             [[debug]]\nname = \"attach\"\nattach = 42\n",
        )
        .unwrap();
        assert_eq!(configs[0].name, "notes");
        assert_eq!(configs[0].args, ["~/notes"]);
        assert_eq!(configs[0].build, "cargo build");
        assert_eq!(configs[0].launch, r#"{"env":{"A":"1"},"justMyCode":false}"#);
        assert_eq!(configs[1].launch, r#"{"pid":42}"#);
        assert!(super::launch_configs("[[debug]\n").is_err());
    }
}
