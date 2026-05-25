//! Loader de `EntityCard` (≡ `brahman_card::Card`) desde archivos JSON.
//!
//! Card-loading consolidado: antes vivía duplicado en `arje-brain/loader.rs`.
//! La fuente de verdad del shape es Rust + serde; en disco se guarda JSON
//! crudo. Toda card-loading del ecosistema vive ahora en `brahman-cards`.

use brahman_card::Card as EntityCard;
use std::path::Path;

/// Carga una `EntityCard` desde un archivo JSON. Pasa por
/// `EntityCard::validate()` antes de devolver — falla rápida.
pub fn load_card_file(path: &Path) -> anyhow::Result<EntityCard> {
    let raw = std::fs::read_to_string(path)?;
    let card = extract_card_from_json(&raw)?;
    card.validate()
        .map_err(|e| anyhow::anyhow!("Card inválida ({}): {e}", path.display()))?;
    Ok(card)
}

/// Extrae una `EntityCard` de JSON. Acepta:
/// 1. Object directamente serializable como EntityCard.
/// 2. Object dict con un único valor que sea EntityCard (compat con
///    salidas de generadores que envuelven en `{"seed": {...}}`).
pub fn extract_card_from_json(raw: &str) -> anyhow::Result<EntityCard> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let direct_err = match serde_json::from_value::<EntityCard>(v.clone()) {
        Ok(c) => return Ok(c),
        Err(e) => e,
    };
    if let serde_json::Value::Object(map) = v {
        for (_, vv) in map {
            if let Ok(c) = serde_json::from_value::<EntityCard>(vv) {
                return Ok(c);
            }
        }
    }
    // Propagamos el error del intento directo: es el caso típico (JSON
    // top-level = EntityCard) y su mensaje apunta al campo que rompió.
    anyhow::bail!("JSON no contiene una EntityCard válida: {direct_err}")
}
