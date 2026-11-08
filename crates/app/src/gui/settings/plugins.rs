//! The Plugins page: each plugin as a row, and a plugin's own page with
//! its permissions, settings and what it adds.

use super::*;
use crate::prefs::{PageTab, SettingsUiView};

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
            240.0_f32.min(c.w * 0.38)
        };
        let top = c.y;
        let picked = v.page.as_ref().map(|pg| pg.name.as_str());
        let mut y = top;
        for row in &v.plugins {
            let r = Rect::new(c.x, y, list_w, 42.0);
            let on = picked == Some(row.name.as_str());
            if on {
                self.s_round(r, 6.0, clip, p.chip);
            } else if self.s_hovered(r, clip) {
                self.s_round(r, 6.0, clip, p.hover);
            }
            self.plugin_mark(&row.mark, r.x + 10.0, r.y + 8.0, 26.0, row.on, clip);
            let name_clip = Rect::new(r.x, r.y, r.w - 64.0, r.h).intersect(clip);
            let fg = if row.on { p.bright } else { p.muted };
            self.s_row_text(
                &row.title,
                Face::ui(12.5).bold(),
                r.x + 44.0,
                r.y + 4.0,
                18.0,
                fg,
                name_clip,
            );
            self.s_row_text(
                &row.name,
                Face::ui(11.0),
                r.x + 44.0,
                r.y + 21.0,
                16.0,
                p.muted,
                name_clip,
            );
            let (state, colour) = if row.failed {
                ("failed", p.error)
            } else if row.on {
                ("● on", p.success)
            } else {
                ("off", p.muted)
            };
            let f = Face::ui(11.0);
            let mem = if row.data_only || row.memory == 0 {
                "—".to_string()
            } else {
                bytes(row.memory)
            };
            let mw = self.text.width(&mem, f);
            self.s_row_text(
                &mem,
                f,
                r.right() - 10.0 - mw,
                r.y + 4.0,
                18.0,
                p.muted,
                clip,
            );
            let sw = self.text.width(state, f);
            self.s_row_text(
                state,
                f,
                r.right() - 10.0 - sw,
                r.y + 21.0,
                16.0,
                colour,
                clip,
            );
            self.s_hit(
                r,
                clip,
                Hit::Settings(SettingsAction::Plugin(row.name.clone())),
            );
            y += 44.0;
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
        for (prog, _, mem) in &page.running {
            let name = prog.rsplit('/').next().unwrap_or(prog);
            sub.push(format!("{name} {}", mem.map_or_else(|| "—".into(), bytes)));
        }
        if page.failed {
            sub.push("it failed to start".into());
        } else if page.waiting {
            sub.push("waiting for your OK".into());
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
