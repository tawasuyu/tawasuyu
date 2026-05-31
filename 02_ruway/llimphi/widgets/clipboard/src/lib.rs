//! `llimphi-clipboard` — el portapapeles del sistema para apps Llimphi.
//!
//! El `text-editor` define el trait [`Clipboard`] pero deja el backend al
//! caller (no quiere acoplarse a X11/Wayland/macOS/Windows). Este crate
//! aporta el backend obvio — [`arboard`] — para que cualquier app lo
//! enchufe en una línea:
//!
//! ```ignore
//! let mut clip = llimphi_clipboard::SystemClipboard::new();
//! editor.apply_key_with_clipboard(&ev, &mut clip);
//! ```
//!
//! Si no hay display (CI headless, sesión sin servidor gráfico) degrada
//! a no-op silencioso: `get` devuelve `None`, `set` descarta. Nunca
//! panica.

#![forbid(unsafe_code)]

use llimphi_widget_text_editor::Clipboard;

/// Portapapeles del sistema vía `arboard`. `None` interno = no se pudo
/// abrir (sin display); en ese caso opera como [`llimphi_widget_text_editor::NullClipboard`].
pub struct SystemClipboard {
    inner: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    pub fn new() -> Self {
        Self {
            inner: arboard::Clipboard::new().ok(),
        }
    }

    /// `true` si el backend del SO está disponible.
    pub fn is_available(&self) -> bool {
        self.inner.is_some()
    }
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipboard for SystemClipboard {
    fn get(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }
    fn set(&mut self, s: &str) {
        if let Some(c) = self.inner.as_mut() {
            let _ = c.set_text(s.to_owned());
        }
    }
}
