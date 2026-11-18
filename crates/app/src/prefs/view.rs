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
                Kind::Patterns => {
                    let list = |t: &toml::Table| -> Vec<String> {
                        get(t, key)
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str())
                                    .map(String::from)
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    let mut items: Vec<(String, bool)> =
                        list(scope).into_iter().map(|p| (p, false)).collect();
                    if ui.workspace {
                        for p in list(user) {
                            if !items.iter().any(|(q, _)| *q == p) {
                                items.push((p, true));
                            }
                        }
                    }
                    Control::Patterns(items)
                }
                Kind::Num(d) => Control::Number(
                    value
                        .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
                        .map_or(d, |f| f as f32),
                ),
            };
            let note = match (in_workspace, ui.workspace) {
                // A list's lines say whose each is.
                _ if matches!(control, Control::Patterns(_)) => None,
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
        // Only yours: a folder's settings can't stop merlin looking for
        // updates. On the Workspace tab it shows yours, locked.
        let updates_key = "editor.check_for_updates";
        let mut updates = row(
            "Check for updates",
            "Check for a new version of merlin every 6 hours.",
            updates_key,
            on(true),
        );
        updates.user_only = true;
        if ui.workspace {
            updates.locked = true;
            updates.in_workspace = false;
            updates.note = None;
            updates.changed = get(user, updates_key).is_some();
            updates.control = Control::Toggle(
                get(user, updates_key)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            );
        }
        #[rustfmt::skip]
        let sections = vec![
            Section {
                category: Category::Editor,
                title: "Font".into(),
                rows: vec![
                    row("Font size", "Size of the code, in points. Ctrl+= and Ctrl+- change it temporarily.", "editor.font_size", Kind::Num(FONT_SIZE)),
                    row("Line height", "Height of each line, as a multiple of the font size.", "editor.line_height", pick(vec!["1.3", "1.55", "1.8"], "1.55")),
                    row("Font ligatures", "Show character pairs such as -> and != as single symbols, in fonts that support it.", "editor.font_ligatures", on(true)),
                    row("Interface text size", "Text size of menus, tabs and panels. The code keeps its own size.", "editor.ui_font_size", pick(vec!["11", "12", "13", "14", "15", "16"], "13")),
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
                    row("Word wrap", "Wrap long lines instead of scrolling sideways. Alt+Z", "editor.word_wrap", on(false)),
                    row("Auto-close brackets", "Typing an opening bracket or quote adds the closing one. With text selected, it wraps the selection.", "editor.auto_closing_brackets", on(true)),
                    row("Detect indentation", "Use the indentation a file already has, instead of the two settings below.", "editor.detect_indentation", on(true)),
                    row("Insert spaces", "Indent with spaces instead of tabs.", "editor.insert_spaces", on(true)),
                    row("Tab size", "Number of spaces in one indent level.", "editor.tab_size", pick(vec!["2", "4", "8"], "4")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Display".into(),
                rows: vec![
                    row("Minimap", "A small overview of the file next to the scrollbar, with matches, problems and changes marked.", "editor.minimap", on(false)),
                    row("Minimap scale", "How large the minimap is drawn.", "editor.minimap_scale", pick(vec!["1", "2", "3"], "1")),
                    row("Breadcrumbs", "Show the file's path and the symbols around the cursor below the tabs.", "editor.breadcrumbs", on(true)),
                    row("Sticky scroll", "Keep the first line of each block you're inside pinned to the top while scrolling.", "editor.sticky_scroll", on(true)),
                    row("Bracket pair colours", "Colour brackets by how deeply they're nested.", "editor.bracket_pair_colorization", on(true)),
                    row("Colour swatches", "Underline colour values in the code, such as #e6c07b, in the colour they describe.", "editor.color_decorators", on(true)),
                    row("Problems inline", "Show the most serious error or warning at the end of its line.", "editor.inline_diagnostics", on(true)),
                    row("Inlay hints", "Show types and parameter names from the language server inside the code.", "editor.inlay_hints", on(true)),
                    row("Glyph icons", "Draw icons with a Nerd Font, or use plain symbols if you don't have one installed.", "editor.icons", pick(vec!["nerd", "plain"], "nerd")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Scrolling".into(),
                rows: vec![
                    row("Smooth scrolling", "Animate scrolling with the mouse wheel.", "editor.smooth_scrolling", on(true)),
                    row("Scroll beyond last line", "Let the last line scroll up to the top of the editor.", "editor.scroll_beyond_last_line", on(true)),
                    row("Wheel sensitivity", "How far one step of the mouse wheel or touchpad scrolls.", "editor.mouse_wheel_scroll_sensitivity", pick(vec!["0.5", "1", "2", "3"], "1")),
                    row("Fast scroll sensitivity", "How much faster scrolling is while holding Alt.", "editor.fast_scroll_sensitivity", pick(vec!["3", "5", "10"], "5")),
                    row("Scrollbar size", "Width of the scrollbar, in points.", "editor.scrollbar_size", pick(vec!["10", "14", "18"], "14")),
                ],
            },
            Section {
                category: Category::Editor,
                title: "Updates".into(),
                rows: vec![updates],
            },
            Section {
                category: Category::Files,
                title: "Exclude".into(),
                rows: vec![
                    row("Exclude", "Left out of the Explorer, Quick Open, Search, symbols, Check Project and plugins' file lists. A folder ends in /. A pattern with no other / matches at any depth, as *.o or third_party/; with one, from the top, as build/_deps/. On the Workspace tab, these add to yours.", "files.exclude", Kind::Patterns),
                    row("Exclude from search", "Still in the Explorer, but left out of Quick Open, Search and symbols.", "search.exclude", Kind::Patterns),
                    row("Use .gitignore", "Files git ignores are excluded too. Turn it off to see and search build/.", "files.use_gitignore", on(true)),
                    row("Show hidden files", "Names starting with a dot, such as .github/ or .clang-format, in the Explorer and Quick Open.", "files.show_hidden", on(false)),
                ],
            },
            Section {
                category: Category::Files,
                title: "Saving".into(),
                rows: vec![
                    row("Auto save", "Save files automatically: after a delay, when you leave the editor, or when you leave the window.", "files.auto_save", pick(vec!["off", "after_delay", "on_focus_change", "on_window_change"], "off")),
                    row("Auto save delay", "How long to wait after the last change before saving, in milliseconds. Used with After delay.", "files.auto_save_delay", pick(vec!["500", "1000", "2000", "5000"], "1000")),
                    row("Format on save", "Format the file with its language server when saving. Changed lines only formats lines git shows as changed.", "editor.format_on_save", pick(vec!["off", "file", "changed_lines"], "off")),
                    row("Trim trailing whitespace", "Remove spaces and tabs at the end of lines when saving. Markdown files are skipped, where trailing spaces mean a line break.", "files.trim_trailing_whitespace", on(false)),
                    row("Insert final newline", "Make sure the file ends with a newline when saving.", "files.insert_final_newline", on(false)),
                    row("Hot exit", "Quit without asking to save. Unsaved files reopen next time.", "files.hot_exit", on(true)),
                    row("Open large files read-only", "Files larger than this open read-only, read from disk a part at a time instead of loaded: scrolling, Find, Go to Line and copying work, and editing asks to load the file first. Auto is a quarter of the memory free when the file opens.", "files.large_file_read_only", pick(vec!["auto", "1 GB", "2 GB", "4 GB", "never"], "auto")),
                ],
            },
            Section {
                category: Category::Keys,
                title: "Keymap".into(),
                rows: vec![
                    row("Keymap", "How keys work while editing. Other shortcuts stay the same. You can also switch from the Keymap item in the status bar.", "editor.keymap", pick(vec!["vscode", "vim"], "vscode")),
                    row("Leader key", "The key that starts merlin's own commands in Vim mode. Which-key lists them.", "vim.leader", pick(vec!["space", ",", "\\"], "space")),
                    row("Which-key delay", "How long to wait after a prefix such as g, the leader or Ctrl+K before showing the keys that can follow, in milliseconds. 0 turns it off.", "vim.which_key_delay", pick(vec!["0", "300", "500", "1000"], "500")),
                    row("Leave Insert mode with", "Two letters typed quickly that work like Esc.", "vim.insert_escape", pick(vec!["off", "jk", "jj", "kj"], "off")),
                    row("Yank to the clipboard", "Make y, d and p use the system clipboard, like clipboard=unnamedplus in Vim.", "vim.clipboard", on(false)),
                    row("Relative line numbers", "Number lines from the cursor in Normal mode.", "editor.line_numbers", pick(vec!["on", "relative"], "on")),
                ],
            },
            Section {
                category: Category::Files,
                title: "Tabs".into(),
                rows: vec![
                    row("Preview tabs", "A single click in the Explorer opens a temporary tab, shown in italics, that the next single click replaces.", "editor.preview_tabs", on(true)),
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

        let updates = self.browse.updates(&|_| None, &self.loaded_requirements());
        let blocked = self.browse.blocked();
        let plugins: Vec<PluginRow> = if ui.category == Category::Plugins {
            self.host
                .plugins()
                .into_iter()
                .map(|p| PluginRow {
                    mark: plugins::mark(&p),
                    what: super::describe(&p),
                    off_here: self.off_here(&p.name),
                    linked: self.install_link(&p).is_some(),
                    blocked: blocked.iter().any(|(n, _)| *n == p.name),
                    update: updates
                        .iter()
                        .find(|u| u.name == p.name)
                        .map(|u| u.withdrawn.is_some()),
                    waiting: p.waiting,
                    memory: p.memory as u64
                        + p.processes
                            .iter()
                            .filter_map(|(_, pid)| self.memory.of(*pid))
                            .sum::<u64>(),
                    updated: self
                        .browse
                        .updated
                        .iter()
                        .find(|(n, ..)| *n == p.name)
                        .map(|(_, from, to)| format!("{from} → {to}")),
                    kept: updates.iter().any(|u| u.name == p.name && u.kept),
                    replaced: self
                        .browse
                        .registry_page(&p.name, None)
                        .is_some_and(|r| r.replaced.is_some()),
                    not_for_this: false,
                    name: p.name,
                    title: p.title,
                    on: p.enabled,
                    failed: p.failed,
                    data_only: p.data_only,
                })
                .chain(self.browse.waiting_plugins(&self.host.plugins()))
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
                version_menu: ui.version_menu.clone(),
                auto_update: self.auto_update(),
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
            browse: if ui.category == Category::Plugins {
                self.browse_view()
            } else {
                Default::default()
            },
            updates: updates.len(),
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
