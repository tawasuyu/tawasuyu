use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldPath {
    pub entity: String,
    pub id: Uuid,
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FieldOp {
    Set {
        path: FieldPath,
        value: Value,
    },
    /// Remove a single field key from a record. Distinto de `Set { value: Null }`:
    /// `Clear` borra la clave del map; un load posterior no encuentra el
    /// campo (`None`/`Value::Null` semantically). `Set Null` por contraste
    /// deja la clave con valor literal `null`. La distinción importa para
    /// downstream code que diferencia "ausente" de "presente como null"
    /// (ej: serialize que `skip_serializing_if = "Option::is_none"`).
    ///
    /// Capability token: `entity.field` (mismo shape que Set).
    Clear {
        path: FieldPath,
    },
    Create {
        entity: String,
        id: Uuid,
        data: Value,
    },
    Delete {
        entity: String,
        id: Uuid,
    },
}

impl FieldOp {
    /// Token a manifest's `writes` list matches against.
    /// "Caja.saldo" for field updates, "Movimiento" for whole-record ops.
    pub fn capability_token(&self) -> String {
        match self {
            FieldOp::Set { path, .. } => format!("{}.{}", path.entity, path.field),
            FieldOp::Clear { path } => format!("{}.{}", path.entity, path.field),
            FieldOp::Create { entity, .. } => entity.clone(),
            FieldOp::Delete { entity, .. } => entity.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn simulate_clear_removes_field() {
        let id = Uuid::new_v4();
        let state = json!({"name": "Acme", "notes": "lorem"});
        let op = FieldOp::Clear {
            path: FieldPath {
                entity: "Customer".into(),
                id,
                field: "notes".into(),
            },
        };
        let after = simulate_on(&state, "Customer", id, &[op]).unwrap();
        let map = after.as_object().unwrap();
        assert!(!map.contains_key("notes"));
        assert_eq!(map.get("name"), Some(&json!("Acme")));
    }

    #[test]
    fn simulate_clear_then_set_same_field_keeps_set() {
        let id = Uuid::new_v4();
        let state = json!({"name": "Acme", "notes": "lorem"});
        let ops = vec![
            FieldOp::Clear {
                path: FieldPath {
                    entity: "Customer".into(),
                    id,
                    field: "notes".into(),
                },
            },
            FieldOp::Set {
                path: FieldPath {
                    entity: "Customer".into(),
                    id,
                    field: "notes".into(),
                },
                value: json!("nuevo"),
            },
        ];
        let after = simulate_on(&state, "Customer", id, &ops).unwrap();
        assert_eq!(after.get("notes"), Some(&json!("nuevo")));
    }

    #[test]
    fn clear_capability_token_matches_set_shape() {
        let id = Uuid::new_v4();
        let set = FieldOp::Set {
            path: FieldPath {
                entity: "Customer".into(),
                id,
                field: "notes".into(),
            },
            value: json!("x"),
        };
        let clear = FieldOp::Clear {
            path: FieldPath {
                entity: "Customer".into(),
                id,
                field: "notes".into(),
            },
        };
        assert_eq!(set.capability_token(), "Customer.notes");
        assert_eq!(
            clear.capability_token(),
            set.capability_token(),
            "Clear y Set comparten token shape para el capability check"
        );
    }
}

/// Apply only the ops that target `(entity, id)` to `state` and return the
/// new value. Returns `None` if a Delete op removes the target — callers
/// should skip post-checks against a deleted entity rather than running
/// them against the stale prior state.
pub fn simulate_on(state: &Value, entity: &str, id: Uuid, ops: &[FieldOp]) -> Option<Value> {
    let mut s: Option<Value> = Some(state.clone());
    for op in ops {
        match op {
            FieldOp::Set { path, value } if path.entity == entity && path.id == id => {
                if let Some(Value::Object(map)) = s.as_mut() {
                    map.insert(path.field.clone(), value.clone());
                }
            }
            FieldOp::Clear { path } if path.entity == entity && path.id == id => {
                if let Some(Value::Object(map)) = s.as_mut() {
                    map.remove(&path.field);
                }
            }
            FieldOp::Create {
                entity: e,
                id: i,
                data,
            } if e == entity && *i == id => {
                s = Some(data.clone());
            }
            FieldOp::Delete {
                entity: e,
                id: i,
            } if e == entity && *i == id => {
                s = None;
            }
            _ => {}
        }
    }
    s
}
