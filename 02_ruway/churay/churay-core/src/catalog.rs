//! El catálogo de la suite — la lista de unidades **app** instalables.
//!
//! Las apps salen de la **única tabla de apps** del repo
//! ([`app_bus::default_entries`]); no se reinventa la lista. A eso se suman la
//! barra `pata` y el shell `shuma` (que `app-bus` no lista). El **sistema base**
//! (compositor + display manager mirada, init) NO es una app: vive aparte, en
//! [`crate::base`], como opción especial. Acá se corrigen descripciones, se
//! declaran sugerencias, se propagan los mimes que cada app abre, y se marca
//! cuáles tienen sentido "Abrir" sueltas.

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
        // `mirada-panel` = el binario `mirada-llimphi`: el **panel de control**
        // de mirada (escritorios, atajos, vista Prezi). El compositor en sí es
        // del sistema base, no esta entrada.
        "mirada-panel" => "Panel de control de mirada (escritorios, atajos)",
        "panel-control" => "Panel de control unificado de la suite",
        "pata" => "Barra de estado / panel tipo waybar para el escritorio",
        "shuma" => "Terminal y workspace inteligente (estilo zellij), con sesión standalone",
        _ => "",
    }
}

/// Sugerencias blandas: unidades que se complementan. `pata` y `shuma` se
/// potencian mutuamente (pata hospeda el shell de shuma; shuma anda mejor con la
/// barra); el panel de mirada gana con la barra `pata`.
fn sugerencias(id: &str) -> Vec<String> {
    let v: &[&str] = match id {
        "pata" => &["shuma", "mirada-panel"],
        "shuma" => &["pata"],
        "mirada-panel" => &["pata"],
        _ => &[],
    };
    v.iter().map(|s| s.to_string()).collect()
}

/// `false` para las piezas "complicadas" que corren en contexto de sesión y no
/// se abren sueltas (la barra `pata`). El resto son apps normales.
fn launchable_de(id: &str) -> bool {
    !matches!(id, "pata")
}

/// Instrucción exacta tras instalar, si la unidad la necesita.
fn post_install_de(id: &str) -> Option<String> {
    match id {
        "pata" => Some("La barra se inicia con la sesión de mirada; no se abre suelta.".into()),
        "mirada-panel" => Some("Necesita el compositor mirada corriendo (instalalo desde «Sistema base»).".into()),
        _ => None,
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
        suggests: sugerencias(&e.id),
        handles: e.handles.clone(),
        launchable: launchable_de(&e.id),
        post_install: post_install_de(&e.id),
        bin_hash: None,
        size_bytes: None,
    })
}

/// Una unidad armada a mano (las que `app-bus` no lista).
fn unit(id: &str, label: &str, icon: &str, program: &str, category: &str) -> Unit {
    Unit {
        id: id.into(),
        label: label.into(),
        version: SUITE_VERSION.to_string(),
        category: category.into(),
        icon: icon.into(),
        description: descripcion(id).to_string(),
        program: program.into(),
        scope: Scope::App,
        suggests: sugerencias(id),
        handles: Vec::new(),
        launchable: launchable_de(id),
        post_install: post_install_de(id),
        bin_hash: None,
        size_bytes: None,
    }
}

/// Unidades que `app-bus` no trae: la barra `pata` y el shell `shuma`.
fn unidades_extra() -> Vec<Unit> {
    vec![
        unit("pata", "Pata", "▬", "pata", "ukupacha"),
        unit("shuma", "Shuma", "❯", "shuma-shell-llimphi", "ruway"),
    ]
}

/// El catálogo de **apps**: las de `app-bus` (orden alfabético por label) + la
/// barra y el shell. El sistema base (compositor/DM) no está acá — ver
/// [`crate::base`]. Fuente del lado B (compilar) y de la UI sin manifiesto.
pub fn suite_catalog() -> Vec<Unit> {
    let mut units: Vec<Unit> =
        app_bus::default_entries().iter().filter_map(unit_de_app).collect();
    units.extend(unidades_extra());
    units.sort_by(|a, b| a.label.cmp(&b.label));
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
        assert!(cat.len() >= 20, "19 apps Exec + pata + shuma");
        assert!(cat.iter().any(|u| u.id == "nada" && u.program == "nada"));
        assert!(cat.iter().any(|u| u.id == "cosmos" && u.program == "cosmos-app-llimphi"));
        // El sistema base no es app: no debe aparecer acá.
        assert!(!cat.iter().any(|u| u.id == "arje"));
    }

    #[test]
    fn apps_propagan_los_mimes_que_abren() {
        let cat = suite_catalog();
        // tullpu abre image/*; nada abre text/*.
        let tullpu = cat.iter().find(|u| u.id == "tullpu").unwrap();
        assert!(tullpu.handles.iter().any(|h| h.starts_with("image/")));
        let nada = cat.iter().find(|u| u.id == "nada").unwrap();
        assert!(nada.handles.iter().any(|h| h.starts_with("text/")));
    }

    #[test]
    fn pata_no_es_lanzable_pero_las_apps_si() {
        let cat = suite_catalog();
        let pata = cat.iter().find(|u| u.id == "pata").unwrap();
        assert!(!pata.launchable, "la barra no se abre suelta");
        assert!(pata.post_install.is_some());
        let cosmos = cat.iter().find(|u| u.id == "cosmos").unwrap();
        assert!(cosmos.launchable);
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
}
