//! Theme packs, which stack like resource packs. A pack is a TOML file that
//! sets any of colours, icons, layout and fonts; packs are layered over
//! merlin's own look, and each layer changes only what it sets. A colour
//! theme is a pack with colours; an icon theme, one with icons.
//!
//! ```toml
//! [ui]                  # colours of the frame (see theme.rs)
//! [code]                # colours of code
//! [layout]              # radius = 10, gap = 6, density = "compact"
//! [fonts]               # ui = "Inter", code = "JetBrains Mono", code-size = 13.5
//! [icons]               # style = "badge" | "glyph" | "none" for files without an icon
//! [icons.files]         # by extension:   "rs" = "icons/rust.svg"
//! [icons.names]         # by file name:   "Cargo.toml" = { svg = "icons/cargo.svg" }
//! [icons.folders]       # "default", "open", or a folder's name
//! [icons.ui]            # explorer, search, source-control, outline, plugin,
//!                       # menu, layout-left, layout-panel, layout-right, split, more
//! ```
//!
//! An icon is an SVG file (a string, or `{ svg = "…" }`), drawn in its own
//! colours or, with `tint`, as a shape in one colour: a theme colour's name
//! such as `"accent"` or `"muted"`, or `"#rrggbb"`. It can also be
//! `{ glyph = "\u{e7a8}", colour = "…" }` from a font, or
//! `{ badge = "RS", colour = "…" }`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::theme::{self, Palette};

/// A pack as read: its name, its file (none for those built in), and what
/// it sets.
#[derive(Clone, Debug)]
pub struct Pack {
    pub label: String,
    pub file: Option<PathBuf>,
    pub table: toml::Table,
}

/// What a pack sets, for the pickers and the packs panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Parts {
    pub colours: bool,
    pub icons: bool,
    pub layout: bool,
    pub fonts: bool,
}

impl Parts {
    /// As the packs panel says it: "colours, icons".
    pub fn describe(self) -> String {
        let parts: Vec<_> = [
            (self.colours, "colours"),
            (self.icons, "icons"),
            (self.layout, "layout"),
            (self.fonts, "fonts"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        if parts.is_empty() {
            "nothing".into()
        } else {
            parts.join(", ")
        }
    }
}

const SECTIONS: &[&str] = &["ui", "code", "layout", "fonts", "icons"];
const ICON_KINDS: &[&str] = &["style", "files", "names", "folders", "ui"];

impl Pack {
    pub fn built_in(label: &str, toml: &str) -> Pack {
        Pack {
            label: label.into(),
            file: None,
            table: toml::from_str(toml).expect("built-in packs parse"),
        }
    }

    pub fn read(label: &str, file: &Path) -> Result<Pack> {
        let text =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let table: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
        for key in table.keys() {
            if !SECTIONS.contains(&key.as_str()) {
                bail!(
                    "{}: [{key}] isn't a pack section (they're {})",
                    file.display(),
                    SECTIONS.join(", ")
                );
            }
        }
        if let Some(icons) = table.get("icons").and_then(|v| v.as_table()) {
            for key in icons.keys() {
                if !ICON_KINDS.contains(&key.as_str()) {
                    bail!("{}: [icons.{key}] isn't an icon section", file.display());
                }
            }
        }
        Ok(Pack {
            label: label.into(),
            file: Some(file.to_path_buf()),
            table,
        })
    }

    pub fn parts(&self) -> Parts {
        let has = |k: &str| self.table.contains_key(k);
        Parts {
            colours: has("ui") || has("code"),
            icons: has("icons"),
            layout: has("layout"),
            fonts: has("fonts"),
        }
    }

    fn dir(&self) -> PathBuf {
        self.file
            .as_ref()
            .and_then(|f| f.parent())
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }
}

/// How a file, folder or part of the frame is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    /// An SVG picture, in its own colours or drawn as a shape in one.
    Svg { file: PathBuf, tint: Tint },
    /// A glyph from the code's font, which with a Nerd Font is an icon.
    Glyph { text: String, colour: Option<u32> },
    /// A few letters on a rounded square.
    Badge { text: String, colour: Option<u32> },
}

/// The colour an SVG icon is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    /// Its own colours.
    Own,
    Colour(u32),
    /// The colour of the spot it's in, such as a button's, brighter under
    /// the mouse: `tint = "current"`.
    Current,
}

/// How files with no icon of their own look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// The file type's letters, as merlin draws them.
    Badge,
    /// The core's Nerd Font glyph for the file type.
    Glyph,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Icons {
    pub fallback: Fallback,
    files: HashMap<String, Icon>,
    names: HashMap<String, Icon>,
    folders: HashMap<String, Icon>,
    ui: HashMap<String, Icon>,
}

impl Icons {
    /// A file's icon, by its name then its extension.
    pub fn file(&self, name: &str) -> Option<&Icon> {
        let lower = name.to_ascii_lowercase();
        self.names.get(&lower).or_else(|| {
            // Longest extension first: "d.ts" before "ts".
            let mut ext = lower.as_str();
            while let Some((_, rest)) = ext.split_once('.') {
                if let Some(icon) = self.files.get(rest) {
                    return Some(icon);
                }
                ext = rest;
            }
            None
        })
    }

    pub fn folder(&self, name: &str, open: bool) -> Option<&Icon> {
        let lower = name.to_ascii_lowercase();
        let named = if open {
            self.folders
                .get(&format!("{lower}-open"))
                .or_else(|| self.folders.get(&lower))
        } else {
            self.folders.get(&lower)
        };
        named.or_else(|| {
            if open {
                self.folders
                    .get("open")
                    .or_else(|| self.folders.get("default"))
            } else {
                self.folders.get("default")
            }
        })
    }

    pub fn ui(&self, id: &str) -> Option<&Icon> {
        self.ui.get(id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub radius: f32,
    pub gap: f32,
    pub compact: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fonts {
    pub ui: Option<String>,
    pub code: Option<String>,
    pub code_size: Option<f32>,
}

/// Everything about how the window looks, from a stack of packs.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub palette: Palette,
    pub icons: Icons,
    pub layout: Layout,
    pub fonts: Fonts,
}

impl Look {
    /// Layers `packs`, lowest first, each over the ones before.
    pub fn stack(packs: &[&Pack]) -> Result<Look> {
        // Colours merge key by key first, so what follows from them follows
        // from the final ones.
        let mut colours = toml::Table::new();
        for pack in packs {
            for part in ["ui", "code"] {
                if let Some(t) = pack.table.get(part).and_then(|v| v.as_table()) {
                    let into = colours
                        .entry(part)
                        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                    if let Some(into) = into.as_table_mut() {
                        for (k, v) in t {
                            into.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
        let palette = Palette::from_table(&colours)?;

        let mut layout = Layout {
            radius: 10.0,
            gap: 6.0,
            compact: false,
        };
        let mut fonts = Fonts::default();
        let mut icons = Icons {
            fallback: Fallback::Badge,
            files: HashMap::new(),
            names: HashMap::new(),
            folders: HashMap::new(),
            ui: HashMap::new(),
        };
        for pack in packs {
            let where_ = || {
                pack.file
                    .as_ref()
                    .map_or(pack.label.clone(), |f| f.display().to_string())
            };
            if let Some(t) = pack.table.get("layout").and_then(|v| v.as_table()) {
                let number = |k: &str| {
                    t.get(k)
                        .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
                };
                if let Some(r) = number("radius") {
                    layout.radius = r.clamp(0.0, 24.0) as f32;
                }
                if let Some(g) = number("gap") {
                    layout.gap = g.clamp(0.0, 24.0) as f32;
                }
                match t.get("density").and_then(|v| v.as_str()) {
                    Some("compact") => layout.compact = true,
                    Some("comfortable") => layout.compact = false,
                    Some(other) => bail!(
                        "{}: density {other:?} should be \"compact\" or \"comfortable\"",
                        where_()
                    ),
                    None => {}
                }
            }
            if let Some(t) = pack.table.get("fonts").and_then(|v| v.as_table()) {
                if let Some(f) = t.get("ui").and_then(|v| v.as_str()) {
                    fonts.ui = Some(f.into());
                }
                if let Some(f) = t.get("code").and_then(|v| v.as_str()) {
                    fonts.code = Some(f.into());
                }
                if let Some(s) = t
                    .get("code-size")
                    .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
                {
                    fonts.code_size = Some(s.clamp(8.0, 32.0) as f32);
                }
            }
            if let Some(t) = pack.table.get("icons").and_then(|v| v.as_table()) {
                match t.get("style").and_then(|v| v.as_str()) {
                    Some("badge") => icons.fallback = Fallback::Badge,
                    Some("glyph") => icons.fallback = Fallback::Glyph,
                    Some("none") => icons.fallback = Fallback::None,
                    Some(other) => bail!(
                        "{}: icon style {other:?} should be badge, glyph or none",
                        where_()
                    ),
                    None => {}
                }
                let dir = pack.dir();
                for (kind, into) in [
                    ("files", &mut icons.files),
                    ("names", &mut icons.names),
                    ("folders", &mut icons.folders),
                    ("ui", &mut icons.ui),
                ] {
                    // The frame's icons follow their buttons' colours unless
                    // they say otherwise.
                    let default = if kind == "ui" {
                        Tint::Current
                    } else {
                        Tint::Own
                    };
                    for (key, value) in t.get(kind).and_then(|v| v.as_table()).into_iter().flatten()
                    {
                        let icon = icon(value, &dir, &palette, default)
                            .with_context(|| format!("{}: [icons.{kind}] {key}", where_()))?;
                        into.insert(key.to_ascii_lowercase(), icon);
                    }
                }
            }
        }
        Ok(Look {
            palette,
            icons,
            layout,
            fonts,
        })
    }
}

fn icon(value: &toml::Value, dir: &Path, p: &Palette, default: Tint) -> Result<Icon> {
    let colour = |v: Option<&toml::Value>| -> Result<Option<u32>> {
        let Some(v) = v else { return Ok(None) };
        let s = v.as_str().context("a colour is a string")?;
        named_colour(s, p)
            .map(Some)
            .with_context(|| format!("{s:?} isn't a colour or a theme colour's name"))
    };
    match value {
        toml::Value::String(file) => Ok(Icon::Svg {
            file: dir.join(file),
            tint: default,
        }),
        toml::Value::Table(t) => {
            if let Some(file) = t.get("svg").and_then(|v| v.as_str()) {
                let tint = match t.get("tint").and_then(|v| v.as_str()) {
                    None => default,
                    Some("own") => Tint::Own,
                    Some("current") => Tint::Current,
                    Some(_) => Tint::Colour(colour(t.get("tint"))?.unwrap_or_default()),
                };
                Ok(Icon::Svg {
                    file: dir.join(file),
                    tint,
                })
            } else if let Some(text) = t.get("glyph").and_then(|v| v.as_str()) {
                Ok(Icon::Glyph {
                    text: text.into(),
                    colour: colour(t.get("colour"))?,
                })
            } else if let Some(text) = t.get("badge").and_then(|v| v.as_str()) {
                Ok(Icon::Badge {
                    text: text.chars().take(3).collect(),
                    colour: colour(t.get("colour"))?,
                })
            } else {
                bail!("an icon needs svg, glyph or badge")
            }
        }
        _ => bail!("an icon is an SVG file or a table"),
    }
}

/// `"#rrggbb"`, or the name of a theme colour.
fn named_colour(s: &str, p: &Palette) -> Option<u32> {
    if let Some(hex) = s.strip_prefix('#') {
        return (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok())?;
    }
    Some(match s {
        "text" => p.text,
        "bright" => p.bright,
        "muted" => p.muted,
        "faint" => p.faint,
        "accent" => p.accent,
        "error" => p.error,
        "warning" => p.warning,
        "success" => p.success,
        "info" => p.info,
        "keyword" => p.keyword,
        "string" => p.string,
        "function" => p.function,
        "type" => p.type_,
        "constant" => p.constant,
        _ => return None,
    })
}

/// merlin's own colours, and its two icon themes.
pub fn built_in() -> Vec<Pack> {
    vec![
        Pack::built_in(theme::DEFAULT_NAME, ""),
        Pack::built_in(LETTERS, "[icons]\nstyle = \"badge\""),
        Pack::built_in(GLYPHS, "[icons]\nstyle = \"glyph\""),
    ]
}

/// The icon theme used unless the settings say otherwise.
pub const LETTERS: &str = "Letters";
pub const GLYPHS: &str = "Nerd Font Glyphs";

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(label: &str, toml: &str) -> Pack {
        Pack {
            label: label.into(),
            file: Some(PathBuf::from("/packs").join(label).join("pack.toml")),
            table: toml::from_str(toml).unwrap(),
        }
    }

    #[test]
    fn higher_layers_win_key_by_key() {
        let theme = pack(
            "theme",
            "[ui]\naccent = \"#112233\"\ntext = \"#dddddd\"\n[layout]\nradius = 4",
        );
        let tweak = pack(
            "tweak",
            "[ui]\naccent = \"#ff0000\"\n[layout]\ndensity = \"compact\"",
        );
        let look = Look::stack(&[&theme, &tweak]).unwrap();
        assert_eq!(look.palette.accent, 0xff0000, "the higher pack's accent");
        assert_eq!(look.palette.text, 0xdddddd, "the lower pack's text stays");
        assert_eq!(look.layout.radius, 4.0);
        assert!(look.layout.compact);
    }

    #[test]
    fn icons_resolve_by_name_extension_and_folder() {
        let icons = pack(
            "icons",
            r##"
            [icons.files]
            "rs" = "rust.svg"
            "d.ts" = { badge = "DT", colour = "accent" }
            "ts" = { glyph = "T" }
            [icons.names]
            "Cargo.toml" = { svg = "cargo.svg", tint = "muted" }
            [icons.folders]
            default = "folder.svg"
            open = "folder-open.svg"
            "src" = "src.svg"
            "##,
        );
        let look = Look::stack(&[&icons]).unwrap();
        let i = &look.icons;
        assert_eq!(
            i.file("main.rs"),
            Some(&Icon::Svg {
                file: "/packs/icons/rust.svg".into(),
                tint: Tint::Own
            })
        );
        assert!(matches!(
            i.file("cargo.TOML"),
            Some(Icon::Svg {
                tint: Tint::Colour(_),
                ..
            })
        ));
        assert!(
            matches!(i.file("x.d.ts"), Some(Icon::Badge { colour: Some(c), .. }) if *c == look.palette.accent)
        );
        assert!(matches!(i.file("x.ts"), Some(Icon::Glyph { .. })));
        assert_eq!(i.file("notes.txt"), None);
        assert!(
            matches!(i.folder("src", true), Some(Icon::Svg { file, .. }) if file.ends_with("src.svg"))
        );
        assert!(
            matches!(i.folder("docs", true), Some(Icon::Svg { file, .. }) if file.ends_with("folder-open.svg"))
        );
        assert!(
            matches!(i.folder("docs", false), Some(Icon::Svg { file, .. }) if file.ends_with("folder.svg"))
        );
    }

    #[test]
    fn an_icon_pack_leaves_the_colours_alone() {
        let theme = pack("theme", "[ui]\naccent = \"#112233\"");
        let icons = pack("icons", "[icons]\nstyle = \"glyph\"");
        let look = Look::stack(&[&theme, &icons]).unwrap();
        assert_eq!(look.palette.accent, 0x112233);
        assert_eq!(look.icons.fallback, Fallback::Glyph);
        assert!(icons.parts().icons && !icons.parts().colours);
    }

    /// The bundled layout and icon packs read, and stack as they should.
    #[test]
    fn bundled_packs_stack() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let read = |label: &str, file: &str| Pack::read(label, &root.join(file)).unwrap();
        let line = read("Line Icons", "merlin-icons/line.toml");
        let compact = read("Compact", "merlin-layouts/compact.toml");
        let flat = read("Flat", "merlin-layouts/flat.toml");
        let soft = read("Soft", "merlin-layouts/soft.toml");
        let origami = read("Origami", "merlin-icons/origami.toml");
        let look = Look::stack(&[&origami, &soft, &line, &compact, &flat]).unwrap();
        assert_eq!(
            (look.layout.gap, look.layout.radius),
            (0.0, 0.0),
            "Flat over Soft"
        );
        assert!(look.layout.compact);
        assert!(look.icons.ui("search").is_some());
        assert!(look.icons.file("main.rs").is_some());
        assert!(look.icons.folder("src", true).is_some());
        for icon in [look.icons.ui("search"), look.icons.file("main.rs")]
            .into_iter()
            .flatten()
        {
            if let Icon::Svg { file, .. } = icon {
                assert!(file.exists(), "{} is there", file.display());
            }
        }
    }

    #[test]
    fn mistakes_say_where_they_are() {
        let bad = pack("bad", "[icons.files]\n\"rs\" = { tint = \"accent\" }");
        let err = format!("{:#}", Look::stack(&[&bad]).unwrap_err());
        assert!(err.contains("[icons.files] rs"), "{err}");
    }
}
