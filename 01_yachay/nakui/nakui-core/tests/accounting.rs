//! Tests de integración del módulo `accounting` — contabilidad de
//! PARTIDA DOBLE sobre el motor de conservación de nakui.
//!
//! La tesis: la invariante `conserve` del kernel (Σ Δ Cuenta.saldo = 0
//! por moneda) ES la partida doble. El módulo modela `Cuenta.saldo` en
//! convención deudor-normal (centavos enteros: débito suma, crédito
//! resta), así que un asiento balanceado conserva el saldo agregado y el
//! executor lo verifica a nivel de delta — no el código de app.
//!
//! Estos tests aseguran:
//!   - `asentar` mueve débito (+) y crédito (−) y persiste el Asiento.
//!   - La balanza de comprobación (Σ saldos por moneda) es CERO tras una
//!     batería de asientos — la identidad contable fundamental, derivada
//!     directamente de la conservación.
//!   - Cuenta.saldo PUEDE ser negativo (pasivo/ingreso deudor-normal):
//!     a diferencia de treasury.Caja, no hay post-check no-negativo.
//!   - Monto negativo → SchemaPostCreate (NoNegativo de Asiento.monto).
//!   - Asiento entre monedas distintas → throw del script (ExecError::Rhai).
//!   - Misma cuenta como débito y crédito → DuplicateInputId del executor.

use std::path::{Path, PathBuf};

use nakui_core::executor::{ExecError, Executor};
use nakui_core::store::{MemoryStore, Store};
use serde_json::{json, Value};
use uuid::Uuid;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root above core/")
        .to_path_buf()
}

fn accounting_module() -> PathBuf {
    workspace_root().join("modules/accounting")
}

fn saldo(store: &MemoryStore, id: Uuid) -> i64 {
    store
        .load("Cuenta", id)
        .and_then(|v| v.get("saldo").and_then(Value::as_i64))
        .expect("cuenta con saldo")
}

/// Siembra una cuenta del plan contable con saldo inicial (centavos).
fn cuenta(
    store: &mut MemoryStore,
    codigo: &str,
    nombre: &str,
    tipo: &str,
    saldo: i64,
    moneda: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    store.seed(
        "Cuenta",
        id,
        json!({
            "id": id.to_string(),
            "codigo": codigo,
            "nombre": nombre,
            "tipo": tipo,
            "saldo": saldo,
            "moneda": moneda,
        }),
    );
    id
}

/// Helper: postea un asiento `debito`→`credito` por `monto`.
fn asentar(
    exec: &Executor,
    store: &mut MemoryStore,
    debito: Uuid,
    credito: Uuid,
    monto: i64,
) -> Result<Vec<nakui_core::delta::FieldOp>, ExecError> {
    exec.run(
        store,
        "asentar",
        &[("debito", debito), ("credito", credito)],
        json!({
            "monto": monto,
            "glosa": "asiento de prueba",
            "fecha": "2026-06-28",
            "diario": "general",
            "asiento_id": Uuid::new_v4().to_string(),
        }),
    )
}

#[test]
fn asiento_balanceado_mueve_debito_y_credito() {
    let exec = Executor::load_module(accounting_module()).expect("load module");
    let mut store = MemoryStore::new();

    // Caja (activo) contra Ventas (ingreso): cobramos $5.000,00.
    let caja = cuenta(&mut store, "1010", "Caja", "activo", 0, "USD");
    let ventas = cuenta(&mut store, "4010", "Ventas", "ingreso", 0, "USD");

    let asiento_id = Uuid::new_v4();
    let ops = exec
        .run(
            &mut store,
            "asentar",
            &[("debito", caja), ("credito", ventas)],
            json!({
                "monto": 500_000_i64,        // $5.000,00 en centavos
                "glosa": "venta al contado",
                "fecha": "2026-06-28",
                "diario": "ventas",
                "asiento_id": asiento_id.to_string(),
            }),
        )
        .expect("el asiento debe pasar");

    assert_eq!(ops.len(), 3, "2 sets (débito+crédito) + 1 create (Asiento)");
    // Débito suma al activo; crédito resta del ingreso (deudor-normal).
    assert_eq!(saldo(&store, caja), 500_000);
    assert_eq!(saldo(&store, ventas), -500_000);

    let asiento = store.load("Asiento", asiento_id).expect("asiento persistido");
    assert_eq!(asiento.get("monto").and_then(Value::as_i64), Some(500_000));
    assert_eq!(
        asiento.get("debito_id").and_then(Value::as_str),
        Some(caja.to_string().as_str())
    );
    assert_eq!(
        asiento.get("credito_id").and_then(Value::as_str),
        Some(ventas.to_string().as_str())
    );
    assert_eq!(asiento.get("moneda").and_then(Value::as_str), Some("USD"));
}

#[test]
fn balanza_de_comprobacion_suma_cero_tras_varios_asientos() {
    // La identidad contable: Σ de todos los saldos (deudor-normal) por
    // moneda es CERO en todo momento. Es consecuencia directa de la
    // conservación que el kernel exige en cada `asentar`. Si alguna pata
    // se descuadrara, el executor habría rebotado con ConservationViolation
    // y este Σ no sería cero.
    let exec = Executor::load_module(accounting_module()).expect("load module");
    let mut store = MemoryStore::new();

    let caja = cuenta(&mut store, "1010", "Caja", "activo", 0, "USD");
    let banco = cuenta(&mut store, "1020", "Banco", "activo", 0, "USD");
    let ventas = cuenta(&mut store, "4010", "Ventas", "ingreso", 0, "USD");
    let proveedores = cuenta(&mut store, "2010", "Proveedores", "pasivo", 0, "USD");
    let gastos = cuenta(&mut store, "5010", "Gastos", "gasto", 0, "USD");
    let cuentas = [caja, banco, ventas, proveedores, gastos];

    // Batería de asientos típicos:
    asentar(&exec, &mut store, caja, ventas, 500_000).expect("cobro al contado");
    asentar(&exec, &mut store, banco, ventas, 250_000).expect("cobro por banco");
    asentar(&exec, &mut store, gastos, proveedores, 120_000).expect("compra a crédito");
    asentar(&exec, &mut store, proveedores, banco, 120_000).expect("pago a proveedor");
    asentar(&exec, &mut store, banco, caja, 300_000).expect("depósito de caja a banco");

    let total: i64 = cuentas.iter().map(|&c| saldo(&store, c)).sum();
    assert_eq!(total, 0, "la balanza de comprobación debe cuadrar en cero");

    // Y los saldos individuales son los esperados:
    assert_eq!(saldo(&store, caja), 500_000 - 300_000); // 200_000
    assert_eq!(saldo(&store, banco), 250_000 - 120_000 + 300_000); // 430_000
    assert_eq!(saldo(&store, ventas), -750_000); // ingreso: acreedor → negativo
    assert_eq!(saldo(&store, proveedores), 120_000 - 120_000); // 0
    assert_eq!(saldo(&store, gastos), 120_000);
}

#[test]
fn cuenta_acreedora_admite_saldo_negativo() {
    // A diferencia de treasury.Caja (no-negativo), una Cuenta puede quedar
    // en negativo: es la naturaleza de las cuentas acreedoras leídas en
    // convención deudor-normal. El post-check de Cuenta NO debe rebotar.
    let exec = Executor::load_module(accounting_module()).expect("load module");
    let mut store = MemoryStore::new();

    let caja = cuenta(&mut store, "1010", "Caja", "activo", 0, "USD");
    let capital = cuenta(&mut store, "3010", "Capital", "patrimonio", 0, "USD");

    asentar(&exec, &mut store, caja, capital, 1_000_000).expect("aporte de capital");
    assert_eq!(saldo(&store, capital), -1_000_000, "patrimonio en negativo es válido");
}

#[test]
fn monto_negativo_rechazado_por_post_check() {
    let exec = Executor::load_module(accounting_module()).expect("load module");
    let mut store = MemoryStore::new();
    let a = cuenta(&mut store, "1010", "Caja", "activo", 0, "USD");
    let b = cuenta(&mut store, "4010", "Ventas", "ingreso", 0, "USD");

    let result = asentar(&exec, &mut store, a, b, -500_000);
    match result {
        Err(ExecError::SchemaPostCreate { entity, .. }) => assert_eq!(entity, "Asiento"),
        other => panic!("esperaba SchemaPostCreate en Asiento, obtuve {:?}", other),
    }
    // Estado intacto: el executor rechazó antes de aplicar.
    assert_eq!(saldo(&store, a), 0);
    assert_eq!(saldo(&store, b), 0);
}

#[test]
fn asiento_entre_monedas_distintas_rechazado() {
    let exec = Executor::load_module(accounting_module()).expect("load module");
    let mut store = MemoryStore::new();
    let usd = cuenta(&mut store, "1010", "Caja USD", "activo", 0, "USD");
    let eur = cuenta(&mut store, "1011", "Caja EUR", "activo", 0, "EUR");

    let result = asentar(&exec, &mut store, usd, eur, 100_000);
    assert!(
        matches!(result, Err(ExecError::Rhai(_))),
        "esperaba throw del script por monedas distintas, obtuve {:?}",
        result
    );
    assert_eq!(saldo(&store, usd), 0);
    assert_eq!(saldo(&store, eur), 0);
}

#[test]
fn misma_cuenta_debito_y_credito_rechazada() {
    let exec = Executor::load_module(accounting_module()).expect("load module");
    let mut store = MemoryStore::new();
    let caja = cuenta(&mut store, "1010", "Caja", "activo", 0, "USD");

    let result = asentar(&exec, &mut store, caja, caja, 100_000);
    assert!(
        matches!(result, Err(ExecError::DuplicateInputId { .. })),
        "esperaba DuplicateInputId, obtuve {:?}",
        result
    );
    assert_eq!(saldo(&store, caja), 0);
}
