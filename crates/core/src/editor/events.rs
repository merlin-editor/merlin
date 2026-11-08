//! What the editor and the app say to each other: the events the editor
//! takes, the effects it asks for, and the types they carry.

use super::*;

/// A plugin command. Its id is local to the plugin; the editor runs it as
/// `<plugin>.<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub title: String,
    /// A key that runs it, such as `ctrl+alt+w`.
    pub key: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Drag,
    Up,
    /// The mouse moved with no button down.
    Move,
    /// A right click, for the menu there.
    Context,
    /// The mouse has rested a while, for a hover.
    Rest,
    /// The middle button: the cursor goes there, and what's selected,
    /// in any app, is pasted, as on Linux.
    Middle,
}

/// The mouse over an editor group's text, as a window measures it: rows
/// down from the top of the view, parts of a row scrolled off included,
/// and columns from the group's left edge, the gutter's included. Drags
/// and the release that ends them are in the group they began in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointer {
    pub group: usize,
    pub kind: MouseKind,
    pub row: f32,
    pub col: f32,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    /// 2 for a double click and 3 for a triple click, on presses; else 1.
    pub clicks: u8,
}

/// A [`Pointer`] in cells of its group's grid, as the text's own code
/// takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mouse {
    pub kind: MouseKind,
    pub x: u16,
    pub y: u16,
    pub shift: bool,
    /// Alt+click adds a cursor.
    pub alt: bool,
    /// Ctrl+click goes to the definition of what's clicked.
    pub ctrl: bool,
    /// 2 for a double click and 3 for a triple click, on presses; else 1.
    pub clicks: u8,
}

#[derive(Clone, Debug)]
pub enum Event {
    Key(Key),
    Paste(String),
    /// A click on the frame of a window that draws its own: a tab, a
    /// button, a row in a side bar.
    Ui(UiEvent),
    Resize(u16, u16),
    /// Each editor group's text area, in columns and rows of the code's
    /// font, as the window has laid them out.
    Viewports(Vec<(u16, u16)>),
    Pointer(Pointer),
    /// Scrolls a group's text by rows, parts of one too, and columns.
    Scroll {
        group: usize,
        rows: f32,
        cols: f32,
    },
    /// Puts a group's top at `row` rows from the top of its file, as the
    /// scrollbar and minimap do.
    ScrollTo {
        group: usize,
        row: f64,
    },
    Command(String),
    Saved {
        doc: u64,
        revision: Revision,
        path: PathBuf,
    },
    SaveFailed(String),
    /// Text an input method is composing, not yet typed, and where its
    /// cursor is in it; empty once it's typed or given up.
    Preedit {
        text: String,
        cursor: Option<usize>,
    },
    /// The file or folder picked for [`Effect::PickFile`], or `None` if
    /// the dialog was cancelled.
    Picked {
        pick: FilePick,
        path: Option<PathBuf>,
    },
    /// A file asked for with [`Effect::Open`].
    Opened {
        path: PathBuf,
        text: String,
    },
    /// What `.editorconfig` files say about an open file.
    FileConfig {
        path: PathBuf,
        config: FileConfig,
    },
    /// Save changed files now, if the settings say to for this.
    AutoSave(AutoSaveWhy),
    /// A terminal's shell set its title.
    TermTitle {
        id: u64,
        title: String,
    },
    /// All a task's terminal printed, when it ends, before its
    /// [`Event::TermExited`].
    TaskOutput {
        id: u64,
        text: String,
    },
    /// A terminal's shell ended, with its exit code if it had one.
    TermExited {
        id: u64,
        code: Option<i32>,
    },
    /// Lays out the editor groups as last time, once their files are open.
    Restore(Workspace),
    /// The breakpoints and what F5 starts, kept with the workspace, when
    /// it isn't restored because files were named to open.
    RestoreBreakpoints(Workspace),
    /// The user's settings, with the workspace's over them.
    Settings(Settings),
    /// Colours for text the editor asked for with
    /// [`Editor::colour_requests`].
    TextColours {
        language: String,
        text: String,
        tokens: Vec<PluginToken>,
    },
    /// The project's files, asked for with [`Effect::ListFiles`].
    Files(Vec<String>),
    /// The system clipboard, asked for with [`Effect::ReadClipboard`].
    /// `None` if it can't be read; the editor then pastes what it last
    /// copied.
    Clipboard(Option<String>),
    Error(String),
    RegisterPlugin {
        plugin: String,
        commands: Vec<Command>,
    },
    PluginFailed {
        plugin: String,
        message: String,
    },
    SetPanel {
        plugin: String,
        panel: Option<Panel>,
    },
    FocusPanel(String),
    /// A plugin's views, as its manifest declares them; none once it's
    /// unloaded or off.
    RegisterViews {
        plugin: String,
        views: Vec<super::ViewSpec>,
    },
    /// A plugin's work under way, shown in the status bar.
    ProgressStart {
        plugin: String,
        id: String,
        title: String,
        cancellable: bool,
    },
    /// How it's going: a line, and how far in thousandths.
    ProgressReport {
        plugin: String,
        id: String,
        message: String,
        done: Option<u16>,
    },
    /// It's over; with no id, all of the plugin's are.
    ProgressEnd {
        plugin: String,
        id: Option<String>,
    },
    /// A plugin's commands in right-click menus; none once it's gone.
    RegisterMenus {
        plugin: String,
        items: Vec<super::MenuItem>,
    },
    /// A plugin's picture for its views' `Image` nodes, by its id; none
    /// forgets it.
    SetImage {
        plugin: String,
        id: String,
        image: Option<std::sync::Arc<crate::PluginImage>>,
    },
    /// A count on a view's icon or tab; 0 for none.
    ViewBadge {
        plugin: String,
        view: String,
        count: u32,
    },
    /// Shows a plugin's view where it lives, and focuses it.
    FocusView {
        plugin: String,
        view: String,
    },
    ShowPicker {
        plugin: String,
        id: String,
        title: String,
        items: Vec<PickItem>,
    },
    InsertText(String),
    Notify(String),
    /// A line for a plugin's channel of the Output panel.
    PluginLog {
        plugin: String,
        level: super::LogLevel,
        text: String,
    },
    /// A notice from a plugin. Its buttons' clicks go back to the plugin's
    /// `on-click`, and it says how to quieten that plugin's notices.
    PluginNotice {
        plugin: String,
        level: super::LogLevel,
        text: String,
        buttons: Vec<crate::panel::Button>,
    },
    /// Shows a channel of the Output panel: `merlin` or a plugin's name.
    ShowOutput(String),
    /// A plugin's diagnostics for a file, replacing its earlier ones.
    SetDiagnostics {
        plugin: String,
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
    /// Text to show next to the cursor until the next key.
    ShowHover(String),
    /// Languages a plugin adds.
    RegisterLanguages {
        plugin: String,
        languages: Vec<Language>,
    },
    /// How a plugin reads problems in tasks' output.
    RegisterProblemMatchers {
        plugin: String,
        matchers: Vec<crate::lang::ProblemMatcher>,
    },
    /// Colours for a file from a plugin, for lines `lines` (end excluded),
    /// or for all of it.
    SetTokens {
        path: PathBuf,
        layer: Layer,
        lines: Option<(u32, u32)>,
        tokens: Vec<PluginToken>,
    },
    /// The symbols in a file, from a plugin.
    SetSymbols {
        path: PathBuf,
        symbols: Vec<PluginSymbol>,
    },
    /// Where a file's blocks fold, as first and last lines, from a plugin.
    SetFolds {
        path: PathBuf,
        ranges: Vec<(u32, u32)>,
    },
    /// A plugin's lines for the language card, replacing its earlier ones
    /// from the same set.
    SetStatus {
        plugin: String,
        set: String,
        lines: Vec<crate::lang::StatusLine>,
    },
    /// A plugin's tasks for Run Task, replacing its earlier ones from the
    /// same set.
    SetTasks {
        plugin: String,
        set: String,
        tasks: Vec<Task>,
    },
    /// A plugin's tests in a file, replacing its earlier ones there: the
    /// Tests view lists them under `group`, such as a crate, as `label`.
    SetTests {
        plugin: String,
        path: PathBuf,
        group: String,
        label: String,
        tests: Vec<PluginTest>,
    },
    /// How a plugin's tests went.
    SetTestResults {
        plugin: String,
        results: Vec<PluginTestResult>,
    },
    /// A plugin's group of tests started or ended, and how long it took.
    SetTestRun {
        plugin: String,
        group: String,
        running: bool,
        seconds: Option<f64>,
    },
    /// A plugin runs a command line in a terminal tab, and hears what it
    /// printed as [`Effect::PluginTerminal`] with `id`.
    RunInTerminal {
        plugin: String,
        id: String,
        name: String,
        command: String,
    },
    /// A plugin starts debugging something, or runs `fallback` in a
    /// terminal without a debugger for its language.
    StartDebugging {
        plugin: String,
        target: Box<DebugTarget>,
        fallback: Option<String>,
    },
    /// Runs a task by name, as a plugin asks.
    RunTask(String),
    /// Keys under the leader a plugin's manifest declares.
    RegisterKeys {
        plugin: String,
        keys: Vec<LeaderKey>,
    },
    /// A pause on a prefix went on as long as
    /// [`Editor::which_key_wait`] asked: which-key shows.
    WhichKey(u64),
    /// A plugin's command that searches the project's symbols, from its
    /// manifest's `symbols`.
    RegisterSymbolSearch {
        plugin: String,
        command: String,
    },
    /// Symbols a plugin found for the palette's search for `query`.
    ProjectSymbols {
        query: String,
        symbols: Vec<crate::lang::ProjectSymbol>,
    },
    /// The palette's text search waited as long as
    /// [`Editor::palette_search_wait`] asked: it searches.
    PaletteSearch(u64),
    /// The pointer stayed on a name with Ctrl down as long as
    /// [`Editor::link_wait`] asked: the language server is asked about it.
    LinkDue(u64),
    /// A macro is playing: [`Editor::macro_playing`]. It plays on for a
    /// moment each time.
    MacroStep,
    /// Debuggers a plugin's manifest declares.
    RegisterDebuggers {
        plugin: String,
        debuggers: Vec<Debugger>,
    },
    /// Which debuggers' programs are installed, by name, asked for with
    /// [`Effect::LocateDebuggers`].
    DebuggersLocated(Vec<(String, bool)>),
    /// The user's processes, newest first, as pid, name and command line,
    /// and Linux's `ptrace_scope`, asked for with
    /// [`Effect::ListProcesses`].
    Processes {
        list: Vec<(u32, String, String)>,
        ptrace_scope: u8,
    },
    /// `.merlin/launch.toml`'s configurations, asked for with
    /// [`Effect::ReadLaunch`], or why it couldn't be read.
    LaunchConfigs(Result<Vec<DebugTarget>, String>),
    /// A plugin's things to debug, replacing its earlier ones.
    SetDebugTargets {
        plugin: String,
        targets: Vec<DebugTarget>,
    },
    /// How a run is going, from the plugin driving its debugger.
    Debug {
        plugin: String,
        update: DebugUpdate,
    },
    /// A file's inlay hints from a plugin, replacing its earlier ones.
    SetInlayHints {
        path: PathBuf,
        hints: Vec<crate::lang::InlayHint>,
    },
    /// Completions for the word at the cursor.
    ShowCompletions(Vec<Completion>),
    /// The signature of the call the cursor is in, or none to hide it.
    ShowSignature(Option<crate::lang::Signature>),
    /// The code actions at the cursor, answering the `code-actions`
    /// command.
    ShowCodeActions(Vec<crate::lang::CodeAction>),
    /// Changes to files from a plugin, such as a code action's.
    ApplyEdits(Vec<crate::lang::FileEdit>),
    /// What the name the rename box is over reaches, or why it can't be
    /// renamed.
    ShowRename(Result<crate::lang::RenameInfo, String>),
    /// A rename's edits, to review before they're made.
    ShowRenameReview(Vec<crate::lang::FileEdit>),
    /// Where the name at the cursor is used, for the peek view.
    ShowReferences(Vec<crate::lang::Location>),
    /// Where the name under Ctrl is defined, from `definition-link`.
    Definitions(Vec<crate::lang::Location>),
    /// Files read for a rename's review, each with its text, or none if
    /// it couldn't be read.
    FilesRead(Vec<(PathBuf, Option<String>)>),
    /// A save has waited [`format::FORMAT_WAIT_MS`] for its formatting: it saves
    /// as it is.
    FormatTimedOut,
    /// Opens a file at a line and UTF-16 column, as go-to-definition does.
    OpenAt {
        path: PathBuf,
        line: u32,
        column: u32,
    },
    /// A plugin proposes changing a file from `old` to `new`, and asks the
    /// user to review it. The answer goes back as [`Effect::PluginReview`].
    ShowReview {
        plugin: String,
        id: String,
        path: PathBuf,
        old: String,
        new: String,
    },
    /// The plugin got its answer some other way; the review closes.
    CloseReview {
        plugin: String,
        id: String,
    },
    /// Unsaved changes kept from the last run: the file's text as it was,
    /// or an untitled file's with no path. The file opens with them,
    /// unsaved. `moved`: the file on disk changed since.
    Recovered {
        path: Option<PathBuf>,
        text: String,
        moved: bool,
    },
    /// Every kept file is back. `crashed`: the last run didn't quit, so
    /// they're offered to be dropped.
    RecoveredAll {
        crashed: bool,
    },
    /// An open file changed on disk. It reloads unless it has unsaved
    /// changes.
    FileChanged {
        path: PathBuf,
        text: String,
    },
    /// A file's text as of the last commit, or `None` if git doesn't track
    /// it. Lines that differ get signs in the gutter.
    GitBase {
        path: PathBuf,
        text: Option<String>,
    },
    /// The project's files for the Explorer, asked for with
    /// [`Effect::ListTree`]. Folders end in `/`, so empty ones show too.
    Tree(Vec<String>),
    /// A file or folder was renamed, by the host after
    /// [`Effect::File`].
    Renamed {
        from: PathBuf,
        to: PathBuf,
    },
    /// A file or folder went to the trash.
    Trashed(PathBuf),
    /// The plugins loaded, for the Plugins view.
    PluginList(Vec<PluginSummary>),
    /// The colour or icon themes there are, by name, and the one in use,
    /// for the theme pickers.
    Themes {
        kind: ThemeKind,
        themes: Vec<String>,
        current: String,
    },
    /// What a project search found, asked for with [`Effect::Search`].
    SearchResults {
        query: String,
        hits: Vec<SearchHit>,
    },
    /// The branch and changed files, asked for with [`Effect::GitStatus`].
    /// No branch means the project isn't in a git repository.
    GitStatus {
        branch: Option<String>,
        files: Vec<(String, char)>,
        /// Files with changes in the index, and with changes not yet staged,
        /// with their letters.
        staged: Vec<(String, char)>,
        changed: Vec<(String, char)>,
    },
    /// Where the branch stands with its upstream, sent with
    /// [`Event::GitStatus`].
    GitSync(SyncState),
    /// Files moved, sent with [`Event::GitStatus`].
    GitMoves(Vec<GitMove>),
    /// The merge, rebase or cherry-pick git is in the middle of, if any,
    /// sent with [`Event::GitStatus`].
    GitMerge(Option<MergeState>),
    /// A conflicted file's sides, asked for with [`Effect::MergeSides`].
    MergeSides {
        path: String,
        /// The version both sides came from, for each conflict's base.
        base: String,
        /// What each side is, as "themes · a41c9e2".
        incoming: String,
        current: String,
    },
    /// A file's changes git can't show as text: a binary file's sizes, in
    /// bytes, before and now.
    ShowBinary {
        path: PathBuf,
        old: u64,
        new: u64,
    },
    /// A push, pull, sync, publish or fetch finished: how, and what git
    /// said, for the Output panel.
    Synced {
        kind: SyncKind,
        outcome: SyncOutcome,
        output: String,
    },
    /// Who last changed each line of a file, as of the text at `version`,
    /// asked for with [`Effect::Blame`]; `now` is the time, for "3 days
    /// ago".
    Blame {
        path: PathBuf,
        version: u64,
        blame: crate::git::Blame,
        now: i64,
    },
    /// A commit's message past its summary, and lines added and removed in
    /// how many files.
    CommitInfo {
        hash: String,
        info: blame::CommitInfo,
    },
    /// Every changed file's hunks, staged and not, sent after
    /// [`Event::GitStatus`].
    GitDiffs {
        staged: Vec<crate::git::FileDiff>,
        changed: Vec<crate::git::FileDiff>,
    },
    /// A file's changes went back to the last commit, after
    /// [`GitAction::Discard`]: all of them, or one. Its text before, for
    /// Undo.
    Discarded {
        path: String,
        before: String,
        all: bool,
    },
    /// A file's changes since the last commit, to look at, asked for with
    /// [`Effect::OpenDiff`].
    ShowDiff {
        path: PathBuf,
        old: String,
        new: String,
    },
}

/// The Settings tab's page name.
pub const SETTINGS_PAGE: &str = "merlin.settings";

/// A plugin as the Plugins view lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSummary {
    pub name: String,
    pub title: String,
    /// What it adds, in a few words.
    pub what: String,
    /// How it runs: "runs programs", "sandboxed", "data only", "off".
    pub tag: String,
    pub tone: crate::panel::Tone,
    /// Its tile's letters and colour: [`plugin_mark`].
    pub mark: (String, crate::style::Hue),
}

/// A plugin's tile: the letters and colour its manifest gives, or its
/// title's first letters, in purple for a plugin of looks and blue for
/// others.
pub fn plugin_mark(
    title: &str,
    badge: Option<&str>,
    hue: Option<&str>,
    looks: bool,
) -> (String, crate::style::Hue) {
    use crate::style::Hue;
    let letters = match badge.map(str::trim).filter(|b| !b.is_empty()) {
        Some(badge) => badge.chars().take(2).collect(),
        None => {
            let mut letters: String = title
                .split_whitespace()
                .filter_map(|w| w.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();
            if letters.chars().count() < 2 {
                letters = title.chars().take(2).collect::<String>().to_uppercase();
            }
            letters
        }
    };
    let hue = hue
        .and_then(Hue::named)
        .unwrap_or(if looks { Hue::Purple } else { Hue::Blue });
    (letters, hue)
}

/// A command and the keys that run it, for the Keys settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub command: String,
    pub title: String,
    /// Each key as settings write it (`ctrl+k`) and as people read it
    /// (`Ctrl+K`).
    pub keys: Vec<(String, String)>,
    /// The settings change its keys.
    pub changed: bool,
    /// The keys it has without the settings, as settings write them.
    pub defaults: Vec<String>,
    /// The plugin it's from, by title.
    pub plugin: Option<String>,
}

/// What a theme picker picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    Colours,
    Icons,
}

/// Work the editor asks its host to do. Results come back as events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The user changed a setting from a menu, such as View › Minimap: save
    /// it in their settings, as a TOML value.
    SetSetting {
        key: String,
        value: String,
    },
    /// Run a plugin's symbol search for the palette, with `query` for its
    /// `symbol-query`.
    SearchSymbols {
        plugin: String,
        command: String,
        query: String,
    },
    /// Save a macro in the user's settings, as `[[macros]]`, in place of
    /// one with its name.
    SaveMacro(Macro),
    /// Open the user's settings file, or the workspace's.
    OpenSettings {
        workspace: bool,
    },
    /// Show a colour or icon theme from a theme picker while it's chosen
    /// from, without keeping it.
    PreviewTheme(ThemeKind, String),
    /// Use a theme from now on, and save it in the user's settings.
    SetTheme(ThemeKind, String),
    /// The Settings tab opened: the window draws it. `plugin` is the
    /// plugin whose page to show; `plugins` opens the list of them.
    OpenSettingsWindow {
        plugin: Option<String>,
        plugins: bool,
    },
    /// Create, rename or trash a file or folder in the project, then send
    /// what changed and list the tree again.
    File(FileOp),
    /// Make the code's font bigger or smaller by `n` steps, or with 0 go
    /// back to its size.
    Zoom(i8),
    /// Something for the host to do with a terminal.
    Terminal(TermOp),
    /// Read files, project paths, and send [`Event::FilesRead`].
    ReadFiles(Vec<PathBuf>),
    Save {
        doc: u64,
        path: PathBuf,
        text: String,
        revision: Revision,
    },
    Open(PathBuf),
    ListFiles,
    Copy(String),
    /// Open a web page in the browser.
    OpenUrl(String),
    ReadClipboard,
    Quit,
    /// Asks the system for a file or folder: see [`FilePick`].
    PickFile(FilePick),
    /// Opens a folder in a new window, as this one quits.
    OpenFolder(PathBuf),
    /// What's selected now, for the selection clipboard a middle click
    /// pastes.
    SetSelection(String),
    /// Asks for the selection clipboard's text, to paste as
    /// [`Event::Paste`].
    ReadSelection,
    PluginCommand {
        plugin: String,
        command: String,
    },
    /// A command line a plugin ran in a terminal, or one of its tests',
    /// ended: all it printed, and its exit code.
    PluginTerminal {
        plugin: String,
        id: String,
        output: String,
        code: Option<i32>,
    },
    /// A command for the plugin driving a debugger, such as `debug.start`
    /// or `debug.step-over`, with what it reads: what to start, for
    /// `debug.start`, and the breakpoints.
    Debug {
        plugin: String,
        command: String,
        start: Option<Box<DebugStart>>,
        breakpoints: Vec<Breakpoint>,
    },
    /// Look for these programs on `PATH`, and answer with
    /// [`Event::DebuggersLocated`].
    LocateDebuggers(Vec<String>),
    /// Read `.merlin/launch.toml`, and answer with [`Event::LaunchConfigs`].
    ReadLaunch,
    /// List the user's processes, to attach to, and answer with
    /// [`Event::Processes`].
    ListProcesses,
    PluginClick {
        plugin: String,
        button: String,
    },
    /// The user cancelled a plugin's work under way.
    PluginCancel {
        plugin: String,
        id: String,
    },
    PluginSubmit {
        plugin: String,
        input: String,
        text: String,
    },
    PluginPick {
        plugin: String,
        picker: String,
        item: String,
    },
    /// The user accepted or rejected a change shown with
    /// [`Event::ShowReview`].
    PluginReview {
        plugin: String,
        id: String,
        accepted: bool,
    },
    ListTree,
    /// Search the project's files for this text.
    Search(String),
    GitStatus,
    OpenDiff(PathBuf),
    /// A conflicted file's sides, for the merge editor: answered with
    /// [`Event::MergeSides`].
    MergeSides(String),
    /// A moved file's changes, from its old path as of the last commit.
    OpenMove {
        from: PathBuf,
        to: PathBuf,
    },
    /// Stage, unstage or commit; the frontend answers with
    /// [`Event::GitStatus`].
    Git(GitAction),
    /// A commit's whole message and how big it is, for its card; answered
    /// with [`Event::CommitInfo`].
    CommitInfo(String),
    /// A commit's changes to a file, to look at; answered with
    /// [`Event::ShowReview`] for `merlin.commit`.
    OpenCommit {
        hash: String,
        path: PathBuf,
    },
    /// Who last changed each line of `text`, the file at `path` as it is
    /// now, leaving out the commits in `ignore`; answered, after a pause
    /// in asking, with [`Event::Blame`].
    Blame {
        path: PathBuf,
        version: u64,
        text: String,
        ignore: Vec<String>,
    },
}

/// A file moved since the last commit: staged as a rename, or a deleted
/// file and a new one alike enough to be one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitMove {
    pub from: String,
    pub to: String,
    pub staged: bool,
    /// How alike they are, in percent, when it was worked out.
    pub similarity: Option<u8>,
}

/// A merge, rebase or cherry-pick git is in the middle of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeState {
    pub kind: MergeKind,
    /// What's coming in: a branch, or a commit's summary.
    pub from: String,
    /// The commit message git has ready, for a merge.
    pub message: String,
    /// Files with conflicts, and how many each has left.
    pub conflicts: Vec<(String, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeKind {
    Merge,
    Rebase,
    CherryPick,
}

/// Where a branch stands with its upstream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncState {
    /// The branch it pushes to and pulls from, as `origin/main`; `None`
    /// when it has none yet.
    pub upstream: Option<String>,
    /// Commits to push, and to pull.
    pub ahead: usize,
    pub behind: usize,
    /// The remote a branch with no upstream would be published to.
    pub remote: Option<String>,
}

/// What a sync button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncKind {
    Push,
    Pull,
    /// Pull, then push.
    Sync,
    /// Push a branch with no upstream, and set it.
    Publish,
    /// Ask the remote what's new, quietly.
    Fetch,
}

/// How a push, pull or fetch went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    Done,
    /// The remote has commits this branch doesn't.
    Rejected,
    /// The remote wants a password or a key, which git isn't asked for.
    NeedsAuth {
        remote: String,
    },
    /// A pull stopped on conflicts, in these files.
    Conflicts(Vec<String>),
    Cancelled,
    /// Git's first line about it.
    Failed(String),
}

/// What Source Control asks git to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitAction {
    /// Push, pull, sync, publish or fetch; answered with [`Event::Synced`].
    Sync(SyncKind),
    /// Stops the push or pull running.
    CancelSync,
    /// Undoes a pull, merge, rebase or cherry-pick that stopped on
    /// conflicts.
    AbortPull,
    /// Goes on with a rebase or cherry-pick once its conflicts are
    /// resolved.
    Continue,
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Commit(String),
    /// Stages a moved file's old and new paths together.
    StageMove {
        from: String,
        to: String,
    },
    /// Adds a file to the project's `.gitignore`.
    Ignore(String),
    /// `git apply` a patch from [`crate::git::patch`]: to the index when
    /// `cached`, and backwards when `reverse`.
    Apply {
        patch: String,
        cached: bool,
        reverse: bool,
    },
    /// Puts a file's changes back as they are in the index: the patch's
    /// lines when given, else all of them. Answered with
    /// [`Event::Discarded`].
    Discard {
        path: String,
        patch: Option<String>,
    },
    /// Writes a discarded file's text back.
    Undiscard {
        path: String,
        text: String,
    },
}

/// Exactly one part of the UI has focus, and only it gets keys and pastes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Find,
    Palette,
    /// Plugin panels, on the right.
    Panel,
    /// The built-in views, on the left.
    SideBar,
    /// The panel under the editor: Problems and Output.
    Bottom,
    Menu,
}

/// Where panels go: built-in views in the side bar on the left, plugin
/// panels on the right, and Problems and Output in the panel at the bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Bottom,
    /// A plugin's view open in an editor tab.
    Tab,
}
