//! The Plugins page: each plugin as a row, and a plugin's own page with
//! its permissions, settings and what it adds.

use super::*;

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
            self.plugin_mark(
                &row.mark,
                r.x + 10.0,
                r.y + 8.0,
                26.0,
                row.on,
                clip,
            );
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
            self.plugin_detail(page, &mut d);
        }
        c.y = list_bottom.max(d.y);
    }

    /// A plugin's mark: its monogram on a tinted square in its accent hue.
    pub(super) fn plugin_mark(
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

    pub(super) fn plugin_detail(&mut self, page: &PageView, c: &mut Ctx) {
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
            ("Output", PageAction::Output),
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
        let origin = if page.installed {
            "installed"
        } else {
            "bundled"
        };
        let state = if page.failed {
            " · it failed to start"
        } else if page.waiting {
            " · waiting for your OK"
        } else if !page.enabled {
            " · off"
        } else {
            ""
        };
        let sub = format!("{} · API {} · {origin}{state}", page.name, page.api);
        self.s_row_text(
            &sub,
            Face::ui(11.5),
            c.x + 46.0,
            c.y + 19.0,
            16.0,
            p.muted,
            tclip,
        );
        c.y += 52.0;

        // What it costs now.
        let mut stats: Vec<(String, String)> = Vec::new();
        if !page.data_only {
            stats.push((bytes(page.memory as u64), "in merlin".into()));
        }
        for (prog, pid, mem) in &page.running {
            let name = prog.rsplit('/').next().unwrap_or(prog);
            stats.push((
                mem.map_or_else(|| "—".into(), bytes),
                format!("{name}, pid {pid}"),
            ));
        }
        if !page.data_only {
            stats.push((
                format!("{:.1} s", page.busy.as_secs_f32()),
                "CPU since it started".into(),
            ));
        }
        stats.push((
            format!("{} · {}", page.commands.len(), page.languages.len()),
            "commands · languages".into(),
        ));
        let mut sx = c.x;
        for (value, what) in &stats {
            let vw = self.s_row_text(value, Face::ui(15.0).bold(), sx, c.y, 20.0, p.bright, clip);
            let ww = self.s_row_text(what, Face::ui(11.0), sx, c.y + 21.0, 16.0, p.muted, clip);
            sx += vw.max(ww) + 28.0;
            if sx > right - 60.0 {
                break;
            }
        }
        c.y += 44.0;

        // What it may do, and what it adds.
        let half = (c.w - 24.0) / 2.0;
        let top = c.y;
        let mut l = Ctx {
            x: c.x,
            w: half,
            y: top,
            clip,
            list: None,
            picked: None,
        };
        self.s_heading("Allowed", None, &mut l);
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
        if page.data_only {
            perms = vec![(false, "Nothing: it's only data".into())];
        }
        for (ok, what) in perms {
            let (mark, colour, fg) = if ok {
                ("✓", p.success, p.text)
            } else {
                ("—", p.faint, p.muted)
            };
            let face = Face::ui(12.5);
            let wrap = (l.w - 20.0).max(40.0);
            let h = self.text.wrapped_height(&what, face, wrap);
            self.s_row_text(mark, Face::ui(12.0), l.x, l.y, 24.0, colour, clip);
            self.text.draw(
                &mut self.canvas,
                &what,
                face,
                l.x + 20.0,
                l.y + (24.0 - 17.0) / 2.0,
                fg,
                clip,
                Some(wrap),
            );
            l.y += (h + 7.0).max(24.0);
        }
        let mut a = Ctx {
            x: c.x + half + 24.0,
            w: half,
            y: top,
            clip,
            list: None,
            picked: None,
        };
        self.s_heading("Adds", None, &mut a);
        // Its commands without its name before each, with their keys.
        let prefix = format!("{}: ", page.title);
        for (title, key) in page.commands.iter().take(5) {
            let title = title.strip_prefix(&prefix).unwrap_or(title);
            let face = Face::ui(12.5);
            let key_w = key
                .as_ref()
                .map_or(0.0, |k| self.text.width(k, Face::ui(11.5)) + 12.0);
            let shown = self.text.fit(title, face, (a.w - key_w).max(40.0));
            let tw = self.s_row_text(&shown, face, a.x, a.y, 24.0, p.text, clip);
            if let Some(k) = key {
                self.s_row_text(k, Face::ui(11.5), a.x + tw + 12.0, a.y, 24.0, p.muted, clip);
            }
            a.y += 24.0;
        }
        let mut adds: Vec<String> = Vec::new();
        if page.commands.len() > 5 {
            adds.push(format!("and {} more commands", page.commands.len() - 5));
        }
        if !page.languages.is_empty() {
            adds.push(format!("Languages: {}", page.languages.join(", ")));
        }
        if !page.looks.is_empty() {
            adds.push(format!("Looks: {}", page.looks.join(", ")));
        }
        if adds.is_empty() && page.commands.is_empty() {
            adds.push("Nothing to show yet".into());
        }
        for line in adds {
            let line = self.text.fit(&line, Face::ui(12.5), a.w);
            self.s_row_text(&line, Face::ui(12.5), a.x, a.y, 24.0, p.text, clip);
            a.y += 24.0;
        }
        c.y = l.y.max(a.y);

        // Its own settings, as rows.
        if !page.settings.is_empty() {
            self.s_heading("Its settings", None, c);
            for (key, value, changed) in &page.settings {
                let r = Rect::new(c.x, c.y, c.w, 40.0);
                let mut x = c.x;
                x += self.s_row_text(key, Face::ui(13.0).medium(), x, r.y, r.h, p.bright, clip)
                    + 8.0;
                if *changed {
                    self.tag("Modified", x, r.y + r.h / 2.0, clip);
                }
                let ctl_right = r.right();
                match value {
                    toml::Value::Boolean(on) => {
                        let s = Rect::new(ctl_right - 34.0, r.y + 10.0, 34.0, 20.0);
                        let action = PageAction::Setting(
                            page.name.clone(),
                            key.clone(),
                            toml::Value::Boolean(!on),
                        );
                        self.switch(*on, s, clip, Hit::Settings(SettingsAction::Page(action)));
                    }
                    other => {
                        let text = match other {
                            toml::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        let f = Face::mono(11.5);
                        let text = self.text.fit(&text, f, c.w * 0.45);
                        let tw = self.text.width(&text, f);
                        let b = Rect::new(ctl_right - tw - 20.0, r.y + 6.0, tw + 20.0, 28.0);
                        self.s_outline(b, 6.0, clip, p.border, p.raised);
                        self.s_row_text(&text, f, b.x + 10.0, b.y, b.h, p.text, clip);
                    }
                }
                self.s_fill(Rect::new(c.x, r.bottom() - 1.0, c.w, 1.0), clip, p.line);
                c.y += 40.0;
            }
            self.s_row_text(
                "Other values are set in settings.toml, under [plugins].",
                Face::ui(11.5),
                c.x,
                c.y + 4.0,
                20.0,
                p.muted,
                clip,
            );
            c.y += 28.0;
        }
        if page.installed {
            c.y += 12.0;
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
