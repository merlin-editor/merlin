# Planning: Merlin Plugin Studio (The Inception Plugin)

This document outlines the design and phased implementation plan for **Merlin Plugin Studio** (`merlin-plugin-studio`), an in-editor plugin built with the Merlin Plugin SDK to create, edit, test, package, and publish Merlin plugins.

---

## 1. Overview & Motivation

Merlin's plugin model is built on WebAssembly components (`wasm32-wasip2`), WASI interfaces defined in WIT (`sdk/rust/wit/merlin.wit`), capability-based permissions, and deterministic distribution packages (`.merlin`).

With the recent and upcoming changes:
1. **SDK crates published to crates.io** (`merlin-plugin`, `merlin-lsp`, `merlin-grammar`, versioned with the plugin API).
2. **Flexible view placement** (`[[views]]` with `where = "tab" | "right" | "bottom" | "left"`).
3. **Richer UI nodes** (`chips`, `edit-card`, `progress`, `input`, `markdown`, `buttons`).
4. **Typed settings with user and workspace scopes** (`[settings.<key>]`).
5. **Per-plugin logging channels** (`log(level, msg)` into dedicated Output channels).
6. **Package format & registry distribution** (`.merlin` tarballs, `wasm-opt`, signed `index.json`): see `docs/plugin-registry.md`.

A dedicated **Plugin Studio** plugin serves as the premier dogfooding project: an inception-style development environment that allows developers to author, preview, test, and package plugins entirely inside Merlin.

---

## 2. Architecture & Permissions

### 2.1 Manifest Declaration

The plugin requires access to workspace files and build tools (`cargo`, `wasm-opt`, `git`), declared in its `plugin.toml`:

```toml
name = "merlin-plugin-studio"
title = "Plugin Studio"
version = "0.1.0"
api = "0.13"
component = "target/wasm32-wasip2/release/merlin_plugin_studio.wasm"

# Main visual workbench opened in an editor tab
[[views]]
id = "studio"
title = "Plugin Studio"
icon = "package"
where = "tab"

# Dev companion for quick actions in the sidebar
[[views]]
id = "devkit"
title = "Plugin DevKit"
icon = "wrench"
where = "right"

[permissions]
workspace-read = true
editor-write = true
processes = ["cargo", "wasm-opt", "git"]
```

### 2.2 Permissions Boundary
* **File Operations**: Uses `workspace-read` and `editor-write` (via `host.read-file`, `host.apply-edits`, `host.list-files`) to inspect project structures and scaffold code.
* **Process Execution**: Strictly limited to build tools (`cargo`, `wasm-opt`, `git`). It runs with standard host permissions upon user approval, keeping compilation out of the UI thread.
* **Host Compatibility**: Its `api` must equal the editor's `API_VERSION` (`0.13` today), and moves with it.
* **Note**: `cargo` runs build scripts, so asking for it means the studio can run anything you can (`docs/plugin-registry.md` 10.2). It falls in the registry's "outside the sandbox" review tier.

---

## 3. Core Modules & Features

```
┌────────────────────────────────────────────────────────────────────────┐
│                        Plugin Studio (Tab View)                        │
├───────────────────┬────────────────────────────────────────────────────┤
│ Navigation Rail   │ Main Content Area                                  │
│ ───────────────── │ ────────────────────────────────────────────────── │
│ • Overview        │ [ Manifest Editor (plugin.toml) ]                  │
│ • Manifest        │ - ID: rust-tools         Version: 0.2.0            │
│ • Views           │ - Permissions: [x] editor-read  [x] processes      │
│ • Settings        │ - Views: [[views]] id="cargo" where="rail"         │
│ • UI Sandbox      │                                                    │
│ • Packaging       │ [ Actions ]                                        │
│                   │ [ Build Component ]  [ Pack .merlin ]  [ Validate ]│
└───────────────────┴────────────────────────────────────────────────────┘
```

### 3.1 Module 1: Project Scaffolding
* **Templates**:
  * *Minimal*: Bare-bones WASI component with simple command and log.
  * *Tool / Sidebar*: Pre-configured with `[[views]]`, state management, and node rendering.
  * *Language Support*: Configured with `merlin-lsp` helper and language server process spawning.
  * *Theme / Icons*: Manifest-only distribution packaging.
* **Outputs**:
  * `Cargo.toml` with `merlin-plugin` dependency from crates.io.
  * `.cargo/config.toml` configuring target `wasm32-wasip2`.
  * `plugin.toml` with validated metadata and permission defaults.
  * `src/lib.rs` implementing `Guest` for the `merlin:plugin/plugin` world.

### 3.2 Module 2: Interactive Manifest (`plugin.toml`) Editor
* **Permissions Configurator**:
  * Toggles for `editor-read`, `editor-write`, `workspace-read`, `terminal`, `debugger`.
  * Process allowlist chip manager (e.g., adding `rust-analyzer`, `clangd`, `git`).
* **Views Designer**:
  * Add/remove `[[views]]`.
  * Placement selection (`left` / rail, `right` sidebar, `bottom` panel, `tab` editor).
  * Icon picker (Nerd Font glyphs or bundled SVG icons).
  * Empty-state fallback text.
* **Settings Schema Builder**:
  * Visual builder for typed settings (`flag`, `string`, `integer`, `float`, `secret`).
  * Scope selector (`workspace` vs `user-only`).
  * Choices list configuration (renders dropdowns in Settings › Plugins).

### 3.3 Module 3: UI Node Component Sandbox
* Mock layout previewer for `panel` node structures before compiling code:
  * Tests layout combinations of `heading`, `styled`, `button`, `chips`, `row`, `edit-card`, `progress`, and `markdown`.
  * Instant feedback on theme contrast and text wrapping without triggering wasm recompilation.

### 3.4 Module 4: Build, Hot-Reload & Diagnostics
* **Compilation**:
  * Executes `cargo build --target wasm32-wasip2 --release` via `host.spawn-process`.
  * Stderr streams into the plugin's dedicated Output channel (`log(log-level::info, ...)`).
* **Compiler Error Mapping**:
  * Parses `cargo` JSON diagnostics and emits them to Merlin's Problems view via `host.set-diagnostics`.
* **Hot-Reload Trigger**:
  * Upon clean compilation, signals the editor host to reload the component under test.

### 3.5 Module 5: Distribution & Packaging (`.merlin`)
* Runs `merlin plugin pack` rather than packing by itself, so the studio, an author's terminal and the registry's CI make the same bytes (`docs/plugin-registry.md` section 4):
  1. **Binary Optimization**: `wasm-opt -Oz` on the component, reporting before/after size reductions.
  2. **Deterministic Archiving**: `plugin.toml` (its `component` rewritten to `plugin.wasm`), the component as `plugin.wasm`, and assets, in a gzip tarball (`<name>-<version>.merlin`).
  3. **Registry Pre-flight Audit**:
     * Verifies `api` equals the editor's `API_VERSION`, and `version` is set.
     * Validates that all declared permissions are justified and minimal.
     * Shows the SHA-256 and size, and drafts the `plugins/<name>.toml` entry for a PR to `merlin-editor/plugins`.

---

## 4. Phased Implementation Roadmap

| Phase | Milestone | Deliverables | "Done When" Test |
|---|---|---|---|
| **Phase 1** | Scaffolding & Manifest Editor | Project creation templates, `plugin.toml` parser/writer, form controls in tab view. | Run "New Plugin", configure permissions in UI, and verify generated `Cargo.toml` and `plugin.toml` on disk. |
| **Phase 2** | Build & Dev Loop | Process execution (`cargo build`), dedicated Output logging, and diagnostic mapping. | Click "Build", observe build logs in Output view, and see compilation errors highlighted on code lines. |
| **Phase 3** | Component Sandbox | Visual previewer for `node` trees and layout combinations. | Author a custom UI view in the sandbox and verify it matches Merlin's native widget rendering. |
| **Phase 4** | Distribution & Packaging | `wasm-opt` pipeline, `.merlin` archive creation, SHA-256 calculation, and registry checklist. | Click "Package", produce optimized `.merlin` tarball, and verify archive contents against registry schema. |

---

## 5. Mockup & UI Reference

* Target UI style conforms to `docs/mockups/SettingsPages.dc.html` and `docs/mockups/PluginViewsMore.dc.html`.
* Primary interface uses native `node` variants (`chips` for lists, `edit-card` for code review, `buttons` for build actions, `row` for settings).
* Future enhancement (Phase 6 of plugin research): leverage custom drawing/tiled surfaces for visual node graph editing or UI tree diagrams.
