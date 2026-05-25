//! Exportación del tema a los toolkits del escritorio (GTK 3 y 4).
//!
//! `nahual` pinta sus apps con GPUI, pero el escritorio carmen también
//! corre apps GTK y Qt. Para que no desentonen, este módulo traduce el
//! [`Theme`] activo a archivos `gtk.css` con overrides `@define-color`:
//! GTK adopta el acento y el neutro claro/oscuro del tema. Las apps Qt
//! se enganchan vía `QT_QPA_PLATFORMTHEME=gtk3` (lo inyecta el
//! compositor a cada hijo), así que siguen a GTK sin config aparte.
//!
//! La paleta de `nahual` usa gradientes (`Background`) que GTK no puede
//! reproducir en una ventana sólida; por eso el neutro GTK se **deriva**
//! de `is_dark` + los slots `Hsla` accesibles (`accent`, `fg_text`,
//! `border`). El acento sí se traslada exacto.
//!
//! [`Theme::set`](super::Theme::set) y
//! [`Theme::install_default`](super::Theme::install_default) llaman a
//! [`export_toolkit_configs`] best-effort.

use std::io;
use std::path::{Path, PathBuf};

use gpui::{hsla, Hsla, Rgba};

use super::{config_home, Theme};

/// Marca en la cabecera de los archivos generados. Si un `gtk.css` ya
/// existe **sin** esta marca, lo dejamos intacto: es del usuario.
const MARKER: &str = "generado por nahual-theme";

/// Resultado de [`export_toolkit_configs`]: qué archivos se (re)escribieron
/// y cuáles se respetaron por ser ajenos al control de nahual.
#[derive(Debug, Default, Clone)]
pub struct ExportReport {
    /// Archivos `gtk.css` (re)escritos por nahual.
    pub written: Vec<PathBuf>,
    /// Archivos saltados: existían y no llevan la marca de nahual, así
    /// que son del usuario y no se tocan.
    pub skipped: Vec<PathBuf>,
}

/// Escribe `gtk-3.0/gtk.css` y `gtk-4.0/gtk.css` bajo el directorio de
/// config del usuario, derivados del `theme`. Best-effort: el caller
/// (`Theme::set`) ignora el resultado. No pisa un `gtk.css` ajeno —
/// ver [`ExportReport::skipped`].
pub fn export_toolkit_configs(theme: &Theme) -> io::Result<ExportReport> {
    let base = config_home().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no se pudo determinar config dir (HOME/XDG_CONFIG_HOME no set)",
        )
    })?;
    export_toolkit_configs_to(&base, theme)
}

/// Variante de [`export_toolkit_configs`] con directorio base de config
/// explícito. Útil para tests y para apps con su propio config root.
pub fn export_toolkit_configs_to(base: &Path, theme: &Theme) -> io::Result<ExportReport> {
    let targets = [
        (base.join("gtk-3.0").join("gtk.css"), gtk3_css(theme)),
        (base.join("gtk-4.0").join("gtk.css"), gtk4_css(theme)),
    ];
    let mut report = ExportReport::default();
    for (path, content) in targets {
        if is_foreign(&path) {
            report.skipped.push(path);
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        report.written.push(path);
    }
    Ok(report)
}

/// `true` si el archivo existe y NO lleva la marca de nahual — es un
/// `gtk.css` propio del usuario y no debemos pisarlo. Si no existe (o no
/// se puede leer), `false`: campo libre para escribir.
fn is_foreign(path: &Path) -> bool {
    match std::fs::read_to_string(path) {
        Ok(existing) => !existing.contains(MARKER),
        Err(_) => false,
    }
}

// ============================================================================
// Generación de CSS
// ============================================================================

/// CSS para `~/.config/gtk-4.0/gtk.css` — nombres de color de libadwaita.
pub fn gtk4_css(theme: &Theme) -> String {
    let p = gtk_palette(theme);
    let mut s = header(theme, "GTK 4 / libadwaita");
    define(&mut s, "accent_color", &p.accent);
    define(&mut s, "accent_bg_color", &p.accent);
    define(&mut s, "accent_fg_color", &p.accent_fg);
    define(&mut s, "window_bg_color", &p.window_bg);
    define(&mut s, "window_fg_color", &p.window_fg);
    define(&mut s, "view_bg_color", &p.view_bg);
    define(&mut s, "view_fg_color", &p.window_fg);
    define(&mut s, "headerbar_bg_color", &p.headerbar_bg);
    define(&mut s, "headerbar_fg_color", &p.window_fg);
    define(&mut s, "card_bg_color", &p.card_bg);
    define(&mut s, "card_fg_color", &p.window_fg);
    define(&mut s, "popover_bg_color", &p.popover_bg);
    define(&mut s, "popover_fg_color", &p.window_fg);
    define(&mut s, "dialog_bg_color", &p.dialog_bg);
    define(&mut s, "dialog_fg_color", &p.window_fg);
    define(&mut s, "sidebar_bg_color", &p.sidebar_bg);
    define(&mut s, "sidebar_fg_color", &p.window_fg);
    define(&mut s, "destructive_color", &p.destructive);
    define(&mut s, "destructive_bg_color", &p.destructive);
    define(&mut s, "destructive_fg_color", &p.accent_fg);
    define(&mut s, "borders", &p.border);
    s
}

/// CSS para `~/.config/gtk-3.0/gtk.css` — nombres de color de Adwaita 3.
pub fn gtk3_css(theme: &Theme) -> String {
    let p = gtk_palette(theme);
    let mut s = header(theme, "GTK 3");
    define(&mut s, "theme_bg_color", &p.window_bg);
    define(&mut s, "theme_fg_color", &p.window_fg);
    define(&mut s, "theme_base_color", &p.view_bg);
    define(&mut s, "theme_text_color", &p.window_fg);
    define(&mut s, "theme_selected_bg_color", &p.accent);
    define(&mut s, "theme_selected_fg_color", &p.accent_fg);
    define(&mut s, "theme_tooltip_bg_color", &p.popover_bg);
    define(&mut s, "theme_tooltip_fg_color", &p.window_fg);
    define(&mut s, "insensitive_bg_color", &p.window_bg);
    define(&mut s, "insensitive_fg_color", &p.fg_disabled);
    define(&mut s, "borders", &p.border);
    define(&mut s, "warning_color", &p.destructive);
    define(&mut s, "error_color", &p.destructive);
    s
}

/// Cabecera común: lleva la [`MARKER`] (la guarda de no-pisar) y avisa
/// que el archivo es regenerado.
fn header(theme: &Theme, toolkit: &str) -> String {
    format!(
        "/* {MARKER} · tema «{}» · {toolkit} */\n\
         /* Lo reescribe nahual al cambiar de tema. Para usar tu propio\n\
         \x20  gtk.css, borra esta cabecera y nahual respetará el archivo. */\n\n",
        theme.name
    )
}

/// Agrega una línea `@define-color <name> <value>;`.
fn define(out: &mut String, name: &str, value: &str) {
    out.push_str("@define-color ");
    out.push_str(name);
    out.push(' ');
    out.push_str(value);
    out.push_str(";\n");
}

/// Paleta intermedia: los colores GTK ya resueltos a hex `#rrggbb`.
struct GtkPalette {
    accent: String,
    accent_fg: String,
    window_bg: String,
    window_fg: String,
    view_bg: String,
    headerbar_bg: String,
    card_bg: String,
    popover_bg: String,
    sidebar_bg: String,
    dialog_bg: String,
    fg_disabled: String,
    border: String,
    destructive: String,
}

/// Deriva la paleta GTK del `Theme`. El acento y los foregrounds son
/// directos; los fondos neutros se sintetizan con un ramp de luminancia
/// (GTK pinta ventanas sólidas, no gradientes), tintado con el matiz del
/// borde del tema para que conserve su "temperatura".
fn gtk_palette(t: &Theme) -> GtkPalette {
    let nh = t.border.h;
    let ns = (t.border.s * 0.7).min(0.5);
    let neutral = |l: f32| hex(hsla(nh, ns, l, 1.0));

    // (window, view, headerbar, card, popover, sidebar, dialog)
    let lum = if t.is_dark {
        [0.11, 0.07, 0.14, 0.15, 0.17, 0.09, 0.13]
    } else {
        [0.95, 0.99, 0.90, 0.98, 0.99, 0.93, 0.96]
    };

    GtkPalette {
        accent: hex(t.accent),
        accent_fg: hex(contrast_fg(t.accent)),
        window_bg: neutral(lum[0]),
        window_fg: hex(t.fg_text),
        view_bg: neutral(lum[1]),
        headerbar_bg: neutral(lum[2]),
        card_bg: neutral(lum[3]),
        popover_bg: neutral(lum[4]),
        sidebar_bg: neutral(lum[5]),
        dialog_bg: neutral(lum[6]),
        fg_disabled: hex(t.fg_disabled),
        border: hex(t.border),
        destructive: hex(t.accent_destructive()),
    }
}

/// Color de texto legible sobre `bg`: casi-negro si el fondo es claro,
/// casi-blanco si es oscuro. Decide por luma perceptual.
fn contrast_fg(bg: Hsla) -> Hsla {
    let rgba: Rgba = bg.into();
    let luma = 0.299 * rgba.r + 0.587 * rgba.g + 0.114 * rgba.b;
    if luma > 0.55 {
        hsla(0.0, 0.0, 0.08, 1.0)
    } else {
        hsla(0.0, 0.0, 0.98, 1.0)
    }
}

/// `Hsla` → `#rrggbb`. Descarta el alfa (GTK lo maneja aparte).
fn hex(c: Hsla) -> String {
    let rgba: Rgba = c.into();
    let q = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(rgba.r), q(rgba.g), q(rgba.b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_of_pure_colors() {
        assert_eq!(hex(hsla(0.0, 0.0, 0.0, 1.0)), "#000000");
        assert_eq!(hex(hsla(0.0, 0.0, 1.0, 1.0)), "#ffffff");
        assert_eq!(hex(hsla(0.0, 1.0, 0.5, 1.0)), "#ff0000");
    }

    #[test]
    fn gtk4_css_carries_marker_and_accent() {
        let css = gtk4_css(&Theme::nebula());
        assert!(css.contains(MARKER), "falta la marca de no-pisar");
        assert!(css.contains("@define-color accent_color #"));
        assert!(css.contains("@define-color window_bg_color #"));
    }

    #[test]
    fn gtk3_css_carries_selection() {
        let css = gtk3_css(&Theme::aurora());
        assert!(css.contains(MARKER));
        assert!(css.contains("@define-color theme_selected_bg_color #"));
    }

    #[test]
    fn every_preset_yields_well_formed_hex() {
        for theme in Theme::all() {
            let p = gtk_palette(&theme);
            for color in [&p.accent, &p.window_bg, &p.view_bg, &p.border] {
                assert_eq!(color.len(), 7, "{}: hex mal formado: {color}", theme.name);
                assert!(color.starts_with('#'), "{}: sin #: {color}", theme.name);
                assert!(
                    color[1..].chars().all(|c| c.is_ascii_hexdigit()),
                    "{}: dígitos no-hex: {color}",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn light_theme_has_light_window_bg() {
        // Solarized Light es claro: su window_bg debe ser luminoso.
        let p = gtk_palette(&Theme::solarized_light());
        let v = u8::from_str_radix(&p.window_bg[1..3], 16).unwrap();
        assert!(
            v > 200,
            "fondo de tema claro demasiado oscuro: {}",
            p.window_bg
        );
    }

    #[test]
    fn dark_theme_has_dark_window_bg() {
        let p = gtk_palette(&Theme::nebula());
        let v = u8::from_str_radix(&p.window_bg[1..3], 16).unwrap();
        assert!(
            v < 60,
            "fondo de tema oscuro demasiado claro: {}",
            p.window_bg
        );
    }

    #[test]
    fn export_writes_both_and_respects_foreign() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "nahual-toolkit-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        let gtk3 = dir.join("gtk-3.0").join("gtk.css");
        let gtk4 = dir.join("gtk-4.0").join("gtk.css");

        // Primera exportación: crea los dos archivos.
        let r1 = export_toolkit_configs_to(&dir, &Theme::nebula()).unwrap();
        assert_eq!(r1.written.len(), 2);
        assert!(r1.skipped.is_empty());
        assert!(gtk3.exists() && gtk4.exists());

        // Re-exportar otro tema: nuestros archivos llevan la marca, se
        // reescriben sin problema.
        let r2 = export_toolkit_configs_to(&dir, &Theme::aurora()).unwrap();
        assert_eq!(r2.written.len(), 2);
        assert!(std::fs::read_to_string(&gtk4).unwrap().contains("Aurora"));

        // Un `gtk.css` ajeno (sin marca) se respeta: no se pisa.
        std::fs::write(&gtk3, "/* css del usuario */\n").unwrap();
        let r3 = export_toolkit_configs_to(&dir, &Theme::sunset()).unwrap();
        assert_eq!(r3.written, vec![gtk4.clone()]);
        assert_eq!(r3.skipped, vec![gtk3.clone()]);
        assert_eq!(
            std::fs::read_to_string(&gtk3).unwrap(),
            "/* css del usuario */\n"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_foreign_detects_user_file() {
        let mut path = std::env::temp_dir();
        path.push(format!("nahual-toolkit-test-{}.css", std::process::id()));

        // Archivo ajeno (sin marca) → foreign.
        std::fs::write(&path, "/* mi css */\n").unwrap();
        assert!(is_foreign(&path));

        // Archivo nuestro (con marca) → no foreign.
        std::fs::write(&path, gtk4_css(&Theme::nebula())).unwrap();
        assert!(!is_foreign(&path));

        let _ = std::fs::remove_file(&path);

        // Archivo inexistente → no foreign (campo libre).
        assert!(!is_foreign(&path));
    }
}
