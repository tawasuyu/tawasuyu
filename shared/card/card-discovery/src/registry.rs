//! Registro local: escaneo de directorios con Cards en disco.

use crate::index::CardIndex;
use std::path::Path;

/// Escanea `dir` (no recursivo) cargando toda Card `*.json` válida.
/// Los archivos que no parsean como Card se saltan en silencio.
pub fn scan_dir(dir: &Path) -> std::io::Result<CardIndex> {
    let mut index = CardIndex::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(card) = brahman_cards::load_card_file(&path) {
                index.insert(card);
            }
        }
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brahman_card::Card;

    #[test]
    fn scans_only_valid_json_cards() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["alpha", "beta"] {
            let card = Card::new(name);
            let json = serde_json::to_string(&card).unwrap();
            std::fs::write(dir.path().join(format!("{name}.json")), json).unwrap();
        }
        // Ruido que debe ignorarse.
        std::fs::write(dir.path().join("readme.txt"), "no soy una card").unwrap();
        std::fs::write(dir.path().join("roto.json"), "{ no json }").unwrap();

        let ix = scan_dir(dir.path()).unwrap();
        assert_eq!(ix.len(), 2);
    }

    #[test]
    fn missing_dir_is_an_error() {
        assert!(scan_dir(Path::new("/no/existe/jamas")).is_err());
    }
}
