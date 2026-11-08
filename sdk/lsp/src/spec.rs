//! Language server definitions and low-footprint default arguments.

/// A language server, and the files it serves.
pub struct Spec {
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// Extensions and the LSP language id of each.
    pub files: &'static [(&'static str, &'static str)],
    /// Optional JSON settings sent as `initializationOptions` and returned
    /// for `workspace/configuration` requests.
    pub settings: Option<&'static str>,
}

pub const RUST_SERVER: Spec = Spec {
    program: "rust-analyzer",
    args: &[],
    files: &[("rs", "rust")],
    settings: None,
};

pub const GO_SERVER: Spec = Spec {
    program: "gopls",
    args: &[],
    files: &[("go", "go")],
    settings: None,
};

/// clangd configured for low memory and CPU overhead: precompiled headers on
/// disk instead of in RAM, low-priority background indexing, and glibc heap
/// trimming after requests.
pub const CPP_SERVER: Spec = Spec {
    program: "clangd",
    args: &[
        "--pch-storage=disk",
        "--background-index-priority=low",
        "--malloc-trim",
        "-j=2",
    ],
    files: &[
        ("c", "c"),
        ("h", "c"),
        ("cc", "cpp"),
        ("cpp", "cpp"),
        ("cxx", "cpp"),
        ("c++", "cpp"),
        ("hpp", "cpp"),
        ("hh", "cpp"),
        ("hxx", "cpp"),
        ("h++", "cpp"),
        ("ipp", "cpp"),
        ("inl", "cpp"),
        ("tpp", "cpp"),
    ],
    settings: None,
};

/// pyright configured to analyze open files only, avoiding whole-workspace
/// and virtualenv background indexing spikes.
pub const PYTHON_SERVER: Spec = Spec {
    program: "pyright-langserver",
    args: &["--stdio"],
    files: &[("py", "python"), ("pyi", "python")],
    settings: Some(r#"{"python":{"analysis":{"diagnosticMode":"openFilesOnly"}}}"#),
};

/// typescript-language-server with a bounded tsserver heap.
pub const JS_SERVER: Spec = Spec {
    program: "typescript-language-server",
    args: &["--stdio"],
    files: &[
        ("ts", "typescript"),
        ("tsx", "typescriptreact"),
        ("js", "javascript"),
        ("mjs", "javascript"),
        ("cjs", "javascript"),
        ("jsx", "javascriptreact"),
    ],
    settings: Some(r#"{"tsserver":{"maxTsServerMemory":512}}"#),
};

pub const SERVERS: &[Spec] = &[
    RUST_SERVER,
    GO_SERVER,
    CPP_SERVER,
    PYTHON_SERVER,
    JS_SERVER,
];
