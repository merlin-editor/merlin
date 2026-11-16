//! A language server as a language plugin describes it: the program, its
//! arguments and the files it serves.

/// A language server, and the files it serves.
pub struct Spec {
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// Extensions, or whole file names such as `CMakeLists.txt`, and the
    /// LSP language id of each.
    pub files: &'static [(&'static str, &'static str)],
    /// Optional JSON settings sent as `initializationOptions` and returned
    /// for `workspace/configuration` requests.
    pub settings: Option<&'static str>,
    /// The plugin's settings the server gets: each setting's key, and
    /// where it goes in the JSON settings, dotted, as `check.command`. A
    /// change to one restarts the server.
    pub options: &'static [(&'static str, &'static str)],
    /// The plugin's settings that become command-line flags: each
    /// setting's key, and the flag. Text and numbers go after the flag, as
    /// `--query-driver=` gives `--query-driver=/usr/bin/gcc`; a list goes
    /// after it joined by commas; a switch that's on adds the flag alone.
    /// A change to one restarts the server.
    pub flags: &'static [(&'static str, &'static str)],
    /// Whether it's told merlin takes snippets in completions, for a
    /// server that completes only then, as neocmakelsp does. merlin
    /// inserts a snippet as plain text, its placeholders' words kept,
    /// without tab stops.
    pub snippets: bool,
    /// Whether the language doesn't mind case, as CMake's commands don't:
    /// a name offered in two cases is listed once.
    pub case_blind: bool,
    /// How to install it, for the language card when it isn't. The
    /// manifest's `[programs]` entry wins, as it can differ per system.
    pub install: Option<&'static str>,
    /// Keys of values other plugins share that the server is set up from:
    /// a change to one restarts it. They must be in the manifest's
    /// `shared.watches`.
    pub shared: &'static [&'static str],
    /// The plugin's settings `adjust` reads: a change to one restarts the
    /// server, as for `options` and `flags`.
    pub restarts: &'static [&'static str],
    /// Has the last say over the JSON settings, after `settings` and
    /// `options`: to put in what a shared value says, say. It runs each
    /// time the server starts.
    pub adjust: Option<fn(&mut serde_json::Value)>,
}
