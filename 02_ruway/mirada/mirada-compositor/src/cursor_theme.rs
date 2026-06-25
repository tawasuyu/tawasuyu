//! Temas de cursor XCursor — los "sets" de puntero tipo `Soberania`.
//!
//! Hasta ahora mirada pintaba un cuadrado sólido casi-blanco como cursor por
//! defecto (las apps que publican su propia superficie de cursor sí se veían
//! bien; el puntero del escritorio/cliente *con nombre* era el cuadrado). Este
//! módulo carga un tema XCursor configurado (`cursor_theme` en `config.ron`) y
//! lo materializa en búferes que el backend DRM sube como textura, igual que el
//! wallpaper o las etiquetas de título.
//!
//! Resolución del tema:
//!   1. Se siembra `XCURSOR_PATH` con los directorios donde mirada trae sus sets
//!      embebidos (`Soberania`, `Soberania-Light`) — así funcionan sin instalar.
//!   2. `xcursor::CursorTheme::load` busca el tema en `XCURSOR_PATH` + los
//!      directorios de íconos XDG (`~/.local/share/icons`, `/usr/share/icons`…),
//!      de modo que **cualquier** tema XCursor instalado también sirve como set.
//!
//! Si el tema no aparece, no tiene el ícono pedido, o el archivo no parsea, el
//! llamador cae al cuadrado de software de siempre.

use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::Transform;

/// Un cursor del tema ya rasterizado, listo para subir como textura.
pub(crate) struct LoadedCursor {
    /// Búfer RGBA (premultiplicado, BGRA en memoria — `Argb8888`).
    pub buffer: MemoryRenderBuffer,
    /// Punto activo (en px del búfer): dónde "pincha" el puntero.
    pub hotspot: (i32, i32),
    /// Tamaño real del búfer en px (ancho, alto).
    pub size: (i32, i32),
}

/// Caché de cursores de un tema XCursor. Resuelve por nombre CSS
/// (`default`, `pointer`, `text`, `ns-resize`…) bajo demanda y memoiza el
/// resultado (incluido el "no se pudo", para no reintentar cada cuadro).
pub(crate) struct CursorTheme {
    name: String,
    size: u32,
    cache: HashMap<String, Option<LoadedCursor>>,
}

impl CursorTheme {
    /// Crea el cargador para `name` con tamaño nominal `size`. Siembra
    /// `XCURSOR_PATH` con los sets embebidos la primera vez.
    pub fn new(name: impl Into<String>, size: u32) -> Self {
        seed_xcursor_path();
        Self {
            name: name.into(),
            size: size.max(1),
            cache: HashMap::new(),
        }
    }

    /// `true` si hay un tema configurado (nombre no vacío). Con `false` el
    /// backend pinta el cuadrado por defecto sin siquiera consultar el tema.
    pub fn is_active(&self) -> bool {
        !self.name.is_empty()
    }

    /// Cursor para la lista de nombres `names` (preferencia descendente: el
    /// nombre CSS, sus alias, y por último `default`). `None` si nada resuelve.
    pub fn get(&mut self, names: &[&str]) -> Option<&LoadedCursor> {
        let key = names.first().copied().unwrap_or("default").to_string();
        if !self.cache.contains_key(&key) {
            let loaded = self.load(names);
            self.cache.insert(key.clone(), loaded);
        }
        self.cache.get(&key).and_then(|o| o.as_ref())
    }

    /// Carga efectiva: localiza el archivo del tema, parsea, elige la imagen de
    /// tamaño más cercano y arma el búfer BGRA premultiplicado.
    fn load(&self, names: &[&str]) -> Option<LoadedCursor> {
        let theme = xcursor::CursorTheme::load(&self.name);
        let path = names.iter().find_map(|n| theme.load_icon(n))?;
        let bytes = std::fs::read(path).ok()?;
        let images = xcursor::parser::parse_xcursor(&bytes)?;
        // Cuadro nominal más cercano al pedido (un .cur trae varios tamaños).
        let img = images
            .iter()
            .min_by_key(|i| (i.size as i32 - self.size as i32).abs())?;
        if img.width == 0 || img.height == 0 {
            return None;
        }
        // `pixels_rgba` viene premultiplicado (lo exige el spec XCursor). Para
        // `Argb8888` la memoria es B,G,R,A (u32 little-endian 0xAARRGGBB).
        let mut bgra = Vec::with_capacity(img.pixels_rgba.len());
        for px in img.pixels_rgba.chunks_exact(4) {
            bgra.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
        let buffer = MemoryRenderBuffer::from_slice(
            &bgra,
            Fourcc::Argb8888,
            (img.width as i32, img.height as i32),
            1,
            Transform::Normal,
            None,
        );
        Some(LoadedCursor {
            buffer,
            hotspot: (img.xhot as i32, img.yhot as i32),
            size: (img.width as i32, img.height as i32),
        })
    }
}

/// Antepone a `XCURSOR_PATH` los directorios donde viven los sets embebidos de
/// mirada, de modo que `Soberania`/`Soberania-Light` se encuentren aunque el
/// usuario no haya corrido el instalador. Idempotente: no duplica entradas.
fn seed_xcursor_path() {
    let candidates = [
        // Árbol de fuentes (corridas `cargo run` en desarrollo).
        concat!(env!("CARGO_MANIFEST_DIR"), "/cursors").to_string(),
        // Instalación empaquetada.
        "/usr/share/mirada/cursors".to_string(),
    ];
    let existing = std::env::var("XCURSOR_PATH").unwrap_or_default();
    let mut parts: Vec<String> = existing
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let mut changed = false;
    for c in candidates {
        if std::path::Path::new(&c).is_dir() && !parts.iter().any(|p| p == &c) {
            parts.insert(0, c);
            changed = true;
        }
    }
    if changed {
        // SAFETY: se siembra una sola vez al construir el tema, antes de que el
        // backend arranque su hilo de render; no hay carrera con otros lectores.
        unsafe { std::env::set_var("XCURSOR_PATH", parts.join(":")) };
    }
}

/// Lista de nombres a probar para un `CursorIcon`, en orden de preferencia:
/// el nombre CSS canónico, sus alias legados (X11) y `default` como último
/// recurso. Así un tema que sólo trae `default`/`pointer`/`text` igual cubre
/// peticiones de cursores de resize, dnd, etc.
pub(crate) fn icon_names(icon: smithay::input::pointer::CursorIcon) -> Vec<&'static str> {
    let mut v = vec![icon.name()];
    v.extend_from_slice(icon.alt_names());
    if !v.contains(&"default") {
        v.push("default");
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El set embebido `Soberania` se localiza desde el árbol de fuentes y al
    /// menos `default` y `pointer` parsean a un búfer con hotspot y tamaño
    /// plausibles. Certifica el cargador sin GPU ni pantalla (texto, no PNG).
    #[test]
    fn soberania_embebida_carga_default_y_pointer() {
        let mut theme = CursorTheme::new("Soberania", 24);
        assert!(theme.is_active());
        for name in ["default", "pointer", "text"] {
            let c = theme
                .get(&[name])
                .unwrap_or_else(|| panic!("falta el cursor «{name}» en Soberania"));
            assert!(c.size.0 > 0 && c.size.1 > 0, "«{name}» con tamaño nulo");
            assert!(
                c.hotspot.0 >= 0 && c.hotspot.0 <= c.size.0,
                "hotspot.x de «{name}» fuera del búfer"
            );
            assert!(
                c.hotspot.1 >= 0 && c.hotspot.1 <= c.size.1,
                "hotspot.y de «{name}» fuera del búfer"
            );
        }
    }

    /// Un tema inexistente no rompe: `get` devuelve `None` y el backend cae al
    /// cuadrado.
    #[test]
    fn tema_inexistente_no_rompe() {
        let mut theme = CursorTheme::new("no-existe-este-tema-xyz", 24);
        assert!(theme.get(&["default"]).is_none());
    }
}
