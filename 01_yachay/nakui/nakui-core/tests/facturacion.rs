//! Tests de integración del módulo `facturacion` — emisión de facturas
//! que ASIENTAN en el libro de `accounting` (cross-module).
//!
//! La tesis: emitir una factura mueve tres cuentas (Clientes/Ventas/IVA)
//! de forma balanceada; el kernel exige Σ Δ Cuenta.saldo = 0 por moneda
//! vía la regla `conserve`, así que una factura NUNCA descuadra el libro.
//! El impuesto se deriva de neto×tasa/100 y total = neto + impuesto.

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

fn facturacion_module() -> PathBuf {
    workspace_root().join("modules/facturacion")
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
            "id": id.to_string(),
            "codigo": codigo,
            "nombre": codigo,
            "tipo": tipo,
            "saldo": 0_i64,
            "moneda": moneda,
        }),
    );
    id
}

/// Cuentas de control típicas de facturación: CxC (activo), Ventas
/// (ingreso), IVA por pagar (pasivo).
fn cuentas_factura(store: &mut MemoryStore, moneda: &str) -> (Uuid, Uuid, Uuid) {
    (
        cuenta(store, "1100", "activo", moneda),   // Clientes (CxC)
        cuenta(store, "4010", "ingreso", moneda),  // Ventas
        cuenta(store, "2110", "pasivo", moneda),   // IVA por pagar
    )
}

fn emitir(
    exec: &Executor,
    store: &mut MemoryStore,
    cuentas: (Uuid, Uuid, Uuid),
    neto: i64,
    tasa: i64,
    factura_id: Uuid,
) -> Result<Vec<nakui_core::delta::FieldOp>, ExecError> {
    let (clientes, ventas, iva) = cuentas;
    exec.run(
        store,
        "emitir_factura",
        &[
            ("clientes_cta", clientes),
            ("ventas_cta", ventas),
            ("iva_cta", iva),
        ],
        json!({
            "cliente": "ACME S.A.",
            "fecha": "2026-06-28",
            "factura_id": factura_id.to_string(),
            "neto": neto,
            "tasa": tasa,
        }),
    )
}

#[test]
fn factura_emitida_asienta_balanceado() {
    let exec = Executor::load_module(facturacion_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cs = cuentas_factura(&mut store, "USD");
    let (clientes, ventas, iva) = cs;

    // Neto 1000, IVA 18% → impuesto 180, total 1180.
    let factura_id = Uuid::new_v4();
    let ops = emitir(&exec, &mut store, cs, 1000, 18, factura_id).expect("factura emitida");

    assert_eq!(ops.len(), 4, "3 sets (Clientes/Ventas/IVA) + 1 create (Factura)");
    assert_eq!(saldo(&store, clientes), 1180); // CxC sube por el total
    assert_eq!(saldo(&store, ventas), -1000); // ingreso acreditado
    assert_eq!(saldo(&store, iva), -180); // IVA por pagar acreditado

    let f = store.load("Factura", factura_id).expect("factura persistida");
    assert_eq!(f.get("neto").and_then(Value::as_i64), Some(1000));
    assert_eq!(f.get("impuesto").and_then(Value::as_i64), Some(180));
    assert_eq!(f.get("total").and_then(Value::as_i64), Some(1180));
    assert_eq!(f.get("estado").and_then(Value::as_str), Some("emitida"));
    assert_eq!(f.get("cliente").and_then(Value::as_str), Some("ACME S.A."));

    // El libro queda cuadrado tras la factura.
    let total: i64 = [clientes, ventas, iva].iter().map(|&c| saldo(&store, c)).sum();
    assert_eq!(total, 0, "la factura conserva la balanza");
}

#[test]
fn factura_sin_impuesto_no_toca_iva() {
    let exec = Executor::load_module(facturacion_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cs = cuentas_factura(&mut store, "USD");
    let (clientes, ventas, iva) = cs;

    let ops = emitir(&exec, &mut store, cs, 500, 0, Uuid::new_v4()).expect("factura exenta");
    assert_eq!(ops.len(), 4);
    assert_eq!(saldo(&store, clientes), 500);
    assert_eq!(saldo(&store, ventas), -500);
    assert_eq!(saldo(&store, iva), 0, "tasa 0 → IVA intacto");
}

#[test]
fn factura_con_iva_truncado_sigue_cuadrando() {
    // neto 999, tasa 21 → impuesto = 209 (209.79 truncado). total = 1208.
    // Aunque el impuesto se trunque, total = neto + impuesto exacto, así
    // que la balanza cierra: el redondeo no rompe la partida doble.
    let exec = Executor::load_module(facturacion_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cs = cuentas_factura(&mut store, "USD");
    let (clientes, ventas, iva) = cs;
    let fid = Uuid::new_v4();

    emitir(&exec, &mut store, cs, 999, 21, fid).expect("factura emitida");
    let f = store.load("Factura", fid).expect("factura");
    assert_eq!(f.get("impuesto").and_then(Value::as_i64), Some(209));
    assert_eq!(f.get("total").and_then(Value::as_i64), Some(1208));

    let total: i64 = [clientes, ventas, iva].iter().map(|&c| saldo(&store, c)).sum();
    assert_eq!(total, 0, "la balanza cierra pese al truncamiento del IVA");
}

#[test]
fn facturar_con_lineas_calcula_neto_y_asienta() {
    // facturar suma las líneas para el neto y crea una LineaFactura por
    // ítem. 2×500 + 1×300 = 1300 neto; IVA 18% = 234; total 1534.
    let exec = Executor::load_module(facturacion_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cs = cuentas_factura(&mut store, "USD");
    let (clientes, ventas, iva) = cs;

    let factura_id = Uuid::new_v4();
    let l1 = Uuid::new_v4();
    let l2 = Uuid::new_v4();
    let ops = exec
        .run(
            &mut store,
            "facturar",
            &[
                ("clientes_cta", clientes),
                ("ventas_cta", ventas),
                ("iva_cta", iva),
            ],
            json!({
                "cliente": "ACME S.A.",
                "fecha": "2026-06-28",
                "factura_id": factura_id.to_string(),
                "tasa": 18_i64,
                "lineas": [
                    { "id": l1.to_string(), "concepto": "Servicio de diseño", "cantidad": 2, "precio_unitario": 500 },
                    { "id": l2.to_string(), "concepto": "Hosting anual", "cantidad": 1, "precio_unitario": 300 }
                ]
            }),
        )
        .expect("facturar debe pasar");

    // 2 creates LineaFactura + 3 sets + 1 create Factura = 6 ops.
    assert_eq!(ops.len(), 6);
    assert_eq!(saldo(&store, clientes), 1534);
    assert_eq!(saldo(&store, ventas), -1300);
    assert_eq!(saldo(&store, iva), -234);

    let f = store.load("Factura", factura_id).expect("factura");
    assert_eq!(f.get("neto").and_then(Value::as_i64), Some(1300));
    assert_eq!(f.get("impuesto").and_then(Value::as_i64), Some(234));
    assert_eq!(f.get("total").and_then(Value::as_i64), Some(1534));

    // Las dos líneas se persistieron con su subtotal y ligadas a la factura.
    let linea1 = store.load("LineaFactura", l1).expect("línea 1");
    assert_eq!(linea1.get("subtotal").and_then(Value::as_i64), Some(1000));
    assert_eq!(
        linea1.get("factura_id").and_then(Value::as_str),
        Some(factura_id.to_string().as_str())
    );
    let linea2 = store.load("LineaFactura", l2).expect("línea 2");
    assert_eq!(linea2.get("subtotal").and_then(Value::as_i64), Some(300));

    let total: i64 = [clientes, ventas, iva].iter().map(|&c| saldo(&store, c)).sum();
    assert_eq!(total, 0, "la factura con líneas conserva la balanza");
}

#[test]
fn factura_entre_monedas_distintas_rechazada() {
    let exec = Executor::load_module(facturacion_module()).expect("load module");
    let mut store = MemoryStore::new();
    // Clientes en USD pero IVA en EUR: el script rechaza.
    let clientes = cuenta(&mut store, "1100", "activo", "USD");
    let ventas = cuenta(&mut store, "4010", "ingreso", "USD");
    let iva = cuenta(&mut store, "2110", "pasivo", "EUR");

    let result = emitir(&exec, &mut store, (clientes, ventas, iva), 1000, 18, Uuid::new_v4());
    assert!(
        matches!(result, Err(ExecError::Rhai(_))),
        "esperaba throw por monedas distintas, obtuve {:?}",
        result
    );
    assert_eq!(saldo(&store, clientes), 0);
}
