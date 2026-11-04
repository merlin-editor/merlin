//! The command palette and the pickers it shows: files, lines, symbols,
//! themes.

use super::*;

impl Editor {
    pub(crate) fn commands(&self) -> Vec<(String, String)> {
        BUILTIN
            .iter()
            .map(|(id, title)| (id.to_string(), title.to_string()))
            .chain(self.plugins.iter().flat_map(|p| {
                p.commands
                    .iter()
                    .map(|c| (format!("{}.{}", p.name, c.id), c.title.clone()))
            }))
            .collect()
    }

    /// The palette's items that match the query, best first.
    pub(crate) fn palette_items(&self) -> Vec<(String, String)> {
        let Some(palette) = &self.palette else {
            return Vec::new();
        };
        let commands;
        let items = match palette.source {
            Source::Commands => {
                commands = self.commands();
                &commands
            }
            Source::Line => {
                let lines = self.doc().last_line() + 1;
                let item = match palette.query.trim().parse::<usize>() {
                    Ok(n) if (1..=lines).contains(&n) => (n.to_string(), format!("Go to line {n}")),
                    _ => (
                        String::new(),
                        format!("Type a line number from 1 to {lines}"),
                    ),
                };
                return vec![item];
            }
            Source::NewFile { .. } | Source::NewFolder { .. } | Source::Rename { .. } => {
                let name = palette.query.trim();
                let what = match &palette.source {
                    Source::NewFile { dir } if dir.is_empty() => "Create".to_string(),
                    Source::NewFile { dir } | Source::NewFolder { dir } => {
                        format!("Create in {dir}/")
                    }
                    _ => "Rename to".to_string(),
                };
                return vec![if name.is_empty() {
                    (String::new(), "Type a name; a/b makes folders too".into())
                } else {
                    (name.to_string(), format!("{what} {name}"))
                }];
            }
            Source::Trash { .. } => {
                return vec![(
                    "yes".into(),
                    "Enter moves it to the trash; Esc keeps it".into(),
                )];
            }
            Source::SaveAs => {
                let path = palette.query.trim();
                return vec![if path.is_empty() {
                    (
                        String::new(),
                        "Type a path in the project, such as src/new.rs".into(),
                    )
                } else {
                    (path.to_string(), format!("Save as {path}"))
                }];
            }
            _ => &palette.items,
        };
        let query = palette.query.to_lowercase();
        let mut ranked: Vec<(i64, usize)> = items
            .iter()
            .enumerate()
            .filter_map(|(i, (_, label))| Some((fuzzy::score(&query, label)?, i)))
            .collect();
        ranked.sort_by_key(|&(score, i)| (Reverse(score), i));
        ranked
            .into_iter()
            .take(PALETTE_LIMIT)
            .map(|(_, i)| items[i].clone())
            .collect()
    }

    pub(crate) fn palette_key(
        &mut self,
        key: Key,
        armed: Option<&'static str>,
        fx: &mut Vec<Effect>,
    ) {
        let count = self.palette_items().len();
        let Some(palette) = &mut self.palette else {
            self.focus = Focus::Editor;
            return;
        };
        if let Source::Themes { kind, original } = &palette.source {
            let kind = *kind;
            if key.code == KeyCode::Esc {
                let original = original.clone();
                self.close_palette();
                fx.push(Effect::PreviewTheme(kind, original));
                return;
            }
            if key.code != KeyCode::Enter {
                self.palette_step(key);
                if let Some((theme, _)) = self
                    .palette_items()
                    .into_iter()
                    .nth(self.palette.as_ref().map_or(0, |p| p.selected))
                {
                    fx.push(Effect::PreviewTheme(kind, theme));
                }
                return;
            }
        }
        let Some(palette) = &mut self.palette else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.close_palette();
            }
            KeyCode::Up => palette.selected = palette.selected.saturating_sub(1),
            KeyCode::Down => palette.selected = (palette.selected + 1).min(count.saturating_sub(1)),
            KeyCode::Backspace => {
                palette.query.pop();
                palette.selected = 0;
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                palette.query.push(c);
                palette.selected = 0;
            }
            KeyCode::Enter => {
                let selected = palette.selected;
                let item = self.palette_items().into_iter().nth(selected);
                let Some(palette) = self.close_palette() else {
                    return;
                };
                match (item, palette.source) {
                    (Some((id, _)), Source::Commands) => self.run(&id, armed, fx),
                    (Some((path, _)), Source::Files) => self.open(PathBuf::from(path), fx),
                    (Some((line, _)), Source::Symbols) => {
                        if let Ok(line) = line.parse() {
                            self.go_to_block(line);
                        }
                    }
                    (
                        Some((name, _)),
                        source @ (Source::NewFile { .. }
                        | Source::NewFolder { .. }
                        | Source::Rename { .. }
                        | Source::Trash { .. }),
                    ) => self.file_prompt(source, &name, fx),
                    (Some((path, _)), Source::SaveAs) => {
                        if !path.is_empty() {
                            self.doc_mut().buffer.path = Some(PathBuf::from(path));
                            self.run("file.save", None, fx);
                        }
                    }
                    (Some((line, _)), Source::Line) => {
                        if let Ok(n) = line.parse::<u32>() {
                            self.jumps.push((self.doc().id, self.cursor()));
                            self.go_to(n - 1, 0);
                        }
                    }
                    (Some((item, _)), Source::Picker { plugin, id }) => {
                        fx.push(Effect::PluginPick {
                            plugin,
                            picker: id,
                            item,
                        })
                    }
                    (Some((theme, _)), Source::Themes { kind, .. }) => {
                        self.theme[kind as usize] = theme.clone();
                        fx.push(Effect::SetTheme(kind, theme));
                    }
                    (None, _) => {}
                }
            }
            _ => {}
        }
    }

    /// Opens the palette on the colour or icon themes, at the one in use.
    pub(crate) fn pick_theme(&mut self, kind: ThemeKind) {
        let i = kind as usize;
        let items = self.themes[i]
            .iter()
            .map(|t| (t.clone(), t.clone()))
            .collect();
        let title = match kind {
            ThemeKind::Colours => "Color Theme",
            ThemeKind::Icons => "File Icon Theme",
        };
        let original = self.theme[i].clone();
        let mut palette =
            Palette::new(Source::Themes { kind, original }, Some(title.into()), items);
        palette.selected = self.themes[i]
            .iter()
            .position(|t| *t == self.theme[i])
            .unwrap_or(0);
        self.palette = Some(palette);
        self.focus = Focus::Palette;
    }

    /// Moves the palette's selection or edits its query, for a key that
    /// doesn't choose or close.
    pub(crate) fn palette_step(&mut self, key: Key) {
        let count = self.palette_items().len();
        let Some(palette) = &mut self.palette else {
            return;
        };
        match key.code {
            KeyCode::Up => palette.selected = palette.selected.saturating_sub(1),
            KeyCode::Down => palette.selected = (palette.selected + 1).min(count.saturating_sub(1)),
            KeyCode::Backspace => {
                palette.query.pop();
                palette.selected = 0;
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                palette.query.push(c);
                palette.selected = 0;
            }
            _ => {}
        }
    }

    /// Closes the palette, returning focus to the panel if a plugin opened
    /// it as a picker.
    pub(super) fn close_palette(&mut self) -> Option<Palette> {
        let palette = self.palette.take();
        let picker = matches!(
            palette,
            Some(Palette {
                source: Source::Picker { .. },
                ..
            })
        );
        self.focus = if picker && self.panel_visible() {
            Focus::Panel
        } else {
            Focus::Editor
        };
        palette
    }

    /// The palette's left edge, width, and first item shown.
    pub(crate) fn palette_geometry(&self) -> (u16, u16, usize) {
        // Room for the frame on each side.
        let width = self.width.saturating_sub(6).min(80);
        let selected = self.palette.as_ref().map_or(0, |p| p.selected);
        (
            (self.width - width) / 2,
            width,
            selected.saturating_sub(PALETTE_ROWS - 1),
        )
    }
}
