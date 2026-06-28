//! Tests del módulo `divisas` — ventas en moneda extranjera contabilizadas
//! en la moneda funcional (USD), con reconocimiento de la diferencia de
//! cambio al cobrar a otra cotización. Todo el libro queda en USD, así que
//! `conserve` cuadra y la ganancia/pérdida cambiaria surge del asiento.

use std::path::{Path, PathBuf};

use nakui_core::executor::{ExecError, Executor};
use nakui_core::store::{MemoryStore, Store};
use serde_json::{json, Value};
use uuid::Uuid;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}
fn divisas_module() -> PathBuf {
    workspace_root().join("modules/divisas")
}
fn saldo(store: &MemoryStore, id: Uuid) -> i64 {
    store.load("Cuenta", id).and_then(|v| v.get("saldo").and_then(Value::as_i64)).expect("cuenta")
}
fn cuenta(store: &mut MemoryStore, codigo: &str, tipo: &str) -> Uuid {
    let id = Uuid::new_v4();
    store.seed("Cuenta", id, json!({
        "id": id.to_string(), "codigo": codigo, "nombre": codigo,
        "tipo": tipo, "saldo": 0_i64, "moneda": "USD",
    }));
    id
}

/// Emite una factura en divisa: `monto_divisa` a `tasa` (×100). Devuelve su id.
fn vender(exec: &Executor, store: &mut MemoryStore, clientes: Uuid, ventas: Uuid, monto_divisa: i64, tasa: i64) -> Uuid {
    let fid = Uuid::new_v4();
    exec.run(store, "vender_divisa",
        &[("clientes_cta", clientes), ("ventas_cta", ventas)],
        json!({
            "cliente": "ACME GmbH", "moneda_origen": "EUR", "monto_divisa": monto_divisa,
            "tasa": tasa, "factura_id": fid.to_string(), "fecha": "2026-06-28",
        }),
    ).expect("vender_divisa");
    fid
}

#[test]
fn vender_divisa_contabiliza_en_funcional() {
    let exec = Executor::load_module(divisas_module()).expect("load module");
    let mut store = MemoryStore::new();
    let clientes = cuenta(&mut store, "1100", "activo");
    let ventas = cuenta(&mut store, "4010", "ingreso");

    // 1000 EUR a 1,10 → 1100 USD.
    let fid = vender(&exec, &mut store, clientes, ventas, 1000, 110);
    assert_eq!(saldo(&store, clientes), 1100, "CxC en USD funcional");
    assert_eq!(saldo(&store, ventas), -1100);

    let f = store.load("FacturaDivisa", fid).expect("factura");
    assert_eq!(f.get("monto_divisa").and_then(Value::as_i64), Some(1000));
    assert_eq!(f.get("monto_usd").and_then(Value::as_i64), Some(1100));
    assert_eq!(f.get("moneda_origen").and_then(Value::as_str), Some("EUR"));
    assert_eq!(saldo(&store, clientes) + saldo(&store, ventas), 0);
}

#[test]
fn cobrar_divisa_con_ganancia_de_cambio() {
    let exec = Executor::load_module(divisas_module()).expect("load module");
    let mut store = MemoryStore::new();
    let clientes = cuenta(&mut store, "1100", "activo");
    let ventas = cuenta(&mut store, "4010", "ingreso");
    let banco = cuenta(&mut store, "1020", "activo");
    let resultado = cuenta(&mut store, "4900", "ingreso"); // resultado por diferencias de cambio

    let fid = vender(&exec, &mut store, clientes, ventas, 1000, 110); // contabilizada a 1100

    // Cobro a 1,15 → 1150 USD. Diferencia = +50 (ganancia).
    let cobro_id = Uuid::new_v4();
    exec.run(&mut store, "cobrar_divisa",
        &[("banco_cta", banco), ("clientes_cta", clientes), ("resultado_cambio_cta", resultado), ("factura", fid)],
        json!({ "tasa": 115_i64, "cobro_id": cobro_id.to_string(), "fecha": "2026-07-01" }),
    ).expect("cobrar_divisa");

    assert_eq!(saldo(&store, banco), 1150, "el banco recibe el equivalente de hoy");
    assert_eq!(saldo(&store, clientes), 0, "la CxC se salda a su valor de libro");
    assert_eq!(saldo(&store, resultado), -50, "ganancia de cambio (ingreso)");

    let c = store.load("CobroDivisa", cobro_id).expect("cobro");
    assert_eq!(c.get("diferencia_cambio").and_then(Value::as_i64), Some(50));

    // El libro entero cuadra: Banco(1150) + Ventas(-1100) + Resultado(-50) = 0.
    let total: i64 = [clientes, ventas, banco, resultado].iter().map(|&c| saldo(&store, c)).sum();
    assert_eq!(total, 0, "vender + cobrar con FX conserva la balanza");
}

#[test]
fn cobrar_divisa_con_perdida_de_cambio() {
    let exec = Executor::load_module(divisas_module()).expect("load module");
    let mut store = MemoryStore::new();
    let clientes = cuenta(&mut store, "1100", "activo");
    let ventas = cuenta(&mut store, "4010", "ingreso");
    let banco = cuenta(&mut store, "1020", "activo");
    let resultado = cuenta(&mut store, "4900", "ingreso");

    let fid = vender(&exec, &mut store, clientes, ventas, 1000, 110); // 1100

    // Cobro a 1,05 → 1050 USD. Diferencia = -50 (pérdida).
    exec.run(&mut store, "cobrar_divisa",
        &[("banco_cta", banco), ("clientes_cta", clientes), ("resultado_cambio_cta", resultado), ("factura", fid)],
        json!({ "tasa": 105_i64, "cobro_id": Uuid::new_v4().to_string(), "fecha": "2026-07-01" }),
    ).expect("cobrar_divisa");

    assert_eq!(saldo(&store, banco), 1050);
    assert_eq!(saldo(&store, clientes), 0);
    assert_eq!(saldo(&store, resultado), 50, "pérdida de cambio (resultado deudor)");
    let total: i64 = [clientes, ventas, banco, resultado].iter().map(|&c| saldo(&store, c)).sum();
    assert_eq!(total, 0, "la pérdida de cambio también conserva la balanza");
}

#[test]
fn cobrar_divisa_ya_cobrada_rechazada() {
    let exec = Executor::load_module(divisas_module()).expect("load module");
    let mut store = MemoryStore::new();
    let clientes = cuenta(&mut store, "1100", "activo");
    let ventas = cuenta(&mut store, "4010", "ingreso");
    let banco = cuenta(&mut store, "1020", "activo");
    let resultado = cuenta(&mut store, "4900", "ingreso");
    let fid = vender(&exec, &mut store, clientes, ventas, 100, 100);

    let cobrar = |store: &mut MemoryStore| {
        exec.run(store, "cobrar_divisa",
            &[("banco_cta", banco), ("clientes_cta", clientes), ("resultado_cambio_cta", resultado), ("factura", fid)],
            json!({ "tasa": 100_i64, "cobro_id": Uuid::new_v4().to_string(), "fecha": "2026-07-01" }),
        )
    };
    cobrar(&mut store).expect("primer cobro");
    assert!(matches!(cobrar(&mut store), Err(ExecError::Rhai(_))), "doble cobro rechazado");
}
