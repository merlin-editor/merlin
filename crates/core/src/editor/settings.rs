//! The editor's settings, as the host reads them from settings.toml.

use super::Editor;
use crate::indent::Indent;
use crate::key::Key;

/// What the `[editor]` and `[keys]` tables of the settings say, with
/// VS Code's defaults for what they leave out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub minimap: bool,
    pub word_wrap: bool,
    pub breadcrumbs: bool,
    pub sticky_scroll: bool,
    /// A click in the Explorer opens a preview tab, which the next one
    /// replaces.
    pub preview_tabs: bool,
    /// Spaces an indent level takes, for files with no indentation of
    /// their own.
    pub tab_size: usize,
    /// Tab and Enter indent with spaces rather than tabs.
    pub insert_spaces: bool,
    /// A file's own indentation wins over `tab_size` and `insert_spaces`.
    pub detect_indentation: bool,
    /// Typing an opening bracket or quote adds its closer.
    pub auto_closing_brackets: bool,
    /// Keys and the commands they run, such as `("ctrl+k", "view.minimap")`.
    /// An empty command unbinds the key.
    pub keys: Vec<(String, String)>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            minimap: false,
            word_wrap: false,
            breadcrumbs: true,
            sticky_scroll: true,
            preview_tabs: true,
            tab_size: 4,
            insert_spaces: true,
            detect_indentation: true,
            auto_closing_brackets: true,
            keys: Vec::new(),
        }
    }
}

impl Editor {
    pub(crate) fn apply_settings(&mut self, settings: Settings) {
        self.minimap = settings.minimap;
        if self.wrap != settings.word_wrap {
            self.wrap = settings.word_wrap;
            self.doc_mut().hscroll = 0;
        }
        self.breadcrumbs = settings.breadcrumbs;
        self.sticky_scroll = settings.sticky_scroll;
        self.preview_tabs = settings.preview_tabs;
        self.indent = Indent {
            tabs: !settings.insert_spaces,
            width: settings.tab_size.clamp(1, 16),
        };
        self.detect_indentation = settings.detect_indentation;
        self.auto_close = settings.auto_closing_brackets;
        if !self.preview_tabs {
            self.preview = None;
        }
        self.user_keys.clear();
        let mut bad = Vec::new();
        for (spec, command) in settings.keys {
            match Key::parse(&spec) {
                Some(key) => self.user_keys.push((key, command)),
                None => bad.push(spec),
            }
        }
        if !bad.is_empty() {
            self.error(format!("Settings: unknown keys: {}", bad.join(", ")));
        }
    }
}
