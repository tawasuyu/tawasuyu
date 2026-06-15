//! drop-bridge — puente de "soltar archivos" entre el compositor (**mirada**)
//! y las apps tawasuyu, porque **winit no entrega drag-and-drop en Wayland**
//! (sólo X11/Windows/macOS). El compositor intercepta el drop sobre una
//! ventana tawasuyu, lee las rutas del `text/uri-list` del origen, y se las
//! manda al proceso destino por un socket Unix nombrado por su PID. La app
//! escucha ese socket y abre lo soltado — drag-and-drop "de verdad" sin
//! depender del receptor DnD de winit.
//!
//! Protocolo (mínimo, sin dependencias): socket en
//! `$XDG_RUNTIME_DIR/tawasuyu/drop/<pid>.sock`. El emisor conecta, escribe
//! **una ruta por línea** (UTF-8, `\n`), y cierra. El oyente acepta, lee
//! líneas y llama al callback por cada ruta.

#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

/// Directorio de los sockets de drop (`$XDG_RUNTIME_DIR/tawasuyu/drop`, o el
/// tmpdir si no hay XDG_RUNTIME_DIR).
pub fn socket_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("tawasuyu").join("drop")
}

/// Socket del proceso `pid`.
pub fn socket_path(pid: u32) -> PathBuf {
    socket_dir().join(format!("{pid}.sock"))
}

/// Manda `paths` al proceso `pid` (best-effort). `Err` si no hay oyente.
pub fn send(pid: u32, paths: &[PathBuf]) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(socket_path(pid))?;
    for p in paths {
        stream.write_all(p.to_string_lossy().as_bytes())?;
        stream.write_all(b"\n")?;
    }
    stream.flush()
}

/// Guard del oyente: al droppearlo, remueve el socket del filesystem.
pub struct DropListener {
    path: PathBuf,
}

impl Drop for DropListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Escucha en el socket del PID actual y llama `on_path` por cada ruta
/// recibida (desde un hilo de fondo). Devuelve un guard que limpia el socket
/// al soltarse. Guardalo vivo en el modelo de la app.
pub fn listen<F>(on_path: F) -> std::io::Result<DropListener>
where
    F: Fn(PathBuf) + Send + 'static,
{
    let path = socket_path(std::process::id());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(&path); // socket viejo/colgado
    let listener = UnixListener::bind(&path)?;
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let reader = BufReader::new(conn);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let line = line.trim();
                if !line.is_empty() {
                    on_path(PathBuf::from(line));
                }
            }
        }
    });
    Ok(DropListener { path })
}

/// Parsea un blob `text/uri-list` a rutas locales. Ignora comentarios (`#`)
/// y líneas vacías; acepta `file://[host]/path` (con %XX) y rutas absolutas
/// crudas. Para el lado del compositor, que lee del origen del drag.
pub fn parse_uri_list(blob: &[u8]) -> Vec<PathBuf> {
    let text = String::from_utf8_lossy(blob);
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("file://") {
            // file://host/path — descartamos el host (vacío/localhost).
            let path = match rest.find('/') {
                Some(i) => &rest[i..],
                None => rest,
            };
            out.push(PathBuf::from(percent_decode(path)));
        } else if line.starts_with('/') {
            out.push(PathBuf::from(line));
        }
    }
    out
}

/// Decodifica `%XX` de un path `file://` (mínimo, sin dependencias).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn parsea_uri_list() {
        let blob = b"#comment\r\nfile:///home/u/My%20Video.mp4\r\nfile://localhost/musica/a.mp3\n/abs/cruda.opus\n\n";
        let got = parse_uri_list(blob);
        assert_eq!(
            got,
            vec![
                PathBuf::from("/home/u/My Video.mp4"),
                PathBuf::from("/musica/a.mp3"),
                PathBuf::from("/abs/cruda.opus"),
            ]
        );
    }

    #[test]
    fn send_listen_roundtrip() {
        // XDG aislado para no chocar con sockets reales.
        let tmp = std::env::temp_dir().join(format!("drop-bridge-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", &tmp);

        let (tx, rx) = mpsc::channel();
        let _guard = listen(move |p| {
            let _ = tx.send(p);
        })
        .expect("listen");
        // Pequeña espera a que el accept loop esté listo.
        std::thread::sleep(Duration::from_millis(50));

        let pid = std::process::id();
        send(pid, &[PathBuf::from("/cine/peli.mkv"), PathBuf::from("/cine/otra.webm")]).expect("send");

        let a = rx.recv_timeout(Duration::from_secs(2)).expect("primera ruta");
        let b = rx.recv_timeout(Duration::from_secs(2)).expect("segunda ruta");
        assert_eq!(a, PathBuf::from("/cine/peli.mkv"));
        assert_eq!(b, PathBuf::from("/cine/otra.webm"));

        std::fs::remove_dir_all(&tmp).ok();
    }
}
