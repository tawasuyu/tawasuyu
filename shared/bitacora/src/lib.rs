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
//! 1. **Captura `stderr` en un archivo** bajo `$XDG_STATE_HOME/tawasuyu/<dominio>/<binario>.log`
//!    (fallback `~/.local/state/...`, y `/tmp/tawasuyu/...` si no hay HOME). Con esto quedan
//!    capturados los `eprintln!`, la salida de librerías y cualquier cosa escrita a fd 2.
//!    `stdout` se deja **intacto** para no romper CLIs ni pipes.
//! 2. **Instala un panic hook** que escribe el panic (con ubicación) a esa misma bitácora,
//!    encadenando el hook previo para no alterar el comportamiento de aborto.
//! 3. **Instala un subscriber de `tracing`** (vía `try_init`, no pisa el que el binario ya
//!    tenga). Como `stderr` ya apunta a la bitácora, los eventos `tracing::` también caen ahí —
//!    igual que los de los binarios que inicializan su propio `fmt().init()`.
//!
//! ## Dos modos de captura
//!
//! - **`tee`**: stderr va al archivo **y** a la terminal original (no perdés nada en consola).
//!   Implementado con un pipe + un thread lector que reparte cada chunk a ambos lados.
//! - **`file`**: stderr se redirige sólo al archivo (consola muda). Más liviano; ideal para
//!   daemons/sesiones donde la consola no va a ningún lado.
//!
//! Por defecto se elige según el contexto: **`tee` si stderr es una TTY** (corrida interactiva,
//! querés ver y loguear) y **`file` si no lo es** (daemon, su stderr no se mira). Override por
//! entorno con `BITACORA`:
//!
//! | `BITACORA` | efecto |
//! |---|---|
//! | `0` / `off` / `false` | no-op total: consola normal, sin captura |
//! | `tee` | fuerza tee aunque no sea TTY |
//! | `file` | fuerza redirect mudo aunque sea TTY |
//! | (sin setear) | `tee` si stderr es TTY, `file` si no |
//!
//! Rotación simple: si la bitácora supera ~8 MiB se renombra a `<binario>.log.1` (un backup).

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Tamaño a partir del cual rotamos la bitácora (8 MiB).
const LIMITE_ROTACION: u64 = 8 * 1024 * 1024;

/// Modo de captura de stderr, resuelto en cada arranque.
enum Modo {
    /// stderr → archivo + terminal original (vía pipe + thread).
    Tee,
    /// stderr → sólo archivo (consola muda).
    File,
}

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

    match modo_captura() {
        Modo::File => redirigir_stderr(archivo),
        Modo::Tee => instalar_tee(archivo),
    }

    instalar_panic_hook(ruta);
    instalar_tracing();
}

/// Resuelve el modo de captura: `BITACORA=tee|file` lo fuerza; sin setear,
/// `tee` si stderr es una TTY (interactivo) y `file` si no (daemon/pipe).
fn modo_captura() -> Modo {
    match std::env::var("BITACORA").ok().as_deref() {
        Some("tee") => Modo::Tee,
        Some("file") => Modo::File,
        _ => {
            let es_tty = unsafe { libc::isatty(libc::STDERR_FILENO) == 1 };
            if es_tty {
                Modo::Tee
            } else {
                Modo::File
            }
        }
    }
}

/// Redirige stderr (fd 2) sólo al archivo. dup2 deja fd 2 apuntando a la misma
/// descripción abierta; el descriptor original puede cerrarse sin afectar la redirección.
fn redirigir_stderr(archivo: File) {
    unsafe {
        libc::dup2(archivo.as_raw_fd(), libc::STDERR_FILENO);
    }
    drop(archivo);
}

/// Modo tee: stderr → archivo **y** terminal original. Guarda el fd real de la
/// terminal (`dup`), interpone un pipe en fd 2, y lanza un thread que lee del pipe
/// y reparte cada chunk al archivo y a la terminal. Best-effort: ante cualquier
/// fallo de setup cae a [`redirigir_stderr`] (sólo archivo).
fn instalar_tee(archivo: File) {
    unsafe {
        // fd real de la terminal (a donde apuntaba stderr antes de tocar nada).
        let terminal_fd = libc::dup(libc::STDERR_FILENO);
        if terminal_fd < 0 {
            return redirigir_stderr(archivo);
        }
        let mut fds = [0i32; 2];
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            libc::close(terminal_fd);
            return redirigir_stderr(archivo);
        }
        let (lectura, escritura) = (fds[0], fds[1]);
        // stderr ahora escribe al extremo de escritura del pipe.
        libc::dup2(escritura, libc::STDERR_FILENO);
        libc::close(escritura);

        // Thread repartidor: pipe → archivo + terminal. Vive lo que vive el proceso
        // (fd 2 mantiene abierto el extremo de escritura, así que nunca llega EOF
        // hasta que el proceso termina y el SO cierra todo).
        let mut pipe = File::from_raw_fd(lectura);
        let mut terminal = File::from_raw_fd(terminal_fd);
        let mut archivo = archivo;
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = archivo.write_all(&buf[..n]);
                        let _ = archivo.flush();
                        let _ = terminal.write_all(&buf[..n]);
                        let _ = terminal.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
    }
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

/// Panic hook que garantiza el postmortem en disco y encadena el previo.
///
/// Escribe el panic **directamente** al archivo (sincrónico) — así queda aunque
/// el modo `tee` no alcance a drenar su pipe antes del aborto. Luego invoca el
/// hook previo: el de por defecto vuelca el panic estándar a stderr, que en
/// `tee` la terminal ve y en `file` cae en la propia bitácora.
fn instalar_panic_hook(ruta: PathBuf) {
    let previo = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let ubic = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "?".into());
        let pid = std::process::id();
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&ruta) {
            let _ = writeln!(f, "\n!!!! PANIC pid={pid} en {ubic}: {info}");
            let _ = f.flush();
        }
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
