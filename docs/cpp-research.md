# C and C++: what merlin is missing

Research of 2026-11-16 on the C/C++ plugin (`plugins/cpp`, version 0.2.0,
API 0.15) against CLion, VS Code, Zed and Qt Creator, and what their users
vote for. Nothing here is built yet; any UI in it needs a mock-up first.

## Where the plugin stands

The plugin is small: `src/lib.rs` (131 lines) starts clangd through the LSP
SDK and colours files with its own lexer, `src/lex.rs` (516 lines). It
declares the `c` and `cpp` languages, lldb-dap and gdb as debuggers, and
clangd as its program. It builds, its 6 lexer tests pass, and colouring is
linear: a 500 KB file takes about 25 ms, and one 64 000-character line about
7 ms. Neither repo has GitHub issues open for it.

### Bugs

Each was confirmed with a test against `lex::lex`.

| Bug | Example | Where |
|---|---|---|
| Tabs break type and call colours: `word_before` and the look-ahead for `(` skip only spaces. | `class\tFoo` gets no type colour, `foo\t(1)` no call colour | `lex.rs` `word_before`, and the `while self.chars.get(j) == Some(&' ')` loop in `lex` |
| A string continued with `\` at the end of a line goes wrong on the next line: the rest is left plain, and the closing `"` starts a new string to the end of the line. | `"abc\` then `def";` | `lex.rs` `quoted` stops at `\n` |
| A `//` comment continued with `\` ends at the first line. Rare. | `// a \` then `more` | `lex.rs`, the `//` arm of `lex` |

### Smaller points

- `.h` is always C, so a C++ header says "C" in the status bar, and the
  catch2 plugin, which looks only at C++ files, may miss tests in `.h` files.
  clangd itself decides from the compile command, so its side is fine.
- The `lldb-dap` install hint, `sudo apt install lldb`, may not give an
  unversioned `lldb-dap` on Ubuntu, which ships `lldb-dap-NN`. Not checked:
  lldb isn't installed here. gdb 17 and clangd 21 are, and are new enough.
- clangd's install command is written twice, in `plugin.toml` and in the
  `Spec` in `lib.rs`, and the `lib.rs` one is Linux-only.
- The description says "outlines", but outlines come only from clangd, so
  without clangd there are none.

### Against the other language plugins

- **No problem matchers.** Compiler errors from gcc and clang
  (`file:line:col: error: …`) in the terminal or a task don't become
  problems, as they do for Rust and Python.
- **No settings.** clangd's arguments are fixed (`CPP_SERVER` in `lib.rs`).
  There's no way to pass `--query-driver` for gcc and cross-compilers, or to
  say where `compile_commands.json` is.
- **No tasks.** Rust has Cargo's; C++ has nothing for CMake, Make or Meson.

## What clangd offers that merlin doesn't use

Asked directly, clangd 21 advertises these, and merlin's LSP client
(`sdk/lsp`) asks for none of them:

| clangd offers | What it gives | Who uses it |
|---|---|---|
| `executeCommandProvider` | clangd's refactors, below | VS Code, Zed, Qt Creator |
| `textDocument/switchSourceHeader` (clangd's own) | Switch between `foo.cpp` and `foo.h` | All of them |
| `callHierarchyProvider`, `typeHierarchyProvider` | Who calls this; base and derived classes | VS Code, CLion |
| `declarationProvider`, `implementationProvider`, `typeDefinitionProvider` | Go to declaration, implementation, type | All of them |
| `documentLinkProvider` | Clickable `#include` lines | VS Code |
| `inactiveRegionsProvider` (clangd's own) | Dim `#if 0` and disabled `#ifdef` blocks | Zed, VS Code, CLion |
| `documentHighlightProvider` | Highlight the other uses of the name under the cursor | All of them |
| `selectionRangeProvider` | Expand selection by syntax | All of them |
| `astProvider`, `memoryUsageProvider` (clangd's own) | AST view, clangd's memory by file | vscode-clangd |

### clangd's refactors are all dropped

merlin keeps only code actions that carry an `edit` (`sdk/lsp/src/handlers.rs`,
the `Request::CodeActions` arm). clangd sends its refactors, which it calls
tweaks, with no edit, only a `clangd.applyTweak` command; the edit comes
back as `workspace/applyEdit` after the client runs the command. Asked for
code actions on `auto x = 1;`, clangd 21 answered with "Replace with deduced
type" as a command and no edit, so merlin would drop it.

Supporting `workspace/executeCommand` brings in, with no new UI:
- Define outline, which generates the definition from a declaration
  (VS Code C/C++ #664, 321 👍).
- Extract function and extract variable (VS Code C/C++ #1162, 235 👍, which
  Microsoft called its most requested C++ feature).
- Expand macro, populate switch, add using, swap if branches, expand `auto`,
  and the rest of clangd's tweaks.

rust-analyzer and gopls send some code actions the same way, so this helps
them too.

## What other editors do, and what their users want

**CLion** is what people point to for C++. Its main draw is CMake built in:
profiles, presets by display name, targets, completion in CMake options, and
a CMake editor, with toolchains found on their own. It also opens projects
set up for VS Code. The usual complaint is memory and CPU use.

**VS Code** splits C++ across extensions: Microsoft's C/C++ or clangd for the
language, CMake Tools for builds, and C/C++ or CodeLLDB for debugging.
Users complain the two language extensions clash, and that debugging goes
with them when the Microsoft one is off. CMake Tools shows the configure,
build and test presets and the build target in the status bar and a side
bar, with Build, Debug and Run buttons.

**Zed** uses clangd and has added its extensions one by one, each after
requests: switch source/header (#12801), inactive regions (#13089), and
`--query-driver` (#4295). Its docs tell CMake users to set
`CMAKE_EXPORT_COMPILE_COMMANDS`, and Make users to run `bear`.

**Qt Creator** moved to clangd, keeps one clangd per project with an index
that survives a restart, and can switch header/source with clangd or its own
code model. Users report clangd's memory use on big sessions.

### Most voted, by 👍

| Request | Votes | Tracker | merlin |
|---|---|---|---|
| Renaming | 550 | VS Code C/C++ #296 | Has it |
| Disassembly view while debugging | 436 | VS Code C/C++ #206 | Missing |
| Inline errors (error lens) | 382 | Zed #4901 | Has it |
| Call hierarchy | 380 | VS Code C/C++ #16 | Missing |
| Memory view while debugging | 367 | VS Code C/C++ #509 | Missing |
| Find all references | 364 | VS Code C/C++ #15 | Has it |
| Memory view and disassembly, instruction stepping | 345 | VS Code C/C++ #941 | Missing |
| Generate definitions from declarations | 321 | VS Code C/C++ #664 | Dropped (clangd tweak) |
| Extract function | 235 | VS Code C/C++ #1162 | Dropped (clangd tweak) |
| Doxygen in hovers | 219 | VS Code C/C++ #658 | Partly: hovers are Markdown |
| clang-tidy | 200 | VS Code C/C++ #2908 | Missing: clangd can run it |
| "Just My Code" (step over the std library) | 171, open | VS Code C/C++ #5763 | Missing |
| Colourised CMake and build output | 167, open | CMake Tools #478 | Missing |
| Call hierarchy panel | 114, open | Zed #14203 | Missing |
| Lower memory use | 89, open | Zed #18673 | Aimed at already |
| CMake support | 66 | Zed #5102 | Missing |
| `--query-driver` for clangd | 51 | Zed #4295 | Missing |
| Parameter hints | 40 | Zed #4879 | Has it |
| Errors for the whole project | 29, open | vscode-clangd #158 | Missing |
| Streamline clangd setup | 24, open | vscode-clangd #323 | Partly: install card |
| Switch source/header | 20 | Zed #12801 | Missing |
| Watch for `compile_commands.json` | 19 | vscode-clangd #6 | Missing |
| Hover comments have no formatting | 17, open | vscode-clangd #10 | Missing |
| Doc comments in C++ | 16 | Zed #18169 | Missing |
| A replacement for the `$gcc` problem matcher | 9, open | vscode-clangd #337 | Missing |
| Dim inactive `#if` blocks | 9 | Zed #13089 | Missing |
| STL pretty printers in the debugger | 7 | Zed #33116 | Missing |

What people praise that merlin already has: low memory use (the C++
plugin's clangd flags aim at it), one plugin for the language server and
debuggers, which avoids VS Code's clash, and inline errors and parameter
hints.

## Where each piece belongs

Not everything belongs in the C++ plugin. Plugins can't call each other: a
dependency only means "install and start this first"
(`docs/plugin-registry.md` §12.4).

### A plugin of its own: CMake

Many C and C++ projects use Make, Meson, Bazel or a plain
`compile_commands.json`, and some people use CMake without C++. So CMake
gets its own plugin, as catch2 already did:
- CMake colouring for `CMakeLists.txt` and `*.cmake`.
- Configure, build and test as tasks, from presets when there's a
  `CMakePresets.json`.
- The preset and build target in the status bar, and colourised build output.
- It starts on `file:CMakeLists.txt`, as the C++ and catch2 plugins do.

It tells the C++ plugin where `compile_commands.json` is by sharing a
`compile-commands` value, which the C++ plugin watches and hands to clangd.
That's plugin API 0.16, built 2026-11-16: see `docs/plugin-registry.md`
section 13. Meson and Bazel plugins would share the same key.

### The LSP SDK, for every language

In `sdk/lsp`:
- `workspace/executeCommand`, and code actions that carry a command.
- Call hierarchy and type hierarchy.
- Go to declaration, implementation and type definition.
- Document links, document highlights and selection ranges.

### The debug UI and the dap plugin, for every language

Disassembly, a memory view, instruction stepping and "Just My Code" are part
of the debug protocol, not of C++. Rust, Go and Zig use them too. STL pretty
printers are lldb-dap's and gdb's own, so they need checking there first.

### The C++ plugin

- The three lexer bugs.
- gcc and clang problem matchers.
- Settings: `--query-driver`, and clang-tidy on or off. The folder of
  `compile_commands.json` is built (API 0.16).
- Switch header/source and inactive regions, as clangd helpers in the LSP
  SDK that the C++ plugin turns on.
- Help when `compile_commands.json` is missing, where today clangd shows
  errors everywhere.
- The smaller points above.

## Order

1. **The C++ plugin's quick fixes**: the lexer bugs, the problem matchers,
   the settings and the smaller points. No UI.
2. **clangd's commands** in the LSP SDK. The biggest gain for the least
   work, and no UI: the code actions menu already exists.
3. **The rest of the LSP SDK**: declaration, implementation, type
   definition, links, highlights, selection ranges, switch header/source,
   and inactive regions. Mostly commands and keys; inactive regions and
   highlights need a look in the mock-ups.
4. **Needs a mock-up first**:
   - The CMake plugin's status bar, presets and build output.
   - A call and type hierarchy view.
   - Disassembly and memory views in the debugger.
   - Help when `compile_commands.json` is missing.
   - Errors for the whole project.

## Sources

- [VS Code C/C++ issues, by votes](https://github.com/microsoft/vscode-cpptools/issues?q=is%3Aissue+sort%3Areactions-%2B1-desc)
- [vscode-clangd issues, by votes](https://github.com/clangd/vscode-clangd/issues?q=is%3Aissue+sort%3Areactions-%2B1-desc)
- [CMake Tools issues, by votes](https://github.com/microsoft/vscode-cmake-tools/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc)
- [Zed issues about clangd, by votes](https://github.com/zed-industries/zed/issues?q=is%3Aissue+clangd+sort%3Areactions-%2B1-desc)
- [A year of C++ improvements in Visual Studio and VS Code (Microsoft)](https://devblogs.microsoft.com/cppblog/?p=33593)
- [clangd's protocol extensions](https://llvm.googlesource.com/clang-tools-extra/+/refs/heads/master/docs/clangd/Extensions.rst)
- [Zed: switch source/header, #14646](https://git.secluded.site/zed/commit/f85ca387a7fe9a8fc420ada97fe84143dc46bb1b)
- [Zed's C++ docs](https://zed.dev/docs/languages/cpp.html)
- [Zed issue #19126, GCC flags through clangd](https://github.com/zed-industries/zed/issues/19126)
- [CLion 2026.1](https://blog.jetbrains.com/clion/2026/03/2026-1-release)
- [CLion and VS Code compared](https://www.markaicode.com/vs/c23-ide-showdown-clion-vs-visual-studio-code-extensions/)
- [CMake Tools presets](https://github.com/microsoft/vscode-cmake-tools/blob/903aad3/docs/cmake-presets.md)
- [CMake Tools 1.16: side bar and status bar](https://devblogs.microsoft.com/cppblog/visual-studio-code-cmake-tools-extension-1-16-update-new-cmake-tools-sidebar-and-cmake-debugging-options/)
- [Qt Creator and clangd](https://qt.io/blog/qt-creator-and-clangd-an-introduction)
- [r/cpp: Linux C++ setups](https://redlib.hbubli.cc/r/cpp/comments/yohaof/linux_c_devs_what_does_your_dev_environment_look)
