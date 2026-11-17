//! Files on disk from the Explorer, as VS Code has it: a new file or
//! folder, rename, delete to the trash, copy a path, and the right-click
//! menu that offers them. Names are typed into the tree itself, where the
//! file will be, and deleting asks first; the host does the work and says
//! what changed. Opening and saving elsewhere use the system's file
//! dialogs.

use std::path::PathBuf;

use super::context::ContextMenu;
use super::dialog::Ask;
use super::views::EXPLORER;
use super::{Editor, Effect, Focus, Side};
use crate::panel::Target;

/// The Explorer's right-click menu, on a file or folder.
pub(crate) const EXPLORER_ITEMS: &[(&str, &str)] = &[
    ("explorer.new_file", "New File…"),
    ("explorer.new_folder", "New Folder…"),
    ("-", ""),
    ("explorer.open_side", "Open to the Side"),
    ("-", ""),
    ("explorer.find_in_folder", "Find in Folder…"),
    ("-", ""),
    ("explorer.exclude", "Exclude from Project"),
    ("explorer.exclude_search", "Exclude from Search Only"),
    ("explorer.include", "Include in Project"),
    ("explorer.gitignore", "Add to .gitignore"),
    ("explorer.open_gitignore", "Open .gitignore"),
    ("explorer.show_hidden", "Show Hidden Files"),
    ("-", ""),
    ("explorer.select_compare", "Select for Compare"),
    ("explorer.compare_with", "Compare with Selected"),
    ("-", ""),
    ("explorer.copy_path", "Copy Path"),
    ("explorer.copy_relative", "Copy Relative Path"),
    ("-", ""),
    ("explorer.rename", "Rename…"),
    ("explorer.delete", "Delete"),
];

/// What a system file dialog is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilePick {
    /// A file to open.
    Open,
    /// A folder to open in a new window.
    OpenFolder,
    /// Where to save document `doc`.
    SaveAs { doc: u64 },
}

/// A name being typed in the Explorer, and what it's for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Naming {
    NewFile { dir: String },
    NewFolder { dir: String },
    Rename { from: String },
}

/// The input the name is typed into.
pub(crate) const NAME_INPUT: &str = "name";

/// A file operation the host carries out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOp {
    CreateFile(PathBuf),
    CreateFolder(PathBuf),
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    /// To the system's trash, so it can be got back.
    Trash(PathBuf),
}

impl Editor {
    /// The Explorer row a file command is about: the one right-clicked,
    /// else the one selected. `d:` and `f:` ids, as the Explorer's rows.
    fn explorer_row(&self) -> Option<String> {
        if let Some(row) = &self.explorer_row {
            return Some(row.clone());
        }
        if self.active_panel_name(Side::Left).as_deref() != Some(EXPLORER) {
            return None;
        }
        let index = self.selected_index(Side::Left);
        match self.panel_targets(Side::Left).into_iter().nth(index) {
            Some(Target::Button(id)) if id.starts_with("d:") || id.starts_with("f:") => Some(id),
            _ => None,
        }
    }

    /// The folder new things go in: the row's, or the one a file is in.
    fn explorer_folder(&self) -> String {
        match self.explorer_row() {
            Some(id) if id.starts_with("d:") => id[2..].to_string(),
            Some(id) => id[2..]
                .rsplit_once('/')
                .map_or(String::new(), |(dir, _)| dir.to_string()),
            None => String::new(),
        }
    }

    /// The right-click menu on an Explorer row, or on the panel's empty
    /// space with `None`.
    pub(crate) fn open_explorer_menu(&mut self, row: Option<String>) {
        self.popup = None;
        let on_row = row.is_some();
        let is_file = row.as_deref().is_some_and(|r| r.starts_with("f:"));
        let is_folder = row.as_deref().is_some_and(|r| r.starts_with("d:"));
        // Why it's dimmed, if it is: what it can be put back with.
        let why = row.as_deref().and_then(|r| {
            let rel = if is_folder {
                format!("{}/", &r[2..])
            } else {
                r[2..].to_string()
            };
            self.tree.as_ref()?.why_left_out(&rel)
        });
        use crate::exclude::Why;
        let ruled = matches!(why, Some(Why::Excluded | Why::SearchOnly));
        self.explorer_row = row;
        let items = EXPLORER_ITEMS
            .iter()
            .map(|(id, _)| *id)
            .filter(|&id| match id {
                "explorer.open_side" | "explorer.select_compare" => is_file,
                "explorer.compare_with" => is_file && self.compare_selected.is_some(),
                "explorer.copy_path"
                | "explorer.copy_relative"
                | "explorer.rename"
                | "explorer.delete" => on_row,
                "explorer.find_in_folder" => is_folder && why.is_none(),
                "explorer.exclude" | "explorer.exclude_search" | "explorer.gitignore" => {
                    on_row && why.is_none()
                }
                "explorer.include" => ruled,
                "explorer.open_gitignore" => why == Some(Why::GitIgnore),
                "explorer.show_hidden" => why == Some(Why::Hidden),
                _ => true,
            })
            .collect::<Vec<_>>();
        let mut items = items;
        items.dedup_by(|a, b| *a == "-" && *b == "-");
        while items.last() == Some(&"-") {
            items.pop();
        }
        self.context = Some(ContextMenu {
            items,
            selected: None,
            title: None,
        });
        let row = self
            .explorer_row
            .as_deref()
            .and_then(|r| r.get(2..))
            .map(str::to_string);
        self.add_plugin_items(super::MenuPlace::Explorer, row);
        self.focus = Focus::Menu;
    }

    /// Starts typing a name in the Explorer, at the folder it goes in or
    /// in place of what's renamed, starting with `text`.
    fn start_naming(&mut self, naming: Naming, text: String) {
        let bar = self.bar_mut(Side::Left);
        bar.visible = true;
        bar.active = Some(EXPLORER.into());
        let Some(tree) = &mut self.tree else { return };
        if let Naming::NewFile { dir } | Naming::NewFolder { dir } = &naming
            && !dir.is_empty()
        {
            tree.expanded.insert(dir.clone());
        }
        tree.edit = Some(naming);
        tree.built_for = None;
        self.inputs
            .insert((EXPLORER.into(), NAME_INPUT.into()), text);
        self.build_views();
        self.bar_mut(Side::Left).target = Some(Target::Input(NAME_INPUT.into()));
        self.focus = Focus::SideBar;
    }

    /// Stops typing a name, without doing anything.
    pub(crate) fn cancel_naming(&mut self) {
        if let Some(tree) = &mut self.tree
            && tree.edit.take().is_some()
        {
            tree.built_for = None;
            self.inputs.remove(&(EXPLORER.into(), NAME_INPUT.into()));
        }
    }

    /// The name typed: does what it was for.
    pub(crate) fn name_typed(&mut self, name: &str, fx: &mut Vec<Effect>) {
        let Some(naming) = self.tree.as_mut().and_then(|t| t.edit.take()) else {
            return;
        };
        if let Some(tree) = &mut self.tree {
            tree.built_for = None;
        }
        self.inputs.remove(&(EXPLORER.into(), NAME_INPUT.into()));
        self.named(naming, name, fx);
        self.focus = Focus::Editor;
    }

    /// Runs an `explorer.` command; false if it isn't one.
    pub(crate) fn explorer_command(&mut self, id: &str, fx: &mut Vec<Effect>) -> bool {
        let row = self.explorer_row();
        let path = row.as_deref().map(|r| r[2..].to_string());
        match id {
            "explorer.new_file" => {
                let dir = self.explorer_folder();
                self.start_naming(Naming::NewFile { dir }, String::new());
            }
            "explorer.new_folder" => {
                let dir = self.explorer_folder();
                self.start_naming(Naming::NewFolder { dir }, String::new());
            }
            "explorer.rename" => {
                if let Some(from) = path {
                    let name = from.rsplit('/').next().unwrap_or(&from).to_string();
                    self.start_naming(Naming::Rename { from }, name);
                }
            }
            "explorer.delete" => {
                if let Some(path) = path {
                    self.ask(Ask::Trash(PathBuf::from(path)));
                }
            }
            "explorer.select_compare" => {
                if let Some(path) = path {
                    self.select_for_compare(PathBuf::from(path));
                }
            }
            "explorer.compare_with" => {
                if let Some(path) = path {
                    self.compare_with_selected(PathBuf::from(path), fx);
                }
            }
            "explorer.copy_path" | "explorer.copy_relative" => {
                if let Some(path) = path {
                    let text = if id == "explorer.copy_path" {
                        self.root.join(&path).display().to_string()
                    } else {
                        path
                    };
                    self.clipboard = text.clone();
                    fx.push(Effect::Copy(text));
                }
            }
            "explorer.find_in_folder" => {
                if let Some(path) = path {
                    let key = (super::views::SEARCH.to_string(), "include".to_string());
                    self.inputs.insert(key, format!("{path}/"));
                    self.show_view(super::views::SEARCH, fx);
                    self.bar_mut(Side::Left).target = Some(Target::Input("query".into()));
                }
            }
            "explorer.exclude" | "explorer.exclude_search" => {
                if let Some(path) = path {
                    let folder = row.as_deref().is_some_and(|r| r.starts_with("d:"));
                    let pattern = crate::exclude::pattern_for(&path, folder);
                    let search = id == "explorer.exclude_search";
                    self.exclude(pattern, search, fx);
                }
            }
            "explorer.include" => {
                if let Some(path) = path {
                    let folder = row.as_deref().is_some_and(|r| r.starts_with("d:"));
                    let rel = crate::exclude::pattern_for(&path, folder);
                    let rules = self.exclusions.rules_for(&rel);
                    if !rules.is_empty() {
                        fx.push(Effect::Include(rules));
                    }
                }
            }
            "explorer.gitignore" => {
                if let Some(path) = path {
                    let folder = row.as_deref().is_some_and(|r| r.starts_with("d:"));
                    let entry = crate::exclude::pattern_for(&path, folder);
                    fx.push(Effect::Git(super::GitAction::Ignore(entry)));
                }
            }
            "explorer.undo_exclude" => self.undo_exclude(fx),
            "explorer.toggle_excluded" => self.toggle_left_out(fx),
            "explorer.open_gitignore" => self.open(PathBuf::from(".gitignore"), fx),
            "explorer.show_hidden" => {
                self.open_page(super::SETTINGS_PAGE, "Settings");
                fx.push(Effect::OpenSettingsFiles);
            }
            "explorer.open_side" => {
                if let Some(path) = path {
                    self.open(PathBuf::from(&path), fx);
                    self.run("view.split", fx);
                }
            }
            _ => return false,
        }
        self.explorer_row = None;
        true
    }

    /// Adds a pattern to the workspace's exclusions, with a notice that
    /// can undo it.
    fn exclude(&mut self, pattern: String, search: bool, fx: &mut Vec<Effect>) {
        fx.push(Effect::Exclude {
            pattern: pattern.clone(),
            search,
        });
        let what = if search {
            "Search, Quick Open and symbols"
        } else {
            "the Explorer, Search and symbols"
        };
        self.message = Some(super::Message {
            text: format!(
                "Excluded {pattern}: it's out of {what}. Saved in .merlin/settings.toml."
            ),
            error: false,
            done: true,
            warning: false,
            actions: Vec::new(),
            link: None,
            code: None,
            choice: None,
        });
        self.add_action("Open Exclusions", "settings.files");
        self.add_action("Undo", "explorer.undo_exclude");
        self.last_exclude = Some((pattern, search));
    }

    /// The notice's Undo: the pattern Exclude added comes out again.
    pub(crate) fn undo_exclude(&mut self, fx: &mut Vec<Effect>) {
        if let Some(rule) = self.last_exclude.take() {
            fx.push(Effect::Include(vec![rule]));
            self.message = None;
        }
    }

    /// The Explorer's ⊘: shows what's left out, dimmed, or stops.
    pub(crate) fn toggle_left_out(&mut self, fx: &mut Vec<Effect>) {
        let tree = self.tree.get_or_insert_with(Default::default);
        tree.show_left_out = !tree.show_left_out;
        tree.built_for = None;
        if !tree.show_left_out {
            tree.left_out.clear();
        }
        fx.push(Effect::ShowLeftOut(tree.show_left_out));
        self.build_views();
    }

    /// What a name typed in the Explorer makes happen.
    fn named(&mut self, naming: Naming, name: &str, fx: &mut Vec<Effect>) {
        let name = name.trim().trim_start_matches('/');
        if name.is_empty() || name.split('/').any(|p| p == "..") {
            return self.error("Type a name inside the project".into());
        }
        let within = |dir: &str| {
            if dir.is_empty() {
                PathBuf::from(name)
            } else {
                PathBuf::from(dir).join(name)
            }
        };
        let op = match naming {
            Naming::NewFile { dir } => FileOp::CreateFile(within(&dir)),
            Naming::NewFolder { dir } => FileOp::CreateFolder(within(&dir)),
            Naming::Rename { from } => {
                let to = match from.rsplit_once('/') {
                    Some((dir, _)) => PathBuf::from(dir).join(name),
                    None => PathBuf::from(name),
                };
                FileOp::Rename {
                    from: PathBuf::from(from),
                    to,
                }
            }
        };
        fx.push(Effect::File(op));
    }

    /// What the system's file dialog came back with.
    pub(crate) fn picked(&mut self, pick: FilePick, path: Option<PathBuf>, fx: &mut Vec<Effect>) {
        let Some(path) = path else {
            // Cancelled: nothing waits on it any more.
            self.quitting = false;
            return;
        };
        // Inside the project, files go by their path in it.
        let path = path
            .strip_prefix(&self.root)
            .map(PathBuf::from)
            .unwrap_or(path);
        match pick {
            FilePick::Open => self.open(path, fx),
            FilePick::OpenFolder => {
                self.open_folder = Some(path);
                self.run("app.quit", fx);
            }
            FilePick::SaveAs { doc } => {
                let Some(i) = self.index_of(doc) else { return };
                let back = self.docs[self.current].id;
                self.current = i;
                self.doc_mut().buffer.path = Some(path);
                self.run("file.save", fx);
                if let Some(j) = self.index_of(back) {
                    self.current = j;
                }
            }
        }
    }

    /// A file or folder was renamed on disk: open files under it follow.
    pub(crate) fn renamed(&mut self, from: PathBuf, to: PathBuf) {
        for doc in &mut self.docs {
            let Some(path) = &doc.buffer.path else {
                continue;
            };
            if let Ok(rest) = path.strip_prefix(&from) {
                doc.buffer.path = Some(if rest.as_os_str().is_empty() {
                    to.clone()
                } else {
                    to.join(rest)
                });
            }
        }
        self.closed.retain(|p| !p.starts_with(&from));
        if let Some(tree) = &mut self.tree {
            tree.built_for = None;
        }
    }

    /// A file or folder went to the trash: open files from it close, if
    /// they have no changes to lose.
    pub(crate) fn trashed(&mut self, path: PathBuf) {
        let gone: Vec<u64> = self
            .docs
            .iter()
            .filter(|d| d.buffer.path.as_ref().is_some_and(|p| p.starts_with(&path)))
            .filter(|d| !d.buffer.is_dirty())
            .map(|d| d.id)
            .collect();
        let keep = self.doc().id;
        for id in gone {
            if let Some(i) = self.index_of(id) {
                self.current = i;
                self.close(true);
                self.sync_groups();
            }
        }
        if let Some(i) = self.index_of(keep) {
            self.current = i;
        }
    }
}
