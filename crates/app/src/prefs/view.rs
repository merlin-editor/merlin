//! What the Settings tab shows: each page's sections and rows, read from
//! the settings files, with search, themes, keys and plugins.

use super::*;

/// How many values a table holds, counting inside its tables.
fn leaves(v: &toml::Value) -> usize {
    match v {
        toml::Value::Table(t) => t.values().map(leaves).sum(),
        _ => 1,
    }
}

/// How a setting's value reads in a note.
pub(super) fn shown(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Boolean(true) => "on".into(),
        toml::Value::Boolean(false) => "off".into(),
        other => other.to_string(),
    }
}

impl App {
    /// Every setting the tab shows, by page and heading.
    pub(super) fn sections(&self) -> Vec<Section> {
        let ui = &self.settings_ui;
        let Loaded {
            user, workspace, ..
        } = &self.loaded;
        let scope = if ui.workspace { workspace } else { user };
        let row = |label: &str, about: &str, key: &str, kind: Kind| {
            // The file's own value, or on the Workspace tab yours under it.
            let value = get(scope, key).or_else(|| get(user, key).filter(|_| ui.workspace));
            let in_workspace = get(workspace, key).is_some();
            let control = match kind {
                Kind::Bool(d) => Control::Toggle(value.and_then(|v| v.as_bool()).unwrap_or(d)),
                Kind::Pick(options, d) => Control::Choice(
                    options
                        .iter()
                        .map(|o| (o.to_string(), option_label(o)))
                        .collect(),
                    value.map_or_else(|| d.to_string(), shown),
                ),
                Kind::Num(d) => Control::Number(
                    value
                        .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
                        .map_or(d, |f| f as f32),
                ),
            };
            let note = match (in_workspace, ui.workspace) {
                (true, true) => {
                    let w = get(workspace, key).map(shown).unwrap_or_default();
                    Some(match get(user, key) {
                        Some(u) => format!("Yours is {}; this folder says {w}.", shown(u)),
                        None => format!("This folder says {w}; yours is the default."),
                    })
                }
                (true, false) => {
                    let w = get(workspace, key).map(shown).unwrap_or_default();
                    Some(format!("This folder says {w}, which wins in it."))
                }
                _ => None,
            };
            Row {
                label: label.into(),
                about: about.into(),
                key: key.into(),
                control,
                changed: get(scope, key).is_some(),
                in_workspace,
                note,
                yours: None,
                user_only: false,
                locked: false,
            }
        };
        let on = Kind::Bool;
        let pick = Kind::Pick;
        #[rustfmt::skip]
        let sections = vec![
            Section {
                category: Category::Editor,
                title: "Font".into(),
                rows: vec![
                    row("Font size", "The code's size, in points. Ctrl+= and Ctrl+- change it for now.", "editor.font_size", Kind::Num(FONT_SIZE)),
                    row("Line height", "Each row's height, times the font size.", "editor.line_height", pick(vec!["1.3", "1.55", "1.8"], "1.55")),
                    row("Font ligatures", "Runs like -> and != join into one picture, in fonts that have them.", "editor.font_ligatures", on(true)),
                    row("Interface text size", "Everything around the code grows with it; the code keeps its own size.", "editor.ui_font_size", pick(vec!["11", "12", "13", "14", "15", "16"], "13")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Cursor".into(),
                rows: vec![
                    row("Cursor style", "How the cursor is drawn.", "editor.cursor_style", pick(vec!["line", "line-thin", "block", "block-outline", "underline", "underline-thin"], "line")),
                    row("Cursor blinking", "How the cursor blinks.", "editor.cursor_blinking", pick(vec!["blink", "smooth", "phase", "expand", "solid"], "blink")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Text".into(),
                rows: vec![
                    row("Word wrap", "Long lines wrap at spaces instead of scrolling sideways. Alt+Z", "editor.word_wrap", on(false)),
                    row("Auto-close brackets", "Typing a bracket or quote adds its closer, and wraps a selection in the pair.", "editor.auto_closing_brackets", on(true)),
                    row("Detect indentation", "A file's own indentation wins over the two settings below.", "editor.detect_indentation", on(true)),
                    row("Insert spaces", "Tab and Enter indent with spaces rather than tabs.", "editor.insert_spaces", on(true)),
                    row("Tab size", "Spaces an indent level takes.", "editor.tab_size", pick(vec!["2", "4", "8"], "4")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Display".into(),
                rows: vec![
                    row("Minimap", "The file's shape beside the scrollbar, with matches, problems and changes marked.", "editor.minimap", on(false)),
                    row("Minimap scale", "How big the minimap draws the file.", "editor.minimap_scale", pick(vec!["1", "2", "3"], "1")),
                    row("Breadcrumbs", "The file's folders and the symbols around the cursor, under the tabs.", "editor.breadcrumbs", on(true)),
                    row("Sticky scroll", "The first lines of the blocks in view stay pinned at the top.", "editor.sticky_scroll", on(true)),
                    row("Bracket pair colours", "Brackets are coloured by how deep they are.", "editor.bracket_pair_colorization", on(true)),
                    row("Colour swatches", "A bar in the colour under colours written in the code, such as #e6c07b.", "editor.color_decorators", on(true)),
                    row("Problems inline", "A line's worst error or warning is written after the end of the line.", "editor.inline_diagnostics", on(true)),
                    row("Inlay hints", "Types and parameter names from the language server, in quiet chips.", "editor.inlay_hints", on(true)),
                    row("Glyph icons", "Icons drawn from the code's font: a Nerd Font's, or plain symbols without one.", "editor.icons", pick(vec!["nerd", "plain"], "nerd")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Scrolling".into(),
                rows: vec![
                    row("Smooth scrolling", "A wheel's steps glide to where they scroll.", "editor.smooth_scrolling", on(true)),
                    row("Scroll beyond last line", "The last line can scroll up to the top of the view.", "editor.scroll_beyond_last_line", on(true)),
                    row("Wheel sensitivity", "How far the wheel and touchpad scroll, times their own.", "editor.mouse_wheel_scroll_sensitivity", pick(vec!["0.5", "1", "2", "3"], "1")),
                    row("Fast scroll sensitivity", "Times that again with Alt held.", "editor.fast_scroll_sensitivity", pick(vec!["3", "5", "10"], "5")),
                    row("Scrollbar size", "The scrollbar's width, in points.", "editor.scrollbar_size", pick(vec!["10", "14", "18"], "14")),
                ],
            },
            Section {
                category: Category::Files,
                title: "Saving".into(),
                rows: vec![
                    row("Auto save", "Files save by themselves: a while after each change, when the code loses the keyboard, or when the window does.", "files.auto_save", pick(vec!["off", "after_delay", "on_focus_change", "on_window_change"], "off")),
                    row("Auto save delay", "Milliseconds after the last change, with After delay.", "files.auto_save_delay", pick(vec!["500", "1000", "2000", "5000"], "1000")),
                    row("Format on save", "Saving formats the file first, with its language server. Changed lines formats only what git says you changed.", "editor.format_on_save", pick(vec!["off", "file", "changed_lines"], "off")),
                    row("Trim trailing whitespace", "Saving takes spaces and tabs off the ends of lines. Not in Markdown, where they break lines.", "files.trim_trailing_whitespace", on(false)),
                    row("Insert final newline", "Saving ends the file with a newline.", "files.insert_final_newline", on(false)),
                    row("Hot exit", "Quit without asking; unsaved files come back next time.", "files.hot_exit", on(true)),
                ],
            },
            Section {
                category: Category::Keys,
                title: "Keymap".into(),
                rows: vec![
                    row("Keymap", "How keys work in the text. The rest of the window keeps its shortcuts, and the status bar's Keymap item switches.", "editor.keymap", pick(vec!["vscode", "vim"], "vscode")),
                    row("Leader key", "Starts merlin's own commands, listed by which-key.", "vim.leader", pick(vec!["space", ",", "\\"], "space")),
                    row("Which-key delay", "Milliseconds on a prefix such as g, the leader or Ctrl+K before the next keys show. 0 never shows them.", "vim.which_key_delay", pick(vec!["0", "300", "500", "1000"], "500")),
                    row("Leave Insert mode with", "Two letters typed quickly, as well as Esc.", "vim.insert_escape", pick(vec!["off", "jk", "jj", "kj"], "off")),
                    row("Yank to the clipboard", "y, d and p use the system clipboard, as clipboard=unnamedplus.", "vim.clipboard", on(false)),
                    row("Relative line numbers", "Counted from the cursor's line, in Normal mode.", "editor.line_numbers", pick(vec!["on", "relative"], "on")),
                ],
            },
            Section {
                category: Category::Files,
                title: "Tabs".into(),
                rows: vec![
                    row("Preview tabs", "A single click in the Explorer opens a tab the next click replaces, in italics.", "editor.preview_tabs", on(true)),
                ],
            },
        ];
        sections
    }

    pub(crate) fn settings_view(&self) -> SettingsView {
        let ui = &self.settings_ui;
        let Loaded {
            user,
            workspace,
            errors,
            ..
        } = &self.loaded;
        let scope = if ui.workspace { workspace } else { user };
        let search = ui.search.to_lowercase();
        let searching = !search.is_empty() && !ui.record;

        let all = self.sections();
        let mut changed = [0; 5];
        for s in &all {
            changed[s.category.index()] += s.rows.iter().filter(|r| r.changed).count();
        }
        changed[Category::Appearance.index()] = [
            "theme",
            "theme_light",
            "theme_follows_system",
            "icon_theme",
            "packs",
        ]
        .iter()
        .filter(|k| get(scope, k).is_some())
        .count();
        changed[Category::Keys.index()] += scope
            .get("keys")
            .and_then(|t| t.as_table())
            .map_or(0, |t| t.len());
        changed[Category::Plugins.index()] = scope
            .get("plugins")
            .and_then(|t| t.as_table())
            .map_or(0, |t| t.values().map(leaves).sum());
        // A search finds plugins' settings too, under each plugin.
        let found_plugins = if searching {
            self.plugin_sections()
        } else {
            Vec::new()
        };
        let sections = all
            .into_iter()
            .chain(found_plugins)
            .filter(|s| searching || s.category == ui.category)
            .map(|mut s| {
                if searching {
                    s.rows.retain(|r| {
                        r.label.to_lowercase().contains(&search)
                            || r.about.to_lowercase().contains(&search)
                            || r.key.contains(&search)
                    });
                }
                s
            })
            .filter(|s| !s.rows.is_empty())
            .collect();

        let follow = get(&self.loaded.merged, "theme_follows_system")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let (dark, light) = self.theme_pair();
        let themes = self
            .previews
            .iter()
            .map(|(label, palette)| {
                let mark = if follow {
                    if *label == dark {
                        Some("✓ Dark")
                    } else if *label == light {
                        Some("✓ Light")
                    } else {
                        None
                    }
                } else {
                    (self
                        .choice
                        .colours
                        .as_deref()
                        .unwrap_or(theme::DEFAULT_NAME)
                        == label)
                        .then_some("✓")
                };
                ThemeCard {
                    label: label.clone(),
                    palette: *palette,
                    light: theme::is_light(palette),
                    mark,
                }
            })
            .collect();
        let icon_theme = self
            .choice
            .icons
            .clone()
            .unwrap_or_else(|| packs::LETTERS.into());
        let icons = self
            .icon_previews
            .iter()
            .map(|(label, icons, from)| IconCard {
                label: label.clone(),
                icons: icons.clone(),
                from: from.clone(),
                current: *label == icon_theme,
            })
            .collect();
        let what = |label: &str| {
            self.packs
                .iter()
                .find(|p| p.label == label)
                .map_or_else(|| "not installed".into(), |p| p.parts().describe())
        };
        let layers = self
            .choice
            .packs
            .iter()
            .map(|p| (p.clone(), what(p)))
            .collect();
        let available = self
            .packs
            .iter()
            .filter(|p| p.file.is_some() && !self.choice.packs.contains(&p.label))
            .filter(|p| {
                // Colour and icon themes have their own cards; a pack of
                // only the frame's icons stacks as layouts do.
                let parts = p.parts();
                (parts.layout || parts.fonts || is_ui_only(p)) && !parts.colours
            })
            .map(|p| (p.label.clone(), p.parts().describe()))
            .collect();

        // Keys are many: worked out only where they show.
        let (keys, key_counts) = if ui.category == Category::Keys || searching {
            self.key_rows(&search)
        } else {
            (Vec::new(), [0; 4])
        };
        let capture = ui.capture.as_ref().map(|c| {
            let bindings = self.editor.key_bindings();
            let spec: Vec<String> = c.keys.iter().map(Key::spec).collect();
            let spec = spec.join(" ");
            let clash = bindings
                .iter()
                .find(|b| b.command != c.command && b.keys.iter().any(|(s, _)| *s == spec))
                .filter(|_| !c.keys.is_empty())
                .map(|b| b.title.clone());
            let mine = bindings.iter().find(|b| b.command == c.command);
            CaptureView {
                title: mine.map_or_else(|| c.command.clone(), |b| b.title.clone()),
                keys: c.keys.iter().map(Key::to_string).collect(),
                clash,
                had: mine.is_some_and(|b| !b.keys.is_empty()),
            }
        });

        let plugins: Vec<PluginRow> = if ui.category == Category::Plugins {
            self.host
                .plugins()
                .into_iter()
                .map(|p| PluginRow {
                    mark: plugins::mark(&p),
                    what: super::describe(&p),
                    off_here: self.off_here(&p.name),
                    waiting: p.waiting,
                    memory: p.memory as u64
                        + p.processes
                            .iter()
                            .filter_map(|(_, pid)| self.memory.of(*pid))
                            .sum::<u64>(),
                    name: p.name,
                    title: p.title,
                    on: p.enabled,
                    failed: p.failed,
                    data_only: p.data_only,
                })
                .collect()
        } else {
            Vec::new()
        };
        let page = if ui.category == Category::Plugins {
            let name = ui
                .plugin
                .clone()
                .filter(|n| plugins.iter().any(|p| p.name == *n))
                .or_else(|| plugins.first().map(|p| p.name.clone()));
            name.and_then(|n| self.page_view(&n))
        } else {
            None
        };

        SettingsView {
            ui: SettingsUiView {
                category: ui.category,
                workspace: ui.workspace,
                search: ui.search.clone(),
                keys: ui.keys,
                record: ui.record,
                key_row: ui.key_row,
                open_choice: ui.open_choice.clone(),
                page_tab: ui.page_tab,
                list_tab: ui.list_tab,
                plugin_filter: ui.filter_text().to_string(),
                plugin_menu: ui.plugin_menu.clone(),
                editing: ui.editing.clone(),
                card: ui.card,
                scroll: ui.scroll,
            },
            changed,
            themes,
            follow,
            icons,
            layers,
            available,
            sections,
            keys,
            key_counts,
            capture,
            plugins,
            page,
            file: self.scope_file(),
            folder: self.root.file_name().map_or_else(
                || "this folder".into(),
                |n| n.to_string_lossy().into_owned(),
            ),
            // The file by its name, or the folder's by where it is in it.
            error: errors.first().map(|e| {
                let (user, workspace) = (&self.settings.user, &self.settings.workspace);
                let short = workspace.strip_prefix(&self.root).map_or_else(
                    |_| workspace.display().to_string(),
                    |p| p.display().to_string(),
                );
                e.replace(&workspace.display().to_string(), &short).replace(
                    &user.display().to_string(),
                    &user
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                )
            }),
        }
    }
}
