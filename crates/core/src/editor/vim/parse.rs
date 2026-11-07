//! Reading Vim's keys as they come: a register, counts, an operator and
//! its motion or text object, as a whole command or a wait for more.

use super::*;

/// The key as the parser sees it, or `None` for one Vim leaves to merlin.
pub(super) fn token(key: &Key) -> Option<char> {
    use KeyCode::*;
    if key.alt {
        return None;
    }
    if key.ctrl {
        return match key.code {
            Char('[') if !key.shift => Some(ESC),
            Char(c) if c.is_ascii_lowercase() && !key.shift => Some(ctrl(c)),
            _ => None,
        };
    }
    if key.shift && !matches!(key.code, Char(_)) {
        return None;
    }
    Some(match key.code {
        Char(c) => c,
        Esc => ESC,
        Enter => ENTER,
        Backspace => BACKSPACE,
        Tab => '\t',
        Left => LEFT,
        Right => RIGHT,
        Up => UP,
        Down => DOWN,
        Home => HOME,
        End => END,
        PageUp => PAGE_UP,
        PageDown => PAGE_DOWN,
        Delete => DELETE,
        _ => return None,
    })
}

pub(super) fn count_at(s: &[char], i: &mut usize) -> Option<usize> {
    let from = *i;
    if s.get(*i).is_some_and(|c| ('1'..='9').contains(c)) {
        while s.get(*i).is_some_and(|c| c.is_ascii_digit()) {
            *i += 1;
        }
    }
    (from < *i).then(|| {
        s[from..*i]
            .iter()
            .collect::<String>()
            .parse::<usize>()
            .unwrap_or(1)
            .min(99_999)
    })
}

/// A motion at the start of `s`, and how many keys it took.
pub(super) fn motion(s: &[char]) -> Result<Option<usize>, ()> {
    let Some(&c) = s.first() else {
        return Ok(None);
    };
    if MOTIONS.contains(c) {
        return Ok(Some(1));
    }
    match c {
        'f' | 'F' | 't' | 'T' | '\'' | '`' => Ok(s.get(1).map(|_| 2)),
        'g' => match s.get(1) {
            None => Ok(None),
            Some('g' | 'e' | 'E' | '_' | '0' | '^' | '$' | 'j' | 'k') => Ok(Some(2)),
            Some(_) => Err(()),
        },
        _ => Err(()),
    }
}

/// A text object at the start of `s`, such as `iw` or `a(`.
pub(super) fn object(s: &[char]) -> Result<Option<usize>, ()> {
    match s.first() {
        Some('i' | 'a') => match s.get(1) {
            None => Ok(None),
            Some(c) if "wWsp()b[]{}B<>\"'`fc".contains(*c) => Ok(Some(2)),
            Some(_) => Err(()),
        },
        _ => Err(()),
    }
}

/// An operator at the start of `s`: which, and how many keys.
pub(super) fn operator(s: &[char]) -> Result<Option<(Op, usize)>, ()> {
    let one = match s.first() {
        Some('d') => Op::Delete,
        Some('c') => Op::Change,
        Some('y') => Op::Yank,
        Some('>') => Op::Indent,
        Some('<') => Op::Outdent,
        Some('g') => {
            return match s.get(1) {
                None => Ok(None),
                Some('~') => Ok(Some((Op::Toggle, 2))),
                Some('u') => Ok(Some((Op::Lower, 2))),
                Some('U') => Ok(Some((Op::Upper, 2))),
                Some('c') => Ok(Some((Op::Comment, 2))),
                Some(_) => Err(()),
            };
        }
        _ => return Err(()),
    };
    Ok(Some((one, 1)))
}

/// Normal mode's other commands, one key or two.
pub(super) fn action(s: &[char], visual: bool) -> Result<Option<usize>, ()> {
    let Some(&c) = s.first() else {
        return Ok(None);
    };
    let one = if visual {
        "xXsSCDYRpPJuUoOIA~vV:/?\x1b\x16"
    } else {
        "iaIAoOxXsSCDYpPJuR.vV~K&:/?\x1b\x12\x16\x01\x18\x0f\x05\x19\x04\x15\x06\x02\t\u{e006}\u{e007}\u{e008}"
    };
    if one.contains(c) {
        return Ok(Some(1));
    }
    match c {
        'r' | 'm' => Ok(s.get(1).map(|_| 2)),
        'q' | '@' if !visual => Ok(s.get(1).map(|_| 2)),
        'z' if !visual => match s.get(1) {
            None => Ok(None),
            Some('z' | 't' | 'b' | '.' | '-' | '\r' | 'o' | 'c' | 'a' | 'R' | 'M') => Ok(Some(2)),
            Some(_) => Err(()),
        },
        '[' | ']' if !visual => match s.get(1) {
            None => Ok(None),
            Some('c' | 'd') => Ok(Some(2)),
            Some(_) => Err(()),
        },
        '\x17' if !visual => match s.get(1) {
            None => Ok(None),
            Some('w' | 'W' | 'v' | 's' | 'c' | 'q' | 'o' | 'h' | 'l' | 'j' | 'k' | '\x17') => {
                Ok(Some(2))
            }
            Some(_) => Err(()),
        },
        'Z' if !visual => match s.get(1) {
            None => Ok(None),
            Some('Z' | 'Q') => Ok(Some(2)),
            Some(_) => Err(()),
        },
        'g' => match s.get(1) {
            None => Ok(None),
            Some('v' | 'J' | 'd' | 'D' | 'r' | 'h' | 'a' | 't' | 'T' | 'i' | 'I' | 'x') => {
                Ok(Some(2))
            }
            Some(_) => Err(()),
        },
        _ => Err(()),
    }
}

pub(super) fn parse(s: &[char], visual: bool) -> Parsed {
    let mut i = 0;
    let mut reg = None;
    if s.first() == Some(&'"') {
        match s.get(1) {
            None => return Parsed::Wait,
            Some(&r) => reg = Some(r),
        }
        i = 2;
    }
    let count = count_at(s, &mut i);
    let rest = &s[i..];
    if rest.is_empty() {
        return Parsed::Wait;
    }
    let done = |what| Parsed::Done(Cmd { reg, count, what });
    let text = |n: usize| rest[..n].iter().collect::<String>();
    if !visual {
        match operator(rest) {
            Ok(None) => return Parsed::Wait,
            Ok(Some((op, n))) => {
                let mut j = n;
                let count2 = count_at(rest, &mut j);
                let after = &rest[j..];
                let Some(&next) = after.first() else {
                    return Parsed::Wait;
                };
                // Doubled, for whole lines: dd, >>, gcc, guu or gugu.
                let last = rest[n - 1];
                if next == last {
                    return done(What::Op(op, count2, Target::Lines));
                }
                if n == 2 && next == 'g' {
                    return match after.get(1) {
                        None => Parsed::Wait,
                        Some(c) if *c == last => done(What::Op(op, count2, Target::Lines)),
                        _ => match motion(after) {
                            Ok(Some(m)) => done(What::Op(
                                op,
                                count2,
                                Target::Motion(after[..m].iter().collect()),
                            )),
                            _ => Parsed::Bad,
                        },
                    };
                }
                return match object(after) {
                    Ok(None) => Parsed::Wait,
                    Ok(Some(m)) => done(What::Op(
                        op,
                        count2,
                        Target::Object(after[..m].iter().collect()),
                    )),
                    Err(()) => match motion(after) {
                        Ok(None) => Parsed::Wait,
                        Ok(Some(m)) => done(What::Op(
                            op,
                            count2,
                            Target::Motion(after[..m].iter().collect()),
                        )),
                        Err(()) => Parsed::Bad,
                    },
                };
            }
            Err(()) => {}
        }
    } else {
        match object(rest) {
            Ok(None) => return Parsed::Wait,
            Ok(Some(n)) => return done(What::Object(text(n))),
            Err(()) => {}
        }
        // An operator acts on the selection at once.
        match rest {
            ['d' | 'c' | 'y' | '>' | '<', ..] => return done(What::Do(text(1))),
            ['g', next, ..] if "~uUcJv".contains(*next) => return done(What::Do(text(2))),
            _ => {}
        }
    }
    match motion(rest) {
        Ok(None) => return Parsed::Wait,
        Ok(Some(n)) => return done(What::Move(text(n))),
        Err(()) => {}
    }
    match action(rest, visual) {
        Ok(None) => Parsed::Wait,
        Ok(Some(n)) => done(What::Do(text(n))),
        Err(()) => Parsed::Bad,
    }
}

/// The commands that change the text, which `.` repeats.
pub(super) fn is_change(what: &What) -> bool {
    match what {
        What::Op(op, ..) => *op != Op::Yank,
        What::Do(keys) => {
            let c = keys.chars().next().unwrap_or(' ');
            "iaIAoOxXsSCDpPJR~r\x01\x18".contains(c) || matches!(keys.as_str(), "gJ" | "gi" | "gI")
        }
        _ => false,
    }
}

/// Keys as the status bar shows them.
pub(super) fn show_keys(keys: &str) -> String {
    keys.chars()
        .map(|c| match c {
            ESC => "⎋".to_string(),
            ENTER => "↵".to_string(),
            c if (c as u32) < 27 => format!("^{}", ((c as u8) + b'A' - 1) as char),
            c => c.to_string(),
        })
        .collect()
}

/// Whether the keys so far are an operator waiting for its motion.
pub(super) fn parse_waits_for_motion(pending: &[char]) -> bool {
    let mut i = 0;
    if pending.first() == Some(&'"') {
        i = 2;
    }
    let _ = count_at(pending, &mut i);
    let rest = pending.get(i..).unwrap_or_default();
    match operator(rest) {
        Ok(Some((_, n))) => {
            let mut j = n;
            let _ = count_at(rest, &mut j);
            matches!(parse(pending, false), Parsed::Wait) && j >= n
        }
        _ => false,
    }
}
