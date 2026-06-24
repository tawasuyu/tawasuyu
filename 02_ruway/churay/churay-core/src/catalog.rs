//! El catálogo de la suite — la lista de unidades instalables.
//!
//! Las apps salen de la **única tabla de apps** del repo
//! ([`app_bus::default_entries`]); no se reinventa la lista. A eso se suman
//! los componentes **de sistema** (hoy `arje`, el init) que sólo tienen
//! sentido con root. El catálogo local (sin manifiesto firmado) sirve para el
//! lado B (compilar desde fuente) y como fuente de `label/icon/category`.

use crate::manifest::{Manifest, Scope, Unit};

/// Versión de la suite que estampan las unidades generadas localmente.
/// (El manifiesto firmado de un release la sobreescribe con la real.)
pub const SUITE_VERSION: &str = "2026.06";

/// Una línea de descripción por id de app. Si falta, se cae al label.
fn descripcion(id: &str) -> &'static str {
    match id {
        "nada" => "Editor de archivos con árbol y resaltado de sintaxis",
        "pluma" => "Editor de texto multilienzo (haz de cuerpos)",
        "pluma-notebook" => "Cuaderno reactivo de celdas con kernels LLM",
        "tullpu" => "Editor de imágenes y pixel art",
        "takiy" => "Editor y estudio de audio",
        "media" => "Reproductor de video y audio",
        "media-tube" => "Frente de video federado (Invidious/PeerTube)",
        "cosmos" => "Cartas astrales y astronomía",
        "dominium" => "Gemelo digital y simulación de dominios",
        "tinkuy" => "Simulador de física por fuerzas",
        "chaka" => "Lenguaje y editor visual de programas",
        "nakui" => "Planilla de cálculo y ERP",
        "puriy" => "Navegador web",
        "raymi" => "Calendario y agenda",
        "supay" => "Motor de juego / raycaster Doom",
        "sandokan-monitor" => "Monitor de procesos del sistema",
        "nahual" => "Explorador de archivos universal",
        "mirada-panel" => "Panel del compositor Wayland",
        "panel-control" => "Panel de control de la suite",
        "arje" => "Init y supervisor del sistema (arranque, servicios)",
        _ => "",
    }
}

/// Construye una `Unit` de app desde una entrada de `app-bus`.
fn unit_de_app(e: &app_bus::AppEntry) -> Option<Unit> {
    // Sólo las apps que se lanzan como binario del host (`Exec`) son
    // instalables; `Action`/`Wasm` las resuelve otro chasis.
    let program = match &e.launch {
        app_bus::Launch::Exec { program, .. } => program.clone(),
        _ => return None,
    };
    let icon = e.icon.clone().unwrap_or_default();
    let category = e.category.clone().unwrap_or_else(|| "ruway".into());
    let desc = descripcion(&e.id);
    Some(Unit {
        id: e.id.clone(),
        label: e.label.clone(),
        version: SUITE_VERSION.to_string(),
        category,
        icon,
        description: if desc.is_empty() { e.label.clone() } else { desc.to_string() },
        program,
        scope: Scope::App,
        bin_hash: None,
        size_bytes: None,
    })
}

/// Las unidades **de sistema** — componentes fuertes que exigen root.
fn unidades_de_sistema() -> Vec<Unit> {
    vec![Unit {
        id: "arje".into(),
        label: "Arje (init)".into(),
        version: SUITE_VERSION.to_string(),
        category: "sistema".into(),
        icon: "⏻".into(),
        description: descripcion("arje").to_string(),
        program: "arje".into(),
        scope: Scope::System,
        bin_hash: None,
        size_bytes: None,
    }]
}

/// El catálogo completo de la suite: apps de `app-bus` (orden alfabético por
/// label) + componentes de sistema. Es la fuente del lado B (compilar) y de la
/// UI cuando no hay manifiesto firmado.
pub fn suite_catalog() -> Vec<Unit> {
    let mut units: Vec<Unit> =
        app_bus::default_entries().iter().filter_map(unit_de_app).collect();
    units.sort_by(|a, b| a.label.cmp(&b.label));
    units.extend(unidades_de_sistema());
    units
}

/// El catálogo como [`Manifest`] sin firmar (para uso local / lado B).
pub fn local_manifest() -> Manifest {
    Manifest::new(SUITE_VERSION, suite_catalog())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogo_no_vacio_y_trae_apps_conocidas() {
        let cat = suite_catalog();
        assert!(cat.len() >= 19, "al menos las 19 apps Exec + sistema");
        assert!(cat.iter().any(|u| u.id == "nada" && u.program == "nada"));
        assert!(cat.iter().any(|u| u.id == "cosmos" && u.program == "cosmos-app-llimphi"));
    }

    #[test]
    fn arje_es_system_y_pide_root() {
        let cat = suite_catalog();
        let arje = cat.iter().find(|u| u.id == "arje").expect("arje en catálogo");
        assert_eq!(arje.scope, Scope::System);
        assert!(arje.requires_root());
    }

    #[test]
    fn las_apps_no_piden_root() {
        for u in suite_catalog().iter().filter(|u| u.scope == Scope::App) {
            assert!(!u.requires_root(), "{} no debería pedir root", u.id);
        }
    }
}
