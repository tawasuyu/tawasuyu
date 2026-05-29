//! El servidor: sirve un `Proveedor` sobre un socket Unix con un thread
//! por conexión. Sin tokio — `std::net` + `std::thread::spawn`.
//!
//! El servidor toma `Arc<P>` con `P: Proveedor + 'static`; cada conexión
//! corre en su propio thread y hace un round-trip por request hasta EOF.
//! La invocación al proveedor es bloqueante — los modelos de píxel
//! tardan decenas/cientos de milisegundos por op, lo que justifica
//! consumir un thread del kernel por cliente activo.

use std::io::{self, BufReader, BufWriter};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use pixel_verbo_core::Proveedor;

use crate::wire::{read_frame, write_frame, Request, Response};

/// Daemon ligado a un socket Unix.
pub struct Servidor {
    listener: UnixListener,
    path: PathBuf,
}

impl Servidor {
    /// Bindea el socket Unix en `path`. Si quedó huérfano de una corrida
    /// anterior, se remueve antes.
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        // Modo no-bloqueante para poder revisar la bandera de apagado
        // entre `accept`s sin un canal extra. Un timeout cortito
        // (`set_read_timeout` por conexión) cumpliría lo mismo, pero
        // `non_blocking` + `WouldBlock` da control igual de fino.
        listener.set_nonblocking(true)?;
        Ok(Self { listener, path })
    }

    /// Ruta del socket.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Atiende conexiones hasta que `apagar` se ponga en `true`. Cada
    /// conexión va a su propio `std::thread`; los handles no se joinean
    /// (las conexiones terminan por su cuenta al EOF del cliente).
    pub fn servir<P: Proveedor + 'static>(
        self,
        proveedor: Arc<P>,
        apagar: Arc<AtomicBool>,
    ) -> io::Result<()> {
        loop {
            if apagar.load(Ordering::SeqCst) {
                break;
            }
            match self.listener.accept() {
                Ok((stream, _addr)) => {
                    let prov = proveedor.clone();
                    thread::spawn(move || {
                        // Una conexión muerta no debe tumbar el daemon —
                        // ignoramos el error.
                        let _ = atender(stream, prov);
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl Drop for Servidor {
    fn drop(&mut self) {
        // Sin esto el socket Unix queda como archivo huérfano.
        let _ = std::fs::remove_file(&self.path);
    }
}

fn atender<P: Proveedor>(stream: UnixStream, proveedor: Arc<P>) -> io::Result<()> {
    // Volvemos a bloqueante para los `read_exact`/`write_all`.
    stream.set_nonblocking(false)?;
    let mut lector = BufReader::new(stream.try_clone()?);
    let mut escritor = BufWriter::new(stream);
    while let Some(req) = read_frame::<_, Request>(&mut lector)? {
        let resp = despachar(&*proveedor, req);
        write_frame(&mut escritor, &resp)?;
    }
    Ok(())
}

fn despachar<P: Proveedor>(proveedor: &P, req: Request) -> Response {
    match req {
        Request::ModelId => Response::ModelId(proveedor.model_id().clone()),
        Request::Ping => Response::Pong,
        Request::Aplicar { op, entrada } => match proveedor.aplicar(&op, entrada) {
            Ok(img) => Response::Imagen(img),
            Err(e) => Response::Error(e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClienteBloqueante;
    use pixel_verbo_core::OpPixel;
    use pixel_verbo_mock::ProveedorMock;
    use std::time::Instant;

    fn socket_temp() -> PathBuf {
        let dir = std::env::temp_dir();
        let nonce = format!(
            "pixel-verbo-test-{}-{}.sock",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        );
        dir.join(nonce)
    }

    #[test]
    fn cliente_y_servidor_hablan_ida_y_vuelta() {
        let path = socket_temp();
        let apagar = Arc::new(AtomicBool::new(false));
        let apagar2 = apagar.clone();
        let path_srv = path.clone();
        let handle = thread::spawn(move || {
            let srv = Servidor::bind(&path_srv).unwrap();
            let prov = Arc::new(ProveedorMock::nuevo());
            srv.servir(prov, apagar2).unwrap();
        });

        // Pequeño retry mientras el server aún no bindeó.
        let cliente = (0..20)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(10));
                ClienteBloqueante::conectar(&path).ok()
            })
            .expect("daemon no levantó");

        assert_eq!(cliente.model_id().name, "pixel-verbo-mock-v0");

        let salida = cliente
            .aplicar(
                &OpPixel::Generar {
                    prompt: "test".into(),
                    ancho: 4,
                    alto: 4,
                },
                None,
            )
            .unwrap();
        assert_eq!(salida.ancho, 4);
        assert_eq!(salida.alto, 4);

        apagar.store(true, Ordering::SeqCst);
        handle.join().unwrap();
    }
}
