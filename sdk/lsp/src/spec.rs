//! A language server as a language plugin describes it: the program, its
//! arguments and the files it serves.

/// A language server, and the files it serves.
pub struct Spec {
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// Extensions and the LSP language id of each.
    pub files: &'static [(&'static str, &'static str)],
    /// Optional JSON settings sent as `initializationOptions` and returned
    /// for `workspace/configuration` requests.
    pub settings: Option<&'static str>,
    /// The plugin's settings the server gets: each setting's key, and
    /// where it goes in the JSON settings, dotted, as `check.command`. A
    /// change to one restarts the server.
    pub options: &'static [(&'static str, &'static str)],
    /// How to install it, for the language card when it isn't.
    pub install: Option<&'static str>,
}
