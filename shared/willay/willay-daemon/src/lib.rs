//! `willay-daemon` — el escritor único del centro de eventos.
//!
//! Es el dueño del índice `sled` (`willay-store`): como sled lockea la DB a un
//! proceso, **todas** las escrituras y lecturas pasan por acá. Atiende el socket
//! Unix con un hilo por conexión, hablando el códec de marcos de `willay-emit`.
//! Cada solicitud se despacha a [`manejar`]. Ver `shared/willay/SDD.md` §1.1.

use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use willay_core::proto::{Respuesta, Solicitud};
use willay_emit::{escribir_marco, leer_marco};
use willay_store::Indice;

/// Los suscriptores vivos: un canal por conexión suscrita. El hilo que hace
/// `append` empuja `()` a cada uno; el hilo de cada suscriptor traduce eso en un
/// frame `Cambio` a su socket (así nadie comparte el stream entre hilos).
type Subs = Arc<Mutex<Vec<Sender<()>>>>;

/// Avisa a todos los suscriptores que el índice cambió; descarta los muertos
/// (su receptor se dropeó al caer la conexión).
fn notificar(subs: &Subs) {
    if let Ok(mut g) = subs.lock() {
        g.retain(|tx| tx.send(()).is_ok());
    }
}

/// Aplica una solicitud al índice y produce su respuesta. Aislado del socket
/// para poder testear el despacho directo. Un error del store no tira la
/// conexión: se devuelve como [`Respuesta::Error`].
pub fn manejar(sol: Solicitud, indice: &Indice) -> Respuesta {
    let r = match sol {
        Solicitud::Emitir(e) => indice.append(&e).map(|_| Respuesta::Ok),
        Solicitud::Recientes(n) => indice.recientes(n as usize).map(Respuesta::Eventos),
        Solicitud::PorClase(c, n) => indice.por_clase(c, n as usize).map(Respuesta::Eventos),
        Solicitud::Buscar(s, n) => indice.buscar(&s, n as usize).map(Respuesta::Eventos),
        Solicitud::Limpiar => indice.clear().map(|_| Respuesta::Ok),
        // `Suscribir` lo intercepta `atender` antes de llegar acá (cambia el modo
        // de la conexión); si cae acá es un error de ruteo.
        Solicitud::Suscribir => Ok(Respuesta::Error("suscribir no pasa por manejar".to_string())),
    };
    r.unwrap_or_else(|e| Respuesta::Error(e.to_string()))
}

/// Atiende una conexión: lee marcos (solicitudes) y responde hasta que el par
/// cierre (EOF limpio entre marcos → `Ok(())`). Un `Suscribir` convierte la
/// conexión en un canal de push (deja de leer solicitudes). Tras un `Emitir`
/// exitoso, notifica a los suscriptores.
pub fn atender(mut stream: UnixStream, indice: Indice, subs: Subs) -> anyhow::Result<()> {
    while let Some(bytes) = leer_marco(&mut stream)? {
        let sol = match postcard::from_bytes::<Solicitud>(&bytes) {
            Ok(s) => s,
            Err(e) => {
                let out = postcard::to_stdvec(&Respuesta::Error(format!("solicitud ilegible: {e}")))?;
                escribir_marco(&mut stream, &out)?;
                continue;
            }
        };
        if matches!(sol, Solicitud::Suscribir) {
            return atender_suscriptor(stream, subs);
        }
        let es_emitir = matches!(sol, Solicitud::Emitir(_));
        let resp = manejar(sol, &indice);
        let aplicado = matches!(resp, Respuesta::Ok);
        let out = postcard::to_stdvec(&resp)?;
        escribir_marco(&mut stream, &out)?;
        if es_emitir && aplicado {
            notificar(&subs);
        }
    }
    Ok(())
}

/// Modo suscriptor: registra esta conexión y bloquea, empujando un frame
/// `Cambio` cada vez que el índice cambia, hasta que el socket caiga.
fn atender_suscriptor(mut stream: UnixStream, subs: Subs) -> anyhow::Result<()> {
    let (tx, rx) = channel::<()>();
    if let Ok(mut g) = subs.lock() {
        g.push(tx);
    }
    let cambio = postcard::to_stdvec(&Respuesta::Cambio)?;
    while rx.recv().is_ok() {
        if escribir_marco(&mut stream, &cambio).is_err() {
            break; // el suscriptor se fue; `notificar` lo limpia al fallar su send
        }
    }
    Ok(())
}

/// Bucle de aceptación: un hilo por conexión, todos comparten el índice (sled es
/// `Arc` por dentro) y el registro de suscriptores. Bloquea para siempre; el bin
/// lo corre tras bindear el socket.
pub fn servir(listener: UnixListener, indice: Indice) {
    let subs: Subs = Arc::new(Mutex::new(Vec::new()));
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let indice = indice.clone();
                let subs = subs.clone();
                thread::spawn(move || {
                    if let Err(e) = atender(stream, indice, subs) {
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
    fn notificar_entrega_a_vivos_y_limpia_muertos() {
        let subs: Subs = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = channel::<()>();
        subs.lock().unwrap().push(tx);
        notificar(&subs);
        assert!(rx.try_recv().is_ok(), "el suscriptor vivo recibió el aviso");
        assert_eq!(subs.lock().unwrap().len(), 1);
        drop(rx); // el suscriptor se va
        notificar(&subs);
        assert_eq!(subs.lock().unwrap().len(), 0, "el muerto se descarta");
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
