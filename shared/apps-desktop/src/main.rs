//! `tawasuyu-apps-desktop` — genera los `.desktop` + íconos SVG de la suite.
//!
//! Hasta ahora las apps de tawasuyu existían como binarios pero eran
//! **invisibles al escritorio**: no había `.desktop` ni íconos instalados. Este
//! tool cierra ese hueco: itera el catálogo de [`app_bus::default_entries`] y,
//! por cada app, escribe en el layout freedesktop:
//!
//! - `icons/hicolor/scalable/apps/tawasuyu-<id>.svg` — el icono de marca,
//!   exportado de `llimphi_icons::app_icons::app_icon_svg` (vectorial, escala a
//!   cualquier tamaño; un solo asset sirve para todos los DPI).
//! - `applications/tawasuyu-<id>.desktop` — el lanzador (Name/Exec/Icon/
//!   Categories/MimeType/StartupWMClass).
//!
//! Por defecto escribe en `$XDG_DATA_HOME` (o `~/.local/share`). Con
//! `--prefix <dir>` escribe a un staging (para instalación de sistema). Tras
//! escribir, refresca la base de datos de escritorio si el binario está.
//!
//! ```text
//! tawasuyu-apps-desktop                 # instala al usuario (~/.local/share)
//! tawasuyu-apps-desktop --prefix /tmp/x # staging
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use app_bus::{default_entries, AppEntry, Launch};
use llimphi_icons::app_icons::{app_icon_svg, AppIcon, ALL};

fn main() {
    let mut base: Option<PathBuf> = None;
    let mut svg_dir: Option<PathBuf> = None;
    let mut rust_iconset: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--prefix" => base = args.next().map(PathBuf::from),
            // Vuelca todos los AppIcon como `<name>.svg` planos en <dir> y sale
            // (para la web u otros consumidores de assets sueltos).
            "--svg-dir" => svg_dir = args.next().map(PathBuf::from),
            // Emite un módulo Rust cero-dep con los SVG embebidos (para consumidores
            // que no pueden depender de llimphi-icons, p.ej. el compositor mirada).
            "--rust-iconset" => rust_iconset = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                eprintln!("uso: tawasuyu-apps-desktop [--prefix <data-dir>] [--svg-dir <dir>] [--rust-iconset <file>]");
                return;
            }
            otro => {
                eprintln!("argumento desconocido: {otro}");
                std::process::exit(2);
            }
        }
    }

    if let Some(dir) = svg_dir {
        volcar_svgs_planos(&dir);
        return;
    }
    if let Some(file) = rust_iconset {
        emitir_rust_iconset(&file);
        return;
    }

    let data_dir = base.unwrap_or_else(default_data_dir);
    let apps_dir = data_dir.join("applications");
    let icons_dir = data_dir.join("icons/hicolor/scalable/apps");
    if let Err(e) = std::fs::create_dir_all(&apps_dir).and(std::fs::create_dir_all(&icons_dir)) {
        eprintln!("no pude crear los directorios de datos: {e}");
        std::process::exit(1);
    }

    let mut n_desktop = 0usize;
    let mut n_icon = 0usize;
    for entry in default_entries() {
        // Solo apps ejecutables (las WASM no tienen lanzador de escritorio).
        let Launch::Exec { program, .. } = &entry.launch else { continue };

        let icon_name = format!("tawasuyu-{}", entry.id);
        // Ícono: AppIcon de marca si resuelve; si no, sin línea Icon.
        let icon_field = match resolver_app_icon(&entry.id) {
            Some(icon) => {
                let svg = app_icon_svg(icon, 1.8);
                let ruta = icons_dir.join(format!("{icon_name}.svg"));
                if let Err(e) = std::fs::write(&ruta, svg) {
                    eprintln!("  ! {}: no pude escribir el icono: {e}", entry.id);
                    None
                } else {
                    n_icon += 1;
                    Some(icon_name.clone())
                }
            }
            None => {
                eprintln!("  · {} sin AppIcon de marca — .desktop sin icono", entry.id);
                None
            }
        };

        let desktop = desktop_entry(&entry, program, icon_field.as_deref());
        let ruta = apps_dir.join(format!("tawasuyu-{}.desktop", entry.id));
        match std::fs::write(&ruta, desktop) {
            Ok(()) => {
                n_desktop += 1;
                println!("ok  {} → {}", entry.id, ruta.display());
            }
            Err(e) => eprintln!("  ! {}: no pude escribir el .desktop: {e}", entry.id),
        }
    }

    println!("{n_desktop} .desktop y {n_icon} íconos en {}", data_dir.display());
    refrescar_db(&apps_dir);
}

/// Vuelca los 29 `AppIcon` como `<name>.svg` planos en `dir` (la web los
/// referencia por `<img src="assets/icons/<name>.svg">`).
fn volcar_svgs_planos(dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("no pude crear {}: {e}", dir.display());
        std::process::exit(1);
    }
    let mut n = 0usize;
    for icon in ALL {
        let ruta = dir.join(format!("{}.svg", icon.name()));
        match std::fs::write(&ruta, app_icon_svg(icon, 1.8)) {
            Ok(()) => n += 1,
            Err(e) => eprintln!("  ! {}: {e}", icon.name()),
        }
    }
    println!("{n} íconos SVG en {}", dir.display());
}

/// Emite un módulo Rust cero-dependencias con los 29 SVG embebidos, para
/// consumidores que no pueden depender de llimphi-icons (el compositor mirada
/// rasteriza estos SVG con resvg). Generado — no editar a mano.
fn emitir_rust_iconset(file: &Path) {
    let mut s = String::new();
    s.push_str("//! Generado por `tawasuyu-apps-desktop --rust-iconset`. NO editar a mano.\n");
    s.push_str("//!\n//! SVG de los íconos de marca de cada app, embebidos como strings. Crate\n");
    s.push_str("//! cero-dependencias: lo consume quien no puede depender de llimphi-icons\n");
    s.push_str("//! (p.ej. el compositor mirada, que rasteriza el SVG con resvg).\n\n");
    s.push_str("/// SVG del ícono de marca de la app `name` (== `AppIcon::name`), o `None`.\n");
    s.push_str("pub fn svg(name: &str) -> Option<&'static str> {\n    Some(match name {\n");
    for icon in ALL {
        let svg = app_icon_svg(icon, 1.8);
        // `r##"…"##`: los colores `stroke="#rrggbb"` meten la secuencia `"#`, que
        // cerraría un `r#"…"#`; con dos almohadillas no hay colisión (`"##` no
        // aparece en un SVG).
        s.push_str(&format!("        {:?} => r##\"{}\"##,\n", icon.name(), svg.trim_end()));
    }
    s.push_str("        _ => return None,\n    })\n}\n");
    if let Err(e) = std::fs::write(file, s) {
        eprintln!("no pude escribir {}: {e}", file.display());
        std::process::exit(1);
    }
    println!("iconset Rust ({} apps) → {}", ALL.len(), file.display());
}

/// `$XDG_DATA_HOME` o `~/.local/share`.
fn default_data_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share")
}

/// Resuelve el `AppIcon` de marca para un id de app-bus, con alias para los ids
/// compuestos que no matchean directo (`pluma-notebook`→Pluma, etc.).
fn resolver_app_icon(id: &str) -> Option<AppIcon> {
    if let Some(icon) = AppIcon::from_app_id(id) {
        return Some(icon);
    }
    let alias = match id {
        "pluma-notebook" => "pluma",
        "media-tube" => "media",
        "sandokan-monitor" => "sandokan",
        "mirada-panel" => "mirada",
        "panel-control" => "wawa",
        _ => return None, // hapiy, raymi: sin AppIcon propio aún
    };
    AppIcon::from_app_id(alias)
}

/// Categorías freedesktop a partir de los mimes que la app abre + su cuadrante.
fn categorias(entry: &AppEntry) -> String {
    let mut cats: Vec<&str> = Vec::new();
    if entry.handles.iter().any(|h| h.starts_with("image/")) {
        cats.push("Graphics");
    }
    if entry.handles.iter().any(|h| h.starts_with("audio/") || h.starts_with("video/")) {
        cats.push("AudioVideo");
    }
    if entry.handles.iter().any(|h| h.starts_with("text/") || h.contains("json") || h.contains("toml")) {
        cats.push("Utility");
        cats.push("TextEditor");
    }
    if cats.is_empty() {
        cats.push("Utility");
    }
    // Categoría propia (prefijo `X-` para extensiones, como pide freedesktop):
    // agrupa la suite en menús que la respeten.
    cats.push("X-Tawasuyu");
    let mut s = cats.join(";");
    s.push(';');
    s
}

/// Mimes válidos para `MimeType=` (full types; descarta prefijos como `text/`
/// que no son tipos MIME válidos en un `.desktop`).
fn mimes(entry: &AppEntry) -> Option<String> {
    let full: Vec<&str> = entry
        .handles
        .iter()
        .filter(|h| !h.ends_with('/'))
        .map(|s| s.as_str())
        .collect();
    if full.is_empty() {
        None
    } else {
        let mut s = full.join(";");
        s.push(';');
        Some(s)
    }
}

fn desktop_entry(entry: &AppEntry, program: &str, icon: Option<&str>) -> String {
    let abre_archivos = !entry.handles.is_empty();
    // %U si abre archivos/URLs; sin campo si es una app sin documentos.
    let exec = if abre_archivos {
        format!("{program} %U")
    } else {
        program.to_string()
    };
    let mut s = String::from("[Desktop Entry]\n");
    s.push_str("Type=Application\n");
    s.push_str(&format!("Name={}\n", entry.label));
    s.push_str(&format!("Exec={exec}\n"));
    if let Some(icon) = icon {
        s.push_str(&format!("Icon={icon}\n"));
    }
    s.push_str("Terminal=false\n");
    s.push_str(&format!("Categories={}\n", categorias(entry)));
    if let Some(m) = mimes(entry) {
        s.push_str(&format!("MimeType={m}\n"));
    }
    // Ayuda al compositor a casar la ventana con este .desktop (taskicons).
    s.push_str(&format!("StartupWMClass={program}\n"));
    s
}

/// Refresca la base de datos de aplicaciones si la herramienta está instalada
/// (no es fatal si falta — los .desktop ya quedaron en disco).
fn refrescar_db(apps_dir: &Path) {
    let _ = Command::new("update-desktop-database").arg(apps_dir).status();
}
