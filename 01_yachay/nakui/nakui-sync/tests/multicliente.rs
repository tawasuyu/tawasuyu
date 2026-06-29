//! Prueba del modelo multi-cliente: un escritor autoritativo, varios
//! clientes contra él, deltas que convergen.
//!
//! Estos tests son la evidencia de que la arquitectura de la fase 1
//! soporta múltiples usuarios concurrentes: el escritor único impone orden
//! total, y cada cliente pone su proyección al día aplicando los commits
//! difundidos — sin CRDTs, sin consenso, sin re-leer el log de disco.

use std::collections::BTreeMap;

use nakui_core::store::{MemoryStore, Store};
use nakui_sync::{apply_commit, Intent, LocalTransport, Transport, Writer};
use serde_json::{json, Map, Value};
use uuid::Uuid;

fn map_of(items: &[(&str, Value)]) -> Map<String, Value> {
    items.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// Monta un escritor con log en tempdir y devuelve el transporte + el dir
/// (que hay que mantener vivo para que el log no se borre).
fn local_transport() -> (LocalTransport, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (writer, _status) = Writer::open(dir.path().join("log.jsonl"), 0, BTreeMap::new());
    (LocalTransport::new(writer), dir)
}

#[test]
fn dos_clientes_convergen_via_delta_broadcast() {
    let (transport, _dir) = local_transport();

    // Cliente A: el que escribe (clon del transporte = otro "asiento").
    let cliente_a = transport.clone();
    // Cliente B: remoto — su propia proyección, puesta al día por broadcast.
    let mut proyeccion_b = MemoryStore::new();
    let rx_b = transport.subscribe();

    // A da de alta un cliente del ERP.
    let commit = cliente_a
        .submit(Intent::Seed {
            entity: "Cliente".into(),
            data: map_of(&[("nombre", json!("Acme"))]),
        })
        .unwrap();
    let id = commit.primary_id.unwrap();

    // B aún no sabe nada.
    assert!(proyeccion_b.load("Cliente", id).is_none());

    // B drena su suscripción y aplica el delta.
    for c in rx_b.try_iter() {
        apply_commit(&mut proyeccion_b, &c).unwrap();
    }

    // Ahora B ve el record que escribió A.
    let rec = proyeccion_b.load("Cliente", id).expect("B ve el alta de A");
    assert_eq!(rec.get("nombre"), Some(&json!("Acme")));

    // A edita; B vuelve a converger.
    cliente_a
        .submit(Intent::Update {
            entity: "Cliente".into(),
            id,
            set: map_of(&[("nombre", json!("Acme S.A."))]),
            clear: vec![],
        })
        .unwrap();
    for c in rx_b.try_iter() {
        apply_commit(&mut proyeccion_b, &c).unwrap();
    }
    assert_eq!(
        proyeccion_b.load("Cliente", id).unwrap().get("nombre"),
        Some(&json!("Acme S.A.")),
    );
}

#[test]
fn escrituras_concurrentes_quedan_en_orden_total_y_convergen() {
    let (transport, _dir) = local_transport();

    // Dos clientes que escriben, cada uno con su proyección remota.
    let a = transport.clone();
    let b = transport.clone();
    let mut proy_a = MemoryStore::new();
    let mut proy_b = MemoryStore::new();
    let rx_a = transport.subscribe();
    let rx_b = transport.subscribe();

    // Interleaved: A, B, A, B... El Mutex del escritor los serializa, así
    // que el log queda con seqs contiguos pase lo que pase.
    let mut ids = Vec::new();
    for i in 0..6 {
        let quien = if i % 2 == 0 { &a } else { &b };
        let commit = quien
            .submit(Intent::Seed {
                entity: "Mov".into(),
                data: map_of(&[("n", json!(i))]),
            })
            .unwrap();
        ids.push(commit.primary_id.unwrap());
        // El seq anexado debe ser exactamente i (contiguo desde 0).
        assert_eq!(commit.last_seq(), Some(i as u64), "orden total: seq contiguo");
    }

    // Ambas proyecciones aplican TODOS los commits difundidos.
    for c in rx_a.try_iter() {
        apply_commit(&mut proy_a, &c).unwrap();
    }
    for c in rx_b.try_iter() {
        apply_commit(&mut proy_b, &c).unwrap();
    }

    // Las dos proyecciones remotas y el store autoritativo del escritor
    // convergen al MISMO estado (mismo hash).
    let autoritativo = {
        let w = transport.writer();
        let store = w.lock().unwrap().store_handle();
        let h = store.lock().unwrap().hash_state().unwrap();
        h
    };
    assert_eq!(proy_a.hash_state().unwrap(), autoritativo, "A converge al autoritativo");
    assert_eq!(proy_b.hash_state().unwrap(), autoritativo, "B converge al autoritativo");
    assert_eq!(proy_a.list_n("Mov"), 6);
}

#[test]
fn apply_commit_es_idempotente_por_seq() {
    let (transport, _dir) = local_transport();
    let mut proy = MemoryStore::new();

    let commit = transport
        .submit(Intent::Seed {
            entity: "Cliente".into(),
            data: map_of(&[("nombre", json!("Uno"))]),
        })
        .unwrap();

    // Aplicar el mismo commit dos veces no debe romper ni duplicar: la
    // segunda pasada se saltea por seq (re-entrega = inofensiva).
    apply_commit(&mut proy, &commit).unwrap();
    let marca = proy.last_applied_seq().unwrap();
    apply_commit(&mut proy, &commit).unwrap();
    assert_eq!(
        proy.last_applied_seq().unwrap(),
        marca,
        "re-aplicar el mismo commit no avanza el marcador"
    );
    assert_eq!(proy.list_n("Cliente"), 1, "sin duplicado");
}

#[test]
fn no_op_update_no_difunde_delta() {
    let (transport, _dir) = local_transport();
    let rx = transport.subscribe();

    let id = transport
        .submit(Intent::Seed {
            entity: "X".into(),
            data: map_of(&[("a", json!(1))]),
        })
        .unwrap()
        .primary_id
        .unwrap();

    // Update vacío = no-op: no anexa entradas, no difunde.
    let commit = transport
        .submit(Intent::Update {
            entity: "X".into(),
            id,
            set: Map::new(),
            clear: vec![],
        })
        .unwrap();
    assert_eq!(commit.changed, 0);
    assert!(commit.entries.is_empty());

    // El receiver sólo vio el seed (1 commit), no el no-op.
    let recibidos: Vec<_> = rx.try_iter().collect();
    assert_eq!(recibidos.len(), 1, "sólo el seed se difundió");
}

// Helper de conveniencia para los tests: cuenta records de una entity.
trait CountExt {
    fn list_n(&self, entity: &str) -> usize;
}
impl CountExt for MemoryStore {
    fn list_n(&self, entity: &str) -> usize {
        self.iter()
            .map(|it| it.filter(|(e, _, _)| e == entity).count())
            .unwrap_or(0)
    }
}

// Silencia warning de import no usado si algún día se recorta.
#[allow(dead_code)]
fn _uses_uuid(_: Uuid) {}
