#!/bin/sh
# Builds the plugins that come with merlin and installs them, so they load
# at every start: the languages (colours for Rust, Python, JavaScript, JSON,
# TOML, C and C++), language servers, and merlin's themes. As VS Code's
# built-in extensions are, they're plugins like any other.
#
#   scripts/install-plugins.sh [plugin...]    (all of them by default)
set -e
cd "$(dirname "$0")/.."
rustup target add wasm32-wasip2 >/dev/null 2>&1 || true
cargo build --release --bin merlin
plugins=${*:-rust python javascript json toml cpp lsp merlin-themes}
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
