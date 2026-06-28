//! Tests de integración del módulo `compras` — facturas de compra y pagos
//! a proveedor que ASIENTAN en el libro de `accounting` (cross-module).
//! Espejo de `facturacion` del lado de cuentas por pagar.
//!
//! La tesis: registrar una compra mueve tres cuentas (gasto/inventario,
//! IVA crédito fiscal y Proveedores) de forma balanceada, y pagarla mueve
//! el total de Proveedores a Banco. El kernel exige Σ Δ Cuenta.saldo = 0
//! por moneda vía `conserve`, así que una compra/pago nunca descuadra.

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

fn compras_module() -> PathBuf {
    workspace_root().join("modules/compras")
}

fn saldo(store: &MemoryStore, id: Uuid) -> i64 {
    store
        .load("Cuenta", id)
        .and_then(|v| v.get("saldo").and_then(Value::as_i64))
        .expect("cuenta con saldo")
}

fn cuenta(store: &mut MemoryStore, codigo: &str, tipo: &str, moneda: &str) -> Uuid {
    let id = Uuid::new_v4();
    store.seed(
        "Cuenta",
        id,
        json!({
            "id": id.to_string(), "codigo": codigo, "nombre": codigo,
            "tipo": tipo, "saldo": 0_i64, "moneda": moneda,
        }),
    );
    id
}

/// Cuentas de una compra: gasto (o inventario), IVA crédito fiscal
/// (activo), Proveedores (pasivo) y Banco (activo).
fn cuentas_compra(store: &mut MemoryStore, moneda: &str) -> (Uuid, Uuid, Uuid, Uuid) {
    (
        cuenta(store, "5010", "gasto", moneda),   // Gastos
        cuenta(store, "1190", "activo", moneda),  // IVA crédito fiscal
        cuenta(store, "2010", "pasivo", moneda),  // Proveedores
        cuenta(store, "1020", "activo", moneda),  // Banco
    )
}

fn registrar(
    exec: &Executor,
    store: &mut MemoryStore,
    gasto: Uuid,
    iva: Uuid,
    proveedores: Uuid,
    neto: i64,
    tasa: i64,
    compra_id: Uuid,
) -> Result<Vec<nakui_core::delta::FieldOp>, ExecError> {
    exec.run(
        store,
        "registrar_compra",
        &[
            ("compra_cta", gasto),
            ("iva_cta", iva),
            ("proveedores_cta", proveedores),
        ],
        json!({
            "proveedor": "Insumos SRL",
            "fecha": "2026-06-28",
            "compra_id": compra_id.to_string(),
            "neto": neto,
            "tasa": tasa,
        }),
    )
}

#[test]
fn registrar_compra_asienta_balanceado() {
    let exec = Executor::load_module(compras_module()).expect("load module");
    let mut store = MemoryStore::new();
    let (gasto, iva, proveedores, _banco) = cuentas_compra(&mut store, "USD");

    // Neto 1000, IVA 18% → impuesto 180, total 1180.
    let compra_id = Uuid::new_v4();
    let ops = registrar(&exec, &mut store, gasto, iva, proveedores, 1000, 18, compra_id)
        .expect("compra registrada");

    assert_eq!(ops.len(), 4, "3 sets (Gasto/IVA/Proveedores) + 1 create (Compra)");
    assert_eq!(saldo(&store, gasto), 1000); // el gasto sube por el neto
    assert_eq!(saldo(&store, iva), 180); // IVA crédito fiscal (activo) sube
    assert_eq!(saldo(&store, proveedores), -1180); // la deuda (pasivo) sube

    let c = store.load("Compra", compra_id).expect("compra");
    assert_eq!(c.get("total").and_then(Value::as_i64), Some(1180));
    assert_eq!(c.get("estado").and_then(Value::as_str), Some("registrada"));

    let total: i64 = [gasto, iva, proveedores].iter().map(|&c| saldo(&store, c)).sum();
    assert_eq!(total, 0, "la compra conserva la balanza");
}

#[test]
fn pagar_compra_salda_la_deuda() {
    let exec = Executor::load_module(compras_module()).expect("load module");
    let mut store = MemoryStore::new();
    let (gasto, iva, proveedores, banco) = cuentas_compra(&mut store, "USD");

    let compra_id = Uuid::new_v4();
    registrar(&exec, &mut store, gasto, iva, proveedores, 1000, 18, compra_id).expect("compra");
    assert_eq!(saldo(&store, proveedores), -1180);

    // Pagar la compra: el total sale del banco hacia el proveedor.
    let pago_id = Uuid::new_v4();
    let ops = exec
        .run(
            &mut store,
            "pagar",
            &[
                ("banco_cta", banco),
                ("proveedores_cta", proveedores),
                ("compra", compra_id),
            ],
            json!({ "fecha": "2026-06-30", "pago_id": pago_id.to_string() }),
        )
        .expect("pago debe pasar");

    assert_eq!(ops.len(), 4, "2 sets cuentas + 1 set estado compra + 1 create Pago");
    assert_eq!(saldo(&store, proveedores), 0, "la deuda queda saldada");
    assert_eq!(saldo(&store, banco), -1180, "el banco pagó el total");

    let c = store.load("Compra", compra_id).expect("compra");
    assert_eq!(c.get("estado").and_then(Value::as_str), Some("pagada"));
    let pago = store.load("Pago", pago_id).expect("pago");
    assert_eq!(pago.get("monto").and_then(Value::as_i64), Some(1180));

    let total: i64 = [gasto, iva, proveedores, banco].iter().map(|&c| saldo(&store, c)).sum();
    assert_eq!(total, 0, "registrar + pagar conserva la balanza");
}

#[test]
fn pagar_compra_ya_pagada_rechazada() {
    let exec = Executor::load_module(compras_module()).expect("load module");
    let mut store = MemoryStore::new();
    let (gasto, iva, proveedores, banco) = cuentas_compra(&mut store, "USD");
    let compra_id = Uuid::new_v4();
    registrar(&exec, &mut store, gasto, iva, proveedores, 500, 0, compra_id).expect("compra");

    let pagar = |store: &mut MemoryStore| {
        exec.run(
            store,
            "pagar",
            &[
                ("banco_cta", banco),
                ("proveedores_cta", proveedores),
                ("compra", compra_id),
            ],
            json!({ "fecha": "2026-06-30", "pago_id": Uuid::new_v4().to_string() }),
        )
    };
    pagar(&mut store).expect("primer pago pasa");
    let snd = pagar(&mut store);
    assert!(
        matches!(snd, Err(ExecError::Rhai(_))),
        "esperaba throw por compra ya pagada, obtuve {:?}",
        snd
    );
    assert_eq!(saldo(&store, banco), -500, "el banco no se movió en el rechazo");
}

#[test]
fn compra_monedas_distintas_rechazada() {
    let exec = Executor::load_module(compras_module()).expect("load module");
    let mut store = MemoryStore::new();
    let gasto = cuenta(&mut store, "5010", "gasto", "USD");
    let iva = cuenta(&mut store, "1190", "activo", "USD");
    let proveedores = cuenta(&mut store, "2010", "pasivo", "EUR");

    let result = registrar(&exec, &mut store, gasto, iva, proveedores, 1000, 18, Uuid::new_v4());
    assert!(
        matches!(result, Err(ExecError::Rhai(_))),
        "esperaba throw por monedas distintas, obtuve {:?}",
        result
    );
    assert_eq!(saldo(&store, gasto), 0);
}
