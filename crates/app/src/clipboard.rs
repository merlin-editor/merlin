//! The system clipboard, through whichever tool the desktop has.

use std::io::{self, Write};
use std::process::{Command, Stdio};

type Tool = &'static [&'static str];

/// The commands that write and read the clipboard here.
fn tools() -> Option<(Tool, Tool)> {
    if cfg!(target_os = "macos") {
        Some((&["pbcopy"], &["pbpaste"]))
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        Some((&["wl-copy"], &["wl-paste", "--no-newline"]))
    } else if std::env::var_os("DISPLAY").is_some() {
        Some((
            &["xclip", "-selection", "clipboard"],
            &["xclip", "-selection", "clipboard", "-o"],
        ))
    } else {
        None
    }
}

fn command(parts: Tool) -> Command {
    let mut command = Command::new(parts[0]);
    command.args(&parts[1..]).stderr(Stdio::null());
    command
}

/// Copies `text`. Without a clipboard tool, asks the terminal to do it with
/// OSC 52, which works over SSH too.
pub fn copy(text: &str) {
    let copied = tools().is_some_and(|(copy, _)| {
        let child = command(copy)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn();
        child.is_ok_and(|mut child| {
            let wrote = child
                .stdin
                .take()
                .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
            // wl-copy and xclip fork to keep serving the clipboard, so this
            // returns once they've read the text.
            child.wait().is_ok_and(|s| s.success()) && wrote
        })
    });
    if !copied {
        let mut out = io::stdout();
        let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
        let _ = out.flush();
    }
}

/// The clipboard's text, or `None` if there's no tool or it's empty.
pub fn paste() -> Option<String> {
    let (_, paste) = tools()?;
    let output = command(paste).stdin(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64("héllo".as_bytes()), "aMOpbGxv");
    }
}
