# Releasing merlin

How to cut an editor release, `v<version>`. What the workflows do is in
[distribution.md](distribution.md); this is the order to do things in.

A release's archives carry the plugins that come with merlin, taken from
merlin's registry ([merlin-editor/plugins](https://github.com/merlin-editor/plugins))
for the release's plugin API. So the registry has to have them first, and
the SDK for that API has to be on crates.io for anyone building plugins.
`scripts/release-check.sh` checks all of it.

## 1. Check

```bash
scripts/release-check.sh 0.1.0
```

It changes nothing, and says what's missing:

- the working tree is clean and at `origin/main`;
- the workspace `version` in `Cargo.toml` is the release's;
- the tag is free;
- CI passed on this commit;
- `merlin-plugin` at `sdk/rust`'s version is on crates.io;
- every plugin of merlin's own in the registry has its version from this
  commit's `plugin.toml` there, for this `API_VERSION`, built from a commit
  whose plugin folder matches this one.

Fix what it lists, in the order below, and run it again.

## 2. The version

Set `version` under `[workspace.package]` in `Cargo.toml` (the editor's
`merlin --version` prints it). Commit, push, and wait for CI.

Release notes go in the GitHub release itself (step 6); write them from
`git log v<previous>..main`.

## 3. The SDK, if the plugin API changed

When `API_VERSION` moved, the SDK's version moved with it. Push its tag
first:

```bash
git tag -a sdk-v0.15.0 -m "Plugin SDK 0.15.0, for plugin API 0.15"
git push origin sdk-v0.15.0
```

The run waits in the `release` environment until a reviewer approves it on
GitHub (Review deployments). Then it publishes `merlin-plugin`, and
`merlin-lsp` and `merlin-grammar` after it. crates.io never takes a version
back: a mistake is fixed with the next patch version.

## 4. The plugins, if any changed

The check names each plugin whose folder changed since the registry built
it, or that has no version for this API.

1. In this repository, bump that plugin's `version` in its `plugin.toml`
   (every plugin, after an API change). Commit and push to `main`.
2. In merlin-editor/plugins, add a `[[versions]]` table to each
   `plugins/<name>.toml` with the new version and the full commit SHA from
   step 1. If the plugin API or the SDK changed, set `MERLIN_REV` in
   `build/toolchain.env` to that commit too, in the same PR: it's the
   merlin the registry packs with.
3. Wait for Check (it builds each version twice and compares), merge, and
   wait for Publish to put the new `index.json` on Pages.

Run the check again: every plugin should be `ok`.

## 5. Tag

```bash
git tag -a v0.1.0 -m "merlin 0.1.0"
git push origin v0.1.0
```

`ci.yml` runs the tests on Linux, macOS and Windows; the `bundle` job fetches
the registry's packages with `merlin plugin fetch` and fails the release if
one is missing; then each platform's job builds its archive with the binary,
`README.md`, `LICENSE` and `plugins/`, and attaches it and its `.sha256` to a
draft release.

## 6. Try the draft, then publish

The draft has five archives. Check each has `plugins/` with 13 packages and
`bundled.toml`. Then install from it as a user would:

```bash
gh release download v0.1.0 --dir /tmp/merlin-v0.1.0
MERLIN_RELEASE_URL=file:///tmp/merlin-v0.1.0 sh scripts/install.sh
```

Start merlin in a folder: it should install the plugins that come with it
and say so in one notice. On a machine that already has merlin, use a
throwaway `HOME` (and `XDG_DATA_HOME`, `XDG_CONFIG_HOME`) so your own
plugins aren't touched.

Then write the release notes on the draft and publish it. `install.sh`
asks GitHub for the latest published release, so users get it from then on.

## If something goes wrong

- **A job fails before the draft exists**: fix it on `main`, delete the tag
  (`git push origin :refs/tags/v0.1.0` and `git tag -d v0.1.0`), and tag the
  fixed commit.
- **The draft is wrong**: delete the draft and the tag, and tag again. Nobody
  has it until it's published.
- **A published release is wrong**: publish the next patch version. Don't
  replace a published release's archives: their checksums are what
  `install.sh` checks against.
- **A plugin that came with it is bad**: release a fixed version to the
  registry and withdraw the bad one (`yanked = "why"`). merlin offers the
  update in Settings › Plugins; the next editor release bundles the fix.

Not yet: signed releases and updating merlin from inside merlin (section 9,
milestone B, of distribution.md), and signing for macOS and Windows.
