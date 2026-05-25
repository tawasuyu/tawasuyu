//! Validador de entities via Nickel contracts (reemplaza el viejo
//! `kcl_wrapper` que shellea el binario `kcl`). Evaluación
//! in-process via `nickel-lang` 2.0.
//!
//! El bundle del módulo (concatenación de los `schema.ncl` que el
//! manifest declara) define un record con un field por entity. Para
//! validar un value V contra el entity E, evaluamos:
//!
//! ```nickel
//! let bundle = (import "<bundle>.ncl") in (V | bundle.E)
//! ```
//!
//! Si Nickel evalúa OK, V cumple el contract. Si rebota con
//! `BlameError` (contract violation), devolvemos
//! `NickelError::ValidationFailed` con el mensaje formateado.
//!
//! El bundle path es exactamente el archivo `.ncl` que arma
//! `Executor::load_module` en tempdir; el snapshot bytes que
//! computa el hash es el mismo archivo, así el `schema_bundle_hash`
//! sigue siendo determinista.

use std::path::Path;

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NickelError {
    #[error("nickel validation failed:\n{0}")]
    ValidationFailed(String),
    #[error("io durante eval Nickel: {0}")]
    Io(#[from] std::io::Error),
    #[error("serializar state a Nickel literal: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Valida `state` contra el entity `schema_name` declarado en el
/// bundle Nickel `schema_path`. Devuelve `Ok(())` si el contract
/// pasa, `Err(ValidationFailed(msg))` si rebota.
///
/// El nombre `vet` se preserva por compat con call sites del
/// executor (ex `kcl_wrapper::vet`).
pub fn vet(schema_path: &Path, state: &Value, schema_name: &str) -> Result<(), NickelError> {
    // El state se inyecta como JSON literal y Nickel lo deserializa
    // con `std.deserialize 'Json`. NO embebemos el state como
    // record literal Nickel directo: la sintaxis JSON usa `:` (que
    // Nickel no acepta dentro de records — usa `=`), y los keys
    // quoted serían parseados como contracts en posición pre-`|`.
    //
    // El JSON va dentro de un raw string Nickel `m%%"..."%%`. JSON
    // no contiene `"%%` literal (no hay forma de generarlo desde
    // serde_json), así que el delimiter es seguro sin más
    // escaping.
    let state_json = serde_json::to_string(state)?;
    let schema_path_str = schema_path.display().to_string();
    let schema_path_escaped = schema_path_str.replace('\\', "\\\\").replace('"', "\\\"");

    let source = format!(
        "let bundle = (import \"{schema_path_escaped}\") in\n\
         (std.deserialize 'Json m%%\"{state_json}\"%%) | bundle.{schema_name}"
    );

    let mut ctx =
        nickel_lang::Context::new().with_source_name(format!("nakui-validate-{schema_name}"));

    match ctx.eval_deep_for_export(&source) {
        Ok(_) => Ok(()),
        Err(e) => Err(NickelError::ValidationFailed(format_nickel_error(&e))),
    }
}

fn format_nickel_error(err: &nickel_lang::Error) -> String {
    let mut buf: Vec<u8> = Vec::new();
    if err
        .format(&mut buf, nickel_lang::ErrorFormat::Text)
        .is_err()
    {
        return format!("{err:?}");
    }
    String::from_utf8(buf).unwrap_or_else(|_| format!("{err:?}"))
}

#[cfg(test)]
mod tests {
    //! Tests del validador via fixtures inline (write a tempfile,
    //! evaluar). Cobertura del happy path + un par de
    //! contract-violation cases.
    use super::*;
    use serde_json::json;

    fn write_schema(content: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "nakui-test-schema-{}-{}.ncl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn vet_passes_when_state_satisfies_contract() {
        let schema = write_schema(
            r#"
            {
              Stock = {
                id | String,
                cantidad | std.contract.from_predicate (fun n => std.is_number n && n >= 0),
              },
            }
            "#,
        );
        let state = json!({"id": "abc", "cantidad": 5});
        vet(&schema, &state, "Stock").unwrap();
        let _ = std::fs::remove_file(&schema);
    }

    #[test]
    fn vet_rejects_when_field_missing() {
        let schema = write_schema(
            r#"
            {
              Stock = { id | String, cantidad | Number },
            }
            "#,
        );
        let state = json!({"id": "abc"}); // falta cantidad
        let err = vet(&schema, &state, "Stock").unwrap_err();
        assert!(matches!(err, NickelError::ValidationFailed(_)));
        let NickelError::ValidationFailed(msg) = err else {
            panic!()
        };
        assert!(
            msg.to_lowercase().contains("cantidad") || msg.to_lowercase().contains("missing"),
            "msg debe mencionar el field missing: {msg}"
        );
        let _ = std::fs::remove_file(&schema);
    }

    #[test]
    fn vet_rejects_when_predicate_fails() {
        let schema = write_schema(
            r#"
            {
              Stock = {
                id | String,
                cantidad | std.contract.from_predicate (fun n => std.is_number n && n >= 0),
              },
            }
            "#,
        );
        let state = json!({"id": "abc", "cantidad": -1});
        let err = vet(&schema, &state, "Stock").unwrap_err();
        assert!(matches!(err, NickelError::ValidationFailed(_)));
        let _ = std::fs::remove_file(&schema);
    }

    /// Repro EXACTO del shape Transferencia del módulo treasury,
    /// incluyendo el predicate cross-field. Reproduce el mismo
    /// flujo que el rhai script emite.
    #[test]
    fn vet_transferencia_real_shape_passes() {
        let schema = write_schema(
            r#"
            let positive_int = std.contract.from_predicate (fun n => std.is_number n && n > 0) in
            let currency_iso = std.contract.from_predicate (fun s => std.is_string s && std.string.length s == 3) in
            {
              Transferencia = std.contract.Sequence [
                {
                  id | String,
                  source_caja_id | String,
                  dest_caja_id | String,
                  monto | positive_int,
                  currency | currency_iso,
                  timestamp | String,
                  memo | String | optional,
                },
                std.contract.from_predicate (fun r =>
                  r.source_caja_id != r.dest_caja_id
                ),
              ],
            }
            "#,
        );
        let state = json!({
            "currency": "USD",
            "dest_caja_id": "8c0b58aa",
            "id": "bb34ae84",
            "memo": "xf",
            "monto": 75000,
            "source_caja_id": "233f780f",
            "timestamp": "2026-05-04T10:30:00Z"
        });
        vet(&schema, &state, "Transferencia").unwrap();
        let _ = std::fs::remove_file(&schema);
    }

    /// Repro del issue de la migración: Transferencia con
    /// múltiples fields requeridos + uno optional. El contract
    /// debería pasar si todos los required están presentes.
    #[test]
    fn vet_passes_with_optional_field_present_or_absent() {
        let schema = write_schema(
            r#"
            {
              Transferencia = {
                id | String,
                source_caja_id | String,
                dest_caja_id | String,
                memo | String | optional,
              },
            }
            "#,
        );
        // Con memo presente.
        let state = json!({
            "id": "t1",
            "source_caja_id": "c1",
            "dest_caja_id": "c2",
            "memo": "x"
        });
        vet(&schema, &state, "Transferencia").unwrap();
        // Sin memo (opcional).
        let state2 = json!({
            "id": "t2",
            "source_caja_id": "c1",
            "dest_caja_id": "c2"
        });
        vet(&schema, &state2, "Transferencia").unwrap();
        let _ = std::fs::remove_file(&schema);
    }

    #[test]
    fn vet_rejects_when_cross_field_invariant_fails() {
        let schema = write_schema(
            r#"
            {
              Venta = {
                cantidad | Number,
                precio_unitario | Number,
                total | Number,
              } | std.contract.from_predicate (fun r =>
                r.total == r.cantidad * r.precio_unitario
              ),
            }
            "#,
        );
        // total mal calculado: 5 * 200 = 1000, no 999.
        let state = json!({"cantidad": 5, "precio_unitario": 200, "total": 999});
        let err = vet(&schema, &state, "Venta").unwrap_err();
        assert!(matches!(err, NickelError::ValidationFailed(_)));
        let _ = std::fs::remove_file(&schema);
    }
}
