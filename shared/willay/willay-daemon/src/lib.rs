//! `willay-daemon` — el escritor único del centro de eventos.
//!
//! Es el dueño del índice `sled` (`willay-store`): como sled lockea la DB a un
//! proceso, **todas** las escrituras y lecturas pasan por acá. Atiende el socket
//! Unix con un hilo por conexión, hablando el códec de marcos de `willay-emit`.
//! Cada solicitud se despacha a [`manejar`]. Ver `shared/willay/SDD.md` §1.1.

use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;

use willay_core::proto::{Respuesta, Solicitud};
use willay_emit::{escribir_marco, leer_marco};
use willay_store::Indice;

/// Aplica una solicitud al índice y produce su respuesta. Aislado del socket
/// para poder testear el despacho directo. Un error del store no tira la
/// conexión: se devuelve como [`Respuesta::Error`].
pub fn manejar(sol: Solicitud, indice: &Indice) -> Respuesta {
    let r = match sol {
        Solicitud::Emitir(e) => indice.append(&e).map(|_| Respuesta::Ok),
        Solicitud::Recientes(n) => indice.recientes(n as usize).map(Respuesta::Eventos),
        Solicitud::PorClase(c, n) => indice.por_clase(c, n as usize).map(Respuesta::Eventos),
        Solicitud::Buscar(s, n) => indice.buscar(&s, n as usize).map(Respuesta::Eventos),
    };
    r.unwrap_or_else(|e| Respuesta::Error(e.to_string()))
}

/// Atiende una conexión: lee marcos (solicitudes) y responde hasta que el par
/// cierre (EOF limpio entre marcos → `Ok(())`).
pub fn atender(mut stream: UnixStream, indice: Indice) -> anyhow::Result<()> {
    while let Some(bytes) = leer_marco(&mut stream)? {
        let resp = match postcard::from_bytes::<Solicitud>(&bytes) {
            Ok(sol) => manejar(sol, &indice),
            Err(e) => Respuesta::Error(format!("solicitud ilegible: {e}")),
        };
        let out = postcard::to_stdvec(&resp)?;
        escribir_marco(&mut stream, &out)?;
    }
    Ok(())
}

/// Bucle de aceptación: un hilo por conexión, todos comparten el índice (sled es
/// `Arc` por dentro — clonar es barato, y este daemon es el único escritor).
/// Bloquea para siempre; el bin lo corre tras bindear el socket.
pub fn servir(listener: UnixListener, indice: Indice) {
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let indice = indice.clone();
                thread::spawn(move || {
                    if let Err(e) = atender(stream, indice) {
                        eprintln!("willay-daemon · conexión: {e}");
                    }
                });
            }
            Err(e) => eprintln!("willay-daemon · accept: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willay_core::{Clase, Evento, Payload};

    fn ev(ts: u64, titulo: &str) -> Evento {
        Evento::nuevo(Clase::Clip, ts, "test", titulo, "", Payload::Nada)
    }

    #[test]
    fn manejar_emite_y_consulta() {
        let ix = Indice::temporary().unwrap();
        assert_eq!(manejar(Solicitud::Emitir(ev(100, "a")), &ix), Respuesta::Ok);
        assert_eq!(manejar(Solicitud::Emitir(ev(200, "b")), &ix), Respuesta::Ok);
        match manejar(Solicitud::Recientes(10), &ix) {
            Respuesta::Eventos(v) => {
                assert_eq!(v.len(), 2);
                assert_eq!(v[0].ts_usec, 200, "recientes primero");
            }
            other => panic!("esperaba Eventos, vino {other:?}"),
        }
    }

    #[test]
    fn manejar_por_clase_y_buscar() {
        let ix = Indice::temporary().unwrap();
        manejar(Solicitud::Emitir(Evento::nuevo(Clase::Captura, 1, "hapiy", "cap", "", Payload::Nada)), &ix);
        manejar(Solicitud::Emitir(ev(2, "API key")), &ix);
        match manejar(Solicitud::PorClase(Clase::Captura, 10), &ix) {
            Respuesta::Eventos(v) => assert_eq!(v.len(), 1),
            other => panic!("{other:?}"),
        }
        // "key" matchea sólo el título "API key" (ojo: "api" matchearía también
        // el origen "hapiy" — la búsqueda barre origen/título/cuerpo).
        match manejar(Solicitud::Buscar("key".into(), 10), &ix) {
            Respuesta::Eventos(v) => assert_eq!(v.len(), 1),
            other => panic!("{other:?}"),
        }
    }
}
