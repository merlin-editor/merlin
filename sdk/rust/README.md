# merlin-plugin

Rust SDK for writing WebAssembly plugins for the [Merlin](https://github.com/merlin-editor/merlin) code editor.

Plugins run inside a sandboxed WebAssembly component (`wasm32-wasip2`) and communicate with the editor using WebAssembly Interface Types (WIT).

## Usage

Add this to your plugin's `Cargo.toml`:

```toml
[dependencies]
merlin-plugin = "0.9.0"
```

Implement `Plugin` for your state struct and export it:

```rust
use merlin_plugin::prelude::*;

#[derive(Default)]
struct Hello;

impl Plugin for Hello {
    fn commands(&self) -> Vec<Command> {
        vec![Command::new("hi", "Hello: Say Hi").key("ctrl+alt+h")]
    }

    fn run_command(&mut self, _id: &str) {
        host::notify("Hi!");
    }
}

merlin_plugin::export!(Hello);
```

Build for `wasm32-wasip2`:

```sh
cargo build --release --target wasm32-wasip2
```
