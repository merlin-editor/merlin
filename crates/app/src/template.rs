//! `merlin plugin new <name>`: a working plugin to start from.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub fn create(name: &str) -> Result<()> {
    let valid = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
    if name.is_empty()
        || !name.starts_with(|c: char| c.is_ascii_lowercase())
        || !name.chars().all(valid)
    {
        bail!("a plugin name is lowercase letters, digits, - and _, starting with a letter");
    }
    let dir = PathBuf::from(name);
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    // The SDK isn't published yet, so plugins use the copy next to the
    // editor's source.
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/rust");
    let sdk = sdk
        .canonicalize()
        .with_context(|| format!("the plugin SDK should be at {}", sdk.display()))?;
    let crate_name = name.replace('-', "_");
    let title: String = name
        .split(['-', '_'])
        .map(|w| w[..1].to_uppercase() + &w[1..])
        .collect::<Vec<_>>()
        .join(" ");
    let type_name = title.replace(' ', "");

    let files = [
        (
            "Cargo.toml",
            format!(
                r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
merlin-plugin = {{ path = "{sdk}" }}

[profile.release]
opt-level = "s"
strip = true

[workspace]
"#,
                sdk = sdk.display()
            ),
        ),
        (
            ".cargo/config.toml",
            "# Plugins are WebAssembly components.\n[build]\ntarget = \"wasm32-wasip2\"\n".into(),
        ),
        (".gitignore", "/target\n".into()),
        (
            "plugin.toml",
            format!(
                r#"name = "{name}"
title = "{title}"
api = "{api}"
component = "target/wasm32-wasip2/release/{crate_name}.wasm"

# What the plugin may do. merlin asks you before granting these.
[permissions]
editor-read = true
# editor-write = true
# Programs it may start. They run with your rights, outside the sandbox.
# processes = ["git"]
"#,
                api = merlin_plugins::API_VERSION
            ),
        ),
        (
            "src/lib.rs",
            format!(
                r#"//! {title}: a merlin plugin.

use merlin_plugin::prelude::*;

#[derive(Default)]
struct {type_name} {{
    greetings: u32,
}}

impl Plugin for {type_name} {{
    fn commands(&self) -> Vec<Command> {{
        vec![Command::new("hello", "{title}: Say Hello")]
    }}

    fn run_command(&mut self, _id: &str) {{
        self.greetings += 1;
        host::notify("Hello from {name}!");
        host::focus_panel();
    }}

    fn panel(&mut self) -> Option<Panel> {{
        let words = host::active_text().map_or(0, |text| text.split_whitespace().count());
        let panel = Panel::new("{title}")
            .heading("Hello!")
            .text(format!("The open file has {{words}} words."))
            .text(format!("You said hello {{}} times.", self.greetings))
            .button("hello", "Say hello");
        Some(panel)
    }}

    fn click(&mut self, button: &str) {{
        self.run_command(button);
    }}

    // Keeps the count when you rebuild while merlin runs.
    fn save_state(&self) -> Option<Vec<u8>> {{
        Some(self.greetings.to_le_bytes().to_vec())
    }}

    fn restore_state(&mut self, state: Vec<u8>) {{
        self.greetings = state.try_into().map(u32::from_le_bytes).unwrap_or(0);
    }}
}}

merlin_plugin::export!({type_name});
"#
            ),
        ),
    ];
    for (path, text) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    println!(
        "Created {name}/. Next:\n\n  \
         rustup target add wasm32-wasip2   # once\n  \
         cd {name} && cargo build --release\n  \
         merlin --plugin {name} some-file.txt\n\n\
         Then edit src/lib.rs and build again: merlin reloads the plugin."
    );
    Ok(())
}
