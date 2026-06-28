//! Tests de integración del módulo `crm`. Mismo kernel que
//! inventory/sales/treasury, apuntado a `modules/crm`: clientes,
//! oportunidades que recorren un pipeline de ventas, e interacciones.

use std::path::{Path, PathBuf};

use nakui_core::executor::{ExecError, Executor};
use nakui_core::store::{MemoryStore, Store};
use serde_json::{json, Value};
use uuid::Uuid;

fn crm_module() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("dir del módulo nakui sobre core/")
        .join("modules/crm")
}

fn seed_cliente(store: &mut MemoryStore, id: Uuid, nombre: &str) {
    store.seed(
        "Cliente",
        id,
        json!({
            "id": id.to_string(),
            "nombre": nombre,
            "email": "contacto@example.com",
            "empresa": nombre,
        }),
    );
}

/// Abre una oportunidad y devuelve su id. Camino feliz (panica si falla).
fn abrir_opp(exec: &Executor, store: &mut MemoryStore, cliente: Uuid) -> Uuid {
    let opp = Uuid::new_v4();
    exec.run(
        store,
        "abrir_oportunidad",
        &[("cliente", cliente)],
        json!({
            "oportunidad_id": opp.to_string(),
            "titulo": "Licencia anual",
            "monto": 12_000_i64,
            "currency": "USD",
            "timestamp": "2026-05-21T10:00:00Z",
        }),
    )
    .expect("abrir_oportunidad debe pasar");
    opp
}

fn etapa(store: &MemoryStore, opp: Uuid) -> String {
    store
        .load("Oportunidad", opp)
        .and_then(|v| v.get("etapa").and_then(Value::as_str).map(String::from))
        .expect("oportunidad con etapa")
}

/// Corre `mover_oportunidad`; devuelve el conteo de ops en éxito.
// `ExecError` es un enum grande — el resto del crate convive con este
// lint; lo suprimimos local en vez de boxear sólo este helper.
#[allow(clippy::result_large_err)]
fn mover(
    exec: &Executor,
    store: &mut MemoryStore,
    opp: Uuid,
    destino: &str,
) -> Result<usize, ExecError> {
    exec.run(
        store,
        "mover_oportunidad",
        &[("oportunidad", opp)],
        json!({ "etapa": destino, "timestamp": "2026-05-21T11:00:00Z" }),
    )
    .map(|ops| ops.len())
}

#[test]
fn abrir_crea_oportunidad_en_prospecto() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");

    let opp = abrir_opp(&exec, &mut store, cliente);

    assert_eq!(etapa(&store, opp), "prospecto", "nace en prospecto");
    let o = store.load("Oportunidad", opp).expect("oportunidad existe");
    let cid = cliente.to_string();
    assert_eq!(
        o.get("cliente_id").and_then(Value::as_str),
        Some(cid.as_str())
    );
    assert_eq!(o.get("monto").and_then(Value::as_i64), Some(12_000));
}

#[test]
fn pipeline_avanza_hasta_ganada() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let opp = abrir_opp(&exec, &mut store, cliente);

    for destino in ["calificado", "propuesta", "negociacion", "ganada"] {
        mover(&exec, &mut store, opp, destino)
            .unwrap_or_else(|e| panic!("mover a {destino} debe pasar: {e:?}"));
        assert_eq!(etapa(&store, opp), destino);
    }
}

#[test]
fn no_se_retrocede_en_el_pipeline() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let opp = abrir_opp(&exec, &mut store, cliente);

    mover(&exec, &mut store, opp, "propuesta").expect("avanzar debe pasar");

    // prospecto está antes de propuesta → retroceso, rechazado por el script.
    let result = mover(&exec, &mut store, opp, "prospecto");
    match result {
        Err(ExecError::Rhai(_)) => {}
        other => panic!("esperaba Rhai (throw por retroceso), obtuve {other:?}"),
    }
    assert_eq!(etapa(&store, opp), "propuesta", "la etapa no cambió");
}

#[test]
fn oportunidad_cerrada_no_se_mueve() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let opp = abrir_opp(&exec, &mut store, cliente);

    // Cerrar es legal desde cualquier etapa abierta.
    mover(&exec, &mut store, opp, "ganada").expect("cerrar debe pasar");

    // Una oportunidad ganada ya no se mueve.
    let result = mover(&exec, &mut store, opp, "negociacion");
    match result {
        Err(ExecError::Rhai(_)) => {}
        other => panic!("esperaba Rhai (throw por cerrada), obtuve {other:?}"),
    }
    assert_eq!(etapa(&store, opp), "ganada");
}

#[test]
fn etapa_destino_desconocida_es_rechazada() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let opp = abrir_opp(&exec, &mut store, cliente);

    let result = mover(&exec, &mut store, opp, "facturada");
    assert!(matches!(result, Err(ExecError::Rhai(_))));
    assert_eq!(etapa(&store, opp), "prospecto");
}

#[test]
fn monto_negativo_es_rechazado() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");

    let opp = Uuid::new_v4();
    let result = exec.run(
        &mut store,
        "abrir_oportunidad",
        &[("cliente", cliente)],
        json!({
            "oportunidad_id": opp.to_string(),
            "titulo": "Trato inválido",
            "monto": -500_i64,
            "currency": "USD",
            "timestamp": "2026-05-21T10:00:00Z",
        }),
    );
    assert!(matches!(result, Err(ExecError::Rhai(_))));
    assert!(store.load("Oportunidad", opp).is_none(), "no se creó nada");
}

/// Corre `marcar_perdida` con un motivo; devuelve el conteo de ops en éxito.
#[allow(clippy::result_large_err)]
fn perder(
    exec: &Executor,
    store: &mut MemoryStore,
    opp: Uuid,
    motivo: &str,
) -> Result<usize, ExecError> {
    exec.run(
        store,
        "marcar_perdida",
        &[("oportunidad", opp)],
        json!({ "motivo": motivo }),
    )
    .map(|ops| ops.len())
}

fn motivo(store: &MemoryStore, opp: Uuid) -> Option<String> {
    store
        .load("Oportunidad", opp)
        .and_then(|v| v.get("motivo").and_then(Value::as_str).map(String::from))
}

#[test]
fn marcar_perdida_cierra_con_motivo() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let opp = abrir_opp(&exec, &mut store, cliente);

    // Se puede perder desde cualquier etapa abierta (acá, prospecto).
    perder(&exec, &mut store, opp, "precio fuera de presupuesto").expect("perder debe pasar");

    assert_eq!(etapa(&store, opp), "perdida");
    assert_eq!(
        motivo(&store, opp).as_deref(),
        Some("precio fuera de presupuesto")
    );
}

#[test]
fn marcar_perdida_exige_motivo() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let opp = abrir_opp(&exec, &mut store, cliente);

    let result = perder(&exec, &mut store, opp, "");
    assert!(matches!(result, Err(ExecError::Rhai(_))));
    assert_eq!(etapa(&store, opp), "prospecto", "la etapa no cambió");
    assert!(motivo(&store, opp).is_none());
}

#[test]
fn oportunidad_perdida_no_se_mueve() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let opp = abrir_opp(&exec, &mut store, cliente);

    perder(&exec, &mut store, opp, "competencia").expect("perder debe pasar");

    // Una oportunidad perdida no avanza por el embudo…
    let mov = mover(&exec, &mut store, opp, "calificado");
    assert!(matches!(mov, Err(ExecError::Rhai(_))));
    // …ni se vuelve a cerrar.
    let again = perder(&exec, &mut store, opp, "otra vez");
    assert!(matches!(again, Err(ExecError::Rhai(_))));
    assert_eq!(etapa(&store, opp), "perdida");
}

#[test]
fn oportunidad_ganada_no_se_marca_perdida() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let opp = abrir_opp(&exec, &mut store, cliente);

    for destino in ["calificado", "propuesta", "negociacion", "ganada"] {
        mover(&exec, &mut store, opp, destino).expect("avanzar debe pasar");
    }

    let result = perder(&exec, &mut store, opp, "tarde");
    assert!(matches!(result, Err(ExecError::Rhai(_))));
    assert_eq!(etapa(&store, opp), "ganada");
}

/// Avanza una oportunidad recién abierta hasta "ganada".
fn ganar(exec: &Executor, store: &mut MemoryStore, opp: Uuid) {
    for destino in ["calificado", "propuesta", "negociacion", "ganada"] {
        mover(exec, store, opp, destino).expect("avanzar debe pasar");
    }
}

/// Siembra una cuenta del plan contable.
fn seed_cuenta(store: &mut MemoryStore, id: Uuid, codigo: &str, tipo: &str, saldo: i64) {
    store.seed(
        "Cuenta",
        id,
        json!({
            "id": id.to_string(),
            "codigo": codigo,
            "nombre": codigo,
            "tipo": tipo,
            "saldo": saldo,
            "moneda": "USD",
        }),
    );
}

fn saldo(store: &MemoryStore, cta: Uuid) -> i64 {
    store
        .load("Cuenta", cta)
        .and_then(|v| v.get("saldo").and_then(Value::as_i64))
        .expect("cuenta con saldo")
}

#[test]
fn cotizar_solo_desde_ganada() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let opp = abrir_opp(&exec, &mut store, cliente);

    // Abierta (prospecto) → no se cotiza.
    let cot = Uuid::new_v4();
    let r = exec.run(
        &mut store,
        "cotizar_oportunidad",
        &[("oportunidad", opp)],
        json!({ "cotizacion_id": cot.to_string() }),
    );
    assert!(matches!(r, Err(ExecError::Rhai(_))));
    assert!(store.load("Cotizacion", cot).is_none(), "no se creó nada");

    // Ganada → sí.
    ganar(&exec, &mut store, opp);
    exec.run(
        &mut store,
        "cotizar_oportunidad",
        &[("oportunidad", opp)],
        json!({ "cotizacion_id": cot.to_string() }),
    )
    .expect("cotizar una ganada debe pasar");

    let c = store.load("Cotizacion", cot).expect("cotización existe");
    assert_eq!(c.get("estado").and_then(Value::as_str), Some("borrador"));
    assert_eq!(c.get("monto").and_then(Value::as_i64), Some(12_000));
    assert_eq!(
        c.get("oportunidad_id").and_then(Value::as_str),
        Some(opp.to_string().as_str())
    );
}

#[test]
fn ganada_genera_cotizacion_y_factura_asentada() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");

    // Plan contable mínimo para asentar la venta.
    let (clientes, ventas, iva) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    seed_cuenta(&mut store, clientes, "1100-Clientes", "activo", 0);
    seed_cuenta(&mut store, ventas, "4000-Ventas", "ingreso", 0);
    seed_cuenta(&mut store, iva, "2100-IVA", "pasivo", 0);

    // Oportunidad ganada → cotización.
    let opp = abrir_opp(&exec, &mut store, cliente);
    ganar(&exec, &mut store, opp);
    let cot = Uuid::new_v4();
    exec.run(
        &mut store,
        "cotizar_oportunidad",
        &[("oportunidad", opp)],
        json!({ "cotizacion_id": cot.to_string() }),
    )
    .expect("cotizar debe pasar");

    // Cotización → factura asentada (neto 12 000, IVA 16% → impuesto 1 920).
    let fact = Uuid::new_v4();
    exec.run(
        &mut store,
        "facturar_cotizacion",
        &[
            ("cotizacion", cot),
            ("clientes_cta", clientes),
            ("ventas_cta", ventas),
            ("iva_cta", iva),
        ],
        json!({ "factura_id": fact.to_string(), "fecha": "2026-05-21", "tasa": 16 }),
    )
    .expect("facturar debe pasar");

    // La factura existe con los importes calculados y la liga a la cotización.
    let f = store.load("Factura", fact).expect("factura existe");
    assert_eq!(f.get("neto").and_then(Value::as_i64), Some(12_000));
    assert_eq!(f.get("impuesto").and_then(Value::as_i64), Some(1_920));
    assert_eq!(f.get("total").and_then(Value::as_i64), Some(13_920));
    assert_eq!(
        f.get("cotizacion_id").and_then(Value::as_str),
        Some(cot.to_string().as_str())
    );

    // El asiento cuadra (deudor-normal): Σ Δ saldo = 0.
    assert_eq!(saldo(&store, clientes), 13_920, "CxC += total");
    assert_eq!(saldo(&store, ventas), -12_000, "Ventas -= neto");
    assert_eq!(saldo(&store, iva), -1_920, "IVA -= impuesto");
    assert_eq!(
        saldo(&store, clientes) + saldo(&store, ventas) + saldo(&store, iva),
        0,
        "partida doble"
    );

    // La cotización quedó facturada.
    let c = store.load("Cotizacion", cot).expect("cotización existe");
    assert_eq!(c.get("estado").and_then(Value::as_str), Some("facturada"));
}

#[test]
fn cotizacion_no_se_factura_dos_veces() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");
    let (clientes, ventas, iva) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    seed_cuenta(&mut store, clientes, "1100-Clientes", "activo", 0);
    seed_cuenta(&mut store, ventas, "4000-Ventas", "ingreso", 0);
    seed_cuenta(&mut store, iva, "2100-IVA", "pasivo", 0);

    let opp = abrir_opp(&exec, &mut store, cliente);
    ganar(&exec, &mut store, opp);
    let cot = Uuid::new_v4();
    exec.run(
        &mut store,
        "cotizar_oportunidad",
        &[("oportunidad", opp)],
        json!({ "cotizacion_id": cot.to_string() }),
    )
    .expect("cotizar debe pasar");

    let facturar = |store: &mut MemoryStore, fact: Uuid| {
        exec.run(
            store,
            "facturar_cotizacion",
            &[
                ("cotizacion", cot),
                ("clientes_cta", clientes),
                ("ventas_cta", ventas),
                ("iva_cta", iva),
            ],
            json!({ "factura_id": fact.to_string(), "fecha": "2026-05-21", "tasa": 16 }),
        )
    };

    facturar(&mut store, Uuid::new_v4()).expect("primera factura debe pasar");
    // Segunda vez: la cotización ya no está en borrador → rechazada, sin tocar saldos.
    let r = facturar(&mut store, Uuid::new_v4());
    assert!(matches!(r, Err(ExecError::Rhai(_))));
    assert_eq!(saldo(&store, clientes), 13_920, "saldo intacto tras rechazo");
}

#[test]
fn registrar_interaccion_crea_registro() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");

    let int_id = Uuid::new_v4();
    exec.run(
        &mut store,
        "registrar_interaccion",
        &[("cliente", cliente)],
        json!({
            "interaccion_id": int_id.to_string(),
            "canal": "llamada",
            "nota": "Primer contacto, interés alto",
            "timestamp": "2026-05-21T09:00:00Z",
        }),
    )
    .expect("registrar_interaccion debe pasar");

    let i = store
        .load("Interaccion", int_id)
        .expect("interacción existe");
    assert_eq!(i.get("canal").and_then(Value::as_str), Some("llamada"));
    let cid = cliente.to_string();
    assert_eq!(
        i.get("cliente_id").and_then(Value::as_str),
        Some(cid.as_str())
    );
}

#[test]
fn canal_invalido_es_rechazado() {
    let exec = Executor::load_module(crm_module()).expect("load module");
    let mut store = MemoryStore::new();
    let cliente = Uuid::new_v4();
    seed_cliente(&mut store, cliente, "Acme Corp");

    let int_id = Uuid::new_v4();
    let result = exec.run(
        &mut store,
        "registrar_interaccion",
        &[("cliente", cliente)],
        json!({
            "interaccion_id": int_id.to_string(),
            "canal": "paloma-mensajera",
            "nota": "canal inexistente",
            "timestamp": "2026-05-21T09:00:00Z",
        }),
    );
    assert!(matches!(result, Err(ExecError::Rhai(_))));
    assert!(store.load("Interaccion", int_id).is_none());
}
