//! Tests del pack fiscal compartido (`modules/_fiscal/documentos.ncl`) cableado
//! sobre la entidad `Cliente` del módulo crm.
//!
//! Valida por el camino REAL del kernel: carga el módulo con
//! `Executor::load_module` (que arma el bundle Nickel mergeando los schemas e
//! importa transitivamente el pack fiscal) y corre `vet` contra el
//! `schema_path` del executor — exactamente lo que hace `validate_entity`
//! internamente cuando el kernel siembra/escribe un Cliente.

use std::path::{Path, PathBuf};

use nakui_core::executor::Executor;
use nakui_core::nickel_validator::vet;
use serde_json::json;

fn crm_module() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("dir del módulo nakui sobre core/")
        .join("modules/crm")
}

/// Carga crm y valida un Cliente. `true` = pasa el contrato, `false` = rebota.
fn cliente_valido(state: serde_json::Value) -> bool {
    let exec = Executor::load_module(crm_module()).expect("cargar módulo crm");
    vet(&exec.schema_path, &state, "Cliente").is_ok()
}

fn cliente(pais: &str, documento: &str) -> serde_json::Value {
    json!({
        "id": "00000000-0000-0000-0000-000000000001",
        "nombre": "Tercero S.A.",
        "email": "contacto@example.com",
        "empresa": "Tercero S.A.",
        "pais": pais,
        "documento": documento,
    })
}

// ── Retrocompatibilidad ────────────────────────────────────────────────────

#[test]
fn cliente_sin_datos_fiscales_sigue_valido() {
    // El seed histórico de crm — sin pais/documento — no debe romperse.
    assert!(cliente_valido(json!({
        "id": "00000000-0000-0000-0000-0000000000aa",
        "nombre": "Cliente Viejo",
        "email": "viejo@example.com",
        "empresa": "Cliente Viejo",
    })));
}

#[test]
fn documento_sin_pais_rebota() {
    // Si das un documento, tenés que decir de qué país es.
    assert!(!cliente_valido(json!({
        "id": "00000000-0000-0000-0000-0000000000bb",
        "nombre": "Sin País",
        "email": "x@example.com",
        "empresa": "Sin País",
        "documento": "V-12345678",
    })));
}

#[test]
fn estructura_base_se_sigue_exigiendo() {
    // Falta `empresa` (campo requerido de la forma) aunque el fiscal pase.
    assert!(!cliente_valido(json!({
        "id": "00000000-0000-0000-0000-0000000000cc",
        "nombre": "Incompleto",
        "email": "x@example.com",
    })));
}

#[test]
fn pais_sin_patron_declarado_no_bloquea() {
    // Estados Unidos no está en el pack: no conocemos la regla, no trabamos.
    assert!(cliente_valido(cliente("US", "cualquier-cosa-123")));
}

// ── Documentos válidos por país ────────────────────────────────────────────

#[test]
fn venezuela_cedula_y_rif() {
    assert!(cliente_valido(cliente("VE", "V-12345678")), "cédula natural");
    assert!(cliente_valido(cliente("VE", "E-12345678")), "cédula extranjero");
    assert!(cliente_valido(cliente("VE", "J-123456789")), "RIF jurídico");
    assert!(cliente_valido(cliente("VE", "J-12345678-9")), "RIF con verificador");
}

#[test]
fn resto_de_paises_validos() {
    assert!(cliente_valido(cliente("BO", "1234567")), "Bolivia CI");
    assert!(cliente_valido(cliente("PE", "12345678")), "Perú DNI");
    assert!(cliente_valido(cliente("PE", "20123456789")), "Perú RUC");
    assert!(cliente_valido(cliente("EC", "1712345678")), "Ecuador cédula");
    assert!(cliente_valido(cliente("EC", "1712345678001")), "Ecuador RUC");
    assert!(cliente_valido(cliente("CO", "1234567890")), "Colombia cédula");
    assert!(cliente_valido(cliente("CL", "12.345.678-9")), "Chile RUT con puntos");
    assert!(cliente_valido(cliente("CL", "12345678-K")), "Chile RUT con K");
    assert!(cliente_valido(cliente("AR", "20-12345678-9")), "Argentina CUIT");
    assert!(cliente_valido(cliente("MX", "ABCD800101XYZ")), "México RFC física");
    assert!(cliente_valido(cliente("MX", "ABC800101XYZ")), "México RFC moral");
}

// ── Documentos mal formados por país ───────────────────────────────────────

#[test]
fn venezuela_documento_invalido_rebota() {
    assert!(!cliente_valido(cliente("VE", "hola")), "texto");
    assert!(!cliente_valido(cliente("VE", "12345678")), "sin prefijo de tipo");
    assert!(!cliente_valido(cliente("VE", "V-123")), "muy corto");
}

#[test]
fn otros_paises_invalidos_rebotan() {
    assert!(!cliente_valido(cliente("AR", "12345678")), "AR sin estructura CUIT");
    assert!(!cliente_valido(cliente("PE", "123")), "PE largo inválido");
    assert!(!cliente_valido(cliente("CL", "sin-rut")), "CL texto");
    assert!(!cliente_valido(cliente("MX", "123456")), "MX sin letras");
}
