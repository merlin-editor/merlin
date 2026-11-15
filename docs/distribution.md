# Distribution, Releases & Updates

This document tracks the design and implementation for distributing **Merlin Editor**, the **Plugin SDK**, and **Plugins**.

---

## 1. Overview of Release Channels

| Component | Target Location | Packaging | Release Trigger |
|---|---|---|---|
| **Plugin SDK** (`merlin-plugin`, `merlin-lsp`, `merlin-grammar`) | [crates.io](https://crates.io) | Cargo crates for `wasm32-wasip2` | Git tag `sdk-v*` |
| **Editor Binary** (`merlin`) | [GitHub Releases](https://github.com/merlin-editor/merlin/releases) | Multi-platform archives (`.tar.gz`, `.zip`) + `.sha256` | Git tag `v*` (e.g., `v0.1.0`) |
| **Installer & Updater** | GitHub raw / repository script | POSIX shell script (`scripts/install.sh`) | Main branch updates |
| **Plugin Registry** | [merlin-editor/plugins](https://github.com/merlin-editor/plugins) | Deterministic `.merlin` archives + signed `index.json` | PRs to plugins repo |

---

## 2. Plugin SDK on crates.io (Completed, 0.14.0)

0.14.0 was published by `sdk-release.yml` on 2026-11-09, its first run, from the `sdk-v0.14.0` tag. `merlin plugin new` writes `merlin-plugin = "<API_VERSION>"`, from crates.io; `MERLIN_SDK_PATH=<checkout>/sdk/rust` makes it a path instead, to change the SDK and a plugin together. The SDK crates are published to crates.io, all three at the same version (the `version` in `sdk/*/Cargo.toml`, which moves with the plugin API's `API_VERSION`):
- `merlin-plugin` (Core WebAssembly guest bindings and runtime macro)
- `merlin-lsp` (Language Server Protocol client helper)
- `merlin-grammar` (Tree-sitter syntax highlighter with bundled wasm C headers)

### Automated Publishing via GitHub Actions (Trusted Publishing)
- **Workflow**: `.github/workflows/sdk-release.yml`
- **Authentication**: OpenID Connect (OIDC) through `rust-lang/crates-io-auth-action`—no permanent secrets or API tokens stored in GitHub.
- **Trigger**:
  ```bash
  git tag sdk-v0.13.0
  git push origin sdk-v0.13.0
  ```
- **Order**: `merlin-plugin` first, then a 25-second wait for the crates.io index, then `merlin-lsp` and `merlin-grammar`, which depend on it. `scripts/publish-sdk.sh` does the same by hand.
- **Environment**: Protected `release` environment in GitHub: only `sdk-v*` tags deploy to it, and a run waits until a required reviewer (marc-cr1810) approves it under Review deployments.

---

## 3. Editor Releases & Multi-Platform CI (Built; v0.1.1 Released 2026-11-15)

- **Workflow**: `.github/workflows/ci.yml`
- **Testing**: On every push to `main` and every pull request to `main`, `cargo test --workspace` runs on Ubuntu, macOS, and Windows.
- **Release Matrix**:
  - Linux `x86_64` (`merlin-linux-x86_64.tar.gz`)
  - Linux `aarch64` (`merlin-linux-aarch64.tar.gz`)
  - macOS Apple Silicon `aarch64` (`merlin-macos-aarch64.tar.gz`)
  - macOS Intel `x86_64` (`merlin-macos-x86_64.tar.gz`)
  - Windows `x86_64` (`merlin-windows-x86_64.zip`)
- **Trigger**:
  ```bash
  git tag v0.1.0
  git push origin v0.1.0
  ```
  It produces a draft GitHub Release with all 5 archives (each holding the binary, `README.md`, `LICENSE` and `plugins/`) and their SHA-256 checksums attached. The release job needs `permissions: contents: write`, since the repository's workflows may only read by default. The Intel Mac build runs on `macos-15-intel`: a job asking for a retired runner such as `macos-13` waits a day in the queue and then fails. The draft has to be published by hand. The tag pattern `v[0-9]+.*` leaves `sdk-v*` tags alone.

---

## 4. One-Line Installer & Updater (Built; Tried Against a Local Release)

- **Script**: `scripts/install.sh`
- **Command**:
  ```sh
  curl -fsSL https://raw.githubusercontent.com/merlin-editor/merlin/main/scripts/install.sh | sh
  ```
- **Behavior**:
  - Automatically detects the host OS and CPU architecture.
  - Installs to `~/.local/bin/merlin` (or `$MERLIN_INSTALL_DIR`), and says so if that folder isn't on `$PATH`.
  - Re-running the command acts as an updater: it asks the GitHub Releases API for the latest tag, compares it with `merlin --version` (which prints `merlin v<version>`), and downloads only when they differ.
  - Windows is handled only under a POSIX shell (Git Bash, MSYS2, Cygwin).
  - It checks the archive against the release's `.sha256` (with `sha256sum` or `shasum`) and installs nothing on a mismatch. It puts the archive's `plugins/` in `$XDG_DATA_HOME/merlin/bundled/` (`~/.local/share/merlin/bundled/`), where merlin finds the plugins that come with it. `MERLIN_RELEASE_URL` takes the archives from elsewhere, to try a release before publishing it.

---

## 5. In-App Update UI (Built 2026-11-15)

- **Mockups**: `docs/mockups/AppUpdate.dc.html` (the pill and Update Ready) and `docs/mockups/AppUpdateStates.html` (release notes in the card, step-by-step progress, Ready to Restart, Update Failed, and Update Available for a copy that can't replace itself), both approved.
- **Placement**: Right side of the window title bar, directly to the left of the three layout toggles, with a line between.
- **Appearance**: A pill (`• Update available ↓`) that shows only when a newer release is published; `Updating 52%` with a ring while it works, and `Restart to update` once it's in place.
- **Interaction**: The pill opens a card. Release notes shows the release's own notes in the card, as Markdown, scrolling, with Open on GitHub. Restart & Update downloads, checks and puts it in place, then restarts with unsaved files kept; Cancel works until it's being put in place.
- **Code**: the card's states in `crates/core/src/editor/self_update.rs` (tests in `crates/core/tests/self_update.rs`), drawn by `crates/app/src/gui/frame/update_card.rs`; finding, downloading and installing in `crates/app/src/update.rs`.

## 6. Plugin Distribution System (Implementation Plan)

The full spec for the package format, the registry repo, `index.json`, signing and installing is in [plugin-registry.md](plugin-registry.md).

### Today
- `merlin plugin install <dir>...` links a built plugin folder into `~/.config/merlin/plugins/` (or `$XDG_CONFIG_HOME/merlin/plugins/`) and grants what its manifest asks for. Plugins there load at every start.
- `merlin plugin pack [dir]` packs a built plugin into a deterministic `<name>-<version>.merlin`, and `merlin plugin install <file.merlin>` unpacks one there, swapping out the version before it and recording it in `installed.toml`.
- `merlin plugin install <name>`, `search`, `update` and `uninstall` work against signed registry indexes: merlin's own at `https://merlin-editor.github.io/plugins/index.json`, and any in `[plugin-sources]` in user settings. merlin's serves Merlin's 13 bundled plugins.
- `scripts/install-plugins.sh` builds the plugins that come with Merlin from source and installs them this way.
- `merlin plugin new <name>` creates a plugin from the template.
- The host accepts only plugins whose manifest `api` equals `API_VERSION` exactly (see section 8).

### Key Decisions
- **Default Source**: GitHub Pages + Releases on `merlin-editor/plugins`.
- **Custom Sources**: Configured in user settings (`[plugin-sources.<name>]`), never workspace settings.
- **Builds**: Pinned commit builds in containerized CI; verified reproducible.
- **Package Format**: Deterministic `.tar.gz` containing `plugin.toml`, `plugin.wasm`, and assets.
- **Security**: Minisign-signed `index.json` with monotonic serials, expiration timestamps, and per-version SHA-256s; grants stay keyed by the install folder, `~/.config/merlin/plugins/<name>/`.

### Phased Implementation Roadmap
1. **Phase 0 (Done)**: Publish SDK crates to crates.io; point `merlin-grammar`'s C build at its bundled headers (`sdk/grammar/.cargo/config.toml`, `wasm-include/`); create automated release workflows.
2. **Phase 1 (Done, API 0.14)**: `.merlin` package format, `merlin plugin pack`, local archive installation, and staging directory swaps.
3. **Phase 2 (Done)**: Sources configuration, index verification (`minisign-verify`), and CLI search/update commands.
4. **Phase 3 (Done)**: Registry at `merlin-editor/plugins` with reproducible CI builds and signing; its first index, with Merlin's 13 bundled plugins, went live on 2026-11-09.
5. **Phase 4 (Done)**: Browse in Settings › Plugins, with updates, withdrawn and blocked plugins, and the permissions prompt (`docs/plugin-registry.md` 9).
6. **Phase 5 (Done, released in v0.1.0)**: Release archives carry the registry's packages, installed on first run (section 9, A3).

---

## 7. Edge Cases, Packaging & Operational Considerations

Of section 7, the `install` hints in 7.2 and the Windows fallback in 7.4 are built.

### 7.1 Bundled / Offline Plugins in Release Archives (Built as Section 9, A3, Describes)
- **Problem**: Fresh installs without a local Rust toolchain or network access should not have an uncoloured editor.
- **Solution**: The editor release workflow (`ci.yml`) compiles and packs core language plugins (`rust`, `python`, `cpp`, `javascript`, `json`, `toml`, `merlin-themes`) into a `bundled-plugins/` directory inside the `.tar.gz` / `.zip`.
- **First-run bootstrap**: On startup, if `~/.config/merlin/plugins/` is empty and no plugins are configured, Merlin auto-extracts the bundled plugins so syntax highlighting, themes, and language server wiring work immediately out-of-the-box.

### 7.2 Native Tool Dependencies (LSP & Debuggers) (Partly Built)
- **Problem**: Plugins are 100% portable wasm, but depend on host system processes (`rust-analyzer`, `clangd`, `lldb-dap`, `gdb`).
- **Built**: `[[debuggers]]` entries in `plugin.toml` carry an `install = "..."` hint (e.g. `sudo apt install lldb`), which the editor shows in a warning with a **Copy Install Command** action when starting a debug session finds none of a language's debuggers.
- **Planned** (Milestone C, plugin-registry.md 12.1 to 12.3): install hints for every program, one per system, and merlin showing what's missing without blocking an install.

### 7.3 Wasm Optimization (`wasm-opt`) (Dropped)
- **Problem**: Component `.wasm` files can be 2–3 MB each (e.g. `rust_language.wasm` is 2.9 MB).
- **Tried, 2026-11-09**: Binaryen's `wasm-opt` (version 120) refuses components: *"this looks like a wasm component, which Binaryen does not support yet"* ([binaryen#6728](https://github.com/WebAssembly/binaryen/issues/6728)). Run on the core module inside, `-Oz` cut raw size by about 23%, but the gzipped size, which is what a package holds, went up for the Rust plugin (686 KB → 710 KB) and saved only 3 KB for Word Count (25 KB → 22 KB). Using it would also mean optimising the module before `wasm-component-ld` wraps it, a change to every plugin's build.
- **Decision**: Packages hold the component as built. Plugins already build with `opt-level = "s"` and `strip = true`, the package is gzipped (the Rust plugin packs to 684 KB), and the editor caches each compiled plugin, so first-load compile time is paid once per version. Worth trying again if Binaryen learns to read components.

### 7.4 Cross-Platform Symlinks & Windows Fallback (`merlin plugin install`) (Built)
- **Problem**: Standard user accounts on Windows often lack permission to create UNIX symbolic links unless Developer Mode is enabled.
- **Solution**: `merlin plugin install <dir>` links with `std::os::windows::fs::symlink_dir` on Windows. If that fails, it packs the folder and installs the package instead, saying it's a copy that needs installing again after a rebuild (`crates/plugin-host/src/package.rs`, `link`). Not yet tried on Windows itself.

### 7.5 Yanks & Security Revocations (Built)
- **Built**: `merlin plugin install` and `update` never choose a yanked or blocked version, and `update` warns about an installed one. Settings › Plugins shows an installed yanked version as withdrawn with its reason and offers the update, and turns a blocked plugin off and keeps it off. The wording below is the plan's; the editor says "withdrawn".
- **Problem**: Mitigating compromised or critically buggy plugin versions after release.
- **Solution**: In `index.json`, versions can have `"yanked": "Reason / CVE"`.
  - Yanked versions are hidden from registry search.
  - If a user has a yanked version installed, Settings › Plugins displays a prominent warning badge:
    > *"Version 0.2.1 was yanked by author: [Reason]. An update is recommended."*

---

## 8. Simultaneous SDK and Editor Releases

When a feature touches both the Editor host and the SDK (e.g., expanding the WIT plugin API):

```bash
# Tag the SDK
git tag sdk-v0.14.0

# Tag the Editor
git tag v0.1.0

# Push both tags simultaneously
git push origin sdk-v0.14.0 v0.1.0
```

GitHub Actions triggers both workflows independently:
1. `sdk-release.yml` authenticates via OIDC and publishes the new SDK crates to crates.io.
2. `ci.yml` runs workspace tests across Ubuntu, macOS, and Windows, then builds the multi-platform release binaries.
3. Compatibility is guarded by `API_VERSION` in `crates/plugin-host/src/lib.rs`: a plugin loads only if its manifest's `api` matches it exactly, so a plugin built against an older SDK must be rebuilt. While the API is in development it breaks freely, and `API_VERSION`, the WIT, the SDK crate versions and every `plugin.toml` are bumped together. Once it settles, an oldest-supported version (`API_OLDEST`) can widen this to a range.

---

## 9. Plan to Finish (Written 2026-11-09)

What's left, in the order to do it. Milestone A is the first public release,
`v0.1.0`; B to D follow it. Items marked **Decide** need a call from the
maintainer before they're built.

### Where things stand

- Built: the SDK release workflow, the editor's release matrix, `install.sh`,
  and plugin distribution phases 1 to 4 (packages, signed indexes, the
  registry at `merlin-editor/plugins`, Browse).
- Never run: no `v*` tag exists, so neither the release job nor `install.sh`
  has been tried end to end. crates.io still has SDK 0.9.0.
- CI on `main` failed once on Windows, at `aea95a8`:
  `catch2_plugin_discovers_cpp_tests_and_skips_other_languages`
  (`crates/plugin-host/tests/api.rs:623`) saw 0.6 ms of plugin CPU time where
  it expects none. It passed at `25d2c3c`, so it's a timing flake: the
  plugin's start-up work can still be counted after `load` returns.
- The registry's index expires on 8 December 2026. `resign.yml` re-signs it
  every Monday at 03:17 UTC, and hasn't run on its schedule yet.

### Milestone A: the first release, v0.1.0

**A1. A green CI (done, `ce8dc0b`).** The flake was in the host, not the
test: `Worker::run` replied before its thread counted the job's time, so a
caller could read a plugin's CPU time without its own call in it. The thread
now counts, then replies. CI passed on Linux, macOS and Windows.

**A2. SDK 0.14.0 on crates.io (done).** `sdk-v0.14.0` published all three
crates through trusted publishing on the workflow's first run. `merlin plugin
new` used to write a path to the SDK in the source it was built from, which a
released binary doesn't have; it now writes the crates.io version, and a
plugin made from it builds.

**A3. Bundled plugins (phase 5): built.** `merlin plugin fetch <dir>`
(`crates/plugin-host/src/bundled.rs`), the `bundle` job in `ci.yml` that hands
`plugins/` to each archive with `LICENSE`, the first-run install in
`crates/app/src/browse.rs` (`install_bundled`), and `installed.toml`'s
`removed` list, which every uninstall writes and linking a folder clears.
Tried on 2026-11-09 with a release laid out by hand and a fresh home: 13
installed and granted, the notice with Review, nothing at the next start,
and an uninstalled one stayed gone. Not tried: the `bundle` job itself, which
runs only on a `v*` tag. A notice that a plugin sends soon after can cover
this one, since one shows at a time; it stays in Output's merlin channel.
`MERLIN_BUNDLE=<dir>` points merlin at another bundle.

The design, as planned: the archive carries the
registry's own packages, so a bundled plugin is byte for byte the one Browse
would install, and it updates from the registry like any other.

- *Archive layout*: `merlin[.exe]`, `README.md`, `LICENSE`, and `plugins/` with
  the 13 `.merlin` files and `plugins/bundled.toml` (name, version, SHA-256).
- *Getting them*: a new `merlin plugin fetch <dir> <name>...` downloads each
  plugin's newest version for this `API_VERSION` from the signed index, and
  checks it as `install` does. Run once in a release job on Linux x86_64, and
  hand the folder to each platform's job as an artifact. Cross-built aarch64
  binaries can't run on the runner, so fetching in each job won't work.
- *Order of a release*: the registry must already have versions for the
  release's API. If the API or any bundled plugin changed since the
  registry's `MERLIN_REV`, release them there first: a registry PR that bumps
  `MERLIN_REV` to the release commit and adds versions. Then tag `v*`. The
  fetch fails the release if a plugin has no version for the API.
- *First run*: merlin looks for `plugins/` beside its executable, following
  symlinks, then in `~/.local/share/merlin/bundled/`, where `install.sh`
  puts it (A4). Each bundled package installs through `package::install_bytes`
  with source `merlin` when:
  - it isn't installed at all and was never uninstalled (`installed.toml`
    gains a `removed` list, so an uninstall sticks), or
  - the installed copy came from source `merlin` and is older. This also
    carries plugins across an API bump when the editor updates.
  A linked plugin of the same name is left alone.
- *Permissions* (decided 2026-11-09): bundled plugins are granted what they
  ask for when they install on first run, since they come with the binary
  and are as trusted as it. One notice lists what was granted, with Review,
  which opens Settings › Plugins. Rust, C/C++, Python and Go run language
  servers outside the sandbox, so the notice names those programs.
- Tests: first run installs all; uninstall then restart leaves it gone; an
  older bundled version doesn't downgrade a newer one from Browse.

**A4. `install.sh` checks what it downloads (done).** It verifies the archive
against its `.sha256` and refuses a mismatch, and puts `plugins/` in
`~/.local/share/merlin/bundled/`. It also compares `merlin --version` exactly,
so `v0.1.0` no longer counts as up to date for `v0.1.01`. Tried on 2026-11-09
with archives built as `ci.yml` builds them, served with
`MERLIN_RELEASE_URL=file://…`: it installed, merlin then installed all 13
plugins from there, and a tampered archive was refused. Not tried: macOS,
Git Bash on Windows, and a real draft release (step 6 of A5). On Windows it
puts the plugins in `%LOCALAPPDATA%\merlin\bundled`.

**Windows before 0.1.0 (done).** merlin found its folders only through
`XDG_*` or `HOME`, which Windows doesn't set outside Git Bash, so started
from Explorer it had no plugins folder and kept its data in the temp folder.
`config_home` and `data_home` in the plugin host now take `XDG_*` first,
then `%APPDATA%` (settings, plugins) and `%LOCALAPPDATA%` (data) on Windows,
then `HOME`. And merlin is a window program on Windows, so no console opens
beside it; started from a terminal, it attaches to it, so `merlin plugin …`
and `--version` still print. Checked by compiling for Windows and macOS with
clippy, and a unit test of the folder order; not yet run on Windows itself.

**A5. A release checklist (done).** [releasing.md](releasing.md) gives the
order: check, version, SDK if the API changed, the registry if plugins
changed, tag, try the draft with `install.sh`, publish. It also covers
undoing a bad tag or draft. `scripts/release-check.sh <version>` checks the
preconditions and changes nothing: a clean tree at `origin/main`, the
workspace version, a free tag, CI passed, the SDK on crates.io, and every
plugin of merlin's own in the registry at its current version for this API,
built from unchanged source. On 2026-11-09 it passed everything for 0.1.0
except CI, which was still running, and the uncommitted script itself.

**A6. v0.1.0 released (done, 2026-11-15).** The first `v0.1.0` tag's run
failed twice over: every upload got a 403, because the release job could
only read, and the Intel Mac job waited a day for the retired `macos-13`
runner. Both were fixed on `main` (`268c5c3`, `4b0bc09`) and the tag moved
to `4b0bc09`. The draft was tried on Linux x86_64 as releasing.md step 6
says: all five archives matched their checksums and held 13 packages and
`bundled.toml`; `install.sh` installed from it into a fresh home, merlin
installed all 13 plugins on first run, and a tampered archive was refused.
Published with release notes the same day, and `install.sh` from `main`
then installed it from GitHub. `install.sh` now asks the copy in its
install folder for its version, not the first `merlin` on `PATH`. Not tried
by hand: macOS and Windows.

**A7. v0.1.1 released (done, 2026-11-15).** The first signed release and
the first with the updater. The `sign` job ran on approval and attached
`SHA256SUMS` and `SHA256SUMS.minisig` (trusted comment `merlin v0.1.1`); by
hand, all five archives matched it and held 13 packages, and `install.sh`
said "Signature matches" both from the draft and, once published, from
GitHub. 0.1.0 can't update itself, so its users get 0.1.1 through
`install.sh`; from 0.1.1 on, new releases show in the title bar.

**Decided**, *macOS and Windows signing*: v0.1.0 shipped unsigned, and its
release notes say how to open it past Gatekeeper and SmartScreen. Unsigned binaries work through
`install.sh` (curl sets no quarantine flag), but a `.zip` downloaded in a
browser meets Gatekeeper or SmartScreen. Recommended: ship v0.1.0 unsigned,
say so in the README, and sign later (Apple's notarization needs a paid
developer account).

### Milestone B: updating merlin from merlin (section 5)

The mock-up (`docs/mockups/AppUpdate.dc.html`) is approved.

**B1. Signed releases (done, 2026-11-15).** A checksum from the same release
page proves nothing against a tampered release, so the updater needs a
signature. Decided: a release key of its own, so one leaked key doesn't sign
both plugins and the editor. Its public half is `keys/release.pub` (id
D91112BFB30E5609, no password, made with `minisign -G -W`); the secret is
`MINISIGN_SECRET_KEY` in the merlin repo's `release` environment, which now
admits `v*` tags beside `sdk-v*`, and a copy is in
`~/.config/merlin-release/` on the maintainer's laptop. A rotation key,
`keys/release-next.pub` (id E34003FE17667298), is kept only on the laptop,
never in CI: merlin and `install.sh` accept a release signed by either, so
if the CI key leaks or must go, releases can be signed with the next one
and the old one dropped in a later release. Its secret has no password;
`minisign -C -s ~/.config/merlin-release/merlin-release-next.key` adds one.

- `ci.yml`'s `sign` job runs after the five builds, in the `release`
  environment, so a release waits for a reviewer's approval before it's
  signed. It downloads the draft's archives, hashes them itself into
  `SHA256SUMS`, signs that with the trusted comment `merlin <tag>`, checks
  the signature against `keys/release.pub`, and attaches `SHA256SUMS` and
  `SHA256SUMS.minisig` to the draft.
- merlin embeds the key from `keys/release.pub` (`crates/app/src/update.rs`,
  `release_key`). `check_sums` refuses a list signed by another key,
  changed since, or signed for another tag (so an older release can't pass
  for the newest), and `check_archive` checks an archive against it.
- `install.sh` checks the signature, the comment and the archive's line in
  the list when `minisign` is installed. Without it, or for a release with
  no `SHA256SUMS` (v0.1.0), it says so and goes by the checksum alone.

**B2. Finding an update (done, 2026-11-15).** On the `Net` lane, 10 s after
the window opens and every 6 hours: ask GitHub's `releases/latest` (or
`MERLIN_UPDATE_URL`, to try a release before it's out), compare its tag's
numbers with this merlin's, and show the pill with the release's notes.
`editor.check_for_updates` in user settings (on by default; Settings ›
Text Editor › Updates, a row only your settings can change) turns it off;
scripts never check. A failed check says so on stderr only.

**B3. Restart & Update (done, 2026-11-15).** Downloads this platform's
archive with progress (Cancel stops it until the swap), then `SHA256SUMS`
and its signature, and checks them as B1 says: anything that doesn't match
fails with "The download doesn't match the release's signature, so nothing
was changed." It unpacks into `.merlin-update/` beside the executable (a zip
with Windows' own `tar`), then swaps: on Unix it renames the new binary over
the old; on Windows it renames the running `.exe` to `.merlin-old.exe` first.
The bundled plugins replace the folder merlin found them in (beside the
executable, or `~/.local/share/merlin/bundled`), so A3's first-run step brings
plugins to the new API. With the card open it then restarts at once; with
it closed it waits, as Ready to Restart. Restarting keeps unsaved files
whatever `files.hot_exit` says, and starts the same executable path with the
same arguments in the same folder, after the backups are written. The next
start removes what was left beside it. A copy that can't replace itself
(`MERLIN_NO_SELF_UPDATE`, a folder under `/usr`, `/opt`, `/nix`, Homebrew
and the like, or one it can't write to) gets the `install.sh` command with
Copy instead.

Tried 2026-11-15 in the real window, with a release served locally as
v0.1.1 and signed with the release key: the pill, the card, the notes
scrolling, progress through each step, the swap to the release binary and
the restart with an unsaved line kept; a tampered archive (Update Failed,
nothing changed); and `MERLIN_NO_SELF_UPDATE` (the command with Copy). Not
tried: macOS, Windows, and a real published release, which needs one signed
by the `sign` job.

### Milestone C: plugin API 0.15 and merlin v0.2.0

Planned in full in plugin-registry.md section 12, for review. In short:

- **C1. How to install a program.** `[programs.<name>]` in the manifest,
  with `install` as one command or one per system, a `url`, and what's
  missing without it. `[[debuggers]] install` takes the same form.
- **C2. Showing what's missing,** in the language card, Browse, the
  plugin's page and the install card. merlin checks `PATH`; nothing blocks
  an install.
- **C3. Systems and processors:** `systems` and `arch` on a plugin, a
  debugger or a program. Version choice respects them, so a plugin can drop
  or add a system between versions.
- **C4. Dependencies** installed together, with version requirements and
  conflicts reported.
- **C5. A `network` permission** for named HTTPS hosts, with `fetch`.
- **C6. Agents:** `[[agents]]`, the chat panel, permission cards and
  reviewing changes, owned by merlin; a plugin translates for each agent.
  Agents can read and write files through the editor and run commands in
  its terminal tabs. The `acp` plugin, then `claude`, follow v0.2.0.
- **C7. Without an API change:** automatic plugin updates, Keep at this
  version, removing settings on uninstall, Replaced by.

Mock-ups first (section 12.8 there), then the build order in 12.9.

### Milestone D: running the registry

- **D1. The Report template.** Browse's Report opens
  `issues/new?title=Report: <name> <version>`; add
  `.github/ISSUE_TEMPLATE/report.yml` to `merlin-editor/plugins` so the issue
  asks for what a maintainer needs.
- **D2. Watching the index (done, 2026-11-15).** The first scheduled re-sign
  ran on 9 November; the index then expired 9 December. GitHub stops a
  repository's schedules after 60 days without activity, and nothing fails
  then, so two daily watches check the live index's signature and days
  left: `watch.yml` in merlin-editor/plugins starts Re-sign with under 21
  days and fails under 14; `registry-watch.yml` here fails under 14, and
  keeps running while merlin has pushes. A failed run emails the
  maintainers.
- **D3. Key rotation runbook**, in the registry's README: how
  `merlin-next.pub` becomes the signing key, which merlin release must ship
  first, and how to retire the old key.
- **D4. Releasing to the registry from the editor's CI.** A3's registry PR by
  hand at first. **Decide** later: automate it with a GitHub App that can open
  PRs on `merlin-editor/plugins`.
- **D5. Windows.** Run `merlin plugin install <dir>`'s copy fallback (7.4) on
  real Windows, or in a CI test that stubs out link creation.

### Later, not planned in detail

- Package managers: a Homebrew tap, AUR, winget, Flatpak; each sets
  `MERLIN_NO_SELF_UPDATE`.
- A macOS `.app` bundle and a Windows installer.
- Plugin API 1.0 and `API_OLDEST` (plugin-registry.md 11), once the API
  stops breaking.
- Delta updates for the editor or plugins, if downloads grow.
