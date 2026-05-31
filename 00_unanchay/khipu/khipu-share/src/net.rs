//! Transporte LAN mínimo para sobres khipu sobre TCP.
//!
//! El sobre ya es firmado y direccionado por contenido, así que el
//! transporte no necesita ser confiable: quien recibe verifica con
//! [`crate::open`] antes de creer nada. Esto es `std::net` puro — sin
//! libp2p ni async — pensado para "jalar el cuaderno de un par en la LAN".
//!
//! Marco de cable: un `u32` big-endian con el largo del sobre, seguido
//! del sobre serializado (postcard). Un sobre por conexión.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};

use crate::{ShareError, SignedBundle};

/// Tope defensivo para un sobre entrante (64 MiB): evita que un par
/// hostil pida un alloc gigante declarando un largo inflado.
const MAX_SOBRE: u32 = 64 * 1024 * 1024;

/// Falla del transporte de sobres.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("io de red: {0}")]
    Io(String),
    #[error("marco inválido: {0}")]
    Protocolo(String),
    #[error(transparent)]
    Sobre(#[from] ShareError),
}

impl From<io::Error> for NetError {
    fn from(e: io::Error) -> Self {
        NetError::Io(e.to_string())
    }
}

fn write_frame(stream: &mut TcpStream, payload: &[u8]) -> io::Result<()> {
    if payload.len() as u64 > MAX_SOBRE as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sobre demasiado grande para el marco",
        ));
    }
    stream.write_all(&(payload.len() as u32).to_be_bytes())?;
    stream.write_all(payload)?;
    stream.flush()
}

fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>, NetError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_SOBRE {
        return Err(NetError::Protocolo(format!(
            "largo declarado {len} excede el tope"
        )));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

/// Conecta a `addr`, lee un sobre y lo deserializa. **No lo verifica** —
/// el caller debe pasar el resultado por [`crate::open`] antes de confiar.
pub fn fetch(addr: impl ToSocketAddrs) -> Result<SignedBundle, NetError> {
    let mut stream = TcpStream::connect(addr)?;
    let bytes = read_frame(&mut stream)?;
    Ok(SignedBundle::from_bytes(&bytes)?)
}

/// Atiende una sola conexión: manda `payload` y vuelve. Útil para tests
/// y para un "compartir una vez".
pub fn serve_once(listener: &TcpListener, payload: &[u8]) -> io::Result<()> {
    let (mut stream, _) = listener.accept()?;
    write_frame(&mut stream, payload)
}

/// Atiende conexiones para siempre. Por cada una llama a `supply` para
/// obtener los bytes a mandar — típicamente leer `compartido.khipu` del
/// disco, así sirve siempre la versión vigente. Una conexión cuyo
/// `supply` falla (todavía no hay sobre, p. ej.) se salta sin tumbar el
/// servidor. Bloqueante: pensado para correr en su propio hilo.
pub fn serve_loop<F>(listener: TcpListener, supply: F)
where
    F: Fn() -> io::Result<Vec<u8>>,
{
    for conn in listener.incoming() {
        let Ok(mut stream) = conn else { continue };
        if let Ok(bytes) = supply() {
            let _ = write_frame(&mut stream, &bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{open, seal, SharedNote};
    use agora_core::Keypair;

    fn nota(title: &str, body: &str) -> SharedNote {
        SharedNote {
            title: title.into(),
            body: body.into(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn fetch_recovers_a_served_bundle_and_verifies() {
        let kp = Keypair::from_seed([11u8; 32]);
        let sobre = seal(&kp, vec![nota("Red", "hola por TCP")], 1).unwrap();
        let bytes = sobre.to_bytes().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || serve_once(&listener, &bytes).unwrap());

        let recibido = fetch(addr).unwrap();
        server.join().unwrap();

        assert_eq!(recibido, sobre);
        // El sobre que llegó por la red verifica firma + hash.
        let bundle = open(&recibido).unwrap();
        assert_eq!(bundle.notes[0].title, "Red");
    }

    #[test]
    fn serve_loop_serves_the_current_supply_each_time() {
        let kp = Keypair::from_seed([12u8; 32]);
        let sobre = seal(&kp, vec![nota("A", "a")], 1).unwrap();
        let bytes = sobre.to_bytes().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        // El servidor corre en su hilo; lo dejamos colgado al terminar el
        // test (los dos fetch ya probaron lo que importa).
        std::thread::spawn(move || {
            serve_loop(listener, move || Ok(bytes.clone()));
        });

        // Dos pares distintos jalan el mismo cuaderno.
        let a = fetch(addr).unwrap();
        let b = fetch(addr).unwrap();
        assert_eq!(a, sobre);
        assert_eq!(b, sobre);
    }

    #[test]
    fn fetch_against_nothing_is_an_error_not_a_panic() {
        // Puerto cerrado: connect falla, devolvemos NetError::Io.
        let err = fetch("127.0.0.1:1").unwrap_err();
        assert!(matches!(err, NetError::Io(_)));
    }
}
