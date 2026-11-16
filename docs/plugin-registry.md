# Plugin Registry

How plugins get from an author to a user's editor through
[merlin-editor/plugins](https://github.com/merlin-editor/plugins). This is the
spec behind phases 1–4 of section 6 in [distribution.md](distribution.md).
Phases 1 to 3 are built: packages (sections 2 and 4), installing,
searching, updating and uninstalling from the command line against signed
indexes (sections 6 and 7), and the registry itself (sections 3 and 5), which
published its first index on 2026-11-09 with Merlin's 13 bundled plugins.
Phase 4, Browse in Settings › Plugins, is built too
(`docs/mockups/PluginBrowse.dc.html`, `crates/app/src/browse.rs`).

---

## 1. The Shape of It

```
author's repo ──PR──▶ merlin-editor/plugins ──CI──▶ GitHub Release   (<name>-<version>.merlin)
  (source,            (plugins/<name>.toml:        GitHub Pages     (index.json + .minisig)
   pinned commit)      repo, path, commit)                │
                                                          ▼
                                       merlin: fetch index, check signature,
                                       download, check hash, unpack, swap in
```

- **Source stays with the author.** The registry holds one small TOML entry per
  plugin, pointing at a public git repo, a folder in it and a pinned commit for
  each version. It never holds plugin source.
- **The registry builds everything.** Authors don't upload binaries. CI builds
  the pinned commit in a fixed container, so what users run is what the source
  says, and anyone can rebuild it and get the same bytes.
- **One packer.** CI runs `merlin plugin pack`, the same command authors run
  locally, from merlin built at a pinned commit (`build/toolchain.env`). A local pack and the registry's pack of
  the same commit are identical.
- **One signed file.** The editor trusts `index.json` because of its minisign
  signature; it trusts each archive because `index.json` gives its SHA-256.

---

## 2. Changes to `plugin.toml`

The registry needs a few things the manifest doesn't have yet. Like any manifest
change, these go in with an `API_VERSION` bump, the WIT, the SDK and every
`plugin.toml` updated together.

```toml
name = "rust"
title = "Rust"
version = "0.1.0"                 # new, required: semver, the registry's version
api = "0.13"
component = "target/wasm32-wasip2/release/rust_language.wasm"
license = "MIT OR Apache-2.0"     # new, optional: SPDX expression
repository = "https://github.com/merlin-editor/merlin"   # new, optional

[package]                         # new, optional: what goes in the archive
exclude = ["tools/", "*.svg.src"] # on top of the default excludes (section 4)
```

- **`version`** is required. The registry entry's version must equal it at the
  pinned commit.
- **`name`** must match `^[a-z][a-z0-9-]{1,39}$`. Names starting `merlin-` are
  reserved for plugins in this repo.

---

## 3. The Registry Repo

```
merlin-editor/plugins
├── plugins/
│   ├── rust.toml            one entry per plugin, named after it
│   └── word-count.toml
├── build/
│   ├── toolchain.env        the merlin commit whose `merlin plugin pack` CI uses
│   ├── apt.txt              what the container installs
│   ├── setup.sh             adds the wasm target and builds that merlin
│   ├── check.sh             builds new versions twice and compares them
│   ├── publish.sh           builds, releases and records new versions
│   └── sign.sh              writes, signs and pushes index.json
├── tools/
│   └── registry.py          validate, pending, build, index, summary
├── keys/
│   └── merlin.pub           minisign public keys (current and next)
├── .github/workflows/
│   ├── check.yml            on PRs
│   ├── publish.yml          on main
│   └── resign.yml           weekly
├── README.md                how to install from the registry
└── CONTRIBUTING.md          how to add or update a plugin, and the review rules
```

The `gh-pages` branch is written only by CI and served by GitHub Pages:

```
gh-pages
├── index.json
├── index.json.minisig
└── records/<name>/<version>.json    what the build produced (section 5)
```

### 3.1 An Entry: `plugins/<name>.toml`

```toml
repository = "https://github.com/merlin-editor/merlin"
path = "plugins/rust"          # folder holding plugin.toml; "" for the repo root
owners = ["marc-cr1810"]       # GitHub logins who may PR new versions

[[versions]]
version = "0.1.0"
commit = "31a1524f0c..."       # full 40-character SHA

[[versions]]
version = "0.1.1"
commit = "9be0c27d41..."
yanked = "Deletes the file on save; fixed in 0.1.2"   # optional
# blocked = "reason"   optional, maintainers only (section 10.5)
```

- Versions are only added, yanked or blocked, never edited or removed. A published
  `(name, version)` is immutable.
- A PR that changes an existing entry needs approval from one of its `owners`
  or a registry maintainer. A new entry needs a maintainer.

### 3.2 Merlin's Own Plugins

They stay in this repo, under `plugins/`. Their registry entries point at
`merlin-editor/merlin` with a `path`. That keeps them next to the editor and the
SDK they're tested with, and their path dependencies on `sdk/*` still resolve
because CI clones the whole repo at the pinned commit.

Third-party plugins must depend on the SDK from crates.io. `check.yml` rejects
path or git dependencies that point outside the plugin's repo.

---

## 4. The Package: `<name>-<version>.merlin`

A gzip-compressed tar file, built so the same input always gives the same bytes:

- **Layout**: files sit at the root of the archive, with no top-level folder.
  `plugin.toml` is always there.
- **Component**: the built component is stored as `plugin.wasm`, and the packed
  `plugin.toml` has `component = "plugin.wasm"`. The rewrite uses `toml_edit`,
  so comments and order survive. Data-only plugins (themes, icons, layouts) have
  no component.
- **Contents**: everything in the plugin's folder except the default excludes
  (`target/`, `src/`, `tests/`, `.cargo/`, `.git/`, `Cargo.toml`, `Cargo.lock`,
  and any `*.merlin` file) and anything in `[package] exclude`. An exclude is a
  glob from the plugin's folder: `*` stays within a folder, `**` crosses them,
  and a pattern that matches a folder leaves out everything in it. Then every file the manifest names (theme,
  pack and icon files) must be present, or packing fails.
- **Determinism**: ustar format; files only, with no entries for folders,
  sorted by path; every mtime is `SOURCE_DATE_EPOCH` (CI sets it to the pinned
  commit's time) or 0 without it; uid/gid 0 with no user or group names; mode
  0644; no symlinks (a symlink in the folder stops packing); gzip at the best
  level with no name or time in its header.
- **No optimisation step**: the component goes in as built. `wasm-opt` can't
  read components and, run on the module inside, made the gzipped package
  larger (distribution.md 7.3). With nothing but Rust and `merlin plugin pack`
  in the build, a local pack of the same commit matches CI's.
- **Limit**: 25 MB per archive, and 200 MB unpacked when installing.

`merlin plugin pack [dir]` writes `<name>-<version>.merlin` and prints its
SHA-256 and size.

---

## 5. CI

### 5.1 `check.yml` (pull requests)

For each entry the PR changes:

1. Validate the TOML: name rules, file name matches `name`, versions only added,
   commits are full SHAs, the PR author is an owner or maintainer.
2. Clone `repository` at `commit` and read `path/plugin.toml`. Its `name` and
   `version` must match the entry; its `api` must be one Merlin has released.
3. Check dependencies (section 3.2) and that `Cargo.lock` exists.
4. Build twice, one after the other, inside the container
   (`rust:1.97.1-slim-trixie`, pinned by digest in the workflows):
   `cargo build --locked --release` with the source, `CARGO_HOME` and
   `RUSTUP_HOME` remapped in `RUSTFLAGS` and `SOURCE_DATE_EPOCH` set to the
   commit time, then `merlin plugin pack`. The two SHA-256s must match.
   Every build is cloned to the same folder, `/build/src`: grammars' C code
   built from different folders gives different bytes even with the paths
   remapped, and the same folder gives the same bytes.
5. Write the archive size and what it asks for to the job summary.
   `processes`, `terminal` and `debugger` are called out, since they reach
   outside the sandbox, and so are programs that run anything (10.2). Comparing
   with the previous version's permissions is still to do.

### 5.2 `publish.yml` (push to `main`)

For each version that has no record on `gh-pages` yet:

1. Build and pack as above.
2. Create the GitHub Release `<name>-v<version>` with the `.merlin` attached. If
   that release already exists, fail; never overwrite.
3. Write `records/<name>/<version>.json`: SHA-256, size, URL, commit, `api`,
   permissions and the manifest fields Browse shows.
4. Regenerate `index.json` from every entry plus every record, with `serial`
   one higher than before and `expires` 30 days on. Yanks come from the entries,
   so a yank needs no rebuild.
5. Sign it, and push `index.json`, `index.json.minisig` and the new record to
   `gh-pages`.

### 5.3 `resign.yml` (weekly)

Regenerates and re-signs `index.json` with a new serial and expiry, so it never
expires while the registry is quiet.

### 5.4 The Signing Key

The minisign secret key is a GitHub Actions secret in a protected `registry`
environment that only `publish.yml` and `resign.yml` on `main` can use, the same
arrangement as the SDK's `release` environment. Signing offline would be safer
but would stop `resign.yml` working without a person.

The editor ships with two public keys: the current one and the next. To rotate,
sign with the next key, release an editor that ships the one after it, and
retire the old key once that release is out.

---

## 6. `index.json`

Served at `https://merlin-editor.github.io/plugins/index.json`, with
`index.json.minisig` next to it.

```json
{
  "schema": 1,
  "serial": 128,
  "generated": "2026-11-09T12:00:00Z",
  "expires": "2026-12-09T12:00:00Z",
  "plugins": {
    "rust": {
      "title": "Rust",
      "description": "Syntax, tests, Cargo tasks & rust-analyzer",
      "badge": "RS",
      "hue": "orange",
      "license": "MIT OR Apache-2.0",
      "repository": "https://github.com/merlin-editor/merlin",
      "path": "plugins/rust",
      "owners": ["marc-cr1810"],
      "versions": [
        {
          "version": "0.1.0",
          "api": "0.13",
          "commit": "31a1524f0c...",
          "published": "2026-11-09T12:00:00Z",
          "url": "https://github.com/merlin-editor/plugins/releases/download/rust-v0.1.0/rust-0.1.0.merlin",
          "sha256": "e3b0c442...",
          "size": 1184022,
          "permissions": {
            "editor-read": true,
            "editor-write": true,
            "processes": ["rust-analyzer"],
            "workspace-read": true,
            "terminal": true
          },
          "depends": [],
          "yanked": null,
          "blocked": null
        }
      ]
    }
  }
}
```

The editor accepts an index only if:

- the signature checks out against one of its keys;
- `schema` is one it understands;
- `expires` is in the future, which stops an old index being replayed forever;
- `serial` is at least the last serial it accepted from this source, which stops
  a rollback to an older index. The last serial is kept per source in the data
  folder.

If any check fails, Browse says why and keeps showing the last good index; it
installs nothing from a bad one.

---

## 7. In the Editor

### 7.1 Choosing a Version

The version to install or update to is the newest one that isn't yanked and whose
`api` equals the editor's `API_VERSION`. When a plugin has no such version, Browse
lists it as "not yet updated for this version of Merlin" and offers nothing to
install.

While the API keeps breaking, every API bump means a new version of every plugin.
For Merlin's own plugins, the editor release adds new registry versions for them
at the release commit (a PR to the registry opened by `ci.yml`). Third-party
authors release their own.

### 7.2 Installing and Updating

1. Download into `~/.cache/merlin/downloads/`. Check size and SHA-256 against
   the index.
2. Unpack into `~/.config/merlin/plugins/.staging/<random>/`. Refuse
   absolute paths, `..`, symlinks, hard links and device files.
3. Read the unpacked `plugin.toml`: `name`, `version` and `api` must match the
   index.
4. Ask for its permissions (7.3).
5. Move the current `~/.config/merlin/plugins/<name>/` to
   `.trash/<random>/<name>`, move the staging folder into its place, delete the
   trash. On start, Merlin clears `.staging/` and puts back anything in
   `.trash/` whose name has nothing in its place.
6. Record it in `installed.toml` in the data folder: name, source, version and
   SHA-256. Update checks read this.

A plugin linked with `merlin plugin install <dir>` is a symlink, not a registry
install. The registry never updates or replaces it, and Browse shows it as
"linked from <dir>".

### 7.3 Permissions

Browse shows a version's permissions before anything downloads, worded as
`merlin plugin install` words them today. Installing grants them, as installing
from a folder does now. Grants stay keyed by the plugin's folder, which for a
registry install is always `~/.config/merlin/plugins/<name>/`, so they carry over
across updates.

An update that asks for more than the current grant covers doesn't install by
itself: Settings › Plugins shows it with what it adds, and the user allows it or
skips it.

### 7.4 Sources

The registry above is the built-in source `merlin`. Others go in user settings,
never workspace settings:

```toml
[plugin-sources.acme]
url = "https://plugins.acme.dev/index.json"
key = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3"

[plugin-sources.merlin]
enabled = false            # turns the built-in one off
```

A plugin remembers the source it came from and updates only from there. If two
sources have the same name, Browse shows both with their source, and installing
one in place of the other asks first.

### 7.5 Commands

| Command | Does |
|---|---|
| `merlin plugin pack [dir]` | Builds the `.merlin` and prints its SHA-256 |
| `merlin plugin install <name \| file.merlin \| dir>` | From the registry, a local archive, or links a folder (today's behaviour) |
| `merlin plugin search <text>` | Searches every enabled source |
| `merlin plugin update [name]` | Updates one or all; asks again if permissions grew |
| `merlin plugin uninstall <name>` | Removes it and its grant |

All five are built (`crates/app/src/plugin_cli.rs`, with the index logic in
`crates/plugin-host/src/registry.rs`). `install` also takes `name@1.2.0` for a
version and `source:name` to pick a source; a plugin with the same name in two
sources needs one. Uninstalling, here or in Settings › Plugins, forgets the
plugin's grant: a linked plugin loses its link, and a packaged one's folder goes
to the system's bin. The last good index of each source is kept in
`sources/<name>/` in the data folder with its serial, for 6's checks and for
searching offline; installing needs a fresh one.

---

## 8. Bundled Plugins

The editor release's archives carry Merlin's own plugins (distribution.md 7.1).
`ci.yml` takes them from the registry's `.merlin` files for the release's commit,
so a bundled plugin is byte-for-byte the registry's. On first run they unpack
through the same steps as 7.2 and are recorded with source `merlin`, so they
update from the registry like any other.

---

## 9. Build Order

1. **Manifest and pack (done, API 0.14)**: `version`, `license`,
   `repository`, `[package]`; `merlin plugin pack`; deterministic archives;
   installing a local `.merlin` with staging and swap; `installed.toml`. The
   grant message lists every permission; `merlin plugin install` builds on
   Windows and copies a folder when links aren't allowed; the WIT package
   version is the API version.
2. **Index and sources (done)**: the `index.json` types, `minisign-verify`,
   serial and expiry checks, `[plugin-sources]`, `search`, `update`,
   `uninstall`. `merlin plugin update` reports a blocked plugin; the editor
   turns it off (phase 4).
3. **Registry repo (done)**: the layout, container, `check.yml`, `publish.yml`,
   `resign.yml`, the key, and entries for Merlin's own plugins, pinned to
   merlin `1c58427`. GitHub Pages serves `gh-pages`.
4. **Browse (done)**: Browse beside Installed in Settings › Plugins, as in
   `docs/mockups/PluginBrowse.dc.html`. merlin checks every source on its own
   thread a few seconds after the window opens, every 6 hours, and when
   Browse opens on a list older than 10 minutes; ↻ checks at once. Installs
   and updates run there too, one at a time, with progress in the status bar,
   and the plugin starts without a restart. Install asks first only for a
   plugin that runs programs outside the sandbox, or that replaces the same
   name from another source. Installed lists Updates first, with Update All
   for those that ask for nothing new; one that asks for more waits on its
   page for Allow and Update or Skip This Version (kept in
   `skipped-updates.toml` in the data folder). Withdrawn (yanked) versions
   and blocked plugins get banners; a blocked one is turned off, at start
   too from the last good index, and can't be turned back on. Banners say
   when a source can't be reached, its index expired, or failed a check.
   The window and the CLI say "withdrawn" for yanked.
5. **Bundling (built)**: release archives carry the registry's packages in
   `plugins/`, fetched by `merlin plugin fetch`; merlin installs them on first
   run and when an update brings newer ones (distribution.md 9, A3).

---

## 10. Reviewing Plugins

The sandbox does most of the protecting, so review effort follows what a plugin
is allowed to do. A plugin that can only read and colour the open file can't do
much harm; one that can start programs can do anything you can.

### 10.1 Tiers

| Tier | Permissions | New plugin | Update from an owner |
|---|---|---|---|
| **Data** | No component (themes, icons, layouts) | Automated checks, then a maintainer merges | Auto-merge when CI passes |
| **Sandboxed** | `editor-read`, `editor-write`, `workspace-read` | Maintainer checks it does what its description says | Auto-merge when CI passes and permissions didn't grow |
| **Outside the sandbox** | `processes`, `terminal`, `debugger` | Written reason for each, and a maintainer reads the code that uses them | Auto-merge only if permissions didn't grow; otherwise the same review as new |

An update that asks for more than the previous version always gets a human
review, whatever its tier. `check.yml` works out the tier and the permission
diff and labels the PR, and a bot merges the auto-merge cases.

### 10.2 What `processes` Really Grants

`spawn-process` checks the program's name against `processes`, but passes any
arguments (`crates/plugin-host/src/state.rs`). So `processes = ["sh"]` lets a
plugin run anything, and so does any shell, interpreter or build tool: `bash`,
`python`, `node`, `cargo`, `make`. The review question for a process isn't "is
it safe" but "is it the tool this plugin is obviously for": `rust-analyzer` for
Rust, `clangd` for C++.

- `check.yml` keeps a list of programs that can run arbitrary code and labels a
  PR that asks for one. Such a request needs a maintainer's sign-off even when
  the tool is plainly the right one (a Python plugin asking for `python`).
- The install prompt says it plainly: "can run `python`, which can run
  anything", not just "can run `python`".

### 10.3 Automated Checks (Every PR)

- The repository is public, and the plugin has an SPDX `license`. A public
  source is required anyway, for reproducible builds.
- The name follows the rules, isn't reserved, and isn't confusingly close to an
  existing one (edit distance 1 or 2, or the same once `-` is removed). Close
  names need a maintainer.
- `title` and `description` are present, and the archive is within the size limit.
- The build is reproducible (5.1).

### 10.4 Owners

- Ownership belongs to GitHub logins in `owners`, not to the source repo. If
  the repo changes hands, published versions don't change, because they're
  pinned commits, and new versions still need an owner's PR.
- Owners are added or removed by a PR that an existing owner approves, or that
  a maintainer approves after trying to reach them for 30 days.

### 10.5 Reports, Yanks and Blocks

- Browse has **Report** on each plugin, which opens an issue on the registry
  repo from a template.
- **`yanked = "reason"`** on a version: not offered for new installs, and users
  who have it see a warning and are offered an update. Owners or maintainers
  can yank.
- **`blocked = "reason"`** on a version or a whole entry: for malicious or
  dangerous plugins. Only maintainers can set it. Editors that have it turn it
  off on their next index fetch and say why; the user can remove it but can't
  turn it back on. `blocked` goes in the entry and in `index.json` next to
  `yanked`.

Because the editor only acts on a signed index, a block reaches users through
the same channel as everything else and can't be forged.

---

## 11. API Compatibility

Today a plugin loads only if its `api` equals the editor's `API_VERSION`. That's
right while the API is breaking freely, but it means every API bump strands every
third-party plugin until its author rebuilds, and a user who updates the editor
loses those plugins until then.

### 11.1 Now (0.x): Keep the Exact Match, Make Bumps Painless

- **Warn before it bites.** Before updating the editor (the update card in
  distribution.md section 5), Merlin checks the index: "2 of your plugins don't
  have a version for this update yet: Go, Catch2." The user can update anyway or
  wait.
- **Tell authors.** When an editor release bumps the API, `publish.yml` opens an
  issue on each third-party plugin's repo saying so, with a link to what changed.
- **Merlin's own plugins** get new versions in the same release, as in 7.1.
- **The WIT package version is the API version** (`merlin:plugin@0.14.0` for
  API `0.14`), bumped together. That's needed for 11.2. It also means a
  component built for another API fails to link, whatever its manifest says.
  Later, `merlin plugin pack` can read the version the component imports and
  check the manifest's `api` against it.

### 11.2 From 1.0: Additive Minors, Adapters for Majors

Once the API is stable enough to call 1.0, bumps are major or minor:

- **Minor (1.3 → 1.4): only adds.** New imports, new optional exports, new
  manifest fields. Changing or removing anything is a major bump. Wasmtime's
  component linker already matches semver-compatible versions, so a plugin built
  against `merlin:plugin@1.3.0` links to a host that provides `@1.4.0` with no
  host code. The host accepts any plugin with the same major and an equal or
  older minor.
- **Major (1.x → 2.0): breaks, with a bridge.** The host keeps the previous
  major's WIT (`wit/v1/`), generates bindings for it as well, and adapts its
  calls onto the current host code. That gives `API_OLDEST`: the oldest major
  the host still loads. Supporting the current and the previous major is
  enough; the bridge for a major is deleted one major later. Zed's extension
  host works this way.
- **Exports** (functions the host calls in the plugin) are the hard part of a
  bridge: if one changes shape, the adapter calls the old one and converts the
  result. Keeping export changes to additions with defaults avoids most of that.

The registry's version choice (7.1) becomes: the newest version whose `api` the
editor accepts, rather than equals.

### 11.3 When to Go 1.0

When the API has gone a month or so of feature work without a breaking change,
and the registry has a few third-party plugins whose authors would feel the
breaks. Until then, the cost of keeping old WIT around is higher than the cost of
rebuilding.

---

## 12. Plugin API 0.15 (Built 2026-11-16, for merlin v0.2.0)

Everything here changes the manifest or the WIT, so it goes in one bump:
`API_VERSION`, the WIT (`merlin:plugin@0.15.0`), the SDK (0.15.0 on
crates.io) and all 13 of merlin's plugins together, released as merlin
v0.2.0. No plugin uses `depends` today, so changing its form breaks nothing
that exists.

The UI parts need approved mock-ups before they're built (12.8).

### 12.1 How to Install a Program

Today only `[[debuggers]]` has an `install` hint, and it's one command for
every system (`sudo apt install lldb`, which is wrong on a Mac). Programs
named in `processes` have nothing.

A plugin describes each program it starts in a `[programs.<name>]` table:

```toml
[permissions]
processes = ["clangd"]

[programs.clangd]
for = "Completion, errors and go to definition"
install = { linux = "sudo apt install clangd", macos = "brew install llvm", windows = "winget install LLVM.LLVM" }
url = "https://clangd.llvm.org/installation"

[programs.rust-analyzer]
for = "Completion, errors and go to definition"
install = "rustup component add rust-analyzer"   # the same everywhere
url = "https://rust-analyzer.github.io/"
```

- **`install`** is one string when the command is the same on every system,
  or a table with any of `linux`, `macos`, `windows`. A system the table
  leaves out gets no command, only the link.
- **`url`** is where to read more. It's the fallback for every system the
  table misses, and for Linux distributions where `apt` is wrong.
- **`for`** says what the plugin can't do without it, in a few words. merlin
  shows it as "clangd isn't installed. Without it: completion, errors and go
  to definition."
- Every key is optional. `merlin plugin pack` warns, but doesn't fail, when a
  name in `processes` has no `[programs]` entry, and fails when a
  `[programs]` entry isn't in `processes` or a debugger's `command`.
- `[[debuggers]] install` takes the same string-or-table form, and gains
  `url`.

`tools/registry.py` copies `programs` into each version in `index.json`, so
Browse can say what's missing before anything downloads.

### 12.2 Showing What's Missing

The plugin decides what it needs (its manifest); merlin checks whether it's
there. merlin looks each program up on `PATH` (the shell's `PATH`, from
`shell_path.rs`) when a plugin starts, when Browse or the plugin's page
opens, and again when the window gets focus, so installing in a terminal
and coming back clears the message without a restart.

Where it shows:

- **The language card** in the status bar, which has a "not installed"
  state in its approved mock-up: the program, what's missing without it,
  the command with Copy, and the link.
- **Browse**, on a plugin's entry: "Needs clangd" under its permissions,
  before install, from the index.
- **The plugin's page** in Settings › Plugins: a Programs list, each found
  (with the path it was found at) or not (with the command, Copy and the
  link).
- **After installing**, the install card names any that are missing.

A plugin can still call `spawn` for a program that isn't there and get the
error back, as now. Nothing here stops a plugin installing or starting.

### 12.3 Systems and Processors

A plugin, or one part of it, can say where it works:

```toml
systems = ["linux", "macos"]          # absent means every system
arch = ["x86_64", "aarch64"]          # absent means every processor

[[debuggers]]
id = "cppvsdbg"
systems = ["windows"]
# ...

[programs.debugpy]
systems = ["linux", "macos", "windows"]
```

- The values are Rust's `std::env::consts::OS` and `ARCH`: `linux`,
  `macos`, `windows`; `x86_64`, `aarch64`. Those are what merlin is
  released for. `pack` refuses anything else, so a typo doesn't hide a
  plugin everywhere.
- **Versions choose.** The version Browse installs or updates to (7.1)
  becomes the newest that isn't yanked, has this `api`, and lists this
  system and processor. So 0.3 can drop Windows while Windows keeps getting
  0.2, and a later version can add it back.
- **Browse** greys out a plugin with no version for this system: "Not
  available for Windows." Search still finds it.
- **The first-run install** of bundled plugins skips them.
- **A linked plugin** (`merlin plugin install <dir>`) whose manifest leaves
  this system out doesn't load, and its page says why.
- A debugger or program left out for this system is as if the manifest
  didn't have it: not tried, not shown as missing.

### 12.4 Dependencies

Today `depends = ["name"]` only stops a plugin starting when the other
isn't installed, and the user has to find and install it themselves.

```toml
[depends]
merlin-lsp = "0.2"        # an example; a semver requirement, read as Cargo reads it: ^0.2
```

- **Installing** a plugin installs what it depends on, in one step. The
  permission card lists each plugin that will be installed and what each
  asks for; one Install covers all of them.
- **Choosing versions:** for each dependency, the newest version that meets
  the requirement, the `api` and the system (12.3). One already installed
  that meets it stays as it is. One that doesn't is updated, if a version
  that meets every requirement on it exists.
- **Conflicts:** only one version of a plugin is installed at a time. When
  no version meets everything, nothing installs, and the card says which
  plugins want what: "Go needs merlin-lsp 0.3; Rust needs 0.2."
- **Updates** follow the same rule: an update that would break another
  plugin's requirement waits, and says why.
- **Removing** a plugin that others need asks first: "Go and Rust need
  merlin-lsp." `installed.toml` records whether a plugin was installed by
  the user or as a dependency; one installed as a dependency that nothing
  needs any more is offered for removal along with the last plugin that
  needed it.
- A dependency only means "install and start this first". It doesn't give
  the plugins a way to call each other; that's out of scope.
- `index.json` already has `depends` per version; it becomes the table.

### 12.5 Network Access

A plugin can reach the network today only by starting a program, which
needs `processes` and could do anything. Plugins that want one API (a
GitHub pull request view, a docs search) need far less.

```toml
[permissions]
network = ["api.github.com", "*.githubusercontent.com"]
```

- A new host function, `fetch(request) -> result<u32, string>`, starts an
  HTTPS request and returns its id at once. The response comes to a new
  export, `on-fetch(id, response)`, as process output comes to
  `on-process`, so a slow server doesn't stall the plugin.
- Only HTTPS, only the hosts listed. `*.` matches any subdomain, not the
  domain itself. Redirects are followed only to listed hosts.
- The permission reads "Connects to api.github.com and
  *.githubusercontent.com". An update that adds a host asks again (7.3).
- Bodies are capped (10 MB in, 10 MB out), and a plugin has at most 8
  requests open at once.
- `processes` stays as it is. A plugin that starts a program which uses the
  network itself (an agent CLI, `gh`) only needs `processes`, and its
  permission says so already (10.2).

### 12.6 Agents

An agent is a program that takes a prompt and works on the project:
Claude Code, opencode, Antigravity and others. They speak different
protocols, so merlin doesn't talk to any of them itself. A plugin starts
the program (`processes`) and translates. merlin draws the chat, and the
plugin can add to it as much as it likes, or draw a view of its own
(below: What the Plugin Draws). What keeps the user safe is always
merlin's: the permission card, reviewing what changed, and commands in
terminal tabs.

```toml
[permissions]
processes = ["claude"]
agent = true

[[agents]]
id = "claude"
name = "Claude Code"

[programs.claude]
for = "Chatting with Claude in merlin"
install = "npm install -g @anthropic-ai/claude-code"
url = "https://docs.claude.com/en/docs/claude-code/setup"
```

The WIT, as built (`sdk/rust/wit/merlin.wit`):

- **Exports** (merlin calls the plugin):
  - `on-agent-prompt(chat, agent, text, context)`: the user sent a message
    to one of its agents; `context` is the open file, or its selected lines.
  - `on-agent-cancel(chat)`: the user pressed Stop.
  - `on-agent-close(chat)`: the tab closed, or New Chat started over.
  - `on-agent-answer(chat, request, answer)`: the user answered a
    permission card: allow once, always allow, or deny. A request the user
    always allows is answered at once, with no card.
  - `on-agent-file(chat, request, got)`: a file read, or a write done.
  - `on-agent-run(chat, run, ended)`: a command ended: all it printed and
    its exit code, or why it didn't run.
- **Host functions** (the plugin calls merlin):
  - `agent-say(chat, part)`: text, a thought, the plan, a step (read,
    edit, run, search) with its state, or panel pieces, streamed in pieces.
    A step sent again with the same id updates its line.
  - `agent-header(chat, chips, note)`: the chat's header, under its tab:
    chips, such as its model, and a quiet note at the right, such as what
    the chat has used. Empty clears it.
  - `agent-commands(chat, commands)`: the agent's slash commands, each a
    name and what it does. Typing `/` lists those that match.
  - `agent-ask(chat, request) -> u32`: shows the permission card for an
    edit (its path and diff), a command, a read, or anything else in words.
  - `agent-done(chat, outcome, message)`: finished, cancelled, or failed
    with what the agent said.
  - `agent-read(chat, path) -> u32`: a file's text as merlin has it, with
    unsaved changes in an open tab; the text comes to `on-agent-file`.
  - `agent-write(chat, path, text) -> u32`: replaces an open tab's text as
    one edit, which Undo reverts, and saves it; a file that isn't open is
    written on disk.
  - `agent-run(chat, command, cwd) -> u32`: runs a command line in a
    terminal tab named for the agent, which the user can watch and stop;
    `agent-stop(run)` ends it. Its output comes when it ends, to
    `on-agent-run`; ACP's `terminal/output` before then has what the plugin
    has. (The plan's `agent-output` was left out: merlin's terminal tabs
    hand a run's output over when it ends.)
- **Where files can be:** `agent-read` and `agent-write` work inside the
  workspace's folders. A path outside them gets a permission card first.
  A write the agent didn't ask permission for (with `agent-ask`) gets one
  too, so an agent can't edit without the user seeing it.
- **Reviewing:** merlin keeps each file as it was before an allowed edit,
  and after a turn lists the files that changed with their diffs, and
  Revert for each. With `agent-write`, merlin knows each edit exactly.
  When the program writes the file itself (the Claude CLI does), merlin
  compares against the copy it kept when the edit was allowed.
- **"Always allow"** is kept per workspace and per tool, in merlin's data
  folder, not in the plugin.
- `agent = true` asks "Adds an agent to the chat panel". With it come
  `agent-read`, `agent-write` and `agent-run`, limited as above; it needs
  neither `editor-write` nor `terminal`. The program itself can still do
  anything the user can, and `processes` already says so.

**What the plugin draws** (AgentUi.html, approved 2026-11-16):

- **Pieces in a message:** `agent-part::panel(list<node>)` puts panel
  nodes in the agent's message, as plugin views use: rows, buttons, a
  progress bar, a picture. Clicks reach the plugin's `on-click`.
- **A header:** `agent-header`'s chips; a click reaches `on-click`, and a
  plugin usually answers with `show-picker`.
- **Slash commands:** `agent-commands`. The menu shows up to eight that
  match what's typed; Enter, with a command half typed, finishes it.
- **A view of its own:** `[[agents]]` `view = "<view id>"` names one of
  the plugin's `[[views]]` (it must be on the right), shown in place of
  merlin's chat. The plugin numbers the chat itself: the first
  `agent-say` or `agent-ask` from a chat makes it. merlin puts its
  permission card, and a finished turn's changes, at the bottom of the
  view.
- **merlin's marks:** ids starting `agent:` are merlin's. In a plugin's
  pieces in the chat they aren't clickable; in a view of its own, only the
  cards merlin adds use them. merlin's permission cards say "merlin asks"
  with a lock, which no plugin piece can draw.

**The chat, reworked** (AgentChat.html, approved 2026-11-16). Built in
this order, each step committed and checked against its cards:

1. API: `agent-modes(chat, modes, current)`, each mode with its name, a
   hue (merlin's, or `#rrggbb` softened to fit the theme) and how it asks
   (reads only, asks first, edits on its own, edits and safe commands,
   never asks); `on-agent-mode(chat, mode)` when the user picks one;
   `agent-usage(chat, used, size, note)` for the context ring and the
   turn's footer; `agent-request::mode(list<string>)` for leaving a mode,
   answered with `agent-answer::mode(string)`. `agent-header`'s chips move
   into the box; its note becomes the ring's hover.
2. merlin-acp: the `mode` config option (or `modes`) as modes, with the
   suggested colours; `usage_update` as usage; a permission whose tool is
   `switch_mode` as a mode request. opencode: build blue, plan purple
   (read-only). Antigravity: Default blue, Accept edits green, Auto orange,
   Plan purple.
3. The box (cards 1, 3, 13, 17, 18, 21): attachments above the text; the
   mode badge, merlin's edit pill (only when the mode doesn't say how it
   asks), the plugin's chips, the ring, Send or Stop; Enter sends, Tab
   switches mode, Shift Tab the edit pill; the chat tinted by the mode.
4. Messages (5, 14, 18): yours as a bubble, the agent's with a line in its
   mode's colour; steps folded per run; the footer under each reply.
5. Above the box (4, 7): the plan, a queued message, and the changed files,
   kept across turns until kept or undone.
6. The editor: Undo and Keep under each change, and the floating bar.
7. Cards (6, 15, 16, 21): keys 1 to 3, "tell it what to do instead", the
   mode hand-over, and asking once before a mode that never asks.
8. @ (2), hover Edit, Copy and Rewind (8), chats kept and listed (9), ↓ and
   Alt ↑ ↓ (10), Follow Along (11), tab dots (12).

**ACP is a library, and each agent is its own plugin** (decided
2026-11-16). merlin's agent API above is the layer between merlin and every
agent plugin; ACP is what many agent programs speak on their side.

- **`merlin-acp`**, a crate in the SDK (`sdk/acp`, published with
  `merlin-plugin`), speaks ACP so a plugin needn't. A plugin names its
  agent's program and arguments and exports `Acp`:
  `merlin_plugin::export!(Acp => Acp::new(&[OPENCODE]))`. Anyone can build an
  agent plugin on it.
- **One plugin per agent**, in the registry and not in merlin's download:
  `antigravity` (`agy acp`) and `opencode` (`opencode acp`), built
  2026-11-16 and tried in the window (Antigravity with agy 1.5.9): each
  answered, edited a file, and its turn ended with the change to review.
  Installing one is turning that agent
  on: its permission card says which program it starts. merlin comes with
  no agent.
- **How ACP maps:**

  | ACP | merlin |
  |---|---|
  | `initialize`, `session/new` | the plugin, when a chat first sends |
  | `session/prompt` | from `on-agent-prompt`; the open file as a `resource_link` |
  | `session/update`: message and thought chunks, tool calls, plans | `agent-say` |
  | `available_commands_update` | `agent-commands` |
  | `configOptions` (selects), or older agents' `models` and `modes` | header chips; picking one sends `session/set_config_option`, `set_model` or `set_mode` |
  | `usage_update`, a turn's `usage` | the header's note: "12.4k tokens · $0.03" |
  | `session/request_permission` | `agent-ask`, answered by `on-agent-answer` with the matching option |
  | `session/cancel` | from `on-agent-cancel`; open cards answered `cancelled` |
  | `fs/read_text_file`, `fs/write_text_file` | `agent-read`, `agent-write` |
  | `terminal/create`, `wait_for_exit`, `kill`, `release` | `agent-run`, `on-agent-run`, `agent-stop` |
  | `terminal/output` | what the run printed, once it ended |

  The plugin tells the agent in `initialize` that merlin reads and writes
  files and runs commands. An agent that edits files itself anyway, as
  opencode does, is still reviewed: a file its last turn says it edits
  that changes on disk goes into the turn's changes, as it was before.
- **When an agent fails,** its whole message shows in the chat, and in its
  plugin's Output with what it printed on stderr. Gemini CLI, tried with a
  personal Google account, answered "This client is no longer supported
  for Gemini Code Assist for individuals", pointing to Antigravity, so the
  plugin for Google's agent is `antigravity`, on `agy acp` (agy 1.5 and
  later).
- **`claude`**, on the Claude CLI's stream-json mode, follows in 0.2.x. It
  needs no API change.

### 12.7 Also in v0.2.0 (No API Change)

- **Automatic plugin updates**, on by default, for updates that ask for no
  new permissions; **Keep at this version** on a plugin's page to stop them
  for that plugin.
- **Removing settings on uninstall:** the uninstall card offers to delete
  the plugin's settings and stored data too (off by default).
- **Replaced by:** a registry entry can say `replaced-by = "<name>"`. Browse
  and the plugin's page then say "Replaced by X" and offer to install it.
  This is in the registry's entry file and the index, not the manifest.
- **Warn before an editor update** (11.1): the update card names installed
  plugins that have no 0.15 version yet.

### 12.7.1 In the Index

`tools/registry.py` in merlin-editor/plugins copies a version's `systems`,
`arch` and `programs` into `index.json`, and its `[depends]` table as
`requires` (with `depends` keeping just the names). `network` and `agent`
go in `more-permissions`, not `permissions`: merlin 0.1 refuses keys it
doesn't know in `permissions`, and with them the whole index. An entry
file can say `replaced-by`, the plugin that takes its place, and
`replaced-note`, a sentence from the registry's maintainers or the old
plugin's owners, such as "Python does everything Pyright did, and runs
tests too." merlin shows it after "Pyright is replaced by Python."

### 12.8 Mock-ups (Approved 2026-11-16)

1. Missing programs: the Browse entry, the plugin page's Programs list, and
   the install card's note.
2. Systems: a greyed-out Browse entry.
3. Dependencies: the install card listing several plugins, the conflict
   message, and removing a plugin others need.
4. Network: the permission line (small; may fold into 3).
5. Agents: the chat panel, the permission card (Allow once / Always allow /
   Deny), and reviewing a turn's changes. Then AgentUi.html: what agent
   plugins add to the chat.
6. Automatic updates and Keep at this version, removing settings on
   uninstall, and the Replaced by notice.

### 12.9 Build Order

1. Manifest: `programs`, the install form, `systems`/`arch`, the `depends`
   table, `network`, `agent`, `[[agents]]`; `pack`'s checks; tests.
2. Version choice and the dependency resolver in `registry.rs`, with tests
   for conflicts and updates.
3. WIT and host: `fetch`/`on-fetch`, the agent functions; the SDK.
4. The UI, from the approved mock-ups.
5. Bump all 13 plugins: `api = "0.15"`, a new `version`, real install
   tables for clangd, rust-analyzer, gopls, pyright, typescript-language-server,
   lldb, gdb and debugpy.
6. Publish SDK 0.15.0, open the registry PR for the 13 plugins, merge it,
   and only then tag v0.2.0: a merlin that updates itself must find 0.15
   versions of the plugins it has installed from the registry. Its
   release notes say "Plugin API 0.15": the update card of a merlin with
   another API reads that line to name plugins that won't work after it.

---

## 13. Out of Scope

- **Paid plugins.**
- **Private plugins**, beyond what a private source (7.4) already covers.
