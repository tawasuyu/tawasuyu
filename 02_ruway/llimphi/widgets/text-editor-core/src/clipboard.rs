//! Clipboard abstracto. El editor no quiere acoplarse a un backend de
//! SO concreto (X11 / Wayland / macOS / Windows), así que define el
//! trait y entrega un mock para tests. La impl real (vía `arboard`)
//! vive del lado del caller — típicamente la app embebida en
//! `nada` o el visor del notebook.

/// Backend de clipboard. `set` mete texto; `get` lo lee. Cualquiera de
/// los dos puede fallar (sin display, headless CI, race con otro
/// programa) — `None` / no-op silencioso es válido.
pub trait Clipboard: Send {
    fn get(&mut self) -> Option<String>;
    fn set(&mut self, s: &str);
}

/// Clipboard de memoria — útil para tests y como fallback cuando el
/// sistema no expone uno.
#[derive(Debug, Default, Clone)]
pub struct MemClipboard {
    content: Option<String>,
}

impl MemClipboard {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with(s: impl Into<String>) -> Self {
        Self { content: Some(s.into()) }
    }
}

impl Clipboard for MemClipboard {
    fn get(&mut self) -> Option<String> {
        self.content.clone()
    }
    fn set(&mut self, s: &str) {
        self.content = Some(s.to_owned());
    }
}

/// "No clipboard" — `set` descarta, `get` devuelve `None`. Útil cuando
/// el caller quiere desactivar copy/paste explícitamente.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullClipboard;

impl Clipboard for NullClipboard {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _: &str) {}
}
