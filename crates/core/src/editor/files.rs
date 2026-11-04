//! Files on disk from the Explorer, as VS Code has it: a new file or
//! folder, rename, delete to the trash, copy a path, and the right-click
//! menu that offers them. Names are typed into the palette; the host does
//! the work and says what changed.

use std::path::PathBuf;

use super::context::ContextMenu;
use super::views::EXPLORER;
use super::{Editor, Effect, Focus, Palette, Side, Source};
use crate::panel::Target;

/// The Explorer's right-click menu, on a file or folder.
pub(crate) const EXPLORER_ITEMS: &[(&str, &str)] = &[
    ("explorer.new_file", "New File…"),
    ("explorer.new_folder", "New Folder…"),
    ("-", ""),
    ("explorer.open_side", "Open to the Side"),
    ("-", ""),
    ("explorer.copy_path", "Copy Path"),
    ("explorer.copy_relative", "Copy Relative Path"),
    ("-", ""),
    ("explorer.rename", "Rename…"),
    ("explorer.delete", "Delete"),
];

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
        self.explorer_row = row;
        let items = EXPLORER_ITEMS
            .iter()
            .map(|(id, _)| *id)
            .filter(|&id| match id {
                "explorer.open_side" => is_file,
                "explorer.copy_path"
                | "explorer.copy_relative"
                | "explorer.rename"
                | "explorer.delete" => on_row,
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
        });
        self.focus = Focus::Menu;
    }

    /// Asks for a name in the palette, starting with `text`.
    fn ask(&mut self, source: Source, title: &str, text: String) {
        let mut palette = Palette::new(source, Some(title.into()), Vec::new());
        palette.query = text;
        self.palette = Some(palette);
        self.focus = Focus::Palette;
    }

    /// Runs an `explorer.` command; false if it isn't one.
    pub(crate) fn explorer_command(&mut self, id: &str, fx: &mut Vec<Effect>) -> bool {
        let row = self.explorer_row();
        let path = row.as_deref().map(|r| r[2..].to_string());
        match id {
            "explorer.new_file" => {
                let dir = self.explorer_folder();
                self.ask(Source::NewFile { dir }, "New file", String::new());
            }
            "explorer.new_folder" => {
                let dir = self.explorer_folder();
                self.ask(Source::NewFolder { dir }, "New folder", String::new());
            }
            "explorer.rename" => {
                if let Some(from) = path {
                    let name = from.rsplit('/').next().unwrap_or(&from).to_string();
                    self.ask(Source::Rename { from }, "Rename to", name);
                }
            }
            "explorer.delete" => {
                if let Some(path) = path {
                    let title = format!("Move {path} to the trash?");
                    self.ask(Source::Trash { path }, &title, String::new());
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
            "explorer.open_side" => {
                if let Some(path) = path {
                    self.open(PathBuf::from(&path), fx);
                    self.run("view.split", None, fx);
                }
            }
            _ => return false,
        }
        self.explorer_row = None;
        true
    }

    /// What Enter in a file prompt does with the name typed.
    pub(super) fn file_prompt(&mut self, source: Source, name: &str, fx: &mut Vec<Effect>) {
        let name = name.trim().trim_start_matches('/');
        let within = |dir: &str| {
            if dir.is_empty() {
                PathBuf::from(name)
            } else {
                PathBuf::from(dir).join(name)
            }
        };
        let op = match source {
            Source::Trash { path } => FileOp::Trash(PathBuf::from(path)),
            _ if name.is_empty() || name.split('/').any(|p| p == "..") => {
                return self.error("Type a name inside the project".into());
            }
            Source::NewFile { dir } => FileOp::CreateFile(within(&dir)),
            Source::NewFolder { dir } => FileOp::CreateFolder(within(&dir)),
            Source::Rename { from } => {
                let to = match from.rsplit_once('/') {
                    Some((dir, _)) => PathBuf::from(dir).join(name),
                    None => PathBuf::from(name),
                };
                FileOp::Rename {
                    from: PathBuf::from(from),
                    to,
                }
            }
            _ => return,
        };
        fx.push(Effect::File(op));
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
