//! The Appearance page: themes as cards, picked with the mouse or keys, and
//! the page's own rows.

use super::*;

impl<'a> Frame<'a> {
    pub(super) fn appearance(&mut self, v: &'a SettingsView, c: &mut Ctx) {
        let p = self.p;
        let more = Hit::Settings(SettingsAction::Category(Category::Plugins));
        self.s_heading("Colour theme", Some(("More themes in Plugins ›", more)), c);
        let follow = Row {
            label: "Follow the system".into(),
            about: "One theme for a dark desktop, another for a light one.".into(),
            key: "editor.theme_follows_system".into(),
            control: Control::Toggle(v.follow),
            changed: false,
            in_workspace: false,
            note: None,
            yours: None,
            user_only: false,
            locked: false,
        };
        // The row's switch flips it, rather than writing it as a toggle.
        let at = self.hits.len();
        self.setting_row(&follow, &v.ui, c);
        let toggle = Hit::Settings(SettingsAction::Toggle(follow.key.clone()));
        for (_, hit) in &mut self.hits[at..] {
            if *hit == toggle {
                *hit = Hit::Settings(SettingsAction::Follow);
            }
        }
        c.y += 12.0;
        let cols = (((c.w + CARD_GAP) / (CARD_W + CARD_GAP)).floor() as usize).clamp(2, 6);
        let cw = (c.w - CARD_GAP * (cols - 1) as f32) / cols as f32;
        let card_h = 62.0 + 28.0;
        for (i, card) in v.themes.iter().enumerate() {
            let (col, line) = (i % cols, i / cols);
            let r = Rect::new(
                c.x + col as f32 * (cw + CARD_GAP),
                c.y + line as f32 * (card_h + CARD_GAP),
                cw,
                card_h,
            );
            self.theme_card(card, r, v.ui.card == Some(i), v.follow, c.clip);
        }
        let lines = v.themes.len().div_ceil(cols);
        c.y += lines as f32 * (card_h + CARD_GAP);

        self.s_heading("Icons", None, c);
        for (i, card) in v.icons.iter().enumerate() {
            let (col, line) = (i % cols, i / cols);
            let r = Rect::new(
                c.x + col as f32 * (cw + CARD_GAP),
                c.y + line as f32 * (52.0 + CARD_GAP),
                cw,
                52.0,
            );
            let clip = c.clip;
            let hovered = self.s_hovered(r, clip);
            let bg = if hovered { p.hover } else { p.widget };
            let edge = if card.current { p.accent } else { p.border };
            self.card_ground(r, clip, bg, edge, card.current);
            let face = Face::ui(12.0);
            self.s_row_text(&card.label, face, r.x + 10.0, r.y + 6.0, 18.0, p.text, clip);
            let old = self.icons;
            self.icons = &card.icons;
            let mut x = r.x + 10.0;
            for name in ["main.rs", "README.md", "app.js", "main.py"] {
                let core = merlin_core::file_icon(name, true);
                x += self.file_icon(name, Some(core), x, r.y + 30.0, 14.0, clip.intersect(r));
            }
            self.icons = old;
            let from = Face::ui(10.5);
            let fw = self.text.width(&card.from, from);
            if x + fw + 8.0 < r.right() {
                self.s_row_text(
                    &card.from,
                    from,
                    r.right() - 10.0 - fw,
                    r.y + 28.0,
                    18.0,
                    p.muted,
                    clip,
                );
            }
            self.s_hit(
                r,
                clip,
                Hit::Settings(SettingsAction::Icons(card.label.clone())),
            );
        }
        c.y += v.icons.len().div_ceil(cols) as f32 * (52.0 + CARD_GAP);

        let add = Hit::Settings(SettingsAction::OpenChoice("packs.add".into()));
        let link = (!v.available.is_empty()).then_some(("Add a layout ▾", add));
        let link_at = c.y + 20.0;
        self.s_heading("Layouts", link, c);
        if v.ui.open_choice.as_deref() == Some("packs.add") {
            let under = Rect::new(c.x + c.w - 120.0, link_at - 2.0, 120.0, 18.0);
            c.list = Some((under, ListOf::Packs(v.available.clone())));
        }
        if v.layers.is_empty() {
            let clip = c.clip;
            self.s_row_text(
                "None stacked: merlin's own layout.",
                Face::ui(12.5),
                c.x,
                c.y,
                24.0,
                p.muted,
                clip,
            );
            c.y += 28.0;
        }
        let n = v.layers.len();
        for (i, (label, what)) in v.layers.iter().enumerate() {
            let clip = c.clip;
            let row = Rect::new(c.x, c.y, c.w, 36.0);
            let face = Face::ui(12.5);
            let lw = self.s_row_text(label, face.medium(), c.x, c.y, 36.0, p.bright, clip);
            self.s_row_text(
                what,
                Face::ui(12.0),
                c.x + 14.0 + lw,
                c.y,
                36.0,
                p.muted,
                clip,
            );
            let mut bx = row.right() - 24.0;
            for (glyph, action, shown) in [
                ("✕", SettingsAction::PackOff(i), true),
                ("↓", SettingsAction::PackDown(i), i + 1 < n),
                ("↑", SettingsAction::PackUp(i), i > 0),
            ] {
                let b = Rect::new(bx, c.y + 6.0, 24.0, 24.0);
                if shown {
                    let bg = if self.s_hovered(b, clip) {
                        p.border
                    } else {
                        p.raised
                    };
                    self.s_outline(b, 5.0, clip, p.border, bg);
                    self.s_row_text(
                        glyph,
                        Face::ui(11.5),
                        b.x + 7.0,
                        b.y,
                        b.h,
                        p.chip_text,
                        clip,
                    );
                    self.s_hit(b, clip, Hit::Settings(action));
                }
                bx -= 28.0;
            }
            if i + 1 < n {
                self.s_fill(Rect::new(c.x, c.y + 35.0, c.w, 1.0), clip, p.line);
            }
            c.y += 36.0;
        }
    }

    /// A card's ground in `bg`, edged in `edge`: 2 points just outside it
    /// when `strong`, else 1 inside.
    pub(super) fn card_ground(&mut self, r: Rect, clip: Rect, bg: u32, edge: u32, strong: bool) {
        let (o, radius, w) = if strong {
            (
                Rect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0),
                10.0,
                2.0,
            )
        } else {
            (r, 8.0, 1.0)
        };
        if o.intersect(clip) == o {
            self.canvas.outline(o, radius, w, edge, bg);
        } else {
            self.s_fill(r, clip, bg);
        }
    }

    /// A theme's card: a few lines of code in its own colours, and its
    /// name.
    pub(super) fn theme_card(
        &mut self,
        card: &ThemeCard,
        r: Rect,
        keyed: bool,
        follow: bool,
        clip: Rect,
    ) {
        let p = self.p;
        let t = card.palette;
        let pv = Rect::new(r.x, r.y, r.w, 62.0);
        let label = Rect::new(r.x, r.y + 62.0, r.w, 28.0);
        let hovered = self.s_hovered(r, clip);
        let bg = if hovered { p.hover } else { p.widget };
        let (edge, strong) = if card.mark.is_some() {
            (p.accent, true)
        } else if keyed {
            (mix(p.accent, p.island, 0.55), true)
        } else {
            (p.border, false)
        };
        self.card_ground(r, clip, bg, edge, strong);
        self.s_fill(pv.inset(1.0), clip, t.island);
        self.s_fill(
            Rect::new(pv.x + 1.0, pv.y + 1.0, 7.0, pv.h - 1.0),
            clip,
            t.ground,
        );
        let code = Face::mono(9.5);
        let lines: [&[(&str, u32)]; 3] = [
            &[
                ("fn", t.keyword),
                (" ", t.text),
                ("main", t.function),
                ("() {", t.text),
            ],
            &[
                ("    let", t.keyword),
                (" s = ", t.text),
                ("\"hi\"", t.string),
                (";", t.text),
            ],
            &[
                ("    Args", t.type_),
                ("::", t.operator),
                ("parse", t.function),
                ("()", t.text),
            ],
        ];
        let inner = pv.inset(1.0).intersect(clip);
        for (n, parts) in lines.iter().enumerate() {
            let mut x = pv.x + 18.0;
            let y = pv.y + 8.0 + n as f32 * 15.0;
            for (text, colour) in *parts {
                x += self
                    .text
                    .draw(&mut self.canvas, text, code, x, y, *colour, inner, None);
            }
        }
        let face = Face::ui(12.0);
        let name = self.text.fit(&card.label, face, r.w - 70.0);
        self.s_row_text(&name, face, label.x + 9.0, label.y, label.h, p.text, clip);
        let (tag, colour, f) = match card.mark {
            Some(mark) => (mark.to_string(), p.accent, Face::ui(10.5).bold()),
            None => (
                if card.light { "Light" } else { "Dark" }.to_string(),
                p.muted,
                Face::ui(10.5),
            ),
        };
        // Without Follow the system, Dark and Light only label the cards.
        let tag = if card.mark.is_none() && !follow && !card.light {
            String::new()
        } else {
            tag
        };
        let tw = self.text.width(&tag, f);
        self.s_row_text(
            &tag,
            f,
            label.right() - 9.0 - tw,
            label.y,
            label.h,
            colour,
            clip,
        );
        self.s_hit(
            r,
            clip,
            Hit::Settings(SettingsAction::Theme(card.label.clone())),
        );
    }
}
