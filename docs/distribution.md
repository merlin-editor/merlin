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

## 3. Editor Releases & Multi-Platform CI (Workflow Built; No Release Yet)

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
  No `v*` tag has been pushed yet. It produces a draft GitHub Release with all 5 archives (each holding the binary and `README.md`) and their SHA-256 checksums attached. The draft has to be published by hand. The tag pattern `v[0-9]+.*` leaves `sdk-v*` tags alone.

---

## 4. One-Line Installer & Updater (Built; Untried Until a Release Exists)

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
  - It doesn't check the `.sha256` it could download, and installs only the binary (section 9, A4).

---

## 5. In-App Update UI (Mockup Approved, Not Built)

- **Mockup File**: `docs/mockups/AppUpdate.dc.html`
- **Placement**: Right side of the window title bar, directly to the left of the three layout toggles.
- **Appearance**: A subtle pill badge (`[ • Update available ↓ ]`) that only shows when a newer release is discovered.
- **Interaction**: Clicking the pill opens a compact dropdown card with release details, release notes, and a 1-click "Restart & Update" button.

---

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
6. **Phase 5 (Next)**: Release archives carry the registry's packages. Planned in section 9, A3.

---

## 7. Edge Cases, Packaging & Operational Considerations

Of section 7, the `install` hints in 7.2 and the Windows fallback in 7.4 are built.

### 7.1 Bundled / Offline Plugins in Release Archives (Planned, Revised in Section 9, A3)
- **Problem**: Fresh installs without a local Rust toolchain or network access should not have an uncoloured editor.
- **Solution**: The editor release workflow (`ci.yml`) compiles and packs core language plugins (`rust`, `python`, `cpp`, `javascript`, `json`, `toml`, `merlin-themes`) into a `bundled-plugins/` directory inside the `.tar.gz` / `.zip`.
- **First-run bootstrap**: On startup, if `~/.config/merlin/plugins/` is empty and no plugins are configured, Merlin auto-extracts the bundled plugins so syntax highlighting, themes, and language server wiring work immediately out-of-the-box.

### 7.2 Native Tool Dependencies (LSP & Debuggers) (Partly Built)
- **Problem**: Plugins are 100% portable wasm, but depend on host system processes (`rust-analyzer`, `clangd`, `lldb-dap`, `gdb`).
- **Built**: `[[debuggers]]` entries in `plugin.toml` carry an `install = "..."` hint (e.g. `sudo apt install lldb`), which the editor shows in a warning with a **Copy Install Command** action when starting a debug session finds none of a language's debuggers.
- **Planned**: The editor does not block installation if a process is missing on `$PATH`. Instead, the install confirmation card reads the `install` hints (to be added for language servers too) and surfaces a copyable command:
  > *"Installed Rust! Note: `rust-analyzer` was not found on your $PATH. Install it to enable completion and diagnostics: `rustup component add rust-analyzer` [Copy]"*

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

**A3. Bundled plugins (phase 5).** This revises 7.1: the archive carries the
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

**A4. `install.sh` checks what it downloads.** Verify the archive against its
`.sha256` (`sha256sum` or `shasum -a 256`), and refuse on a mismatch. Unpack
`plugins/` into `~/.local/share/merlin/bundled/`. Try it against the draft
release before publishing.

**A5. A release checklist**, as `docs/releasing.md`:
1. CI green on `main`.
2. Bump `version` in the root `Cargo.toml`, and write the release notes.
3. If the API or plugins changed: the registry PR (A3), merged and published.
4. If the API changed: `sdk-v<api>` first, and wait for crates.io.
5. Tag `v<version>` and push it. Check the draft has 5 archives, each with
   `plugins/`.
6. Try `install.sh` against the draft, then publish it.

**Decide**, *macOS and Windows signing*: unsigned binaries work through
`install.sh` (curl sets no quarantine flag), but a `.zip` downloaded in a
browser meets Gatekeeper or SmartScreen. Recommended: ship v0.1.0 unsigned,
say so in the README, and sign later (Apple's notarization needs a paid
developer account).

### Milestone B: updating merlin from merlin (section 5)

The mock-up (`docs/mockups/AppUpdate.dc.html`) is approved.

**B1. Signed releases.** A checksum from the same release page proves nothing
against a tampered release, so the updater needs a signature. The release job
signs a `SHA256SUMS` file with minisign, using a key of its own in the
`release` environment. merlin embeds the public key, as it does the
registry's. `install.sh` checks the signature too when `minisign` is
installed. **Decide**: a new release key (recommended, so one leaked key
doesn't sign both plugins and the editor) or the registry's key.

**B2. Finding an update.** On the `Net` lane, at start and every 6 hours like
the registry: ask GitHub's `releases/latest`, compare versions, and show the
title bar's pill. Setting `editor.check_for_updates` (on by default). Off in
scripts.

**B3. Restart & Update.** Download this platform's archive, check it against
the signed `SHA256SUMS`, and unpack it beside the executable. Then swap it in:
on Windows, rename the running `.exe` first, since it can't be overwritten.
Replace `plugins/` too, so A3's first-run step brings plugins to the new API.
Restart with the same arguments; hot exit brings the tabs back. Offer this only
when merlin can write to its own folder and wasn't installed by a package
manager (`MERLIN_NO_SELF_UPDATE`, or a path under `/usr`). Otherwise the card
gives the command to run.

### Milestone C: programs plugins need (7.2)

**C1. Install hints for every program.** The manifest gains
`[programs.<name>] install = { linux = "…", macos = "…", windows = "…" }`
for what `processes` names, as `[[debuggers]]` has `install` today. The
manifest refuses unknown keys, so this is an API bump, done with the next one.
`tools/registry.py` copies the hints into `index.json`.

**C2. Showing them.** After an install from Browse, and on the plugin's page,
a program that isn't on `PATH` gets its command with Copy. The language
status card already has a "not installed" state in its approved mock-up. The
Browse page's part needs a small mock-up first.

### Milestone D: running the registry

- **D1. The Report template.** Browse's Report opens
  `issues/new?title=Report: <name> <version>`; add
  `.github/ISSUE_TEMPLATE/report.yml` to `merlin-editor/plugins` so the issue
  asks for what a maintainer needs.
- **D2. Watch the first scheduled re-sign** (Monday 10 November, 03:17 UTC),
  and add an alert: the workflow fails loudly if the index has under 14 days
  left.
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
