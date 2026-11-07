//! What text on screen means: its style, the surface it's on and colours
//! over it, for the window's theme to colour; and how many columns text
//! takes.

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
    /// What Vim's `:s` takes away, struck through while it's typed.
    Replaced,
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
    /// The words that changed in those lines.
    DiffAddedWord,
    DiffRemovedWord,
    /// Lines of a conflict in the merge editor's Result: from Incoming,
    /// from Current, or its base, not chosen yet.
    MergeIncoming,
    MergeCurrent,
    MergeBase,
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
    /// Search matches on the scrollbar.
    ScrollMatch,
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
    /// A name drawn as a link, underlined: Ctrl over it goes to its
    /// definition.
    Link,
}

/// Columns a character takes: 0 for combining marks, 2 for wide ones.
/// Control characters count as 1.
pub fn char_width(c: char) -> usize {
    c.width().unwrap_or(1)
}

pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// What a piece of text is, for its colours: one character's style,
/// syntax, diagnostic, surface and tint. The window's theme turns it into
/// colours.
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
