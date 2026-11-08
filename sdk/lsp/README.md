# merlin-lsp

Language Server Protocol (LSP) client support for [Merlin](https://github.com/merlin-editor/merlin) language plugins.

Builds on top of `merlin-plugin` to provide diagnostics, completions, hover, formatting, go-to-definition, and symbol searches by driving language server processes through Merlin's process API.

## Usage

```toml
[dependencies]
merlin-plugin = "0.9.0"
merlin-lsp = "0.9.0"
```
