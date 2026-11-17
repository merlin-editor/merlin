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
| Coloured CMake and build output | 167, open | CMake Tools #478 | Built: a real terminal, and `CMAKE_COLOR_DIAGNOSTICS` on |
| ANSI codes garbled in output | 58, open | CMake Tools #3203 | Runs in a real terminal |
| Several `CMakeLists.txt` without a multi-root workspace | 47 | CMake Tools #1374 | Built (`CMakeProjects.html`) |
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

### Needs a mock-up first (approved and built 2026-11-17)

`docs/mockups/CMake.html`, approved 2026-11-17 ("Look great"), with a
fourth artboard for coloured completions in every language. Built:
- The status bar's preset and target (a plugin's `set-status-chips`),
  each a picker with details and groups, with Build, Run and Debug.
- The CMake view in the rail: the build with Configure and a menu,
  targets by folder with their sources, tests with Run All, and the
  cache with ON/OFF and choice chips or a box, changes held until
  Configure.
- Configuring with a spinner, failed in red with CMake's errors in
  Problems at their line (a problem matcher, so the language server's
  problems for the same file don't replace them), and not configured.
- A new source file saved: a notice offers the nearest targets, and
  Add puts it in the command that adds the target, as an edit.
- It first differed from the mock-up in three ways, as plugin views
  couldn't do what it shows: a row's actions always showed, a boolean
  was ON and OFF chips, and adding a file was a button per target. With
  plugin API 0.16's row actions, switches and coloured marks, pickers
  that open on the current item, and notices with a dropdown (built
  2026-11-17), it now matches.

6. **The status bar**: the preset and the target, each a picker, with
   Build, Run and Debug, as CMake Tools has it.
7. **A CMake view**: targets by folder (`FOLDER`, else the directory),
   each with Build, Run and Debug; the configure's state; and the
   cache's variables, searchable, booleans as switches, as CLion has
   them.
8. **Adding a new file to a target**, as Qt Creator does.

### CMake's debugger (built 2026-11-17)

CMake 3.27 and later can configure under a debugger, stopping at
breakpoints in `CMakeLists.txt` and `.cmake` files, as Qt Creator and
CMake Tools offer. It waits for the editor on a pipe it makes
(`--debugger-pipe`), not stdin. Built:
- `spawn` takes `${pipe}`: the host puts a new pipe's path in the
  argument, connects once CMake has made it, and DAP goes through it.
- The plugin's manifest declares the `cmake` debugger, run by the `dap`
  plugin, with `${args}` taking the configure's arguments.
- Run and Debug offers "CMake: Configure Debug in CMake's Debugger" for
  each configure preset, configured or not. It stops at breakpoints
  with CMake's variables, call stack and stop filters (warnings,
  errors).
- Found while trying it in the window: F5 before the Debugger plugin was
  allowed left a run stuck at Starting, which Stop couldn't end; and a
  target's build step that names a task (as `CMake: Build app`) ran the
  name as a command. All three are fixed, so F5 on a CMake program now
  builds it first and stops in gdb.

### Several projects in one folder (approved and built 2026-11-17)

`docs/mockups/CMakeProjects.html`, approved 2026-11-17 ("Go ahead").
Built:
- Projects are the `CMakeLists.txt` files with a `project()` call that no
  other one reaches through `add_subdirectory`, outside build folders
  and `_deps` (`plugins/cmake/src/projects.rs`).
- With more than one, a ▣ project chip comes first in the status bar,
  with a picker of each project's folder and state, and "Follow the open
  file", on until a project is picked by hand. The CMake view gets a
  Projects section, the chosen one's build, targets, tests and cache
  below.
- Each project reads its own presets, configures in its own folder
  (`cd tools && cmake …`), and keeps its build while another is in use.
  clangd gets the project's `compile_commands.json`, and none for a
  project not configured.
- Run Task and Run and Debug list every project's tasks and programs,
  named `CMake tools: Build plot` with the project as the detail.
- CMake's debugger takes `-S` with the project's folder.
- The plugin starts for a project in a folder at the top: `activate-on`
  takes `file:*/CMakeLists.txt`. A project deeper down starts it once
  one of its CMake files opens.

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
