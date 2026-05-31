//! `host` — módulos *vivos* del launcher (reloj, cpu, ram, volumen).
//!
//! `launcher-llimphi` es un renderer sin estado: los `kind` builtin
//! (`launch`/`dock`/`app_menu`/`spacer`) los pinta él, pero los módulos
//! dinámicos (reloj, medidores) los delega al host vía
//! [`crate::LauncherSpec::render_module`]. Este módulo es la
//! implementación de referencia de ese hook para el host Linux: lee la
//! hora del sistema y `/proc` para CPU/RAM, sin dependencias nativas.
//!
//! El host arma un [`SysStats`] (lo refresca con un tick periódico) y
//! pasa `move |m| host::module_view(m, &stats, &theme)` como
//! `render_module`. Así el reloj marca la hora real y los medidores la
//! carga real, en vez de los chips estáticos del demo.

use launcher_core::Module;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, AlignItems, Size, Style};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Una muestra cruda de `/proc/stat` (línea agregada `cpu`). El % de CPU
/// es un *delta* entre dos muestras, así que el host guarda la anterior.
#[derive(Clone, Copy, Debug)]
pub struct CpuSample {
    pub idle: u64,
    pub total: u64,
}

/// Lee la línea agregada `cpu` de `/proc/stat`. `None` si no está
/// disponible (otro SO, sandbox sin `/proc`, formato inesperado).
pub fn read_cpu_sample() -> Option<CpuSample> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let line = stat.lines().next()?;
    let mut it = line.split_whitespace();
    if it.next()? != "cpu" {
        return None;
    }
    let vals: Vec<u64> = it.filter_map(|x| x.parse().ok()).collect();
    if vals.len() < 4 {
        return None;
    }
    // idle = idle + iowait (índices 3 y 4 si existe).
    let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
    let total: u64 = vals.iter().sum();
    Some(CpuSample { idle, total })
}

/// % de CPU ocupado entre dos muestras. `None` si el delta es nulo o la
/// muestra retrocedió (contadores reiniciados).
pub fn cpu_pct(prev: &CpuSample, cur: &CpuSample) -> Option<f32> {
    let dt = cur.total.checked_sub(prev.total)?;
    let di = cur.idle.checked_sub(prev.idle)?;
    if dt == 0 {
        return None;
    }
    Some((1.0 - di as f32 / dt as f32) * 100.0)
}

/// % de RAM en uso, vía `/proc/meminfo` (`MemTotal` − `MemAvailable`).
pub fn mem_pct() -> Option<f32> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total: Option<f64> = None;
    let mut avail: Option<f64> = None;
    let field = |v: &str| -> Option<f64> {
        v.split_whitespace().next().and_then(|n| n.parse::<f64>().ok())
    };
    for l in info.lines() {
        if let Some(v) = l.strip_prefix("MemTotal:") {
            total = field(v);
        } else if let Some(v) = l.strip_prefix("MemAvailable:") {
            avail = field(v);
        }
    }
    let (t, a) = (total?, avail?);
    if t == 0.0 {
        return None;
    }
    Some(((t - a) / t * 100.0) as f32)
}

/// La hora local como `HH:MM`.
pub fn now_hms() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// Lo que el host refresca en cada tick y pasa al render de módulos.
/// Vacío/`None` = aún sin lectura (el reloj cae a la hora actual).
#[derive(Clone, Debug, Default)]
pub struct SysStats {
    pub time: String,
    pub cpu_pct: Option<f32>,
    pub mem_pct: Option<f32>,
}

impl SysStats {
    /// Una lectura instantánea (hora + RAM). El % de CPU necesita dos
    /// muestras, así que arranca en `None` y lo llena el primer tick.
    pub fn snapshot() -> Self {
        Self {
            time: now_hms(),
            cpu_pct: None,
            mem_pct: mem_pct(),
        }
    }
}

/// El hook `render_module` de referencia: pinta `clock`/`cpu`/`ram`/
/// `volume` con datos reales de `stats`. Devuelve `None` para cualquier
/// otro `kind` (que el renderer resolverá con su placeholder).
pub fn module_view<Msg: Clone + 'static>(
    m: &Module,
    stats: &SysStats,
    theme: &Theme,
) -> Option<View<Msg>> {
    let text = match m.kind.as_str() {
        "clock" => {
            if stats.time.is_empty() {
                now_hms()
            } else {
                stats.time.clone()
            }
        }
        "cpu" => match stats.cpu_pct {
            Some(p) => format!("CPU {p:.0}%"),
            None => "CPU —".to_string(),
        },
        "ram" => match stats.mem_pct {
            Some(p) => format!("RAM {p:.0}%"),
            None => "RAM —".to_string(),
        },
        "volume" => "🔊".to_string(),
        _ => return None,
    };
    Some(chip(theme, &text))
}

/// Chip de ancho aproximado al texto (sin medir la fuente), igual criterio
/// que el resto del launcher.
fn chip<Msg: Clone + 'static>(theme: &Theme, text: &str) -> View<Msg> {
    let w = (text.chars().count() as f32 * 8.0 + 16.0).max(40.0);
    View::new(Style {
        size: Size {
            width: length(w),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(4.0)
    .text_aligned(text.to_string(), 11.0, theme.fg_text, Alignment::Center)
}
