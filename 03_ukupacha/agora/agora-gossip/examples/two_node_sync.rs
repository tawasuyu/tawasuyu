//! Demo ejecutable del protocolo gossip de agora.
//!
//! `cargo run -p agora-gossip --example two_node_sync`
//!
//! Construye dos `TrustGraph`s con atestaciones distintas, simula una
//! ronda anti-entropy completa entre ambos nodos (announce → request →
//! bundle, en los dos sentidos) y verifica que terminan idénticos. Es
//! la prueba viva de que [`responder`] basta para cerrar la convergencia
//! en una ronda — sin red, sin firmas adicionales, sólo intercambio de
//! mensajes.

use agora_core::{Attestation, Claim, IdentityKind, Keypair};
use agora_gossip::{responder, Digest, GossipStats, Message};
use agora_graph::TrustGraph;

const T0: u64 = 1_700_000_000;

fn attest(by: &Keypair, subject_id: agora_core::IdentityId, pred: &str, val: &str) -> Attestation {
    Attestation::create(by, Claim::new(subject_id, pred, val, T0))
}

fn print_digest(rotulo: &str, g: &TrustGraph) {
    let d = Digest::from_graph(g);
    println!("    {rotulo}: {} atestaciones", d.len());
    for h in d.haves.iter().take(3) {
        // Primeros 8 bytes en hex — suficiente para distinguir.
        print!("      ");
        for b in &h[..8] {
            print!("{b:02x}");
        }
        println!("…");
    }
}

fn main() {
    println!("\n  agora · convergencia anti-entropy de TrustGraphs\n");

    // --- Identidades determinísticas (semillas fijas → demo reproducible). ---
    let yumaira = Keypair::from_seed([20; 32]);
    let venezuela = Keypair::from_seed([10; 32]);
    let comunidad = Keypair::from_seed([30; 32]);
    let vecina = Keypair::from_seed([40; 32]);

    // --- Nodo A conoce sólo la nacionalidad. ---
    let mut nodo_a = TrustGraph::new();
    nodo_a.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
    nodo_a.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
    nodo_a.add_attestation(attest(
        &venezuela,
        yumaira.identity_id(),
        "nacionalidad",
        "venezolana",
    ))
    .unwrap();

    // --- Nodo B conoce sólo membresía + habilidad. ---
    let mut nodo_b = TrustGraph::new();
    nodo_b.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
    nodo_b.register(comunidad.identity(IdentityKind::Community, "Vecinos del Valle"));
    nodo_b.register(vecina.identity(IdentityKind::Person, "Carmen"));
    nodo_b.add_attestation(attest(
        &comunidad,
        yumaira.identity_id(),
        "miembro-de",
        "Vecinos del Valle",
    ))
    .unwrap();
    nodo_b.add_attestation(attest(
        &vecina,
        yumaira.identity_id(),
        "habilidad",
        "soldadura",
    ))
    .unwrap();

    println!("  estado inicial:");
    print_digest("A", &nodo_a);
    print_digest("B", &nodo_b);

    let mut stats_a = GossipStats::default();
    let mut stats_b = GossipStats::default();

    // --- Ronda 1: A anuncia, B detecta lo que le falta y pide. ---
    println!("\n  ronda 1  A → B");
    let anun_a = Message::Announce(Digest::from_graph(&nodo_a));
    let req_b = responder(&mut nodo_b, &anun_a, &mut stats_b).expect("B debería pedir");
    if let Message::Request(hashes) = &req_b {
        println!("    B pide {} atestación(es)", hashes.len());
    }
    let bun_a = responder(&mut nodo_a, &req_b, &mut stats_a).expect("A debería servir");
    if let Message::Bundle(bundle) = &bun_a {
        println!("    A entrega {} atestación(es)", bundle.len());
    }
    let _ = responder(&mut nodo_b, &bun_a, &mut stats_b);

    // --- Ronda 2: B anuncia, A detecta y pide. ---
    println!("\n  ronda 2  B → A");
    let anun_b = Message::Announce(Digest::from_graph(&nodo_b));
    let req_a = responder(&mut nodo_a, &anun_b, &mut stats_a).expect("A debería pedir");
    if let Message::Request(hashes) = &req_a {
        println!("    A pide {} atestación(es)", hashes.len());
    }
    let bun_b = responder(&mut nodo_b, &req_a, &mut stats_b).expect("B debería servir");
    if let Message::Bundle(bundle) = &bun_b {
        println!("    B entrega {} atestación(es)", bundle.len());
    }
    let _ = responder(&mut nodo_a, &bun_b, &mut stats_a);

    println!("\n  estado final:");
    print_digest("A", &nodo_a);
    print_digest("B", &nodo_b);

    let d_a = Digest::from_graph(&nodo_a);
    let d_b = Digest::from_graph(&nodo_b);
    assert_eq!(d_a, d_b, "los nodos no convergieron");
    println!("\n  ✔ los digests coinciden — ambos nodos tienen {} atestaciones", d_a.len());

    println!("\n  stats A: ok={} rechazados={} request_serve={} sin_match={}",
        stats_a.bundles_recibidos_ok,
        stats_a.bundles_recibidos_rechazados,
        stats_a.requests_atendidos,
        stats_a.requests_sin_match,
    );
    println!("  stats B: ok={} rechazados={} request_serve={} sin_match={}",
        stats_b.bundles_recibidos_ok,
        stats_b.bundles_recibidos_rechazados,
        stats_b.requests_atendidos,
        stats_b.requests_sin_match,
    );

    // --- Ronda 3 (post-convergencia): no debería haber mensajes nuevos. ---
    let anun = Message::Announce(Digest::from_graph(&nodo_a));
    let resp = responder(&mut nodo_b, &anun, &mut stats_b);
    println!(
        "\n  idempotencia: B {} a un Announce post-convergencia",
        if resp.is_none() { "no responde (correcto)" } else { "respondería (BUG)" },
    );
    assert!(resp.is_none());

    println!();
}
