//! Evidencia del caché de shaping: simula N redraws de una UI con texto
//! mayormente estable (chrome + un párrafo) más una línea que cambia cada
//! frame (un contador/caret tipeando). Reporta tiempo total y hit-rate con el
//! caché vivo vs. el costo de re-shapear siempre (clave única por frame).
//!
//!   cargo run -p llimphi-text --example bench_cache --release

use llimphi_text::{Alignment, Typesetter};
use std::time::Instant;

const FRAMES: usize = 600; // ~10 s a 60 fps

// Un bloque de chrome típico: labels que NO cambian entre frames.
const CHROME: &[&str] = &[
    "Archivo", "Editar", "Ver", "Insertar", "Formato", "Herramientas", "Ayuda",
    "Guardar", "Abrir", "Nuevo", "Buscar", "Reemplazar", "Deshacer", "Rehacer",
];
const PARRAFO: &str = "Un documento es un haz de cuerpos sobre el mismo material, \
    alineados párrafo a párrafo por sus hebras; si la madre cambia, la hija queda stale.";

fn pintar_frame(ts: &mut Typesetter, frame: usize, estatico: bool) {
    // Chrome estable + párrafo estable: misma clave cada frame ⇒ hit con caché.
    for label in CHROME {
        let _ = ts.layout(label, 13.0, None, Alignment::Start, 1.2, false, None, 400.0);
    }
    let _ = ts.layout(PARRAFO, 15.0, Some(420.0), Alignment::Start, 1.4, false, None, 400.0);
    // Una línea que cambia cada frame (caret/contador): siempre miss.
    // Con `estatico=true` la forzamos constante para ver el techo del caché.
    let dinamico = if estatico {
        "estado: listo".to_string()
    } else {
        format!("línea {frame} · col {}", frame % 80)
    };
    let _ = ts.layout(&dinamico, 13.0, None, Alignment::Start, 1.2, false, None, 400.0);
}

fn corrida(nombre: &str, estatico: bool) {
    let mut ts = Typesetter::new();
    // Warmup: primera pasada llena el caché (no la medimos).
    pintar_frame(&mut ts, 0, estatico);
    let base = ts.cache_stats();
    let t0 = Instant::now();
    for f in 1..=FRAMES {
        pintar_frame(&mut ts, f, estatico);
    }
    let dt = t0.elapsed();
    let s = ts.cache_stats();
    let hits = s.hits - base.hits;
    let misses = s.misses - base.misses;
    let total = hits + misses;
    println!(
        "{nombre:<28} {FRAMES} frames en {:>7.2?}  ({:>6.1} µs/frame)  hit-rate {:.1}% ({hits}/{total})  entradas vivas {}",
        dt,
        dt.as_micros() as f64 / FRAMES as f64,
        100.0 * hits as f64 / total as f64,
        s.entries,
    );
}

/// Baseline sin caché para la MISMA carga: cada texto se hace único por frame
/// (sufijo invisible) ⇒ 100% miss ⇒ shaping completo siempre. Es el costo que
/// el caché evita en el chrome+párrafo estables.
fn corrida_sin_cache() {
    let mut ts = Typesetter::new();
    let frame_texts = |f: usize| -> Vec<String> {
        let mut v: Vec<String> = CHROME.iter().map(|l| format!("{l}\u{200b}{f}")).collect();
        v.push(format!("{PARRAFO}\u{200b}{f}"));
        v.push(format!("línea {f} · col {}", f % 80));
        v
    };
    let _ = frame_texts(0); // simetría con el warmup de `corrida`
    let t0 = Instant::now();
    for f in 1..=FRAMES {
        for t in frame_texts(f) {
            let _ = ts.layout(&t, 13.0, Some(420.0), Alignment::Start, 1.2, false, None, 400.0);
        }
    }
    let dt = t0.elapsed();
    println!(
        "{:<28} {FRAMES} frames en {:>7.2?}  ({:>6.1} µs/frame)  hit-rate   0.0% (todo re-shapeado)",
        "Sin caché (baseline)",
        dt,
        dt.as_micros() as f64 / FRAMES as f64,
    );
}

fn main() {
    println!("Caché de shaping de llimphi-text — {FRAMES} frames\n");
    // Baseline: la misma carga, re-shapeando todo cada frame.
    corrida_sin_cache();
    // Caso real: chrome+párrafo estable, 1 línea cambiante por frame.
    corrida("UI típica (1 línea cambia)", false);
    // Techo: todo estable (lo que pasa en idle/hover sin cambio de texto).
    corrida("Todo estable (techo)", true);
}
