//! Permisos de capacidad — config declarativa de qué ejecutable puede usar
//! cada global Wayland sensible.
//!
//! Mismo patrón que [`crate::rules`]: RON de texto en
//! `~/.config/mirada/caps.ron`, que el [`Desktop`](crate::Desktop) carga al
//! arrancar y empuja al Cuerpo como [`BrainCommand::SetCapabilities`]. El Cuerpo
//! es quien **otorga el protocolo**: una capacidad denegada no se concede por
//! una tabla eludible sino **no anunciando el global** al cliente.
//!
//! Gatea cinco capacidades: el snoop del portapapeles (`zwlr_data_control`),
//! la inyección de pulsaciones (`zwp_virtual_keyboard`), el censo de ventanas
//! (`ext_foreign_toplevel_list`), la captura de pantalla (`zwlr_screencopy`) y
//! el atajo de búferes de GPU zero-copy (`zwp_linux_dmabuf`).
//! La identidad del cliente es su **ejecutable real**
//! (`SO_PEERCRED → /proc/<pid>/exe`), no su `app_id` (falsificable).
//! Postura: **permitir por defecto**, con una denylist de ejecutables.
//!
//! El tipo de datos en sí ([`Permisos`]) vive en `mirada-protocol` porque cruza
//! el cable Cerebro↔Cuerpo; este módulo sólo aporta la carga desde disco.

use std::path::{Path, PathBuf};

pub use mirada_protocol::Permisos;

/// Parsea los permisos desde el texto RON de un archivo de config.
pub fn from_ron(text: &str) -> Result<Permisos, String> {
    ron::from_str(text).map_err(|e| format!("RON inválido: {e}"))
}

/// La ruta canónica de los permisos: `~/.config/mirada/caps.ron`.
pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "mirada").map(|d| d.config_dir().join("caps.ron"))
}

/// Carga los permisos de un archivo RON.
pub fn load(path: &Path) -> Result<Permisos, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("E/S: {e}"))?;
    from_ron(&text)
}

/// Vigila el archivo de permisos para recargarlo en caliente.
pub fn watch(path: &Path) -> notify::Result<crate::watch::FileWatch> {
    crate::watch::FileWatch::new(path)
}

/// Carga los permisos del usuario con un fallback amable: si el archivo no
/// existe, escribe una plantilla documentada y devuelve permisos vacíos (todo
/// permitido); si está corrupto, avisa y devuelve vacíos.
pub fn load_or_default(path: &Path) -> Permisos {
    if path.exists() {
        match load(path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "mirada · permisos «{}» inválidos ({e}); los ignoro (todo permitido).",
                    path.display()
                );
                Permisos::default()
            }
        }
    } else {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(path, PERMISOS_TEMPLATE) {
            Ok(()) => eprintln!("mirada · plantilla de permisos escrita en {}", path.display()),
            Err(e) => eprintln!("mirada · no pude escribir la plantilla de permisos: {e}"),
        }
        Permisos::default()
    }
}

/// La plantilla que se escribe la primera vez — sin denegaciones, con ejemplos
/// comentados para que el usuario los descubra.
const PERMISOS_TEMPLATE: &str = "\
// Permisos de capacidad de mirada — qué ejecutable puede usar cada global
// Wayland sensible. El compositor es quien OTORGA el protocolo: lo denegado no
// se anuncia al cliente (frontera física, no tabla eludible).
//
// La identidad del cliente es su ejecutable real (vía SO_PEERCRED), no su
// app_id (falsificable). Postura: permitir por defecto.
//
// `clipboard_denylist`: ejecutables a los que se NIEGA `zwlr_data_control` (el
// snoop del portapapeles). Casa por subcadena del path del ejecutable, sin
// distinguir mayúsculas. Vacía = todos permitidos.
//
// `virtual_input_denylist`: ejecutables a los que se NIEGA `zwp_virtual_keyboard`
// (inyectar pulsaciones sintéticas). Misma semántica de subcadena. Vacía = todos
// permitidos.
//
// `window_list_denylist`: ejecutables a los que se NIEGA
// `ext_foreign_toplevel_list` (el censo de ventanas: título + app_id de todo lo
// abierto). Misma semántica de subcadena. Vacía = todos permitidos.
//
// `screencopy_denylist`: ejecutables a los que se NIEGA `zwlr_screencopy`
// (capturar los píxeles de la pantalla — la capacidad más sensible). Misma
// semántica de subcadena. Vacía = todos permitidos.
//
// `dmabuf_denylist`: ejecutables a los que se NIEGA `zwp_linux_dmabuf` (importar
// búferes de GPU compartidos, zero-copy). Negarlo no rompe la app: cae al camino
// `wl_shm` por software, sólo pierde el atajo. Misma semántica de subcadena.
// Vacía = todos permitidos.
//
// Descomenta y edita:
(
    clipboard_denylist: [
        // \"wl-paste\",
        // \"/opt/sospechoso/bin/spyware\",
    ],
    virtual_input_denylist: [
        // \"wtype\",
        // \"/opt/sospechoso/bin/autoclicker\",
    ],
    window_list_denylist: [
        // \"lswt\",
        // \"/opt/sospechoso/bin/vigia\",
    ],
    screencopy_denylist: [
        // \"/opt/sospechoso/bin/captor\",
    ],
    dmabuf_denylist: [
        // \"/opt/sospechoso/bin/leak\",
    ],
)
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_parses_to_empty_permits_all() {
        let p = from_ron(PERMISOS_TEMPLATE).unwrap();
        assert!(p.clipboard_denylist.is_empty());
        assert!(p.virtual_input_denylist.is_empty());
        assert!(p.window_list_denylist.is_empty());
        assert!(p.screencopy_denylist.is_empty());
        assert!(p.dmabuf_denylist.is_empty());
        assert!(p.clipboard_permitido("/usr/bin/wl-paste"));
        assert!(p.virtual_input_permitido("/usr/bin/wtype"));
        assert!(p.window_list_permitido("/usr/bin/lswt"));
        assert!(p.screencopy_permitido("/usr/bin/grim"));
        assert!(p.dmabuf_permitido("/usr/bin/firefox"));
    }

    #[test]
    fn permisos_parse_from_ron() {
        let ron = r#"( clipboard_denylist: ["wl-paste", "spyware"], virtual_input_denylist: ["wtype"], window_list_denylist: ["lswt"] )"#;
        let p = from_ron(ron).unwrap();
        assert_eq!(p.clipboard_denylist.len(), 2);
        assert!(!p.clipboard_permitido("/usr/bin/wl-paste"));
        assert!(p.clipboard_permitido("/usr/bin/firefox"));
        assert!(!p.virtual_input_permitido("/usr/bin/wtype"));
        assert!(p.virtual_input_permitido("/usr/bin/firefox"));
        assert!(!p.window_list_permitido("/usr/bin/lswt"));
        assert!(p.window_list_permitido("/usr/bin/firefox"));
        assert!(p.screencopy_permitido("/usr/bin/grim"));
    }

    #[test]
    fn campos_ausentes_caen_a_vacio() {
        // Una config vieja (sólo clipboard) sigue parseando: los campos nuevos
        // caen a vacío por `#[serde(default)]`.
        let p = from_ron(r#"( clipboard_denylist: ["wl-paste"] )"#).unwrap();
        assert!(p.virtual_input_denylist.is_empty());
        assert!(p.virtual_input_permitido("/usr/bin/wtype"));
        assert!(p.window_list_denylist.is_empty());
        assert!(p.window_list_permitido("/usr/bin/lswt"));
        assert!(p.screencopy_denylist.is_empty());
        assert!(p.screencopy_permitido("/usr/bin/grim"));
        assert!(p.dmabuf_denylist.is_empty());
        assert!(p.dmabuf_permitido("/usr/bin/firefox"));
    }
}
