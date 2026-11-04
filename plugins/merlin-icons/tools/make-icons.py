#!/usr/bin/env python3
"""Draws merlin's icon packs as SVG files, with their pack files.

    python3 tools/make-icons.py      (from plugins/merlin-icons)

Origami: folded paper in each language's colour, with a white mark for the
kind of file. Outline: the same shapes as outlines, tinted by the theme.
Line: the frame's icons, which follow their buttons' colours.
"""
import os

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Marks for kinds of file, drawn in a box about x 7.5-16.5, y 10-18.
MARKS = {
    "code": '<path d="M10 11.5 7.5 14l2.5 2.5M14 11.5l2.5 2.5-2.5 2.5" {s}/>',
    "data": '<path d="M10 10.5c-1.2 0-1.5.6-1.5 1.5v.8c0 .7-.4 1.2-1 1.2.6 0 1 .5 1 1.2v.8c0 .9.3 1.5 1.5 1.5M14 10.5c1.2 0 1.5.6 1.5 1.5v.8c0 .7.4 1.2 1 1.2-.6 0-1 .5-1 1.2v.8c0 .9-.3 1.5-1.5 1.5" {s}/>',
    "text": '<path d="M8 11.5h8M8 14.5h8M8 17.5h5" {s}/>',
    "config": '<path d="M8 12h8M8 16.5h8" {s}/><circle cx="10.5" cy="12" r="1.5" {f}/><circle cx="14" cy="16.5" r="1.5" {f}/>',
    "shell": '<path d="M8 11.5l2.5 2.5L8 16.5M12 17h4" {s}/>',
    "lock": '<path d="M10 13.5v-1.5a2 2 0 0 1 4 0v1.5" {s}/><rect x="8.5" y="13.5" width="7" height="4.5" rx="1" {f}/>',
    "image": '<path d="M7.5 17.8l3-3.6 2 2.3 1.6-1.8 2.4 3.1z" {f}/><circle cx="14.6" cy="11.8" r="1.2" {f}/>',
    "git": '<path d="M9.5 11.2v6M9.5 14.2c0-1.6 5-1.2 5-3" {s}/><circle cx="9.5" cy="17.4" r="1.2" {f}/><circle cx="14.5" cy="11" r="1.2" {f}/>',
    "law": '<path d="M12 10.5v7M8.5 12h7M9.5 12l-1.5 3h3zM14.5 12l-1.5 3h3z" {s}/>',
    "box": '<path d="M8 12.3 12 10.5l4 1.8v4L12 18l-4-1.7zM8 12.3l4 1.8 4-1.8M12 14.1V18" {s}/>',
    "none": "",
}

# Extension: (colour, mark). Colours sit between the dark and light themes.
FILES = {
    "rs": ("#d77a4f", "code"), "py": ("#4f8fd7", "code"), "js": ("#d4ad35", "code"),
    "mjs": ("#d4ad35", "code"), "cjs": ("#d4ad35", "code"), "jsx": ("#d4ad35", "code"),
    "ts": ("#3f7fd0", "code"), "tsx": ("#3f7fd0", "code"), "go": ("#36a9bb", "code"),
    "c": ("#5f7fbf", "code"), "h": ("#8a6fc0", "code"), "cpp": ("#4f6fb5", "code"),
    "cc": ("#4f6fb5", "code"), "hpp": ("#8a6fc0", "code"), "java": ("#c0743f", "code"),
    "kt": ("#9a6fd8", "code"), "swift": ("#e0703f", "code"), "rb": ("#c94a4a", "code"),
    "php": ("#7a7fc0", "code"), "lua": ("#3f5fb8", "code"), "zig": ("#e0a03f", "code"),
    "hs": ("#7f6fb0", "code"), "ex": ("#8a5fb0", "code"), "exs": ("#8a5fb0", "code"),
    "dart": ("#3fa0d0", "code"), "vue": ("#4fb07f", "code"), "svelte": ("#e0603f", "code"),
    "html": ("#d8643f", "code"), "css": ("#4f7fd8", "code"), "scss": ("#c9609a", "code"),
    "wit": ("#9a7fd0", "code"), "nix": ("#6f8fd0", "config"), "sql": ("#c9a23a", "data"),
    "json": ("#c9a23a", "data"), "jsonc": ("#c9a23a", "data"), "csv": ("#5aa86a", "data"),
    "xml": ("#d8643f", "data"), "md": ("#5b8fc9", "text"), "txt": ("#8a8f99", "text"),
    "rst": ("#5b8fc9", "text"), "toml": ("#8a8f99", "config"), "yaml": ("#b06fa8", "config"),
    "yml": ("#b06fa8", "config"), "ini": ("#8a8f99", "config"), "conf": ("#8a8f99", "config"),
    "sh": ("#5aa86a", "shell"), "bash": ("#5aa86a", "shell"), "zsh": ("#5aa86a", "shell"),
    "fish": ("#5aa86a", "shell"), "lock": ("#8a8f99", "lock"), "png": ("#b06fa8", "image"),
    "jpg": ("#b06fa8", "image"), "jpeg": ("#b06fa8", "image"), "gif": ("#b06fa8", "image"),
    "svg": ("#d39a3f", "image"), "webp": ("#b06fa8", "image"), "ttf": ("#8a8f99", "text"),
    "wasm": ("#7a6fd0", "box"),
}
# Whole file names, which win over extensions.
NAMES = {
    "Cargo.toml": ("#d77a4f", "box"), "Cargo.lock": ("#d77a4f", "lock"),
    "package.json": ("#5aa86a", "box"), "package-lock.json": ("#5aa86a", "lock"),
    "README.md": ("#7c9cff", "text"), "LICENSE": ("#c9a23a", "law"),
    "LICENSE.txt": ("#c9a23a", "law"), "Makefile": ("#8a8f99", "shell"),
    "Dockerfile": ("#3f8fd8", "box"), ".gitignore": ("#d8643f", "git"),
    ".gitmodules": ("#d8643f", "git"), ".editorconfig": ("#8a8f99", "config"),
    "plugin.toml": ("#9a7fd0", "box"),
}
FOLDER = "#8f9bb3"
# Folders with their own colour and mark.
FOLDERS = {
    "src": ("#6f8fd0", "code"), "tests": ("#5aa86a", "check"), "test": ("#5aa86a", "check"),
    "docs": ("#5b8fc9", "text"), ".git": ("#d8643f", "git"), "target": ("#7a7f8a", "box"),
    "build": ("#7a7f8a", "box"), "dist": ("#7a7f8a", "box"), "plugins": ("#9a7fd0", "box"),
    "assets": ("#b06fa8", "image"), "images": ("#b06fa8", "image"), "scripts": ("#5aa86a", "shell"),
    "crates": ("#d77a4f", "box"), "node_modules": ("#5aa86a", "box"), "fonts": ("#8a8f99", "text"),
    "themes": ("#c9609a", "image"), "tools": ("#8a8f99", "config"),
}
MARKS["check"] = '<path d="M8.5 14.5l2.3 2.3 4.7-4.8" {s}/>'

DOC = "M6 2.5h8.5L19.5 7.5V20a1.5 1.5 0 0 1-1.5 1.5H6A1.5 1.5 0 0 1 4.5 20V4A1.5 1.5 0 0 1 6 2.5z"
FOLD = "M14.5 2.5V6a1.5 1.5 0 0 0 1.5 1.5h3.5z"
FOLDER_BACK = "M2.5 6.5A1.5 1.5 0 0 1 4 5h5l2 2h9a1.5 1.5 0 0 1 1.5 1.5V18a1.5 1.5 0 0 1-1.5 1.5H4A1.5 1.5 0 0 1 2.5 18z"
FOLDER_FRONT = "M2.5 10a1 1 0 0 1 1-1h17a1 1 0 0 1 1 1v8a1.5 1.5 0 0 1-1.5 1.5H4A1.5 1.5 0 0 1 2.5 18z"
FOLDER_OPEN = "M4.3 10.2a1 1 0 0 1 1-.7h16a1 1 0 0 1 1 1.2l-1.6 7.6a1.5 1.5 0 0 1-1.5 1.2H4.1a1.3 1.3 0 0 1-1.3-1.5z"


def light(hex_colour, t):
    """The colour mixed with white, `t` of the way."""
    c = [int(hex_colour[i:i + 2], 16) for i in (1, 3, 5)]
    return "#" + "".join(f"{round(x + (255 - x) * t):02x}" for x in c)


def svg(body):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">{body}</svg>\n'


def mark(kind, colour, width="2.3"):
    s = f'fill="none" stroke="{colour}" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round"'
    f = f'fill="{colour}"'
    return MARKS[kind].format(s=s, f=f)


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "w").write(text)


def key(name):
    return name.replace(".", "_").lower()


def origami():
    out = os.path.join(HERE, "origami")
    entries = {"files": [], "names": [], "folders": []}
    for ext, (colour, kind) in FILES.items():
        write(os.path.join(out, f"file-{ext}.svg"), svg(
            f'<path d="{DOC}" fill="{colour}"/><path d="{FOLD}" fill="{light(colour, .45)}"/>'
            + mark(kind, "#ffffff")))
        entries["files"].append((ext, f"origami/file-{ext}.svg"))
    for name, (colour, kind) in NAMES.items():
        write(os.path.join(out, f"name-{key(name)}.svg"), svg(
            f'<path d="{DOC}" fill="{colour}"/><path d="{FOLD}" fill="{light(colour, .45)}"/>'
            + mark(kind, "#ffffff")))
        entries["names"].append((name, f"origami/name-{key(name)}.svg"))
    for name, colour, kind in [("default", FOLDER, "none")] + [(n, c, k) for n, (c, k) in FOLDERS.items()]:
        for state in ("closed", "open"):
            front = FOLDER_OPEN if state == "open" else FOLDER_FRONT
            body = (f'<path d="{FOLDER_BACK}" fill="{colour}"/>'
                    f'<path d="{front}" fill="{light(colour, .3)}"/>')
            if kind != "none":
                body += f'<g transform="translate(0 0.8)">{mark(kind, "#ffffff", "2.1")}</g>'
            file = f"folder-{key(name)}{'-open' if state == 'open' else ''}.svg"
            write(os.path.join(out, file), svg(body))
            label = name if state == "closed" else f"{name}-open"
            if name == "default":
                label = "default" if state == "closed" else "open"
            entries["folders"].append((label, f"origami/{file}"))
    return entries


def outline():
    out = os.path.join(HERE, "outline")
    entries = {"files": [], "names": [], "folders": []}
    stroke = 'fill="none" stroke="#000" stroke-width="1.5" stroke-linejoin="round"'
    doc = f'<path d="{DOC}" {stroke}/><path d="M14.5 2.5V6a1.5 1.5 0 0 0 1.5 1.5h3.5" {stroke}/>'
    kinds = {kind for _, kind in list(FILES.values()) + list(NAMES.values())}
    for kind in sorted(kinds):
        write(os.path.join(out, f"file-{kind}.svg"), svg(doc + mark(kind, "#000", "1.5")))
    for ext, (_, kind) in FILES.items():
        entries["files"].append((ext, f"outline/file-{kind}.svg"))
    for name, (_, kind) in NAMES.items():
        entries["names"].append((name, f"outline/file-{kind}.svg"))
    write(os.path.join(out, "folder.svg"), svg(f'<path d="{FOLDER_BACK}" {stroke}/><path d="M2.5 10h19" {stroke}/>'))
    write(os.path.join(out, "folder-open.svg"), svg(f'<path d="{FOLDER_BACK}" {stroke}/><path d="{FOLDER_OPEN}" {stroke}/>'))
    entries["folders"] += [("default", "outline/folder.svg"), ("open", "outline/folder-open.svg")]
    return entries


LINE = {
    "explorer": '<path d="M9 3h6.5l4 4v10.5A1.5 1.5 0 0 1 18 19H9a1.5 1.5 0 0 1-1.5-1.5v-13A1.5 1.5 0 0 1 9 3z"/><path d="M15.5 3v4h4M4.5 7.5v12A1.5 1.5 0 0 0 6 21h9"/>',
    "search": '<circle cx="10.5" cy="10.5" r="6"/><path d="m15 15 5 5"/>',
    "source-control": '<circle cx="7" cy="6" r="2.2"/><circle cx="7" cy="18" r="2.2"/><circle cx="17" cy="8" r="2.2"/><path d="M7 8.2v7.6M17 10.2c0 4-5 3.4-9.2 6"/>',
    "outline": '<path d="M4 6h11M9 12h11M9 18h8"/><path d="M5 11.2v1.6M5 17.2v1.6"/>',
    "plugin": '<path d="M5 8h3a2 2 0 1 1 4 0h3a1 1 0 0 1 1 1v3a2 2 0 1 1 0 4v3a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1z"/>',
    "menu": '<path d="M4 7h16M4 12h16M4 17h16"/>',
    "layout-left": '<rect x="3" y="4.5" width="18" height="15" rx="3"/><path d="M9.5 4.5v15"/>',
    "layout-panel": '<rect x="3" y="4.5" width="18" height="15" rx="3"/><path d="M3 14h18"/>',
    "layout-right": '<rect x="3" y="4.5" width="18" height="15" rx="3"/><path d="M14.5 4.5v15"/>',
    "split": '<rect x="3" y="4.5" width="18" height="15" rx="3"/><path d="M12 4.5v15"/>',
    "more": '<circle cx="5.5" cy="12" r="1.3" fill="#000"/><circle cx="12" cy="12" r="1.3" fill="#000"/><circle cx="18.5" cy="12" r="1.3" fill="#000"/>',
}


def line():
    out = os.path.join(HERE, "line")
    entries = []
    for name, body in LINE.items():
        write(os.path.join(out, f"{name}.svg"), svg(
            f'<g fill="none" stroke="#000" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">{body}</g>'))
        entries.append((name, f"line/{name}.svg"))
    return entries


def toml_icons(sections, tint=None):
    lines = []
    for section in ("files", "names", "folders"):
        if not sections.get(section):
            continue
        lines += ["", f"[icons.{section}]"]
        for k, f in sections[section]:
            value = f'{{ svg = "{f}", tint = "{tint}" }}' if tint else f'"{f}"'
            lines.append(f'"{k}" = {value}')
    return lines


o = origami()
write(os.path.join(HERE, "origami.toml"), "\n".join(
    ["# Origami: folded paper in each language's colour, with a white mark for",
     "# the kind of file. Folders fold open; some have their own colour.", "",
     "[icons]", 'style = "badge"'] + toml_icons(o)) + "\n")
ol = outline()
write(os.path.join(HERE, "outline.toml"), "\n".join(
    ["# Outline: thin outlines in the theme's muted colour, one shape for each",
     "# kind of file. It follows whichever colour theme is in use.", "",
     "[icons]", 'style = "none"'] + toml_icons(ol, tint="muted")) + "\n")
li = line()
write(os.path.join(HERE, "line.toml"), "\n".join(
    ["# Line: the frame's icons drawn with thin rounded lines. They follow the",
     "# colours of their buttons. A pack of its own, so it layers over any icon",
     "# theme.", "", "[icons.ui]"] + [f'"{k}" = "{f}"' for k, f in li]) + "\n")
print(f"origami: {sum(len(v) for v in o.values())} icons, outline: {sum(len(v) for v in ol.values())}, line: {len(li)}")
