//! Contrato fiscal cableado a comprobantes: Factura (facturacion) y Compra
//! (compras) validan el documento del cliente/proveedor — FORMA por el
//! contrato Nickel `ConDocumentoFiscal`, DÍGITO VERIFICADOR por el módulo
//! Rhai compartido "fiscal" dentro del morfismo de alta.
//!
//! Es opt-in: sin `documento` en los params, el comprobante se emite igual
//! (los tests preexistentes de facturacion/compras lo siguen probando).

use std::path::{Path, PathBuf};

use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use serde_json::{json, Value};
use uuid::Uuid;

fn module(nombre: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("modules")
        .join(nombre)
}

fn cuenta(store: &mut MemoryStore, codigo: &str, tipo: &str) -> Uuid {
    let id = Uuid::new_v4();
    store.seed(
        "Cuenta",
        id,
        json!({ "id": id.to_string(), "codigo": codigo, "nombre": codigo,
                "tipo": tipo, "saldo": 0_i64, "moneda": "USD" }),
    );
    id
}

/// Emite una factura con datos fiscales del cliente. `Ok` si el alta pasó.
fn emitir_factura_fiscal(pais: &str, documento: &str) -> Result<Uuid, String> {
    let exec = Executor::load_module(module("facturacion")).unwrap();
    let mut store = MemoryStore::new();
    let clientes = cuenta(&mut store, "1100", "activo");
    let ventas = cuenta(&mut store, "4010", "ingreso");
    let iva = cuenta(&mut store, "2110", "pasivo");
    let fid = Uuid::new_v4();
    exec.run(
        &mut store,
        "emitir_factura",
        &[("clientes_cta", clientes), ("ventas_cta", ventas), ("iva_cta", iva)],
        json!({
            "cliente": "ACME S.A.", "fecha": "2026-06-28", "factura_id": fid.to_string(),
            "neto": 1000_i64, "tasa": 18_i64, "pais": pais, "documento": documento,
        }),
    )
    .map_err(|e| e.to_string())?;
    let _ = store.load("Factura", fid).ok_or("factura no persistida")?;
    Ok(fid)
}

/// Registra una compra con datos fiscales del proveedor. `Ok` si pasó.
fn registrar_compra_fiscal(pais: &str, documento: &str) -> Result<Uuid, String> {
    let exec = Executor::load_module(module("compras")).unwrap();
    let mut store = MemoryStore::new();
    let gasto = cuenta(&mut store, "5010", "gasto");
    let iva = cuenta(&mut store, "1190", "activo");
    let prov = cuenta(&mut store, "2010", "pasivo");
    let cid = Uuid::new_v4();
    exec.run(
        &mut store,
        "registrar_compra",
        &[("compra_cta", gasto), ("iva_cta", iva), ("proveedores_cta", prov)],
        json!({
            "proveedor": "Insumos SRL", "fecha": "2026-06-28", "compra_id": cid.to_string(),
            "neto": 1000_i64, "tasa": 18_i64, "pais": pais, "documento": documento,
        }),
    )
    .map_err(|e| e.to_string())?;
    Ok(cid)
}

// ── Factura ────────────────────────────────────────────────────────────────

#[test]
fn factura_con_rif_valido_guarda_datos_fiscales() {
    let exec = Executor::load_module(module("facturacion")).unwrap();
    let mut store = MemoryStore::new();
    let clientes = cuenta(&mut store, "1100", "activo");
    let ventas = cuenta(&mut store, "4010", "ingreso");
    let iva = cuenta(&mut store, "2110", "pasivo");
    let fid = Uuid::new_v4();
    exec.run(
        &mut store,
        "emitir_factura",
        &[("clientes_cta", clientes), ("ventas_cta", ventas), ("iva_cta", iva)],
        json!({
            "cliente": "ACME S.A.", "fecha": "2026-06-28", "factura_id": fid.to_string(),
            "neto": 1000_i64, "tasa": 18_i64, "pais": "VE", "documento": "G-20009048-0",
        }),
    )
    .expect("factura con RIF válido se emite");
    let f = store.load("Factura", fid).expect("factura");
    assert_eq!(f.get("documento").and_then(Value::as_str), Some("G-20009048-0"));
    assert_eq!(f.get("pais").and_then(Value::as_str), Some("VE"));
    assert_eq!(f.get("total").and_then(Value::as_i64), Some(1180), "el asiento sigue intacto");
}

#[test]
fn factura_con_dv_invalido_rebota() {
    // RIF con formato OK pero dígito verificador cambiado → lo corta el morfismo.
    assert!(emitir_factura_fiscal("VE", "G-20009048-5").is_err());
}

#[test]
fn factura_con_formato_invalido_rebota() {
    // "hola" no matchea el regex VE → lo corta el post-check Nickel del kernel.
    assert!(emitir_factura_fiscal("VE", "hola").is_err());
}

#[test]
fn factura_con_cuit_argentino_valido() {
    assert!(emitir_factura_fiscal("AR", "20-17254359-7").is_ok());
}

// ── Compra ─────────────────────────────────────────────────────────────────

#[test]
fn compra_con_cuit_valido_guarda_datos_fiscales() {
    let exec = Executor::load_module(module("compras")).unwrap();
    let mut store = MemoryStore::new();
    let gasto = cuenta(&mut store, "5010", "gasto");
    let iva = cuenta(&mut store, "1190", "activo");
    let prov = cuenta(&mut store, "2010", "pasivo");
    let cid = Uuid::new_v4();
    exec.run(
        &mut store,
        "registrar_compra",
        &[("compra_cta", gasto), ("iva_cta", iva), ("proveedores_cta", prov)],
        json!({
            "proveedor": "Insumos SRL", "fecha": "2026-06-28", "compra_id": cid.to_string(),
            "neto": 1000_i64, "tasa": 18_i64, "pais": "AR", "documento": "20-17254359-7",
        }),
    )
    .expect("compra con CUIT válido se registra");
    let c = store.load("Compra", cid).expect("compra");
    assert_eq!(c.get("documento").and_then(Value::as_str), Some("20-17254359-7"));
    assert_eq!(c.get("pais").and_then(Value::as_str), Some("AR"));
    assert_eq!(c.get("total").and_then(Value::as_i64), Some(1180), "el asiento sigue intacto");
}

#[test]
fn compra_con_dv_invalido_rebota() {
    assert!(registrar_compra_fiscal("AR", "20-17254359-1").is_err());
}

#[test]
fn compra_con_rif_valido() {
    assert!(registrar_compra_fiscal("VE", "G-20009048-0").is_ok());
}
