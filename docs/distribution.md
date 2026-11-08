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

## 2. Plugin SDK on crates.io (Completed)

The SDK crates are published to crates.io, all three at the same version (the `version` in `sdk/*/Cargo.toml`, which moves with the plugin API's `API_VERSION`):
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
- **Environment**: Protected `release` environment in GitHub (configured with `sdk-v*` deployment branch/tag rule).

---

## 3. Editor Releases & Multi-Platform CI (Completed)

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
  Produces a draft GitHub Release with all 5 archives (each holding the binary and `README.md`) and their SHA-256 checksums attached. The draft has to be published by hand. The tag pattern `v[0-9]+.*` leaves `sdk-v*` tags alone.

---

## 4. One-Line Installer & Updater (Completed)

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
- `merlin plugin install <name>`, `search`, `update` and `uninstall` work against signed registry indexes: merlin's own at `https://merlin-editor.github.io/plugins/index.json`, and any in `[plugin-sources]` in user settings. merlin's has no index yet, so for now they report a 404 for it.
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
4. **Phase 3 (Next)**: Registry template repo at `merlin-editor/plugins` with reproducible CI builds and signing.
5. **Phase 4**: In-app "Browse" tab in Settings › Plugins with update indicators.

---

## 7. Edge Cases, Packaging & Operational Considerations

Of section 7, the `install` hints in 7.2 and the Windows fallback in 7.4 are built.

### 7.1 Bundled / Offline Plugins in Release Archives (Planned)
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

### 7.5 Yanks & Security Revocations (Partly Built)
- **Built**: `merlin plugin install` and `update` never choose a yanked or blocked version, and `update` warns about an installed one. The Settings badge below waits for phase 4.
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
git tag sdk-v0.13.0

# Tag the Editor
git tag v0.1.0

# Push both tags simultaneously
git push origin sdk-v0.9.1 v0.1.0
```

GitHub Actions triggers both workflows independently:
1. `sdk-release.yml` authenticates via OIDC and publishes the new SDK crates to crates.io.
2. `ci.yml` runs workspace tests across Ubuntu, macOS, and Windows, then builds the multi-platform release binaries.
3. Compatibility is guarded by `API_VERSION` in `crates/plugin-host/src/lib.rs`: a plugin loads only if its manifest's `api` matches it exactly, so a plugin built against an older SDK must be rebuilt. While the API is in development it breaks freely, and `API_VERSION`, the WIT, the SDK crate versions and every `plugin.toml` are bumped together. Once it settles, an oldest-supported version (`API_OLDEST`) can widen this to a range.
