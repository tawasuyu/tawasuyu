//! Demo del módulo `crm`: un escenario realista — tres clientes, sus
//! oportunidades recorriendo el pipeline de ventas, e interacciones.
//!
//! A diferencia de los otros demos, **no borra el event log**: lo deja
//! en disco para que `nakui-explorer` lo muestre. Al terminar imprime
//! el comando exacto para abrir el explorador sobre este log.
//!
//! ```sh
//! cargo run -p nakui-core --bin crm_demo
//! # …luego, con la ruta que imprime:
//! NAKUI_EVENT_LOG=/tmp/nakui-crm.jsonl cargo run -p nakui-explorer
//! ```

use std::path::{Path, PathBuf};

use nakui_core::event_log::{execute_and_log, seed_and_log, EventLog, ExecuteError, LogEntry};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use serde_json::json;
use uuid::Uuid;

const TS: &str = "2026-05-21T12:00:00Z";

fn main() {
    let module_dir = std::env::var("NAKUI_MODULE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("dir del módulo nakui sobre core/")
                .join("modules/crm")
        });
    let exec = Executor::load_module(&module_dir).expect("cargar el módulo crm");

    let log_path = std::env::var("NAKUI_EVENT_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("nakui-crm.jsonl"));
    let _ = std::fs::remove_file(&log_path); // empezar de cero
    let mut log = EventLog::open(&log_path).expect("abrir el event log");
    let mut store = MemoryStore::new();

    // --- Seed: tres clientes -------------------------------------------
    section("seed · 3 clientes");
    let acme = Uuid::new_v4();
    let beta = Uuid::new_v4();
    let gamma = Uuid::new_v4();
    seed_cliente(
        &exec,
        &mut store,
        &mut log,
        acme,
        "Acme Corp",
        "compras@acme.com",
    );
    seed_cliente(&exec, &mut store, &mut log, beta, "Beta SA", "ti@beta.com");
    seed_cliente(
        &exec,
        &mut store,
        &mut log,
        gamma,
        "Gamma Ltda",
        "ceo@gamma.com",
    );

    // --- Acme: una oportunidad que se gana -----------------------------
    section("Acme · «Licencia anual» $12 000 — recorre el pipeline");
    let opp_acme = Uuid::new_v4();
    abrir(
        &exec,
        &mut store,
        &mut log,
        acme,
        opp_acme,
        "Licencia anual",
        12_000,
    );
    interaccion(
        &exec,
        &mut store,
        &mut log,
        acme,
        "llamada",
        "Primer contacto, interés alto",
    );
    for etapa in ["calificado", "propuesta", "negociacion", "ganada"] {
        mover(&exec, &mut store, &mut log, opp_acme, etapa);
    }
    interaccion(
        &exec,
        &mut store,
        &mut log,
        acme,
        "email",
        "Contrato firmado recibido",
    );

    // --- Beta: una oportunidad que se pierde ---------------------------
    section("Beta · «Piloto trimestral» $3 000 — se pierde");
    let opp_beta = Uuid::new_v4();
    abrir(
        &exec,
        &mut store,
        &mut log,
        beta,
        opp_beta,
        "Piloto trimestral",
        3_000,
    );
    interaccion(
        &exec,
        &mut store,
        &mut log,
        beta,
        "reunion",
        "Demo en sus oficinas",
    );
    mover(&exec, &mut store, &mut log, opp_beta, "calificado");
    mover(&exec, &mut store, &mut log, opp_beta, "propuesta");
    perder(
        &exec,
        &mut store,
        &mut log,
        opp_beta,
        "precio fuera de presupuesto",
    );

    // --- Gamma: una oportunidad en curso -------------------------------
    section("Gamma · «Expansión regional» $25 000 — en curso");
    let opp_gamma = Uuid::new_v4();
    abrir(
        &exec,
        &mut store,
        &mut log,
        gamma,
        opp_gamma,
        "Expansión regional",
        25_000,
    );
    mover(&exec, &mut store, &mut log, opp_gamma, "calificado");
    interaccion(
        &exec,
        &mut store,
        &mut log,
        gamma,
        "llamada",
        "Pidieron referencias",
    );

    // --- Operaciones inválidas: el kernel las rechaza, no se loguean ---
    section("validaciones · estas operaciones se rechazan");
    mover(&exec, &mut store, &mut log, opp_acme, "propuesta"); // ya cerrada
    mover(&exec, &mut store, &mut log, opp_gamma, "prospecto"); // retroceso
    abrir(
        &exec,
        &mut store,
        &mut log,
        gamma,
        Uuid::new_v4(),
        "Trato inválido",
        -500,
    );
    interaccion(
        &exec,
        &mut store,
        &mut log,
        gamma,
        "paloma",
        "canal inexistente",
    );

    // --- Estado final --------------------------------------------------
    section("estado final · oportunidades");
    print_oportunidad(&store, "Acme ", opp_acme);
    print_oportunidad(&store, "Beta ", opp_beta);
    print_oportunidad(&store, "Gamma", opp_gamma);

    let entries = log.entries().expect("leer el log");
    let seeds = entries
        .iter()
        .filter(|e| matches!(e, LogEntry::Seed { .. }))
        .count();
    let morphs = entries.len() - seeds;
    section(&format!(
        "log · {} eventos ({seeds} seeds, {morphs} morfismos)",
        entries.len()
    ));
    println!("  archivo: {}", log_path.display());
    println!();
    println!("para ver el módulo CRM en el explorador:");
    println!(
        "  NAKUI_EVENT_LOG={} cargo run -p nakui-explorer",
        log_path.display()
    );
}

fn seed_cliente(
    exec: &Executor,
    store: &mut MemoryStore,
    log: &mut EventLog,
    id: Uuid,
    nombre: &str,
    email: &str,
) {
    seed_and_log(
        exec,
        store,
        log,
        "Cliente",
        id,
        json!({
            "id": id.to_string(),
            "nombre": nombre,
            "email": email,
            "empresa": nombre,
        }),
    )
    .unwrap_or_else(|e| panic!("seed cliente {nombre}: {e}"));
    println!("  ok · cliente {nombre}");
}

fn abrir(
    exec: &Executor,
    store: &mut MemoryStore,
    log: &mut EventLog,
    cliente: Uuid,
    opp: Uuid,
    titulo: &str,
    monto: i64,
) {
    report(
        &format!("abrir_oportunidad «{titulo}»"),
        execute_and_log(
            exec,
            store,
            log,
            "abrir_oportunidad",
            &[("cliente", cliente)],
            json!({
                "oportunidad_id": opp.to_string(),
                "titulo": titulo,
                "monto": monto,
                "currency": "USD",
                "timestamp": TS,
            }),
        ),
    );
}

fn mover(exec: &Executor, store: &mut MemoryStore, log: &mut EventLog, opp: Uuid, destino: &str) {
    report(
        &format!("mover_oportunidad → {destino}"),
        execute_and_log(
            exec,
            store,
            log,
            "mover_oportunidad",
            &[("oportunidad", opp)],
            json!({ "etapa": destino, "timestamp": TS }),
        ),
    );
}

fn perder(exec: &Executor, store: &mut MemoryStore, log: &mut EventLog, opp: Uuid, motivo: &str) {
    report(
        &format!("marcar_perdida ({motivo})"),
        execute_and_log(
            exec,
            store,
            log,
            "marcar_perdida",
            &[("oportunidad", opp)],
            json!({ "motivo": motivo, "timestamp": TS }),
        ),
    );
}

fn interaccion(
    exec: &Executor,
    store: &mut MemoryStore,
    log: &mut EventLog,
    cliente: Uuid,
    canal: &str,
    nota: &str,
) {
    report(
        &format!("registrar_interaccion ({canal})"),
        execute_and_log(
            exec,
            store,
            log,
            "registrar_interaccion",
            &[("cliente", cliente)],
            json!({
                "interaccion_id": Uuid::new_v4().to_string(),
                "canal": canal,
                "nota": nota,
                "timestamp": TS,
            }),
        ),
    );
}

/// Reporta el resultado de un morfismo. Genérico sobre el tipo de op
/// para no exponer el tipo interno del executor.
fn report<T>(label: &str, result: Result<Vec<T>, ExecuteError>) {
    match result {
        Ok(ops) => println!("  ok · {label} ({} ops)", ops.len()),
        Err(ExecuteError::PreLog(e)) => println!("  rechazado · {label}: {e}"),
        Err(e) => println!("  ERROR · {label}: {e:?}"),
    }
}

fn print_oportunidad(store: &MemoryStore, etiqueta: &str, id: Uuid) {
    match store.load("Oportunidad", id) {
        Some(v) => {
            let titulo = v.get("titulo").and_then(|x| x.as_str()).unwrap_or("?");
            let etapa = v.get("etapa").and_then(|x| x.as_str()).unwrap_or("?");
            let monto = v.get("monto").and_then(|x| x.as_i64()).unwrap_or(0);
            let motivo = v
                .get("motivo")
                .and_then(|x| x.as_str())
                .map(|m| format!(" ({m})"))
                .unwrap_or_default();
            println!("  {etiqueta} · {titulo} — ${monto} — etapa: {etapa}{motivo}");
        }
        None => println!("  {etiqueta} · (sin oportunidad)"),
    }
}

fn section(title: &str) {
    println!("\n— {title}");
}
