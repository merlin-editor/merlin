#!/bin/sh
# Merlin Editor installer & updater.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/merlin-editor/merlin/main/scripts/install.sh | sh
set -e

REPO="merlin-editor/merlin"
INSTALL_DIR="${MERLIN_INSTALL_DIR:-$HOME/.local/bin}"
# Where merlin looks for the plugins that come with it, beside its other data.
BUNDLE_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/merlin/bundled"
# MERLIN_RELEASE_URL serves the archives from elsewhere, to try a release.
RELEASE_URL="${MERLIN_RELEASE_URL:-}"

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)
        case "$ARCH" in
            x86_64)  TARGET="linux-x86_64" ;;
            aarch64) TARGET="linux-aarch64" ;;
            *) echo "Unsupported Linux architecture: $ARCH" >&2; exit 1 ;;
        esac
        EXT="tar.gz"
        ;;
    Darwin)
        case "$ARCH" in
            x86_64) TARGET="macos-x86_64" ;;
            arm64)  TARGET="macos-aarch64" ;;
            *) echo "Unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
        esac
        EXT="tar.gz"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        TARGET="windows-x86_64"
        EXT="zip"
        ;;
    *)
        echo "Unsupported operating system: $OS" >&2
        exit 1
        ;;
esac

echo "==> Checking for latest Merlin release for $OS ($ARCH)..."

LATEST_TAG=$(curl -sSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null | grep '"tag_name":' | head -n1 | cut -d '"' -f 4)

# Check if currently installed
CURRENT_VERSION=""
if command -v merlin >/dev/null 2>&1; then
    CURRENT_VERSION="$(merlin --version 2>/dev/null || true)"
fi

if [ -n "$LATEST_TAG" ] && [ -n "$CURRENT_VERSION" ]; then
    if [ "$CURRENT_VERSION" = "merlin $LATEST_TAG" ]; then
        echo "==> Merlin is already up to date ($LATEST_TAG)."
        exit 0
    fi
    echo "==> Updating Merlin: $CURRENT_VERSION -> $LATEST_TAG"
fi

if [ -n "$RELEASE_URL" ]; then
    DOWNLOAD_URL="$RELEASE_URL/merlin-$TARGET.$EXT"
elif [ -z "$LATEST_TAG" ]; then
    DOWNLOAD_URL="https://github.com/$REPO/releases/latest/download/merlin-$TARGET.$EXT"
else
    DOWNLOAD_URL="https://github.com/$REPO/releases/download/$LATEST_TAG/merlin-$TARGET.$EXT"
fi

# A SHA-256 tool, to check the download against the release's checksum.
if command -v sha256sum >/dev/null 2>&1; then
    SHA256="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
    SHA256="shasum -a 256"
else
    echo "Can't check the download: neither sha256sum nor shasum is installed." >&2
    exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "==> Downloading $DOWNLOAD_URL..."
curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/merlin.$EXT"
curl -fsSL "$DOWNLOAD_URL.sha256" -o "$TMP_DIR/merlin.$EXT.sha256"

# The checksum file names the archive as released; only its hash matters.
EXPECTED="$(cut -d ' ' -f 1 < "$TMP_DIR/merlin.$EXT.sha256")"
ACTUAL="$($SHA256 "$TMP_DIR/merlin.$EXT" | cut -d ' ' -f 1)"
if [ -z "$EXPECTED" ] || [ "$EXPECTED" != "$ACTUAL" ]; then
    echo "The download doesn't match its checksum; nothing was installed." >&2
    echo "  expected: ${EXPECTED:-none}" >&2
    echo "  got:      $ACTUAL" >&2
    exit 1
fi
echo "==> Checksum matches."

UNPACKED="$TMP_DIR/unpacked"
mkdir -p "$UNPACKED" "$INSTALL_DIR"
if [ "$EXT" = "zip" ]; then
    unzip -q "$TMP_DIR/merlin.zip" -d "$UNPACKED"
    BINARY="merlin.exe"
else
    tar -xzf "$TMP_DIR/merlin.tar.gz" -C "$UNPACKED"
    BINARY="merlin"
fi
# The zip may hold its files in a folder of their own.
if [ ! -f "$UNPACKED/$BINARY" ] && [ -f "$UNPACKED/staging/$BINARY" ]; then
    UNPACKED="$UNPACKED/staging"
fi
cp "$UNPACKED/$BINARY" "$INSTALL_DIR/$BINARY"
chmod +x "$INSTALL_DIR/$BINARY" 2>/dev/null || true
echo "==> Merlin installed/updated to $INSTALL_DIR/$BINARY"

# The plugins that come with merlin, which it installs when it next starts.
if [ -f "$UNPACKED/plugins/bundled.toml" ]; then
    rm -rf "$BUNDLE_DIR.new"
    mkdir -p "$(dirname "$BUNDLE_DIR")"
    cp -R "$UNPACKED/plugins" "$BUNDLE_DIR.new"
    rm -rf "$BUNDLE_DIR"
    mv "$BUNDLE_DIR.new" "$BUNDLE_DIR"
    echo "==> Its plugins are in $BUNDLE_DIR; merlin installs them when it starts."
fi

# PATH guidance
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo ""
        echo "Note: $INSTALL_DIR is not in your \$PATH."
        echo "Add it to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        echo "  export PATH=\"\$PATH:$INSTALL_DIR\""
        ;;
esac

echo ""
echo "Merlin is ready! Run 'merlin' to launch."
