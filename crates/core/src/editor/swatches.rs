//! Colours written in code, such as `#e6c07b` or `rgb(230 192 123)`, for
//! the swatch the window draws under each.

/// Each colour on a line: its first character, its length in characters,
/// and the colour as `0xRRGGBBAA`.
pub(crate) fn find(chars: &[char]) -> Vec<(usize, usize, u32)> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let after_word = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        let hit = match chars[i] {
            '#' if i == 0 || !chars[i - 1].is_alphanumeric() => hex(&chars[i..]),
            'r' | 'h' if !after_word => function(&chars[i..]),
            _ => None,
        };
        match hit {
            Some((len, colour)) => {
                found.push((i, len, colour));
                i += len;
            }
            None => i += 1,
        }
    }
    found
}

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`, not followed by more of a
/// word.
fn hex(chars: &[char]) -> Option<(usize, u32)> {
    let digits = chars[1..]
        .iter()
        .take_while(|c| c.is_ascii_hexdigit())
        .count();
    if chars
        .get(1 + digits)
        .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        return None;
    }
    let d: Vec<u32> = chars[1..=digits]
        .iter()
        .map(|c| c.to_digit(16).unwrap_or(0))
        .collect();
    let colour = match digits {
        3 | 4 => {
            let a = if digits == 4 { d[3] * 17 } else { 255 };
            (d[0] * 17) << 24 | (d[1] * 17) << 16 | (d[2] * 17) << 8 | a
        }
        6 | 8 => {
            let byte = |k: usize| d[k] << 4 | d[k + 1];
            let a = if digits == 8 { byte(6) } else { 255 };
            byte(0) << 24 | byte(2) << 16 | byte(4) << 8 | a
        }
        _ => return None,
    };
    Some((1 + digits, colour))
}

/// `rgb(…)`, `rgba(…)`, `hsl(…)` or `hsla(…)`, with commas or spaces, and
/// an alpha after a comma or `/`.
fn function(chars: &[char]) -> Option<(usize, u32)> {
    let open = chars.iter().take(5).position(|&c| c == '(')?;
    let name: String = chars[..open].iter().collect();
    let hsl = match name.as_str() {
        "rgb" | "rgba" => false,
        "hsl" | "hsla" => true,
        _ => return None,
    };
    let close = open + chars[open..].iter().take(64).position(|&c| c == ')')?;
    let inside: String = chars[open + 1..close].iter().collect();
    let parts: Vec<&str> = inside
        .split([',', ' ', '/'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    // A number, maybe a percentage of `full`, or an angle in degrees.
    let value = |p: &str, full: f32| -> Option<f32> {
        match p.strip_suffix('%') {
            Some(n) => n.parse::<f32>().ok().map(|n| n / 100.0 * full),
            None => p.strip_suffix("deg").unwrap_or(p).parse::<f32>().ok(),
        }
    };
    let alpha = match parts.get(3) {
        Some(a) => value(a, 1.0)?.clamp(0.0, 1.0),
        None => 1.0,
    };
    let (r, g, b) = if hsl {
        let h = value(parts[0], 360.0)?.rem_euclid(360.0) / 360.0;
        let s = value(parts[1], 1.0)?.clamp(0.0, 1.0);
        let l = value(parts[2], 1.0)?.clamp(0.0, 1.0);
        hsl_to_rgb(h, s, l)
    } else {
        let c = |p: &str| value(p, 255.0).map(|n| n.clamp(0.0, 255.0) / 255.0);
        (c(parts[0])?, c(parts[1])?, c(parts[2])?)
    };
    let byte = |f: f32| (f * 255.0).round() as u32;
    Some((
        close + 1,
        byte(r) << 24 | byte(g) << 16 | byte(b) << 8 | byte(alpha),
    ))
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use super::find;

    fn colours(text: &str) -> Vec<(usize, usize, u32)> {
        find(&text.chars().collect::<Vec<_>>())
    }

    #[test]
    fn hex_colours_in_their_forms() {
        assert_eq!(colours("a: #e6c07b;"), vec![(3, 7, 0xe6c07bff)]);
        assert_eq!(colours("#fff"), vec![(0, 4, 0xffffffff)]);
        assert_eq!(colours("#0008"), vec![(0, 5, 0x00000088)]);
        assert_eq!(colours("\"#11223344\""), vec![(1, 9, 0x11223344)]);
    }

    #[test]
    fn not_ids_or_other_hashes() {
        assert!(colours("#12345").is_empty());
        assert!(colours("#fffz").is_empty());
        assert!(colours("issue#123").is_empty());
        assert!(colours("#include").is_empty());
    }

    #[test]
    fn rgb_and_hsl() {
        assert_eq!(colours("rgb(255, 0, 0)"), vec![(0, 14, 0xff0000ff)]);
        assert_eq!(colours("rgba(0 0 255 / 50%)"), vec![(0, 19, 0x0000ff80)]);
        assert_eq!(colours("hsl(120, 100%, 50%)"), vec![(0, 19, 0x00ff00ff)]);
        assert!(colours("color(srgb 1 0 0)").is_empty());
        assert!(colours("hrgb(1,2,3)").is_empty());
    }
}
