//! Protocolo de cable del daemon — frames postcard con prefijo de largo.
//!
//! Cada mensaje va como `u32` little-endian (largo) + bytes postcard.
//! Mismo encuadre que rimay-verbo y el resto de sockets de la suite,
//! pero en variante **sincrónica** (`std::io::Read/Write`) porque el
//! daemon de píxeles no usa tokio (ver doc del crate).

use std::io::{self, ErrorKind, Read, Write};

use pixel_verbo_core::{Imagen, ModelId, OpPixel};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Tope de tamaño de un frame. Una imagen 4K Rgba8 sin compresión pesa
/// ~33 MiB; subimos el tope a 64 MiB para dejar margen al postcard y a
/// los campos de prompt/params, sin volverse atractivo para una
/// DOS por frame gigante.
const MAX_FRAME: usize = 64 * 1024 * 1024;

/// Petición del cliente al daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Handshake: pide la identidad del modelo servido.
    ModelId,
    /// Health check sin invocar el modelo.
    Ping,
    /// Ejecuta una op. La entrada va `Some` salvo para `OpPixel::Generar`.
    Aplicar { op: OpPixel, entrada: Option<Imagen> },
}

/// Respuesta del daemon al cliente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    ModelId(ModelId),
    Pong,
    Imagen(Imagen),
    /// El backend falló; el texto es el `Display` del `Error`.
    Error(String),
}

/// Serializa `msg` y lo escribe como frame con prefijo de largo.
pub fn write_frame<W, T>(w: &mut W, msg: &T) -> io::Result<()>
where
    W: Write,
    T: Serialize,
{
    let bytes =
        postcard::to_stdvec(msg).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "frame demasiado grande",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(&bytes)?;
    w.flush()?;
    Ok(())
}

/// Lee un frame y lo deserializa. `Ok(None)` si el peer cerró limpio
/// antes de empezar un frame nuevo (EOF esperado).
pub fn read_frame<R, T>(r: &mut R) -> io::Result<Option<T>>
where
    R: Read,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "frame demasiado grande",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    let msg = postcard::from_bytes(&buf)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frame_round_trip() {
        let mut buf: Vec<u8> = Vec::new();
        let req = Request::Aplicar {
            op: OpPixel::Restyle {
                prompt: "tropical".into(),
            },
            entrada: None,
        };
        write_frame(&mut buf, &req).unwrap();
        let mut cursor = Cursor::new(buf);
        let got: Request = read_frame(&mut cursor).unwrap().unwrap();
        assert!(matches!(got, Request::Aplicar { .. }));
    }

    #[test]
    fn empty_stream_is_none() {
        let mut cursor: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let got: Option<Request> = read_frame(&mut cursor).unwrap();
        assert!(got.is_none());
    }
}
