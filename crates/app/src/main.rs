mod clipboard;
mod gui;
mod settings;
mod template;
mod theme;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};
use std::{env, fs, io};

use anyhow::{Context, Result, bail};
use merlin_core::screen::{Screen, WIDE_TAIL};
use merlin_core::script::{self, Step};
use merlin_core::{Editor, Effect, Event, GitAction, SearchHit, Workspace};
use merlin_plugins::{BufferEvent, Config, PluginHost, Selection, Snapshot};

const USAGE: &str = "\
merlin - a code editor with sandboxed plugins

Usage: merlin [options] [file...]
       merlin plugin new <name>   Start a plugin in a new folder

Options:
  --plugin <dir>     Load the plugin in <dir> (repeatable). Plugins in
                     ~/.config/merlin/plugins load too. Plugins reload when
                     their component file changes.
  --exec <script>    Run a script headless and print the final screen
                     (see crates/core/src/script.rs for the format)
  --size <WxH>       Screen size for --exec (default 80x24)
  --html <file>      With --exec, also write the final screen, in colour,
                     as an HTML page. Scripts grant plugins their
                     permissions without asking, and remember nothing.
  -h, --help         Show this help
";

struct Args {
    files: Vec<PathBuf>,
    plugins: Vec<PathBuf>,
    exec: Option<String>,
    size: (u16, u16),
    html: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        files: Vec::new(),
        plugins: Vec::new(),
        exec: None,
        size: (80, 24),
        html: None,
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
            "--html" => args.html = Some(it.next().context("--html needs a file")?.into()),
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
        dirs.extend(found.into_iter().filter(|d| d.join("plugin.toml").exists()));
    }
    dirs
}

/// Connects the editor to the outside world: it carries out the editor's
/// effects and feeds the results back in as events.
struct App {
    editor: Editor,
    host: PluginHost,
    root: PathBuf,
    /// The file and buffer version the panels were last rendered for.
    panels_for: Option<(u64, u64)>,
    /// The file and buffer version plugins last heard about.
    buffer_seen: Option<(u64, u64)>,
    /// Paths saved since plugins last heard.
    saved: Vec<String>,
    /// Off for scripts, so they don't touch the user's clipboard.
    system_clipboard: bool,
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
        while let Some(event) = queue.pop_front() {
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
                    } => match fs::write(self.root.join(&path), text) {
                        Ok(()) => {
                            self.disk
                                .insert(path.clone(), modified(&self.root.join(&path)));
                            self.saved.push(path.display().to_string());
                            let settings = self.is_settings(&path);
                            queue.push_back(Event::Saved {
                                doc,
                                revision,
                                path,
                            });
                            // Saving a settings file applies it.
                            if settings {
                                queue.extend(self.load_settings());
                            }
                        }
                        Err(err) => queue.push_back(Event::SaveFailed(err.to_string())),
                    },
                    Effect::Open(path) => {
                        let opened = self.open(&path);
                        if let Event::Opened { path, .. } = &opened {
                            let text = git_base(&self.root, path);
                            queue.push_back(opened.clone());
                            queue.push_back(Event::GitBase {
                                path: path.clone(),
                                text,
                            });
                        } else {
                            queue.push_back(opened);
                        }
                    }
                    Effect::ListFiles => queue.push_back(Event::Files(list_files(&self.root))),
                    Effect::ListTree => queue.push_back(Event::Tree(list_files(&self.root))),
                    Effect::Search(query) => {
                        let hits = search(&self.root, &query);
                        queue.push_back(Event::SearchResults { query, hits });
                    }
                    Effect::GitStatus => queue.push_back(self.git_status()),
                    Effect::Git(action) => {
                        let commit = matches!(action, GitAction::Commit(_));
                        queue.push_back(self.git(action));
                        if commit {
                            // The new commit is the base for gutter signs.
                            self.git_index = None;
                            queue.push_back(self.git_status());
                        }
                    }
                    Effect::OpenDiff(path) => {
                        let old = git_base(&self.root, &path).unwrap_or_default();
                        let new = fs::read_to_string(self.root.join(&path)).unwrap_or_default();
                        queue.push_back(Event::ShowDiff { path, old, new });
                    }
                    Effect::Copy(text) => {
                        if self.system_clipboard {
                            clipboard::copy(&text);
                        }
                    }
                    Effect::ReadClipboard => {
                        let text = self.system_clipboard.then(clipboard::paste).flatten();
                        queue.push_back(Event::Clipboard(text));
                    }
                    Effect::Quit => self.quit = true,
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
                    Effect::OpenSettings { workspace } => match self.settings.ensure(workspace) {
                        Ok(path) => queue.push_back(self.open(&path)),
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
        let mut events = vec![Event::Settings(editor)];
        events.extend(
            errors
                .into_iter()
                .map(|e| Event::Error(format!("Settings: {e}"))),
        );
        events
    }

    /// Reads a file to open, and starts watching it. A path that doesn't
    /// exist yet opens empty, and is created on save.
    fn open(&mut self, path: &Path) -> Event {
        let path = relative_to(&self.root, path);
        self.disk
            .insert(path.clone(), modified(&self.root.join(&path)));
        match fs::read_to_string(self.root.join(&path)) {
            Ok(text) => Event::Opened { path, text },
            Err(err) if err.kind() == io::ErrorKind::NotFound => Event::Opened {
                path,
                text: String::new(),
            },
            Err(err) => Event::Error(format!("Can't open {}: {err}", path.display())),
        }
    }

    /// The branch and changed files, with paths relative to the project.
    fn git_status(&self) -> Event {
        let output = self.git_dir.as_ref().and_then(|_| {
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
        let Some(output) = output else {
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
            let path = path.strip_prefix(&self.git_prefix).unwrap_or(path);
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
    fn git(&self, action: GitAction) -> Event {
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
                _ => self.git_status(),
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

    /// Reloads open files that changed on disk, such as ones another program
    /// edited, and refreshes git signs after a commit.
    fn check_disk(&mut self) {
        let index = self
            .git_dir
            .as_ref()
            .and_then(|dir| modified(&dir.join("index")));
        let committed = index != self.git_index;
        self.git_index = index;
        if committed {
            let status = self.git_status();
            self.dispatch(status);
        }
        let open: Vec<PathBuf> = self.editor.open_files().into_iter().flatten().collect();
        self.disk.retain(|path, _| open.contains(path));
        for path in open {
            let full = self.root.join(&path);
            let now = modified(&full);
            let changed = self.disk.get(&path).is_some_and(|seen| *seen != now);
            if changed {
                self.disk.insert(path.clone(), now);
                if let Ok(text) = fs::read_to_string(&full) {
                    self.dispatch(Event::FileChanged {
                        path: path.clone(),
                        text,
                    });
                }
            }
            if committed {
                let text = git_base(&self.root, &path);
                self.dispatch(Event::GitBase { path, text });
            }
        }
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
        text: buffer.text(),
        root: root.to_path_buf(),
        cursor: editor.cursor_utf16(),
        selection: editor
            .selection_utf16()
            .zip(editor.selection())
            .map(|((start, end), text)| Selection { start, end, text }),
    }
}

fn main() -> Result<()> {
    let argv: Vec<String> = env::args().skip(1).collect();
    if let [plugin, new, name] = &argv[..]
        && plugin == "plugin"
        && new == "new"
    {
        return template::create(name);
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
        editor: Editor::new("", None),
        host: PluginHost::new(config)?,
        root,
        panels_for: None,
        buffer_seen: None,
        saved: Vec::new(),
        system_clipboard: args.exec.is_none(),
        disk: HashMap::new(),
        git_prefix,
        git_dir,
        git_index: None,
        quit: false,
        settings: settings_files,
        scripted: args.exec.is_some(),
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
    for file in &args.files {
        let event = app.open(file);
        if let Event::Error(err) = event {
            bail!(err);
        }
        app.dispatch(event);
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
                let event = app.open(&file);
                app.dispatch(event);
            }
        }
        app.dispatch(Event::Restore(ws));
    }
    // Loads git signs for the files opened so far; later ones get theirs
    // when they open.
    app.check_disk();
    if args.exec.is_none() {
        gui::set_waker(&mut app.host);
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

    if let Some(script) = &args.exec {
        app.dispatch(Event::Resize(args.size.0, args.size.1));
        let result = run_script(&mut app, script);
        let screen = app.editor.render();
        print!("{}", screen.to_text());
        if let Some(path) = &args.html {
            fs::write(path, screen_html(&screen))
                .with_context(|| format!("writing {}", path.display()))?;
        }
        return result;
    }

    let result = gui::run(&mut app);
    let _ = fs::create_dir_all(workspace_path.parent().unwrap_or(Path::new(".")))
        .and_then(|()| fs::write(&workspace_path, app.editor.workspace().to_text()));
    result
}

/// Where the open tabs and groups of the project at `root` are kept
/// between runs.
fn workspace_file(data_dir: &Path, root: &Path) -> PathBuf {
    let name: String = root
        .display()
        .to_string()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    data_dir.join("workspaces").join(name)
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
                while !app.editor.render().to_text().contains(&text) {
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

/// The screen as a standalone HTML page, in the Dark Modern colours the
/// window shows. For screenshots and bug reports.
fn screen_html(screen: &Screen) -> String {
    let look = |cell: merlin_core::screen::Cell| theme::dark_cell(&cell).css();
    let escape = |c: char| match c {
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '&' => "&amp;".to_string(),
        c => c.to_string(),
    };
    let mut rows = String::new();
    for y in 0..screen.height {
        // Runs of text with the same look, as one span each.
        let mut run: Option<(String, String)> = None;
        let mut row = String::new();
        let flush = |run: &mut Option<(String, String)>, row: &mut String| {
            if let Some((style, text)) = run.take() {
                row.push_str(&format!("<span style=\"{style}\">{text}</span>"));
            }
        };
        for x in 0..screen.width {
            let cell = screen.cell(x, y);
            if cell.ch == WIDE_TAIL {
                continue;
            }
            if screen.cursor == Some((x, y)) {
                flush(&mut run, &mut row);
                row.push_str(&format!(
                    "<span class=\"cursor\">{}</span>",
                    escape(cell.ch)
                ));
                continue;
            }
            let style = look(cell);
            match &mut run {
                Some((current, text)) if *current == style => text.push_str(&escape(cell.ch)),
                _ => {
                    flush(&mut run, &mut row);
                    run = Some((style, escape(cell.ch)));
                }
            }
        }
        flush(&mut run, &mut row);
        rows.push_str(&row);
        rows.push('\n');
    }
    format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<title>merlin</title>\n<style>\n\
         body {{ margin: 0; background: #1f1f1f; }}\n\
         pre {{ margin: 0; padding: 14px 16px; display: inline-block; background: #1f1f1f; color: #cccccc;\n\
         font: 15px/1.35 'JetBrains Mono', 'JetBrainsMono Nerd Font', 'DejaVu Sans Mono', monospace; }}\n\
         .cursor {{ background: #aeafad; color: #1f1f1f; }}\n</style>\n<pre>{rows}</pre>\n"
    )
}
