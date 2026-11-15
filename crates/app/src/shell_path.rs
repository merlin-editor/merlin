//! The `PATH` your shell sets up, for a merlin started from the desktop.
//!
//! A program started from a desktop launcher gets the desktop session's
//! `PATH`, not the one `~/.profile`, `~/.zshrc` or `~/.bashrc` builds. So
//! programs in `~/.local/bin`, `~/.cargo/bin`, npm's or Go's folders look
//! missing: `rust-analyzer` won't start, and neither would `claude`. As
//! VS Code does, merlin asks your login shell for its `PATH` once, at
//! start. Started from a terminal, it already has it.

use std::io::{IsTerminal, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Printed around the shell's environment, to find it among whatever its
/// startup files print.
const MARK: &str = "_MERLIN_SHELL_ENV_";
/// The longest merlin waits for the shell.
const WAIT: Duration = Duration::from_secs(3);

/// Takes the login shell's `PATH` when merlin wasn't started from a
/// terminal. Call it first thing in `main`, before any other thread starts.
/// `MERLIN_NO_SHELL_PATH` turns it off.
pub(crate) fn adopt() {
    if !cfg!(unix)
        || std::io::stdin().is_terminal()
        || std::env::var_os("MERLIN_NO_SHELL_PATH").is_some()
    {
        return;
    }
    let Some(shell) = std::env::var("SHELL").ok().filter(|s| !s.is_empty()) else {
        return;
    };
    if let Some(path) = login_path(&shell) {
        // SAFETY: called at the start of `main`, while it's the only thread.
        unsafe { std::env::set_var("PATH", path) };
    }
}

/// The `PATH` an interactive login `shell` ends up with, or none if it
/// fails or takes too long.
fn login_path(shell: &str) -> Option<String> {
    // `env` and `echo` read the same in sh, bash, zsh and fish, unlike
    // "$PATH", which fish joins with spaces.
    let script = format!("echo {MARK}; env; echo {MARK}");
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    // Read on a thread, so a shell that prints a lot can't fill the pipe
    // and stall while this waits for it to exit.
    // Bytes, not text: one variable that isn't UTF-8 mustn't lose them all.
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < WAIT => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    path_in(&reader.join().ok()?)
}

/// The `PATH=` line between the marks in a shell's output. A mark counts
/// only as a whole line: the shell's `$_` holds the last word of `echo`,
/// so `env` prints `_=` and the mark too.
fn path_in(out: &str) -> Option<String> {
    let mut lines = out.lines().skip_while(|l| *l != MARK);
    lines.next()?;
    lines
        .take_while(|l| *l != MARK)
        .find_map(|l| l.strip_prefix("PATH="))
        .map(str::to_string)
        .filter(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_is_read_from_between_the_marks() {
        let out = format!(
            "Welcome!\nPATH=/not/this\n{MARK}\nHOME=/home/a\nPATH=/home/a/.local/bin:/usr/bin\n{MARK}\nbye\n"
        );
        assert_eq!(
            path_in(&out).as_deref(),
            Some("/home/a/.local/bin:/usr/bin")
        );
        assert_eq!(path_in("PATH=/usr/bin\n"), None, "no marks");
        let with_underscore = format!("{MARK}\n_={MARK}\nPATH=/home/a/bin:/usr/bin\n{MARK}\n");
        assert_eq!(
            path_in(&with_underscore).as_deref(),
            Some("/home/a/bin:/usr/bin"),
            "$_ isn't the end"
        );
        assert_eq!(
            path_in(&format!("{MARK}\nHOME=/x\n{MARK}")),
            None,
            "no PATH"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_gives_its_path() {
        // sh -l -i reads ~/.profile; whatever it says, the PATH comes back.
        let path = login_path("/bin/sh").expect("sh answers");
        assert!(path.contains('/'), "{path}");
    }

    #[test]
    fn a_shell_that_isnt_there_gives_nothing() {
        assert_eq!(login_path("/no/such/shell"), None);
    }
}
