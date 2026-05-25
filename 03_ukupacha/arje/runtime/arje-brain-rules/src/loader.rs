//! Loader de Reglas (`Rule`) desde archivos JSON / JSONL.
//!
//! La carga de `Rule` vive aquí, junto a su definición. La carga de
//! `EntityCard` se consolidó en `brahman-cards::entity_loader`.

use crate::rules::Rule;
use std::path::Path;
use tracing::info;

/// Carga reglas desde un archivo JSON / JSONL.
pub fn load_rules_file(path: &Path) -> anyhow::Result<Vec<Rule>> {
    info!(path = %path.display(), "cargando reglas desde JSON");
    let raw = std::fs::read_to_string(path)?;
    extract_rules_from_json(&raw)
}

/// Extrae un `Vec<Rule>` de un blob de texto. Acepta tres formas:
/// 1. JSONL: una `Rule` por línea.
/// 2. Array directo: `[{...}, {...}]`.
/// 3. Object con un campo array: `{"rules": [...]}`.
///
/// Líneas vacías o que empiecen con `#` se ignoran (compat con archivos
/// editados a mano que dejen comentarios estilo shell).
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
        // Caer a JSONL si el documento único no parsea.
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
    use crate::rules::{Action, EventKind, EventPattern, LogLevel, Rule, Scope};
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
    fn jsonl_roundtrip_preserves_order_and_ids() {
        // Roundtrip JSONL escrito manualmente (una Rule por línea).
        let r1 = sample_rule();
        let r2 = sample_rule();
        let raw = format!(
            "{}\n{}\n",
            serde_json::to_string(&r1).unwrap(),
            serde_json::to_string(&r2).unwrap(),
        );
        let parsed = extract_rules_from_json(&raw).expect("roundtrip parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, r1.id);
        assert_eq!(parsed[1].id, r2.id);
    }
}
