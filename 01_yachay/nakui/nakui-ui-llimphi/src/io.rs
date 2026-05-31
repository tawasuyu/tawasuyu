use super::*;

/// Carga UiModules desde un directorio via el brazo unificado
/// `cards::load_cards_from_dir`. Aplica las reglas específicas de la
/// UI: sólo `CardBody::UiModule` cuenta; otros body kinds se reportan
/// en el `skipped` para que el runtime los muestre como banner
/// informativo; cada `Module` se valida via `Module::validate()`;
/// detecta `id` duplicados entre módulos UiModule.
///
/// Devuelve `(modules, skipped_ids)` ordenados por id.
pub(crate) fn load_ui_modules(dir: &std::path::Path) -> Result<(Vec<Module>, Vec<String>), String> {
    let cards = cards::load_cards_from_dir(dir).map_err(|e| e.to_string())?;
    let mut modules: Vec<Module> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for c in cards {
        match c.body {
            CardBody::UiModule(m) => modules.push(m),
            other => skipped.push(format!("{}({})", c.id, other.kind_name())),
        }
    }
    for m in &modules {
        m.validate()
            .map_err(|e| format!("módulo '{}' inválido: {e}", m.id))?;
    }
    modules.sort_by(|a, b| a.id.cmp(&b.id));
    let mut prev: Option<&Module> = None;
    for cur in &modules {
        if let Some(p) = prev {
            if p.id == cur.id {
                return Err(format!(
                    "id de módulo duplicado: '{}' aparece más de una vez",
                    cur.id
                ));
            }
        }
        prev = Some(cur);
    }
    Ok((modules, skipped))
}

/// Siembra datos de ejemplo de cada módulo que traiga un `seed.json`
/// junto a su `module.json` (en `<modules_dir>/<module.id>/seed.json`),
/// **sólo** para las entities que estén vacías en el backend. Devuelve
/// un toast resumen si sembró algo.
///
/// Formato del `seed.json`:
/// ```json
/// { "seed": [
///     { "entity": "Customer", "records": [
///         { "handle": "acme", "data": { "name": "ACME", ... } } ] },
///     { "entity": "Order", "records": [
///         { "data": { "customer": "@acme", "monto": 1200 } } ] } ] }
/// ```
/// Los valores string que empiezan con `@` se resuelven al UUID del
/// record sembrado con ese `handle` (los bloques se procesan en orden,
/// así una entity puede referenciar a otra ya sembrada).
pub(crate) fn seed_demo_data(
    backend: &mut NakuiBackend,
    modules: &[Module],
    modules_dir: &std::path::Path,
) -> Option<String> {
    let mut total = 0usize;
    let mut entities_seeded: Vec<String> = Vec::new();
    for m in modules {
        let path = modules_dir.join(&m.id).join("seed.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(blocks) = doc.get("seed").and_then(Value::as_array) else {
            continue;
        };
        // handle → UUID de los records ya sembrados (para resolver `@`).
        let mut handles: BTreeMap<String, String> = BTreeMap::new();
        for block in blocks {
            let Some(entity) = block.get("entity").and_then(Value::as_str) else {
                continue;
            };
            // Idempotencia: no sembrar si la entity ya tiene records.
            if !backend.list_records(entity).is_empty() {
                continue;
            }
            let Some(records) = block.get("records").and_then(Value::as_array) else {
                continue;
            };
            let mut count = 0usize;
            for rec in records {
                let Some(data) = rec.get("data").and_then(Value::as_object) else {
                    continue;
                };
                // Resolver refs `@handle` a UUIDs ya sembrados.
                let mut obj = data.clone();
                for v in obj.values_mut() {
                    if let Value::String(s) = v {
                        if let Some(key) = s.strip_prefix('@') {
                            if let Some(uuid) = handles.get(key) {
                                *v = Value::String(uuid.clone());
                            }
                        }
                    }
                }
                match backend.seed(entity, obj) {
                    Ok(outcome) => {
                        count += 1;
                        if let (Some(handle), Some(id)) =
                            (rec.get("handle").and_then(Value::as_str), outcome.id)
                        {
                            handles.insert(handle.to_string(), id.to_string());
                        }
                    }
                    Err(_) => continue,
                }
            }
            if count > 0 {
                entities_seeded.push(format!("{entity}×{count}"));
                total += count;
            }
        }
    }
    (total > 0).then(|| format!("sembré datos de ejemplo: {}", entities_seeded.join(", ")))
}

/// Carga el sidecar del layout del grafo (posiciones de nodos por
/// `(module_id, morfismo)`). Formato: array de `{module, morphism, x,
/// y}`. Ausente/ilegible → mapa vacío (layout automático).
pub(crate) fn load_graph_layout(path: &std::path::Path) -> BTreeMap<(String, String), (f32, f32)> {
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    let Ok(arr) = serde_json::from_str::<Vec<Value>>(&text) else {
        return out;
    };
    for e in arr {
        let (Some(m), Some(f), Some(x), Some(y)) = (
            e.get("module").and_then(Value::as_str),
            e.get("morphism").and_then(Value::as_str),
            e.get("x").and_then(Value::as_f64),
            e.get("y").and_then(Value::as_f64),
        ) else {
            continue;
        };
        out.insert((m.to_string(), f.to_string()), (x as f32, y as f32));
    }
    out
}

/// Persiste el layout del grafo al sidecar. Errores de IO se ignoran
/// (perder un layout no es fatal — se recae al automático).
pub(crate) fn save_graph_layout(pos: &BTreeMap<(String, String), (f32, f32)>, path: &std::path::Path) {
    let arr: Vec<Value> = pos
        .iter()
        .map(|((m, f), (x, y))| {
            serde_json::json!({ "module": m, "morphism": f, "x": x, "y": y })
        })
        .collect();
    if let Ok(text) = serde_json::to_string_pretty(&arr) {
        let _ = std::fs::write(path, text);
    }
}
