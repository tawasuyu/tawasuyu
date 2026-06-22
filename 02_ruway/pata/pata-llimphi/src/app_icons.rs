//! Resolución de íconos para entradas del menú de inicio / dock / spotlight.
//!
//! Las apps tawasuyu declaran su ícono como un glyph corto (`✶`, `▶`, etc.).
//! Las apps `.desktop` externas declaran un **nombre freedesktop** (`firefox`,
//! `google-chrome`, `org.gnome.Files`) que no es renderizable por sí solo: hay
//! que ubicarlo en algún tema XDG y cargar el archivo. La mayoría del software
//! del sistema trae el ícono como **PNG** (no SVG), así que resolvemos ambos.
//! Sin eso, pata caía al glyph genérico `▸` para casi toda `.desktop` — feo y
//! poco distintivo.
//!
//! Este módulo hace dos cosas:
//!
//! 1. **Resolución XDG mínima** ([`resolve_icon_path`]): busca un `.svg` o
//!    `.png` en los paths canónicos del freedesktop icon theme spec, en orden
//!    de prioridad (escalable primero, luego tamaños grandes a chicos). No
//!    parseamos `index.theme`; tomamos atajos pragmáticos (Adwaita/Papirus/
//!    hicolor cubren el ~95% del software del sistema).
//!
//! 2. **Cache de assets parseados** ([`get_or_load`]): parsear SVG / decodificar
//!    PNG no es gratis. Una lista de 80 apps reparseando en cada frame mata el
//!    thread de UI; el cache convierte ese costo en "parseá una vez, stampeá N".
//!    Cacheamos también los nombres que fallaron (`None`) para no re-walkear el
//!    filesystem 60 veces por segundo buscando un ícono que no existe.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use llimphi_image::Image;
use llimphi_svg::SvgAsset;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::{ImageFit, View};

/// Un ícono de app ya resuelto y listo para pintar: vector (SVG) o raster
/// (PNG decodificado). Ambos son baratos de clonar (`Arc` internamente).
#[derive(Clone)]
pub enum AppIcon {
    Svg(SvgAsset),
    Raster(Image),
}

impl AppIcon {
    /// Una `View` que pinta el ícono, ajustado al contenedor (`Contain` para
    /// el raster — preserva el aspect ratio sin recortar).
    pub fn view<Msg: Clone + Send + Sync + 'static>(&self) -> View<Msg> {
        match self {
            AppIcon::Svg(a) => a.view::<Msg>(),
            AppIcon::Raster(img) => View::new(Style {
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .image(img.clone())
            .image_fit(ImageFit::Contain),
        }
    }
}

/// Cache singleton de íconos resueltos. Key = nombre freedesktop crudo (lo que
/// dice `.desktop`'s `Icon=…`). Value:
/// - `Some(icon)` si lo encontramos y parseó/decodificó.
/// - `None` si no pudimos resolverlo o falló el load — cacheado para no
///   repetir el trabajo cada frame.
fn cache() -> &'static Mutex<HashMap<String, Option<AppIcon>>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Mutex<HashMap<String, Option<AppIcon>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Tope de tamaño en disco de un ícono raster (defensa contra un PNG gigante
/// que reviente la RAM del thread de UI). 4 MiB sobra para cualquier ícono.
const MAX_ICON_BYTES: u64 = 4 * 1024 * 1024;

/// Devuelve el [`AppIcon`] para el nombre freedesktop dado, parseando/
/// decodificando una vez y cacheando para el resto del proceso. `None` si el
/// nombre no resuelve a un archivo válido. **NO bloquea** sobre el filesystem
/// en frames siguientes: el resultado (positivo o negativo) queda fijo hasta
/// que la app reinicie.
pub fn get_or_load(name: &str) -> Option<AppIcon> {
    if name.is_empty() {
        return None;
    }
    {
        let guard = cache().lock().ok()?;
        if let Some(slot) = guard.get(name) {
            return slot.clone();
        }
    }
    // Si el nombre ya es un path absoluto a un ícono válido, lo usamos directo —
    // algunos `.desktop` ponen `Icon=/usr/share/foo/icon.png`.
    let resolved = if name.starts_with('/') {
        let p = PathBuf::from(name);
        p.is_file().then_some(p)
    } else {
        resolve_icon_path(name)
    };
    let icon = resolved.and_then(|p| load_icon_file(&p));
    if let Ok(mut guard) = cache().lock() {
        guard.insert(name.to_string(), icon.clone());
    }
    icon
}

/// Carga un archivo de ícono según su extensión: SVG → [`SvgAsset`]; cualquier
/// otra cosa (png/jpg/…) → decode raster a `peniko::Image`.
fn load_icon_file(p: &Path) -> Option<AppIcon> {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "svg" || ext == "svgz" {
        let s = std::fs::read_to_string(p).ok()?;
        SvgAsset::from_str(&s).ok().map(AppIcon::Svg)
    } else {
        llimphi_image::load_path(p, MAX_ICON_BYTES).ok().map(AppIcon::Raster)
    }
}

/// Busca un ícono (`.svg` o `.png`) para `name` en los paths canónicos XDG. No
/// parsea `index.theme`; chequea themes en orden de preferencia × subdirs por
/// tamaño (escalable/grande→chico), probando `.svg` antes que `.png` dentro de
/// cada carpeta. Pública para tests. `None` si nada encaja.
pub fn resolve_icon_path(name: &str) -> Option<PathBuf> {
    // Themes en orden de preferencia: Adwaita (GNOME), Papirus (popular),
    // breeze (KDE) y hicolor (el fallback obligatorio del spec).
    const THEMES: &[&str] = &[
        "Adwaita",
        // Adwaita moderno dejó de traer muchos íconos fullcolor de app/categoría;
        // AdwaitaLegacy (instalado junto con Adwaita) los conserva en su contexto
        // `legacy/`. Lo buscamos para que resuelvan `applications-*` y similares.
        "AdwaitaLegacy",
        "Papirus",
        "Papirus-Dark",
        "breeze",
        "breeze-dark",
        "hicolor",
    ];
    // Subdirs por contexto × tamaño. `apps` primero (la mayoría de los íconos de
    // app); luego `categories` y `legacy` (íconos de categoría freedesktop como
    // `applications-multimedia`, que viven ahí, no en `apps`). SVG antes que
    // raster, y raster de grande a chico (mejor nitidez al escalar).
    const SUBDIRS: &[&str] = &[
        "scalable/apps",
        "512x512/apps",
        "256x256/apps",
        "128x128/apps",
        "96x96/apps",
        "64x64/apps",
        "48x48/apps",
        "symbolic/apps",
        "scalable/categories",
        "64x64/categories",
        "48x48/categories",
        "32x32/categories",
        "24x24/categories",
        "22x22/categories",
        "scalable/legacy",
        "48x48/legacy",
        "32x32/legacy",
        "24x24/legacy",
        "22x22/legacy",
    ];
    // Extensiones, en orden: vector primero (escala sin perder), luego raster.
    const EXTS: &[&str] = &["svg", "png"];

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(h) = home {
        roots.push(h.join(".local/share/icons"));
        roots.push(h.join(".icons"));
    }
    roots.push(PathBuf::from("/usr/share/icons"));
    roots.push(PathBuf::from("/usr/local/share/icons"));

    for root in &roots {
        for theme in THEMES {
            for sub in SUBDIRS {
                for ext in EXTS {
                    let p = root.join(theme).join(sub).join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }

    // Pixmaps fallback (no por theme): svg o png sueltos.
    for ext in EXTS {
        let pix = PathBuf::from("/usr/share/pixmaps").join(format!("{name}.{ext}"));
        if pix.is_file() {
            return Some(pix);
        }
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
        let n = "icono-que-no-existe-xyz123-tawasuyu-test";
        assert!(get_or_load(n).is_none());
        // Segunda llamada usa el cache (sigue dando None y no panic).
        assert!(get_or_load(n).is_none());
    }
}
