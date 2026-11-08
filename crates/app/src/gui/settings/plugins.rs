//! The Plugins page: each plugin as a row, and a plugin's own page with
//! its permissions, settings and what it adds.

use super::*;
use crate::prefs::{ListTab, PLUGIN_FILTER, PageTab, PluginRow, SettingsUiView};

impl Frame<'_> {
    pub(super) fn plugins_page(&mut self, v: &SettingsView, c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        c.y += 12.0;
        // Narrow, the list goes over the page rather than beside it.
        let stacked = c.w < 640.0;
        let list_w = if stacked {
            c.w
        } else {
            260.0_f32.min(c.w * 0.4)
        };
        let top = c.y;
        let ui = &v.ui;
        // Installed, and Browse for more.
        let tabs = [
            ("Installed", Some(v.plugins.len()), ListTab::Installed),
            ("Browse", None, ListTab::Browse),
        ];
        let bar = Rect::new(c.x, top, list_w, 30.0);
        self.s_fill(
            Rect::new(bar.x, bar.bottom() - 1.0, bar.w, 1.0),
            clip,
            p.line,
        );
        let mut x = bar.x + 4.0;
        for (label, n, tab) in tabs {
            let f = Face::ui(12.5);
            let count = Face::ui(11.0);
            let lw = self.text.width(label, f);
            let n = n.map(|n| n.to_string()).unwrap_or_default();
            let w = lw
                + if n.is_empty() {
                    0.0
                } else {
                    5.0 + self.text.width(&n, count)
                };
            let r = Rect::new(x, bar.y, w, bar.h);
            let on = ui.list_tab == tab;
            let fg = if on || self.s_hovered(r, clip) {
                p.bright
            } else {
                p.muted
            };
            self.s_row_text(label, f, x, bar.y, bar.h - 8.0, fg, clip);
            self.s_row_text(&n, count, x + lw + 5.0, bar.y, bar.h - 8.0, p.faint, clip);
            if on {
                self.s_fill(Rect::new(x, bar.bottom() - 2.0, w, 2.0), clip, p.accent);
            }
            self.s_hit(r, clip, Hit::Settings(SettingsAction::ListTab(tab)));
            x += w + 20.0;
        }
        let mut y = bar.bottom() + 10.0;
        if ui.list_tab == ListTab::Browse {
            y = self.plugins_browse(c.x, y, list_w, clip);
        } else {
            // The filter, then the plugins it lets through.
            let typing = ui
                .editing
                .as_ref()
                .filter(|e| e.key == PLUGIN_FILTER)
                .map(|e| e.text.as_str());
            let field = Rect::new(c.x, y, list_w, 30.0);
            self.filter_field(&ui.plugin_filter, typing, field, clip);
            y += 38.0;
            let filter = ui.plugin_filter.trim().to_lowercase();
            let shown: Vec<&PluginRow> = v
                .plugins
                .iter()
                .filter(|r| {
                    filter.is_empty()
                        || [&r.title, &r.name, &r.what]
                            .iter()
                            .any(|t| t.to_lowercase().contains(&filter))
                })
                .collect();
            if shown.is_empty() {
                let what = if v.plugins.is_empty() {
                    "No plugins are installed."
                } else {
                    "No plugins match."
                };
                self.s_row_text(what, Face::ui(12.5), c.x + 4.0, y, 24.0, p.muted, clip);
                y += 28.0;
            }
            let picked = v.page.as_ref().map(|pg| pg.name.as_str());
            for row in shown {
                y = self.plugin_row(row, picked, ui, Rect::new(c.x, y, list_w, 46.0), c);
            }
        }
        let list_bottom = y;
        let (dx, dy) = if stacked {
            self.s_fill(Rect::new(c.x, list_bottom + 8.0, c.w, 1.0), clip, p.line);
            (c.x, list_bottom + 24.0)
        } else {
            let dx = c.x + list_w + 24.0;
            self.s_fill(
                Rect::new(dx - 12.0, top, 1.0, (list_bottom - top).max(200.0)),
                clip,
                p.line,
            );
            (dx, top)
        };
        let mut d = Ctx {
            x: dx,
            w: c.x + c.w - dx,
            y: dy,
            clip,
            list: None,
            picked: None,
        };
        if let Some(page) = &v.page {
            self.plugin_detail(page, &v.ui, &mut d);
        }
        if d.list.is_some() {
            c.list = d.list.take();
        }
        c.y = list_bottom.max(d.y);
    }

    /// A plugin in the list: its tile, title and what it does, a chip for
    /// what's unusual, its switch, and ⋯ for the rest. Returns where the
    /// next goes.
    fn plugin_row(
        &mut self,
        row: &PluginRow,
        picked: Option<&str>,
        ui: &SettingsUiView,
        r: Rect,
        c: &mut Ctx,
    ) -> f32 {
        let p = self.p;
        let clip = c.clip;
        let on = picked == Some(row.name.as_str());
        let menu_open = ui.plugin_menu.as_deref() == Some(row.name.as_str());
        let hovered = self.s_hovered(r, clip);
        if on {
            self.s_round(r, 6.0, clip, p.chip);
        } else if hovered || menu_open {
            self.s_round(r, 6.0, clip, p.hover);
        }
        // The whole row picks it; the switch and ⋯ over it do their own.
        self.s_hit(
            r,
            clip,
            Hit::Settings(SettingsAction::Plugin(row.name.clone())),
        );
        let lit = row.on && !row.off_here;
        self.plugin_mark(&row.mark, r.x + 10.0, r.y + 10.0, 26.0, lit, clip);
        let mid = r.y + r.h / 2.0;
        let sw = Rect::new(r.right() - 44.0, mid - 10.0, 34.0, 20.0);
        self.switch(
            row.on,
            sw,
            clip,
            Hit::Settings(SettingsAction::Page(PageAction::Enable(
                row.name.clone(),
                !row.on,
            ))),
        );
        let mut x = sw.x - 6.0;
        if hovered || menu_open {
            let m = Rect::new(x - 24.0, mid - 12.0, 24.0, 24.0);
            if self.s_hovered(m, clip) || menu_open {
                self.s_round(m, 5.0, clip, p.border);
            }
            let f = Face::ui(13.0);
            let w = self.text.width("⋯", f);
            self.s_row_text("⋯", f, m.x + (m.w - w) / 2.0, m.y, m.h, p.bright, clip);
            let open = (!menu_open).then(|| row.name.clone());
            self.s_hit(m, clip, Hit::Settings(SettingsAction::PluginMenu(open)));
            if menu_open {
                c.list = Some((m, ListOf::PluginMenu(row.clone())));
            }
            x = m.x - 4.0;
        }
        let chip = if row.failed {
            Some("failed")
        } else if row.off_here {
            Some("off here")
        } else if row.waiting {
            Some("needs OK")
        } else {
            None
        };
        if let Some(chip) = chip {
            let w = self.text.width(chip, Face::ui(10.5).medium()) + 12.0;
            x -= w;
            self.tag(chip, x, mid, clip);
            x -= 6.0;
        }
        let tclip = Rect::new(r.x, r.y, x - r.x, r.h).intersect(clip);
        let fg = if lit { p.bright } else { p.muted };
        self.s_row_text(
            &row.title,
            Face::ui(12.5).bold(),
            r.x + 44.0,
            r.y + 5.0,
            18.0,
            fg,
            tclip,
        );
        let what = if row.what.is_empty() {
            &row.name
        } else {
            &row.what
        };
        let what = self
            .text
            .fit(what, Face::ui(11.5), tclip.right() - r.x - 48.0);
        self.s_row_text(
            &what,
            Face::ui(11.5),
            r.x + 44.0,
            r.y + 23.0,
            16.0,
            p.muted,
            tclip,
        );
        r.bottom() + 2.0
    }

    /// The filter over the plugin list: what's typed, or its hint.
    fn filter_field(&mut self, value: &str, typing: Option<&str>, r: Rect, clip: Rect) {
        let p = self.p;
        let border = if typing.is_some() {
            p.accent
        } else if self.s_hovered(r, clip) {
            mix(p.border, p.muted, 0.6)
        } else {
            p.border
        };
        self.s_outline(r, 6.0, clip, border, p.raised);
        let face = Face::ui(12.5);
        let inner = Rect::new(r.x + 10.0, r.y, r.w - 20.0, r.h).intersect(clip);
        match typing {
            Some(text) => {
                let tw = self.s_row_text(text, face, inner.x, r.y, r.h, p.bright, inner);
                self.s_fill(
                    Rect::new(inner.x + tw + 1.0, r.y + 7.0, 1.5, r.h - 14.0),
                    clip,
                    p.accent,
                );
            }
            None if value.is_empty() => {
                self.s_row_text(
                    "Filter installed plugins…",
                    face,
                    inner.x,
                    r.y,
                    r.h,
                    p.faint,
                    inner,
                );
            }
            None => {
                self.s_row_text(value, face, inner.x, r.y, r.h, p.text, inner);
            }
        }
        self.s_hit(
            r,
            clip,
            Hit::Settings(SettingsAction::Edit(PLUGIN_FILTER.into(), false)),
        );
    }

    /// Browse, until merlin has a registry: how to add a plugin.
    fn plugins_browse(&mut self, x: f32, mut y: f32, w: f32, clip: Rect) -> f32 {
        let p = self.p;
        let face = Face::ui(12.5);
        for (text, mono) in [
            (
                "merlin has no plugin registry yet. To add a plugin, build it and install it from its folder:",
                false,
            ),
            ("merlin plugin install <folder>", true),
            ("Or start your own:", false),
            ("merlin plugin new <name>", true),
        ] {
            if mono {
                let r = Rect::new(x, y, w, 30.0);
                self.s_round(r, 6.0, clip, p.chip);
                self.s_row_text(text, Face::mono(11.5), x + 10.0, y, 30.0, p.bright, clip);
                y += 40.0;
            } else {
                let h = self.text.wrapped_height(text, face, w);
                self.text
                    .draw(&mut self.canvas, text, face, x, y, p.text, clip, Some(w));
                y += h + 10.0;
            }
        }
        y
    }

    /// A plugin row's menu: on or off, here or everywhere; Reload, its
    /// Output and folder; and Uninstall.
    pub(super) fn plugin_menu(&mut self, under: Rect, row: &PluginRow, page: Rect) {
        let p = self.p;
        let name = || row.name.clone();
        let act = |a: PageAction| Some(Hit::Settings(SettingsAction::Page(a)));
        let mut items: Vec<(&str, &str, Option<Hit>, bool)> = Vec::new();
        if row.off_here {
            items.push((
                "Turn On Here",
                "",
                act(PageAction::EnableHere(name(), true)),
                false,
            ));
        }
        if row.on {
            items.push((
                "Turn Off Everywhere",
                "",
                act(PageAction::Enable(name(), false)),
                false,
            ));
            if !row.off_here {
                items.push((
                    "Turn Off in This Workspace",
                    "",
                    act(PageAction::EnableHere(name(), false)),
                    false,
                ));
            }
        } else {
            items.push(("Turn On", "", act(PageAction::Enable(name(), true)), false));
        }
        items.push(("", "", None, false));
        items.push((
            "Reload",
            "from disk",
            act(PageAction::Reload(name())),
            false,
        ));
        items.push(("Open Output", "", act(PageAction::Output(name())), false));
        items.push((
            "Show Its Folder",
            "",
            act(PageAction::Folder(name())),
            false,
        ));
        items.push(("", "", None, false));
        items.push(("Uninstall", "", act(PageAction::Uninstall(name())), true));
        let face = Face::ui(12.5);
        let title_h = 26.0;
        let item_h = |label: &str| if label.is_empty() { 9.0 } else { 28.0 };
        let w = items
            .iter()
            .map(|(l, d, ..)| self.text.width(l, face) + self.text.width(d, Face::ui(11.5)) + 48.0)
            .fold(220.0, f32::max);
        let h = items.iter().map(|(l, ..)| item_h(l)).sum::<f32>() + title_h + 8.0;
        let x = (under.right() - w).max(page.x + 8.0);
        let mut y = under.bottom() + 4.0;
        if y + h > page.bottom() - 8.0 {
            y = (under.y - 4.0 - h).max(page.y + 8.0);
        }
        let r = Rect::new(x, y, w, h);
        self.canvas.shadow(r, 8.0, false);
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        self.text.draw_in_row(
            &mut self.canvas,
            &row.title,
            Face::ui(11.5),
            r.x + 14.0,
            r.y + 4.0,
            title_h,
            p.muted,
            r,
        );
        let mut iy = r.y + 4.0 + title_h;
        for (label, what, hit, danger) in items {
            let ih = item_h(label);
            let Some(hit) = hit else {
                self.canvas
                    .fill(Rect::new(r.x + 10.0, iy + 4.0, r.w - 20.0, 1.0), p.chip);
                iy += ih;
                continue;
            };
            let row = Rect::new(r.x + 4.0, iy, r.w - 8.0, ih);
            if self.hovered(row) {
                self.canvas.round(row, 5.0, p.focus_row);
            }
            let fg = if danger { p.error } else { p.text };
            let lw =
                self.text
                    .draw_in_row(&mut self.canvas, label, face, row.x + 10.0, iy, ih, fg, row);
            if !what.is_empty() {
                self.text.draw_in_row(
                    &mut self.canvas,
                    what,
                    Face::ui(11.5),
                    row.x + 18.0 + lw,
                    iy,
                    ih,
                    p.muted,
                    row,
                );
            }
            self.hit(row, hit);
            iy += ih;
        }
    }

    /// A plugin off in this workspace: so, what isn't running, and a way
    /// back on.
    fn off_here_banner(&mut self, page: &PageView, c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        let r = Rect::new(c.x, c.y, c.w, 38.0);
        self.s_round(r, 7.0, clip, mix(p.warning, p.island, 0.14));
        let face = Face::ui(12.5);
        let label = "Turn On Here";
        let bw = self.text.width(label, face) + 20.0;
        let b = Rect::new(r.right() - bw - 8.0, r.y + 6.0, bw, 26.0);
        let bg = if self.s_hovered(b, clip) {
            p.border
        } else {
            p.chip
        };
        self.s_round(b, 6.0, clip, bg);
        self.s_row_text(label, face, b.x + 10.0, b.y, b.h, p.bright, clip);
        self.s_hit(
            b,
            clip,
            Hit::Settings(SettingsAction::Page(PageAction::EnableHere(
                page.name.clone(),
                true,
            ))),
        );
        let mut msg = "Off in this workspace, on everywhere else.".to_string();
        if let Some(prog) = page.processes.first() {
            let prog = prog.rsplit('/').next().unwrap_or(prog);
            msg += &format!(" {prog} isn't running.");
        }
        let tclip = Rect::new(r.x, r.y, b.x - r.x - 8.0, r.h).intersect(clip);
        let msg = self.text.fit(&msg, face, tclip.w - 24.0);
        self.s_row_text(&msg, face, r.x + 12.0, r.y, r.h, p.text, tclip);
        c.y += 48.0;
    }

    /// A plugin's mark: its monogram on a tinted square in its accent hue.
    pub(crate) fn plugin_mark(
        &mut self,
        (letters, hue): &(String, merlin_core::style::Hue),
        x: f32,
        y: f32,
        size: f32,
        on: bool,
        clip: Rect,
    ) {
        let p = self.p;
        let colour = if on {
            crate::theme::hue(&p, *hue)
        } else {
            p.muted
        };
        let r = Rect::new(x, y, size, size);
        self.s_round(r, 6.0, clip, mix(colour, p.island, 0.18));
        let face = Face::ui((size * 0.39).round()).bold();
        let lw = self.text.width(letters, face);
        self.s_row_text(letters, face, x + (size - lw) / 2.0, y, size, colour, clip);
    }

    pub(super) fn plugin_detail(&mut self, page: &PageView, ui: &SettingsUiView, c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        self.plugin_mark(&page.mark, c.x, c.y, 34.0, page.enabled, clip);
        let right = c.x + c.w;
        // Its switch, then Reload and Output.
        let sw = Rect::new(right - 34.0, c.y + 7.0, 34.0, 20.0);
        self.switch(
            page.enabled,
            sw,
            clip,
            Hit::Settings(SettingsAction::Page(PageAction::Enable(
                page.name.clone(),
                !page.enabled,
            ))),
        );
        let mut bx = sw.x - 12.0;
        let face = Face::ui(12.0);
        for (label, action) in [
            ("Output", PageAction::Output(page.name.clone())),
            ("Reload", PageAction::Reload(page.name.clone())),
        ] {
            let bw = self.text.width(label, face) + 20.0;
            bx -= bw;
            let b = Rect::new(bx, c.y + 4.0, bw, 26.0);
            let bg = if self.s_hovered(b, clip) {
                p.border
            } else {
                p.chip
            };
            self.s_round(b, 6.0, clip, bg);
            self.s_row_text(label, face, bx + 10.0, b.y, b.h, p.bright, clip);
            self.s_hit(b, clip, Hit::Settings(SettingsAction::Page(action)));
            bx -= 8.0;
        }
        let tclip = Rect::new(c.x, c.y, bx - c.x - 8.0, 40.0).intersect(clip);
        self.s_row_text(
            &page.title,
            Face::ui(15.0).bold(),
            c.x + 46.0,
            c.y - 1.0,
            20.0,
            p.bright,
            tclip,
        );
        // What it is and what it costs now, in a line.
        let mut sub = vec![page.name.clone(), format!("API {}", page.api)];
        if !page.data_only {
            sub.push(format!("{} in merlin", bytes(page.memory as u64)));
        }
        if page.pictures > 0 {
            sub.push(format!("{} of pictures", bytes(page.pictures as u64)));
        }
        for (prog, _, mem) in &page.running {
            let name = prog.rsplit('/').next().unwrap_or(prog);
            sub.push(format!("{name} {}", mem.map_or_else(|| "—".into(), bytes)));
        }
        if page.failed {
            sub.push("it failed to start".into());
        } else if page.waiting {
            sub.push("waiting for your OK".into());
        } else if page.dormant {
            sub.push("starts when needed".into());
        } else if !page.enabled {
            sub.push("off".into());
        }
        let sub = self
            .text
            .fit(&sub.join(" · "), Face::ui(11.5), tclip.w - 46.0);
        self.s_row_text(
            &sub,
            Face::ui(11.5),
            c.x + 46.0,
            c.y + 19.0,
            16.0,
            p.muted,
            tclip,
        );
        c.y += 48.0;
        if page.off_here {
            self.off_here_banner(page, c);
        }

        // Settings first, with what it may do and what it adds beside.
        let allowed = allowed(page);
        let adds = page.commands.len() + page.languages.len() + page.looks.len();
        let tabs = [
            ("Settings", page.rows.len(), PageTab::Settings),
            (
                "Allowed",
                allowed.iter().filter(|(ok, _)| *ok).count(),
                PageTab::Allowed,
            ),
            ("Adds", adds, PageTab::Adds),
        ];
        let bar = Rect::new(c.x, c.y, c.w, 30.0);
        self.s_fill(
            Rect::new(bar.x, bar.bottom() - 1.0, bar.w, 1.0),
            clip,
            p.line,
        );
        let mut x = bar.x;
        for (label, n, tab) in tabs {
            let f = Face::ui(12.5);
            let count = Face::ui(11.0);
            let lw = self.text.width(label, f);
            let n = n.to_string();
            let w = lw + 5.0 + self.text.width(&n, count);
            let r = Rect::new(x, bar.y, w, bar.h);
            let on = ui.page_tab == tab;
            let fg = if on || self.s_hovered(r, clip) {
                p.bright
            } else {
                p.muted
            };
            self.s_row_text(label, f, x, bar.y, bar.h - 8.0, fg, clip);
            self.s_row_text(&n, count, x + lw + 5.0, bar.y, bar.h - 8.0, p.faint, clip);
            if on {
                self.s_fill(Rect::new(x, bar.bottom() - 2.0, w, 2.0), clip, p.accent);
            }
            self.s_hit(r, clip, Hit::Settings(SettingsAction::PageTab(tab)));
            x += w + 20.0;
        }
        c.y += bar.h + 4.0;

        match ui.page_tab {
            PageTab::Settings if page.rows.is_empty() => {
                c.y += 8.0;
                let what = if page.data_only {
                    "It only adds looks, so it has nothing to set."
                } else {
                    "It has nothing to set."
                };
                self.s_row_text(what, Face::ui(12.5), c.x, c.y, 24.0, p.muted, clip);
                c.y += 28.0;
            }
            PageTab::Settings => {
                for row in &page.rows {
                    self.setting_row(row, ui, c);
                }
            }
            PageTab::Allowed => {
                c.y += 8.0;
                for (ok, what) in allowed {
                    let (mark, colour, fg) = if ok {
                        ("✓", p.success, p.text)
                    } else {
                        ("—", p.faint, p.muted)
                    };
                    let face = Face::ui(12.5);
                    let wrap = (c.w - 20.0).max(40.0);
                    let h = self.text.wrapped_height(&what, face, wrap);
                    self.s_row_text(mark, Face::ui(12.0), c.x, c.y, 24.0, colour, clip);
                    self.text.draw(
                        &mut self.canvas,
                        &what,
                        face,
                        c.x + 20.0,
                        c.y + (24.0 - 17.0) / 2.0,
                        fg,
                        clip,
                        Some(wrap),
                    );
                    c.y += (h + 7.0).max(24.0);
                }
                if !page.data_only {
                    let cpu = format!("{:.1} s of CPU since it started", page.busy.as_secs_f32());
                    c.y += 6.0;
                    self.s_row_text(&cpu, Face::ui(11.5), c.x, c.y, 20.0, p.muted, clip);
                    c.y += 24.0;
                }
            }
            PageTab::Adds => {
                c.y += 8.0;
                // Its commands without its name before each, with their keys.
                let prefix = format!("{}: ", page.title);
                for (title, key) in &page.commands {
                    let title = title.strip_prefix(&prefix).unwrap_or(title);
                    let face = Face::ui(12.5);
                    let key_w = key
                        .as_ref()
                        .map_or(0.0, |k| self.text.width(k, Face::ui(11.5)) + 12.0);
                    let shown = self.text.fit(title, face, (c.w - key_w).max(40.0));
                    if c.shows(24.0) {
                        self.s_row_text(&shown, face, c.x, c.y, 24.0, p.text, clip);
                        if let Some(k) = key {
                            let kw = self.text.width(k, Face::ui(11.5));
                            self.s_row_text(
                                k,
                                Face::ui(11.5),
                                right - kw,
                                c.y,
                                24.0,
                                p.muted,
                                clip,
                            );
                        }
                    }
                    c.y += 24.0;
                }
                let mut more: Vec<String> = Vec::new();
                if !page.languages.is_empty() {
                    more.push(format!("Languages: {}", page.languages.join(", ")));
                }
                if !page.looks.is_empty() {
                    more.push(format!("Looks: {}", page.looks.join(", ")));
                }
                if more.is_empty() && page.commands.is_empty() {
                    more.push("Nothing to show yet".into());
                }
                for line in more {
                    let face = Face::ui(12.5);
                    let h = self.text.wrapped_height(&line, face, c.w);
                    self.text.draw(
                        &mut self.canvas,
                        &line,
                        face,
                        c.x,
                        c.y + 4.0,
                        p.text,
                        clip,
                        Some(c.w),
                    );
                    c.y += h + 10.0;
                }
            }
        }
        if page.installed {
            c.y += 16.0;
            let label = if page.uninstall_armed {
                "Uninstall: press again"
            } else {
                "Uninstall"
            };
            let bw = self.text.width(label, face) + 20.0;
            let b = Rect::new(c.x, c.y, bw, 26.0);
            let bg = if self.s_hovered(b, clip) {
                p.border
            } else {
                p.chip
            };
            self.s_round(b, 6.0, clip, bg);
            self.s_row_text(label, face, b.x + 10.0, b.y, b.h, p.error, clip);
            self.s_hit(
                b,
                clip,
                Hit::Settings(SettingsAction::Page(PageAction::Uninstall(
                    page.name.clone(),
                ))),
            );
            c.y += 34.0;
        }
    }
}

/// What a plugin may do, each allowed or not.
fn allowed(page: &PageView) -> Vec<(bool, String)> {
    if page.data_only {
        return vec![(false, "Nothing: it's only data".into())];
    }
    let mut perms: Vec<(bool, String)> = vec![
        (page.read, "Read the open files".into()),
        (page.write, "Change files".into()),
    ];
    // Only plugins that ask for these say so: most never do.
    if page.workspace_read {
        perms.push((true, "Read the project's files".into()));
    }
    if page.terminal {
        perms.push((true, "Run command lines in a terminal".into()));
    }
    if page.processes.is_empty() {
        perms.push((false, "Start programs".into()));
    } else {
        perms.push((true, format!("Start {}", page.processes.join(", "))));
    }
    perms
}
