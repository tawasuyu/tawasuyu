//! Contrato de handoff de Fase 2 (lado splash). Ver
//! `SDD-ARRANQUE-SIN-PARPADEO.md` §Fase 2.
//!
//! El splash escucha en un socket Unix conocido. Cuando mirada está por tomar
//! la pantalla, conecta y manda `READY`. El splash lo detecta (polling no
//! bloqueante desde su bucle de animación), hace el fade-out, **suelta el DRM
//! master** y recién entonces responde `RELEASED` — la secuencia importa: mirada
//! espera ese `RELEASED` antes de tomar master, así no pelean por el device.
//!
//! Best-effort: si el socket no se puede crear (FS de solo lectura, permisos),
//! no hay handoff y el splash cae a su tope de tiempo (Fase 1). Si no aparece
//! ningún cliente, lo mismo.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// Ruta por defecto del socket. Override por `ARJE_SPLASH_SOCK`. mirada usa la
/// misma convención (ver `mirada-compositor::handoff`).
pub const DEFAULT_SOCK: &str = "/run/arje-splash.sock";
/// Mensaje del cliente (mirada) → «estoy por tomar la pantalla».
pub const MSG_READY: &str = "READY";
/// Respuesta del splash → «solté el DRM master, es tuyo».
pub const MSG_RELEASED: &str = "RELEASED";

/// Resuelve la ruta del socket (env o default).
pub fn sock_path() -> PathBuf {
    std::env::var_os("ARJE_SPLASH_SOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCK))
}

/// Estado del handoff: el listener y, una vez que alguien conecta, su stream.
pub struct Handoff {
    path: PathBuf,
    listener: Option<UnixListener>,
    client: Option<UnixStream>,
    buf: Vec<u8>,
}

impl Handoff {
    /// Crea el socket en `path` (best-effort, no bloqueante). Si el bind falla,
    /// `active()` será `false` y el splash sigue sin handoff.
    pub fn bind(path: &Path) -> Self {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Un socket viejo (de un arranque previo sin limpiar) impide el bind.
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).ok();
        if let Some(l) = &listener {
            let _ = l.set_nonblocking(true);
        }
        Handoff {
            path: path.to_path_buf(),
            listener,
            client: None,
            buf: Vec::new(),
        }
    }

    /// ¿El socket quedó escuchando?
    pub fn active(&self) -> bool {
        self.listener.is_some()
    }

    /// Polling no bloqueante: acepta un cliente si aún no hay, lee lo que mandó
    /// y devuelve `true` en cuanto llegó `READY`. Idempotente una vez disparado.
    pub fn poll_ready(&mut self) -> bool {
        if self.client.is_none() {
            if let Some(l) = &self.listener {
                if let Ok((stream, _)) = l.accept() {
                    let _ = stream.set_nonblocking(true);
                    self.client = Some(stream);
                }
            }
        }
        if let Some(c) = &mut self.client {
            let mut tmp = [0u8; 64];
            // WouldBlock / 0 bytes → todavía nada; cualquier error real lo
            // tratamos como «sin novedad» y reintentamos el próximo frame.
            if let Ok(n) = c.read(&mut tmp) {
                if n > 0 {
                    self.buf.extend_from_slice(&tmp[..n]);
                }
            }
            if let Ok(s) = std::str::from_utf8(&self.buf) {
                return s.contains(MSG_READY);
            }
        }
        false
    }

    /// Avisa al cliente que ya soltamos el DRM master (manda `RELEASED`).
    /// Llamar **después** de soltar el master.
    pub fn send_released(&mut self) {
        if let Some(c) = &mut self.client {
            let _ = c.write_all(format!("{MSG_RELEASED}\n").as_bytes());
            let _ = c.flush();
        }
    }
}

impl Drop for Handoff {
    fn drop(&mut self) {
        // Sacamos el socket del filesystem para no dejar basura entre arranques.
        if self.listener.is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Cliente de prueba (`arje-splash --poke`): conecta al socket, manda `READY` y
/// espera `RELEASED`. Simula a mirada para verificar el handoff end-to-end en
/// QEMU sin levantar el compositor entero. Reintenta el connect un rato porque
/// el servidor (el splash) puede tardar en bindear.
pub fn poke(path: &Path) -> std::io::Result<()> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match UnixStream::connect(path) {
            Ok(s) => break s,
            Err(e) if Instant::now() < deadline => {
                eprintln!("[arje-splash --poke] esperando el socket ({e})…");
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(e),
        }
    };
    eprintln!("[arje-splash --poke] conectado; mando {MSG_READY}");
    stream.write_all(format!("{MSG_READY}\n").as_bytes())?;
    stream.flush()?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    // Leemos un chunk (no hasta EOF): el servidor responde RELEASED y recién
    // después cierra al salir; con timeout no nos colgamos si demora.
    let mut tmp = [0u8; 64];
    let n = stream.read(&mut tmp)?;
    eprintln!(
        "[arje-splash --poke] respuesta: {}",
        String::from_utf8_lossy(&tmp[..n]).trim()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::Duration;

    #[test]
    fn protocolo_ready_release_completo() {
        // Socket en un tmp único (sin colisión entre tests paralelos).
        let dir = std::env::temp_dir().join(format!("arje-splash-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("h.sock");

        let mut h = Handoff::bind(&sock);
        assert!(h.active(), "el bind debe dejar el listener escuchando");

        // Sin cliente todavía: poll_ready no dispara.
        assert!(!h.poll_ready());

        // Conecta un cliente y manda READY.
        let mut client = UnixStream::connect(&sock).unwrap();
        client.write_all(b"READY\n").unwrap();
        client.flush().unwrap();

        // El servidor lo detecta (con unos reintentos por la asincronía).
        let mut seen = false;
        for _ in 0..50 {
            if h.poll_ready() {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(seen, "el splash debe ver READY");

        // El splash responde RELEASED (tras soltar el master, acá simulado).
        h.send_released();
        client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut tmp = [0u8; 64];
        let n = client.read(&mut tmp).unwrap();
        let resp = String::from_utf8_lossy(&tmp[..n]);
        assert!(resp.contains("RELEASED"), "el cliente debe recibir RELEASED, vino: {resp:?}");

        // Al soltar el Handoff, el socket se borra del FS.
        drop(h);
        assert!(!sock.exists(), "el socket debe limpiarse en Drop");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bind_en_ruta_invalida_no_panica_y_queda_inactivo() {
        let h = Handoff::bind(Path::new("/proc/no-se-puede/x.sock"));
        assert!(!h.active(), "un bind imposible deja el handoff inactivo, sin pánico");
    }
}
