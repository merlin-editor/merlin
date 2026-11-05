//! The side bar's built-in views, as in VS Code: the Explorer (the project's
//! files), Search across them, and Source Control (what changed since the
//! last commit). They're panels like plugins', built by the editor.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{Editor, Effect, Focus, GitAction, Plugin, Side};
use crate::icons::{Icon, file_icon, icon};
use crate::key::{Key, KeyCode};
use crate::lang::Severity;
use crate::panel::{Button, Entry, Node, Panel, Target, Tone};
use crate::style::Hue;

pub(crate) const EXPLORER: &str = "merlin.explorer";
pub(crate) const SEARCH: &str = "merlin.search";
pub(crate) const CHANGES: &str = "merlin.changes";
pub(crate) const OUTLINE: &str = "merlin.outline";
pub(crate) const PROBLEMS: &str = "merlin.problems";
pub(crate) const OUTPUT: &str = "merlin.output";
pub(crate) const PLUGINS: &str = "merlin.plugins";

/// Most messages the Output panel keeps.
const LOG_LINES: usize = 1000;

/// Most search results shown.
const SHOWN_HITS: usize = 500;

/// A line that matched a project search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub path: String,
    /// From 0, with the column in UTF-16 units as LSP counts them.
    pub line: u32,
    pub column: u32,
    pub text: String,
}

/// The project's files, and which folders are open.
#[derive(Default)]
pub(crate) struct Tree {
    pub files: Vec<String>,
    pub expanded: HashSet<String>,
    /// What the panel was built for, to rebuild it when that changes.
    pub built_for: Option<(u64, u64)>,
    /// A file to focus once the list arrives.
    pub reveal: Option<String>,
    /// A name being typed in the tree.
    pub edit: Option<super::files::Naming>,
}

#[derive(Default)]
struct Folder {
    folders: BTreeMap<String, Folder>,
    files: Vec<String>,
}

#[derive(Default)]
pub(crate) struct Search {
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub searching: bool,
}

/// What git says about the project.
#[derive(Default)]
pub(crate) struct Git {
    pub branch: Option<String>,
    /// Changed files and their status letter, as `git status` gives it.
    pub files: Vec<(String, char)>,
    /// The same files split as Source Control shows them: changes in the
    /// index, and changes not staged.
    pub staged: Vec<(String, char)>,
    pub changed: Vec<(String, char)>,
    /// Bumped on every update, so views built from it know to rebuild.
    pub version: u64,
}

impl Git {
    fn status(&self, path: &str) -> Option<char> {
        self.files.iter().find(|(p, _)| p == path).map(|(_, s)| *s)
    }
}

/// The letter VS Code shows for a git status: U for files git doesn't track.
fn letter(status: char) -> char {
    if status == '?' { 'U' } else { status }
}

fn tone(status: char) -> Tone {
    match status {
        'A' | '?' => Tone::Success,
        'D' => Tone::Error,
        _ => Tone::Warning,
    }
}

/// Errors and warnings in a file, for the Explorer.
type Counts = HashMap<String, (usize, usize)>;

/// What the Explorer shows next to names: counts and git, by file.
struct Marks<'a> {
    git: &'a Git,
    counts: Counts,
    current: Option<&'a str>,
    nerd: bool,
}

impl Tree {
    /// The rows shown, as entries.
    fn rows(&self, marks: &Marks) -> Vec<Entry> {
        let mut root = Folder::default();
        for file in &self.files {
            let mut folder = &mut root;
            // A folder, which may have nothing in it yet.
            if let Some(dir) = file.strip_suffix('/') {
                for part in dir.split('/') {
                    folder = folder.folders.entry(part.to_string()).or_default();
                }
                continue;
            }
            let mut parts: Vec<&str> = file.split('/').collect();
            let name = parts.pop().unwrap_or_default();
            for part in parts {
                folder = folder.folders.entry(part.to_string()).or_default();
            }
            folder.files.push(name.to_string());
        }
        let mut rows = Vec::new();
        self.walk(&root, "", 0, marks, &mut rows);
        rows
    }

    fn walk(
        &self,
        folder: &Folder,
        prefix: &str,
        depth: usize,
        marks: &Marks,
        rows: &mut Vec<Entry>,
    ) {
        for (name, sub) in &folder.folders {
            let path = format!("{prefix}{name}");
            let open = self.expanded.contains(&path);
            let inside = format!("{path}/");
            let changed = marks.git.files.iter().any(|(p, _)| p.starts_with(&inside));
            let broken = marks
                .counts
                .iter()
                .any(|(p, (e, _))| *e > 0 && p.starts_with(&inside));
            let tone = match (broken, changed) {
                (true, _) => Tone::Error,
                (_, true) => Tone::Warning,
                _ => Tone::Normal,
            };
            rows.push(Entry {
                id: format!("d:{path}"),
                label: name.clone(),
                tone,
                depth,
                twisty: Some(icon(
                    if open { Icon::Open } else { Icon::Closed },
                    marks.nerd,
                )),
                badge: if tone == Tone::Normal {
                    String::new()
                } else {
                    "●".into()
                },
                badge_tone: tone,
                ..Default::default()
            });
            if open {
                self.walk(sub, &inside, depth + 1, marks, rows);
            }
        }
        for name in &folder.files {
            let path = format!("{prefix}{name}");
            let status = marks.git.status(&path);
            let (errors, warnings) = marks.counts.get(&path).copied().unwrap_or_default();
            let tone = match status {
                _ if errors > 0 => Tone::Error,
                Some(status) => tone(status),
                None if warnings > 0 => Tone::Warning,
                None if Some(path.as_str()) == marks.current => Tone::Strong,
                None => Tone::Normal,
            };
            let mut badge = Vec::new();
            if errors + warnings > 0 {
                badge.push((errors + warnings).to_string());
            }
            if let Some(status) = status {
                badge.push(letter(status).to_string());
            }
            rows.push(Entry {
                id: format!("f:{path}"),
                label: name.clone(),
                tone,
                depth,
                twisty: Some(' '),
                icon: Some(file_icon(name, marks.nerd)),
                badge: badge.join(", "),
                badge_tone: tone,
                ..Default::default()
            });
        }
    }

    /// Opens the folders a file is in.
    fn open_folders(&mut self, file: &str) {
        let mut path = String::new();
        let parts: Vec<&str> = file.split('/').collect();
        for part in &parts[..parts.len().saturating_sub(1)] {
            path.push_str(part);
            self.expanded.insert(path.clone());
            path.push('/');
        }
    }
}

impl Editor {
    /// Adds the built-in views to the left side bar.
    pub(crate) fn add_views(&mut self) {
        for (name, title, side) in [
            (EXPLORER, "Explorer", Side::Left),
            (SEARCH, "Search", Side::Left),
            (CHANGES, "Source Control", Side::Left),
            (OUTLINE, "Outline", Side::Left),
            (PLUGINS, "Plugins", Side::Left),
            (PROBLEMS, "Problems", Side::Bottom),
            (OUTPUT, "Output", Side::Bottom),
            (super::terminal::TERMINAL, "Terminal", Side::Bottom),
        ] {
            self.plugins.push(Plugin {
                name: name.into(),
                commands: Vec::new(),
                keys: Vec::new(),
                panel: Some(Panel {
                    title: title.into(),
                    nodes: Vec::new(),
                }),
                error: None,
                side,
            });
        }
    }

    fn set_view(&mut self, name: &str, nodes: Vec<Node>) {
        if let Some(plugin) = self.plugins.iter_mut().find(|p| p.name == name)
            && let Some(panel) = &mut plugin.panel
        {
            panel.nodes = nodes;
        }
    }

    /// Shows a view in the side bar and focuses it. Showing the focused
    /// view again goes back to the text.
    pub(crate) fn show_view(&mut self, name: &str, fx: &mut Vec<Effect>) {
        let showing = self.bar(Side::Left).visible
            && self.focus == Focus::SideBar
            && self.active_panel_name(Side::Left).as_deref() == Some(name);
        if showing {
            self.focus = Focus::Editor;
            return;
        }
        self.bar_mut(Side::Left).active = Some(name.into());
        self.refresh_view(name, fx);
        self.build_views();
        self.focus_panel(Side::Left);
        if name == EXPLORER {
            self.reveal_current();
        }
    }

    /// Shows or hides the side bar, keeping focus where it is.
    pub(crate) fn toggle_side_bar(&mut self, fx: &mut Vec<Effect>) {
        let bar = self.bar_mut(Side::Left);
        bar.visible = !bar.visible;
        if bar.visible {
            let name = self
                .active_panel_name(Side::Left)
                .unwrap_or(EXPLORER.into());
            self.refresh_view(&name, fx);
            if name == EXPLORER {
                self.reveal_current();
            }
        } else if self.focus == Focus::SideBar {
            self.focus = Focus::Editor;
        }
    }

    /// Asks for what a view shows.
    pub(crate) fn refresh_view(&mut self, name: &str, fx: &mut Vec<Effect>) {
        match name {
            EXPLORER => {
                fx.push(Effect::ListTree);
                fx.push(Effect::GitStatus);
                self.tree.get_or_insert_with(Tree::default);
            }
            CHANGES => fx.push(Effect::GitStatus),
            _ => {}
        }
    }

    /// Shows the Explorer, whatever the side bar showed, with the open
    /// file focused in it.
    pub(crate) fn reveal_in_explorer(&mut self, fx: &mut Vec<Effect>) {
        let bar = self.bar_mut(Side::Left);
        bar.visible = true;
        bar.active = Some(EXPLORER.into());
        self.refresh_view(EXPLORER, fx);
        self.build_views();
        self.focus_panel(Side::Left);
        self.reveal_current();
    }

    /// Focuses the open file in the Explorer, opening its folders.
    fn reveal_current(&mut self) {
        let current = self
            .doc()
            .buffer
            .path
            .as_ref()
            .map(|p| p.display().to_string());
        if let (Some(tree), Some(file)) = (&mut self.tree, current) {
            tree.open_folders(&file);
            tree.reveal = Some(file);
            tree.built_for = None;
        }
        self.build_views();
    }

    /// Rebuilds the views whose content changed.
    pub(crate) fn build_views(&mut self) {
        self.build_explorer();
        let search = self.search_nodes();
        self.set_view(SEARCH, search);
        let changes = self.changes_nodes();
        self.set_view(CHANGES, changes);
        self.build_outline();
        let problems = self.problems_nodes();
        self.set_view(PROBLEMS, problems);
        let output = self.output_nodes();
        self.set_view(OUTPUT, output);
        let plugins = self.plugins_nodes();
        self.set_view(PLUGINS, plugins);
    }

    /// The Plugins view: each plugin, how it runs, and what it adds.
    fn plugins_nodes(&self) -> Vec<Node> {
        use crate::icons::{Icon, icon};
        let mut nodes = vec![Node::Heading(format!(
            "Installed · {}",
            self.plugin_list.len()
        ))];
        for p in &self.plugin_list {
            nodes.push(Node::Entry(crate::panel::Entry {
                id: format!("plugin:{}", p.name),
                label: p.title.clone(),
                detail: p.what.clone(),
                icon: Some((icon(Icon::Plugin, self.nerd), crate::style::Hue::Blue)),
                badge: p.tag.clone(),
                badge_tone: p.tone,
                ..Default::default()
            }));
        }
        nodes
    }

    /// The Outline view: the open file's symbols, nested, while it shows.
    fn build_outline(&mut self) {
        let showing = self.bar(Side::Left).visible
            && self.active_panel_name(Side::Left).as_deref() == Some(OUTLINE);
        let doc = self.doc();
        let key = (doc.id, doc.buffer.version());
        if !showing || self.outline_for == Some(key) {
            return;
        }
        let symbols = crate::symbols::symbols(doc);
        self.outline_for = Some(key);
        let nodes: Vec<Node> = symbols
            .into_iter()
            .map(|s| {
                let (glyph, hue) = symbol_icon(&s.kind);
                Node::Entry(Entry {
                    id: format!("o:{}", s.line),
                    label: s.name,
                    depth: s.depth,
                    icon: Some((glyph, hue)),
                    detail: if s.kind == "#" || s.kind == "impl" {
                        String::new()
                    } else {
                        s.kind
                    },
                    ..Default::default()
                })
            })
            .collect();
        let nodes = if nodes.is_empty() {
            vec![Node::Styled(
                "No symbols found in this file.".into(),
                Tone::Muted,
            )]
        } else {
            nodes
        };
        self.set_view(OUTLINE, nodes);
    }

    fn build_explorer(&mut self) {
        let key = (
            self.doc().id,
            self.git.version ^ (self.diagnostics_version << 32),
        );
        let current = self
            .doc()
            .buffer
            .path
            .as_ref()
            .map(|p| p.display().to_string());
        let Some(tree) = &mut self.tree else { return };
        if tree.built_for == Some(key) {
            return;
        }
        tree.built_for = Some(key);
        let nodes = if tree.files.is_empty() {
            vec![Node::Styled("Listing files…".into(), Tone::Muted)]
        } else {
            let marks = Marks {
                git: &self.git,
                counts: problem_counts(&self.diagnostics, &tree.files),
                current: current.as_deref(),
                nerd: self.nerd,
            };
            let mut nodes: Vec<Node> = tree.rows(&marks).into_iter().map(Node::Entry).collect();
            // A name being typed goes where the file will be: under its
            // folder, or in place of what's renamed.
            if let Some(edit) = &tree.edit {
                use super::files::{NAME_INPUT, Naming};
                let row = |id: &str| {
                    nodes
                        .iter()
                        .position(|n| matches!(n, Node::Entry(e) if e.id == id))
                };
                let (at, replace, placeholder) = match edit {
                    Naming::NewFile { dir } | Naming::NewFolder { dir } if dir.is_empty() => {
                        (Some(0), false, "Name")
                    }
                    Naming::NewFile { dir } | Naming::NewFolder { dir } => {
                        (row(&format!("d:{dir}")).map(|i| i + 1), false, "Name")
                    }
                    Naming::Rename { from } => (
                        row(&format!("f:{from}")).or_else(|| row(&format!("d:{from}"))),
                        true,
                        "New name",
                    ),
                };
                let input = Node::Input {
                    id: NAME_INPUT.into(),
                    placeholder: placeholder.into(),
                    secret: false,
                };
                match at {
                    Some(i) if replace => nodes[i] = input,
                    Some(i) => nodes.insert(i, input),
                    None => nodes.insert(0, input),
                }
            }
            nodes
        };
        let reveal = (!tree.files.is_empty())
            .then(|| tree.reveal.take())
            .flatten();
        self.set_view(EXPLORER, nodes);
        if let Some(file) = reveal {
            self.bar_mut(Side::Left).target = Some(Target::Button(format!("f:{file}")));
        }
    }

    fn search_nodes(&self) -> Vec<Node> {
        let search = &self.search;
        let mut nodes = vec![Node::Input {
            id: "query".into(),
            placeholder: "Search".into(),
            secret: false,
        }];
        let files = {
            let mut seen: Vec<&str> = search.hits.iter().map(|h| h.path.as_str()).collect();
            seen.dedup();
            seen.len()
        };
        let summary = match (search.searching, search.hits.len()) {
            (true, _) => "Searching…".to_string(),
            _ if search.query.is_empty() => "Type, then Enter to search the project.".into(),
            (_, 0) => "No results".into(),
            (_, 1) => "1 result".into(),
            (_, n) if files == 1 => format!("{n} results in 1 file"),
            (_, n) => format!("{n} results in {files} files"),
        };
        nodes.push(Node::Styled(summary, Tone::Muted));
        let mut last: Option<&str> = None;
        for (i, hit) in search.hits.iter().enumerate().take(SHOWN_HITS) {
            if last != Some(&hit.path) {
                let count = search.hits.iter().filter(|h| h.path == hit.path).count();
                nodes.push(Node::Entry(self.file_entry(
                    format!("f:{}", hit.path),
                    Path::new(&hit.path),
                    count.to_string(),
                )));
                last = Some(&hit.path);
            }
            nodes.push(Node::Entry(Entry {
                id: format!("h:{i}"),
                label: hit.text.trim().to_string(),
                depth: 1,
                detail: format!("{}", hit.line + 1),
                ..Default::default()
            }));
        }
        nodes
    }

    /// A file's row as a header: arrow, icon, name, its folder dimmed, and
    /// a count.
    fn file_entry(&self, id: String, path: &Path, count: String) -> Entry {
        let name = path
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().to_string());
        Entry {
            id,
            icon: Some(file_icon(&name, self.nerd)),
            twisty: Some(icon(Icon::Open, self.nerd)),
            label: name,
            detail: path
                .parent()
                .map_or(String::new(), |p| p.display().to_string()),
            badge: format!(" {count} "),
            badge_tone: Tone::Badge,
            ..Default::default()
        }
    }

    fn changes_nodes(&self) -> Vec<Node> {
        let git = &self.git;
        let Some(branch) = &git.branch else {
            return vec![Node::Styled("Not in a git repository".into(), Tone::Muted)];
        };
        let branch_icon = icon(Icon::Branch, self.nerd);
        let mut nodes = vec![
            Node::Input {
                id: "message".into(),
                placeholder: format!("Message (Enter to commit on {branch})"),
                secret: false,
            },
            Node::Buttons(vec![
                Button {
                    id: "g:commit".into(),
                    label: "✓ Commit".into(),
                },
                if git.changed.is_empty() && !git.staged.is_empty() {
                    Button {
                        id: "g:unstage".into(),
                        label: "− Unstage All".into(),
                    }
                } else {
                    Button {
                        id: "g:stage".into(),
                        label: "+ Stage All".into(),
                    }
                },
            ]),
            Node::Styled(format!("{branch_icon} {branch}"), Tone::Muted),
        ];
        if git.files.is_empty() {
            nodes.push(Node::Styled("No changes".into(), Tone::Muted));
        }
        // Staged changes first, with − to unstage; then the rest, with + to
        // stage, as VS Code lists them.
        for (title, list, action, prefix) in [
            ("Staged Changes", &git.staged, '−', "u:"),
            ("Changes", &git.changed, '+', "a:"),
        ] {
            if list.is_empty() {
                continue;
            }
            nodes.push(Node::Heading(format!("{title}  {}", list.len())));
            for (path, status) in list {
                let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
                nodes.push(Node::Entry(Entry {
                    id: format!("c:{path}"),
                    label: name.to_string(),
                    tone: tone(*status),
                    icon: Some(file_icon(name, self.nerd)),
                    detail: dir.to_string(),
                    badge: letter(*status).to_string(),
                    badge_tone: tone(*status),
                    action: Some((action, format!("{prefix}{path}"))),
                    ..Default::default()
                }));
            }
        }
        nodes
    }

    /// Every error, warning and note, by file, for the Problems panel.
    fn problems_nodes(&mut self) -> Vec<Node> {
        let mut by_file: BTreeMap<&PathBuf, Vec<&crate::lang::Diagnostic>> = BTreeMap::new();
        for ((_, path), list) in &self.diagnostics {
            by_file
                .entry(path)
                .or_default()
                .extend(list.iter().filter(|d| d.severity != Severity::Hint));
        }
        by_file.retain(|_, list| !list.is_empty());
        let mut problems = Vec::new();
        let mut nodes = Vec::new();
        for (path, mut list) in by_file {
            list.sort_by_key(|d| (d.severity, d.start));
            nodes.push(Node::Entry(self.file_entry(
                format!("f:{}", path.display()),
                path,
                list.len().to_string(),
            )));
            for d in list {
                let (glyph, hue) = match d.severity {
                    Severity::Error => (Icon::Error, Hue::Problem),
                    Severity::Warning => (Icon::Warning, Hue::Warned),
                    _ => (Icon::Info, Hue::Blue),
                };
                nodes.push(Node::Entry(Entry {
                    id: format!("p:{}", problems.len()),
                    label: d.message.lines().next().unwrap_or_default().to_string(),
                    depth: 1,
                    icon: Some((icon(glyph, self.nerd), hue)),
                    detail: format!("[Ln {}, Col {}]", d.start.0 + 1, d.start.1 + 1),
                    ..Default::default()
                }));
                problems.push((path.clone(), d.start.0, d.start.1));
            }
        }
        if nodes.is_empty() {
            nodes.push(Node::Styled(
                "No problems have been detected in the workspace.".into(),
                Tone::Muted,
            ));
        }
        self.problems = problems;
        nodes
    }

    /// Messages so far, newest last.
    fn output_nodes(&self) -> Vec<Node> {
        let start = self.log.len().saturating_sub(LOG_LINES);
        self.log[start..]
            .iter()
            .map(|(error, text)| {
                let (level, tone) = if *error {
                    ("error", Tone::Error)
                } else {
                    ("info", Tone::Normal)
                };
                Node::Styled(format!("[{level}] {text}"), tone)
            })
            .collect()
    }

    /// Shows or hides the panel under the editor.
    pub(crate) fn toggle_bottom(&mut self) {
        let bar = self.bar_mut(Side::Bottom);
        bar.visible = !bar.visible;
        if !bar.visible && self.focus == Focus::Bottom {
            self.focus = Focus::Editor;
        }
    }

    /// Shows a view in the panel and focuses it.
    pub(crate) fn show_bottom(&mut self, name: &str) {
        self.bar_mut(Side::Bottom).active = Some(name.into());
        self.build_views();
        self.focus_panel(Side::Bottom);
    }

    pub(crate) fn view_click(&mut self, id: &str, fx: &mut Vec<Effect>) {
        if let Some(name) = id.strip_prefix("plugin:") {
            let title = self
                .plugin_list
                .iter()
                .find(|p| p.name == name)
                .map_or_else(|| name.to_string(), |p| p.title.clone());
            return self.open_page(name, &title);
        }
        if let Some(folder) = id.strip_prefix("d:") {
            if let Some(tree) = &mut self.tree {
                if !tree.expanded.remove(folder) {
                    tree.expanded.insert(folder.to_string());
                }
                tree.built_for = None;
            }
            self.build_views();
        } else if let Some(file) = id.strip_prefix("f:") {
            // One click previews the file, as in VS Code; a double click or
            // Enter keeps it open.
            if self.clicks == 1 && self.preview_tabs {
                self.open_preview(PathBuf::from(file), fx);
            } else {
                self.open(PathBuf::from(file), fx);
                if self.doc().buffer.path.as_deref() == Some(Path::new(file)) {
                    self.keep_open();
                }
            }
        } else if let Some(i) = id.strip_prefix("h:").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(hit) = self.search.hits.get(i).cloned() {
                self.open_at(PathBuf::from(hit.path), hit.line, hit.column, fx);
            }
        } else if let Some(i) = id.strip_prefix("p:").and_then(|i| i.parse::<usize>().ok()) {
            if let Some((path, line, column)) = self.problems.get(i).cloned() {
                self.open_at(path, line, column, fx);
            }
        } else if let Some(line) = id.strip_prefix("o:").and_then(|l| l.parse().ok()) {
            self.go_to_block(line);
        } else if let Some(path) = id.strip_prefix("c:") {
            fx.push(Effect::OpenDiff(PathBuf::from(path)));
        } else if let Some(path) = id.strip_prefix("a:") {
            fx.push(Effect::Git(GitAction::Stage(path.into())));
        } else if let Some(path) = id.strip_prefix("u:") {
            fx.push(Effect::Git(GitAction::Unstage(path.into())));
        } else if id == "g:stage" {
            fx.push(Effect::Git(GitAction::StageAll));
        } else if id == "g:unstage" {
            fx.push(Effect::Git(GitAction::UnstageAll));
        } else if id == "g:commit" {
            let key = (CHANGES.to_string(), "message".to_string());
            let message = self.inputs.get(&key).cloned().unwrap_or_default();
            self.commit(&message, fx);
        }
    }

    /// Commits what's staged with `message`, as Source Control's Commit
    /// does. With nothing staged, it says so instead of committing nothing.
    fn commit(&mut self, message: &str, fx: &mut Vec<Effect>) {
        if message.trim().is_empty() {
            return self.error("Type a commit message first".into());
        }
        if self.git.staged.is_empty() {
            return self.error("Nothing staged: stage changes with + or Stage All".into());
        }
        self.inputs
            .remove(&(CHANGES.to_string(), "message".to_string()));
        fx.push(Effect::Git(GitAction::Commit(message.trim().to_string())));
    }

    /// Keys a view handles itself: in the Explorer, Left closes a folder or
    /// goes to its parent, and Right opens it.
    pub(crate) fn explorer_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        if self.active_panel_name(Side::Left).as_deref() != Some(EXPLORER) {
            return false;
        }
        // Typing a name: Enter does it, Esc doesn't.
        if self.tree.as_ref().is_some_and(|t| t.edit.is_some()) {
            match key.code {
                KeyCode::Esc => {
                    self.cancel_naming();
                    self.focus = Focus::Editor;
                    return true;
                }
                KeyCode::Enter => {
                    let text = self
                        .inputs
                        .get(&(EXPLORER.to_string(), super::files::NAME_INPUT.to_string()))
                        .cloned()
                        .unwrap_or_default();
                    self.name_typed(&text, fx);
                    return true;
                }
                _ => return false,
            }
        }
        let index = self.selected_index(Side::Left);
        let Some(Target::Button(id)) = self.panel_targets(Side::Left).into_iter().nth(index) else {
            return false;
        };
        let Some(tree) = &self.tree else { return false };
        let path = id[2..].to_string();
        let folder = id.starts_with("d:");
        let open = tree.expanded.contains(&path);
        match key.code {
            KeyCode::Right if folder && !open => self.view_click(&id, fx),
            KeyCode::Left if folder && open => self.view_click(&id, fx),
            KeyCode::Left => {
                if let Some((parent, _)) = path.rsplit_once('/') {
                    self.bar_mut(Side::Left).target = Some(Target::Button(format!("d:{parent}")));
                }
            }
            KeyCode::Right => {}
            _ => return false,
        }
        true
    }

    pub(crate) fn view_submit(&mut self, view: &str, text: &str, fx: &mut Vec<Effect>) {
        if view == CHANGES {
            return self.commit(text, fx);
        }
        if view == SEARCH {
            self.search.query = text.to_string();
            self.search.searching = true;
            self.search.hits.clear();
            fx.push(Effect::Search(text.to_string()));
        }
    }
}

/// Errors and warnings by project file, matching diagnostics' paths to the
/// Explorer's whether they're relative or absolute.
fn problem_counts(
    diagnostics: &HashMap<(String, PathBuf), Vec<crate::lang::Diagnostic>>,
    files: &[String],
) -> Counts {
    let mut counts = Counts::new();
    for ((_, path), list) in diagnostics {
        let errors = list
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warnings = list
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        if errors + warnings == 0 {
            continue;
        }
        let key = path.display().to_string();
        let file = if files.contains(&key) {
            Some(key)
        } else {
            files.iter().find(|f| path.ends_with(f.as_str())).cloned()
        };
        if let Some(file) = file {
            let entry = counts.entry(file).or_default();
            entry.0 += errors;
            entry.1 += warnings;
        }
    }
    counts
}

/// How a symbol of a kind shows in the Outline: a glyph and its colour, as
/// VS Code colours functions purple and types orange.
pub(crate) fn symbol_icon(kind: &str) -> (char, Hue) {
    match kind {
        "fn" | "def" | "function" | "func" | "macro_rules!" | "method" | "constructor"
        | "operator" => ('ƒ', Hue::Purple),
        "struct" | "class" | "enum" | "union" | "type" | "type-parameter" => ('◇', Hue::Orange),
        "trait" | "interface" => ('◈', Hue::Blue),
        "field" | "property" | "variable" | "constant" | "enum-member" => ('▪', Hue::Blue),
        "#" => ('#', Hue::Grey),
        _ => ('{', Hue::Blue),
    }
}
