//! The system's fonts for shaping, read once and shared by the interface's
//! text and the code's.
//!
//! cosmic-text keeps every font it opens mapped for good, and when a
//! character is in no font it knows to try for it, it opens every font
//! there is: on a system with Noto's thousands of files, a few hundred MB.
//! So the last resort here is a short list of broad fonts. A script's own
//! fonts and the common fallbacks are tried as cosmic-text has them, and
//! open only when text needs them.

use std::collections::HashSet;
use std::sync::OnceLock;

use cosmic_text::fontdb::Database;
use cosmic_text::{Fallback, FontSystem, PlatformFallback};
use unicode_script::Script;

/// Families tried for a character after its script's and the common ones:
/// broad fonts, symbols and emoji. Icon fonts, named in [`is_icons`], are
/// tried too.
const LAST_RESORT: &[&str] = &[
    "Noto Sans",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Math",
    "Noto Color Emoji",
    "Noto Emoji",
    "DejaVu Sans",
    "DejaVu Sans Mono",
    "Symbols Nerd Font",
    "Symbols Nerd Font Mono",
];

/// Icon fonts, which carry the icons the core draws without a Nerd Font
/// for the code.
fn is_icons(family: &str) -> bool {
    ["Nerd Font", "Awesome", "Material Design Icons", "Powerline"]
        .iter()
        .any(|part| family.contains(part))
}

/// The fallbacks: the platform's, but the last resort never opens a
/// family outside [`LAST_RESORT`] or an icon font.
struct Fallbacks {
    forbidden: Vec<&'static str>,
}

impl Fallback for Fallbacks {
    fn common_fallback(&self) -> &[&'static str] {
        PlatformFallback.common_fallback()
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &self.forbidden
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        PlatformFallback.script_fallback(script, locale)
    }
}

/// The fonts installed, read once, and the families the last resort
/// mustn't open.
fn installed() -> &'static (String, Database, Vec<&'static str>) {
    static FONTS: OnceLock<(String, Database, Vec<&'static str>)> = OnceLock::new();
    FONTS.get_or_init(|| {
        let (locale, db) = FontSystem::new().into_locale_and_db();
        // A face is ruled out by any of its names, so one wanted by any
        // name, as "JetBrainsMono Nerd Font" is also "JetBrainsMono NF",
        // keeps them all.
        let wanted = |name: &str| LAST_RESORT.contains(&name) || is_icons(name);
        let kept: HashSet<&str> = db
            .faces()
            .filter(|f| f.families.iter().any(|(n, _)| wanted(n)))
            .flat_map(|f| f.families.iter().map(|(n, _)| n.as_str()))
            .collect();
        let mut seen = HashSet::new();
        let mut forbidden: Vec<&'static str> = Vec::new();
        for face in db.faces() {
            for (name, _) in &face.families {
                if !kept.contains(name.as_str()) && seen.insert(name.clone()) {
                    forbidden.push(Box::leak(name.clone().into_boxed_str()));
                }
            }
        }
        (locale, db, forbidden)
    })
}

/// A font system over the installed fonts, with the short last resort.
pub fn system() -> FontSystem {
    let (locale, db, forbidden) = installed();
    FontSystem::new_with_locale_and_db_and_fallback(
        locale.clone(),
        db.clone(),
        Fallbacks {
            forbidden: forbidden.clone(),
        },
    )
}
