//! Bitácora — telemetría local para la etapa de desarrollo.
//!
//! Una sola llamada al arrancar un binario:
//!
//! ```no_run
//! fn main() {
//!     bitacora::abrir("mirada");
//!     // … resto del programa
//! }
//! ```
//!
//! Hace tres cosas, todas best-effort (jamás aborta el programa si algo falla):
//!
//! 1. **Redirige `stderr` a un archivo** bajo `$XDG_STATE_HOME/tawasuyu/<dominio>/<binario>.log`
//!    (fallback `~/.local/state/...`, y `/tmp/tawasuyu/...` si no hay HOME). Con esto quedan
//!    capturados los `eprintln!`, la salida de librerías y cualquier cosa escrita a fd 2.
//!    `stdout` se deja **intacto** para no romper CLIs ni pipes.
//! 2. **Instala un panic hook** que escribe el panic (con ubicación) a esa misma bitácora,
//!    encadenando el hook previo para no alterar el comportamiento de aborto.
//! 3. **Instala un subscriber de `tracing`** (vía `try_init`, no pisa el que el binario ya
//!    tenga). Como `stderr` ya apunta al archivo, los eventos `tracing::` también caen ahí —
//!    igual que los de los binarios que inicializan su propio `fmt().init()`.
//!
//! Escotilla de escape: con `BITACORA=0` en el entorno, `abrir` es un no-op total (consola
//! normal, sin redirección) — útil para correr un binario a mano y ver todo en la terminal.
//!
//! Rotación simple: si la bitácora supera ~8 MiB se renombra a `<binario>.log.1` (un backup).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Tamaño a partir del cual rotamos la bitácora (8 MiB).
const LIMITE_ROTACION: u64 = 8 * 1024 * 1024;

/// Abre la bitácora del binario actual bajo el `dominio` dado.
///
/// `dominio` es la carpeta de agrupación (p. ej. `"mirada"`, `"pata"`, `"shuma"`, `"arje"`);
/// el nombre del archivo se deriva del binario en ejecución. Llamar lo antes posible en `main`.
pub fn abrir(dominio: &str) {
    if desactivada() {
        return;
    }
    let Some(ruta) = ruta_bitacora(dominio) else {
        return;
    };
    if let Some(dir) = ruta.parent() {
        let _ = fs::create_dir_all(dir);
    }
    rotar_si_grande(&ruta);

    let archivo = match OpenOptions::new().create(true).append(true).open(&ruta) {
        Ok(f) => f,
        Err(_) => return,
    };

    escribir_encabezado(&archivo, &ruta);

    // Redirigir stderr (fd 2) al archivo. dup2 deja fd 2 apuntando a la misma descripción
    // de archivo abierta; el descriptor original puede cerrarse sin afectar la redirección.
    unsafe {
        libc::dup2(archivo.as_raw_fd(), libc::STDERR_FILENO);
    }
    drop(archivo);

    instalar_panic_hook(ruta);
    instalar_tracing();
}

/// `true` si `BITACORA=0` (o `BITACORA=off`) — la telemetría queda deshabilitada.
fn desactivada() -> bool {
    matches!(
        std::env::var("BITACORA").ok().as_deref(),
        Some("0") | Some("off") | Some("false")
    )
}

/// Calcula `$XDG_STATE_HOME/tawasuyu/<dominio>/<binario>.log` con sus fallbacks.
fn ruta_bitacora(dominio: &str) -> Option<PathBuf> {
    let base = directorio_estado()?;
    Some(base.join("tawasuyu").join(dominio).join(format!("{}.log", nombre_binario())))
}

/// Raíz de estado: `$XDG_STATE_HOME`, luego `~/.local/state`, luego `/tmp`.
fn directorio_estado() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home).join(".local").join("state"));
        }
    }
    Some(PathBuf::from("/tmp"))
}

/// Nombre del binario en ejecución (sin ruta). Fallbacks razonables si no se puede leer.
fn nombre_binario() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "proceso".to_string())
}

/// Renombra la bitácora a `.log.1` si supera el límite (un solo backup).
fn rotar_si_grande(ruta: &PathBuf) {
    if let Ok(meta) = fs::metadata(ruta) {
        if meta.len() > LIMITE_ROTACION {
            let backup = ruta.with_extension("log.1");
            let _ = fs::rename(ruta, backup);
        }
    }
}

/// Escribe una línea de sesión al abrir: época, pid, ejecutable y argumentos.
fn escribir_encabezado(mut archivo: &std::fs::File, _ruta: &PathBuf) {
    let epoca = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let pid = std::process::id();
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".into());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _ = writeln!(
        archivo,
        "\n──── bitácora abierta · epoch={epoca} pid={pid} exe={exe} args={:?} ────",
        args
    );
}

/// Panic hook que registra el panic en la bitácora (stderr ya redirigido) y encadena el previo.
fn instalar_panic_hook(_ruta: PathBuf) {
    let previo = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let ubic = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "?".into());
        // stderr está redirigido a la bitácora, así que esto cae en el archivo.
        eprintln!("\n!!!! PANIC en {ubic}: {info}");
        previo(info);
    }));
}

/// Subscriber de tracing best-effort. `try_init` no pisa al que el binario ya haya puesto;
/// y como stderr apunta al archivo, los eventos caen en la bitácora de todos modos.
fn instalar_tracing() {
    use tracing_subscriber::EnvFilter;
    let filtro = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filtro)
        .with_target(true)
        .try_init();
}
