//! Integración: el rail soberano sobre libp2p entre nodos reales en localhost,
//! con **descubrimiento por DHT** (sin conocer la dirección del destino).
//! Espeja `khipu-brahman/tests/p2p_roundtrip.rs`: un rendezvous arma la malla;
//! el receptor se anuncia bajo su identidad; el emisor lo descubre por la DHT y
//! le empuja un sobre, que llega y se verifica.

use std::thread;
use std::time::{Duration, Instant};

use agora_core::Keypair;
use paloma_core::{Address, Flags, Message, MessageId, SignatureStatus};
use paloma_rail::{open, seal, RailTransport};
use paloma_rail_net::Libp2pRail;

fn mensaje(subject: &str, body: &str) -> Message {
    Message {
        id: MessageId("<x@suyu>".into()),
        from: Address::named("Ana", "ana@suyu.net"),
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: subject.into(),
        date: 0,
        in_reply_to: None,
        references: vec![],
        body_text: body.into(),
        body_html: None,
        flags: Flags::default(),
        signature: SignatureStatus::Unsigned,
        mailbox: "Borradores".into(),
        cuerpos: vec![],
        signer: None,
    }
}

#[test]
fn rail_libp2p_descubre_por_dht_y_entrega() {
    let ana = Keypair::from_seed([1; 32]); // receptor
    let bob = Keypair::from_seed([2; 32]); // emisor

    // Rendezvous: nodo de la malla al que ambos se conectan para poblar la DHT.
    let (rdv, _rx_r) = Libp2pRail::new([9; 32], [9; 32]).unwrap();
    let r_addr = rdv.listen("/ip4/127.0.0.1/tcp/0").unwrap();

    // Ana (receptora): escucha, se une a la malla y se anuncia bajo su identidad.
    let (rail_a, rx_a) = Libp2pRail::new([1; 32], ana.public_key()).unwrap();
    let a_addr = rail_a.listen("/ip4/127.0.0.1/tcp/0").unwrap();
    rail_a.dial(&r_addr).unwrap();
    thread::sleep(Duration::from_secs(2));
    rail_a.announce();

    // Bob (emisor): escucha y se une a la malla.
    let (rail_b, _rx_b) = Libp2pRail::new([2; 32], bob.public_key()).unwrap();
    let _b_addr = rail_b.listen("/ip4/127.0.0.1/tcp/0").unwrap();
    rail_b.dial(&r_addr).unwrap();
    thread::sleep(Duration::from_secs(2));

    // Bob descubre a Ana por la DHT (sin conocer su dirección).
    let descubierta = {
        let t0 = Instant::now();
        loop {
            if !rail_b.resolve(&ana.public_key()).is_empty() {
                break true;
            }
            if t0.elapsed() > Duration::from_secs(20) {
                break false;
            }
            thread::sleep(Duration::from_millis(300));
        }
    };
    assert!(descubierta, "Bob debe descubrir la identidad de Ana por la DHT");

    // La identidad ya se descubrió por DHT (arriba). Para abrir el stream el
    // swarm necesita una dirección dialable del destino: en producción llega por
    // el relay/rendezvous (reserva de circuito); acá la sembramos con la addr de
    // Ana —representa esa dirección ya conocida— para certificar la ENTREGA por
    // libp2p. La resolución autónoma PeerId→addr vía relay queda como refinamiento.
    rail_b.dial(&a_addr).unwrap();
    thread::sleep(Duration::from_secs(1));

    // Bob sella y empuja; reintenta por si el stream tarda en establecerse.
    let env = seal(&bob, ana.public_key(), &mensaje("minga", "vení el sábado"), vec![]).unwrap();
    let recibido = {
        let t0 = Instant::now();
        loop {
            rail_b.send(ana.public_key(), &env).unwrap();
            if let Ok(e) = rx_a.recv_timeout(Duration::from_millis(800)) {
                break Some(e);
            }
            if t0.elapsed() > Duration::from_secs(20) {
                break None;
            }
        }
    };
    let recibido = recibido.expect("Ana debe recibir el sobre por libp2p");
    let msg = open(&recibido, ana.public_key()).unwrap();
    assert_eq!(msg.subject, "minga");
    assert_eq!(msg.signature, SignatureStatus::Verified);
}
