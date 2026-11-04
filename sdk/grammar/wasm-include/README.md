The C library headers tree-sitter needs to build for WebAssembly, from the
`tree-sitter-language` crate (MIT, see LICENSE). Language plugins build
their grammars against these instead of a system C library; Rust's
`wasm32-wasip2` target supplies the functions behind them.
