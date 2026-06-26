//! Puente al portapapeles del sistema vía `arboard`.

use llimphi_widget_text_editor::Clipboard;

/// Wrapper sobre `arboard::Clipboard`. Si el sistema no expone uno
/// (headless CI, sin Wayland/X), `inner` queda en `None` y los métodos
/// son no-op silenciosos — exactamente la semántica documentada del
/// trait [`Clipboard`].
pub(crate) struct ArboardClipboard {
    inner: Option<arboard::Clipboard>,
}

impl ArboardClipboard {
    pub(crate) fn new() -> Self {
        Self {
            inner: arboard::Clipboard::new().ok(),
        }
    }
}

impl Clipboard for ArboardClipboard {
    fn get(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }
    fn set(&mut self, s: &str) {
        if let Some(c) = self.inner.as_mut() {
            let _ = c.set_text(s.to_owned());
        }
    }
}
