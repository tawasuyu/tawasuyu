//! Despacho a **nivel de Mónada** — el análogo del `viewer_registry` (hoja
//! → visor) pero un nivel arriba: una Mónada, según su lente, declara qué
//! **panel in-canvas** la pinta y con qué **app** se abre para editarla.
//!
//! Es la pieza de la "regla reformulada": nahual no reimplementa editores,
//! los **hospeda**. Una Mónada de imágenes se previsualiza en una galería
//! dentro del canvas y se *edita* abriendo tullpu; una de código se navega
//! como árbol de archivos y se *edita* en el editor; una de datos se ve
//! como tabla y se abre en nakui. El panel es in-process (mismo canvas); el
//! "abrir en…" sale por [`app_bus`] — la interfaz estilo AppBus con
//! implementación in-process primero (híbrido por capas).
//!
//! Dos mapeos, ambos data-driven y deterministas:
//!
//! - [`panel_for`]: lente → [`MonadPanel`] (qué se pinta *dentro* de nahual).
//! - [`default_app_for`]: lente → la app de la suite que *edita* esa
//!   naturaleza, resuelta por **id** contra el [`AppRegistry`]. Se rutea por
//!   id (no por mime) a propósito: los `handles` de app-bus usan prefijos
//!   (`text/`), así que un mime como `text/csv` caería en el editor de texto
//!   antes que en nakui por orden de label. El id expresa la intención sin
//!   ambigüedad. El registro de mimes (`handlers_for`) sigue disponible para
//!   un menú "Abrir con…" completo.

use app_bus::{AppEntry, AppRegistry};
use nahual_source_core::{Lens, ViewMode};

/// El panel que pinta una Mónada *dentro* del canvas de nahual. Es la vista
/// de la Mónada-como-unidad (no la de una hoja suelta): read-only, para
/// previsualizar/navegar; la edición real se delega a la app de dominio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonadPanel {
    /// Galería de miniaturas (reusa `nahual-gallery-llimphi`). Mónadas de
    /// imágenes.
    Gallery,
    /// Árbol/lista de archivos + preview — el explorador dual que el shell
    /// ya tiene. Mónadas de código y estructurales.
    Files,
    /// Tabla. Mónadas de datos (CSV/DB).
    Sheet,
    /// Documento renderizado. Mónadas de markdown/texto.
    Reader,
    /// Grilla de íconos genérica. Mónadas sin lente fuerte.
    Generic,
}

/// El panel in-canvas para una Mónada de este lente.
pub fn panel_for(lens: Lens) -> MonadPanel {
    match lens {
        Lens::Gallery => MonadPanel::Gallery,
        Lens::Code => MonadPanel::Files,
        Lens::Database => MonadPanel::Sheet,
        Lens::Markdown => MonadPanel::Reader,
        Lens::Tree => MonadPanel::Files,
        Lens::Grid => MonadPanel::Generic,
    }
}

/// El **id de app** (en el catálogo de `shared/app-bus`) que edita una
/// Mónada de este lente — la pieza "imágenes→tullpu, proyectos→editor,
/// datos→nakui". `None` para lentes sin editor de dominio natural (se
/// quedan en el panel in-canvas).
///
/// Hoy "código" abre `nada` (el editor de archivos/código de la suite);
/// cuando se registre un IDE dedicado, basta cambiar este id (o que el IDE
/// gane el ruteo por mime) sin tocar el resto.
pub fn default_app_id(lens: Lens) -> Option<&'static str> {
    match lens {
        Lens::Gallery => Some("tullpu"),
        Lens::Code => Some("nada"),
        Lens::Database => Some("nakui"),
        Lens::Markdown => Some("pluma"),
        Lens::Tree => Some("nada"),
        Lens::Grid => None,
    }
}

/// La [`AppEntry`] que edita una Mónada de este lente, resuelta contra el
/// registro. `None` si el lente no tiene editor natural o la app no está
/// registrada (el shell cae al panel in-canvas).
pub fn default_app_for<'a>(reg: &'a AppRegistry, lens: Lens) -> Option<&'a AppEntry> {
    default_app_id(lens).and_then(|id| reg.get(id))
}

/// El `ViewMode` por defecto de una Mónada, a partir del **mime-hint** con
/// que `nahual-source-core::lens_mime` la etiqueta (`monada/<lente>`). Es la
/// pieza "la Mónada declara su vista": al entrar a una Mónada el shell fija
/// la vista del panel a esto (galería para fotos, detalle para código/datos,
/// lista para texto). `None` = no es una Mónada con lente conocido → el
/// shell conserva la vista vigente.
///
/// Clave sobre el string que emite `lens_mime`; el test `view_mode_sigue_a_
/// lens_mime` ata ambos lados para que no driften.
pub fn view_mode_for_hint(hint: &str) -> Option<ViewMode> {
    Some(match hint {
        "monada/gallery" => ViewMode::Gallery,
        "monada/code" | "monada/database" => ViewMode::Details,
        "monada/markdown" | "monada/tree" => ViewMode::List,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_bus::{default_entries, AppRegistry};

    #[test]
    fn cada_lente_tiene_panel() {
        assert_eq!(panel_for(Lens::Gallery), MonadPanel::Gallery);
        assert_eq!(panel_for(Lens::Code), MonadPanel::Files);
        assert_eq!(panel_for(Lens::Database), MonadPanel::Sheet);
        assert_eq!(panel_for(Lens::Markdown), MonadPanel::Reader);
        assert_eq!(panel_for(Lens::Tree), MonadPanel::Files);
        assert_eq!(panel_for(Lens::Grid), MonadPanel::Generic);
    }

    #[test]
    fn open_with_rutea_al_especialista_correcto() {
        // Contra el catálogo por defecto de la suite, cada lente abre su app
        // de dominio — sin la ambigüedad del ruteo por prefijo de mime.
        let reg = AppRegistry::new(default_entries());
        assert_eq!(default_app_for(&reg, Lens::Gallery).map(|e| e.id.as_str()), Some("tullpu"));
        assert_eq!(default_app_for(&reg, Lens::Code).map(|e| e.id.as_str()), Some("nada"));
        assert_eq!(default_app_for(&reg, Lens::Database).map(|e| e.id.as_str()), Some("nakui"));
        assert_eq!(default_app_for(&reg, Lens::Markdown).map(|e| e.id.as_str()), Some("pluma"));
        // Grid no tiene editor de dominio: se queda en el panel in-canvas.
        assert_eq!(default_app_for(&reg, Lens::Grid), None);
    }

    #[test]
    fn view_mode_sigue_a_lens_mime() {
        use nahual_source_core::lens_mime;
        // Para cada lente con hint, el hint que emite source-core debe mapear a
        // un ViewMode acá: ata los dos lados para que no driften.
        for (lens, esperado) in [
            (Lens::Gallery, Some(ViewMode::Gallery)),
            (Lens::Code, Some(ViewMode::Details)),
            (Lens::Database, Some(ViewMode::Details)),
            (Lens::Markdown, Some(ViewMode::List)),
            (Lens::Tree, Some(ViewMode::List)),
        ] {
            let hint = lens_mime(lens).expect("lente con hint");
            assert_eq!(view_mode_for_hint(hint), esperado, "lente {lens:?}");
        }
        // Grid no tiene hint (el front conserva su vista).
        assert_eq!(lens_mime(Lens::Grid), None);
        // Un hint ajeno no fuerza vista.
        assert_eq!(view_mode_for_hint("text/plain"), None);
    }

    #[test]
    fn app_no_registrada_devuelve_none() {
        // Registro vacío: ningún id resuelve, el shell cae al panel.
        let reg = AppRegistry::new(vec![]);
        assert!(default_app_for(&reg, Lens::Gallery).is_none());
    }

    #[test]
    fn ruteo_por_id_evita_la_ambiguedad_del_mime() {
        // Documenta el porqué del ruteo por id: text/csv matchea TANTO nada
        // (prefijo text/) como nakui, y nada gana por label — por eso datos
        // se rutea por id, no por mime.
        let reg = AppRegistry::new(default_entries());
        let por_mime = reg.handlers_for("text/csv");
        assert!(por_mime.iter().any(|e| e.id == "nada"), "el prefijo text/ de nada captura csv");
        // El despacho de Mónada, en cambio, va derecho a nakui.
        assert_eq!(default_app_for(&reg, Lens::Database).map(|e| e.id.as_str()), Some("nakui"));
    }
}
