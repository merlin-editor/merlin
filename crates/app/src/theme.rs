//! Colours: VS Code's Dark Modern.

use merlin_core::Syntax;
use merlin_core::lang::Severity;
use merlin_core::screen::{Cell, Hue, Style as Kind, Surface};

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

const EDITOR: u32 = 0x1f1f1f;
const BAR: u32 = 0x181818;
const HOVER: u32 = 0x2a2d2e;
const TEXT: u32 = 0xcccccc;
const MUTED: u32 = 0x9d9d9d;
const BLUE: u32 = 0x0078d4;
const WIDGET: u32 = 0x252526;
const INPUT: u32 = 0x313131;
const RED: u32 = 0xf14c4c;
const YELLOW: u32 = 0xcca700;

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

/// Dark Modern's colours for each kind of cell. Text kinds have no
/// background, and show the surface's.
pub fn dark(kind: Kind) -> Look {
    match kind {
        Kind::Normal | Kind::ScrollTrack => text(TEXT),
        Kind::Gutter => text(0x6e7681),
        Kind::GutterActive => text(TEXT),
        Kind::GutterError => text(RED).bold(),
        Kind::GutterWarning => text(YELLOW),
        Kind::Muted => text(MUTED),
        Kind::Border => text(0x2b2b2b),
        Kind::Match => look(TEXT, 0x623315),
        Kind::MatchCurrent => look(0xffffff, 0x9e6a03),
        Kind::Selection => look(TEXT, 0x264f78),
        Kind::CurrentLine => look(TEXT, 0x282828),
        Kind::WordMatch => look(TEXT, 0x3b3f45),
        Kind::Heading => text(0xe7e7e7).bold(),
        Kind::Button => look(TEXT, INPUT),
        Kind::ButtonFocused => look(0xffffff, BLUE),
        Kind::Prompt | Kind::Widget => look(TEXT, WIDGET),
        Kind::WidgetInput => look(TEXT, INPUT),
        Kind::WidgetMuted => look(MUTED, WIDGET),
        Kind::Status | Kind::MenuBar => look(TEXT, BAR),
        Kind::TabBar => look(MUTED, BAR),
        Kind::StatusError => look(RED, BAR),
        Kind::StatusAccent => look(0xffffff, BLUE),
        Kind::MenuTitle => text(MUTED),
        Kind::MenuOpen => look(0xffffff, INPUT),
        Kind::Menu => look(TEXT, WIDGET),
        Kind::MenuSelected => look(0xffffff, BLUE),
        Kind::MenuMuted => look(MUTED, WIDGET),
        Kind::CommandCenter => look(MUTED, 0x2b2b2b),
        Kind::Selected => look(0xffffff, 0x04395e),
        Kind::Warning => text(0xe2c08d),
        Kind::Error => text(RED),
        Kind::Success => text(0x73c991),
        Kind::Input => look(TEXT, INPUT),
        Kind::InputPlaceholder => look(0x989898, INPUT),
        Kind::Tab => text(MUTED),
        Kind::TabActive => look(0xffffff, EDITOR).underline(BLUE),
        Kind::TabPreview => text(MUTED).italic(),
        Kind::TabPreviewActive => look(0xffffff, EDITOR).underline(BLUE).italic(),
        Kind::Popup => look(TEXT, 0x202020),
        Kind::PopupMuted => look(MUTED, 0x202020).italic(),
        Kind::Emphasis => text(TEXT).italic(),
        Kind::Code => text(0xd7ba7d),
        Kind::CodeBlock => look(TEXT, BAR),
        Kind::Link => text(0x4daafc).underline(0x4daafc),
        Kind::DiffAdded => look(TEXT, 0x373d29),
        Kind::DiffRemoved => look(TEXT, 0x4b1d1d),
        Kind::GitAdded => text(0x2ea043),
        Kind::GitChanged => text(BLUE),
        Kind::GitRemoved => text(0xf85149),
        Kind::ScrollThumb => look(TEXT, 0x424242),
        Kind::SideTitle => text(0xbbbbbb),
        Kind::SideTitleFocused => text(0xe7e7e7),
        Kind::PanelTitle => text(MUTED),
        Kind::PanelTitleActive => text(0xe7e7e7).underline(BLUE),
        Kind::Toast => look(TEXT, WIDGET),
        Kind::ToastError => look(0xffffff, 0x5a1d1d),
        Kind::Frame => text(0x454545),
        Kind::WidgetFrame => look(0x5a5a5a, WIDGET),
        Kind::Badge => look(0xffffff, 0x616161),
        Kind::Activity => text(0x868686),
        Kind::ActivityActive => text(0xe7e7e7),
        Kind::ActivityMarker => text(BLUE),
        Kind::ActivityBadge => look(0xffffff, BLUE),
        Kind::Minimap => text(MUTED),
        Kind::MinimapView => look(MUTED, 0x2e2e2e),
        Kind::MinimapMatch => look(MUTED, 0x623315),
        Kind::ScrollMatch => text(0xd18616),
        Kind::Caret => look(EDITOR, 0xaeafad),
    }
}

fn surface(surface: Surface) -> u32 {
    match surface {
        Surface::Editor => EDITOR,
        Surface::Bar => BAR,
        Surface::Hover => HOVER,
        Surface::Sticky => 0x252526,
    }
}

/// Dark Modern's colours for icons, guides and brackets.
fn hue(hue: Hue) -> u32 {
    match hue {
        Hue::Blue => 0x519aba,
        Hue::Green => 0x8dc149,
        Hue::Yellow => 0xcbcb41,
        Hue::Orange => 0xdea584,
        Hue::Red => 0xcc3e44,
        Hue::Purple => 0xa074c4,
        Hue::Cyan => 0x519aba,
        Hue::Grey => 0x8a9aa8,
        Hue::Guide => 0x404040,
        Hue::GuideActive => 0x707070,
        Hue::TreeGuide => 0x585858,
        Hue::Bracket1 => 0xffd700,
        Hue::Bracket2 => 0xda70d6,
        Hue::Bracket3 => 0x179fff,
        Hue::Modified => 0xe2c08d,
        Hue::Untracked => 0x73c991,
        Hue::Deleted => 0xc74e39,
        Hue::Problem => 0xf88070,
        Hue::Warned => YELLOW,
    }
}

fn severity(severity: Severity) -> u32 {
    match severity {
        Severity::Error => RED,
        Severity::Warning => YELLOW,
        Severity::Info => 0x3794ff,
        Severity::Hint => 0x8b949e,
    }
}

/// Dark Modern's colours for code.
pub fn code(syntax: Syntax) -> Option<u32> {
    Some(match syntax {
        Syntax::Keyword => 0x569cd6,
        Syntax::String => 0xce9178,
        Syntax::Comment => 0x6a9955,
        Syntax::Function => 0xdcdcaa,
        Syntax::Type => 0x4ec9b0,
        Syntax::Constant => 0xb5cea8,
        Syntax::Attribute | Syntax::Property | Syntax::Variable => 0x9cdcfe,
        Syntax::Macro => 0xc586c0,
        Syntax::Escape => 0xd7ba7d,
        Syntax::Operator => 0xd4d4d4,
        Syntax::Plain | Syntax::Punctuation => return None,
    })
}

/// Whether code colours show on a cell, in 24-bit colour or not.
pub fn shows_code(kind: Kind, true_colour: bool) -> bool {
    match kind {
        Kind::Normal
        | Kind::Match
        | Kind::Selection
        | Kind::CodeBlock
        | Kind::CurrentLine
        | Kind::WordMatch
        | Kind::Minimap
        | Kind::MinimapView
        | Kind::MinimapMatch => true,
        // Diff lines are tinted in 24-bit colour, and coloured red and green
        // in the terminal's palette.
        Kind::DiffAdded | Kind::DiffRemoved => true_colour,
        _ => false,
    }
}

/// How a cell looks in Dark Modern: its kind, on its surface, with code
/// colours, a tint and a problem's underline.
pub fn dark_cell(cell: &Cell) -> Look {
    let mut l = dark(cell.style);
    if l.bg.is_none() {
        l.bg = Some(surface(cell.surface));
    }
    if shows_code(cell.style, true)
        && let Some(colour) = code(cell.syntax)
    {
        l.fg = Some(colour);
        l.italic |= cell.syntax == Syntax::Comment;
    }
    // The minimap's dots are dimmer than the code they stand for.
    if matches!(
        cell.style,
        Kind::Minimap | Kind::MinimapView | Kind::MinimapMatch
    ) && let (Some(fg), Some(bg)) = (l.fg, l.bg)
    {
        l.fg = Some(mix(fg, bg, 0.6));
        l.italic = false;
    }
    if let Some(h) = cell.tint {
        l.fg = Some(hue(h));
    }
    if let Some(s) = cell.mark {
        l.underline = true;
        l.curly = true;
        l.underline_colour = Some(severity(s));
    }
    l
}

/// `a` with `t` of it kept and the rest `b`, channel by channel.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    [16, 8, 0].iter().fold(0, |out, shift| {
        let (x, y) = ((a >> shift) & 0xff, (b >> shift) & 0xff);
        let c = (x as f32 * t + y as f32 * (1.0 - t)).round() as u32;
        out | (c << shift)
    })
}

impl Look {
    pub fn css(self) -> String {
        let mut css = String::new();
        if let Some(fg) = self.fg {
            css.push_str(&format!("color:#{fg:06x};"));
        }
        if let Some(bg) = self.bg {
            css.push_str(&format!("background:#{bg:06x};"));
        }
        if self.bold {
            css.push_str("font-weight:bold;");
        }
        if self.italic {
            css.push_str("font-style:italic;");
        }
        if self.underline {
            let wavy = if self.curly { "wavy " } else { "" };
            let colour = self
                .underline_colour
                .map(|c| format!(" #{c:06x}"))
                .unwrap_or_default();
            css.push_str(&format!("text-decoration:underline {wavy}{colour};"));
        }
        css
    }
}
