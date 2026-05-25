//! Ring buffer en memoria para capturar stdout/stderr de comandos.
//!
//! Tamaño fijo por comando (config: `MAX_LOG_BYTES`). Cuando se llena,
//! descarta los bytes más viejos. Pensado para diagnostico rápido, no
//! para retención histórica — eso es trabajo de un journald-like aparte.

use std::sync::{Arc, Mutex};

/// Bytes máximos retenidos por comando. 64 KiB cubre logs típicos sin
/// abusar de memoria si el daemon tiene cientos de comandos vivos.
pub const MAX_LOG_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct LogBuf {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug)]
struct Inner {
    /// Bytes raw. Cuando se acerca al cap, descartamos head para mantener
    /// el tail.
    buf: Vec<u8>,
    cap: usize,
    /// Total escrito alguna vez (no decrementado al recortar).
    written_total: u64,
}

impl LogBuf {
    pub fn new() -> Self {
        Self::with_cap(MAX_LOG_BYTES)
    }

    pub fn with_cap(cap: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                buf: Vec::with_capacity(cap.min(4096)),
                cap,
                written_total: 0,
            })),
        }
    }

    pub fn append(&self, data: &[u8]) {
        let Ok(mut g) = self.inner.lock() else { return };
        g.written_total += data.len() as u64;
        g.buf.extend_from_slice(data);
        // Recorte cuando excede cap (con un pequeño slack para evitar
        // shift en cada append). El usuario ve sólo el tail.
        if g.buf.len() > g.cap + 1024 {
            let drop = g.buf.len() - g.cap;
            g.buf.drain(..drop);
        }
    }

    /// Devuelve el tail de hasta `n` bytes (o todo si `n=0`).
    pub fn tail(&self, n: usize) -> Vec<u8> {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        if n == 0 || n >= g.buf.len() {
            return g.buf.clone();
        }
        g.buf[g.buf.len() - n..].to_vec()
    }

    /// Cuántos bytes hay actualmente en el buffer.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.buf.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn written_total(&self) -> u64 {
        self.inner.lock().map(|g| g.written_total).unwrap_or(0)
    }
}

impl Default for LogBuf {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_tail_basic() {
        let lb = LogBuf::with_cap(100);
        lb.append(b"hello ");
        lb.append(b"world\n");
        let t = lb.tail(0);
        assert_eq!(t, b"hello world\n");
    }

    #[test]
    fn cap_drops_oldest() {
        let lb = LogBuf::with_cap(10);
        lb.append(&[b'a'; 8]);
        lb.append(&[b'b'; 8]);
        // Después del recorte, debe quedar ~10 bytes pero el slack
        // permite hasta 10+1024. Como pasamos slack, no se recorta aún
        // en este caso (16 bytes < 10+1024). Forzamos un append grande.
        lb.append(&[b'c'; 2048]);
        assert!(lb.len() <= 10 + 1024);
        let t = lb.tail(0);
        // El tail debe contener 'c's (los más recientes).
        assert!(t.iter().filter(|&&b| b == b'c').count() > 0);
    }

    #[test]
    fn written_total_tracks_all() {
        let lb = LogBuf::with_cap(10);
        lb.append(b"abcdef");
        lb.append(b"ghijkl");
        assert_eq!(lb.written_total(), 12);
    }
}
