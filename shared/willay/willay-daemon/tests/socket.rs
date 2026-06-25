//! Round-trip end-to-end: emisor → socket → daemon → índice → consulta.

use std::os::unix::net::UnixListener;
use std::thread;

use willay_core::proto::{Respuesta, Solicitud};
use willay_core::{Clase, Evento, Payload};
use willay_emit::Emisor;
use willay_store::Indice;

#[test]
fn emite_y_consulta_por_socket() {
    let path = std::env::temp_dir().join(format!("willay-test-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let indice = Indice::temporary().unwrap();
    // El bucle de servicio corre en un hilo que se mata al terminar el test.
    thread::spawn(move || willay_daemon::servir(listener, indice));

    let mut em = Emisor::conectar_en(&path).unwrap();
    let e = Evento::nuevo(
        Clase::Captura,
        100,
        "hapiy",
        "Captura DP-1",
        "",
        Payload::Archivo { ruta: "/tmp/x.png".into(), mime: "image/png".into() },
    );
    em.emitir(&e).unwrap();

    match em.pedir(&Solicitud::Recientes(10)).unwrap() {
        Respuesta::Eventos(v) => {
            assert_eq!(v.len(), 1);
            assert_eq!(v[0].id, e.id, "el evento vuelve idéntico por la wire");
        }
        other => panic!("esperaba Eventos, vino {other:?}"),
    }

    // Una segunda solicitud por la MISMA conexión (el daemon la mantiene viva).
    match em.pedir(&Solicitud::PorClase(Clase::Notificacion, 10)).unwrap() {
        Respuesta::Eventos(v) => assert!(v.is_empty(), "no hay notificaciones"),
        other => panic!("{other:?}"),
    }

    let _ = std::fs::remove_file(&path);
}
