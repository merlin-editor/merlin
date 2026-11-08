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

The SDK crates are published to crates.io at version `0.9.0`:
- `merlin-plugin` (Core WebAssembly guest bindings and runtime macro)
- `merlin-lsp` (Language Server Protocol client helper)
- `merlin-grammar` (Tree-sitter syntax highlighter with bundled wasm C headers)

### Automated Publishing via GitHub Actions (Trusted Publishing)
- **Workflow**: `.github/workflows/sdk-release.yml`
- **Authentication**: OpenID Connect (OIDC) through `rust-lang/crates-io-auth-action`—no permanent secrets or API tokens stored in GitHub.
- **Trigger**:
  ```bash
  git tag sdk-v0.9.1
  git push origin sdk-v0.9.1
  ```
- **Environment**: Protected `release` environment in GitHub (configured with `sdk-v*` deployment branch/tag rule).

---

## 3. Editor Releases & Multi-Platform CI (Completed)

- **Workflow**: `.github/workflows/ci.yml`
- **Testing**: On every push to `main` and Pull Request, tests run across Ubuntu, macOS, and Windows.
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
  Produces a draft GitHub Release with all 5 packaged binary archives and their SHA-256 checksums attached.

---

## 4. One-Line Installer & Updater (Completed)

- **Script**: `scripts/install.sh`
- **Command**:
  ```sh
  curl -fsSL https://raw.githubusercontent.com/merlin-editor/merlin/main/scripts/install.sh | sh
  ```
- **Behavior**:
  - Automatically detects the host OS and CPU architecture.
  - Installs to `~/.local/bin/merlin`.
  - Re-running the command acts as an updater: queries the GitHub Releases API, checks `merlin --version`, and updates if a newer tag is available.

---

## 5. In-App Update UI (Mockup Completed)

- **Mockup File**: `docs/mockups/AppUpdate.dc.html`
- **Placement**: Right side of the window title bar, directly to the left of the three layout toggles.
- **Appearance**: A subtle pill badge (`[ • Update available ↓ ]`) that only shows when a newer release is discovered.
- **Interaction**: Clicking the pill opens a compact dropdown card with release details, release notes, and a 1-click "Restart & Update" button.

---

## 6. Plugin Distribution System (Implementation Plan)

Full specifications for the plugin package format and registry are detailed below (also documented in `docs/plugin-distribution.md`).

### Key Decisions
- **Default Source**: GitHub Pages + Releases on `merlin-editor/plugins`.
- **Custom Sources**: Configured in user settings (`[plugin-sources.<name>]`), never workspace settings.
- **Builds**: Pinned commit builds in containerized CI; verified reproducible.
- **Package Format**: Deterministic `.tar.gz` containing `plugin.toml`, `plugin.wasm`, and assets.
- **Security**: Minisign-signed `index.json` with monotonic serials, expiration timestamps, and name-keyed permission grants.

### Phased Implementation Roadmap
1. **Phase 0 (Done)**: Publish SDK crates to crates.io; add `.cargo/config.toml` headers for wasm builds; create automated release workflows.
2. **Phase 1 (Next)**: `.merlin` package format, `merlin plugin pack`, local archive installation, and atomic staging directory swaps.
3. **Phase 2**: Sources configuration, index verification (`minisign-verify`), and CLI search/update commands.
4. **Phase 3**: Registry template repo at `merlin-editor/plugins` with reproducible CI builds and signing.
5. **Phase 4**: In-app "Browse" tab in Settings › Plugins with update indicators.

---

## 7. Edge Cases, Packaging & Operational Considerations

### 7.1 Bundled / Offline Plugins in Release Archives
- **Problem**: Fresh installs without a local Rust toolchain or network access should not have an uncoloured editor.
- **Solution**: The editor release workflow (`ci.yml`) compiles and packs core language plugins (`rust`, `python`, `cpp`, `javascript`, `json`, `toml`, `merlin-themes`) into a `bundled-plugins/` directory inside the `.tar.gz` / `.zip`.
- **First-run bootstrap**: On startup, if `~/.local/share/merlin/plugins/` is empty and no plugins are configured, Merlin auto-extracts the bundled plugins so syntax highlighting, themes, and language server wiring work immediately out-of-the-box.

### 7.2 Native Tool Dependencies (LSP & Debuggers)
- **Problem**: Plugins are 100% portable wasm, but depend on host system processes (`rust-analyzer`, `clangd`, `lldb-dap`, `gdb`).
- **Solution**: The editor does not block installation if a process is missing on `$PATH`. Instead, the install confirmation card reads `install = "..."` hints from `plugin.toml` and surfaces a copyable command:
  > *"Installed Rust! Note: `rust-analyzer` was not found on your $PATH. Install it to enable completion and diagnostics: `rustup component add rust-analyzer` [Copy]"*

### 7.3 Wasm Optimization (`wasm-opt`)
- **Problem**: Unoptimized component `.wasm` files can be 2–3 MB each (e.g. `rust_language.wasm` is 2.9 MB).
- **Solution**: Both `merlin plugin pack` and the registry CI run `wasm-opt -Oz` (Binaryen) if installed, typically reducing wasm size by 40–60% and improving cold-start compilation times.

### 7.4 Cross-Platform Symlinks & Windows Fallback (`merlin plugin link`)
- **Problem**: Standard user accounts on Windows often lack permission to create UNIX symbolic links unless Developer Mode is enabled.
- **Solution**: In `merlin plugin link`, if `std::os::windows::fs::symlink_dir` fails with an access error, fall back to directory junctions or directory copying with a helpful notice.

### 7.5 Yanks & Security Revocations
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
git tag sdk-v0.9.1

# Tag the Editor
git tag v0.1.0

# Push both tags simultaneously
git push origin sdk-v0.9.1 v0.1.0
```

GitHub Actions triggers both workflows independently:
1. `sdk-release.yml` authenticates via OIDC and publishes the new SDK crates to crates.io.
2. `ci.yml` runs workspace tests across Ubuntu, macOS, and Windows, then builds the multi-platform release binaries.
3. Compatibility is guarded by `API_VERSION` / `API_OLDEST` ranges in `crates/plugin-host/src/lib.rs`.
