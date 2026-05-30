//! Portapapeles (arboard) + helper de color transparente.
#![allow(unused_imports)]
use crate::prelude::*;
use crate::*;
use crate::actions::*;
use crate::fsutil::*;
use crate::view::*;
use crate::session::*;
use crate::clipboard::*;
use crate::keys::*;
use crate::update::*;
pub(crate) struct ArboardClipboard {
    inner: Option<arboard::Clipboard>,
}

impl ArboardClipboard {
    fn new() -> Self {
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

/// `Color::transparent()` para fills "vacíos" sin importar tema — quedaba
/// huérfano de un branch viejo, lo dejamos por si surge un placeholder
/// que lo necesite.
#[allow(dead_code)]
pub(crate) fn transparent() -> Color {
    Color::TRANSPARENT
}
