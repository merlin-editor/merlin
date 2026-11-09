#!/usr/bin/env bash
# scripts/release-check.sh <version>: says whether main is ready to be
# tagged v<version>. Changes nothing. See docs/releasing.md.
#
# Checks: the tree is clean and at origin/main; the workspace version is
# <version>; the tag is free; CI passed on this commit; the SDK for this
# plugin API is on crates.io; and merlin's registry has, for this API, the
# version of every plugin that comes with merlin, built from source that
# hasn't changed since.
set -uo pipefail

cd "$(dirname "$0")/.."

version="${1:-}"
version="${version#v}"
if [ -z "$version" ]; then
    echo "usage: scripts/release-check.sh <version>   (as 0.1.0)" >&2
    exit 2
fi
tag="v$version"
INDEX_URL="${MERLIN_INDEX_URL:-https://merlin-editor.github.io/plugins/index.json}"

problems=0
ok() { echo "  ok    $*"; }
bad() { echo "  NO    $*"; problems=$((problems + 1)); }

echo "Checking main for $tag:"

git fetch -q origin main --tags
if [ -n "$(git status --porcelain)" ]; then
    bad "the working tree has changes; commit or stash them"
else
    ok "the working tree is clean"
fi
head=$(git rev-parse HEAD)
if [ "$head" = "$(git rev-parse origin/main)" ]; then
    ok "HEAD is origin/main ($(git rev-parse --short HEAD))"
else
    bad "HEAD isn't origin/main: tag what's on main, after pushing"
fi

workspace=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
if [ "$workspace" = "$version" ]; then
    ok "the workspace version is $version"
else
    bad "the workspace version is $workspace; set it to $version in Cargo.toml"
fi

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null ||
    git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
    bad "$tag already exists"
else
    ok "$tag is free"
fi

if command -v gh >/dev/null 2>&1; then
    ci=$(gh run list --workflow ci.yml --commit "$head" --json conclusion,status \
        -q '.[0] | "\(.status) \(.conclusion)"' 2>/dev/null)
    case "$ci" in
        "completed success") ok "CI passed on this commit" ;;
        "") bad "CI hasn't run on this commit" ;;
        *) bad "CI on this commit is $ci" ;;
    esac
else
    bad "gh isn't installed, so CI can't be checked"
fi

api=$(sed -n 's/^pub const API_VERSION: &str = "\(.*\)";/\1/p' crates/plugin-host/src/lib.rs)
sdk=$(sed -n 's/^version = "\(.*\)"/\1/p' sdk/rust/Cargo.toml | head -n1)
if curl -fsS -H "User-Agent: merlin-release-check" \
    "https://crates.io/api/v1/crates/merlin-plugin/$sdk" >/dev/null 2>&1; then
    ok "merlin-plugin $sdk (plugin API $api) is on crates.io"
else
    bad "merlin-plugin $sdk isn't on crates.io: push sdk-v$sdk first"
fi

# Every plugin of merlin's own in the registry: its version at HEAD must be
# there, for this API, from a commit whose source matches HEAD's.
index=$(mktemp)
trap 'rm -f "$index"' EXIT
if ! curl -fsS "$INDEX_URL" -o "$index"; then
    bad "can't read the registry's index at $INDEX_URL"
else
    while IFS=$'\t' read -r name path have_api commit; do
        manifest="$path/plugin.toml"
        if [ ! -f "$manifest" ]; then
            bad "$name: the registry builds $path, which isn't in this repository"
            continue
        fi
        mine=$(sed -n 's/^version = "\(.*\)"/\1/p' "$manifest" | head -n1)
        if [ "$have_api" = "-" ]; then
            bad "$name $mine isn't in the registry: release it there first"
        elif [ "$have_api" != "$api" ]; then
            bad "$name $mine is in the registry for API $have_api, not $api"
        elif ! git diff --quiet "$commit" HEAD -- "$path"; then
            bad "$name changed since the registry built $mine at ${commit:0:7}: bump its version and release it"
        else
            ok "$name $mine is in the registry for API $api"
        fi
    done < <(python3 - "$index" <<'EOF'
import json, re, sys
index = json.load(open(sys.argv[1]))
for name, entry in sorted(index.get("plugins", {}).items()):
    if not entry.get("repository", "").rstrip("/").endswith("merlin-editor/merlin"):
        continue
    path = entry.get("path", "")
    try:
        text = open(f"{path}/plugin.toml").read()
        mine = re.search(r'^version = "(.*)"', text, re.M).group(1)
    except (OSError, AttributeError):
        print(f"{name}\t{path}\t-\t-")
        continue
    found = [v for v in entry.get("versions", []) if v["version"] == mine]
    if not found:
        print(f"{name}\t{path}\t-\t-")
    else:
        v = found[0]
        print(f"{name}\t{path}\t{v['api']}\t{v.get('commit', '')}")
EOF
)
fi

echo
if [ "$problems" -eq 0 ]; then
    echo "Ready: tag it with"
    echo "  git tag -a $tag -m \"merlin $version\" && git push origin $tag"
else
    echo "$problems thing(s) to fix before tagging $tag."
    exit 1
fi
