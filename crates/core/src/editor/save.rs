//! What happens around saving: an `.editorconfig`'s say over a file,
//! tidying whitespace on save, and saving by itself.

use super::{Editor, Effect, Focus};
use crate::edit::Edit;
use crate::indent::Indent;

/// What `.editorconfig` files say about one file. `None` leaves it to the
/// settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileConfig {
    /// `indent_style`: tabs, or spaces.
    pub tabs: Option<bool>,
    /// `indent_size`, in columns.
    pub indent_size: Option<usize>,
    /// `tab_width`: how wide a tab is drawn.
    pub tab_width: Option<usize>,
    pub trim_trailing_whitespace: Option<bool>,
    pub insert_final_newline: Option<bool>,
}

/// When files save by themselves, as VS Code's `files.autoSave` has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoSave {
    #[default]
    Off,
    /// A while after the last change: see `Settings::auto_save_delay`.
    AfterDelay,
    /// When the code loses the keyboard, to another file, a panel or
    /// another window.
    OnFocusChange,
    /// When the window loses the keyboard.
    OnWindowChange,
}

/// What asks for an automatic save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSaveWhy {
    /// The delay after the last change is up.
    Delay,
    /// The window lost the keyboard.
    Window,
}

/// Saving's state in the editor.
#[derive(Debug, Default)]
pub(crate) struct Saving {
    pub auto: AutoSave,
    pub delay_ms: u64,
    pub trim_trailing_whitespace: bool,
    pub insert_final_newline: bool,
}

impl Editor {
    /// The file's indentation: the `.editorconfig`'s, else the file's own
    /// when detection is on, else the settings'.
    pub(crate) fn indent(&self) -> Indent {
        let doc = self.doc();
        let config = &doc.config;
        let mut indent = match doc.indent {
            Some(found) if self.detect_indentation && config.tabs.is_none() => Indent {
                // A tab is drawn as wide as the settings say.
                width: if found.tabs {
                    self.indent.width
                } else {
                    found.width
                },
                ..found
            },
            _ => self.indent,
        };
        if let Some(tabs) = config.tabs {
            indent.tabs = tabs;
        }
        let width = if indent.tabs {
            config.tab_width.or(config.indent_size)
        } else {
            config.indent_size
        };
        if let Some(width) = width {
            indent.width = width.clamp(1, 16);
        }
        indent
    }

    /// Takes in what `.editorconfig` files say about an open file.
    pub(crate) fn file_config(&mut self, path: &std::path::Path, config: FileConfig) {
        if let Some(doc) = self
            .docs
            .iter_mut()
            .find(|d| d.buffer.path.as_deref() == Some(path))
        {
            doc.config = config;
        }
    }

    /// Saves the current file, tidied as the settings and its
    /// `.editorconfig` ask.
    pub(crate) fn save_current(&mut self, fx: &mut Vec<Effect>) {
        self.tidy_for_save(false);
        self.push_save(fx);
    }

    fn push_save(&mut self, fx: &mut Vec<Effect>) {
        let doc = self.doc();
        if let Some(path) = &doc.buffer.path {
            fx.push(Effect::Save {
                doc: doc.id,
                path: path.clone(),
                text: doc.buffer.text(),
                revision: doc.buffer.revision(),
            });
        }
    }

    /// Trims trailing whitespace and adds or keeps one final newline, as
    /// one step to undo. Markdown keeps its trailing spaces, which break
    /// lines, unless an `.editorconfig` says otherwise. Saving by itself
    /// leaves the cursors' lines, so a space just typed stays.
    fn tidy_for_save(&mut self, auto: bool) {
        let skip: Vec<usize> = if auto {
            self.carets()
                .iter()
                .map(|c| self.doc().line_col(c.cursor).0)
                .collect()
        } else {
            Vec::new()
        };
        let doc = self.doc();
        if doc.page.is_some() || doc.review.is_some() {
            return;
        }
        let markdown = matches!(self.extension(), "md" | "markdown");
        let trim = doc
            .config
            .trim_trailing_whitespace
            .unwrap_or(self.saving.trim_trailing_whitespace && !markdown);
        let final_newline = doc
            .config
            .insert_final_newline
            .unwrap_or(self.saving.insert_final_newline);
        if !trim && !final_newline {
            return;
        }
        let rope = doc.buffer.rope();
        let mut edits = Vec::new();
        if trim {
            for line in (0..rope.len_lines()).filter(|l| !skip.contains(l)) {
                let start = rope.line_to_char(line);
                let text = rope.line(line);
                let body: String = text
                    .chars()
                    .take_while(|&c| c != '\n' && c != '\r')
                    .collect();
                let kept = body.trim_end_matches([' ', '\t']).chars().count();
                let len = body.chars().count();
                if kept < len {
                    edits.push(Edit {
                        at: start + kept,
                        removed: body.chars().skip(kept).collect(),
                        inserted: String::new(),
                    });
                }
            }
        }
        let len = rope.len_chars();
        if final_newline && len > 0 && rope.char(len - 1) != '\n' {
            let crlf = rope.len_lines() > 1 && rope.line(0).to_string().ends_with("\r\n");
            edits.push(Edit {
                at: len,
                removed: String::new(),
                inserted: if crlf { "\r\n" } else { "\n" }.into(),
            });
        }
        if !edits.is_empty() {
            self.apply_all(edits);
        }
    }

    /// Saves every changed file that has a name, without saying so for
    /// each.
    pub(crate) fn auto_save(&mut self, why: AutoSaveWhy, fx: &mut Vec<Effect>) {
        let wanted = match why {
            AutoSaveWhy::Delay => self.saving.auto == AutoSave::AfterDelay,
            AutoSaveWhy::Window => matches!(
                self.saving.auto,
                AutoSave::OnFocusChange | AutoSave::OnWindowChange
            ),
        };
        if wanted {
            let dirty: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
            self.save_quietly(&dirty, fx);
        }
    }

    /// Saves those of `docs` that have changes and a name.
    fn save_quietly(&mut self, docs: &[u64], fx: &mut Vec<Effect>) {
        let back = self.current;
        for &id in docs {
            let Some(i) = self.index_of(id) else { continue };
            let doc = &self.docs[i];
            if !doc.buffer.is_dirty() || doc.buffer.path.is_none() || doc.page.is_some() {
                continue;
            }
            self.current = i;
            self.tidy_for_save(true);
            self.push_save(fx);
        }
        self.current = back.min(self.docs.len().saturating_sub(1));
    }

    /// With saving on focus change, the file the code was showing saves
    /// once the keyboard leaves it.
    pub(crate) fn save_on_focus_change(&mut self, was: (Focus, u64), fx: &mut Vec<Effect>) {
        if self.saving.auto != AutoSave::OnFocusChange || was.0 != Focus::Editor {
            return;
        }
        if (self.focus, self.doc().id) != was {
            self.save_quietly(&[was.1], fx);
        }
    }

    /// How long after the last change files save by themselves, if they
    /// do.
    pub fn auto_save_delay(&self) -> Option<u64> {
        (self.saving.auto == AutoSave::AfterDelay).then_some(self.saving.delay_ms)
    }
}
