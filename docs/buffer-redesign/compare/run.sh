#!/bin/bash
# run.sh <vscode|merlin> <file in files/> [seconds]: opens the file in one
# editor with a throwaway profile, and prints its memory and CPU.
# Make the files first with make_files.py; build merlin with
# `cargo build --release -p merlin`. Close any VS Code first: its
# processes are counted by program file.
C=$(cd "$(dirname "$0")" && pwd); E=$1; F=$C/files/$2; S=${3:-45}
W=$C/work; P=$W/profile-$E; mkdir -p "$P"
MERLIN=$C/../../../target/release/merlin
# No language servers for anyone.
export PATH=/usr/local/bin:/usr/bin:/bin
cd "$C/files"
case $E in
  vscode)
    mkdir -p "$P/data/User"
    [ -f "$P/data/User/settings.json" ] || cat > "$P/data/User/settings.json" <<'JSON'
{ "workbench.editorLargeFileConfirmation": 100000, "telemetry.telemetryLevel": "off", "update.mode": "none" }
JSON
    MEASURE_EXE=/usr/share/code/code MEASURE_MARK=$P/data exec python3 "$C/measure.py" "vscode $2" "$S" \
      /usr/share/code/code --user-data-dir "$P/data" --extensions-dir "$P/ext" --new-window \
      --disable-workspace-trust --skip-welcome --skip-release-notes --disable-telemetry "$F" ;;
  merlin)
    # merlin's own plugins, as installed, but nothing else of the user's.
    mkdir -p "$W/merlin-cfg/merlin"
    [ -e "$W/merlin-cfg/merlin/plugins" ] || cp -P -r "${XDG_CONFIG_HOME:-$HOME/.config}/merlin/plugins" "$W/merlin-cfg/merlin/" 2>/dev/null
    XDG_CONFIG_HOME=$W/merlin-cfg XDG_DATA_HOME=$P/data XDG_STATE_HOME=$P/state XDG_CACHE_HOME=$P/cache \
      exec python3 "$C/measure.py" "merlin $2" "$S" "$MERLIN" "$F" ;;
  *) echo "run.sh <vscode|merlin> <file> [seconds]"; exit 1 ;;
esac
