//! `willay-emit` — el cliente fino del centro de eventos y el **códec de marcos**.
//!
//! Un productor (hapiy, el clipboard, el espejo de pata-notify) abre un
//! [`Emisor`] y manda [`Evento`]s al daemon willay por el socket Unix. La regla
//! de oro: **emitir nunca debe romper al productor** — si el daemon no corre,
//! [`emitir_silencioso`] no-opea. El daemon (`willay-daemon`) reusa el códec
//! [`leer_marco`]/[`escribir_marco`] de acá para hablar el mismo idioma de wire.
//!
//! Wire: cada mensaje es `u32 LE longitud ++ postcard(bytes)`. Ver
//! `shared/willay/SDD.md` §1.1.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use willay_core::proto::{Respuesta, Solicitud};
use willay_core::Evento;

/// Tope de tamaño de un marco (64 MiB) — guarda contra una longitud corrupta
/// que pediría una reserva gigante. Un evento sano pesa kilobytes.
const MARCO_MAX: u32 = 64 * 1024 * 1024;

/// Ruta del socket del daemon: `$XDG_RUNTIME_DIR/willay.sock`, con fallback a
/// `/tmp/willay-<uid?>.sock` si no hay runtime dir.
pub fn socket_path() -> PathBuf {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("willay.sock");
    }
    std::env::temp_dir().join("willay.sock")
}

/// Escribe un marco length-prefixed (`u32 LE` + bytes).
pub fn escribir_marco<W: Write>(w: &mut W, bytes: &[u8]) -> io::Result<()> {
    let len = bytes.len() as u32;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(bytes)?;
    w.flush()
}

/// Lee un marco length-prefixed. `Ok(None)` en EOF limpio (la conexión cerró
/// entre marcos); `Err` si el stream corta a la mitad o la longitud es absurda.
pub fn leer_marco<R: Read>(r: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut lb = [0u8; 4];
    match r.read_exact(&mut lb) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(lb);
    if len > MARCO_MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "marco demasiado grande"));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Cliente del daemon. Una conexión reusable: puede emitir y consultar varias
/// veces antes de dropearse.
pub struct Emisor {
    stream: UnixStream,
}

impl Emisor {
    /// Conecta al socket por defecto ([`socket_path`]).
    pub fn conectar() -> io::Result<Self> {
        Self::conectar_en(socket_path())
    }

    /// Conecta a un socket explícito (para tests o instancias alternativas).
    pub fn conectar_en(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
        Ok(Self { stream: UnixStream::connect(path)? })
    }

    /// Manda una solicitud y espera su respuesta (un round-trip).
    pub fn pedir(&mut self, sol: &Solicitud) -> anyhow::Result<Respuesta> {
        let bytes = postcard::to_stdvec(sol)?;
        escribir_marco(&mut self.stream, &bytes)?;
        let resp = leer_marco(&mut self.stream)?
            .ok_or_else(|| anyhow::anyhow!("el daemon cerró la conexión sin responder"))?;
        Ok(postcard::from_bytes(&resp)?)
    }

    /// Se **suscribe** a los cambios del índice y bloquea, llamando `on_cambio`
    /// por cada notificación push del daemon, hasta que la conexión caiga. La
    /// conexión queda dedicada a esto (no la uses para consultar). Pensado para
    /// correr en su propio hilo (el feed lo usa para refrescar al instante en vez
    /// de pollear).
    pub fn escuchar_cambios(mut self, mut on_cambio: impl FnMut()) -> anyhow::Result<()> {
        let bytes = postcard::to_stdvec(&Solicitud::Suscribir)?;
        escribir_marco(&mut self.stream, &bytes)?;
        // En modo suscripción, cada frame del daemon es un `Cambio`.
        while leer_marco(&mut self.stream)?.is_some() {
            on_cambio();
        }
        Ok(())
    }

    /// Emite un evento (escritura). Error si el daemon respondió algo que no es
    /// [`Respuesta::Ok`].
    pub fn emitir(&mut self, e: &Evento) -> anyhow::Result<()> {
        match self.pedir(&Solicitud::Emitir(e.clone()))? {
            Respuesta::Ok => Ok(()),
            Respuesta::Error(m) => Err(anyhow::anyhow!("daemon: {m}")),
            Respuesta::Eventos(_) | Respuesta::Cambio => {
                Err(anyhow::anyhow!("respuesta inesperada a Emitir"))
            }
        }
    }
}

/// Microsegundos desde epoch — el `ts_usec` de un evento que acaba de ocurrir.
/// Conveniencia para los productores (el esquema `willay-core` es no_std y no
/// puede leer el reloj del sistema).
pub fn ahora_usec() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Emite un evento **sin fallar nunca**: si el daemon no corre o algo sale mal,
/// no-opea (silencioso). Es la vía que usan los productores — un screenshot o un
/// copy-paste no se rompen porque el centro de eventos esté caído.
pub fn emitir_silencioso(e: &Evento) {
    if let Ok(mut em) = Emisor::conectar() {
        let _ = em.emitir(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_de_un_marco_por_un_pipe_en_memoria() {
        let mut buf: Vec<u8> = Vec::new();
        escribir_marco(&mut buf, b"hola").unwrap();
        let mut cur = io::Cursor::new(buf);
        assert_eq!(leer_marco(&mut cur).unwrap().as_deref(), Some(&b"hola"[..]));
        // Segundo read sobre EOF limpio → None.
        assert_eq!(leer_marco(&mut cur).unwrap(), None);
    }

    #[test]
    fn emitir_silencioso_no_panica_sin_daemon() {
        // Apunta a un socket inexistente; debe tragarse el error.
        std::env::set_var("XDG_RUNTIME_DIR", std::env::temp_dir().join("willay-no-existe-xyz"));
        let e = Evento::nuevo(
            willay_core::Clase::Clip,
            1,
            "test",
            "x",
            "",
            willay_core::Payload::Nada,
        );
        emitir_silencioso(&e); // no debe panicar
    }

    #[test]
    fn marco_demasiado_grande_es_error() {
        let mut bytes = (MARCO_MAX + 1).to_le_bytes().to_vec();
        bytes.extend_from_slice(b"x");
        let mut cur = io::Cursor::new(bytes);
        assert!(leer_marco(&mut cur).is_err());
    }
}
