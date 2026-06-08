//! `pixel-verbo-daemon` — binario que sirve un proveedor de píxeles a
//! N procesos por socket Unix.
//!
//! Es el espejo de `rimay-verbo-daemon-bin` aplicado a píxeles. Carga un
//! `Proveedor` en RAM y atiende clientes (`ClienteBloqueante`) que lo
//! consumen como local. Un modelo, muchos clientes; coexistencia
//! multi-modelo = un daemon por socket.
//!
//! ## Uso
//!
//! ```text
//!   $ pixel-verbo-daemon                              # mock en $XDG_RUNTIME_DIR/pixel-verbo.sock
//!   $ pixel-verbo-daemon --socket /tmp/test.sock      # socket alternativo
//!   $ pixel-verbo-daemon --provider mock              # hoy es la única opción
//! ```
//!
//! El socket por defecto vive en `$XDG_RUNTIME_DIR/pixel-verbo.sock`. Si
//! la variable no está, cae a `/tmp/pixel-verbo-{uid}.sock` — siempre
//! prefijado por UID para no chocar en sistemas multiusuario. (Mismo
//! pattern que rimay-verbo-daemon-bin.)

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use pixel_verbo_core::Proveedor;
use pixel_verbo_daemon::Servidor;
use pixel_verbo_mock::ProveedorMock;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProveedorKind {
    /// Determinista, sin descargas ni pesos. Útil para desarrollo,
    /// tests y CI. Misma op + mismo prompt → mismo output siempre.
    Mock,
}

#[derive(Debug, Parser)]
#[command(
    name = "pixel-verbo-daemon",
    about = "Daemon de modelos de píxel de la suite tawasuyu — sirve un Proveedor por socket Unix.",
    version
)]
struct Cli {
    /// Ruta del socket Unix donde escuchar. Default: $XDG_RUNTIME_DIR/pixel-verbo.sock
    /// con fallback a /tmp/pixel-verbo-{uid}.sock.
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Proveedor a servir.
    #[arg(long, value_enum, default_value_t = ProveedorKind::Mock)]
    provider: ProveedorKind,
}

fn socket_por_defecto() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join("pixel-verbo.sock");
    }
    let uid = leer_uid();
    PathBuf::from(format!("/tmp/pixel-verbo-{uid}.sock"))
}

fn leer_uid() -> u32 {
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&u| u != u32::MAX)
        .unwrap_or(1000)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(socket_por_defecto);
    eprintln!("pixel-verbo-daemon :: bind {}", socket.display());

    let servidor = Servidor::bind(&socket).context("bindear socket Unix")?;
    // El daemon termina por SIGINT del shell (^C); el bool queda para
    // pruebas y para una integración futura con `signal-hook` sin tocar
    // la API del `Servidor::servir`.
    let apagar = Arc::new(AtomicBool::new(false));

    eprintln!(
        "pixel-verbo-daemon :: escuchando — ^C para terminar (path {})",
        servidor.path().display()
    );

    match cli.provider {
        ProveedorKind::Mock => {
            let proveedor = Arc::new(ProveedorMock::nuevo());
            eprintln!(
                "pixel-verbo-daemon :: proveedor {}",
                proveedor.model_id()
            );
            servidor
                .servir(proveedor, apagar)
                .context("loop del servidor")?;
        }
    }
    eprintln!("pixel-verbo-daemon :: apagado limpio");
    Ok(())
}
