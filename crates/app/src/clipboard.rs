//! The system clipboard, and the selection clipboard that a middle click
//! pastes on Linux, through the window's own connection to the desktop:
//! Wayland's or X11's, as any desktop app has them.

use copypasta::ClipboardProvider;
use copypasta::wayland_clipboard::create_clipboards_from_external;
use copypasta::x11_clipboard::{Primary, X11ClipboardContext};
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

pub struct Clipboard {
    clipboard: Box<dyn ClipboardProvider>,
    selection: Option<Box<dyn ClipboardProvider>>,
}

impl Clipboard {
    /// The clipboards of the desktop `window` is on. It must be dropped
    /// before the window: on Wayland it shares its connection.
    pub fn for_window(window: &impl HasDisplayHandle) -> Option<Self> {
        match window.display_handle().ok()?.as_raw() {
            RawDisplayHandle::Wayland(handle) => {
                // SAFETY: the pointer is the window's live Wayland display,
                // and the clipboard is dropped before the window is.
                let (selection, clipboard) =
                    unsafe { create_clipboards_from_external(handle.display.as_ptr()) };
                Some(Self {
                    clipboard: Box::new(clipboard),
                    selection: Some(Box::new(selection)),
                })
            }
            RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => Some(Self {
                clipboard: Box::new(
                    X11ClipboardContext::<copypasta::x11_clipboard::Clipboard>::new().ok()?,
                ),
                selection: X11ClipboardContext::<Primary>::new()
                    .ok()
                    .map(|c| Box::new(c) as Box<dyn ClipboardProvider>),
            }),
            _ => None,
        }
    }

    pub fn copy(&mut self, text: &str) {
        let _ = self.clipboard.set_contents(text.to_owned());
    }

    /// The clipboard's text, or `None` if it's empty or not text.
    pub fn paste(&mut self) -> Option<String> {
        self.clipboard.get_contents().ok().filter(|t| !t.is_empty())
    }

    /// Makes `text` the selection, for a middle click anywhere to paste.
    pub fn select(&mut self, text: &str) {
        if let Some(selection) = &mut self.selection {
            let _ = selection.set_contents(text.to_owned());
        }
    }

    /// The selection's text: what's selected in whichever app.
    pub fn selection(&mut self) -> Option<String> {
        self.selection
            .as_mut()?
            .get_contents()
            .ok()
            .filter(|t| !t.is_empty())
    }
}
