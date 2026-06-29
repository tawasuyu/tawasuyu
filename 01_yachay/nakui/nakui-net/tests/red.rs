//! Prueba del transporte de red: un servidor autoritativo + clientes
//! remotos sobre card-net (libp2p, loopback TCP), convergiendo por delta.
//!
//! Es el mismo contrato que los tests multi-cliente de `nakui-sync`, pero
//! ahora el `Transport` cruza la red real (Noise+yamux sobre 127.0.0.1).
//! Evidencia de la fase 2: un cliente remoto escribe, el otro lo ve.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use nakui_core::store::{MemoryStore, Store};
use nakui_net::{serve, CardNetTransport};
use nakui_sync::{apply_commit, Intent, Transport, Writer};
use serde_json::{json, Map, Value};
use uuid::Uuid;

fn map_of(items: &[(&str, Value)]) -> Map<String, Value> {
    items.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// Levanta un servidor con escritor autoritativo (log en tempdir) y
/// devuelve su dirección dialable + el dir (mantener vivo). El `Writer` se
/// construye dentro del thread del actor vía la clausura.
fn servidor() -> (nakui_net::ServerHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("log.jsonl");
    let handle = serve(
        move || Writer::open(log_path, 0, BTreeMap::new()).0,
        "/ip4/127.0.0.1/tcp/0",
    )
    .expect("servidor arranca");
    (handle, dir)
}

/// Drena el receiver de un cliente aplicando los commits a su proyección,
/// hasta que la entity tenga `objetivo` records o se agote el tiempo.
fn esperar_records(
    rx: &std::sync::mpsc::Receiver<nakui_sync::Commit>,
    proy: &mut MemoryStore,
    entity: &str,
    objetivo: usize,
    segundos: u64,
) -> bool {
    let limite = Instant::now() + Duration::from_secs(segundos);
    loop {
        for commit in rx.try_iter() {
            apply_commit(proy, &commit).unwrap();
        }
        let n = proy
            .iter()
            .map(|it| it.filter(|(e, _, _)| e == entity).count())
            .unwrap_or(0);
        if n >= objetivo {
            return true;
        }
        if Instant::now() > limite {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn cliente_remoto_escribe_y_otro_cliente_lo_ve() {
    let (handle, _dir) = servidor();

    // Dos clientes remotos, cada uno con su propia conexión y proyección.
    let escritor = CardNetTransport::connect(handle.dial_addr()).expect("cliente A conecta");
    let lector = CardNetTransport::connect(handle.dial_addr()).expect("cliente B conecta");
    let rx_lector = lector.subscribe();
    let mut proy_lector = MemoryStore::new();

    // A da de alta un cliente del ERP. submit bloquea hasta el commit
    // autoritativo (ya durable en el log del servidor).
    let commit = escritor
        .submit(Intent::Seed {
            entity: "Cliente".into(),
            data: map_of(&[("nombre", json!("Acme"))]),
        })
        .expect("seed remoto");
    let id = commit.primary_id.unwrap();
    assert_eq!(commit.changed, 1);

    // B converge por difusión.
    assert!(
        esperar_records(&rx_lector, &mut proy_lector, "Cliente", 1, 15),
        "B debe ver el alta que hizo A por la red"
    );
    let rec = proy_lector.load("Cliente", id).expect("B tiene el record");
    assert_eq!(rec.get("nombre"), Some(&json!("Acme")));
}

#[test]
fn escrituras_remotas_concurrentes_convergen_y_quedan_ordenadas() {
    let (handle, _dir) = servidor();

    let a = CardNetTransport::connect(handle.dial_addr()).expect("A conecta");
    let b = CardNetTransport::connect(handle.dial_addr()).expect("B conecta");
    let rx_a = a.subscribe();
    let rx_b = b.subscribe();

    // A y B escriben interleaved. El escritor único del servidor los
    // serializa: los seqs salen contiguos pase lo que pase.
    let mut seqs = Vec::new();
    for i in 0..6 {
        let quien = if i % 2 == 0 { &a } else { &b };
        let commit = quien
            .submit(Intent::Seed {
                entity: "Mov".into(),
                data: map_of(&[("n", json!(i))]),
            })
            .expect("seed remoto");
        seqs.push(commit.last_seq().expect("commit con entrada"));
    }
    seqs.sort_unstable();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4, 5], "orden total: seqs contiguos sin huecos");

    // Ambas proyecciones remotas convergen a los 6 movimientos.
    let mut proy_a = MemoryStore::new();
    let mut proy_b = MemoryStore::new();
    assert!(esperar_records(&rx_a, &mut proy_a, "Mov", 6, 15), "A converge a 6");
    assert!(esperar_records(&rx_b, &mut proy_b, "Mov", 6, 15), "B converge a 6");
    assert_eq!(
        proy_a.hash_state().unwrap(),
        proy_b.hash_state().unwrap(),
        "las dos proyecciones remotas convergen al mismo estado"
    );
}

#[test]
fn intencion_rechazada_devuelve_error_sin_romper_el_canal() {
    let (handle, _dir) = servidor();
    let cliente = CardNetTransport::connect(handle.dial_addr()).expect("conecta");

    // Update sobre un id inexistente: el escritor lo rechaza (NotFound). El
    // error vuelve por el canal de respuesta, no rompe la conexión.
    let err = cliente
        .submit(Intent::Update {
            entity: "Cliente".into(),
            id: Uuid::new_v4(),
            set: map_of(&[("nombre", json!("X"))]),
            clear: vec![],
        })
        .unwrap_err();
    assert!(!err.is_empty(), "debe haber mensaje de error: {err}");

    // El canal sigue vivo: un seed posterior funciona.
    let ok = cliente
        .submit(Intent::Seed {
            entity: "Cliente".into(),
            data: map_of(&[("nombre", json!("Acme"))]),
        })
        .expect("el canal sigue usable tras el rechazo");
    assert_eq!(ok.changed, 1);
}
