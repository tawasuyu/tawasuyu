//! `verbo-daemon` — binario del daemon de embeddings de la suite.
//!
//! Carga un `Provider` en RAM y lo sirve sobre un socket Unix. N procesos
//! lo consumen vía `rimay_verbo_daemon::DaemonClient`, que implementa
//! `Provider` y los consumidores ven como local. Un modelo, muchos
//! clientes; coexistencia multi-modelo = un daemon por socket.
//!
//! ## Uso típico
//!
//! ```text
//!   $ verbo-daemon                              # mock 384d en $XDG_RUNTIME_DIR/verbo.sock
//!   $ verbo-daemon --socket /tmp/test.sock      # socket alternativo
//!   $ verbo-daemon --provider mock --dim 768    # otra dimensión
//! ```
//!
//! Por convención de la suite: el socket por defecto vive en
//! `$XDG_RUNTIME_DIR/verbo.sock` (la ubicación canónica para sockets de
//! servicios por-usuario en Linux). Si esa variable no está, cae a
//! `/tmp/verbo-{uid}.sock` — siempre prefijado por UID para no chocar en
//! sistemas multiusuario.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use rimay_verbo_core::Provider;
use rimay_verbo_daemon::Daemon;
use rimay_verbo_fastembed::{FastembedProvider, ENV_ALLOW_DOWNLOAD};
use rimay_verbo_mock::MockProvider;

/// Provider concreto que el daemon expondrá. `mock` para desarrollo
/// determinista, `fastembed` para embeddings semánticos reales en CPU
/// con descarga del modelo en el primer arranque.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProviderKind {
    /// Provider determinista por hash (FNV-1a + LCG). Sin descargas, sin
    /// API keys; útil para desarrollo, tests y CI. Vectores aleatorios
    /// estables: misma cadena → mismo vector siempre.
    Mock,
    /// Provider real vía fastembed (ONNX en CPU). Modelo por defecto:
    /// `multilingual-e5-small` (384d), multilingüe — sirve es/qu/en/otros.
    /// Descarga el modelo a `~/.cache/fastembed` en el primer arranque.
    Fastembed,
}

#[derive(Debug, Parser)]
#[command(
    name = "verbo-daemon",
    about = "Daemon de embeddings de la suite gioser — sirve un Provider por socket Unix.",
    version,
)]
struct Cli {
    /// Ruta del socket Unix donde escuchar. Si se omite, default por
    /// `$XDG_RUNTIME_DIR/verbo.sock`, con fallback a
    /// `/tmp/verbo-{uid}.sock` cuando esa variable no esté.
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Provider concreto a servir.
    #[arg(long, value_enum, default_value_t = ProviderKind::Mock)]
    provider: ProviderKind,

    /// Dimensión del vector — solo aplica al provider `mock`. Los
    /// providers reales fijan su propia dimensión según el modelo
    /// cargado.
    #[arg(long, default_value_t = 384)]
    dim: usize,

    /// Autoriza al provider `fastembed` a descargar el modelo desde
    /// Hugging Face si no estuviera en cache. Equivalente a setear la
    /// env var `RIMAY_VERBO_ALLOW_DOWNLOAD=1`. Sin esto, el provider
    /// `fastembed` aborta con un mensaje explicando cómo habilitarlo.
    #[arg(long)]
    allow_download: bool,
}

fn socket_por_defecto() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join("verbo.sock");
    }
    // Fallback portátil: /tmp prefijado por UID para evitar colisión.
    // `users` no es dep — leemos el UID via libc-free.
    let uid = libc_uid();
    PathBuf::from(format!("/tmp/verbo-{uid}.sock"))
}

/// Lee el UID del proceso sin meter `libc` como dep. Usa la sintaxis
/// `/proc/self/loginuid` como heurística — si falla, devuelve 1000
/// (UID típico del primer usuario). El socket sigue siendo único por
/// usuario en la práctica.
fn libc_uid() -> u32 {
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&u| u != u32::MAX) // -1 cuando no hay sesión
        .unwrap_or(1000)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let socket = cli.socket.unwrap_or_else(socket_por_defecto);
    eprintln!("verbo-daemon :: bind {}", socket.display());

    // Provider configurado por CLI. Cada variante construye su propio
    // `Arc<P>` concreto y lo pasa a `Daemon::serve_with_shutdown`, que es
    // genérico — así no se mezclan `Arc<dyn Provider>` distintos en una
    // sola variable (cada brazo del match queda con su tipo estable).
    let daemon = Daemon::bind(&socket).context("bindear socket Unix")?;
    eprintln!("verbo-daemon :: escuchando — ^C para terminar");
    match cli.provider {
        ProviderKind::Mock => {
            let provider = Arc::new(MockProvider::new(cli.dim));
            eprintln!(
                "verbo-daemon :: provider {} ({}d)",
                provider.model_id().name,
                provider.model_id().dimension
            );
            daemon
                .serve_with_shutdown(provider, esperar_apagado())
                .await
                .context("loop del daemon")?;
        }
        ProviderKind::Fastembed => {
            // El flag CLI funde con la env var: si el operador pasó
            // `--allow-download`, lo equiparamos a setear la env var
            // antes de tocar el provider, así el gate de fastembed la
            // ve y permite la descarga. Si NO se pasó y la var tampoco
            // está, fastembed falla con un mensaje explicativo.
            if cli.allow_download {
                std::env::set_var(ENV_ALLOW_DOWNLOAD, "1");
                eprintln!("verbo-daemon :: descarga del modelo autorizada por --allow-download");
            }
            // La descarga del modelo se hace ANTES de spawn del runtime
            // tokio (estamos dentro de main pero antes del `await` del
            // serve, así que no bloqueamos un workers pool ya activo).
            let provider = Arc::new(
                FastembedProvider::try_default()
                    .context("inicializando fastembed (multilingual-e5-small)")?,
            );
            eprintln!(
                "verbo-daemon :: provider {} ({}d)",
                provider.model_id().name,
                provider.model_id().dimension
            );
            daemon
                .serve_with_shutdown(provider, esperar_apagado())
                .await
                .context("loop del daemon")?;
        }
    }
    eprintln!("verbo-daemon :: apagado limpio");
    Ok(())
}

/// Resuelve cuando llega SIGINT (^C) o SIGTERM (`systemctl stop`). En
/// caso de fallo al instalar el handler de SIGTERM (poco probable en
/// Linux) se cae solo a SIGINT — el daemon sigue apagándose con ^C.
async fn esperar_apagado() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("verbo-daemon :: SIGTERM no disponible ({e}) — solo SIGINT");
                tokio::signal::ctrl_c().await.ok();
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => eprintln!("verbo-daemon :: SIGINT recibido"),
            _ = term.recv() => eprintln!("verbo-daemon :: SIGTERM recibido"),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
    }
}

