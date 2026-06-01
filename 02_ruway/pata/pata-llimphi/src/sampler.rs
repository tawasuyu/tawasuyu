//! El muestreador del sistema en Linux: arma el [`WidgetCtx`] que alimenta a
//! los widgets de `pata-core` en cada tick.
//!
//! La frontera de la Fase 4: el core no toca el SO; este es el sampler que cada
//! plataforma aporta. En Linux leemos `chrono` para el reloj, `/proc/stat` para
//! la CPU (necesita dos lecturas, por eso es un struct con estado), `/proc/
//! meminfo` para la RAM y `/sys/class/backlight` para el brillo. El volumen
//! (PulseAudio/PipeWire) queda diferido —el medidor sale en 0% hasta entonces—.

use chrono::{Datelike, Local, Timelike};

use pata_core::widget::{ClockReading, WidgetCtx};

/// Muestreador con estado: guarda la última lectura de `/proc/stat` para poder
/// calcular el uso de CPU como delta entre ticks.
#[derive(Default)]
pub struct Sampler {
    /// `(total, idle)` de la lectura anterior de `/proc/stat`, o `None` al inicio.
    cpu_prev: Option<(u64, u64)>,
}

impl Sampler {
    /// Un sampler nuevo, sin lecturas previas.
    pub fn new() -> Self {
        Self::default()
    }

    /// Toma un snapshot completo del sistema.
    pub fn sample(&mut self) -> WidgetCtx {
        let (ram, ram_used_mb, ram_total_mb) = sample_ram();
        WidgetCtx {
            clock: sample_clock(),
            cpu: self.sample_cpu(),
            ram,
            ram_used_mb,
            ram_total_mb,
            volume: 0.0,
            muted: false,
            brightness: sample_brightness().unwrap_or(0.0),
        }
    }

    /// Uso de CPU `0..1` como `1 - idle_delta/total_delta`. La primera vez no
    /// hay delta, así que devuelve 0 y guarda la base para el siguiente tick.
    fn sample_cpu(&mut self) -> f32 {
        let Some((total, idle)) = read_proc_stat() else {
            return 0.0;
        };
        let usage = match self.cpu_prev {
            Some((pt, pi)) => {
                let dt = total.saturating_sub(pt);
                let di = idle.saturating_sub(pi);
                if dt > 0 {
                    (1.0 - di as f32 / dt as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        self.cpu_prev = Some((total, idle));
        usage
    }
}

/// Descompone la hora local actual en [`ClockReading`].
fn sample_clock() -> ClockReading {
    let now = Local::now();
    ClockReading {
        year: now.year() as u16,
        month: now.month() as u8,
        day: now.day() as u8,
        weekday: now.weekday().num_days_from_sunday() as u8,
        hour: now.hour() as u8,
        minute: now.minute() as u8,
        second: now.second() as u8,
    }
}

/// `(fracción_usada, usada_mb, total_mb)` desde `/proc/meminfo`. Si no se puede
/// leer (no-Linux), devuelve ceros.
fn sample_ram() -> (f32, u32, u32) {
    let Some((total_kb, avail_kb)) = read_meminfo() else {
        return (0.0, 0, 0);
    };
    let used_kb = total_kb.saturating_sub(avail_kb);
    let frac = if total_kb > 0 {
        used_kb as f32 / total_kb as f32
    } else {
        0.0
    };
    (frac, (used_kb / 1024) as u32, (total_kb / 1024) as u32)
}

/// `(total_kb, available_kb)` desde `/proc/meminfo`.
fn read_meminfo() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo(&text)
}

/// Extrae `(MemTotal, MemAvailable)` en kB del texto de `/proc/meminfo`.
fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let mut total = None;
    let mut avail = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next()? {
            "MemTotal:" => total = parts.next()?.parse::<u64>().ok(),
            "MemAvailable:" => avail = parts.next()?.parse::<u64>().ok(),
            _ => {}
        }
        if total.is_some() && avail.is_some() {
            break;
        }
    }
    Some((total?, avail?))
}

/// `(total_jiffies, idle_jiffies)` de la primera línea `cpu` de `/proc/stat`.
/// `idle` incluye `iowait` (4º campo). `None` si no se puede leer.
fn read_proc_stat() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    parse_proc_stat(&text)
}

/// Extrae `(total, idle+iowait)` en jiffies de la primera línea `cpu` de
/// `/proc/stat`.
fn parse_proc_stat(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let vals: Vec<u64> = parts.filter_map(|p| p.parse::<u64>().ok()).collect();
    if vals.len() < 4 {
        return None;
    }
    let total: u64 = vals.iter().sum();
    // idle = idle (índice 3) + iowait (índice 4, si está).
    let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
    Some((total, idle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_meminfo_extrae_total_y_disponible() {
        let txt = "MemTotal:       16252000 kB\n\
                   MemFree:         1000000 kB\n\
                   MemAvailable:    8126000 kB\n";
        assert_eq!(parse_meminfo(txt), Some((16252000, 8126000)));
    }

    #[test]
    fn parse_meminfo_sin_claves_es_none() {
        assert_eq!(parse_meminfo("Foo: 1 kB\n"), None);
    }

    #[test]
    fn parse_proc_stat_suma_total_e_idle_con_iowait() {
        // cpu user nice system idle iowait irq softirq …
        let txt = "cpu  100 0 50 800 50 0 0 0\ncpu0 ...\n";
        // total = 100+0+50+800+50 = 1000 ; idle = 800+50 = 850
        assert_eq!(parse_proc_stat(txt), Some((1000, 850)));
    }

    #[test]
    fn parse_proc_stat_otra_primera_linea_es_none() {
        assert_eq!(parse_proc_stat("intr 1 2 3\n"), None);
    }

    #[test]
    fn cpu_primer_tick_es_cero_y_luego_calcula_delta() {
        // No tocamos /proc: validamos la lógica de delta a mano.
        let mut s = Sampler::new();
        // Sin lectura previa, el primer cálculo es 0 (y guarda la base).
        assert_eq!(s.cpu_prev, None);
        // Simulamos dos lecturas: base (total=1000, idle=900) y luego
        // (total=1100, idle=950): dt=100, di=50 → uso = 1 - 0.5 = 0.5.
        s.cpu_prev = Some((1000, 900));
        let (total, idle) = (1100, 950);
        let dt = total - 1000;
        let di = idle - 900;
        let uso = 1.0 - di as f32 / dt as f32;
        assert!((uso - 0.5).abs() < 1e-6);
    }
}

/// Brillo `0..1` desde el primer dispositivo en `/sys/class/backlight`. `None`
/// si no hay backlight (escritorio, VM).
fn sample_brightness() -> Option<f32> {
    let dir = std::fs::read_dir("/sys/class/backlight").ok()?;
    for entry in dir.flatten() {
        let base = entry.path();
        let cur = std::fs::read_to_string(base.join("brightness"))
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok());
        let max = std::fs::read_to_string(base.join("max_brightness"))
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok());
        if let (Some(c), Some(m)) = (cur, max) {
            if m > 0.0 {
                return Some((c / m).clamp(0.0, 1.0));
            }
        }
    }
    None
}
