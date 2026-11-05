//! The editor's settings, as the host reads them from settings.toml.

use super::{AutoSave, Editor};
use crate::indent::Indent;

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
    /// The last line can scroll up to the top, rather than stopping at
    /// the bottom of the view.
    pub scroll_beyond_last_line: bool,
    /// Brackets are coloured by how deep they are.
    pub bracket_pair_colorization: bool,
    /// A swatch before colours written in the code, such as `#e6c07b`.
    pub color_decorators: bool,
    /// A line's worst problem is written after its end.
    pub inline_diagnostics: bool,
    /// Saving takes the spaces and tabs off the ends of lines.
    pub trim_trailing_whitespace: bool,
    /// Saving ends the file with a newline.
    pub insert_final_newline: bool,
    pub auto_save: AutoSave,
    /// Milliseconds after the last change, with [`AutoSave::AfterDelay`].
    pub auto_save_delay: u64,
    /// Keys and the commands they run, such as `ctrl+k ctrl+m` and
    /// `view.minimap`, maybe only when a condition holds. An empty command
    /// unbinds the keys.
    pub keys: Vec<super::UserKey>,
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
            scroll_beyond_last_line: true,
            bracket_pair_colorization: true,
            color_decorators: true,
            inline_diagnostics: true,
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            auto_save: AutoSave::Off,
            auto_save_delay: 1000,
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
        self.scroll_beyond_last_line = settings.scroll_beyond_last_line;
        self.bracket_colours = settings.bracket_pair_colorization;
        self.colour_swatches = settings.color_decorators;
        self.inline_notes = settings.inline_diagnostics;
        self.saving.trim_trailing_whitespace = settings.trim_trailing_whitespace;
        self.saving.insert_final_newline = settings.insert_final_newline;
        self.saving.auto = settings.auto_save;
        self.saving.delay_ms = settings.auto_save_delay.max(10);
        if !self.preview_tabs {
            self.preview = None;
        }
        self.user_keys.clear();
        let mut bad = Vec::new();
        for user in settings.keys {
            let Some(keys) = super::keymap::parse_keys(&user.keys) else {
                bad.push(format!("unknown keys: {}", user.keys));
                continue;
            };
            let when = match user.when.as_deref().map(super::When::parse) {
                None => super::When::default(),
                Some(Ok(when)) => when,
                Some(Err(err)) => {
                    bad.push(err);
                    continue;
                }
            };
            self.user_keys.push(super::keymap::Binding {
                keys,
                command: user.command,
                when,
                from: super::keymap::From::Settings,
            });
        }
        self.rebuild_keymap();
        if !bad.is_empty() {
            self.error(format!("Settings: {}", bad.join("; ")));
        }
    }
}
