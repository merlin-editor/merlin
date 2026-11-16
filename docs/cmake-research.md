# CMake: what merlin is missing

Research of 2026-11-17 on the CMake plugin (`plugins/cmake`) against VS
Code's CMake Tools, CLion and Qt Creator, and what their users vote for.
New UI in it needs a mock-up the user approves first.

## Where the plugin stands

Built 2026-11-16 and 17:
- Colours for `CMakeLists.txt` and `.cmake` files, from its own lexer.
- neocmakelsp for completion, hover, go to definition, formatting and
  symbols, when it's installed. It completes only for editors that take
  snippets, so the LSP SDK now inserts snippets as plain text.
- Configure, Build and Test tasks from `CMakePresets.json` and
  `CMakeUserPresets.json`, following `inherits`, or into `build/` without
  presets.
- The configured build shared as `compile-commands`, so the C/C++
  plugin's clangd reads it wherever the preset builds.

## What other editors do

**VS Code's CMake Tools** is what most people use.
- A side bar with the active presets and targets, and Configure, Build,
  Test, Debug and Launch on them. The status bar shows the build preset,
  the target, and Build, Debug and Run.
- Debugging a target needs no `launch.json`: it asks once which target,
  remembers it, and shows it in the status bar.
- CTest's tests in the Test Explorer, with Run and Debug over them in
  the editor. Debugging a test takes its program, arguments and folder
  from CTest.
- Configures when `CMakeLists.txt` is saved, which can be turned off.

**CLion** builds its whole project model on CMake.
- Profiles hold the toolchain, build type, generator and environment, and
  presets turn into profiles.
- A run configuration for every target, picked from the toolbar.
- A CMake tool window with the configure's output and status, Reload
  CMake Project, and Reset Cache.
- The cache's variables in a table, searchable, with descriptions, and
  switches for the booleans.

**Qt Creator** reads CMake's file API.
- The project tree comes from it, by target, and it can add new files to
  a target.
- It writes the file API query itself, and runs CMake to answer it.
- A CMake debugger, over DAP, with CMake 3.27 or later.

## Most voted, by 👍

| Request | Votes | Tracker | merlin |
|---|---|---|---|
| Colours for `CMakeLists.txt` | 186 | CMake Tools #534 | Built |
| Coloured CMake and build output | 167, open | CMake Tools #478 | Terminal colours; diagnostics colours to add |
| ANSI codes garbled in output | 58, open | CMake Tools #3203 | Runs in a real terminal |
| Several `CMakeLists.txt` without a multi-root workspace | 47 | CMake Tools #1374 | Root only |
| A CMake language server | 44 | CMake Tools #93 | Built (neocmakelsp) |
| Run CTest's tests in parallel | 36 | CMake Tools #3091 | To add |
| Add to cache variables when using presets | 26 | CMake Tools #1836 | To add |
| Turn off configure on save | 25 | CMake Tools #1259 | To add, with the switch |
| Targets organised by the `FOLDER` property | 23 | CMake Tools #491 | Needs the targets view |
| Test Explorer | 24 | CMake Tools #1682 | To add (Tests view) |
| Build, install, then debug from the install folder | 23 | CMake Tools #532 | Install task to add |
| CMake support | 66 | Zed #5102 | This plugin |
| One command to set the build and debug target | 14, open | CMake Tools #1941 | Status bar target, with a mock-up |
| Prefer Ninja when it's there | 10, open | CMake Tools #1347 | Generator setting to add |

## What to build

### No new UI (built 2026-11-17)

1. **Targets from CMake's file API.** Before configuring, the plugin
   writes `.cmake/api/v1/query/client-merlin/codemodel-v2` in the build
   folder; after, it reads the reply: each target's name, type and
   program. From that:
   - a Build task per target, and a Run task per program;
   - each program in Run and Debug, so F5 debugs it with the C/C++
     plugin's gdb or lldb-dap, building it first, as CMake Tools does
     without a `launch.json`.
2. **CTest's tests in the Tests view.** `ctest --show-only=json-v1` lists
   them with the `add_test` line each comes from; ▶ runs one, a file's
   or all, with `--output-on-failure` and in parallel.
3. **Configure on save** of `CMakeLists.txt`, a `.cmake` file or the
   presets, once the build has been configured, with a switch to turn it
   off.
4. **Settings**: the build folder, build type and generator for projects
   without presets, and cache variables added to every configure, presets
   or not. Coloured diagnostics (`CMAKE_COLOR_DIAGNOSTICS`) on.
5. **More tasks**: Configure from scratch (`--fresh`), Clean and
   Install.

How it went: the configure command creates the file API query itself
(`cmake -E touch`), as plugins can't write files, and a new host call,
`list-folder`, finds the reply's index, which `list-files` skips as a
hidden folder. CTest's tests are listed by running `ctest` as a process,
and run with the build first (`cmake --build … && ctest -j …`), as CMake
Tools does. `-j` runs them in parallel (#3091). Tested with the real
cmake and ctest.

### Needs a mock-up first

`docs/mockups/CMake.html` proposes them, not approved yet.

6. **The status bar**: the preset and the target, each a picker, with
   Build, Run and Debug, as CMake Tools has it.
7. **A CMake view**: targets by folder (`FOLDER`, else the directory),
   each with Build, Run and Debug; the configure's state; and the
   cache's variables, searchable, booleans as switches, as CLion has
   them.
8. **Adding a new file to a target**, as Qt Creator does.

### Later

- CMake's own debugger (DAP, CMake 3.27+). It talks over a pipe, not
  stdin, so merlin's debuggers would need to connect to one.
- Several `CMakeLists.txt` projects in one window, with "Several
  folders in one window".

## Sources

- [CMake Tools issues, by votes](https://github.com/microsoft/vscode-cmake-tools/issues?q=is%3Aissue+sort%3Areactions-%2B1-desc)
- [CMake Tools: debugging and launching targets](https://vector-of-bool.github.io/docs/vscode-cmake-tools/debugging.html)
- [CMake Tools 1.16: the side bar](https://devblogs.microsoft.com/cppblog/visual-studio-code-cmake-tools-extension-1-16-update-new-cmake-tools-sidebar-and-cmake-debugging-options/)
- [CMake Tools 1.14: CTest in the Test Explorer](https://sdtimes.com/microsoft/cmake-tools-1-14-in-vs-code-adds-test-explorer-for-ctest/)
- [CLion: CMake profiles](https://www.jetbrains.com/help/clion/2024.2/cmake-profile.html)
- [CLion: working with CMake](https://www.jetbrains.com/help/clion/working-with-cmake.html)
- [Qt Creator 4.15: CMake](https://www.qt.io/blog/qt-creator-4.15-cmake-new-features)
- [Qt Creator: debugging CMake files](https://doc.qt.io/qtcreator/creator-how-to-debug-cmake-files.html)
- [cmake-file-api(7)](https://man.archlinux.org/man/cmake-file-api.7.en)
- [Zed issues about CMake](https://github.com/zed-industries/zed/issues?q=is%3Aissue+cmake+sort%3Areactions-%2B1-desc)
- [neocmakelsp](https://github.com/neocmakelsp/neocmakelsp)
