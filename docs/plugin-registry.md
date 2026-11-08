# Plugin Registry

How plugins get from an author to a user's editor through
[merlin-editor/plugins](https://github.com/merlin-editor/plugins). This is the
spec behind phases 1–4 of section 6 in [distribution.md](distribution.md).
Phase 1 (sections 2 and 4, and installing a package from disk) is built; the
rest isn't yet. The registry repo has its license, keys, `gh-pages` branch
and `registry` environment, and no plugins.

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
  locally, from a pinned Merlin release. A local pack and the registry's pack of
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
│   ├── Containerfile        the build image, pinned by digest in CI
│   └── toolchain.toml       the Rust version, and the Merlin release whose `merlin plugin pack` CI uses
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
4. Build twice in separate jobs inside the pinned container:
   `cargo build --locked --release --target wasm32-wasip2` with
   `RUSTFLAGS=--remap-path-prefix=$PWD=/build` and `SOURCE_DATE_EPOCH` set to
   the commit time, then `merlin plugin pack`. The two SHA-256s must match.
5. Comment on the PR with the archive size, the permissions it asks for, and how
   those differ from the previous version. `processes` and `terminal` are called
   out, since they reach outside the sandbox.

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

Built so far: `pack`, and `install` of a folder or a `.merlin` file.
Uninstalling is in Settings › Plugins: a linked plugin loses its link, and a
packaged one's folder goes to the system's bin and leaves `installed.toml`.

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
2. **Index and sources**: the `index.json` types, `minisign-verify`, serial and
   expiry checks, `[plugin-sources]`, `search`, `update`, `uninstall`.
3. **Registry repo**: the layout, container, `check.yml`, `publish.yml`,
   `resign.yml`, the key, and entries for Merlin's own plugins. Turn on GitHub
   Pages for `gh-pages`.
4. **Browse**: the Browse tab in Settings › Plugins, update badges, yank
   warnings, the permissions prompt. Needs an approved mock-up first.
5. **Bundling**: release archives carry the registry's packages.

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

## 12. Out of Scope

- **Paid plugins.**
- **Private plugins**, beyond what a private source (7.4) already covers.
