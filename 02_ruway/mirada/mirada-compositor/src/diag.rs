//! Telemetría de diagnóstico del Cuerpo — el cuaderno de bitácora del
//! compositor mientras se usa de verdad.
//!
//! Tres servicios, todos volcando a un **directorio local persistente**
//! (no `/tmp`, que se borra al reiniciar): por defecto
//! `$XDG_STATE_HOME/mirada` o, si no, `~/.local/state/mirada`.
//!
//! 1. **Bitácora de eventos** (`eventos.log`): un archivo append con
//!    cada error/aviso que el compositor emite vía [`dlog!`]. Es lo que
//!    se revisa "de vez en cuando" para ver qué viene fallando.
//! 2. **Migas** (breadcrumbs): un anillo en memoria con los últimos N
//!    hechos (eventos del Cuerpo, combos, errores). No se escribe en
//!    caliente; se vuelca **sólo** cuando hay un crash, como contexto.
//! 3. **Reporte de crash** (`crash-<n>.log`): un `panic::set_hook` que,
//!    ante un panic, escribe payload + ubicación + backtrace + las migas.
//!    Así un panic deja postmortem en disco aunque tumbe el proceso.
//!
//! La filosofía Brain/Body deja el Body delgado; este módulo es la red
//! que convierte "se cerró sin más" en "se cerró por *esto*, mirá el log".

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Cuántas migas guardamos para el contexto del crash.
const MAX_MIGAS: usize = 256;

/// Anillo de migas — lo lee el panic hook al volcar el reporte.
static MIGAS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// El archivo de bitácora abierto en modo append (None si no se pudo abrir).
static LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();

/// El directorio de diagnóstico, resuelto una sola vez.
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Resuelve el directorio de diagnóstico persistente.
///
/// `$MIRADA_DEBUG_DIR` lo fuerza (útil para tests y para apuntarlo a
/// otro disco). Si no, `$XDG_STATE_HOME/mirada`; si no, `~/.local/state/mirada`.
/// Último recurso: el cwd. **Nunca** `/tmp`.
pub(crate) fn debug_dir() -> &'static PathBuf {
    DIR.get_or_init(|| {
        let base = std::env::var_os("MIRADA_DEBUG_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_STATE_HOME").map(|x| PathBuf::from(x).join("mirada")))
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".local/state/mirada"))
            })
            .unwrap_or_else(|| PathBuf::from("mirada-debug"));
        let _ = std::fs::create_dir_all(&base);
        base
    })
}

/// Segundos desde epoch — sello de tiempo para líneas y nombres de archivo.
/// (Wall-clock; si el reloj no está, cae a 0 y seguimos.)
fn ahora_segundos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Sello legible `HH:MM:SS` derivado del epoch (UTC, sin tocar zonas).
fn sello() -> String {
    let s = ahora_segundos();
    let (h, m, sec) = ((s / 3600) % 24, (s / 60) % 60, s % 60);
    format!("{h:02}:{m:02}:{sec:02}")
}

/// Arranca la telemetría: crea el dir, abre la bitácora e instala el
/// panic hook. Idempotente y best-effort — si algo falla, el compositor
/// arranca igual, sólo sin diagnóstico.
pub(crate) fn init() {
    let dir = debug_dir();
    let log_path = dir.join("eventos.log");
    let archivo = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok()
        .map(Mutex::new);
    let _ = LOG.set(archivo);

    log(format_args!(
        "── arranque · diagnóstico en {} ──",
        dir.display()
    ));

    instalar_panic_hook();
}

/// Escribe una línea con sello de tiempo a la bitácora **y** a stderr, y
/// la deja como miga para el contexto de un eventual crash. Usar vía
/// [`dlog!`]; este `fn` es el sumidero.
pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    let linea = format!("[{}] {args}", sello());
    eprintln!("{linea}");
    miga(linea.clone());
    if let Some(Some(m)) = LOG.get() {
        if let Ok(mut f) = m.lock() {
            let _ = writeln!(f, "{linea}");
            let _ = f.flush();
        }
    }
}

/// Agrega una miga al anillo (descarta la más vieja al pasar `MAX_MIGAS`).
/// Barato: no toca disco. Útil para hechos de alta frecuencia (eventos del
/// Cuerpo) que no queremos en la bitácora pero sí en el reporte de crash.
pub(crate) fn miga(s: impl Into<String>) {
    if let Ok(mut q) = MIGAS.lock() {
        if q.len() == MAX_MIGAS {
            q.pop_front();
        }
        q.push_back(format!("[{}] {}", sello(), s.into()));
    }
}

/// Vuelca las migas actuales como bloque de texto (para el reporte).
fn migas_volcado() -> String {
    MIGAS
        .lock()
        .map(|q| q.iter().cloned().collect::<Vec<_>>().join("\n"))
        .unwrap_or_else(|_| "<migas no disponibles: lock envenenado>".into())
}

/// Instala el panic hook que deja postmortem en disco. Encadena el hook
/// previo (que sigue imprimiendo a stderr) para no perder ese comportamiento.
fn instalar_panic_hook() {
    let anterior = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // El reporte primero, por si el hook anterior aborta.
        escribir_reporte_crash(info);
        anterior(info);
    }));
}

/// Compone y escribe `crash-<epoch>.log` con todo el contexto disponible.
/// Best-effort y a prueba de re-panic: cualquier error de IO se traga.
fn escribir_reporte_crash(info: &std::panic::PanicHookInfo<'_>) {
    let dir = debug_dir();
    let path = dir.join(format!("crash-{}.log", ahora_segundos()));

    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<payload no textual>".into());
    let ubicacion = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<ubicación desconocida>".into());
    let hilo = std::thread::current()
        .name()
        .unwrap_or("<sin nombre>")
        .to_string();
    let backtrace = std::backtrace::Backtrace::force_capture();

    let reporte = format!(
        "═══════════════════════════════════════════════════════\n\
         mirada-compositor · PANIC\n\
         epoch:     {}\n\
         hilo:      {hilo}\n\
         ubicación: {ubicacion}\n\
         mensaje:   {payload}\n\
         ───────────────────────── migas (últimos hechos) ─────────────────────────\n\
         {}\n\
         ───────────────────────── backtrace ─────────────────────────\n\
         {backtrace}\n\
         ═══════════════════════════════════════════════════════\n",
        ahora_segundos(),
        migas_volcado(),
    );

    if let Ok(mut f) = File::create(&path) {
        let _ = f.write_all(reporte.as_bytes());
        let _ = f.flush();
    }
    // También a la bitácora, para que el resumen aparezca en el archivo que
    // se revisa de rutina (sin el backtrace completo, que ya está en crash-*).
    if let Some(Some(m)) = LOG.get() {
        if let Ok(mut f) = m.lock() {
            let _ = writeln!(
                f,
                "[{}] PANIC {ubicacion} — {payload} (reporte: {})",
                sello(),
                path.display()
            );
            let _ = f.flush();
        }
    }
    eprintln!(
        "mirada-compositor · PANIC capturado — reporte en {}",
        path.display()
    );
}

/// Corre `f` aislada: si paniquea, lo registra (vía el panic hook) y
/// devuelve `false` en vez de propagar el unwind. Pensada para envolver
/// el dispatch de clientes en el bucle — un cliente buggy no debe tumbar
/// la sesión entera.
///
/// `AssertUnwindSafe`: tras un panic a mitad de dispatch el estado podría
/// quedar algo inconsistente, pero para un escritorio "seguir vivo" gana a
/// "morir limpio". El reporte queda en disco para diagnosticar la causa.
pub(crate) fn aislar<F: FnOnce()>(etiqueta: &str, f: F) -> bool {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => true,
        Err(_) => {
            log(format_args!(
                "recuperado de un panic en «{etiqueta}» — sesión sigue viva (ver crash-*.log)"
            ));
            false
        }
    }
}

/// Bitácora con sello de tiempo a disco + stderr + miga. Mismo uso que
/// `eprintln!`. Es el canal canónico de errores/avisos del compositor.
macro_rules! dlog {
    ($($arg:tt)*) => {
        $crate::diag::log(format_args!($($arg)*))
    };
}
