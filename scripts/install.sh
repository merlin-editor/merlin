#!/bin/sh
# Merlin Editor installer & updater.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/merlin-editor/merlin/main/scripts/install.sh | sh
set -e

REPO="merlin-editor/merlin"
INSTALL_DIR="${MERLIN_INSTALL_DIR:-$HOME/.local/bin}"

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
    if echo "$CURRENT_VERSION" | grep -q "$LATEST_TAG"; then
        echo "==> Merlin is already up to date ($LATEST_TAG)."
        exit 0
    fi
    echo "==> Updating Merlin: $CURRENT_VERSION -> $LATEST_TAG"
fi

if [ -z "$LATEST_TAG" ]; then
    DOWNLOAD_URL="https://github.com/$REPO/releases/latest/download/merlin-$TARGET.$EXT"
else
    DOWNLOAD_URL="https://github.com/$REPO/releases/download/$LATEST_TAG/merlin-$TARGET.$EXT"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "==> Downloading $DOWNLOAD_URL..."
curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/merlin.$EXT"

mkdir -p "$INSTALL_DIR"

if [ "$EXT" = "zip" ]; then
    unzip -q "$TMP_DIR/merlin.zip" -d "$TMP_DIR"
    cp "$TMP_DIR/merlin.exe" "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/merlin.exe" 2>/dev/null || true
    echo "==> Merlin installed/updated to $INSTALL_DIR/merlin.exe"
else
    tar -xzf "$TMP_DIR/merlin.tar.gz" -C "$TMP_DIR"
    cp "$TMP_DIR/merlin" "$INSTALL_DIR/merlin"
    chmod +x "$INSTALL_DIR/merlin"
    echo "==> Merlin installed/updated to $INSTALL_DIR/merlin"
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
