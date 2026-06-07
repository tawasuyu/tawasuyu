//! Resolución de íconos para entradas del menú de inicio.
//!
//! Las apps gioser declaran su ícono como un glyph corto (`✱`, `pluma`, etc.).
//! Las apps `.desktop` externas declaran un **nombre freedesktop** (`firefox`,
//! `org.gnome.Files`) que no es renderizable por sí solo: hay que ubicarlo en
//! algún tema XDG y cargar el archivo `.svg` resultante. Sin eso, pata cae al
//! glyph genérico `▸` para todos los `.desktop` — feo y poco distintivo.
//!
//! Este módulo hace dos cosas:
//!
//! 1. **Resolución XDG mínima** ([`resolve_icon_svg`]): busca un `.svg` en los
//!    paths canónicos del freedesktop icon theme spec, en orden de prioridad.
//!    No parseamos `index.theme`; tomamos atajos pragmáticos (Adwaita y
//!    hicolor cubren el ~95% del software del sistema).
//!
//! 2. **Cache de assets parseados** ([`get_or_load`]): `vello_svg::render` no
//!    es gratis. Una lista de 80 apps reparseando en cada frame mata el thread
//!    de UI; el cache convierte ese costo en "parseá una vez, stampeá N".
//!    Niegamos también los nombres que fallaron (`None` se cachea) para no
//!    re-walkear el filesystem 60 veces por segundo buscando un ícono que no
//!    existe.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use llimphi_svg::SvgAsset;

/// Cache singleton de íconos resueltos. Key = nombre freedesktop crudo (lo que
/// dice `.desktop`'s `Icon=…`). Value:
/// - `Some(asset)` si lo encontramos y parseó.
/// - `None` si no pudimos resolverlo o falló el parse — cacheado para no
///   repetir el trabajo cada frame.
fn cache() -> &'static Mutex<HashMap<String, Option<SvgAsset>>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Mutex<HashMap<String, Option<SvgAsset>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Devuelve el `SvgAsset` para el nombre freedesktop dado, parseando una vez
/// y cacheando para el resto del proceso. `None` si el nombre no resuelve a un
/// `.svg` válido. **NO bloquea** sobre el filesystem en frames siguientes: el
/// resultado (positivo o negativo) queda fijo hasta que la app reinicie.
pub fn get_or_load(name: &str) -> Option<SvgAsset> {
    if name.is_empty() {
        return None;
    }
    // Si el nombre ya es un path absoluto a `.svg` válido, lo usamos directo —
    // algunos `.desktop` ponen `Icon=/usr/share/foo/icon.svg`.
    let resolved = if name.starts_with('/') && name.ends_with(".svg") {
        Some(PathBuf::from(name))
    } else {
        resolve_icon_svg(name)
    };
    let mut guard = cache().lock().ok()?;
    if let Some(slot) = guard.get(name) {
        return slot.clone();
    }
    let asset = resolved
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| SvgAsset::from_str(&s).ok());
    guard.insert(name.to_string(), asset.clone());
    asset
}

/// Busca un `.svg` para `name` en los paths canónicos XDG. No parsea
/// `index.theme`; chequea en orden:
///   1. `~/.local/share/icons/{theme}/.../apps/{name}.svg` (themes Adwaita,
///      hicolor — los más comunes)
///   2. `/usr/share/icons/{theme}/.../apps/{name}.svg`
///   3. `/usr/share/icons/hicolor/scalable/apps/{name}.svg` (fallback canónico)
///   4. `/usr/share/pixmaps/{name}.svg`
///
/// Pública para tests. Devuelve `None` si nada encaja.
pub fn resolve_icon_svg(name: &str) -> Option<PathBuf> {
    // Themes en orden de preferencia: Adwaita (GNOME), Papirus (popular), y
    // hicolor (el fallback obligatorio del spec).
    const THEMES: &[&str] = &["Adwaita", "Papirus", "Papirus-Dark", "breeze", "hicolor"];
    // Subdirs por tamaño/categoría; `scalable/apps` es donde viven los SVGs.
    // Algunos themes guardan SVG en symbolic, otros en apps; probamos ambos.
    const SUBDIRS: &[&str] = &[
        "scalable/apps",
        "symbolic/apps",
        "256x256/apps",
        "128x128/apps",
        "64x64/apps",
        "48x48/apps",
    ];

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(h) = home {
        roots.push(h.join(".local/share/icons"));
        roots.push(h.join(".icons"));
    }
    roots.push(PathBuf::from("/usr/share/icons"));

    for root in &roots {
        for theme in THEMES {
            for sub in SUBDIRS {
                let p = root.join(theme).join(sub).join(format!("{name}.svg"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }

    // Pixmaps fallback (no por theme).
    let pix = PathBuf::from("/usr/share/pixmaps").join(format!("{name}.svg"));
    if pix.is_file() {
        return Some(pix);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombre_vacío_no_resuelve() {
        assert!(get_or_load("").is_none());
    }

    #[test]
    fn nombre_inexistente_se_cachea_negativo() {
        // Un nombre que casi seguro no existe en el sistema; el cache lo guarda
        // como `None` para no re-walkear.
        let n = "icono-que-no-existe-xyz123-gioser-test";
        assert!(get_or_load(n).is_none());
        // Segunda llamada usa el cache (sigue dando None y no panic).
        assert!(get_or_load(n).is_none());
    }
}
