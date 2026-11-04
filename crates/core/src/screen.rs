use merlin_syntax::Syntax;
use unicode_width::UnicodeWidthChar;

use crate::lang::Severity;

/// What a cell means, not how it looks. The frontend picks the colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Normal,
    Gutter,
    GutterActive,
    /// Line numbers of lines with errors or warnings.
    GutterError,
    GutterWarning,
    Muted,
    Match,
    MatchCurrent,
    Selection,
    Border,
    Heading,
    Button,
    ButtonFocused,
    Prompt,
    Status,
    StatusError,
    Selected,
    Warning,
    Error,
    Input,
    InputPlaceholder,
    Tab,
    TabActive,
    /// A preview tab's name, in italics.
    TabPreview,
    TabPreviewActive,
    /// Hover and completion popups.
    Popup,
    PopupMuted,
    /// Markdown in panels.
    Emphasis,
    Code,
    CodeBlock,
    Link,
    /// Lines in a review of proposed changes.
    DiffAdded,
    DiffRemoved,
    /// Gutter signs for lines changed since the last commit.
    GitAdded,
    GitChanged,
    GitRemoved,
    /// Text in a panel that reads as good news, such as a new file.
    Success,
    /// The line with the cursor, under the text.
    CurrentLine,
    ScrollTrack,
    ScrollThumb,
    TabBar,
    MenuBar,
    /// A menu bar title whose menu is open.
    MenuOpen,
    /// The window title in the middle of the menu bar.
    MenuTitle,
    Menu,
    MenuSelected,
    MenuMuted,
    /// A side bar's title for the panel shown.
    SideTitle,
    SideTitleFocused,
    /// Floating boxes: find and the palette.
    Widget,
    WidgetInput,
    WidgetMuted,
    Toast,
    ToastError,
    /// Lines around panes and tabs.
    Frame,
    /// Lines around popups: menus, the palette, hover, find.
    WidgetFrame,
    /// Other uses of the word under the cursor.
    WordMatch,
    /// A count in a pill, such as problems in a file.
    Badge,
    /// The activity bar's icons, and the one for the view shown.
    Activity,
    ActivityActive,
    /// The bar beside the active icon, and the count on Source Control.
    ActivityMarker,
    ActivityBadge,
    /// The search box in the middle of the title bar.
    CommandCenter,
    /// The bottom panel's tab titles.
    PanelTitle,
    PanelTitleActive,
    /// A coloured item at the left of the status bar, for what needs doing.
    StatusAccent,
    /// The minimap's dots, coloured like code but dimmer, and the rows
    /// for the lines in view.
    Minimap,
    MinimapView,
    /// Cells of the minimap that hold a search match.
    MinimapMatch,
    /// Search matches on the scrollbar.
    ScrollMatch,
    /// A cursor other than the terminal's own, when there are several.
    Caret,
}

/// The part of the window a cell is in. Text styles without a background
/// of their own show the part's, as VS Code shades its parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Editor,
    /// Side bars, the panel, the title and tab bars.
    Bar,
    /// A row or button under the mouse.
    Hover,
    /// Sticky scroll: the lines that start the blocks in view, pinned at
    /// the top of the text.
    Sticky,
}

/// A text colour for one cell, over its style: file icons, bracket pairs,
/// indent guides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hue {
    Blue,
    Green,
    Yellow,
    Orange,
    Red,
    Purple,
    Cyan,
    Grey,
    Guide,
    GuideActive,
    /// Indent guides in trees, on the side bar's darker shade.
    TreeGuide,
    Bracket1,
    Bracket2,
    Bracket3,
    /// Names coloured by git or problems, as VS Code colours them.
    Modified,
    Untracked,
    Deleted,
    Problem,
    Warned,
}

/// The right half of a double-width character. Frontends draw nothing there.
pub const WIDE_TAIL: char = '\0';

/// Cells a character takes: 0 for combining marks, 2 for wide ones.
/// Control characters count as 1; [`Screen::put`] draws them as `�`.
pub fn char_width(c: char) -> usize {
    c.width().unwrap_or(1)
}

pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
    /// The colour of code, under the style.
    pub syntax: Syntax,
    /// A diagnostic, drawn as an underline.
    pub mark: Option<Severity>,
    pub surface: Surface,
    /// A text colour over the style's.
    pub tint: Option<Hue>,
}

/// A rendered frame: a grid of cells and where the cursor goes.
#[derive(Clone, Debug)]
pub struct Screen {
    pub width: u16,
    pub height: u16,
    cells: Vec<Cell>,
    pub cursor: Option<(u16, u16)>,
}

impl Screen {
    pub fn new(width: u16, height: u16) -> Self {
        let blank = Cell {
            ch: ' ',
            style: Style::Normal,
            syntax: Syntax::Plain,
            mark: None,
            surface: Surface::Editor,
            tint: None,
        };
        Self {
            width,
            height,
            cells: vec![blank; width as usize * height as usize],
            cursor: None,
        }
    }

    pub fn cell(&self, x: u16, y: u16) -> Cell {
        self.cells[y as usize * self.width as usize + x as usize]
    }

    fn index(&self, x: u16, y: u16) -> usize {
        y as usize * self.width as usize + x as usize
    }

    fn set(&mut self, x: u16, y: u16, ch: char, style: Style) {
        if x >= self.width || y >= self.height {
            return;
        }
        // Don't leave half of a wide character behind.
        let old = self.cell(x, y).ch;
        if old == WIDE_TAIL && x > 0 {
            let i = self.index(x - 1, y);
            self.cells[i].ch = ' ';
        } else if char_width(old) == 2 && x + 1 < self.width {
            let i = self.index(x + 1, y);
            self.cells[i].ch = ' ';
        }
        let i = self.index(x, y);
        self.cells[i] = Cell {
            ch,
            style,
            syntax: Syntax::Plain,
            mark: None,
            surface: self.cells[i].surface,
            tint: None,
        };
    }

    /// Colours code and underlines diagnostics on a cell already drawn.
    pub fn decorate(&mut self, x: u16, y: u16, syntax: Syntax, mark: Option<Severity>) {
        if x < self.width && y < self.height {
            let i = self.index(x, y);
            self.cells[i].syntax = syntax;
            self.cells[i].mark = mark;
        }
    }

    /// Sets the part of the window an area is in, and clears it.
    pub fn shade(&mut self, x: u16, y: u16, w: u16, h: u16, surface: Surface) {
        for row in y..(y + h).min(self.height) {
            for col in x..(x + w).min(self.width) {
                let i = self.index(col, row);
                self.cells[i] = Cell {
                    ch: ' ',
                    style: Style::Normal,
                    syntax: Syntax::Plain,
                    mark: None,
                    surface,
                    tint: None,
                };
            }
        }
    }

    /// Marks cells as under the mouse, keeping what's drawn on them.
    pub fn hover(&mut self, x: u16, y: u16, w: u16) {
        if y < self.height {
            for col in x..(x + w).min(self.width) {
                let i = self.index(col, y);
                self.cells[i].surface = Surface::Hover;
            }
        }
    }

    /// Colours a cell's text, over its style.
    pub fn tint(&mut self, x: u16, y: u16, hue: Hue) {
        if x < self.width && y < self.height {
            let i = self.index(x, y);
            self.cells[i].tint = Some(hue);
        }
    }

    /// Draws one character and returns the cells it took. A wide character
    /// that doesn't fit before the right edge is drawn as a space.
    pub fn put(&mut self, x: u16, y: u16, ch: char, style: Style) -> u16 {
        let ch = if ch.is_control() { '\u{FFFD}' } else { ch };
        match char_width(ch) {
            0 => 0,
            2 if x + 1 < self.width => {
                self.set(x, y, ch, style);
                self.set(x + 1, y, WIDE_TAIL, style);
                2
            }
            2 => {
                self.set(x, y, ' ', style);
                1
            }
            _ => {
                self.set(x, y, ch, style);
                1
            }
        }
    }

    /// Writes `text` from `x`, at most `max` cells. Returns the cells used.
    pub fn print(&mut self, x: u16, y: u16, text: &str, style: Style, max: u16) -> u16 {
        let mut used = 0;
        for ch in text.chars() {
            let w = char_width(ch) as u16;
            if used + w > max || x + used + w > self.width {
                break;
            }
            used += self.put(x + used, y, ch, style);
        }
        used
    }

    /// A box with rounded corners, its inside filled with `inside`.
    pub fn frame(&mut self, x: u16, y: u16, w: u16, h: u16, line: Style, inside: Style) {
        if w < 2 || h < 2 {
            return;
        }
        let (right, bottom) = (x + w - 1, y + h - 1);
        for row in y + 1..bottom {
            self.fill(x + 1, row, w - 2, inside);
            self.put(x, row, '│', line);
            self.put(right, row, '│', line);
        }
        for col in x + 1..right {
            self.put(col, y, '─', line);
            self.put(col, bottom, '─', line);
        }
        self.put(x, y, '╭', line);
        self.put(right, y, '╮', line);
        self.put(x, bottom, '╰', line);
        self.put(right, bottom, '╯', line);
    }

    pub fn fill(&mut self, x: u16, y: u16, width: u16, style: Style) {
        for i in 0..width {
            self.put(x + i, y, ' ', style);
        }
    }

    /// The frame as text, one line per row with trailing spaces trimmed.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for y in 0..self.height {
            let row: String = (0..self.width)
                .map(|x| self.cell(x, y).ch)
                .filter(|c| *c != WIDE_TAIL)
                .collect();
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }
}
