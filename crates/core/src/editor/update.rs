//! What the editor does with each event the app sends: keys, the mouse,
//! files read and written, plugins' answers, and the rest, each turned into
//! changes to its state and the effects it asks for.

use std::path::Path;

use super::*;

impl Editor {
    pub fn update(&mut self, event: Event) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.clicks = 0;
        // Only something the user does cancels a pending confirmation.
        let by_user = matches!(
            event,
            Event::Key(_) | Event::Paste(_) | Event::Command(_) | Event::Ui(_)
        ) || matches!(event, Event::Pointer(p) if p.kind == MouseKind::Down);
        // A dialog takes the keys, and the mouse is for its buttons.
        if self.dialog.is_some() {
            match event {
                Event::Key(key) => {
                    self.dialog_key(key, &mut fx);
                    return fx;
                }
                Event::Ui(UiEvent::DialogButton(i)) => {
                    self.answer(i, &mut fx);
                    return fx;
                }
                Event::Pointer(_) | Event::Paste(_) | Event::Ui(_) | Event::Command(_) => {
                    return fx;
                }
                _ => {}
            }
        }
        if by_user {
            // A notice's button needs the notice to run.
            if !matches!(event, Event::Ui(UiEvent::ToastButton(_))) {
                self.message = None;
            }
        }
        let before = self.view_key();
        let was = (self.focus, self.doc().id);
        match event {
            Event::Key(key) => {
                self.probe = None;
                self.hover_anchor = None;
                self.link = None;
                self.macro_key(key, &mut fx)
            }
            Event::Paste(text) => {
                if !self.rename_paste(&text) && !self.terminal_paste(&text, &mut fx) {
                    self.paste(&text);
                }
            }
            Event::Ui(ui) => self.ui(ui, &mut fx),
            // Every group as big, until the window says otherwise.
            Event::Resize(w, h) => {
                self.width = w;
                self.height = h;
                self.viewports.clear();
            }
            Event::Viewports(sizes) => self.viewports = sizes,
            Event::Pointer(p) => self.pointer(p, &mut fx),
            Event::Scroll { group, rows, cols } => self.scroll_group(group, rows, cols),
            Event::ScrollTo { group, row } => self.in_group(group, |e| e.set_top_row(row)),
            Event::Command(id) => self.run(&id, &mut fx),
            Event::Saved {
                doc,
                revision,
                path,
            } => {
                if let Some(d) = self.docs.iter_mut().find(|d| d.id == doc) {
                    d.buffer.mark_saved(revision);
                    d.buffer.path = Some(path.clone());
                }
                // A save says nothing: the tab's dot going is enough.
                if self.git.branch.is_some() {
                    fx.push(Effect::GitStatus);
                }
                self.after_save(doc, &mut fx);
            }
            Event::Picked { pick, path } => self.picked(pick, path, &mut fx),
            Event::Preedit { text, cursor } => {
                self.preedit = (!text.is_empty()).then_some((text, cursor));
            }
            Event::SaveFailed(err) => self.error(format!("Save failed: {err}")),
            Event::Opened { path, text } => {
                self.opened(path.clone(), &text);
                self.split_opened(&path);
                self.opened_for_edits(&path);
                self.start_merge();
                self.tasks_opened(&path);
                self.launch_opened(&path);
            }
            Event::FileConfig { path, config } => self.file_config(&path, config),
            Event::AutoSave(why) => self.auto_save(why, &mut fx),
            Event::TermTitle { id, title } => self.terminal_title(id, title),
            Event::TaskOutput { id, text } => self.task_output(id, &text),
            Event::TermExited { id, code } => {
                self.terminal_exited(id, code);
                self.task_exited(id, code, &mut fx);
                self.debug_build_exited(id, code, &mut fx);
            }
            Event::Restore(ws) => self.restore(ws),
            Event::RestoreBreakpoints(ws) => {
                self.debug.watches = ws.watches;
                self.restore_view_layout(ws.view_places, ws.hidden_views);
                self.restore_breakpoints(ws.breakpoints, ws.debug_target);
            }
            Event::Settings(settings) => self.apply_settings(settings),
            Event::TextColours {
                language,
                text,
                tokens,
            } => self.text_coloured(language, text, tokens),
            Event::Files(files) if self.search_open() => self.search_files_came(files),
            Event::Files(files) => self.files_came(files),
            Event::Clipboard(text) => self.clipboard_came(text, &mut fx),
            Event::Error(text) => self.error(text),
            Event::RegisterPlugin { plugin, commands } => self.register(plugin, commands),
            Event::PluginFailed { plugin, message } => self.plugin_failed(plugin, message),
            Event::SetPanel { plugin, panel } => self.set_panel(&plugin, panel),
            Event::RegisterViews { plugin, views } => self.register_views(&plugin, views),
            Event::RegisterMenus { plugin, items } => self.register_menus(&plugin, items),
            Event::ProgressStart {
                plugin,
                id,
                title,
                cancellable,
            } => self.progress_start(plugin, id, title, cancellable),
            Event::ProgressReport {
                plugin,
                id,
                message,
                done,
            } => self.progress_report(&plugin, &id, message, done),
            Event::ProgressEnd { plugin, id } => self.progress_end(&plugin, id.as_deref()),
            Event::ViewBadge {
                plugin,
                view,
                count,
            } => self.set_view_badge(&plugin, &view, count),
            Event::FocusView { plugin, view } => {
                self.show_plugin_view(&super::view_key(&plugin, &view))
            }
            Event::FocusPanel(plugin) => {
                self.bar_mut(Side::Right).active = Some(plugin);
                self.focus_panel(Side::Right);
            }
            Event::ShowPicker {
                plugin,
                id,
                title,
                items,
            } => {
                let items = items.into_iter().map(|i| (i.id, i.label)).collect();
                self.palette = Some(Palette::new(
                    Source::Picker { plugin, id },
                    Some(title),
                    items,
                ));
                self.focus = Focus::Palette;
            }
            Event::InsertText(text) => self.type_text(&text, Kind::Other),
            Event::Notify(text) => self.notify(text),
            Event::PluginLog {
                plugin,
                level,
                text,
            } => self.log_line(&plugin, level, &text),
            Event::PluginNotice {
                plugin,
                level,
                text,
                buttons,
            } => self.plugin_notice(&plugin, level, text, buttons),
            Event::ShowOutput(channel) => self.show_output(&channel),
            Event::SetDiagnostics {
                plugin,
                path,
                diagnostics,
            } => {
                self.diagnostics.insert((plugin, path.clone()), diagnostics);
                self.diagnostics_version += 1;
                self.place_marks(&path);
            }
            Event::RegisterLanguages { plugin, languages } => {
                self.languages.retain(|(p, _)| *p != plugin);
                self.languages
                    .extend(languages.into_iter().map(|l| (plugin.clone(), l)));
            }
            Event::RegisterProblemMatchers { plugin, matchers } => {
                self.problem_matchers.retain(|(p, _)| *p != plugin);
                self.problem_matchers
                    .extend(matchers.into_iter().map(|m| (plugin.clone(), m)));
            }
            Event::SetTokens {
                path,
                layer,
                lines,
                tokens,
            } => self.tokens_came(&path, layer, lines, tokens),
            Event::SetSymbols { path, symbols } => self.symbols_came(&path, symbols),
            Event::SetStatus { plugin, set, lines } => self.status_came(plugin, set, lines),
            Event::SetTasks { plugin, set, tasks } => self.tasks_came(plugin, set, tasks),
            Event::SetTests {
                plugin,
                path,
                group,
                label,
                tests,
            } => self.set_plugin_tests(plugin, &path, group, label, tests),
            Event::SetTestResults { plugin, results } => self.set_plugin_results(&plugin, results),
            Event::SetTestRun {
                plugin,
                group,
                running,
                seconds,
            } => self.set_test_run(&plugin, &group, running, seconds),
            Event::RunInTerminal {
                plugin,
                id,
                name,
                command,
            } => {
                self.run_for_plugin(plugin, id, &name, &command, &mut fx);
            }
            Event::StartDebugging {
                plugin,
                target,
                fallback,
            } => self.debug_for_plugin(plugin, *target, fallback, &mut fx),
            Event::RunTask(name) => self.run_task_named(&name, &mut fx),
            Event::RegisterKeys { plugin, keys } => self.register_keys(plugin, keys),
            Event::WhichKey(tick) => self.which_key_due(tick),
            Event::MacroStep => self.macro_step(&mut fx),
            Event::PaletteSearch(tick) => self.palette_search_due(tick, &mut fx),
            Event::LinkDue(tick) => self.link_due(tick, &mut fx),
            Event::RegisterSymbolSearch { plugin, command } => {
                self.symbol_search.retain(|(p, _)| *p != plugin);
                self.symbol_search.push((plugin, command));
            }
            Event::ProjectSymbols { query, symbols } => self.project_symbols_came(&query, symbols),
            Event::RegisterDebuggers { plugin, debuggers } => {
                self.register_debuggers(plugin, debuggers, &mut fx)
            }
            Event::DebuggersLocated(found) => self.debug.installed.extend(found),
            Event::SetDebugTargets { plugin, targets } => self.set_debug_targets(plugin, targets),
            Event::LaunchConfigs(configs) => self.launch_configs(configs, &mut fx),
            Event::Processes { list, ptrace_scope } => self.pick_process(list, ptrace_scope),
            Event::Debug { plugin, update } => self.debug_update(&plugin, update, &mut fx),
            Event::SetFolds { path, ranges } => self.folds_came(&path, ranges),
            Event::SetInlayHints { path, hints } => self.inlay_hints_came(&path, hints),
            Event::ShowHover(text) => {
                let lines: Vec<String> = text.trim().lines().map(String::from).collect();
                if self.focus == Focus::Editor && !lines.is_empty() {
                    self.show_hover(lines);
                }
            }
            Event::ShowCodeActions(actions) => self.show_code_actions(actions),
            Event::FormatTimedOut => self.format_timed_out(&mut fx),
            Event::ApplyEdits(edits) => {
                self.rename_arrived(&edits);
                self.apply_file_edits(edits, &mut fx);
            }
            Event::ShowRename(info) => self.show_rename(info),
            Event::ShowRenameReview(edits) => self.show_rename_review(edits, &mut fx),
            Event::ShowReferences(places) => self.show_references(places, &mut fx),
            Event::Definitions(places) => self.definitions_came(places, &mut fx),
            Event::FilesRead(files) => {
                self.tasks_read(&files, &mut fx);
                self.compare_files_read(&files, &mut fx);
                self.peek_files_read(&files);
                self.link_files_read(&files);
                self.files_read(files);
            }
            Event::ShowSignature(signature) => self.signature_came(signature),
            Event::ShowCompletions(items) => self.completions_came(items),
            Event::OpenAt { path, line, column } => self.open_at(path, line, column, &mut fx),
            Event::ShowReview {
                plugin,
                id,
                path,
                old,
                new,
            } => self.show_review(plugin, id, path, &old, &new),
            Event::CloseReview { plugin, id } => self.close_review(&plugin, &id),
            Event::FileChanged { path, text } => self.file_changed(&path, &text),
            Event::Recovered { path, text, moved } => self.recovered(path, &text, moved),
            Event::RecoveredAll { crashed } => self.recovered_all(crashed),
            Event::GitBase { path, text } => {
                for doc in &mut self.docs {
                    if doc.buffer.path.as_ref() == Some(&path) {
                        doc.set_git_base(text.clone());
                    }
                }
            }
            Event::PluginList(list) => {
                self.plugin_list = list;
            }
            Event::Themes {
                kind,
                themes,
                current,
            } => {
                let i = kind as usize;
                self.themes[i] = themes;
                self.theme[i] = current;
            }
            Event::Renamed { from, to } => self.renamed(from, to),
            Event::Trashed(path) => self.trashed(path),
            Event::Tree(files) => {
                let tree = self.tree.get_or_insert_with(Default::default);
                tree.files = files;
                tree.built_for = None;
            }
            Event::SearchResults { query, hits } => {
                self.palette_search_came(&query, &hits);
                if query == self.search.query {
                    self.search.hits = hits;
                    self.search.searching = false;
                }
            }
            Event::GitStatus {
                branch,
                files,
                staged,
                changed,
            } => {
                self.git.branch = branch;
                self.git.files = files;
                self.git.staged = staged;
                self.git.changed = changed;
                self.git.version += 1;
            }
            Event::GitDiffs { staged, changed } => self.git_diffs_came(staged, changed, &mut fx),
            Event::Discarded { path, before, all } => self.discarded(path, before, all),
            Event::Blame {
                path,
                version,
                blame,
                now,
            } => {
                self.now = now;
                self.blames.insert(path, (version, blame));
            }
            Event::CommitInfo { hash, info } => {
                self.commit_info.insert(hash, info);
            }
            Event::GitMerge(merge) => self.merge_state(merge),
            Event::GitMoves(moves) => {
                self.git.moves = moves;
                self.git.version += 1;
            }
            Event::ShowBinary { path, old, new } => self.show_binary(path, old, new),
            Event::MergeSides {
                path,
                base,
                incoming,
                current,
            } => self.merge_sides(path, base, (incoming, current)),
            Event::GitSync(state) => {
                self.git.sync = state;
                self.git.version += 1;
            }
            Event::Synced {
                kind,
                outcome,
                output,
            } => self.synced(kind, outcome, output),
            Event::ShowDiff { path, old, new } => self.diff_came(path, &old, &new, &mut fx),
        }
        self.sync_docs();
        self.after_event(before, was, &mut fx);
        fx
    }

    /// Each document's edits since the last event, followed by Find in
    /// Selection's range and by the other groups showing it.
    fn sync_docs(&mut self) {
        let shown = self.docs[self.current].id;
        for doc in &mut self.docs {
            let changes = doc.sync();
            // Find in Selection's range moves with edits.
            if doc.id == shown
                && let Some((a, b)) = self.find.as_mut().and_then(|f| f.scope.as_mut())
            {
                for change in &changes {
                    *a = change.map(*a);
                    *b = change.map(*b);
                }
            }
            // Other groups showing the document keep their place in it.
            for group in &mut self.groups {
                if let Some(view) = group.saved.get_mut(&doc.id) {
                    view.follow(&changes);
                    // A fold whose first line was joined to another goes.
                    let rope = doc.buffer.rope();
                    view.folds.retain(|&f| f == 0 || rope.char(f - 1) == '\n');
                }
            }
        }
    }

    /// What follows any event: the views rebuilt, the cursor followed if
    /// something moved it, and the focus's and selection's effects.
    fn after_event(
        &mut self,
        before: (u64, usize, u64, u16, u16, bool, bool),
        was: (Focus, u64),
        fx: &mut Vec<Effect>,
    ) {
        self.follow_change_peek();
        self.follow_merges();
        self.ask_blame(fx);
        self.scroll_out_of_folds();
        // An edited preview stays open.
        if let Some(id) = self.preview
            && self
                .index_of(id)
                .is_none_or(|i| self.docs[i].buffer.is_dirty())
        {
            self.preview = None;
        }
        self.sync_groups();
        self.build_views();
        self.check_completion();
        self.check_signature();
        // Follow the cursor only when something moved it, so a view scrolled
        // with the wheel stays put while plugins update their panels.
        if self.view_key() != before {
            // Moving into a folded block opens it.
            let line = self.doc().line_col(self.doc().cursor).0;
            if self.is_hidden(line) {
                self.reveal_cursor();
            }
            self.scroll_to_cursor();
        }
        self.share_selection(fx);
        self.save_on_focus_change(was, fx);
        self.note_recent();
        for side in [Side::Left, Side::Right, Side::Bottom] {
            self.select(side, self.selected_index(side));
        }
    }

    /// The project's files, for Open: the palette on them.
    fn files_came(&mut self, files: Vec<String>) {
        if files.is_empty() {
            self.notify("No files found".into());
        } else {
            let items = files.into_iter().map(|f| (f.clone(), f)).collect();
            self.palette = Some(Palette::new(Source::Files, Some("Open".into()), items));
            self.focus = Focus::Palette;
        }
    }

    /// The clipboard's text, read for a paste: to whatever takes it, else the
    /// text. `None` if it couldn't be read; merlin's own last copy then.
    fn clipboard_came(&mut self, text: Option<String>, fx: &mut Vec<Effect>) {
        let text = text.unwrap_or_else(|| self.clipboard.clone());
        if self.vim_clipboard_put(&text)
            || self.compare_clipboard(&text, fx)
            || text.is_empty()
            || self.rename_paste(&text)
            || self.terminal_paste(&text, fx)
        {
        } else if self.focus == Focus::Editor && (self.in_changes_tab() || self.in_compare()) {
            self.through_file(fx, |e, _| e.paste(&text));
        } else {
            self.paste(&text);
        }
    }

    /// A plugin stopped: its commands and keys go, and its panel says why.
    fn plugin_failed(&mut self, plugin: String, message: String) {
        let entry = self.plugin_mut(&plugin);
        entry.commands.clear();
        entry.keys.clear();
        // Its panel says what went wrong, until it loads again.
        let title = entry
            .panel
            .as_ref()
            .map_or(plugin.clone(), |p| p.title.clone());
        entry.panel = Some(Panel {
            title,
            nodes: vec![
                crate::panel::Node::Styled(format!("{plugin} failed"), crate::panel::Tone::Error),
                crate::panel::Node::Text(message.clone()),
            ],
        });
        entry.error = Some(message.clone());
        self.log_line(
            &plugin,
            super::LogLevel::Error,
            &format!("Failed: {message}"),
        );
        self.error(format!("Plugin {plugin} failed: {message}"));
        self.add_action("Show Output", &format!("output.show:{plugin}"));
    }

    /// A plugin's tokens for a file's `layer`, on `lines` or all of it.
    fn tokens_came(
        &mut self,
        path: &Path,
        layer: Layer,
        lines: Option<(u32, u32)>,
        tokens: Vec<PluginToken>,
    ) {
        if let Some(doc) = self.doc_at(path) {
            let place = |(l, c): (u32, u32)| doc.pos_at_utf16(l, c);
            let tokens = tokens
                .into_iter()
                .map(|t| Token {
                    start: place(t.start),
                    end: place(t.end),
                    kind: t.kind,
                })
                .filter(|t| t.start < t.end)
                .collect();
            let within = lines.map(|(a, b)| place((a, 0))..place((b, 0)));
            doc.layers.set(layer, within, tokens);
        }
    }

    /// A file's symbols, for the outline and breadcrumbs.
    fn symbols_came(&mut self, path: &Path, symbols: Vec<PluginSymbol>) {
        if let Some(doc) = self.doc_at(path) {
            let place = |(l, c): (u32, u32)| doc.pos_at_utf16(l, c);
            let symbols = symbols
                .into_iter()
                .map(|s| DocSymbol {
                    start: place(s.start),
                    end: place(s.end),
                    name: s.name,
                    kind: s.kind,
                    detail: s.detail,
                    depth: s.depth as usize,
                })
                .collect();
            doc.layers.symbols = Some(symbols);
            self.outline_for = None;
        }
    }

    /// The tasks a plugin offers, in place of what it offered before.
    fn tasks_came(&mut self, plugin: String, set: String, mut tasks: Vec<Task>) {
        for task in &mut tasks {
            task.plugin = Some(plugin.clone());
        }
        let key = (plugin, set);
        self.tasks.plugins.retain(|(k, _)| *k != key);
        if !tasks.is_empty() {
            self.tasks.plugins.push((key, tasks));
        }
        self.build_views();
    }

    /// Where a file's blocks fold, by line.
    fn folds_came(&mut self, path: &Path, ranges: Vec<(u32, u32)>) {
        if let Some(doc) = self.doc_at(path) {
            let folds = ranges
                .into_iter()
                .filter(|(a, b)| b > a)
                .map(|(a, b)| (doc.pos_at_utf16(a, 0), doc.pos_at_utf16(b, 0)))
                .collect();
            doc.layers.folds = Some(folds);
        }
    }

    /// A file's inlay hints, unless they're turned off.
    fn inlay_hints_came(&mut self, path: &Path, hints: Vec<crate::lang::InlayHint>) {
        let on = self.inlay_hints;
        if let Some(doc) = self.doc_at(path) {
            let mut inlays: Vec<Inlay> = hints
                .into_iter()
                .filter(|_| on)
                .map(|h| Inlay {
                    pos: doc.pos_at_utf16(h.at.0, h.at.1),
                    cols: crate::style::str_width(&h.label) + 1,
                    label: h.label,
                    kind: h.kind,
                })
                .collect();
            inlays.sort_by_key(|i| i.pos);
            doc.inlays = inlays;
        }
    }

    /// The signature of the call the cursor is in, kept where it was first
    /// shown while it's the same document's.
    fn signature_came(&mut self, signature: Option<crate::lang::Signature>) {
        let doc = self.doc().id;
        self.signature = match signature {
            Some(s) if self.focus == Focus::Editor => {
                let at = match &self.signature {
                    Some((d, at, _)) if *d == doc => *at,
                    _ => self.cursor(),
                };
                Some((doc, at, s))
            }
            _ => None,
        };
    }

    /// Completions for the word before the cursor.
    fn completions_came(&mut self, items: Vec<Completion>) {
        if items.is_empty() {
            self.notify("No completions".into());
        } else if self.focus == Focus::Editor {
            let start = self.word_start(self.cursor());
            self.popup = Some(Popup::Completion {
                items,
                selected: 0,
                start,
            });
        }
    }

    /// What's staged and changed, file by file; open changes tabs follow.
    fn git_diffs_came(
        &mut self,
        staged: Vec<crate::git::FileDiff>,
        changed: Vec<crate::git::FileDiff>,
        fx: &mut Vec<Effect>,
    ) {
        let by_path = |list: Vec<crate::git::FileDiff>| {
            list.into_iter().map(|d| (d.path.clone(), d)).collect()
        };
        self.git.staged_diffs = by_path(staged);
        self.git.changed_diffs = by_path(changed);
        self.git.version += 1;
        // Open changes tabs follow what changed on disk and in the
        // index.
        for doc in &self.docs {
            if let Some(r) = doc.review.as_ref().filter(|r| r.plugin.is_empty()) {
                self.diff_refresh.insert(r.path.clone());
                fx.push(Effect::OpenDiff(r.path.clone()));
            }
        }
        self.mark_staged();
    }

    /// A file's changes since the index or the last commit, for its changes
    /// tab.
    fn diff_came(&mut self, path: PathBuf, old: &str, new: &str, fx: &mut Vec<Effect>) {
        let id = path.display().to_string();
        let refresh = self.diff_refresh.contains(&path);
        // An open file's text, unsaved edits and all, is what the
        // tab shows and edits.
        let new = self
            .docs
            .iter()
            .find(|d| d.buffer.path.as_ref() == Some(&path))
            .map_or(new.to_string(), |d| d.buffer.text());
        self.show_review(String::new(), id, path.clone(), old, &new);
        self.mark_staged();
        if !refresh {
            self.open_beside(path, fx);
        }
    }

    /// A new selection in the text goes to the selection clipboard, for a
    /// middle click to paste anywhere.
    pub(super) fn share_selection(&mut self, fx: &mut Vec<Effect>) {
        if self.focus != Focus::Editor {
            return;
        }
        let doc = self.doc();
        let now = doc
            .selection()
            .map(|(a, b)| (doc.id, doc.buffer.version(), a, b));
        if now != self.selection_shared {
            self.selection_shared = now;
            if let Some(text) = self.selection().filter(|t| !t.is_empty()) {
                fx.push(Effect::SetSelection(text));
            }
        }
    }
}
