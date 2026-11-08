//! Documents a plugin draws, such as PDFs: a file of a type its
//! `[[editors]]` opens shows as a tab of pages. The editor owns scrolling
//! and zoom and keeps the tiles the plugin drew; the plugin draws what
//! comes into view, and says where its words are. Those words are the
//! tab's text, so Find, selecting, Copy and Select All work as anywhere.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use super::*;
use crate::PluginImage;

/// Logical pixels a point is drawn as at 100%: 96 to an inch, as CSS.
pub const PX_PER_PT: f32 = 96.0 / 72.0;
/// Room between pages, and above the first and below the last.
pub const PAGE_GAP: f32 = 16.0;
/// A tile's side, in physical pixels.
pub const TILE: u32 = 512;
/// A thumbnail's width, in physical pixels.
pub const THUMB: u32 = 128;
/// What one document's tiles may take, as RGBA, before the oldest go.
const BUDGET: usize = 64 << 20;
/// The zooms Zoom In and Zoom Out step through.
const ZOOMS: [f32; 14] = [
    0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0,
];

/// A file type a plugin opens, as its manifest's `[[editors]]` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocEditor {
    pub plugin: String,
    pub id: String,
    pub title: String,
    /// Lowercase, without the dot.
    pub extensions: Vec<String>,
}

/// A page's size in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// Part of a page drawn at a scale: from (`x`, `y`), `width` by `height`
/// pixels, at `scale` thousandths of a pixel a point. A thumbnail is the
/// whole page at a small scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub page: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
}

impl TileKey {
    /// Pixels a point.
    pub fn scale(&self) -> f32 {
        self.scale as f32 / 1000.0
    }
}

/// Where a word is on a page, in points from its top left.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub text: String,
}

/// A document's tiles, the oldest first, within its budget.
#[derive(Debug, Default)]
pub struct Tiles {
    map: HashMap<TileKey, Arc<PluginImage>>,
    order: VecDeque<TileKey>,
    bytes: usize,
    version: u64,
}

impl PartialEq for Tiles {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

impl Tiles {
    pub fn get(&self, key: &TileKey) -> Option<&Arc<PluginImage>> {
        self.map.get(key)
    }

    /// The tiles of a page, at any scale.
    pub fn of_page(&self, page: u32) -> impl Iterator<Item = (&TileKey, &Arc<PluginImage>)> {
        self.map.iter().filter(move |(k, _)| k.page == page)
    }

    /// Bytes the tiles take.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Keeps a tile, dropping the oldest past the budget. Thumbnails stay:
    /// they stand in while pages are drawn again.
    fn insert(&mut self, key: TileKey, image: Arc<PluginImage>) {
        self.bytes += image.pixels.len();
        if let Some(old) = self.map.insert(key, image) {
            self.bytes -= old.pixels.len();
        } else {
            self.order.push_back(key);
        }
        let mut kept = VecDeque::new();
        while self.bytes > BUDGET {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if oldest == key || is_thumb(&oldest) {
                kept.push_back(oldest);
                continue;
            }
            if let Some(gone) = self.map.remove(&oldest) {
                self.bytes -= gone.pixels.len();
            }
        }
        for k in kept.into_iter().rev() {
            self.order.push_front(k);
        }
        self.version += 1;
    }

    /// Drops all but the thumbnails, as when the tab is hidden.
    fn drop_pages(&mut self) {
        if self.order.iter().all(is_thumb) {
            return;
        }
        let map = &mut self.map;
        let bytes = &mut self.bytes;
        self.order.retain(|k| {
            if is_thumb(k) {
                return true;
            }
            if let Some(gone) = map.remove(k) {
                *bytes -= gone.pixels.len();
            }
            false
        });
        self.version += 1;
    }
}

/// A thumbnail: the whole page, from its corner, at most a thumbnail wide.
fn is_thumb(key: &TileKey) -> bool {
    key.x == 0 && key.y == 0 && key.width <= THUMB
}

/// Where a document is.
#[derive(Clone, Debug, PartialEq)]
pub enum Pages {
    /// The plugin is reading it.
    Opening,
    /// It couldn't, and why.
    Failed(String),
    Ready(Vec<PageSize>),
}

pub(crate) struct Surface {
    pub id: u32,
    /// The tab's document.
    pub doc: u64,
    pub plugin: String,
    /// What the file type is called: PDF.
    pub kind: String,
    pub pages: Pages,
    /// 1 is 100%. Fit Width sets it from the view's width.
    pub zoom: f32,
    pub fit: bool,
    /// Logical pixels from the top, and from the left when the pages are
    /// wider than their room.
    pub scroll: f32,
    pub hscroll: f32,
    pub thumbs: bool,
    /// The pages' room in logical pixels, as last drawn.
    pub area: (f32, f32),
    pub tiles: Arc<Tiles>,
    /// Each page's words, in order as they arrive, and where each word
    /// starts in the tab's text.
    pub words: Vec<Vec<TextBox>>,
    pub starts: Vec<Vec<usize>>,
    /// The cursor as last followed, so moving it shows it.
    followed: usize,
    /// The page the right-click menu was opened on.
    menu_page: u32,
}

/// A document tab, as the window draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceView {
    pub id: u32,
    pub plugin: String,
    /// The plugin's title.
    pub from: String,
    pub kind: String,
    pub pages: Pages,
    /// As drawn: Fit Width's, or the one chosen.
    pub zoom: f32,
    pub fit: bool,
    pub scroll: f32,
    pub hscroll: f32,
    pub thumbs: bool,
    /// The pages' room the editor knows of, to say when it changed.
    pub area: (f32, f32),
    /// The page most in view, from 0.
    pub current: usize,
    pub tiles: Arc<Tiles>,
    /// Pages whose words are known.
    pub words: usize,
    /// What's selected and what Find matched, as boxes in points on a
    /// page; and which match is the current one.
    pub selected: Vec<(u32, [f32; 4])>,
    pub hits: Vec<(u32, [f32; 4])>,
    pub hit: Option<usize>,
    pub focused: bool,
}

/// Where each page goes at `zoom`, in a room `width` wide: x, y, width
/// and height in logical pixels, centred, with a gap around each; and how
/// tall they are together.
pub fn page_layout(pages: &[PageSize], zoom: f32, width: f32) -> (Vec<[f32; 4]>, f32) {
    let mut y = PAGE_GAP;
    let rects = pages
        .iter()
        .map(|p| {
            let (w, h) = (p.width * PX_PER_PT * zoom, p.height * PX_PER_PT * zoom);
            let r = [((width - w) / 2.0).max(PAGE_GAP), y, w, h];
            y += h + PAGE_GAP;
            r
        })
        .collect();
    (rects, y)
}

/// The tab's page name for a document.
pub(crate) fn surface_page(id: u32) -> String {
    format!("doc:{id}")
}

impl Surface {
    /// The zoom pages are drawn at: Fit Width's, or the one chosen.
    pub fn zoom(&self) -> f32 {
        match &self.pages {
            Pages::Ready(pages) if self.fit && self.area.0 > 0.0 => {
                let widest = pages.iter().map(|p| p.width).fold(1.0, f32::max);
                ((self.area.0 - 2.0 * PAGE_GAP - 8.0) / (widest * PX_PER_PT)).clamp(0.1, 8.0)
            }
            _ => self.zoom,
        }
    }

    fn layout(&self) -> (Vec<[f32; 4]>, f32) {
        match &self.pages {
            Pages::Ready(pages) => page_layout(pages, self.zoom(), self.area.0),
            _ => (Vec::new(), 0.0),
        }
    }

    /// The page most in view: the one a third of the way down.
    fn current(&self) -> usize {
        let (rects, _) = self.layout();
        let mark = self.scroll + self.area.1 / 3.0;
        rects.iter().rposition(|r| r[1] <= mark).unwrap_or(0)
    }

    /// Keeps the scroll inside the pages, both ways.
    fn clamp(&mut self) {
        let (rects, total) = self.layout();
        self.scroll = self.scroll.clamp(0.0, (total - self.area.1).max(0.0));
        let wide = rects.iter().map(|r| r[0] + r[2]).fold(0.0, f32::max) + PAGE_GAP;
        self.hscroll = self.hscroll.clamp(0.0, (wide - self.area.0).max(0.0));
    }

    /// Changes the zoom, keeping the point a third of the way down where
    /// it is.
    fn set_zoom(&mut self, zoom: f32) {
        let before = self.zoom();
        let mark = self.scroll + self.area.1 / 3.0;
        self.fit = false;
        self.zoom = zoom.clamp(0.1, 8.0);
        let after = self.zoom();
        self.scroll = (mark - PAGE_GAP) * after / before + PAGE_GAP - self.area.1 / 3.0;
        self.clamp();
    }

    /// Scrolls so `page` starts at the top.
    fn go_to(&mut self, page: usize) {
        let (rects, _) = self.layout();
        if let Some(r) = rects.get(page) {
            self.scroll = r[1] - PAGE_GAP;
            self.clamp();
        }
    }

    /// The word holding character `at` of the tab's text: page and word.
    fn word_at(&self, at: usize) -> Option<(usize, usize)> {
        for (p, starts) in self.starts.iter().enumerate() {
            let i = starts.partition_point(|&s| s <= at);
            if i > 0 {
                let w = i - 1;
                let end = starts[w] + self.words[p][w].text.chars().count();
                if at <= end {
                    return Some((p, w));
                }
            }
        }
        None
    }

    /// The boxes the characters from `a` to `b` cover, each cut to the
    /// part of its word they take.
    fn boxes(&self, a: usize, b: usize) -> Vec<(u32, [f32; 4])> {
        let mut out: Vec<(u32, [f32; 4])> = Vec::new();
        for (p, starts) in self.starts.iter().enumerate() {
            for (w, &s) in starts.iter().enumerate() {
                let word = &self.words[p][w];
                let len = word.text.chars().count();
                let (from, to) = (a.max(s), b.min(s + len));
                if from >= to {
                    continue;
                }
                let per = word.width / len.max(1) as f32;
                let r = [
                    word.x + per * (from - s) as f32,
                    word.y,
                    per * (to - from) as f32,
                    word.height,
                ];
                // The word before on the line, also covered: one band
                // across both, and the space between.
                match out.last_mut() {
                    Some((lp, l))
                        if *lp == p as u32 && (l[1] - r[1]).abs() < 1.0 && r[0] > l[0] =>
                    {
                        l[2] = r[0] + r[2] - l[0];
                        l[3] = l[3].max(r[3]);
                    }
                    _ => out.push((p as u32, r)),
                }
            }
        }
        out
    }
}

impl Editor {
    /// Keeps the file types plugins open, replacing a plugin's own. Tabs
    /// already open on such a file, as when the workspace was restored
    /// before plugins loaded, become its document.
    pub(crate) fn register_editors(
        &mut self,
        plugin: &str,
        editors: Vec<DocEditor>,
        fx: &mut Vec<Effect>,
    ) {
        self.doc_editors.retain(|e| e.plugin != plugin);
        self.doc_editors.extend(editors);
        for i in 0..self.docs.len() {
            let doc = &self.docs[i];
            if doc.page.is_some() || doc.review.is_some() || doc.buffer.is_dirty() {
                continue;
            }
            let Some(path) = doc.buffer.path.clone() else {
                continue;
            };
            let Some(editor) = self.doc_editor_for(&path) else {
                continue;
            };
            self.make_surface(i, path, editor, fx);
        }
    }

    /// Whether a plugin opens `path` as a document, rather than as text.
    pub fn opens_as_document(&self, path: &std::path::Path) -> bool {
        self.doc_editor_for(path).is_some()
    }

    /// The plugin editor that opens `path`, by its extension.
    pub(crate) fn doc_editor_for(&self, path: &std::path::Path) -> Option<DocEditor> {
        let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
        self.doc_editors
            .iter()
            .find(|e| e.extensions.contains(&ext))
            .cloned()
    }

    /// Opens `path` as a document its plugin draws, in a tab of its own,
    /// or switches to it.
    pub(crate) fn open_surface(&mut self, path: PathBuf, editor: DocEditor, fx: &mut Vec<Effect>) {
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.buffer.path.as_ref() == Some(&path))
        {
            self.current = i;
            self.focus = Focus::Editor;
            return;
        }
        let id = self.new_id();
        let doc = Doc::new(id, "", Some(path.clone()));
        self.current = (self.current + 1).min(self.docs.len());
        self.docs.insert(self.current, doc);
        self.make_surface(self.current, path, editor, fx);
        self.focus = Focus::Editor;
        self.popup = None;
        self.sync_groups();
    }

    /// Makes the tab at `i` the document of `path`, which `editor` draws,
    /// and asks its plugin to read it.
    fn make_surface(&mut self, i: usize, path: PathBuf, editor: DocEditor, fx: &mut Vec<Effect>) {
        let id = self.docs[i].id;
        let surface = id as u32;
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let doc = &mut self.docs[i];
        doc.page = Some((surface_page(surface), name));
        doc.buffer = crate::buffer::Buffer::new("", Some(path.clone()));
        (doc.cursor, doc.anchor) = (0, None);
        self.surfaces.push(Surface {
            id: surface,
            doc: id,
            plugin: editor.plugin.clone(),
            kind: editor.title.clone(),
            pages: Pages::Opening,
            zoom: 1.0,
            fit: true,
            scroll: 0.0,
            hscroll: 0.0,
            thumbs: true,
            area: (0.0, 0.0),
            tiles: Arc::default(),
            words: Vec::new(),
            starts: Vec::new(),
            followed: 0,
            menu_page: 0,
        });
        fx.push(Effect::OpenDocument {
            plugin: editor.plugin,
            editor: editor.id,
            path,
            surface,
        });
    }

    fn surface_mut(&mut self, id: u32) -> Option<&mut Surface> {
        self.surfaces.iter_mut().find(|s| s.id == id)
    }

    /// The current tab's document, if it's one.
    pub(crate) fn current_surface(&self) -> Option<&Surface> {
        let id = self.doc().id;
        self.surfaces.iter().find(|s| s.doc == id)
    }

    fn current_surface_mut(&mut self) -> Option<&mut Surface> {
        let id = self.doc().id;
        self.surfaces.iter_mut().find(|s| s.doc == id)
    }

    /// The plugin read the document, or couldn't. Its words are asked
    /// for, a page at a time in order.
    pub(crate) fn document_opened(
        &mut self,
        id: u32,
        pages: Result<Vec<PageSize>, String>,
        fx: &mut Vec<Effect>,
    ) {
        let Some(s) = self.surface_mut(id) else {
            return;
        };
        match pages {
            Ok(pages) => {
                let n = pages.len() as u32;
                s.pages = Pages::Ready(pages);
                let plugin = s.plugin.clone();
                for page in 0..n {
                    fx.push(Effect::PageText {
                        plugin: plugin.clone(),
                        surface: id,
                        page,
                    });
                }
            }
            Err(why) => s.pages = Pages::Failed(why),
        }
    }

    /// A tile the plugin drew, or why it couldn't.
    pub(crate) fn document_tile(
        &mut self,
        id: u32,
        tile: TileKey,
        image: Result<Arc<PluginImage>, String>,
    ) {
        let Some(s) = self.surface_mut(id) else {
            return;
        };
        match image {
            Ok(image) => tiles_mut(&mut s.tiles).insert(tile, image),
            Err(why) => {
                let plugin = s.plugin.clone();
                self.log_line(
                    &plugin,
                    LogLevel::Warn,
                    &format!("Couldn't draw page {}: {why}", tile.page + 1),
                );
            }
        }
    }

    /// A page's words, which go on the tab's text: words on a line with a
    /// space between, lines with a line break, pages with a blank line.
    pub(crate) fn document_text(&mut self, id: u32, page: u32, boxes: Vec<TextBox>) {
        let Some(s) = self.surface_mut(id) else {
            return;
        };
        // In order: a page out of turn would move the words after it.
        if page as usize != s.words.len() {
            return;
        }
        let doc_id = s.doc;
        let mut text = String::new();
        if page > 0 {
            text.push_str("\n\n");
        }
        let Some(i) = self.docs.iter().position(|d| d.id == doc_id) else {
            return;
        };
        let mut at = self.docs[i].buffer.len() + text.chars().count();
        let mut starts = Vec::new();
        let mut last: Option<&TextBox> = None;
        for word in &boxes {
            if let Some(prev) = last {
                let gap = if (word.y - prev.y).abs() < prev.height * 0.5 {
                    " "
                } else {
                    "\n"
                };
                text.push_str(gap);
                at += 1;
            }
            starts.push(at);
            text.push_str(&word.text);
            at += word.text.chars().count();
            last = Some(word);
        }
        let doc = &mut self.docs[i];
        let mut whole = doc.buffer.text();
        whole.push_str(&text);
        let path = doc.buffer.path.clone();
        let (cursor, anchor) = (doc.cursor, doc.anchor);
        doc.buffer = crate::buffer::Buffer::new(&whole, path);
        doc.cursor = cursor;
        doc.anchor = anchor;
        let s = self.surface_mut(id).expect("found above");
        s.words.push(boxes);
        s.starts.push(starts);
    }

    /// The window says how much room a document's pages have.
    pub(crate) fn surface_area(&mut self, id: u32, w: f32, h: f32) {
        if let Some(s) = self.surface_mut(id) {
            s.area = (w, h);
            s.clamp();
        }
    }

    /// The wheel, or a key, scrolls a document: down, or across.
    pub(crate) fn surface_scroll(&mut self, id: u32, by: f32, across: bool) {
        if let Some(s) = self.surface_mut(id) {
            if across {
                s.hscroll += by;
            } else {
                s.scroll += by;
            }
            s.clamp();
        }
    }

    /// A button of a document's toolbar, or a thumbnail.
    pub(crate) fn surface_ui(&mut self, id: u32, what: SurfaceAction) {
        let Some(s) = self.surface_mut(id) else {
            return;
        };
        match what {
            SurfaceAction::Thumbs => s.thumbs = !s.thumbs,
            SurfaceAction::Page(page) => s.go_to(page),
            SurfaceAction::Previous => {
                let page = s.current().saturating_sub(1);
                s.go_to(page);
            }
            SurfaceAction::Next => {
                let page = s.current() + 1;
                s.go_to(page);
            }
            SurfaceAction::ZoomIn => {
                let z = s.zoom();
                let next = ZOOMS.iter().copied().find(|&n| n > z + 0.001).unwrap_or(z);
                s.set_zoom(next);
            }
            SurfaceAction::ZoomOut => {
                let z = s.zoom();
                let next = ZOOMS
                    .iter()
                    .rev()
                    .copied()
                    .find(|&n| n < z - 0.001)
                    .unwrap_or(z);
                s.set_zoom(next);
            }
            SurfaceAction::Fit => {
                s.fit = true;
                s.clamp();
            }
        }
        self.focus = Focus::Editor;
    }

    /// A press or drag in a document's pages, at a point on a page in
    /// points: selects from where it started to the nearest word.
    pub(crate) fn surface_point(&mut self, id: u32, page: u32, x: f32, y: f32, drag: bool) {
        let Some(s) = self.surfaces.iter().find(|s| s.id == id) else {
            return;
        };
        let Some(starts) = s.starts.get(page as usize) else {
            return;
        };
        let words = &s.words[page as usize];
        // The nearest word, then the character nearest across it.
        let near = words.iter().enumerate().min_by(|(_, a), (_, b)| {
            let d = |w: &TextBox| {
                let dx = (w.x - x).max(x - (w.x + w.width)).max(0.0);
                let dy = (w.y - y).max(y - (w.y + w.height)).max(0.0);
                dx * dx + dy * dy * 4.0
            };
            d(a).total_cmp(&d(b))
        });
        let Some((w, word)) = near else {
            return;
        };
        let len = word.text.chars().count();
        let part = ((x - word.x) / word.width.max(0.1) * len as f32).round();
        let at = starts[w] + (part.clamp(0.0, len as f32) as usize);
        self.focus = Focus::Editor;
        let doc = self.doc_mut();
        if drag {
            if doc.anchor.is_none() {
                doc.anchor = Some(doc.cursor);
            }
        } else {
            doc.anchor = None;
        }
        doc.cursor = at;
        if doc.anchor == Some(at) {
            doc.anchor = None;
        }
        if let Some(s) = self.current_surface_mut() {
            // Moved by the mouse, where it already shows.
            s.followed = at;
        }
    }

    /// Keys a document takes before the bindings: scrolling, paging,
    /// zoom, Copy and Select All. Returns whether it took the key.
    pub(crate) fn surface_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        if self.focus != Focus::Editor || self.current_surface().is_none() {
            return false;
        }
        // Its words can be copied and selected, but not changed.
        match key.code {
            KeyCode::Char('c' | 'C') if key.ctrl && !key.alt => {
                let doc = self.doc();
                if let Some((a, b)) = doc.selection() {
                    fx.push(Effect::Copy(doc.slice(a, b)));
                }
                return true;
            }
            KeyCode::Char('a' | 'A') if key.ctrl && !key.alt => {
                let doc = self.doc_mut();
                doc.anchor = Some(0);
                doc.cursor = doc.buffer.len();
                return true;
            }
            _ => {}
        }
        let Some(s) = self.current_surface_mut() else {
            return false;
        };
        let line = 40.0;
        let page = (s.area.1 - 60.0).max(line);
        match key.code {
            KeyCode::Down if !key.ctrl && !key.alt && !key.shift => s.scroll += line,
            KeyCode::Up if !key.ctrl && !key.alt && !key.shift => s.scroll -= line,
            KeyCode::Right if !key.ctrl && !key.alt && !key.shift => s.hscroll += line,
            KeyCode::Left if !key.ctrl && !key.alt && !key.shift => s.hscroll -= line,
            KeyCode::PageDown | KeyCode::Char(' ') if !key.ctrl && !key.alt => s.scroll += page,
            KeyCode::PageUp => s.scroll -= page,
            KeyCode::Home if key.ctrl => s.scroll = 0.0,
            KeyCode::End if key.ctrl => s.scroll = f32::MAX,
            KeyCode::Char('=' | '+') if key.ctrl => {
                let id = s.id;
                self.surface_ui(id, SurfaceAction::ZoomIn);
                return true;
            }
            KeyCode::Char('-') if key.ctrl => {
                let id = s.id;
                self.surface_ui(id, SurfaceAction::ZoomOut);
                return true;
            }
            KeyCode::Char('0') if key.ctrl => s.fit = true,
            _ => return false,
        }
        s.clamp();
        true
    }

    /// Shows the cursor's word when something moved it: Find, mostly.
    pub(crate) fn surface_follow(&mut self) {
        let cursor = self.doc().cursor;
        let Some(s) = self.current_surface_mut() else {
            return;
        };
        if s.followed == cursor {
            return;
        }
        s.followed = cursor;
        let Some((p, w)) = s.word_at(cursor) else {
            return;
        };
        let (rects, _) = s.layout();
        let zoom = s.zoom();
        let word = &s.words[p][w];
        let y = rects[p][1] + word.y * PX_PER_PT * zoom;
        if y < s.scroll || y > s.scroll + s.area.1 - 40.0 {
            s.scroll = y - s.area.1 / 3.0;
            s.clamp();
        }
    }

    /// The right-click menu on a document's `page`: Copy and Search the
    /// Project for the selection, plugins' items for the file, and Copy
    /// Page as Image.
    pub(crate) fn surface_menu(&mut self, id: u32, page: u32) {
        let Some(s) = self.surfaces.iter_mut().find(|s| s.id == id) else {
            return;
        };
        s.menu_page = page;
        let selected = self.doc().selection().is_some();
        let items = if selected {
            vec!["edit.copy", "surface.search"]
        } else {
            Vec::new()
        };
        self.context = Some(super::context::ContextMenu {
            items,
            selected: None,
            title: None,
        });
        let file = self
            .doc()
            .buffer
            .path
            .as_ref()
            .map(|p| p.display().to_string());
        self.add_plugin_items(MenuPlace::Editor, file);
        if let Some(menu) = &mut self.context {
            if !menu.items.is_empty() {
                menu.items.push("-");
            }
            menu.items.push("surface.copy_page");
        }
        self.focus = Focus::Menu;
    }

    /// Searches the project for what's selected, in the palette.
    pub(crate) fn surface_search(&mut self, fx: &mut Vec<Effect>) {
        let doc = self.doc();
        let Some(text) = doc
            .selection()
            .map(|(a, b)| doc.slice(a, b).replace('\n', " "))
        else {
            return;
        };
        self.open_search(super::search_all::Scope::Text, fx);
        if let Some(palette) = &mut self.palette {
            palette.query = format!("/{text}");
        }
        self.search_changed();
    }

    /// Asks the plugin for the page the menu was on, twice as sharp as at
    /// 100%, to copy as a picture.
    pub(crate) fn surface_copy_page(&mut self, fx: &mut Vec<Effect>) {
        let Some(s) = self.current_surface() else {
            return;
        };
        let Pages::Ready(pages) = &s.pages else {
            return;
        };
        let Some(size) = pages.get(s.menu_page as usize) else {
            return;
        };
        // Twice 100%, but no side past 4096 pixels.
        let scale = (2.0 * PX_PER_PT).min(4096.0 / size.width.max(size.height).max(1.0));
        fx.push(Effect::PageImage {
            plugin: s.plugin.clone(),
            surface: s.id,
            tile: TileKey {
                page: s.menu_page,
                x: 0,
                y: 0,
                width: (size.width * scale).round().max(1.0) as u32,
                height: (size.height * scale).round().max(1.0) as u32,
                scale: (scale * 1000.0).round() as u32,
            },
        });
    }

    /// Closes what tabs no longer show: a closed tab's document goes, and
    /// a hidden one drops its tiles but its thumbnails.
    pub(crate) fn surfaces_tidy(&mut self, fx: &mut Vec<Effect>) {
        let shown: Vec<u64> = self.groups_showing();
        let docs: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
        self.surfaces.retain(|s| {
            let open = docs.contains(&s.doc);
            if !open {
                fx.push(Effect::CloseDocument {
                    plugin: s.plugin.clone(),
                    surface: s.id,
                });
            }
            open
        });
        for s in &mut self.surfaces {
            if !shown.contains(&s.doc) && !s.tiles.is_empty() {
                tiles_mut(&mut s.tiles).drop_pages();
            }
        }
    }

    /// The documents groups show, by their ids.
    fn groups_showing(&self) -> Vec<u64> {
        self.groups
            .iter()
            .map(|g| g.active)
            .chain([self.doc().id])
            .collect()
    }

    /// The documents the window shows, for each group showing one.
    pub(crate) fn surface_views(&self) -> Vec<(String, SurfaceView)> {
        let shown = self.groups_showing();
        self.surfaces
            .iter()
            .filter(|s| shown.contains(&s.doc))
            .map(|s| {
                let doc = self.docs.iter().find(|d| d.id == s.doc);
                let selected = doc
                    .and_then(|d| d.selection())
                    .map(|(a, b)| s.boxes(a, b))
                    .unwrap_or_default();
                let current_doc = self.doc().id == s.doc;
                let (hits, hit) = match (current_doc, self.find.is_some(), self.query()) {
                    (true, true, Some(matcher)) => {
                        let matches = self.all_matches(&matcher);
                        // Find leaves the cursor at the match it went to.
                        let at = doc.map(|d| d.selection().map_or(d.cursor, |(a, _)| a));
                        let hit = at.and_then(|a| matches.iter().position(|m| m.0 == a));
                        let mut boxes = Vec::new();
                        let mut current = None;
                        for (i, &(m, len)) in matches.iter().enumerate() {
                            if Some(i) == hit {
                                current = Some(boxes.len());
                            }
                            boxes.extend(s.boxes(m, m + len));
                        }
                        (boxes, current)
                    }
                    _ => (Vec::new(), None),
                };
                let from = self
                    .plugin_list
                    .iter()
                    .find(|p| p.name == s.plugin)
                    .map_or(s.plugin.clone(), |p| p.title.clone());
                (
                    surface_page(s.id),
                    SurfaceView {
                        id: s.id,
                        plugin: s.plugin.clone(),
                        from,
                        kind: s.kind.clone(),
                        pages: s.pages.clone(),
                        zoom: s.zoom(),
                        fit: s.fit,
                        scroll: s.scroll,
                        hscroll: s.hscroll,
                        thumbs: s.thumbs,
                        area: s.area,
                        current: s.current(),
                        tiles: s.tiles.clone(),
                        words: s.words.len(),
                        selected,
                        hits,
                        hit,
                        focused: current_doc && self.focus == Focus::Editor,
                    },
                )
            })
            .collect()
    }

    /// What a plugin's pictures take: its views' pictures, and the pages
    /// of its documents drawn, for its page in Settings.
    pub fn picture_bytes(&self, plugin: &str) -> usize {
        let images: usize = self
            .images
            .iter()
            .filter(|((p, _), _)| p == plugin)
            .map(|(_, i)| i.pixels.len())
            .sum();
        let tiles: usize = self
            .surfaces
            .iter()
            .filter(|s| s.plugin == plugin)
            .map(|s| s.tiles.bytes())
            .sum();
        images + tiles
    }

    /// The status bar's words for the current document: its page, its
    /// zoom, and its type.
    pub(crate) fn surface_status(&self) -> Option<(String, String, String)> {
        let s = self.current_surface()?;
        let Pages::Ready(pages) = &s.pages else {
            return Some((String::new(), String::new(), s.kind.clone()));
        };
        Some((
            format!("Page {} of {}", s.current() + 1, pages.len()),
            format!("{}%", (s.zoom() * 100.0).round()),
            s.kind.clone(),
        ))
    }
}

/// What a click in a document's toolbar or thumbnails does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceAction {
    Thumbs,
    Page(usize),
    Previous,
    Next,
    ZoomIn,
    ZoomOut,
    Fit,
}

/// The tiles to change: the window only reads them while it draws, so
/// one it still holds is swapped for a copy of its map.
fn tiles_mut(tiles: &mut Arc<Tiles>) -> &mut Tiles {
    if Arc::get_mut(tiles).is_none() {
        *tiles = Arc::new(Tiles {
            map: tiles.map.clone(),
            order: tiles.order.clone(),
            bytes: tiles.bytes,
            version: tiles.version,
        });
    }
    Arc::get_mut(tiles).expect("just made unique")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_stack_centred_with_gaps() {
        let a4 = PageSize {
            width: 72.0,
            height: 144.0,
        };
        let (rects, total) = page_layout(&[a4, a4], 1.0, 400.0);
        assert_eq!(rects[0], [152.0, 16.0, 96.0, 192.0]);
        assert_eq!(rects[1][1], 16.0 + 192.0 + 16.0);
        assert_eq!(total, 16.0 + 2.0 * (192.0 + 16.0));
    }

    #[test]
    fn tiles_past_the_budget_drop_the_oldest_but_thumbnails() {
        let mut tiles = Tiles::default();
        let image = |n: usize| Arc::new(PluginImage::new(1, 1, vec![0; n]));
        let key = |page, x| TileKey {
            page,
            x,
            y: 0,
            width: TILE,
            height: TILE,
            scale: 1000,
        };
        let thumb = TileKey {
            page: 0,
            x: 0,
            y: 0,
            width: THUMB,
            height: 180,
            scale: 200,
        };
        tiles.insert(thumb, image(1000));
        tiles.insert(key(0, 0), image(BUDGET / 2));
        tiles.insert(key(0, 512), image(BUDGET / 2));
        tiles.insert(key(1, 0), image(BUDGET / 2));
        assert!(tiles.get(&thumb).is_some());
        assert!(tiles.get(&key(0, 0)).is_none());
        assert!(tiles.get(&key(1, 0)).is_some());
        assert!(tiles.bytes() <= BUDGET);
        tiles.drop_pages();
        assert_eq!(tiles.len(), 1);
    }
}
