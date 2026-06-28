//! Demo del módulo `accounting`: contabilidad de PARTIDA DOBLE sobre el
//! motor de conservación de nakui. `Cuenta.saldo` es deudor-normal
//! (centavos): el débito suma, el crédito resta. La invariante `conserve`
//! del kernel (Σ Δ Cuenta.saldo = 0 por moneda) exige que cada asiento
//! cuadre — la partida doble la garantiza el executor, no el script.
//!
//! Corre la balanza de comprobación al final: Σ de todos los saldos debe
//! dar exactamente 0, la identidad contable fundamental.

use nakui_core::event_log::{
    execute_and_log, replay, seed_and_log, verify_log, EventLog, ExecuteError,
};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use serde_json::json;
use uuid::Uuid;

fn main() {
    let module_dir = std::env::var("NAKUI_MODULE").unwrap_or_else(|_| "modules/accounting".into());
    let exec = Executor::load_module(&module_dir).expect("load module");

    let log_path = std::env::temp_dir().join(format!("nakui_acc_{}.jsonl", Uuid::new_v4()));
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut store = MemoryStore::new();

    // Plan de cuentas mínimo (saldos en centavos).
    let caja = seed_cuenta(&exec, &mut store, &mut log, "1010", "Caja", "activo");
    let banco = seed_cuenta(&exec, &mut store, &mut log, "1020", "Banco", "activo");
    let ventas = seed_cuenta(&exec, &mut store, &mut log, "4010", "Ventas", "ingreso");
    let proveedores = seed_cuenta(&exec, &mut store, &mut log, "2010", "Proveedores", "pasivo");
    let gastos = seed_cuenta(&exec, &mut store, &mut log, "5010", "Gastos", "gasto");

    section("== plan de cuentas sembrado ==");
    for (label, id) in [
        ("Caja", caja),
        ("Banco", banco),
        ("Ventas", ventas),
        ("Proveedores", proveedores),
        ("Gastos", gastos),
    ] {
        print_cuenta(&store, label, id);
    }

    section("== asentar: cobro al contado $5.000,00 (debe Caja / haber Ventas) ==");
    asentar(&exec, &mut store, &mut log, caja, ventas, 500_000, "venta al contado");
    print_cuenta(&store, "Caja", caja);
    print_cuenta(&store, "Ventas", ventas);

    section("== asentar: compra a crédito $1.200,00 (debe Gastos / haber Proveedores) ==");
    asentar(&exec, &mut store, &mut log, gastos, proveedores, 120_000, "compra insumos");

    section("== asentar: pago a proveedor $1.200,00 (debe Proveedores / haber Banco) ==");
    asentar(&exec, &mut store, &mut log, proveedores, banco, 120_000, "pago proveedor");

    section("== reject: monto negativo (post-check NoNegativo de Asiento.monto) ==");
    asentar(&exec, &mut store, &mut log, caja, ventas, -100, "monto inválido");

    section("== reject: mismo asiento contra una sola cuenta (DuplicateInputId) ==");
    asentar(&exec, &mut store, &mut log, caja, caja, 100, "auto-asiento");

    section("== balanza de comprobación ==");
    let total: i64 = [caja, banco, ventas, proveedores, gastos]
        .iter()
        .map(|&id| {
            let s = saldo(&store, id);
            print_cuenta(&store, "  ", id);
            s
        })
        .sum();
    println!("  --------");
    println!(
        "  Σ saldos = {} centavos  → {}",
        total,
        if total == 0 { "CUADRA ✓" } else { "DESCUADRE ✗" }
    );

    section("== replay verification (state) ==");
    let replayed = replay(&log).expect("replay");
    println!(
        "  {}",
        if store == replayed {
            "ok: replay byte-equal al store vivo"
        } else {
            "MISMATCH"
        }
    );

    section("== determinism verification (ops) ==");
    match verify_log(&log, &exec) {
        Ok(()) => println!("  ok: cada asiento reprodujo sus ops al re-ejecutar"),
        Err(e) => println!("  nondeterminism detectado: {}", e),
    }

    let _ = std::fs::remove_file(&log_path);
}

fn seed_cuenta(
    exec: &Executor,
    store: &mut MemoryStore,
    log: &mut EventLog,
    codigo: &str,
    nombre: &str,
    tipo: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    seed_and_log(
        exec,
        store,
        log,
        "Cuenta",
        id,
        json!({
            "id": id.to_string(),
            "codigo": codigo,
            "nombre": nombre,
            "tipo": tipo,
            "saldo": 0_i64,
            "moneda": "USD",
        }),
    )
    .expect("seed cuenta");
    id
}

#[allow(clippy::too_many_arguments)]
fn asentar(
    exec: &Executor,
    store: &mut MemoryStore,
    log: &mut EventLog,
    debito: Uuid,
    credito: Uuid,
    monto: i64,
    glosa: &str,
) {
    let res = execute_and_log(
        exec,
        store,
        log,
        "asentar",
        &[("debito", debito), ("credito", credito)],
        json!({
            "monto": monto,
            "glosa": glosa,
            "fecha": "2026-06-28",
            "diario": "general",
            "asiento_id": Uuid::new_v4().to_string(),
        }),
    );
    match res {
        Ok(ops) => println!("  ok ({} ops, logged #{})", ops.len(), log.next_seq() - 1),
        Err(ExecuteError::PreLog(e)) => println!("  rejected: {}", e),
        Err(ExecuteError::LogAppend(e)) => println!("  LOG APPEND FAILED: {}", e),
        Err(ExecuteError::PostLogStore(e)) => println!("  POST-LOG STORE FAILED: {}", e),
    }
}

fn saldo(store: &MemoryStore, id: Uuid) -> i64 {
    store
        .load("Cuenta", id)
        .and_then(|v| v.get("saldo").and_then(|s| s.as_i64()))
        .unwrap_or(0)
}

fn print_cuenta(store: &MemoryStore, label: &str, id: Uuid) {
    let v = store.load("Cuenta", id).expect("cuenta existe");
    let codigo = v.get("codigo").and_then(|s| s.as_str()).unwrap_or("?");
    let nombre = v.get("nombre").and_then(|s| s.as_str()).unwrap_or("?");
    let saldo = v.get("saldo").and_then(|s| s.as_i64()).unwrap_or(0);
    println!("  {} [{}] {:<12} saldo={} (centavos)", label, codigo, nombre, saldo);
}

fn section(title: &str) {
    println!("\n{}", title);
}
