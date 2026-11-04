#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Esc,
    /// A function key, F1 to F12.
    F(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
    /// Only reported for keys that aren't characters: `A` is just `A`.
    pub shift: bool,
}

/// As shown to people, such as `Ctrl+Alt+W`.
impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("Space"),
            KeyCode::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            KeyCode::PageUp => f.write_str("PageUp"),
            KeyCode::PageDown => f.write_str("PageDown"),
            KeyCode::BackTab => f.write_str("Shift+Tab"),
            KeyCode::F(n) => write!(f, "F{n}"),
            other => write!(f, "{other:?}"),
        }
    }
}

impl Key {
    /// As settings write it, such as `ctrl+shift+p`, which [`Key::parse`]
    /// reads back.
    pub fn spec(&self) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.ctrl, "ctrl+"),
            (self.alt, "alt+"),
            (self.shift, "shift+"),
        ] {
            if on {
                s.push_str(name);
            }
        }
        match self.code {
            KeyCode::Char(' ') => s.push_str("space"),
            KeyCode::Char(c) => s.push(c.to_ascii_lowercase()),
            KeyCode::F(n) => s.push_str(&format!("f{n}")),
            other => s.push_str(&format!("{other:?}").to_lowercase()),
        }
        s
    }

    pub fn plain(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// Parses specs like `a`, `enter`, `space`, `ctrl+s` or `ctrl+shift+left`.
    pub fn parse(spec: &str) -> Option<Key> {
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        let mut rest = spec;
        loop {
            if let Some(r) = rest.strip_prefix("ctrl+") {
                ctrl = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("alt+") {
                alt = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("shift+") {
                shift = true;
                rest = r;
            } else {
                break;
            }
        }
        let code = match rest {
            "enter" => KeyCode::Enter,
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" => KeyCode::PageUp,
            "pagedown" => KeyCode::PageDown,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "esc" => KeyCode::Esc,
            "space" => KeyCode::Char(' '),
            f if f.len() > 1
                && f.starts_with('f')
                && f[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) =>
            {
                KeyCode::F(f[1..].parse().unwrap())
            }
            s => {
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return None,
                }
            }
        };
        Some(Key {
            code,
            ctrl,
            alt,
            shift,
        })
    }
}
