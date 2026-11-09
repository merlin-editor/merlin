//! Browse in Settings › Plugins, and what registries add to Installed:
//! the list of every source's plugins, a plugin's page with what
//! installing lets it do and its versions, updates, the banners for
//! withdrawn and blocked plugins, and the Install? card. As in
//! `docs/mockups/PluginBrowse.dc.html`.

use super::*;
use crate::browse::{
    BrowseAction, BrowsePage, BrowseRow, BrowseView, ConfirmView, RegistryPage, RowState,
    UpdateRow, VersionNote, VersionRow,
};
use crate::prefs::{PLUGIN_FILTER, SettingsUiView};
use merlin_plugins::registry::StaleKind;

/// A hit for a Browse click.
fn act(a: BrowseAction) -> Hit {
    Hit::Settings(SettingsAction::Browse(a))
}

/// How a banner is tinted.
#[derive(Clone, Copy)]
pub(super) enum Tint {
    Neutral,
    Accent,
    Warning,
    Error,
}

impl Frame<'_> {
    /// A small button ending at `right`: its rect. One without a hit is
    /// drawn faint.
    pub(super) fn s_button(
        &mut self,
        label: &str,
        right: f32,
        y: f32,
        primary: bool,
        hit: Option<Hit>,
        clip: Rect,
    ) -> Rect {
        let p = self.p;
        let face = Face::ui(12.0).medium();
        let w = self.text.width(label, face) + 20.0;
        let r = Rect::new(right - w, y, w, 26.0);
        let enabled = hit.is_some();
        let hovered = enabled && self.s_hovered(r, clip);
        let (mut bg, mut fg) = self.button_colours(primary && enabled, hovered);
        if !enabled {
            bg = mix(p.chip, p.island, 0.5);
            fg = p.faint;
        }
        self.s_round(r, 6.0, clip, bg);
        self.s_row_text(label, face, r.x + 10.0, r.y, r.h, fg, clip);
        if let Some(hit) = hit {
            self.s_hit(r, clip, hit);
        }
        r
    }

    /// A banner: wrapped text, and buttons at its right, the first the
    /// primary one.
    pub(super) fn s_banner(
        &mut self,
        text: &str,
        tint: Tint,
        buttons: &[(&str, Hit)],
        c: &mut Ctx,
    ) {
        let p = self.p;
        let clip = c.clip;
        let colour = match tint {
            Tint::Neutral => p.muted,
            Tint::Accent => p.accent,
            Tint::Warning => p.warning,
            Tint::Error => p.error,
        };
        let face = Face::ui(12.5);
        let widths: f32 = buttons
            .iter()
            .map(|(l, _)| self.text.width(l, Face::ui(12.0).medium()) + 28.0)
            .sum();
        let wrap = (c.w - 24.0 - widths).max(120.0);
        let th = self.text.wrapped_height(text, face, wrap);
        let h = (th + 20.0).max(38.0);
        let r = Rect::new(c.x, c.y, c.w, h);
        self.s_round(r, 7.0, clip, mix(colour, p.island, 0.13));
        self.text.draw(
            &mut self.canvas,
            text,
            face,
            r.x + 12.0,
            r.y + (h - th) / 2.0,
            p.text,
            clip,
            Some(wrap),
        );
        let mut right = r.right() - 8.0;
        for (i, (label, hit)) in buttons.iter().enumerate().rev() {
            let b = self.s_button(
                label,
                right,
                r.y + (h - 26.0) / 2.0,
                i == 0,
                Some(hit.clone()),
                clip,
            );
            right = b.x - 6.0;
        }
        c.y += h + 10.0;
    }

    /// Browse's list, under the tabs: banners, the filter, the sources'
    /// line and each plugin. Returns where it ends.
    pub(super) fn browse_list(
        &mut self,
        v: &BrowseView,
        ui: &SettingsUiView,
        x: f32,
        y: f32,
        w: f32,
        clip: Rect,
    ) -> f32 {
        let p = self.p;
        let mut c = Ctx {
            x,
            w,
            y,
            clip,
            list: None,
            picked: None,
        };
        for b in &v.banners {
            let tint = match b.kind {
                StaleKind::Unreachable => Tint::Neutral,
                StaleKind::Expired => Tint::Warning,
                StaleKind::Rejected => Tint::Error,
            };
            let retry = act(BrowseAction::Check);
            let buttons: &[(&str, Hit)] = match b.kind {
                StaleKind::Unreachable => &[("Try Again", retry)],
                _ => &[],
            };
            self.s_banner(&b.text, tint, buttons, &mut c);
        }
        let typing = ui
            .editing
            .as_ref()
            .filter(|e| e.key == PLUGIN_FILTER)
            .map(|e| e.text.as_str());
        let hint = format!("Search {} plugins…", v.rows.len());
        let field = Rect::new(x, c.y, w, 30.0);
        self.filter_field(&ui.plugin_filter, typing, &hint, field, clip);
        c.y += 36.0;
        // Where the list is from, and ↻ to check again.
        let f = Face::ui(11.0);
        self.s_row_text(&v.status, f, x + 4.0, c.y, 18.0, p.faint, clip);
        let again = Rect::new(x + w - 22.0, c.y - 2.0, 20.0, 22.0);
        let fg = if self.s_hovered(again, clip) {
            p.bright
        } else {
            p.accent
        };
        self.centred("↻", Face::ui(12.5), again, fg, clip);
        self.s_hit(again, clip, act(BrowseAction::Check));
        c.y += 24.0;
        if v.rows.is_empty() && v.banners.is_empty() {
            let what = if ui.plugin_filter.is_empty() && typing.is_none_or(str::is_empty) {
                "Nothing yet: merlin is checking the registry."
            } else {
                "No plugins match."
            };
            self.s_row_text(what, Face::ui(12.5), x + 4.0, c.y, 24.0, p.muted, clip);
            c.y += 28.0;
        }
        let picked = v
            .page
            .as_ref()
            .map(|pg| (pg.source.as_str(), pg.name.as_str()));
        for row in &v.rows {
            let r = Rect::new(x, c.y, w, 46.0);
            self.browse_row(row, picked, v, r, clip);
            c.y += 48.0;
        }
        // Plugins no registry has.
        c.y += 6.0;
        let f = Face::ui(11.5);
        let lead = "Not in a registry? ";
        let lw = self.s_row_text(lead, f, x + 4.0, c.y, 20.0, p.faint, clip);
        self.s_row_text(
            "merlin plugin install <folder>",
            Face::mono(11.0),
            x + 4.0 + lw,
            c.y,
            20.0,
            p.muted,
            clip,
        );
        c.y + 26.0
    }

    fn browse_row(
        &mut self,
        row: &BrowseRow,
        picked: Option<(&str, &str)>,
        v: &BrowseView,
        r: Rect,
        clip: Rect,
    ) {
        let p = self.p;
        let on = picked == Some((row.source.as_str(), row.name.as_str()));
        if on {
            self.s_round(r, 6.0, clip, p.chip);
        } else if self.s_hovered(r, clip) {
            self.s_round(r, 6.0, clip, p.hover);
        }
        self.s_hit(
            r,
            clip,
            act(BrowseAction::Pick(row.source.clone(), row.name.clone())),
        );
        let usable = row.state != RowState::NotForThis;
        self.plugin_mark(&row.mark, r.x + 10.0, r.y + 10.0, 26.0, usable, clip);
        let mid = r.y + r.h / 2.0;
        // At the right: what you have of it, then where it's from.
        let mut right = r.right() - 10.0;
        let busy = v.busy.as_deref() == Some(row.name.as_str());
        let (chip, colour) = match row.state {
            _ if busy => (Some("installing…".to_string()), p.accent),
            RowState::Installed => (Some("✓ installed".into()), p.success),
            RowState::Update => (Some("update".into()), p.accent),
            RowState::Linked => (Some("linked".into()), p.muted),
            RowState::NotForThis => (
                Some(format!(
                    "for {}",
                    row.api.as_deref().unwrap_or("another merlin")
                )),
                p.muted,
            ),
            RowState::Available => (None, p.muted),
        };
        match chip {
            Some(chip) => {
                right -= self.s_chip(&chip, right, mid, colour, clip) + 6.0;
            }
            None if !row.version.is_empty() => {
                let f = Face::ui(11.0);
                let vw = self.text.width(&row.version, f);
                self.s_row_text(&row.version, f, right - vw, r.y, r.h, p.faint, clip);
                right -= vw + 8.0;
            }
            None => {}
        }
        if v.many {
            right -= self.s_chip(&row.source, right, mid, p.muted, clip) + 6.0;
        }
        let tclip = Rect::new(r.x, r.y, right - r.x, r.h).intersect(clip);
        let fg = if usable { p.bright } else { p.muted };
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
    }

    /// A small label in `colour`, ending at `right`: its width.
    pub(super) fn s_chip(
        &mut self,
        label: &str,
        right: f32,
        mid: f32,
        colour: u32,
        clip: Rect,
    ) -> f32 {
        let p = self.p;
        let face = Face::ui(10.5).medium();
        let w = self.text.width(label, face) + 12.0;
        let r = Rect::new(right - w, mid - 8.0, w, 16.0);
        self.s_round(r, 4.0, clip, mix(colour, p.island, 0.16));
        self.s_row_text(label, face, r.x + 6.0, r.y, r.h, colour, clip);
        w
    }

    /// A plugin's page in Browse.
    pub(super) fn browse_page(&mut self, page: &BrowsePage, v: &BrowseView, c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        let usable = page.version.is_some();
        self.plugin_mark(&page.mark, c.x, c.y, 34.0, usable, clip);
        let right = c.x + c.w;
        // Install, or what you have; then Report.
        let busy = v.busy.as_deref() == Some(page.name.as_str());
        let install = act(BrowseAction::Install {
            source: page.source.clone(),
            name: page.name.clone(),
            confirmed: false,
        });
        let (label, hit) = match page.state {
            _ if busy => ("Installing…", None),
            RowState::Installed => ("Installed", None),
            RowState::Update => ("Update", Some(act(BrowseAction::Update(page.name.clone())))),
            RowState::Linked => ("Linked", None),
            RowState::NotForThis => ("Install", None),
            RowState::Available if usable => ("Install", Some(install)),
            RowState::Available => ("Install", None),
        };
        let b = self.s_button(label, right, c.y + 4.0, true, hit, clip);
        let mut bx = b.x - 8.0;
        if let Some(url) = &page.report {
            let r = self.s_button(
                "⚑ Report",
                bx,
                c.y + 4.0,
                false,
                Some(act(BrowseAction::Open(url.clone()))),
                clip,
            );
            bx = r.x - 8.0;
        }
        let card_under = b;
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
        let mut sub = vec![page.name.clone()];
        if let Some(v) = page
            .version
            .as_ref()
            .or(page.versions.first().map(|v| &v.version))
        {
            sub.push(v.clone());
        }
        if let Some(l) = &page.license {
            sub.push(l.clone());
        }
        if !page.owners.is_empty() {
            sub.push(format!("by {}", page.owners.join(", ")));
        }
        if v.many {
            sub.push(format!("from {}", page.source));
        }
        let sub = self
            .text
            .fit(&sub.join(" · "), Face::ui(11.5), tclip.w - 46.0);
        let sw = self.s_row_text(
            &sub,
            Face::ui(11.5),
            c.x + 46.0,
            c.y + 19.0,
            16.0,
            p.muted,
            tclip,
        );
        if let Some(repo) = page
            .repository
            .as_ref()
            .filter(|r| r.starts_with("https://"))
        {
            let shown = repo.trim_start_matches("https://");
            let link = format!(" · {shown} ↗");
            let lx = c.x + 46.0 + sw;
            let lw = self.text.width(&link, Face::ui(11.5));
            let lr = Rect::new(lx, c.y + 19.0, lw, 16.0);
            let fg = if self.s_hovered(lr, tclip) {
                mix(p.accent, 0xffffff, 0.8)
            } else {
                p.accent
            };
            self.s_row_text(&link, Face::ui(11.5), lx, c.y + 19.0, 16.0, fg, tclip);
            self.s_hit(
                lr.intersect(tclip),
                clip,
                act(BrowseAction::Open(repo.clone())),
            );
        }
        c.y += 50.0;
        if let Some(none) = &page.none {
            self.s_banner(none, Tint::Neutral, &[], c);
        }
        if !page.description.is_empty() {
            let face = Face::ui(13.0);
            let wrap = c.w.min(640.0);
            let h = self.text.wrapped_height(&page.description, face, wrap);
            self.text.draw(
                &mut self.canvas,
                &page.description,
                face,
                c.x,
                c.y,
                p.text,
                clip,
                Some(wrap),
            );
            c.y += h + 6.0;
        }

        self.s_heading(
            if page.state == RowState::Installed {
                "It's allowed to"
            } else {
                "Installing lets it"
            },
            None,
            c,
        );
        if page.permissions.is_empty() {
            self.perm_line(
                "✓",
                p.success,
                "Nothing beyond merlin's sandbox: it reads what merlin hands it",
                p.text,
                c,
            );
        }
        for (what, outside) in &page.permissions {
            let what = capital(what);
            if *outside {
                self.perm_line("⚠", p.warning, &what, p.text, c);
            } else {
                self.perm_line("✓", p.success, &what, p.text, c);
            }
        }
        for what in &page.not_asked {
            self.perm_line("—", p.faint, what, p.muted, c);
        }

        if !page.versions.is_empty() {
            self.s_heading("Versions", None, c);
            self.versions_table(&page.versions, c);
            if let Some(commit) = &page.commit {
                let short = commit.get(..7).unwrap_or(commit);
                let line = format!(
                    "Built in the registry's CI from commit {short} · SHA-256 checked on download"
                );
                c.y += 6.0;
                self.s_row_text(&line, Face::ui(11.5), c.x, c.y, 18.0, p.faint, clip);
                c.y += 24.0;
            }
        }
        if let Some(card) = &v.confirm {
            c.list = Some((card_under, ListOf::Confirm(card.clone())));
        }
    }

    fn perm_line(&mut self, mark: &str, colour: u32, what: &str, fg: u32, c: &mut Ctx) {
        let face = Face::ui(12.5);
        let wrap = (c.w - 22.0).max(40.0);
        let h = self.text.wrapped_height(what, face, wrap);
        self.s_row_text(mark, Face::ui(12.0), c.x, c.y, 24.0, colour, c.clip);
        self.text.draw(
            &mut self.canvas,
            what,
            face,
            c.x + 22.0,
            c.y + (24.0 - 17.0) / 2.0,
            fg,
            c.clip,
            Some(wrap),
        );
        c.y += (h + 7.0).max(24.0);
    }

    /// Versions, newest first: each with its API, day, size and note.
    pub(super) fn versions_table(&mut self, rows: &[VersionRow], c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        let w = c.w.min(640.0);
        for v in rows {
            let r = Rect::new(c.x, c.y, w, 30.0);
            self.s_fill(Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0), clip, p.line);
            let dim = matches!(
                v.note,
                VersionNote::Withdrawn(_) | VersionNote::Blocked(_) | VersionNote::Older
            );
            let fg = if dim { p.faint } else { p.text };
            let vw = self.s_row_text(
                &v.version,
                Face::mono(12.0),
                r.x,
                r.y,
                r.h,
                if dim { p.faint } else { p.bright },
                clip,
            );
            if matches!(v.note, VersionNote::Withdrawn(_) | VersionNote::Blocked(_)) {
                self.s_fill(Rect::new(r.x, r.y + r.h / 2.0, vw, 1.0), clip, p.faint);
            }
            let cols = [
                (format!("API {}", v.api), 70.0),
                (v.published.clone(), 160.0),
                (bytes(v.size), 260.0),
            ];
            for (text, at) in cols {
                if r.x + at < r.right() - 60.0 {
                    self.s_row_text(&text, Face::ui(12.0), r.x + at, r.y, r.h, fg, clip);
                }
            }
            let mid = r.y + r.h / 2.0;
            let note = match &v.note {
                VersionNote::Installs => Some(("installs".to_string(), p.accent)),
                VersionNote::Installed => Some(("installed".to_string(), p.success)),
                VersionNote::Withdrawn(why) => Some((format!("withdrawn: {why}"), p.warning)),
                VersionNote::Blocked(why) => Some((format!("blocked: {why}"), p.error)),
                VersionNote::Older => {
                    let t = "for an older merlin";
                    let tw = self.text.width(t, Face::ui(12.0));
                    self.s_row_text(t, Face::ui(12.0), r.right() - tw, r.y, r.h, p.faint, clip);
                    None
                }
                VersionNote::None => None,
            };
            if let Some((label, colour)) = note {
                let room = (r.w - 340.0).max(80.0);
                let label = self.text.fit(&label, Face::ui(10.5).medium(), room);
                self.s_chip(&label, r.right(), mid, colour, clip);
            }
            c.y += 30.0;
        }
    }

    /// Updates, over the installed plugins: each with Update, or Review…
    /// when it asks for more. Returns where the list goes on.
    pub(super) fn updates_list(
        &mut self,
        updates: &[UpdateRow],
        busy: Option<&str>,
        x: f32,
        mut y: f32,
        w: f32,
        clip: Rect,
    ) -> f32 {
        let p = self.p;
        let head = |n: usize| format!("UPDATES · {n}");
        let face = Face::ui(10.5).bold();
        self.s_row_text(&head(updates.len()), face, x + 10.0, y, 20.0, p.muted, clip);
        if updates.iter().any(|u| u.more.is_empty()) {
            let f = Face::ui(11.5).medium();
            let label = "Update All";
            let tw = self.text.width(label, f);
            let r = Rect::new(x + w - 10.0 - tw, y, tw, 20.0);
            let fg = if self.s_hovered(r, clip) {
                mix(p.accent, 0xffffff, 0.8)
            } else {
                p.accent
            };
            self.s_row_text(label, f, r.x, y, 20.0, fg, clip);
            self.s_hit(r, clip, act(BrowseAction::UpdateAll));
        }
        y += 24.0;
        for u in updates {
            let r = Rect::new(x, y, w, 46.0);
            if self.s_hovered(r, clip) {
                self.s_round(r, 6.0, clip, p.hover);
            }
            self.s_hit(
                r,
                clip,
                Hit::Settings(SettingsAction::Plugin(u.name.clone())),
            );
            self.plugin_mark(&u.mark, r.x + 10.0, r.y + 10.0, 26.0, true, clip);
            let mid = r.y + 10.0;
            let (label, hit) = if busy == Some(u.name.as_str()) {
                ("Updating…", None)
            } else if u.more.is_empty() {
                ("Update", Some(act(BrowseAction::Update(u.name.clone()))))
            } else {
                (
                    "Review…",
                    Some(Hit::Settings(SettingsAction::Plugin(u.name.clone()))),
                )
            };
            let b = self.s_button(
                label,
                r.right() - 8.0,
                mid,
                u.more.is_empty() && u.withdrawn.is_some(),
                hit,
                clip,
            );
            let tclip = Rect::new(r.x, r.y, b.x - r.x - 6.0, r.h).intersect(clip);
            self.s_row_text(
                &u.title,
                Face::ui(12.5).bold(),
                r.x + 44.0,
                r.y + 5.0,
                18.0,
                p.bright,
                tclip,
            );
            let mut what = format!("{} → {}", u.from, u.to);
            let colour = if u.withdrawn.is_some() {
                what = format!("{} · withdrawn", u.from);
                p.warning
            } else if !u.more.is_empty() {
                what += " · wants more";
                p.muted
            } else {
                p.muted
            };
            self.s_row_text(
                &what,
                Face::ui(11.5),
                r.x + 44.0,
                r.y + 23.0,
                16.0,
                colour,
                tclip,
            );
            y += 48.0;
        }
        self.s_row_text("ALL", face, x + 10.0, y + 4.0, 20.0, p.muted, clip);
        y + 30.0
    }

    /// A registry plugin's banners on its Installed page: blocked,
    /// withdrawn, or an update, saying what it newly asks for.
    pub(super) fn registry_banners(
        &mut self,
        name: &str,
        reg: &RegistryPage,
        busy: bool,
        c: &mut Ctx,
    ) {
        let uninstall = Hit::Settings(SettingsAction::Page(crate::prefs::PageAction::Uninstall(
            name.to_string(),
        )));
        if let Some(why) = &reg.blocked {
            let text = format!(
                "Blocked by the {} registry's maintainers: {why}. It's off, and can't be turned back on.",
                reg.source
            );
            self.s_banner(&text, Tint::Error, &[("Uninstall", uninstall)], c);
            return;
        }
        let update = || act(BrowseAction::Update(name.to_string()));
        if let Some(why) = &reg.withdrawn {
            let next = reg.update.as_ref().map(|u| u.to.clone());
            let text = match &next {
                Some(to) => format!("Its owners withdrew {}: “{why}”. {to} is out.", reg.version),
                None => format!(
                    "Its owners withdrew {}: “{why}”. There's no newer version yet.",
                    reg.version
                ),
            };
            let buttons: Vec<(&str, Hit)> = match (&next, busy) {
                (Some(_), false) if reg.update.as_ref().is_some_and(|u| u.more.is_empty()) => {
                    vec![("Update", update())]
                }
                _ => Vec::new(),
            };
            self.s_banner(&text, Tint::Warning, &buttons, c);
        }
        let Some(u) = &reg.update else {
            return;
        };
        if busy {
            self.s_banner(&format!("Updating to {}…", u.to), Tint::Accent, &[], c);
        } else if !u.more.is_empty() {
            let mut text = format!("{} also wants to:", u.to);
            for (what, _) in &u.more {
                text += &format!("\n⚠ {}", capital(what));
            }
            let skip = act(BrowseAction::Skip(name.to_string(), u.to.clone()));
            self.s_banner(
                &text,
                Tint::Warning,
                &[("Allow and Update", update()), ("Skip This Version", skip)],
                c,
            );
        } else if reg.withdrawn.is_none() {
            let skip = act(BrowseAction::Skip(name.to_string(), u.to.clone()));
            self.s_banner(
                &format!("{} is out. You have {}.", u.to, u.from),
                Tint::Accent,
                &[("Update", update()), ("Skip", skip)],
                c,
            );
        }
    }

    /// The Install? or Replace? card, under the button that opened it.
    pub(super) fn confirm_card(&mut self, under: Rect, card: &ConfirmView, area: Rect) {
        let p = self.p;
        let w = 340.0_f32.min(area.w - 16.0);
        let face = Face::ui(12.0);
        let wrap = w - 32.0;
        let text_h = self.text.wrapped_height(&card.text, face, wrap);
        let items_h: f32 = card
            .items
            .iter()
            .map(|i| {
                self.text
                    .wrapped_height(&capital(i), Face::ui(12.5), wrap - 22.0)
                    + 8.0
            })
            .sum();
        let h = 16.0 + 20.0 + 6.0 + text_h + 8.0 + items_h + 12.0 + 26.0 + 16.0;
        let x = (under.right() - w).max(area.x + 8.0);
        let y = under.bottom() + 6.0;
        let r = Rect::new(x, y, w, h);
        self.canvas.shadow(r, 10.0, false);
        self.canvas.outline(r, 10.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let mut cy = r.y + 14.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &card.title,
            Face::ui(13.5).bold(),
            r.x + 16.0,
            cy,
            20.0,
            p.bright,
            r,
        );
        cy += 26.0;
        self.text.draw(
            &mut self.canvas,
            &card.text,
            face,
            r.x + 16.0,
            cy,
            p.muted,
            r,
            Some(wrap),
        );
        cy += text_h + 8.0;
        for item in &card.items {
            let item = capital(item);
            let ih = self.text.wrapped_height(&item, Face::ui(12.5), wrap - 22.0);
            self.text.draw_in_row(
                &mut self.canvas,
                "⚠",
                Face::ui(12.0),
                r.x + 16.0,
                cy,
                18.0,
                p.warning,
                r,
            );
            self.text.draw(
                &mut self.canvas,
                &item,
                Face::ui(12.5),
                r.x + 38.0,
                cy + 1.0,
                p.text,
                r,
                Some(wrap - 22.0),
            );
            cy += ih + 8.0;
        }
        cy += 4.0;
        let ok = self.s_button(
            &card.button,
            r.right() - 16.0,
            cy,
            true,
            Some(act(card.action.clone())),
            r,
        );
        self.s_button(
            "Cancel",
            ok.x - 8.0,
            cy,
            false,
            Some(act(BrowseAction::Cancel)),
            r,
        );
    }
}

/// "run `go`, outside…" as "Run `go`, outside…".
fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
