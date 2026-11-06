//! Icons, from a Nerd Font as VS Code's codicons and file icon theme show
//! them, or ordinary symbols for fonts without them. A Nerd Font glyph
//! takes one cell but may draw into the next, so callers leave a space
//! after each.

use crate::style::Hue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    Explorer,
    Search,
    SourceControl,
    /// A plugin's panel in the activity bar.
    Plugin,

    Open,
    Closed,
    Error,
    Warning,
    Info,
    Branch,
    Diff,
    Outline,
    Testing,
    Debug,
}

pub(crate) fn icon(icon: Icon, nerd: bool) -> char {
    use Icon::*;
    if nerd {
        match icon {
            Explorer => '\u{eaf0}',
            Search => '\u{ea6d}',
            SourceControl => '\u{ea68}',
            Plugin => '\u{eae6}',
            Open => '\u{eab4}',
            Closed => '\u{eab6}',
            Error => '\u{ea87}',
            Warning => '\u{ea6c}',
            Info => '\u{ea74}',
            Branch => '\u{e725}',
            Diff => '\u{eae1}',
            Outline => '\u{eb86}',
            Testing => '\u{ea79}',
            Debug => '\u{eb91}',
        }
    } else {
        match icon {
            Explorer => '❐',
            Search => '⌕',
            SourceControl => '⑂',
            Plugin => '✦',
            Open => '▾',
            Closed => '▸',
            Error => '⊗',
            Warning => '⚠',
            Info => 'ⓘ',
            Branch => '⎇',
            Diff => '⇆',
            Outline => '≣',
            Testing => '⚗',
            Debug => '▷',
        }
    }
}

/// A file's icon and its colour, by name.
pub fn file_icon(name: &str, nerd: bool) -> (char, Hue) {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    let (glyph, plain, hue) = match ext {
        "rs" => ('\u{e7a8}', '◆', Hue::Orange),
        "py" => ('\u{e73c}', '◆', Hue::Blue),
        "js" | "mjs" | "cjs" => ('\u{e74e}', '◆', Hue::Yellow),
        "ts" | "tsx" => ('\u{e628}', '◆', Hue::Blue),
        "json" => ('\u{e60b}', '◇', Hue::Yellow),
        "md" => ('\u{e73e}', '≡', Hue::Blue),
        "toml" | "yaml" | "yml" | "ini" | "conf" => ('\u{e615}', '⚙', Hue::Grey),
        "lock" => ('\u{f023}', '▤', Hue::Grey),
        "sh" | "bash" | "zsh" | "fish" => ('\u{e795}', '$', Hue::Green),
        "c" | "h" => ('\u{e61e}', '◆', Hue::Blue),
        "cc" | "cpp" | "hpp" => ('\u{e61d}', '◆', Hue::Blue),
        "go" => ('\u{e627}', '◆', Hue::Cyan),
        "html" => ('\u{e736}', '◇', Hue::Orange),
        "css" => ('\u{e749}', '◇', Hue::Blue),
        "lua" => ('\u{e620}', '◆', Hue::Blue),
        _ if lower.starts_with(".git") => ('\u{e702}', '▫', Hue::Orange),
        _ => ('\u{ea7b}', '▫', Hue::Grey),
    };
    (if nerd { glyph } else { plain }, hue)
}
