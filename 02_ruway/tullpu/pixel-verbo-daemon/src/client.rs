//! El cliente bloqueante: consume un daemon presentándose como un
//! `Proveedor` local. Sin tokio — `std::os::unix::net::UnixStream`
//! con `read_exact`/`write_all` directos.
//!
//! Cada llamada abre una conexión nueva al socket. Es ligeramente más
//! caro que mantener una conexión persistente, pero elimina del cliente
//! toda la complejidad de reintento, pool de conexiones y reconexión
//! tras crash del daemon. Para los volúmenes esperados (uno a unos
//! pocos requests por interacción de usuario) el overhead es invisible.

use std::io::{BufReader, BufWriter};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use pixel_verbo_core::{Error, Imagen, ModelId, OpPixel, Proveedor};

use crate::wire::{read_frame, write_frame, Request, Response};

/// Cliente bloqueante de un [`crate::Servidor`]. Implementa
/// `pixel_verbo_core::Proveedor` — el consumidor no nota que el modelo
/// vive en otro proceso.
#[derive(Debug)]
pub struct ClienteBloqueante {
    path: PathBuf,
    model: ModelId,
}

impl ClienteBloqueante {
    /// Conecta al daemon, hace el handshake de modelo y guarda el
    /// `ModelId` en cache. Devuelve `Error::Backend` si la conexión
    /// inicial falla (socket inexistente, daemon caído).
    pub fn conectar(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        let resp = round_trip(&path, &Request::ModelId)?;
        let model = match resp {
            Response::ModelId(m) => m,
            otra => return Err(inesperada(otra)),
        };
        Ok(Self { path, model })
    }

    /// Health-check sin invocar al modelo.
    pub fn ping(&self) -> Result<(), Error> {
        match round_trip(&self.path, &Request::Ping)? {
            Response::Pong => Ok(()),
            otra => Err(inesperada(otra)),
        }
    }
}

impl Proveedor for ClienteBloqueante {
    fn model_id(&self) -> &ModelId {
        &self.model
    }

    fn aplicar(&self, op: &OpPixel, entrada: Option<Imagen>) -> Result<Imagen, Error> {
        let req = Request::Aplicar {
            op: op.clone(),
            entrada,
        };
        match round_trip(&self.path, &req)? {
            Response::Imagen(img) => Ok(img),
            Response::Error(s) => Err(Error::Backend(s)),
            otra => Err(inesperada(otra)),
        }
    }
}

/// Mapea una respuesta fuera de contrato a un `Error::Backend`.
fn inesperada(r: Response) -> Error {
    match r {
        Response::Error(e) => Error::Backend(e),
        _ => Error::Backend("respuesta del daemon pixel-verbo inesperada".into()),
    }
}

/// Un round-trip sobre un socket recién abierto: abre, escribe, lee, cierra.
fn round_trip(path: &Path, req: &Request) -> Result<Response, Error> {
    let stream = UnixStream::connect(path)
        .map_err(|e| Error::Backend(format!("conexión a {}: {e}", path.display())))?;
    let mut lector = BufReader::new(
        stream
            .try_clone()
            .map_err(|e| Error::Backend(format!("clonar socket: {e}")))?,
    );
    let mut escritor = BufWriter::new(stream);
    write_frame(&mut escritor, req)
        .map_err(|e| Error::Backend(format!("envío al daemon: {e}")))?;
    let resp = read_frame::<_, Response>(&mut lector)
        .map_err(|e| Error::Backend(format!("lectura del daemon: {e}")))?
        .ok_or_else(|| Error::Backend("daemon cerró sin responder".into()))?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conectar_a_socket_inexistente_da_backend_error() {
        let err = ClienteBloqueante::conectar("/tmp/pixel-verbo-no-existe.sock").unwrap_err();
        assert!(matches!(err, Error::Backend(_)));
    }
}
