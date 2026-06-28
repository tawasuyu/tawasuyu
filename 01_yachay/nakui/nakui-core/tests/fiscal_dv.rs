//! Tests del dígito verificador (morfismo `registrar_cliente` de crm).
//!
//! El morfismo Rhai valida la aritmética módulo 11/10 del documento ANTES de
//! crear el Cliente; el kernel valida la FORMA con Nickel en el post-check.
//! Los vectores «válidos» son números reales verificados contra fuentes
//! públicas (ver comentarios del .rhai); los «inválidos» son el mismo número
//! con el DV manipulado.

use std::path::{Path, PathBuf};

use nakui_core::executor::Executor;
use nakui_core::store::MemoryStore;
use serde_json::json;
use uuid::Uuid;

fn crm_module() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("modules/crm")
}

/// Intenta registrar un cliente con (pais, documento). `true` si el alta pasó
/// (DV ok + forma ok), `false` si el kernel/morfismo la rechazó.
fn alta(pais: &str, documento: &str) -> bool {
    let exec = Executor::load_module(crm_module()).expect("cargar crm");
    let mut store = MemoryStore::new();
    let id = Uuid::new_v4();
    exec.run(
        &mut store,
        "registrar_cliente",
        &[],
        json!({
            "id": id.to_string(),
            "nombre": "Tercero S.A.",
            "email": "contacto@example.com",
            "empresa": "Tercero S.A.",
            "pais": pais,
            "documento": documento,
        }),
    )
    .is_ok()
}

// ── Chile · RUT ────────────────────────────────────────────────────────────
#[test]
fn cl_rut_dv_valido_e_invalido() {
    assert!(alta("CL", "27.962.409-2"), "RUT real válido (DV 2)");
    assert!(alta("CL", "12345678-5"), "RUT válido sin puntos (DV 5)");
    assert!(!alta("CL", "27.962.409-3"), "DV cambiado debe rebotar");
    assert!(!alta("CL", "12345678-9"), "DV erróneo (real es 5)");
}

// ── Argentina · CUIT ───────────────────────────────────────────────────────
#[test]
fn ar_cuit_dv_valido_e_invalido() {
    assert!(alta("AR", "20-17254359-7"), "CUIT real válido (DV 7)");
    assert!(alta("AR", "20-12345678-6"), "CUIT válido (DV 6)");
    assert!(!alta("AR", "20-17254359-1"), "DV cambiado debe rebotar");
    assert!(!alta("AR", "20-12345678-9"), "DV erróneo (real es 6)");
}

// ── Ecuador · cédula ───────────────────────────────────────────────────────
#[test]
fn ec_cedula_dv_valido_e_invalido() {
    assert!(alta("EC", "1710034065"), "cédula real válida (DV 5)");
    assert!(!alta("EC", "1710034060"), "DV cambiado debe rebotar");
    assert!(!alta("EC", "1710034061"), "DV erróneo");
}

// ── Venezuela · RIF y cédula ───────────────────────────────────────────────
#[test]
fn ve_rif_dv_valido_e_invalido() {
    assert!(alta("VE", "G-20009048-0"), "RIF real válido (DV 0)");
    assert!(!alta("VE", "G-20009048-5"), "DV cambiado debe rebotar");
    assert!(!alta("VE", "J-200090480"), "mismo cuerpo, DV erróneo");
}

#[test]
fn ve_cedula_sin_dv_pasa() {
    // La cédula (letra + ≤8 dígitos) no lleva dígito verificador: sólo forma.
    assert!(alta("VE", "V-12345678"), "cédula venezolana sin DV");
}

// ── Perú · RUC (módulo 11) ─────────────────────────────────────────────────
#[test]
fn pe_ruc_dv_valido_e_invalido() {
    assert!(alta("PE", "20306051017"), "RUC real válido (DV 7)");
    assert!(!alta("PE", "20306051011"), "DV cambiado debe rebotar");
    assert!(alta("PE", "12345678"), "DNI (8 díg) no lleva DV → pasa por forma");
}

// ── Colombia · NIT (módulo 11, primos) ─────────────────────────────────────
#[test]
fn co_nit_dv_valido_e_invalido() {
    assert!(alta("CO", "800197268-4"), "NIT real válido (DV 4)");
    assert!(!alta("CO", "800197268-1"), "DV cambiado debe rebotar");
    assert!(alta("CO", "1234567"), "cédula sin guion → sin DV, pasa por forma");
}

// ── México · RFC (módulo 11, tabla SAT) ────────────────────────────────────
#[test]
fn mx_rfc_dv_valido_e_invalido() {
    assert!(alta("MX", "GODE561231GR8"), "RFC real válido (DV 8)");
    assert!(!alta("MX", "GODE561231GR0"), "DV cambiado debe rebotar");
}

// ── Bolivia: sin algoritmo de DV (sufijo asignado, no de control) ───────────
#[test]
fn bo_pasa_por_forma_sin_dv() {
    assert!(alta("BO", "1234567"), "BO valida forma, sin verificación de DV");
}

// ── La forma sigue gateada por Nickel aunque el DV no aplique ───────────────
#[test]
fn forma_invalida_rebota_aunque_no_haya_dv() {
    // "hola" no matchea el regex VE → el post-check Nickel del kernel rebota.
    assert!(!alta("VE", "hola"));
}
