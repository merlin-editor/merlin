//! Git jobs, run on their own thread: status, diffs, blame, commits, and
//! push, pull and fetch, which can be stopped.

use super::*;

/// How often to ask the remote what's new.
pub(super) const FETCH_EVERY: Duration = Duration::from_secs(5 * 60);

/// How long blame waits for the asking to stop, as typing does.
pub(super) const BLAME_PAUSE: Duration = Duration::from_millis(400);

/// A file's blame to work out: its text as the editor has it.
pub(super) struct Blame {
    pub(super) path: PathBuf,
    pub(super) version: u64,
    pub(super) text: String,
    pub(super) ignore: Vec<String>,
}

/// The push, pull or fetch running, shared with the window's thread so it
/// can be stopped.
#[derive(Clone, Default)]
pub(super) struct GitNet {
    pub(super) pid: std::sync::Arc<std::sync::Mutex<Option<u32>>>,
    pub(super) cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl GitNet {
    /// Stops git, and the ssh it started, as a group.
    pub(super) fn cancel(&self) {
        let pid = *self.pid.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pid) = pid {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
            // The whole group, or tree on Windows: git and what it started.
            #[cfg(unix)]
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{pid}")])
                .status();
            #[cfg(windows)]
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .status();
        }
    }
}

/// What git jobs need to know of the project, to run on their own thread.
pub(super) struct Git {
    pub(super) root: PathBuf,
    pub(super) net: GitNet,
    /// Whether the project is in a repository at all.
    pub(super) repo: bool,
    /// Where the project is in the repository, as git names paths.
    pub(super) prefix: String,
    /// The repository's git folder.
    pub(super) dir: Option<PathBuf>,
}

impl Git {
    /// The branch and changed files, with paths relative to the project,
    /// then where the branch stands with its upstream.
    pub(super) fn status(&self) -> Vec<Event> {
        let output = self.repo.then(|| {
            Command::new("git")
                .args([
                    "status",
                    "--porcelain=v1",
                    "--branch",
                    "--untracked-files=all",
                    "--",
                    ".",
                ])
                .current_dir(&self.root)
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
        });
        let Some(output) = output.flatten() else {
            return vec![
                Event::GitStatus {
                    branch: None,
                    files: Vec::new(),
                    staged: Vec::new(),
                    changed: Vec::new(),
                },
                Event::GitSync(SyncState::default()),
                Event::GitMoves(Vec::new()),
                Event::GitMerge(None),
            ];
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let mut sync = SyncState::default();
        let mut moves = Vec::new();
        let mut branch = None;
        let mut files = Vec::new();
        let mut staged = Vec::new();
        let mut changed = Vec::new();
        for line in text.lines() {
            if let Some(head) = line.strip_prefix("## ") {
                sync = sync_state(head);
                let name = head.split("...").next().unwrap_or(head);
                let name = name.strip_prefix("No commits yet on ").unwrap_or(name);
                branch = Some(if name.starts_with("HEAD") {
                    "detached".into()
                } else {
                    name.to_string()
                });
                continue;
            }
            let (Some(status), Some(path)) = (line.get(..2), line.get(3..)) else {
                continue;
            };
            // A file with conflicts shows as `!`, in neither list.
            if matches!(status, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU") {
                let path = path.trim_matches('"');
                let path = path.strip_prefix(&self.prefix).unwrap_or(path);
                files.push((path.to_string(), '!'));
                continue;
            }
            let letter = match status {
                "??" => '?',
                s => s.chars().find(|c| *c != ' ').unwrap_or('M'),
            };
            // A rename shows as `old -> new`.
            if let Some((from, to)) = path.split_once(" -> ") {
                let strip = |p: &str| {
                    let p = p.trim_matches('"');
                    p.strip_prefix(&self.prefix).unwrap_or(p).to_string()
                };
                moves.push(GitMove {
                    from: strip(from),
                    to: strip(to),
                    staged: true,
                    similarity: None,
                });
            }
            let path = path.rsplit(" -> ").next().unwrap_or(path).trim_matches('"');
            let path = path.strip_prefix(&self.prefix).unwrap_or(path);
            files.push((path.to_string(), letter));
            // X is the index, Y the working tree.
            let mut xy = status.chars();
            let (x, y) = (xy.next().unwrap_or(' '), xy.next().unwrap_or(' '));
            if status == "??" {
                changed.push((path.to_string(), '?'));
                continue;
            }
            if x != ' ' {
                staged.push((path.to_string(), x));
            }
            if y != ' ' {
                changed.push((path.to_string(), y));
            }
        }
        // A branch with nowhere to push yet is published to the first
        // remote.
        if sync.upstream.is_none() {
            sync.remote = Command::new("git")
                .arg("remote")
                .current_dir(&self.root)
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .and_then(|o| {
                    let text = String::from_utf8_lossy(&o.stdout).into_owned();
                    text.lines().next().map(str::to_string)
                });
        }
        moves.extend(self.pair_moves(&changed));
        let conflicted: Vec<String> = files
            .iter()
            .filter(|(_, s)| *s == '!')
            .map(|(p, _)| p.clone())
            .collect();
        let merge = self.merge_state(&conflicted);
        vec![
            Event::GitStatus {
                branch,
                files,
                staged,
                changed,
            },
            Event::GitSync(sync),
            Event::GitMoves(moves),
            Event::GitMerge(merge),
        ]
    }

    /// The merge, rebase or cherry-pick git is in the middle of, from the
    /// files it keeps in its folder, and the conflicts left in each of
    /// `conflicted`, by their markers.
    pub(super) fn merge_state(&self, conflicted: &[String]) -> Option<MergeState> {
        let dir = self.dir.as_ref()?;
        let read = |name: &str| fs::read_to_string(dir.join(name)).ok();
        let message = read("MERGE_MSG").unwrap_or_default();
        let first = message.lines().next().unwrap_or_default().to_string();
        let (kind, from) = if dir.join("MERGE_HEAD").exists() {
            // "Merge branch 'feature/themes'" names it between quotes.
            let from = first.split('\'').nth(1).unwrap_or(&first).to_string();
            (MergeKind::Merge, from)
        } else if dir.join("rebase-merge").exists() || dir.join("rebase-apply").exists() {
            let onto = read("rebase-merge/head-name")
                .or_else(|| read("rebase-apply/head-name"))
                .unwrap_or_default();
            let from = onto.trim().trim_start_matches("refs/heads/").to_string();
            (MergeKind::Rebase, from)
        } else if dir.join("CHERRY_PICK_HEAD").exists() {
            (MergeKind::CherryPick, first.clone())
        } else {
            return None;
        };
        let conflicts = conflicted
            .iter()
            .map(|p| {
                let text = fs::read_to_string(self.root.join(p)).unwrap_or_default();
                let n = text.lines().filter(|l| l.starts_with("<<<<<<< ")).count();
                (p.clone(), n)
            })
            .collect();
        // Comment lines git adds after the message aren't part of it.
        let message: String = message
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        Some(MergeState {
            kind,
            from,
            message,
            conflicts,
        })
    }

    /// Deleted and new files not staged that are alike enough to be one
    /// file moved, as git pairs them once both are staged: at least half
    /// the same. Only a few of each, and not big ones, are compared.
    pub(super) fn pair_moves(&self, changed: &[(String, char)]) -> Vec<GitMove> {
        const MOST: usize = 20;
        const BIGGEST: u64 = 512 * 1024;
        let deleted: Vec<&String> = changed
            .iter()
            .filter(|(_, s)| *s == 'D')
            .map(|(p, _)| p)
            .take(MOST)
            .collect();
        if deleted.is_empty() {
            return Vec::new();
        }
        let added: Vec<&String> = changed
            .iter()
            .filter(|(_, s)| *s == '?')
            .map(|(p, _)| p)
            .filter(|p| fs::metadata(self.root.join(p)).is_ok_and(|m| m.len() <= BIGGEST))
            .take(MOST)
            .collect();
        let olds: Vec<(&String, String)> = deleted
            .into_iter()
            .filter_map(|p| Some((p, git_base(&self.root, Path::new(p))?)))
            .collect();
        let mut taken = Vec::new();
        let mut out = Vec::new();
        for new in added {
            let Ok(text) = fs::read_to_string(self.root.join(new)) else {
                continue;
            };
            let best = olds
                .iter()
                .filter(|(p, _)| !taken.contains(p))
                .map(|(p, old)| (*p, merlin_core::diff::similarity(old, &text)))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((from, alike)) = best.filter(|(_, a)| *a >= 0.5) {
                taken.push(from);
                out.push(GitMove {
                    from: from.clone(),
                    to: new.clone(),
                    staged: false,
                    similarity: Some((alike * 100.0).round() as u8),
                });
            }
        }
        out
    }

    /// Each changed file's hunks: in the index, and not yet staged.
    pub(super) fn diffs(&self) -> Event {
        let diff = |cached: bool| {
            if !self.repo {
                return Vec::new();
            }
            let mut args = vec!["diff", "--no-color", "--no-ext-diff", "--relative"];
            if cached {
                args.push("--cached");
            }
            Command::new("git")
                .args(&args)
                .current_dir(&self.root)
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| merlin_core::git::parse(&String::from_utf8_lossy(&o.stdout)))
                .unwrap_or_default()
        };
        Event::GitDiffs {
            staged: diff(true),
            changed: diff(false),
        }
    }

    /// `git apply` of a patch from `git diff --relative`, whose paths are
    /// the project's: git takes them from the repository's top.
    pub(super) fn apply(&self, patch: &str, cached: bool, reverse: bool) -> Result<(), String> {
        let directory = format!("--directory={}", self.prefix);
        let mut args = vec!["apply", "--recount", "--whitespace=nowarn"];
        if !self.prefix.is_empty() {
            args.push(&directory);
        }
        if cached {
            args.push("--cached");
        }
        if reverse {
            args.push("-R");
        }
        args.push("-");
        self.git(&args, Some(patch))
    }

    /// Runs git with `args`, and `input` on its standard input. Returns the
    /// first line of what it said when it failed.
    pub(super) fn git(&self, args: &[&str], input: Option<&str>) -> Result<(), String> {
        use std::io::Write;
        let mut child = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .stdin(if input.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("git: {e}"))?;
        if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
            let _ = stdin.write_all(text.as_bytes());
        }
        let o = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
        if o.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&o.stderr);
        let out = String::from_utf8_lossy(&o.stdout);
        let text = if err.trim().is_empty() { out } else { err };
        let first = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("failed")
            .trim_start_matches("error: ")
            .to_string();
        Err(format!("git {}: {first}", args[0]))
    }

    /// Runs git to talk to a remote: never asking for a password, since
    /// there's no terminal to type it in, and in its own process group so
    /// a cancel stops what it started too. Returns how it went and what it
    /// said.
    pub(super) fn net(&self, args: &[&str]) -> (SyncOutcome, String) {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(&self.root)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        // ssh asks for passphrases itself: not when nothing set how it runs.
        let configured = Command::new("git")
            .args(["config", "--get", "core.sshCommand"])
            .current_dir(&self.root)
            .output()
            .is_ok_and(|o| !o.stdout.is_empty());
        if env::var_os("GIT_SSH_COMMAND").is_none() && !configured {
            command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => return (SyncOutcome::Failed(format!("git: {e}")), String::new()),
        };
        let lock = |net: &GitNet| net.pid.lock().unwrap_or_else(|e| e.into_inner()).take();
        *self.net.pid.lock().unwrap_or_else(|e| e.into_inner()) = Some(child.id());
        let output = child.wait_with_output();
        lock(&self.net);
        let cancelled = self
            .net
            .cancelled
            .swap(false, std::sync::atomic::Ordering::SeqCst);
        let Ok(o) = output else {
            return (SyncOutcome::Failed("git didn't run".into()), String::new());
        };
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        if cancelled {
            return (SyncOutcome::Cancelled, said);
        }
        if o.status.success() {
            return (SyncOutcome::Done, said);
        }
        let has = |words: &[&str]| words.iter().any(|w| said.contains(w));
        let outcome = if has(&["[rejected]", "non-fast-forward", "fetch first"]) {
            SyncOutcome::Rejected
        } else if has(&[
            "terminal prompts disabled",
            "could not read Username",
            "could not read Password",
            "Authentication failed",
            "Permission denied (publickey",
            "Host key verification failed",
        ]) {
            SyncOutcome::NeedsAuth {
                remote: self.remote().unwrap_or_else(|| "The remote".into()),
            }
        } else if has(&["CONFLICT", "Automatic merge failed", "could not apply"]) {
            SyncOutcome::Conflicts(self.conflicted())
        } else {
            let first = said
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with("hint:"))
                .unwrap_or("failed");
            let first = first
                .trim_start_matches("fatal: ")
                .trim_start_matches("error: ");
            SyncOutcome::Failed(first.to_string())
        };
        (outcome, said)
    }

    /// The remote the branch pushes to, else the first there is.
    pub(super) fn remote(&self) -> Option<String> {
        let read = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&self.root)
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    let text = String::from_utf8_lossy(&o.stdout).into_owned();
                    text.lines().next().map(str::to_string)
                })
        };
        read(&["rev-parse", "--abbrev-ref", "@{upstream}"])
            .and_then(|u| u.split('/').next().map(str::to_string))
            .or_else(|| read(&["remote"]))
    }

    /// Files with conflicts, as the project names them.
    pub(super) fn conflicted(&self) -> Vec<String> {
        Command::new("git")
            .args(["diff", "--name-only", "--relative", "--diff-filter=U"])
            .current_dir(&self.root)
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `git pull`, as git is set to: rebase or merge. Set to neither, it
    /// merges, as git did before it asked.
    pub(super) fn pull(&self) -> (SyncOutcome, String) {
        let set = |key: &str| {
            Command::new("git")
                .args(["config", "--get", key])
                .current_dir(&self.root)
                .output()
                .is_ok_and(|o| !o.stdout.is_empty())
        };
        if set("pull.rebase") || set("pull.ff") {
            self.net(&["pull"])
        } else {
            self.net(&["pull", "--no-rebase"])
        }
    }

    /// A conflicted file's sides for the merge editor: the version both
    /// came from, for each conflict's base, and what each side is, as
    /// "themes · a41c9e2".
    pub(super) fn merge_sides(&self, path: String) -> Event {
        let base = self
            .read(&["show", &format!(":1:./{path}")])
            .unwrap_or_default();
        let short = |rev: &str| {
            self.read(&["rev-parse", "--short", rev])
                .map(|h| h.trim().to_string())
                .unwrap_or_default()
        };
        let branch = self
            .read(&["branch", "--show-current"])
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "HEAD".into());
        let current = format!("{branch} · {}", short("HEAD"));
        let merge = self.merge_state(&[]);
        let head = ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REBASE_HEAD"]
            .into_iter()
            .find(|h| self.dir.as_ref().is_some_and(|d| d.join(h).exists()))
            .unwrap_or("MERGE_HEAD");
        let from = merge.map(|m| m.from).unwrap_or_else(|| "Incoming".into());
        let incoming = format!("{from} · {}", short(head));
        Event::MergeSides {
            path,
            base,
            incoming,
            current,
        }
    }

    /// Git's words for `args`, or nothing if it failed.
    pub(super) fn read(&self, args: &[&str]) -> Option<String> {
        Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    }

    /// A commit's message past its summary, and its size.
    pub(super) fn commit_info(&self, hash: String) -> Event {
        let message = self
            .read(&["show", "-s", "--format=%B", &hash])
            .unwrap_or_default();
        let body = message
            .split_once('\n')
            .map_or("", |(_, rest)| rest)
            .trim()
            .to_string();
        let mut info = merlin_core::editor::CommitInfo {
            body,
            ..Default::default()
        };
        let stat = self
            .read(&["show", "--numstat", "--format=", &hash])
            .unwrap_or_default();
        for line in stat.lines().filter(|l| !l.trim().is_empty()) {
            let mut parts = line.split('\t');
            info.added += parts
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0);
            info.removed += parts
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0);
            info.files += 1;
        }
        Event::CommitInfo { hash, info }
    }

    /// What a commit changed in a file: the file before it and after.
    pub(super) fn commit_diff(&self, hash: &str, path: PathBuf) -> Event {
        let at = |rev: String| {
            self.read(&["show", &format!("{rev}:./{}", path.display())])
                .unwrap_or_default()
        };
        Event::ShowReview {
            plugin: "merlin.commit".into(),
            id: hash.chars().take(7).collect(),
            path: path.clone(),
            old: at(format!("{hash}^")),
            new: at(hash.to_string()),
        }
    }

    /// Who last changed each line of the editor's text of a file. A file
    /// git doesn't know has none.
    pub(super) fn blame(&self, ask: Blame) -> Event {
        use std::io::Write;
        let mut args = vec![
            "blame".to_string(),
            "--porcelain".into(),
            "--contents".into(),
        ];
        args.push("-".into());
        for rev in &ask.ignore {
            args.push("--ignore-rev".into());
            args.push(rev.clone());
        }
        args.push("--".into());
        args.push(ask.path.display().to_string());
        let child = Command::new("git")
            .args(&args)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let text = child.ok().and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(ask.text.as_bytes());
            }
            let o = child.wait_with_output().ok()?;
            o.status
                .success()
                .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
        });
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        Event::Blame {
            path: ask.path,
            version: ask.version,
            blame: text
                .map(|t| merlin_core::git::parse_blame(&t))
                .unwrap_or_default(),
            now,
        }
    }

    /// A push, pull, sync, publish or fetch, then what git says now.
    pub(super) fn sync(&self, kind: SyncKind) -> Vec<Event> {
        let (outcome, output) = match kind {
            SyncKind::Fetch => self.net(&["fetch", "--quiet", "--prune"]),
            SyncKind::Push => self.net(&["push"]),
            SyncKind::Pull => self.pull(),
            SyncKind::Sync => match self.pull() {
                (SyncOutcome::Done, pulled) => {
                    let (outcome, pushed) = self.net(&["push"]);
                    (outcome, pulled + &pushed)
                }
                other => other,
            },
            SyncKind::Publish => match self.remote() {
                Some(remote) => self.net(&["push", "--set-upstream", &remote, "HEAD"]),
                None => (
                    SyncOutcome::Failed("there's no remote to publish to".into()),
                    String::new(),
                ),
            },
        };
        // A rejected push learns how far behind it is, to say so.
        if outcome == SyncOutcome::Rejected {
            self.net(&["fetch", "--quiet"]);
        }
        let mut events = self.status();
        events.push(Event::Synced {
            kind,
            outcome,
            output,
        });
        events
    }

    /// Runs what Source Control asked for, and says how it went.
    pub(super) fn run(&self, action: GitAction) -> Vec<Event> {
        let done = match &action {
            GitAction::Sync(kind) => return self.sync(*kind),
            GitAction::CancelSync => return Vec::new(),
            // A merge stopped part way, else a rebase.
            GitAction::AbortPull => self
                .git(&["merge", "--abort"], None)
                .or_else(|_| self.git(&["rebase", "--abort"], None))
                .or_else(|_| self.git(&["cherry-pick", "--abort"], None)),
            // No editor opens: git keeps the message it has.
            GitAction::Continue => {
                let kind = if self
                    .dir
                    .as_ref()
                    .is_some_and(|d| d.join("CHERRY_PICK_HEAD").exists())
                {
                    "cherry-pick"
                } else {
                    "rebase"
                };
                let out = Command::new("git")
                    .args([kind, "--continue"])
                    .env("GIT_EDITOR", "true")
                    .current_dir(&self.root)
                    .stdin(std::process::Stdio::null())
                    .output();
                match out {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(format!(
                        "git {kind}: {}",
                        String::from_utf8_lossy(&o.stderr)
                            .lines()
                            .find(|l| !l.trim().is_empty())
                            .unwrap_or("failed")
                    )),
                    Err(e) => Err(format!("git: {e}")),
                }
            }
            GitAction::Stage(path) => self.git(&["add", "--", path], None),
            GitAction::Unstage(path) => self.git(&["restore", "--staged", "--", path], None),
            GitAction::StageAll => self.git(&["add", "--all", "--", "."], None),
            GitAction::UnstageAll => self.git(&["restore", "--staged", "--", "."], None),
            GitAction::Commit(message) => self.git(&["commit", "--quiet", "-m", message], None),
            GitAction::Apply {
                patch,
                cached,
                reverse,
            } => self.apply(patch, *cached, *reverse),
            GitAction::StageMove { from, to } => self.git(&["add", "-A", "--", from, to], None),
            GitAction::Ignore(path) => {
                let file = self.root.join(".gitignore");
                let mut text = fs::read_to_string(&file).unwrap_or_default();
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&format!("/{path}\n"));
                fs::write(&file, text).map_err(|e| format!(".gitignore: {e}"))
            }
            // A deleted file comes back from the last commit, in the index
            // too; there's nothing to undo to.
            GitAction::Discard { path, patch: None } if !self.root.join(path).exists() => {
                let done = self.git(
                    &[
                        "restore",
                        "--source=HEAD",
                        "--staged",
                        "--worktree",
                        "--",
                        path,
                    ],
                    None,
                );
                return match done {
                    Ok(()) => {
                        let name = path.rsplit('/').next().unwrap_or(path);
                        std::iter::once(Event::Notify(format!("Restored {name}")))
                            .chain(self.status())
                            .collect()
                    }
                    Err(e) => vec![Event::Error(e)],
                };
            }
            GitAction::Discard { path, patch } => {
                let file = self.root.join(path);
                let before = fs::read_to_string(&file).unwrap_or_default();
                let done = match patch {
                    Some(patch) => self.apply(patch, false, true),
                    None => self.git(&["restore", "--worktree", "--", path], None),
                };
                return match done {
                    Ok(()) => std::iter::once(Event::Discarded {
                        path: path.clone(),
                        before,
                        all: patch.is_none(),
                    })
                    .chain(self.status())
                    .collect(),
                    Err(e) => vec![Event::Error(e)],
                };
            }
            GitAction::Undiscard { path, text } => {
                fs::write(self.root.join(path), text).map_err(|e| format!("{path}: {e}"))
            }
        };
        match (done, action) {
            (Ok(()), GitAction::Commit(message)) => {
                let first = message.lines().next().unwrap_or_default().to_string();
                vec![Event::Notify(format!("Committed: {first}"))]
            }
            (Ok(()), _) => self.status(),
            (Err(e), _) => vec![Event::Error(e)],
        }
    }
}

/// Where a branch stands, from `git status --branch`'s first line, as
/// `main...origin/main [ahead 1, behind 2]`.
pub(super) fn sync_state(head: &str) -> SyncState {
    let mut sync = SyncState::default();
    let Some((_, rest)) = head.split_once("...") else {
        return sync;
    };
    let (upstream, counts) = match rest.split_once(" [") {
        Some((u, c)) => (u, c.trim_end_matches(']')),
        None => (rest, ""),
    };
    // An upstream that's gone is none.
    if counts == "gone" {
        return sync;
    }
    sync.upstream = Some(upstream.to_string());
    for part in counts.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            sync.ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            sync.behind = n.parse().unwrap_or(0);
        }
    }
    sync
}

/// The git folder of the repository `root` is in, if any.
pub(super) fn git_dir(root: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let dir = String::from_utf8(output.stdout).ok()?;
    output.status.success().then(|| root.join(dir.trim()))
}

/// Where `root` is in its repository, as `git status` prefixes paths.
pub(super) fn git_prefix(root: &Path) -> String {
    Command::new("git")
        .args(["rev-parse", "--show-prefix"])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// A file as of the last commit, or `None` if git doesn't track it.
pub(super) fn git_base(root: &Path, path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("show")
        .arg(format!("HEAD:./{}", path.display()))
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}
