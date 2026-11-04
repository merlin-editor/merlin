//! Colours. A theme is a [`Palette`]: merlin's own, or one read from a theme
//! file that a plugin adds. A theme file names the colours it cares about;
//! the rest follow from those, so a theme can be a dozen lines.

use anyhow::{Result, bail};
use merlin_core::Syntax;
use merlin_core::lang::Severity;
use merlin_core::style::{Cell, Hue, Style as Kind, Surface};

/// How a cell looks, as colours and text attributes.
#[derive(Clone, Copy, Default)]
pub struct Look {
    pub fg: Option<u32>,
    pub bg: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// The underline's colour, if not the text's.
    pub underline_colour: Option<u32>,
    /// A wavy underline, as for problems.
    pub curly: bool,
}

/// Every colour the window uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Behind the islands: the title bar, rail and status bar.
    pub ground: u32,
    /// The side bar, editor and panel.
    pub island: u32,
    /// The active tab, inputs, pressed buttons.
    pub raised: u32,
    pub hover: u32,
    /// A selected row.
    pub selected: u32,
    /// A selected row where keys go.
    pub focus_row: u32,
    /// Lines between parts.
    pub line: u32,
    /// Keys, counts and other small boxes, and their text.
    pub chip: u32,
    pub chip_text: u32,
    /// Around inputs and floating boxes.
    pub border: u32,
    /// Floating boxes: the palette, notices, hovers, menus.
    pub widget: u32,
    pub text: u32,
    pub bright: u32,
    pub muted: u32,
    /// Line numbers.
    pub faint: u32,
    /// Lines, markers, the cursor and the main button.
    pub accent: u32,
    /// Text on the accent.
    pub on_accent: u32,
    pub error: u32,
    pub warning: u32,
    pub success: u32,
    pub info: u32,

    pub current_line: u32,
    pub selection: u32,
    /// Other uses of the word under the cursor.
    pub word_match: u32,
    pub find_match: u32,
    pub find_current: u32,
    pub keyword: u32,
    pub string: u32,
    pub comment: u32,
    pub function: u32,
    pub type_: u32,
    pub constant: u32,
    pub attribute: u32,
    pub macro_: u32,
    pub operator: u32,
    pub variable: u32,
    pub escape: u32,
    pub git_added: u32,
    pub git_changed: u32,
    pub git_removed: u32,
    pub diff_added: u32,
    pub diff_removed: u32,
    pub brackets: [u32; 3],
    pub guide: u32,
    pub guide_active: u32,
}

/// The name of merlin's own theme, as the theme picker shows it.
pub const DEFAULT_NAME: &str = "merlin Dark";

/// merlin's own theme: quiet islands on a darker ground, with a blue accent.
pub const DEFAULT: Palette = Palette {
    ground: 0x0f1012,
    island: 0x17181b,
    raised: 0x1f2024,
    hover: 0x1d1e22,
    selected: 0x24262c,
    focus_row: 0x262b3d,
    line: 0x222328,
    chip: 0x26272c,
    chip_text: 0xb4b6bd,
    border: 0x2c2d33,
    widget: 0x1c1d21,
    text: 0xd4d5da,
    bright: 0xeef0f3,
    muted: 0x8e9099,
    faint: 0x5f626b,
    accent: 0x7c9cff,
    on_accent: 0x0f1012,
    error: 0xf0807a,
    warning: 0xe3b341,
    success: 0x6cc070,
    info: 0x5aa0e6,
    current_line: 0x1d1e23,
    selection: 0x2c3a5e,
    word_match: 0x2a2d36,
    find_match: 0x4a3a1e,
    find_current: 0x8a6416,
    keyword: 0xc4a7e7,
    string: 0xa6d189,
    comment: 0x7f8794,
    function: 0xf0c674,
    type_: 0x9ccfd8,
    constant: 0xf5a97f,
    attribute: 0xe0a8d8,
    macro_: 0x9ccfd8,
    operator: 0xa3a6ad,
    variable: 0xd4d5da,
    escape: 0xf5a97f,
    git_added: 0x6cc070,
    git_changed: 0x5aa0e6,
    git_removed: 0xe5534b,
    diff_added: 0x24301f,
    diff_removed: 0x3a1d1d,
    brackets: [0xe6c07b, 0xc4a7e7, 0x7fb0ff],
    guide: 0x2a2b31,
    guide_active: 0x4a4c55,
};

impl Palette {
    /// A palette from a pack's `[ui]` and `[code]` colours, as `"#rrggbb"`.
    /// Anything left out follows from what's given, then from merlin's own.
    pub fn from_table(table: &toml::Table) -> Result<Palette> {
        let section = |name: &str| table.get(name).and_then(|v| v.as_table());
        let (ui, code) = (section("ui"), section("code"));
        for (name, part) in [("ui", ui), ("code", code)] {
            for (key, value) in part.into_iter().flatten() {
                if key != "brackets" && parse(value).is_none() {
                    bail!("[{name}] {key} should be a colour like \"#1a2b3c\"");
                }
            }
        }
        let get =
            |part: Option<&toml::Table>, key: &str| part.and_then(|t| t.get(key)).and_then(parse);
        let d = DEFAULT;
        let island = get(ui, "island").unwrap_or(d.island);
        let text = get(ui, "text").unwrap_or(d.text);
        let accent = get(ui, "accent").unwrap_or(d.accent);
        let ground = get(ui, "ground").unwrap_or_else(|| mix(island, 0x000000, 0.7));
        let over = |t: f32| mix(text, island, t);
        let warning = get(ui, "warning").unwrap_or(d.warning);
        let error = get(ui, "error").unwrap_or(d.error);
        let success = get(ui, "success").unwrap_or(d.success);
        let info = get(ui, "info").unwrap_or(d.info);
        let ui = |key: &str, or: u32| get(ui, key).unwrap_or(or);
        let c = |key: &str, or: u32| get(code, key).unwrap_or(or);
        let keyword = c("keyword", d.keyword);
        let type_ = c("type", d.type_);
        let constant = c("constant", d.constant);
        let brackets = code
            .and_then(|t| t.get("brackets"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(parse).collect::<Vec<_>>())
            .filter(|a| a.len() == 3)
            .map_or([warning, keyword, type_], |a| [a[0], a[1], a[2]]);
        Ok(Palette {
            ground,
            island,
            raised: ui("raised", over(0.06)),
            hover: ui("hover", over(0.035)),
            selected: ui("selected", over(0.09)),
            focus_row: ui("focus-row", mix(accent, island, 0.16)),
            line: ui("line", over(0.07)),
            chip: ui("chip", over(0.1)),
            chip_text: ui("chip-text", over(0.8)),
            border: ui("border", over(0.13)),
            widget: ui("widget", mix(text, ground, 0.06)),
            text,
            bright: ui("bright", mix(0xffffff, text, 0.5)),
            muted: ui("muted", over(0.6)),
            faint: ui("faint", over(0.35)),
            accent,
            on_accent: ui("on-accent", ground),
            error,
            warning,
            success,
            info,
            current_line: c("current-line", over(0.035)),
            selection: c("selection", mix(accent, island, 0.25)),
            word_match: c("word-match", over(0.08)),
            find_match: c("find-match", mix(warning, island, 0.3)),
            find_current: c("find-current", mix(warning, island, 0.6)),
            keyword,
            string: c("string", d.string),
            comment: c("comment", over(0.5)),
            function: c("function", d.function),
            type_,
            constant,
            attribute: c("attribute", keyword),
            macro_: c("macro", type_),
            operator: c("operator", over(0.75)),
            variable: c("variable", text),
            escape: c("escape", constant),
            git_added: c("git-added", success),
            git_changed: c("git-changed", info),
            git_removed: c("git-removed", error),
            diff_added: c("diff-added", mix(success, island, 0.14)),
            diff_removed: c("diff-removed", mix(error, island, 0.14)),
            brackets,
            guide: c("guide", over(0.1)),
            guide_active: c("guide-active", over(0.28)),
        })
    }
}

/// `"#1a2b3c"` as a colour.
fn parse(value: &toml::Value) -> Option<u32> {
    let s = value.as_str()?.strip_prefix('#')?;
    (s.len() == 6).then(|| u32::from_str_radix(s, 16).ok())?
}

fn look(fg: u32, bg: u32) -> Look {
    Look {
        fg: Some(fg),
        bg: Some(bg),
        ..Look::default()
    }
}

/// A look whose background is the surface's.
fn text(fg: u32) -> Look {
    Look {
        fg: Some(fg),
        ..Look::default()
    }
}

impl Look {
    fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    fn underline(mut self, colour: u32) -> Self {
        self.underline = true;
        self.underline_colour = Some(colour);
        self
    }
}

/// The palette's colours for each kind of cell. Text kinds have no
/// background, and show the surface's.
pub fn kind(p: &Palette, kind: Kind) -> Look {
    let fill = mix(p.accent, 0x000000, 0.55);
    match kind {
        Kind::Normal | Kind::ScrollTrack => text(p.text),
        Kind::Gutter => text(p.faint),
        Kind::GutterActive => text(p.text),
        Kind::GutterError => text(p.error).bold(),
        Kind::GutterWarning => text(p.warning),
        Kind::Muted => text(p.muted),
        Kind::Border => text(p.border),
        Kind::Match => look(p.text, p.find_match),
        Kind::MatchCurrent => look(p.bright, p.find_current),
        Kind::Selection => look(p.text, p.selection),
        Kind::CurrentLine => look(p.text, p.current_line),
        Kind::WordMatch => look(p.text, p.word_match),
        Kind::Heading => text(p.bright).bold(),
        Kind::Button => look(p.text, p.chip),
        Kind::ButtonFocused => look(0xffffff, fill),
        Kind::Prompt | Kind::Widget => look(p.text, p.widget),
        Kind::WidgetInput => look(p.text, p.chip),
        Kind::WidgetMuted => look(p.muted, p.widget),
        Kind::Status | Kind::MenuBar => look(p.text, p.ground),
        Kind::TabBar => look(p.muted, p.ground),
        Kind::StatusError => look(p.error, p.ground),
        Kind::StatusAccent => look(0xffffff, fill),
        Kind::MenuTitle => text(p.muted),
        Kind::MenuOpen => look(p.bright, p.chip),
        Kind::Menu => look(p.text, p.widget),
        Kind::MenuSelected => look(0xffffff, fill),
        Kind::MenuMuted => look(p.muted, p.widget),
        Kind::CommandCenter => look(p.muted, p.raised),
        Kind::Selected => look(p.bright, p.selected),
        Kind::Warning => text(p.warning),
        Kind::Error => text(p.error),
        Kind::Success => text(p.success),
        Kind::Input => look(p.text, p.chip),
        Kind::InputPlaceholder => look(p.muted, p.chip),
        Kind::Tab => text(p.muted),
        Kind::TabActive => look(p.bright, p.island).underline(p.accent),
        Kind::TabPreview => text(p.muted).italic(),
        Kind::TabPreviewActive => look(p.bright, p.island).underline(p.accent).italic(),
        Kind::Popup => look(p.text, p.widget),
        Kind::PopupMuted => look(p.muted, p.widget).italic(),
        Kind::Emphasis => text(p.text).italic(),
        Kind::Code => text(p.constant),
        Kind::CodeBlock => look(p.text, p.ground),
        Kind::Link => text(p.accent).underline(p.accent),
        Kind::DiffAdded => look(p.text, p.diff_added),
        Kind::DiffRemoved => look(p.text, p.diff_removed),
        Kind::GitAdded => text(p.git_added),
        Kind::GitChanged => text(p.git_changed),
        Kind::GitRemoved => text(p.git_removed),
        Kind::ScrollThumb => look(p.text, p.border),
        Kind::SideTitle => text(p.text),
        Kind::SideTitleFocused => text(p.bright),
        Kind::PanelTitle => text(p.muted),
        Kind::PanelTitleActive => text(p.bright).underline(p.accent),
        Kind::Toast => look(p.text, p.widget),
        Kind::ToastError => look(0xffffff, mix(p.error, p.island, 0.3)),
        Kind::Frame => text(p.border),
        Kind::WidgetFrame => look(p.border, p.widget),
        Kind::Badge => look(p.bright, p.chip),
        Kind::Activity => text(p.muted),
        Kind::ActivityActive => text(p.bright),
        Kind::ActivityMarker => text(p.accent),
        Kind::ActivityBadge => look(p.on_accent, p.accent),
        Kind::ScrollMatch => text(p.warning),
    }
}

fn surface(p: &Palette, surface: Surface) -> u32 {
    match surface {
        Surface::Editor => p.island,
        Surface::Bar => p.ground,
        Surface::Hover => p.hover,
        Surface::Sticky => p.widget,
    }
}

/// Whether the palette is light, with dark text on pale islands.
pub fn is_light(p: &Palette) -> bool {
    let c = p.island;
    ((c >> 16) & 0xff) + ((c >> 8) & 0xff) + (c & 0xff) > 3 * 128
}

/// Colours for icons, guides and brackets. File icons keep their usual
/// hues in every theme, so files look the same wherever they're seen,
/// darkened on light themes to stay readable.
pub fn hue(p: &Palette, hue: Hue) -> u32 {
    let file = |c: u32| {
        if is_light(p) {
            mix(c, 0x000000, 0.62)
        } else {
            c
        }
    };
    match hue {
        Hue::Blue => file(0x519aba),
        Hue::Green => file(0x8dc149),
        Hue::Yellow => file(0xcbcb41),
        Hue::Orange => file(0xdea584),
        Hue::Red => file(0xcc3e44),
        Hue::Purple => file(0xa074c4),
        Hue::Cyan => file(0x519aba),
        Hue::Grey => file(0x8a9aa8),
        Hue::Guide => p.guide,
        Hue::GuideActive => p.guide_active,
        Hue::TreeGuide => p.guide,
        Hue::Bracket1 => p.brackets[0],
        Hue::Bracket2 => p.brackets[1],
        Hue::Bracket3 => p.brackets[2],
        Hue::Modified => p.git_changed,
        Hue::Untracked => p.git_added,
        Hue::Deleted => p.git_removed,
        Hue::Problem => p.error,
        Hue::Warned => p.warning,
    }
}

fn severity(p: &Palette, severity: Severity) -> u32 {
    match severity {
        Severity::Error => p.error,
        Severity::Warning => p.warning,
        Severity::Info => p.info,
        Severity::Hint => p.muted,
    }
}

/// The palette's colours for code.
pub fn code(p: &Palette, syntax: Syntax) -> Option<u32> {
    Some(match syntax {
        Syntax::Keyword => p.keyword,
        Syntax::String => p.string,
        Syntax::Comment => p.comment,
        Syntax::Function => p.function,
        Syntax::Type => p.type_,
        Syntax::Constant => p.constant,
        Syntax::Attribute => p.attribute,
        Syntax::Property | Syntax::Variable => p.variable,
        Syntax::Macro => p.macro_,
        Syntax::Escape => p.escape,
        Syntax::Operator => p.operator,
        Syntax::Plain | Syntax::Punctuation => return None,
    })
}

/// Whether code colours show on a cell.
pub fn shows_code(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Normal
            | Kind::Match
            | Kind::Selection
            | Kind::CodeBlock
            | Kind::CurrentLine
            | Kind::WordMatch
            | Kind::DiffAdded
            | Kind::DiffRemoved
    )
}

/// How a cell looks: its kind, on its surface, with code colours, a tint
/// and a problem's underline.
pub fn cell(p: &Palette, cell: &Cell) -> Look {
    let mut l = kind(p, cell.style);
    if l.bg.is_none() {
        l.bg = Some(surface(p, cell.surface));
    }
    if shows_code(cell.style)
        && let Some(colour) = code(p, cell.syntax)
    {
        l.fg = Some(colour);
        l.italic |= cell.syntax == Syntax::Comment;
    }
    if let Some(h) = cell.tint {
        l.fg = Some(hue(p, h));
    }
    if let Some(s) = cell.mark {
        l.underline = true;
        l.curly = true;
        l.underline_colour = Some(severity(p, s));
    }
    l
}

/// `a` with `t` of it kept and the rest `b`, channel by channel.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    [16, 8, 0].iter().fold(0, |out, shift| {
        let (x, y) = ((a >> shift) & 0xff, (b >> shift) & 0xff);
        let c = (x as f32 * t + y as f32 * (1.0 - t)).round() as u32;
        out | (c << shift)
    })
}

impl Look {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_short_theme_fills_in_the_rest() {
        let table: toml::Table = toml::from_str(
            r##"
            [ui]
            island = "#141c24"
            text = "#f1f7fc"
            accent = "#75b0de"
            [code]
            keyword = "#e6a3c8"
            "##,
        )
        .unwrap();
        let p = Palette::from_table(&table).unwrap();
        assert_eq!(p.island, 0x141c24);
        assert_eq!(p.keyword, 0xe6a3c8);
        assert_eq!(p.attribute, p.keyword, "attributes follow keywords");
        assert!(p.ground < p.island, "the ground is darker than the islands");
        assert_ne!(
            p.selection, DEFAULT.selection,
            "selection follows the accent"
        );
    }

    /// WCAG contrast between two colours.
    fn contrast(a: u32, b: u32) -> f32 {
        let lum = |c: u32| {
            let ch = |s: u32| {
                let x = ((c >> s) & 0xff) as f32 / 255.0;
                if x <= 0.03928 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// The themes that come with merlin stay comfortable to read: text
    /// clear but not glaring, comments and code readable, buttons legible.
    #[test]
    fn bundled_themes_are_readable() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/merlin-themes/themes");
        let mut themes: Vec<(String, Palette)> = vec![(DEFAULT_NAME.into(), DEFAULT)];
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let pack = crate::packs::Pack::read("test", &path).unwrap();
            let look = crate::packs::Look::stack(&[&pack]).unwrap();
            themes.push((path.display().to_string(), look.palette));
        }
        assert!(themes.len() > 8, "the bundled themes are there");
        for (name, p) in themes {
            let text = contrast(p.text, p.island);
            assert!((7.0..=17.0).contains(&text), "{name}: text at {text:.1}:1");
            for (what, colour) in [
                ("muted", p.muted),
                ("comment", p.comment),
                ("keyword", p.keyword),
                ("string", p.string),
                ("function", p.function),
                ("type", p.type_),
                ("constant", p.constant),
            ] {
                let c = contrast(colour, p.island);
                assert!(c >= 4.5, "{name}: {what} at {c:.1}:1");
            }
            let on = contrast(p.on_accent, p.accent);
            assert!(on >= 4.5, "{name}: text on the accent at {on:.1}:1");
        }
    }

    #[test]
    fn a_bad_colour_says_where() {
        let table: toml::Table = toml::from_str("[ui]\naccent = \"blue\"").unwrap();
        let err = Palette::from_table(&table).unwrap_err().to_string();
        assert!(err.contains("accent"), "{err}");
    }
}
