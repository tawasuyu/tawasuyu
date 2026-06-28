//! Normalización del documento fiscal: el alta guarda la FORMA CANÓNICA del
//! país (RIF "J-12345678-9", RUT "12.345.678-5", CUIT "20-12345678-9"),
//! cualquiera sea la forma de entrada. Que el create no rebote prueba además
//! que la forma canónica sigue matcheando el regex del contrato Nickel.

use std::path::{Path, PathBuf};

use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use serde_json::{json, Value};
use uuid::Uuid;

fn module(nombre: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("modules").join(nombre)
}

fn cuenta(store: &mut MemoryStore, codigo: &str, tipo: &str) -> Uuid {
    let id = Uuid::new_v4();
    store.seed("Cuenta", id, json!({ "id": id.to_string(), "codigo": codigo, "nombre": codigo,
        "tipo": tipo, "saldo": 0_i64, "moneda": "USD" }));
    id
}

#[test]
fn cliente_guarda_documento_canonico() {
    let exec = Executor::load_module(module("crm")).unwrap();
    let mut store = MemoryStore::new();
    let id = Uuid::new_v4();
    // RUT chileno válido SIN puntos en la entrada.
    exec.run(&mut store, "registrar_cliente", &[], json!({
        "id": id.to_string(), "nombre": "X", "email": "x@x.com", "empresa": "X",
        "pais": "CL", "documento": "27962409-2",
    })).expect("alta con RUT válido sin puntos");
    let c = store.load("Cliente", id).expect("cliente");
    assert_eq!(c.get("documento").and_then(Value::as_str), Some("27.962.409-2"),
        "se guarda el RUT con puntos (forma canónica)");
}

#[test]
fn cliente_rif_venezolano_canonico() {
    let exec = Executor::load_module(module("crm")).unwrap();
    let mut store = MemoryStore::new();
    let id = Uuid::new_v4();
    // RIF válido pegado, sin guiones.
    exec.run(&mut store, "registrar_cliente", &[], json!({
        "id": id.to_string(), "nombre": "X", "email": "x@x.com", "empresa": "X",
        "pais": "VE", "documento": "G200090480",
    })).expect("alta con RIF válido pegado");
    let c = store.load("Cliente", id).expect("cliente");
    assert_eq!(c.get("documento").and_then(Value::as_str), Some("G-20009048-0"),
        "se guarda el RIF con guiones (forma canónica)");
}

#[test]
fn factura_guarda_cuit_canonico() {
    let exec = Executor::load_module(module("facturacion")).unwrap();
    let mut store = MemoryStore::new();
    let clientes = cuenta(&mut store, "1100", "activo");
    let ventas = cuenta(&mut store, "4010", "ingreso");
    let iva = cuenta(&mut store, "2110", "pasivo");
    let fid = Uuid::new_v4();
    // CUIT válido sin guiones.
    exec.run(&mut store, "emitir_factura",
        &[("clientes_cta", clientes), ("ventas_cta", ventas), ("iva_cta", iva)],
        json!({ "cliente": "ACME", "fecha": "2026-06-28", "factura_id": fid.to_string(),
            "neto": 1000_i64, "tasa": 18_i64, "pais": "AR", "documento": "20172543597" }))
        .expect("factura con CUIT válido sin guiones");
    let f = store.load("Factura", fid).expect("factura");
    assert_eq!(f.get("documento").and_then(Value::as_str), Some("20-17254359-7"),
        "la factura guarda el CUIT con guiones (forma canónica)");
}

#[test]
fn compra_guarda_rut_canonico() {
    let exec = Executor::load_module(module("compras")).unwrap();
    let mut store = MemoryStore::new();
    let gasto = cuenta(&mut store, "5010", "gasto");
    let iva = cuenta(&mut store, "1190", "activo");
    let prov = cuenta(&mut store, "2010", "pasivo");
    let cid = Uuid::new_v4();
    exec.run(&mut store, "registrar_compra",
        &[("compra_cta", gasto), ("iva_cta", iva), ("proveedores_cta", prov)],
        json!({ "proveedor": "Insumos", "fecha": "2026-06-28", "compra_id": cid.to_string(),
            "neto": 1000_i64, "tasa": 18_i64, "pais": "CL", "documento": "27962409-2" }))
        .expect("compra con RUT válido sin puntos");
    let c = store.load("Compra", cid).expect("compra");
    assert_eq!(c.get("documento").and_then(Value::as_str), Some("27.962.409-2"),
        "la compra guarda el RUT con puntos (forma canónica)");
}
