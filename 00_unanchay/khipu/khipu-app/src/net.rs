//! `net` — el lado P2P del cuaderno: sellar/exportar el sobre firmado,
//! verificar e ingerir uno ajeno, descubrir pares y publicarse por libp2p.
//! El transporte (LAN directo + WAN vía `card-net`/Kademlia + relay/NAT)
//! vive en `khipu-share`/`khipu-brahman`; acá sólo se orquesta.

use std::sync::Arc;

use khipu_share::{SharedNote, SignedBundle};
use llimphi_ui::Handle;

use crate::panels::note_matches;
use crate::{khipu_dir, now_secs, schedule_embedding, Model, Msg, P2p};

pub(crate) fn export_notebook(model: &Model) -> String {
    let Some(kp) = model.keypair.as_ref() else {
        return "sin identidad para firmar".into();
    };
    let Some(dir) = khipu_dir() else {
        return "sin directorio de datos".into();
    };
    // Compartir selectivo: si hay texto en el buscador, exportamos sólo
    // las notas que filtra (mismo criterio que la lista); si está vacío,
    // todo el cuaderno.
    let query = model.search.text();
    let q = query.trim();
    let notes: Vec<SharedNote> = model
        .order
        .iter()
        .filter_map(|id| model.store.get(*id))
        .filter(|n| q.is_empty() || note_matches(n, q))
        .map(SharedNote::from_note)
        .collect();
    if notes.is_empty() {
        return "no hay notas para exportar (¿el filtro no coincide?)".into();
    }
    let n = notes.len();
    let sobre = match khipu_share::seal(kp, notes, now_secs()) {
        Ok(s) => s,
        Err(_) => return "falló el sellado".into(),
    };
    let Ok(bytes) = sobre.to_bytes() else {
        return "falló serializar el sobre".into();
    };
    let path = dir.join("compartido.khipu");
    let tmp = path.with_extension("khipu.tmp");
    if std::fs::write(&tmp, &bytes)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .is_err()
    {
        return "no se pudo escribir el sobre".into();
    }
    let hash = sobre.content_address().unwrap_or([0u8; 32]);
    let filtro = if q.is_empty() {
        String::new()
    } else {
        format!(" (filtro «{q}»)")
    };
    format!(
        "exportadas {n} notas{filtro} → compartido.khipu · {}",
        hex8(&hash)
    )
}

/// Verifica e ingiere `compartido.khipu`. Las notas nuevas nacen con
/// gravedad fresca; sus embeddings se recalculan en segundo plano. Un
/// sobre con firma inválida se rechaza entero. Devuelve la línea de estado.
pub(crate) fn import_notebook(model: &mut Model, h: &Handle<Msg>) -> String {
    let Some(dir) = khipu_dir() else {
        return "sin directorio de datos".into();
    };
    let path = dir.join("compartido.khipu");
    let Ok(bytes) = std::fs::read(&path) else {
        return "no hay compartido.khipu para importar".into();
    };
    let sobre = match SignedBundle::from_bytes(&bytes) {
        Ok(s) => s,
        Err(_) => return "sobre ilegible".into(),
    };
    let outcome = match khipu_share::open(&sobre) {
        Ok(bundle) => khipu_share::import_into(&mut model.store, bundle, now_secs()),
        Err(_) => return "firma inválida — sobre rechazado".into(),
    };
    for id in &outcome.created {
        model.order.push(*id);
        schedule_embedding(model, *id, h);
    }
    format!(
        "importadas {} · omitidas {} (ya existían)",
        outcome.created.len(),
        outcome.skipped
    )
}

/// Dirección donde el servidor escucha. `KHIPU_BIND` la sobrescribe;
/// default localhost para no exponerse sin querer.
pub(crate) fn bind_addr() -> String {
    std::env::var("KHIPU_BIND").unwrap_or_else(|_| "127.0.0.1:7700".into())
}

/// Dirección del par a quien jalarle el cuaderno. `KHIPU_PEER` la
/// sobrescribe; default coincide con [`bind_addr`] para probar en local.
pub(crate) fn peer_addr() -> String {
    std::env::var("KHIPU_PEER").unwrap_or_else(|_| "127.0.0.1:7700".into())
}

/// Arma el nodo libp2p la primera vez que se necesita: runtime tokio
/// dedicado + `KhipuNode` que empieza a escuchar (para ser alcanzable y
/// obtener nuestra dirección de marcado). Idempotente. `false` si no se
/// pudo (sin runtime o sin red).
pub(crate) fn ensure_p2p(model: &mut Model) -> bool {
    if model.p2p.is_some() {
        return true;
    }
    let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    else {
        return false;
    };
    // `KhipuNode::standalone` arranca el swarm con `tokio::spawn`: hay que
    // estar dentro del runtime.
    let node = {
        let _g = rt.enter();
        match khipu_brahman::KhipuNode::standalone() {
            Ok(n) => Arc::new(n),
            Err(_) => return false,
        }
    };
    let dial_addr = rt
        .block_on(node.listen_str("/ip4/0.0.0.0/tcp/0"))
        .unwrap_or_default();
    // Si hay un nodo bootstrap configurado, nos unimos a la malla DHT para
    // poder descubrir y ser descubiertos (`anunciar`/`descubrir`).
    if let Ok(boot) = std::env::var("KHIPU_BOOTSTRAP") {
        let _ = node.dial_str(&boot);
    }
    model.p2p = Some(P2p {
        rt: Arc::new(rt),
        node,
        dial_addr,
        serving: false,
    });
    true
}

/// Levanta (una sola vez) el servidor TCP que sirve `compartido.khipu`.
/// El hilo lee el archivo en cada conexión, así sirve siempre la versión
/// vigente; vive hasta que el proceso termina. Devuelve la línea de estado.
pub(crate) fn start_publishing(model: &mut Model, h: &Handle<Msg>) -> String {
    if model.publishing {
        return format!("ya publicando en {}", bind_addr());
    }
    let Some(dir) = khipu_dir() else {
        return "sin directorio de datos".into();
    };
    let addr = bind_addr();
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => return format!("no se pudo escuchar en {addr}: {e}"),
    };
    // Puerto efectivo (resuelve `:0` si se usara) para anunciarlo en la baliza.
    let tcp_port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let path = dir.join("compartido.khipu");
    std::thread::spawn(move || {
        khipu_share::net::serve_loop(listener, move || std::fs::read(&path));
    });
    // Baliza periódica para que los pares nos descubran sin saber la IP.
    let beacon = khipu_share::discovery::Beacon {
        author: model.keypair.as_ref().map(|k| k.public_key()).unwrap_or([0u8; 32]),
        port: tcp_port,
        name: "khipu".into(),
    };
    std::thread::spawn(move || loop {
        let _ = khipu_share::discovery::anunciar(&beacon);
        std::thread::sleep(std::time::Duration::from_secs(2));
    });
    model.publishing = true;

    // Además del TCP/LAN, servimos por libp2p (cifrado, WAN). El nodo se
    // arma perezoso; servimos `compartido.khipu` y nos anunciamos en la DHT.
    let p2p_status = if ensure_p2p(model) {
        let dir2 = dir.clone();
        if let Some(p) = model.p2p.as_mut() {
            if !p.serving {
                let path2 = dir2.join("compartido.khipu");
                let node = p.node.clone();
                let _g = p.rt.enter();
                node.run_serve(move || std::fs::read(&path2).ok());
                node.anunciar();
                p.serving = true;
            }
            // Si hay un relay configurado (KHIPU_RELAY=/ip4/.../p2p/<id>),
            // reservamos un circuito ahí para ser alcanzables detrás de NAT.
            // Async (dial + identify + reserva tardan ~2s): cuando termina,
            // reentra con Msg::RelayReady para mostrar la dirección.
            if let Ok(relay) = std::env::var("KHIPU_RELAY") {
                let (rt, node, h2) = (p.rt.clone(), p.node.clone(), h.clone());
                rt.spawn(async move {
                    let _ = node.dial_str(&relay);
                    // Esperamos a que AutoNAT confirme la dirección del relay
                    // (boot_delay + dial-back) antes de pedir la reserva.
                    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                    let circuit = format!("{relay}/p2p-circuit");
                    let msg = match node.listen_str(&circuit).await {
                        Ok(addr) => addr,
                        Err(e) => format!("falló reservar circuito: {e}"),
                    };
                    h2.dispatch(Msg::RelayReady(msg));
                });
            }
            format!(" · libp2p: {}", p.dial_addr)
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    format!("publicando en {addr} (LAN){p2p_status}")
}

/// Prefijo hex (4 bytes / 8 hex) de un hash, para mostrar una dirección
/// de contenido sin abrumar.
pub(crate) fn hex8(hash: &[u8; 32]) -> String {
    hash[..4].iter().map(|b| format!("{b:02x}")).collect()
}
