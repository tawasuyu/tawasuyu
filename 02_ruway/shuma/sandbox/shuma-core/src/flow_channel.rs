//! Flow channels: data plane sobre Unix socket por edge enriquecido.
//!
//! Cuando un splitter detecta el TypeRef de un edge, además de replicar a
//! los consumers internos del pipeline, se levanta un FlowChannel que
//! expone los bytes a subscribers externos (otros módulos del fractal).
//!
//! ## Diseño
//!
//! - `tokio::sync::broadcast::channel` para fan-out lock-less entre el
//!   splitter (sender) y los N subscribers conectados.
//! - `UnixListener` accept-loop: por cada cliente nuevo, spawn una task
//!   que drena el receiver y escribe al socket.
//! - Subscribers lentos pueden perder mensajes (broadcast::Receiver::Lagged)
//!   — se loguea warn y se sigue. Esto es deliberado para no bloquear el
//!   splitter en consumers lentos.
//!
//! ## Lifetime
//!
//! `FlowChannel` se construye con `new(path)`. Cuando se drop:
//! - El `accept_task` se cancela (vía drop del `tokio::task::JoinHandle`
//!   que tenemos abort-on-drop).
//! - El socket file se borra del FS (`Drop` impl).
//!
//! Sender clones son baratos; los subscribers conectados se enteran del
//! cierre cuando todos los senders se dropean.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tokio::task::AbortHandle;
use tracing::{debug, warn};

/// Capacidad del broadcast channel. Si un subscriber está más de N chunks
/// atrasado, queda `Lagged` y empieza a perder mensajes.
const BROADCAST_CAP: usize = 64;

/// Chunks default del replay buffer. Cuando un cliente nuevo se conecta,
/// recibe hasta estos N chunks antes de iniciar el broadcast live.
/// Override via `FlowChannel::with_replay_cap`.
pub const DEFAULT_REPLAY_CHUNKS: usize = 32;

pub struct FlowChannel {
    sender: broadcast::Sender<Arc<Vec<u8>>>,
    replay: Arc<Mutex<VecDeque<Arc<Vec<u8>>>>>,
    replay_caps: ReplayCaps,
    socket_path: PathBuf,
    meter: Arc<FlowMeter>,
    _accept_handle: AbortOnDrop,
}

/// Contador de bytes y rate (bytes/s ventana 1s).
#[derive(Debug)]
pub struct FlowMeter {
    /// Bytes acumulados desde la creación del FlowChannel.
    total_bytes: std::sync::atomic::AtomicU64,
    /// Ring buffer de (timestamp_ms, bytes_acumulados) para calcular
    /// el rate sobre los últimos N samples.
    rate_window: Mutex<VecDeque<(u64, u64)>>,
}

const RATE_WINDOW_SAMPLES: usize = 32;

impl FlowMeter {
    fn new() -> Self {
        Self {
            total_bytes: std::sync::atomic::AtomicU64::new(0),
            rate_window: Mutex::new(VecDeque::with_capacity(RATE_WINDOW_SAMPLES)),
        }
    }

    fn record(&self, delta: u64) {
        let now = self.total_bytes
            .fetch_add(delta, std::sync::atomic::Ordering::Relaxed)
            + delta;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if let Ok(mut w) = self.rate_window.lock() {
            if w.len() >= RATE_WINDOW_SAMPLES {
                w.pop_front();
            }
            w.push_back((ts, now));
        }
    }

    /// Bytes totales acumulados desde la creación.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Bytes por segundo (rolling sobre la ventana). 0 si no hay
    /// historia suficiente o si el último sample es muy viejo (>5s).
    pub fn bytes_per_sec(&self) -> f64 {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let w = match self.rate_window.lock() {
            Ok(w) => w,
            Err(_) => return 0.0,
        };
        if w.len() < 2 {
            return 0.0;
        }
        let last = w.back().copied().unwrap();
        // Si el último sample tiene >5s, asumimos idle.
        if now_ms.saturating_sub(last.0) > 5000 {
            return 0.0;
        }
        let first = w.front().copied().unwrap();
        let dt_ms = last.0.saturating_sub(first.0).max(1);
        let d_bytes = last.1.saturating_sub(first.1);
        (d_bytes as f64 * 1000.0) / dt_ms as f64
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReplayCaps {
    /// Máximo de chunks retenidos.
    pub chunks: usize,
    /// Máximo de bytes (sumando len de chunks). `0` = sin tope.
    pub bytes: usize,
}

impl ReplayCaps {
    pub fn chunks_only(chunks: usize) -> Self {
        Self { chunks: chunks.max(1), bytes: 0 }
    }
    pub fn new(chunks: usize, bytes: usize) -> Self {
        Self { chunks: chunks.max(1), bytes }
    }
}

#[derive(Clone)]
pub struct FlowSender {
    sender: broadcast::Sender<Arc<Vec<u8>>>,
    replay: Arc<Mutex<VecDeque<Arc<Vec<u8>>>>>,
    replay_caps: ReplayCaps,
    meter: Arc<FlowMeter>,
}

impl FlowSender {
    /// Pushea al broadcast y al replay buffer. Si no hay subscribers,
    /// el broadcast::send retorna Err pero igual guardamos en replay
    /// (subscribers tarde verán los chunks pasados).
    pub fn send(&self, data: Arc<Vec<u8>>) {
        let incoming = data.len();
        let caps = self.replay_caps;
        if let Ok(mut g) = self.replay.lock() {
            evict_for_incoming(&mut g, caps, incoming);
            g.push_back(data.clone());
        }
        self.meter.record(incoming as u64);
        let _ = self.sender.send(data);
    }
}

/// Evict los chunks más viejos para hacer espacio a un chunk entrante de
/// `incoming` bytes — el buffer post-push queda dentro de los caps.
fn evict_for_incoming(buf: &mut VecDeque<Arc<Vec<u8>>>, caps: ReplayCaps, incoming: usize) {
    // 1) chunks: dejar lugar para 1 más.
    while buf.len() + 1 > caps.chunks {
        if buf.pop_front().is_none() {
            break;
        }
    }
    // 2) bytes (si está activado).
    if caps.bytes > 0 {
        let mut current: usize = buf.iter().map(|a| a.len()).sum();
        while current + incoming > caps.bytes {
            match buf.pop_front() {
                Some(c) => current = current.saturating_sub(c.len()),
                None => break,
            }
        }
    }
}

impl std::fmt::Debug for FlowChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowChannel")
            .field("socket_path", &self.socket_path)
            .field("subscribers", &self.sender.receiver_count())
            .finish()
    }
}

impl FlowChannel {
    /// Crea un FlowChannel atado al path `socket_path`. Si el path ya
    /// existe, lo borra antes de bind (asume restart limpio).
    pub fn new(socket_path: PathBuf) -> std::io::Result<Self> {
        Self::with_replay_caps(socket_path, ReplayCaps::chunks_only(DEFAULT_REPLAY_CHUNKS))
    }

    pub fn with_replay_cap(socket_path: PathBuf, chunks: usize) -> std::io::Result<Self> {
        Self::with_replay_caps(socket_path, ReplayCaps::chunks_only(chunks))
    }

    pub fn with_replay_caps(socket_path: PathBuf, caps: ReplayCaps) -> std::io::Result<Self> {
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let listener = UnixListener::bind(&socket_path)?;
        let (tx, _rx_unused) = broadcast::channel::<Arc<Vec<u8>>>(BROADCAST_CAP);
        let replay: Arc<Mutex<VecDeque<Arc<Vec<u8>>>>> =
            Arc::new(Mutex::new(VecDeque::with_capacity(caps.chunks)));
        let tx_for_accept = tx.clone();
        let replay_for_accept = replay.clone();
        let path_for_log = socket_path.clone();

        let join = tokio::spawn(async move {
            debug!(path = %path_for_log.display(), "flow channel listening");
            loop {
                let (mut stream, _addr) = match listener.accept().await {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(?e, "flow channel accept failed");
                        return;
                    }
                };
                // Snapshot del replay buffer Y subscribe al broadcast.
                // El orden es crítico: subscribe ANTES de drenar el replay
                // para no perder chunks que llegan justo en el medio.
                let mut rx = tx_for_accept.subscribe();
                let snapshot: Vec<Arc<Vec<u8>>> = {
                    let g = replay_for_accept.lock().expect("replay lock");
                    g.iter().cloned().collect()
                };
                tokio::spawn(async move {
                    // Fase 1: drenar replay snapshot al subscriber.
                    for chunk in &snapshot {
                        if let Err(e) = stream.write_all(chunk).await {
                            debug!(?e, "flow subscriber dropped during replay");
                            return;
                        }
                    }
                    // Fase 2: live broadcast.
                    loop {
                        match rx.recv().await {
                            Ok(chunk) => {
                                if let Err(e) = stream.write_all(&chunk).await {
                                    debug!(?e, "flow subscriber dropped");
                                    return;
                                }
                            }
                            Err(broadcast::error::RecvError::Closed) => return,
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(skipped = n, "flow subscriber lagged");
                            }
                        }
                    }
                });
            }
        });

        Ok(Self {
            sender: tx,
            replay,
            replay_caps: caps,
            socket_path,
            meter: Arc::new(FlowMeter::new()),
            _accept_handle: AbortOnDrop(join.abort_handle()),
        })
    }

    pub fn meter(&self) -> &FlowMeter {
        &self.meter
    }

    /// Push un chunk al channel. Si no hay subscribers, drop silencioso.
    /// Siempre se guarda en el replay buffer (con cap rotation por chunks
    /// y opcionalmente por bytes).
    pub fn send(&self, data: Vec<u8>) {
        let incoming = data.len();
        let arc = Arc::new(data);
        let caps = self.replay_caps;
        if let Ok(mut g) = self.replay.lock() {
            evict_for_incoming(&mut g, caps, incoming);
            g.push_back(arc.clone());
        }
        self.meter.record(incoming as u64);
        let _ = self.sender.send(arc);
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Handle clone-able para que tasks externas (splitter) pushen al
    /// channel sin tener ownership del FlowChannel. Cada push se guarda
    /// también en el replay buffer y se contabiliza en el meter.
    pub fn sender_handle(&self) -> FlowSender {
        FlowSender {
            sender: self.sender.clone(),
            replay: self.replay.clone(),
            replay_caps: self.replay_caps,
            meter: self.meter.clone(),
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Drop for FlowChannel {
    fn drop(&mut self) {
        // El AbortOnDrop cancela el accept loop; sólo nos queda limpiar el
        // socket file.
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

struct AbortOnDrop(AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Path canónico para un flow channel: `$XDG_RUNTIME_DIR/shuma-flow-<id>.sock`.
pub fn default_flow_socket_path(id: &str) -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        let uid = nix::unistd::getuid().as_raw();
        let p = format!("/run/user/{uid}");
        if std::path::Path::new(&p).exists() {
            p
        } else {
            "/tmp".into()
        }
    });
    PathBuf::from(base).join(format!("shuma-flow-{id}.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::UnixStream;

    #[tokio::test]
    async fn channel_delivers_to_subscriber() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("flow.sock");
        let ch = FlowChannel::new(path.clone()).unwrap();

        // Subscriber se conecta.
        let path_clone = path.clone();
        let task = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&path_clone).await.unwrap();
            let mut buf = vec![0u8; 64];
            let n = stream.read(&mut buf).await.unwrap();
            buf.truncate(n);
            buf
        });

        // Damos tiempo al accept.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // Hasta que haya 1 receiver_count, el send no llega.
        for _ in 0..50 {
            if ch.subscriber_count() >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        ch.send(b"hello-flow".to_vec());

        let received = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("timeout")
            .unwrap();
        assert_eq!(received, b"hello-flow");
    }

    #[tokio::test]
    async fn replay_buffer_serves_late_subscriber() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("flow.sock");
        let ch = FlowChannel::new(path.clone()).unwrap();

        // Pushes ANTES de cualquier subscriber: van solo al replay.
        ch.send(b"chunk-1".to_vec());
        ch.send(b"chunk-2".to_vec());
        ch.send(b"chunk-3".to_vec());

        // Subscriber LATE — debe recibir los 3 chunks del replay.
        let path_clone = path.clone();
        let task = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&path_clone).await.unwrap();
            let mut buf = vec![0u8; 256];
            // Leemos hasta recibir los 3 chunks (21 bytes esperados).
            let mut total = Vec::new();
            for _ in 0..20 {
                let n = stream.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                total.extend_from_slice(&buf[..n]);
                if total.len() >= 21 {
                    break;
                }
            }
            total
        });

        let received = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("timeout")
            .unwrap();
        let s = String::from_utf8_lossy(&received);
        assert!(s.contains("chunk-1"), "got: {s:?}");
        assert!(s.contains("chunk-2"), "got: {s:?}");
        assert!(s.contains("chunk-3"), "got: {s:?}");
    }

    #[tokio::test]
    async fn replay_evicts_by_bytes_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("flow.sock");
        // chunks=100 (no limita), bytes=20: deberíamos retener sólo los
        // últimos chunks cuyos bytes sumen ≤ 20.
        let ch = FlowChannel::with_replay_caps(path.clone(), ReplayCaps::new(100, 20)).unwrap();
        ch.send(b"AAAAAAAA".to_vec()); // 8 bytes
        ch.send(b"BBBBBBBB".to_vec()); // 8 → total 16
        ch.send(b"CCCCCCCC".to_vec()); // 8 → total 24 > 20, evict A → 16
        ch.send(b"DDDDDDDD".to_vec()); // 8 → total 24 > 20, evict B → 16

        let path_clone = path.clone();
        let task = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&path_clone).await.unwrap();
            let mut buf = vec![0u8; 64];
            let mut total = Vec::new();
            for _ in 0..20 {
                let n = stream.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                total.extend_from_slice(&buf[..n]);
                if total.len() >= 16 {
                    break;
                }
            }
            total
        });
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("timeout")
            .unwrap();
        let s = String::from_utf8_lossy(&got);
        // Sólo C y D (los más viejos A y B fueron evicted).
        assert!(!s.contains("AAAA"), "should have evicted A: {s:?}");
        assert!(!s.contains("BBBB"), "should have evicted B: {s:?}");
        assert!(s.contains("CCCC"), "should keep C: {s:?}");
        assert!(s.contains("DDDD"), "should keep D: {s:?}");
    }

    #[tokio::test]
    async fn drop_removes_socket() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("flow.sock");
        {
            let _ch = FlowChannel::new(path.clone()).unwrap();
            assert!(path.exists());
        }
        // Después del drop, el socket file no debe quedar.
        // Damos un pelín de tiempo al runtime para que el drop corra
        // mientras estamos en task.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(!path.exists());
    }
}
