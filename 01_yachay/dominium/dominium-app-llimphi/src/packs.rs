//! Packs de Conceptos: scenarios embebidos en el binario y persistencia del
//! pack del usuario en `$XDG_CONFIG_HOME/dominium/pack.json`.

use std::path::PathBuf;

use dominium_core::{Conceptos, SimParams};
use dominium_iso::ZWeights;
use serde::{Deserialize, Serialize};

/// Pack JSON por defecto — iglesia / banco / comuna / laboratorio + variantes.
/// Embebido para que el binario corra sin archivos sueltos en cwd.
pub(crate) const DEFAULT_PACK: &str = include_str!("../conceptos.default.json");
/// Scenarios embebidos: civilizaciones-arquetipo. Cada uno es un JSON con
/// la misma forma que el `DEFAULT_PACK`; el picker del panel cicla entre
/// ellos sin necesidad de archivos sueltos.
pub(crate) const PACK_ANDES: &str = include_str!("../packs/andes.json");
pub(crate) const PACK_MESOPOTAMIA: &str = include_str!("../packs/mesopotamia.json");
pub(crate) const PACK_CAPITALISMO: &str = include_str!("../packs/capitalismo.json");

/// Parsea el pack JSON embebido. Si el JSON está malformado el binario
/// arranca con la colección vacía — la sim corre igual.
pub(crate) fn default_conceptos() -> Conceptos {
    serde_json::from_str::<Conceptos>(DEFAULT_PACK).unwrap_or_default()
}

/// Listado ordenado de packs embebidos disponibles en el picker. El primero
/// es el default; el ciclo es circular. Tupla `(id legible, JSON raw)`.
pub(crate) fn scenario_packs() -> [(&'static str, &'static str); 4] {
    [
        ("default", DEFAULT_PACK),
        ("andes", PACK_ANDES),
        ("mesopotamia", PACK_MESOPOTAMIA),
        ("capitalismo", PACK_CAPITALISMO),
    ]
}

/// Path absoluto al pack del usuario: `$XDG_CONFIG_HOME/dominium/pack.json`
/// (típicamente `~/.config/dominium/pack.json`). `None` si la plataforma
/// no expone un config dir.
pub(crate) fn user_pack_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "dominium")
        .map(|d| d.config_dir().join("pack.json"))
}

/// Escenario completo serializable: la termodinámica del motor
/// (`params`), el relieve visual (`weights`) y los Conceptos del mundo, en
/// un solo archivo reproducible. Es el "pack" en su forma rica — guardar y
/// cargar restituye el mundo *y su sintonía*, no sólo las fichas.
///
/// `params` y `weights` son `Option` para que un pack viejo (sólo
/// `Conceptos`) cargue sin ellos y la app conserve su sintonía actual. La
/// retrocompatibilidad la garantiza [`parse_escenario`], no el `#[serde]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Escenario {
    /// Sintonía del motor. `None` = no tocar los `SimParams` vigentes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<SimParams>,
    /// Relieve visual (`ZWeights`). `None` = no tocar el relieve vigente.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<ZWeights>,
    /// Las fichas del mundo. Siempre presente (puede ir vacío).
    #[serde(default)]
    pub conceptos: Conceptos,
}

/// Parsea un JSON de pack tolerando los dos formatos:
/// 1. **Escenario rico** `{ "params": …, "weights": …, "conceptos": {…} }`.
/// 2. **Pack histórico** `{ "items": [ … ] }` — sólo `Conceptos`, sin
///    sintonía; se envuelve en un `Escenario` con `params`/`weights` a
///    `None`.
///
/// El discriminante es la clave `conceptos`: si está, es formato rico; si
/// no, se intenta como `Conceptos` plano.
fn parse_escenario(raw: &str) -> Option<Escenario> {
    if raw.contains("\"conceptos\"") {
        match serde_json::from_str::<Escenario>(raw) {
            Ok(esc) => return Some(esc),
            Err(e) => eprintln!("dominium · escenario malformado: {e}"),
        }
    }
    match serde_json::from_str::<Conceptos>(raw) {
        Ok(conceptos) => Some(Escenario {
            params: None,
            weights: None,
            conceptos,
        }),
        Err(e) => {
            eprintln!("dominium · pack corrupto: {e}");
            None
        }
    }
}

/// Escribe el escenario completo (sintonía + relieve + Conceptos) al pack
/// del usuario. Crea el directorio padre si no existe. Errores van a
/// stderr (la app no muere).
pub(crate) fn save_user_escenario(params: &SimParams, weights: &ZWeights, cs: &Conceptos) {
    let Some(path) = user_pack_path() else {
        eprintln!("dominium · no hay ProjectDirs en esta plataforma");
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("dominium · no pude crear {}: {e}", parent.display());
            return;
        }
    }
    let esc = Escenario {
        params: Some(params.clone()),
        weights: Some(*weights),
        conceptos: cs.clone(),
    };
    match serde_json::to_string_pretty(&esc) {
        Ok(json) => match std::fs::write(&path, json) {
            Ok(()) => eprintln!("dominium · escenario guardado en {}", path.display()),
            Err(e) => eprintln!("dominium · error escribiendo {}: {e}", path.display()),
        },
        Err(e) => eprintln!("dominium · error serializando escenario: {e}"),
    }
}

/// Carga el escenario del usuario si existe. Devuelve `None` si el archivo
/// no está, o si el contenido no parsea por ninguno de los dos formatos.
pub(crate) fn load_user_escenario() -> Option<Escenario> {
    let path = user_pack_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let esc = parse_escenario(&raw)?;
    eprintln!("dominium · escenario cargado desde {}", path.display());
    Some(esc)
}

/// Carga sólo los Conceptos del pack del usuario — vista estrecha sobre
/// [`load_user_escenario`] para el seeder del mundo (que no toca params).
pub(crate) fn load_user_pack() -> Option<Conceptos> {
    load_user_escenario().map(|e| e.conceptos)
}
