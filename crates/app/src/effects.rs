//! What the editor asks of the app, done: files, git, plugins, terminals,
//! the clipboard and the rest, each turned back into events.

use super::*;

impl App {
    pub(super) fn run_events(&mut self, mut queue: VecDeque<Event>) {
        loop {
            let Some(event) = queue.pop_front() else {
                // Jobs done at once, as scripts have them, finish here.
                match self.jobs.take() {
                    Some(finish) => {
                        queue.extend(finish(self));
                        continue;
                    }
                    None => break,
                }
            };
            if matches!(event, Event::RegisterPlugin { .. }) {
                self.buffer_seen = None;
            }
            for effect in self.editor.update(event) {
                self.run_effect(effect, &mut queue);
            }
        }
    }

    /// Does what the editor asked: at once, or as a job whose events come
    /// back later. Events it makes at once join `queue`.
    fn run_effect(&mut self, effect: Effect, queue: &mut VecDeque<Event>) {
        match effect {
            Effect::Save {
                doc,
                path,
                text,
                revision,
            } => {
                let full = self.root.join(&path);
                self.jobs.run(Lane::Disk, move || {
                    let written = disk::write_atomic(&full, &text).map(|()| modified(&full));
                    Box::new(move |app: &mut App| match written {
                        Ok(when) => app.saved_file(doc, revision, path, when),
                        Err(err) => vec![Event::SaveFailed(err.to_string())],
                    })
                });
            }
            Effect::Open(path) => self.open_later(&path),
            Effect::ListFiles => {
                let root = self.root.clone();
                self.jobs.run(Lane::Scan, move || {
                    jobs::events(vec![Event::Files(list_files(&root))])
                });
            }
            Effect::ListTree => self.list_tree_later(),
            Effect::File(op) => self.file_later(op),
            Effect::Search(query) => {
                let root = self.root.clone();
                self.jobs.run(Lane::Scan, move || {
                    let hits = search(&root, &query);
                    jobs::events(vec![Event::SearchResults { query, hits }])
                });
            }
            Effect::GitStatus => self.git_status_later(),
            // Asked for as the text changes, it runs after a pause.
            Effect::Blame {
                path,
                version,
                text,
                ignore,
            } => {
                let due = Instant::now() + BLAME_PAUSE;
                self.blame_due = Some((
                    due,
                    Blame {
                        path,
                        version,
                        text,
                        ignore,
                    },
                ));
            }
            Effect::MergeSides(path) => {
                let git = self.git_info();
                self.jobs
                    .run(Lane::Git, move || jobs::events(vec![git.merge_sides(path)]));
            }
            Effect::CommitInfo(hash) => {
                let git = self.git_info();
                self.jobs
                    .run(Lane::Git, move || jobs::events(vec![git.commit_info(hash)]));
            }
            Effect::OpenCommit { hash, path } => {
                let git = self.git_info();
                self.jobs.run(Lane::Git, move || {
                    jobs::events(vec![git.commit_diff(&hash, path)])
                });
            }
            // Stopping a push or pull can't wait behind it.
            Effect::Git(GitAction::CancelSync) => self.git_net.cancel(),
            Effect::Git(action) => self.git_later(action),
            Effect::OpenDiff(path) => {
                let root = self.root.clone();
                self.jobs.run(Lane::Git, move || {
                    jobs::events(vec![changes_of(&root, &path, path.clone())])
                });
            }
            Effect::OpenMove { from, to } => {
                let root = self.root.clone();
                self.jobs.run(Lane::Git, move || {
                    jobs::events(vec![changes_of(&root, &from, to)])
                });
            }
            Effect::ReadFiles(paths) => {
                let root = self.root.clone();
                self.jobs.run(Lane::Disk, move || {
                    let files = paths
                        .into_iter()
                        .map(|p| {
                            let text = fs::read_to_string(root.join(&p)).ok();
                            (p, text)
                        })
                        .collect();
                    jobs::events(vec![Event::FilesRead(files)])
                });
            }
            Effect::OpenUrl(url) => open_url(&url),
            Effect::Copy(text) => {
                if let Some(clipboard) = &mut self.clipboard {
                    clipboard.copy(&text);
                }
            }
            Effect::Terminal(op) => self.terminal(op),
            Effect::ReadClipboard => {
                let text = self.clipboard.as_mut().and_then(|c| c.paste());
                queue.push_back(Event::Clipboard(text));
            }
            Effect::SetSelection(text) => {
                if let Some(clipboard) = &mut self.clipboard {
                    clipboard.select(&text);
                }
            }
            Effect::ReadSelection => {
                if let Some(text) = self.clipboard.as_mut().and_then(|c| c.selection()) {
                    queue.push_back(Event::Paste(text));
                }
            }
            Effect::Quit => self.quit = true,
            // Scripts can't answer a dialog: it's as if cancelled.
            Effect::PickFile(pick) if self.scripted => {
                queue.push_back(Event::Picked { pick, path: None });
            }
            Effect::PickFile(pick) => self.pick_file(pick),
            Effect::OpenFolder(folder) => {
                if let Err(err) = open_window(&folder) {
                    queue.push_back(Event::Error(format!(
                        "Can't open {}: {err}",
                        folder.display()
                    )));
                }
            }
            Effect::Debug {
                plugin,
                command,
                start,
                breakpoints,
            } => {
                self.host.set_debug(&plugin, start.map(|s| *s), breakpoints);
                let snapshot = self.snapshot();
                queue.extend(self.host.run_command(&plugin, &command, snapshot));
                self.panels_for = None;
            }
            Effect::ListProcesses => {
                let (list, ptrace_scope) = processes();
                queue.push_back(Event::Processes { list, ptrace_scope });
            }
            Effect::ReadLaunch => {
                let path = self.root.join(".merlin/launch.toml");
                let configs = match fs::read_to_string(&path) {
                    Ok(text) => launch_configs(&text),
                    Err(_) => Ok(Vec::new()),
                };
                queue.push_back(Event::LaunchConfigs(configs));
            }
            Effect::LocateDebuggers(programs) => {
                let found = programs
                    .into_iter()
                    .map(|p| {
                        let there = on_path(&p);
                        (p, there)
                    })
                    .collect();
                queue.push_back(Event::DebuggersLocated(found));
            }
            Effect::PluginCommand { plugin, command } => {
                let snapshot = self.snapshot();
                queue.extend(self.host.run_command(&plugin, &command, snapshot));
                self.panels_for = None;
            }
            Effect::PluginClick { plugin, button } => {
                let snapshot = self.snapshot();
                queue.extend(self.host.click(&plugin, &button, snapshot));
                self.panels_for = None;
            }
            Effect::PluginSubmit {
                plugin,
                input,
                text,
            } => {
                let snapshot = self.snapshot();
                queue.extend(self.host.submit(&plugin, &input, &text, snapshot));
                self.panels_for = None;
            }
            Effect::PluginPick {
                plugin,
                picker,
                item,
            } => {
                let snapshot = self.snapshot();
                queue.extend(self.host.pick(&plugin, &picker, &item, snapshot));
                self.panels_for = None;
            }
            Effect::PluginReview {
                plugin,
                id,
                accepted,
            } => {
                let snapshot = self.snapshot();
                queue.extend(self.host.review(&plugin, &id, accepted, snapshot));
                self.panels_for = None;
            }
            Effect::SetSetting { key, value } => {
                if !self.scripted
                    && let Err(err) = self.settings.set(&key, &value)
                {
                    queue.push_back(Event::Error(format!("Can't save {key}: {err}")));
                }
            }
            Effect::SearchSymbols {
                plugin,
                command,
                query,
            } => {
                let snapshot = self.snapshot();
                queue.extend(
                    self.host
                        .search_symbols(&plugin, &command, &query, snapshot),
                );
            }
            Effect::SaveMacro(saved) => {
                if !self.scripted
                    && let Err(err) = self.settings.save_macro(&saved)
                {
                    queue.push_back(Event::Error(format!("Can't save the macro: {err}")));
                }
            }
            Effect::PreviewTheme(kind, name) => {
                queue.extend(self.preview(kind, name));
            }
            Effect::SetTheme(kind, name) => queue.extend(self.choose(kind, name)),
            Effect::OpenSettings { workspace } => match self.settings.ensure(workspace) {
                Ok(path) => self.open_later(&path),
                Err(err) => queue.push_back(Event::Error(format!("Can't open settings: {err}"))),
            },
            Effect::Zoom(step) => {
                self.zoom = match step {
                    0 => 0.0,
                    n => (self.zoom + n as f32).clamp(-6.0, 18.0),
                };
            }
            Effect::OpenSettingsWindow { plugin } => self.open_settings(plugin),
        }
    }

    /// A file operation from the explorer, as a job on the disk's lane.
    pub(super) fn file_later(&mut self, op: FileOp) {
        let root = self.root.clone();
        self.jobs.run(Lane::Disk, move || {
            let done = file_op(&root, op);
            Box::new(move |app: &mut App| {
                for event in &done {
                    match event {
                        Event::Renamed { from, .. } => app.disk.retain(|p, _| !p.starts_with(from)),
                        Event::Trashed(path) => app.disk.retain(|p, _| !p.starts_with(path)),
                        Event::Opened { path, .. } => {
                            let when = modified(&app.root.join(path));
                            app.disk.insert(path.clone(), when);
                        }
                        _ => {}
                    }
                }
                app.list_tree_later();
                app.git_status_later();
                done
            })
        });
    }

    /// Runs a git action as a job, on its lane: one that changes the
    /// repository, or a fetch, pull or push.
    pub(super) fn git_later(&mut self, action: GitAction) {
        let commit = matches!(action, GitAction::Commit(_));
        let git = self.git_info();
        self.jobs.run(Lane::Git, move || {
            let mut done = git.run(action);
            if !commit && done.iter().any(|e| matches!(e, Event::GitStatus { .. })) {
                done.push(git.diffs());
            }
            Box::new(move |app: &mut App| {
                if commit {
                    // The new commit is the base for gutter
                    // signs.
                    app.git_index = None;
                    app.check_disk();
                }
                done
            })
        });
    }

    /// Opens the system's file picker for `pick`, in the active file's
    /// folder as VS Code starts, on a thread of its own.
    pub(super) fn pick_file(&mut self, pick: FilePick) {
        // In the active file's folder, as VS Code starts.
        let path = self.editor.buffer().path.clone();
        let folder = path
            .as_ref()
            .and_then(|p| p.parent())
            .map_or(self.root.clone(), |p| self.root.join(p));
        let name = path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        self.jobs.run(Lane::Ask, move || {
            let dialog = rfd::FileDialog::new().set_directory(&folder);
            let path = match &pick {
                FilePick::Open => dialog.set_title("Open File").pick_file(),
                FilePick::OpenFolder => dialog.set_title("Open Folder").pick_folder(),
                FilePick::SaveAs { .. } => {
                    let dialog = dialog.set_title("Save As");
                    match name {
                        Some(name) => dialog.set_file_name(name),
                        None => dialog,
                    }
                    .save_file()
                }
            };
            jobs::events(vec![Event::Picked { pick, path }])
        });
    }
}
