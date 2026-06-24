//! El catálogo de la suite — la lista de unidades instalables.
//!
//! Las apps salen de la **única tabla de apps** del repo
//! ([`app_bus::default_entries`]); no se reinventa la lista. A eso se suman
//! componentes que `app-bus` no lista (la barra `pata`, el shell `shuma`) y los
//! de **sistema** (`arje`, el init) que sólo tienen sentido con root. Acá
//! también se corrigen descripciones y se declaran las **sugerencias** entre
//! unidades (p.ej. `pata` ↔ `shuma`).

use crate::manifest::{Manifest, Scope, Unit};

/// Versión de la suite que estampan las unidades generadas localmente.
/// (El manifiesto firmado de un release la sobreescribe con la real.)
pub const SUITE_VERSION: &str = "2026.06";

/// Una línea de descripción por id. Si falta, se cae al label.
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
        // Correcciones pedidas: mirada NO es un panel, es el compositor.
        "mirada-panel" => "Compositor Wayland y gestor de ventanas",
        "panel-control" => "Panel de control de la suite",
        "pata" => "Barra de estado / panel tipo waybar para el escritorio",
        "shuma" => "Terminal y workspace inteligente (estilo zellij), con sesión standalone",
        "arje" => "Init y supervisor del sistema (arranque, servicios)",
        _ => "",
    }
}

/// Etiqueta corregida para algunos ids (cuando la de `app-bus` confunde).
fn label_override(id: &str) -> Option<&'static str> {
    match id {
        "mirada-panel" => Some("Mirada (compositor)"),
        _ => None,
    }
}

/// Sugerencias blandas: unidades que se complementan. `pata` y `shuma` se
/// potencian mutuamente (pata hospeda el shell de shuma; shuma anda mejor con la
/// barra); `mirada` (compositor) gana con `pata` (su barra).
fn sugerencias(id: &str) -> Vec<String> {
    let v: &[&str] = match id {
        "pata" => &["shuma", "mirada-panel"],
        "shuma" => &["pata"],
        "mirada-panel" => &["pata"],
        _ => &[],
    };
    v.iter().map(|s| s.to_string()).collect()
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
        label: label_override(&e.id).map(String::from).unwrap_or_else(|| e.label.clone()),
        version: SUITE_VERSION.to_string(),
        category,
        icon,
        description: if desc.is_empty() { e.label.clone() } else { desc.to_string() },
        program,
        scope: Scope::App,
        suggests: sugerencias(&e.id),
        bin_hash: None,
        size_bytes: None,
    })
}

/// Una unidad armada a mano (las que `app-bus` no lista).
fn unit(id: &str, label: &str, icon: &str, program: &str, category: &str, scope: Scope) -> Unit {
    Unit {
        id: id.into(),
        label: label.into(),
        version: SUITE_VERSION.to_string(),
        category: category.into(),
        icon: icon.into(),
        description: descripcion(id).to_string(),
        program: program.into(),
        scope,
        suggests: sugerencias(id),
        bin_hash: None,
        size_bytes: None,
    }
}

/// Unidades que `app-bus` no trae: la barra `pata` y el shell `shuma`.
fn unidades_extra() -> Vec<Unit> {
    vec![
        unit("pata", "Pata", "▬", "pata", "ukupacha", Scope::App),
        unit("shuma", "Shuma", "❯", "shuma-shell-llimphi", "ruway", Scope::App),
    ]
}

/// Las unidades **de sistema** — componentes fuertes que exigen root.
fn unidades_de_sistema() -> Vec<Unit> {
    vec![unit("arje", "Arje (init)", "⏻", "arje", "sistema", Scope::System)]
}

/// El catálogo completo de la suite: apps de `app-bus` (orden alfabético por
/// label) + barra/shell + componentes de sistema. Fuente del lado B (compilar)
/// y de la UI cuando no hay manifiesto firmado.
pub fn suite_catalog() -> Vec<Unit> {
    let mut units: Vec<Unit> =
        app_bus::default_entries().iter().filter_map(unit_de_app).collect();
    units.extend(unidades_extra());
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
        assert!(cat.len() >= 21, "19 apps Exec + pata + shuma + sistema");
        assert!(cat.iter().any(|u| u.id == "nada" && u.program == "nada"));
        assert!(cat.iter().any(|u| u.id == "cosmos" && u.program == "cosmos-app-llimphi"));
    }

    #[test]
    fn mirada_es_compositor_no_panel() {
        let cat = suite_catalog();
        let m = cat.iter().find(|u| u.id == "mirada-panel").unwrap();
        assert!(m.description.to_lowercase().contains("compositor"));
        assert!(m.label.to_lowercase().contains("compositor"));
    }

    #[test]
    fn pata_y_shuma_existen_y_se_sugieren_mutuamente() {
        let cat = suite_catalog();
        let pata = cat.iter().find(|u| u.id == "pata").expect("pata");
        let shuma = cat.iter().find(|u| u.id == "shuma").expect("shuma");
        assert_eq!(pata.program, "pata");
        assert_eq!(shuma.program, "shuma-shell-llimphi");
        assert!(pata.suggests.contains(&"shuma".to_string()));
        assert!(shuma.suggests.contains(&"pata".to_string()));
    }

    #[test]
    fn arje_es_system_y_pide_root() {
        let cat = suite_catalog();
        let arje = cat.iter().find(|u| u.id == "arje").expect("arje en catálogo");
        assert_eq!(arje.scope, Scope::System);
        assert!(arje.requires_root());
    }
}
