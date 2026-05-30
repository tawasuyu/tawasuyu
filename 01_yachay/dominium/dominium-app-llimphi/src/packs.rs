//! Packs de Conceptos: scenarios embebidos en el binario y persistencia del
//! pack del usuario en `$XDG_CONFIG_HOME/dominium/pack.json`.

use std::path::PathBuf;

use dominium_core::Conceptos;

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

/// Escribe la colección actual al pack del usuario. Crea el directorio
/// padre si no existe. Errores van a stderr (la app no muere).
pub(crate) fn save_user_pack(cs: &Conceptos) {
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
    match serde_json::to_string_pretty(cs) {
        Ok(json) => match std::fs::write(&path, json) {
            Ok(()) => eprintln!("dominium · pack guardado en {}", path.display()),
            Err(e) => eprintln!("dominium · error escribiendo {}: {e}", path.display()),
        },
        Err(e) => eprintln!("dominium · error serializando pack: {e}"),
    }
}

/// Carga el pack del usuario si existe. Devuelve `None` si el archivo no
/// está, o si el contenido no es un `Conceptos` válido. Errores van a stderr.
pub(crate) fn load_user_pack() -> Option<Conceptos> {
    let path = user_pack_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<Conceptos>(&raw) {
        Ok(cs) => {
            eprintln!("dominium · pack cargado desde {}", path.display());
            Some(cs)
        }
        Err(e) => {
            eprintln!("dominium · {} corrupto: {e}", path.display());
            None
        }
    }
}
