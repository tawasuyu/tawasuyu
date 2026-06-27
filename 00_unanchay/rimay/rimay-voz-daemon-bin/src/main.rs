//! `voz-daemon` — binario del daemon de voz de la suite.
//!
//! Carga un par STT+TTS en RAM y lo sirve sobre un socket Unix. N procesos lo
//! consumen vía `rimay_voz_daemon::DaemonClient`, que implementa `Transcriptor`
//! y `Locutor` y los consumidores ven como locales. Es el **brazo local** del
//! híbrido de voz (`VozConfig` con `Backend::Local`).
//!
//! ## Uso típico
//!
//! ```text
//!   $ voz-daemon                                # mocks en $XDG_RUNTIME_DIR/voz.sock
//!   $ voz-daemon --socket /tmp/test.sock        # socket alternativo
//!   $ RIMAY_VOZ_STT=local cosmos                # un consumidor pega al daemon
//! ```
//!
//! Por convención de la suite (idéntica a `verbo-daemon`, sólo cambia el
//! nombre): el socket por defecto vive en `$XDG_RUNTIME_DIR/voz.sock`, con
//! fallback a `/tmp/voz-{uid}.sock`.
//!
//! Hoy sólo sirve **mocks** deterministas: los backends reales (whisper para
//! STT, piper para TTS) entran como variantes nuevas del `--stt`/`--tts` cuando
//! aterricen sus crates, sin tocar el protocolo ni el cliente.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use rimay_voz_core::{Locutor, Transcriptor};
use rimay_voz_daemon::Daemon;
use rimay_voz_mock::{LocutorMock, TranscriptorMock};

/// Backend concreto para un lado (STT o TTS). Hoy sólo `mock`; los reales
/// (whisper/piper) se agregan acá cuando estén en disco.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum BackendKind {
    /// Determinista, sin modelo ni descargas. STT reconoce siempre el mismo
    /// texto; TTS sintetiza silencio proporcional. Para desarrollo y CI.
    Mock,
}

#[derive(Debug, Parser)]
#[command(
    name = "voz-daemon",
    about = "Daemon de voz de la suite tawasuyu — sirve un par STT+TTS por socket Unix.",
    version
)]
struct Cli {
    /// Ruta del socket Unix donde escuchar. Si se omite, default por
    /// `$XDG_RUNTIME_DIR/voz.sock`, con fallback a `/tmp/voz-{uid}.sock`.
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Backend de STT (reconocimiento) a servir.
    #[arg(long, value_enum, default_value_t = BackendKind::Mock)]
    stt: BackendKind,

    /// Backend de TTS (síntesis) a servir.
    #[arg(long, value_enum, default_value_t = BackendKind::Mock)]
    tts: BackendKind,
}

/// Convención de la suite para la ruta del socket (gemela de `verbo-daemon`).
fn socket_por_defecto() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join("voz.sock");
    }
    let uid = uid_actual();
    PathBuf::from(format!("/tmp/voz-{uid}.sock"))
}

/// UID vía `/proc/self/loginuid`; 1000 si falla (misma heurística que verbo).
fn uid_actual() -> u32 {
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&u| u != u32::MAX)
        .unwrap_or(1000)
}

fn construir_stt(kind: BackendKind) -> Arc<dyn Transcriptor> {
    match kind {
        BackendKind::Mock => Arc::new(TranscriptorMock::default()),
    }
}

fn construir_tts(kind: BackendKind) -> Arc<dyn Locutor> {
    match kind {
        BackendKind::Mock => Arc::new(LocutorMock),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(socket_por_defecto);
    eprintln!("voz-daemon :: bind {}", socket.display());

    let daemon = Daemon::bind(&socket).context("bindear socket Unix")?;
    let stt = construir_stt(cli.stt);
    let tts = construir_tts(cli.tts);
    eprintln!(
        "voz-daemon :: STT={} · TTS={} — ^C para terminar",
        Transcriptor::modelo(&*stt),
        Locutor::modelo(&*tts),
    );

    daemon
        .serve_with_shutdown(stt, tts, esperar_apagado())
        .await
        .context("loop del daemon")?;
    eprintln!("voz-daemon :: apagado limpio");
    Ok(())
}

/// Resuelve cuando llega SIGINT (^C) o SIGTERM (`systemctl stop`).
async fn esperar_apagado() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("voz-daemon :: SIGTERM no disponible ({e}) — solo SIGINT");
                tokio::signal::ctrl_c().await.ok();
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => eprintln!("voz-daemon :: SIGINT recibido"),
            _ = term.recv() => eprintln!("voz-daemon :: SIGTERM recibido"),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
    }
}
