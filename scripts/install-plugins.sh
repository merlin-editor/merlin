#!/bin/sh
# Builds the plugins that come with merlin and installs them, so they load
# at every start: the languages (colours and language servers for Rust,
# Python, JavaScript, JSON, TOML, C, C++ and Go), Catch2 tests, the
# debugger, PDFs, and merlin's themes, icons and layouts. As VS Code's
# built-in extensions are, they're plugins like any other. The examples,
# testbed and word-count, are left out.
#
#   scripts/install-plugins.sh [plugin...]    (all of them by default)
set -e
cd "$(dirname "$0")/.."
rustup target add wasm32-wasip2 >/dev/null 2>&1 || true
cargo build --release --bin merlin
plugins=${*:-rust python javascript json toml cpp go catch2 dap pdf merlin-themes merlin-icons merlin-layouts}
for name in $plugins; do
    # Theme plugins are data, with nothing to build.
    [ -f "plugins/$name/Cargo.toml" ] || continue
    echo "Building $name"
    (cd "plugins/$name" && cargo build --release --quiet)
done
dirs=""
for name in $plugins; do
    dirs="$dirs plugins/$name"
done
# shellcheck disable=SC2086
target/release/merlin plugin install $dirs
