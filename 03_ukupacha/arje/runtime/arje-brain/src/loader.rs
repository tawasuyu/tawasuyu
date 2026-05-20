//! Loader de Cards y Reglas desde archivos JSON.
//!
//! Sustituye al antiguo `kcl_loader.rs` (eliminado): la rama KCL invocaba
//! un subprocess al CLI Go `kcl` que ningún target real tenía instalado y
//! cuya validación duplicaba `EntityCard::validate()`. La fuente de verdad
//! del shape de la Card es Rust + serde; en disco se guarda JSON crudo.
//!
//! Ergonomía de autoría futura (RON, Dhall, etc.) se añade como ramas
//! adicionales aquí cuando duela escribir JSON a mano. Hoy: una sola rama.

use arje_brain_rules::rules::Rule;
use arje_card::EntityCard;
use std::path::Path;
use tracing::info;

/// Carga una `EntityCard` desde un archivo JSON. Pasa por
/// `EntityCard::validate()` antes de devolver — falla rápida.
pub fn load_card_file(path: &Path) -> anyhow::Result<EntityCard> {
    info!(path = %path.display(), "cargando Card desde JSON");
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
    // Propagamos el error del intento directo: es el caso típico (JSON top-level
    // = EntityCard) y su mensaje apunta al campo concreto que rompió.
    anyhow::bail!("JSON no contiene una EntityCard válida: {direct_err}")
}

/// Carga reglas desde un archivo JSON.
pub fn load_rules_file(path: &Path) -> anyhow::Result<Vec<Rule>> {
    info!(path = %path.display(), "cargando reglas desde JSON");
    let raw = std::fs::read_to_string(path)?;
    extract_rules_from_json(&raw)
}

/// Extrae un `Vec<Rule>` de un blob de texto. Acepta tres formas:
/// 1. JSONL: una `Rule` por línea (el formato que escribe `append_rule_jsonl`).
/// 2. Array directo: `[{...}, {...}]`.
/// 3. Object con un campo array: `{"rules": [...]}`.
///
/// Heurística: si el primer carácter no-blanco es `[` o `{` con formato
/// "objeto-con-array", parseamos como JSON único; en otro caso intentamos
/// línea-por-línea. Líneas vacías o que empiecen con `#` se ignoran (compat
/// con archivos editados a mano que dejen comentarios estilo shell).
pub fn extract_rules_from_json(raw: &str) -> anyhow::Result<Vec<Rule>> {
    let trimmed_start = raw.trim_start();
    let looks_jsonl = trimmed_start.starts_with('{')
        && raw.lines().filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        }).count() > 1;

    if !looks_jsonl {
        // Camino clásico: un único documento JSON (array o objeto).
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
            let arr = match v {
                serde_json::Value::Array(_) => v,
                serde_json::Value::Object(map) => map
                    .into_values()
                    .find(|x| x.is_array())
                    .ok_or_else(|| anyhow::anyhow!("JSON no contiene ningún array"))?,
                _ => anyhow::bail!("JSON debe ser array o object con campo array"),
            };
            return Ok(serde_json::from_value(arr)?);
        }
        // Caer a JSONL si el documento único no parsea — útil para archivos
        // que mezclan comentarios `#` (no JSON válido como documento único).
    }

    let mut rules = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') { continue; }
        let rule: Rule = serde_json::from_str(t)
            .map_err(|e| anyhow::anyhow!("JSONL línea {}: {e}", idx + 1))?;
        rules.push(rule);
    }
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::introspect::append_rule_jsonl;
    use arje_brain_rules::rules::{Action, EventKind, EventPattern, LogLevel, Rule, Scope};
    use ulid::Ulid;

    fn sample_rule() -> Rule {
        Rule {
            id: Ulid::new(),
            priority: 5,
            when: EventPattern::Single { kind: EventKind::EnteSpawned },
            then: vec![Action::Log {
                level: LogLevel::Info,
                message: "test".into(),
            }],
            scope: Scope::default(),
        }
    }

    #[test]
    fn rules_from_array() {
        let r = sample_rule();
        let raw = format!("[{}]", serde_json::to_string(&r).unwrap());
        let parsed = extract_rules_from_json(&raw).expect("array parse");
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn rules_from_object_with_array() {
        let r = sample_rule();
        let raw = format!(r#"{{"rules":[{}]}}"#, serde_json::to_string(&r).unwrap());
        let parsed = extract_rules_from_json(&raw).expect("object parse");
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn rules_from_jsonl_with_comments_and_blanks() {
        let r1 = sample_rule();
        let r2 = sample_rule();
        let raw = format!(
            "# header comment\n\n{}\n# inline comment\n{}\n\n",
            serde_json::to_string(&r1).unwrap(),
            serde_json::to_string(&r2).unwrap()
        );
        let parsed = extract_rules_from_json(&raw).expect("jsonl parse");
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn append_rule_jsonl_roundtrip() {
        let dir = tempdir_unique();
        let path = dir.join("rules.jsonl");
        let r1 = sample_rule();
        let r2 = sample_rule();
        append_rule_jsonl(&path, &r1).expect("append 1");
        append_rule_jsonl(&path, &r2).expect("append 2");
        let raw = std::fs::read_to_string(&path).expect("read back");
        let parsed = extract_rules_from_json(&raw).expect("roundtrip parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, r1.id);
        assert_eq!(parsed[1].id, r2.id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tempdir_unique() -> std::path::PathBuf {
        let base = std::env::temp_dir();
        let p = base.join(format!("ente-brain-loader-{}", Ulid::new()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
