//! Tests del módulo `inventario` — stock valuado a costo promedio
//! ponderado con reflejo contable en `accounting` (cross-module).
//!
//! La tesis: recibir/despachar mercadería mueve el valor del stock y la
//! cuenta de Inventario por el mismo importe (perpetuo), y el ASIENTO
//! cuadra siempre (Σ Δ Cuenta.saldo = 0 vía `conserve`). El promedio
//! ponderado se mantiene sin guardarlo: costo = valor_total / cantidad.

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

fn inventario_module() -> PathBuf {
    workspace_root().join("modules/inventario")
}

fn saldo(store: &MemoryStore, id: Uuid) -> i64 {
    store.load("Cuenta", id).and_then(|v| v.get("saldo").and_then(Value::as_i64)).expect("cuenta")
}
fn prod_field(store: &MemoryStore, id: Uuid, field: &str) -> i64 {
    store.load("Producto", id).and_then(|v| v.get(field).and_then(Value::as_i64)).expect("producto")
}

fn cuenta(store: &mut MemoryStore, codigo: &str, tipo: &str) -> Uuid {
    let id = Uuid::new_v4();
    store.seed("Cuenta", id, json!({
        "id": id.to_string(), "codigo": codigo, "nombre": codigo,
        "tipo": tipo, "saldo": 0_i64, "moneda": "USD",
    }));
    id
}
fn producto(store: &mut MemoryStore, sku: &str) -> Uuid {
    let id = Uuid::new_v4();
    store.seed("Producto", id, json!({
        "id": id.to_string(), "sku": sku, "nombre": sku,
        "cantidad": 0_i64, "valor_total": 0_i64, "moneda": "USD",
    }));
    id
}

fn recibir(exec: &Executor, store: &mut MemoryStore, prod: Uuid, inv: Uuid, contra: Uuid, cantidad: i64, costo_unit: i64) {
    exec.run(store, "recibir_mercaderia",
        &[("producto", prod), ("inventario_cta", inv), ("contrapartida_cta", contra)],
        json!({ "cantidad": cantidad, "costo_unitario": costo_unit, "fecha": "2026-06-28", "movimiento_id": Uuid::new_v4().to_string() }),
    ).expect("recibir debe pasar");
}

#[test]
fn recibir_sube_stock_valor_y_asienta() {
    let exec = Executor::load_module(inventario_module()).expect("load module");
    let mut store = MemoryStore::new();
    let p = producto(&mut store, "cafe-hn");
    let inv = cuenta(&mut store, "1300", "activo");       // Inventario
    let prov = cuenta(&mut store, "2010", "pasivo");      // Proveedores
    let mov_id = Uuid::new_v4();

    let ops = exec.run(&mut store, "recibir_mercaderia",
        &[("producto", p), ("inventario_cta", inv), ("contrapartida_cta", prov)],
        json!({ "cantidad": 10_i64, "costo_unitario": 100_i64, "fecha": "2026-06-28", "movimiento_id": mov_id.to_string() }),
    ).expect("recibir");

    assert_eq!(ops.len(), 5, "2 sets producto + 2 sets cuentas + 1 create movimiento");
    assert_eq!(prod_field(&store, p, "cantidad"), 10);
    assert_eq!(prod_field(&store, p, "valor_total"), 1000);
    assert_eq!(saldo(&store, inv), 1000, "el activo Inventario sube por el costo");
    assert_eq!(saldo(&store, prov), -1000, "la deuda con proveedores sube");
    // El asiento cuadra.
    assert_eq!(saldo(&store, inv) + saldo(&store, prov), 0);

    let m = store.load("MovimientoStock", mov_id).expect("movimiento");
    assert_eq!(m.get("tipo").and_then(Value::as_str), Some("entrada"));
    assert_eq!(m.get("costo_total").and_then(Value::as_i64), Some(1000));
}

#[test]
fn promedio_ponderado_y_cogs() {
    let exec = Executor::load_module(inventario_module()).expect("load module");
    let mut store = MemoryStore::new();
    let p = producto(&mut store, "cafe-hn");
    let inv = cuenta(&mut store, "1300", "activo");
    let prov = cuenta(&mut store, "2010", "pasivo");
    let cogs = cuenta(&mut store, "5020", "gasto");

    // 10 @ 100 y luego 10 @ 200 → cantidad 20, valor 3000 (promedio 150).
    recibir(&exec, &mut store, p, inv, prov, 10, 100);
    recibir(&exec, &mut store, p, inv, prov, 10, 200);
    assert_eq!(prod_field(&store, p, "cantidad"), 20);
    assert_eq!(prod_field(&store, p, "valor_total"), 3000);

    // Despachar 10 → costo de salida = 3000*10/20 = 1500 (al promedio).
    let ops = exec.run(&mut store, "despachar_mercaderia",
        &[("producto", p), ("inventario_cta", inv), ("cogs_cta", cogs)],
        json!({ "cantidad": 10_i64, "fecha": "2026-06-29", "movimiento_id": Uuid::new_v4().to_string() }),
    ).expect("despachar");

    assert_eq!(ops.len(), 5);
    assert_eq!(prod_field(&store, p, "cantidad"), 10);
    assert_eq!(prod_field(&store, p, "valor_total"), 1500, "queda el valor del stock restante");
    assert_eq!(saldo(&store, cogs), 1500, "el COGS toma el costo promedio");
    assert_eq!(saldo(&store, inv), 3000 - 1500, "Inventario baja por el costo de salida");
    // El libro entero cuadra: Inventario(1500) + Proveedores(-3000) + COGS(1500) = 0.
    assert_eq!(saldo(&store, inv) + saldo(&store, prov) + saldo(&store, cogs), 0);
}

#[test]
fn despachar_todo_deja_valor_en_cero() {
    let exec = Executor::load_module(inventario_module()).expect("load module");
    let mut store = MemoryStore::new();
    let p = producto(&mut store, "cafe-hn");
    let inv = cuenta(&mut store, "1300", "activo");
    let prov = cuenta(&mut store, "2010", "pasivo");
    let cogs = cuenta(&mut store, "5020", "gasto");

    recibir(&exec, &mut store, p, inv, prov, 7, 143); // valor 1001, cantidad 7
    exec.run(&mut store, "despachar_mercaderia",
        &[("producto", p), ("inventario_cta", inv), ("cogs_cta", cogs)],
        json!({ "cantidad": 7_i64, "fecha": "2026-06-29", "movimiento_id": Uuid::new_v4().to_string() }),
    ).expect("despachar todo");

    assert_eq!(prod_field(&store, p, "cantidad"), 0);
    assert_eq!(prod_field(&store, p, "valor_total"), 0, "vaciar el stock deja valor exacto en 0");
    assert_eq!(saldo(&store, inv), 0, "Inventario vuelve a 0");
}

#[test]
fn sobregiro_de_stock_rechazado() {
    let exec = Executor::load_module(inventario_module()).expect("load module");
    let mut store = MemoryStore::new();
    let p = producto(&mut store, "cafe-hn");
    let inv = cuenta(&mut store, "1300", "activo");
    let prov = cuenta(&mut store, "2010", "pasivo");
    let cogs = cuenta(&mut store, "5020", "gasto");
    recibir(&exec, &mut store, p, inv, prov, 5, 100);

    // Despachar 9 con sólo 5 en mano → NoNegativo rebota.
    let result = exec.run(&mut store, "despachar_mercaderia",
        &[("producto", p), ("inventario_cta", inv), ("cogs_cta", cogs)],
        json!({ "cantidad": 9_i64, "fecha": "2026-06-29", "movimiento_id": Uuid::new_v4().to_string() }),
    );
    match result {
        Err(ExecError::SchemaPost { role, entity, .. }) => {
            assert_eq!(role, "producto");
            assert_eq!(entity, "Producto");
        }
        other => panic!("esperaba SchemaPost en producto, obtuve {:?}", other),
    }
    assert_eq!(prod_field(&store, p, "cantidad"), 5, "el stock no se tocó");
}
