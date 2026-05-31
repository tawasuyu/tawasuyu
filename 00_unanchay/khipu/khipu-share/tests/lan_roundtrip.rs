//! Integración: el camino completo de compartir por LAN, todo en loopback.
//!
//! Sella un cuaderno → lo sirve por TCP → anuncia su baliza → otro par lo
//! descubre por UDP, le jala el sobre y lo verifica. Es la cadena que
//! ejercitan los botones «publicar» y «recibir» de khipu-app, sin GUI.

use std::net::{Ipv4Addr, TcpListener, UdpSocket};
use std::time::Duration;

use agora_core::Keypair;
use khipu_core::NoteStore;
use khipu_share::discovery::{escuchar_en, Beacon};
use khipu_share::{net, open, seal, SharedNote};

#[test]
fn descubrir_jalar_y_verificar_de_punta_a_punta() {
    // --- Lado que publica ---
    let autor = Keypair::from_seed([21u8; 32]);
    let sobre = seal(
        &autor,
        vec![SharedNote {
            title: "Compartida por LAN".into(),
            body: "viajó por descubrimiento + TCP".into(),
            tags: vec!["red".into()],
        }],
        100,
    )
    .unwrap();
    let bytes = sobre.to_bytes().unwrap();

    // Sirve el sobre por TCP en un puerto efímero.
    let tcp = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let tcp_port = tcp.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let _ = net::serve_once(&tcp, &bytes);
    });

    // Anuncia la baliza apuntando a ese puerto TCP.
    let beacon = Beacon {
        author: autor.public_key(),
        port: tcp_port,
        name: "khipu publicador".into(),
    };

    // --- Lado que recibe ---
    // Escucha balizas en un puerto UDP efímero (en la app sería el
    // estándar 7701); el publicador le manda la suya por loopback.
    let listener = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let destino = listener.local_addr().unwrap();
    let emisor = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    emisor.send_to(&beacon.encode(), destino).unwrap();

    // Descubre al par y le jala el cuaderno.
    let pares = escuchar_en(&listener, Duration::from_millis(500));
    assert_eq!(pares.len(), 1, "debería haber descubierto un par");
    let par = &pares[0];
    assert_eq!(par.fetch_addr.port(), tcp_port);
    assert_eq!(par.beacon.author, autor.public_key());

    let recibido = net::fetch(par.fetch_addr).unwrap();

    // Verifica firma + hash e ingiere como nota fresca.
    let bundle = open(&recibido).unwrap();
    let mut store = NoteStore::new();
    let resultado = khipu_share::import_into(&mut store, bundle, 9_000);
    assert_eq!(resultado.created.len(), 1);
    let nota = store.get(resultado.created[0]).unwrap();
    assert_eq!(nota.title, "Compartida por LAN");
    assert!(nota.tags.contains(&"red".to_string()));
    // La nota lleva la procedencia del autor que la selló.
    assert!(nota.tags.contains(&khipu_share::tag_de(&autor.public_key())));
    // Gravedad fresca en el receptor.
    assert_eq!(nota.mass, 1.0);
    assert_eq!(nota.last_access, 9_000);
}
