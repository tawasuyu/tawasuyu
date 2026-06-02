//! OSD (On-Screen Display, U4): el cartelito transitorio que muestra volumen,
//! seek, velocidad, etc. sobre el video y se desvanece solo. Núcleo **puro**
//! (regla #2): modela *qué* mensaje está activo y con *cuánta* opacidad en un
//! instante dado; la UI sólo lo pinta. El tiempo se **inyecta** (`now` en
//! segundos) para ser determinista y testeable sin reloj real.

/// Estado de un OSD de un solo mensaje (el nuevo reemplaza al anterior, como
/// VLC/mpv). Guarda el texto, cuándo se mostró y cuándo expira.
#[derive(Debug, Clone, Default)]
pub struct Osd {
    msg: Option<String>,
    shown_at: f64,
    deadline: f64,
}

impl Osd {
    /// Duración por defecto de un mensaje en pantalla (segundos).
    pub const DEFAULT_SECS: f64 = 2.0;
    /// Tramo final en el que el mensaje se desvanece (segundos).
    pub const DEFAULT_FADE: f64 = 0.4;

    pub fn new() -> Self {
        Osd::default()
    }

    /// Muestra `text` desde `now` durante `duration` segundos (reemplaza lo
    /// anterior). `duration` ≤ 0 se ignora.
    pub fn show(&mut self, text: impl Into<String>, now: f64, duration: f64) {
        if duration <= 0.0 {
            return;
        }
        self.msg = Some(text.into());
        self.shown_at = now;
        self.deadline = now + duration;
    }

    /// Atajo: muestra con la duración por defecto.
    pub fn flash(&mut self, text: impl Into<String>, now: f64) {
        self.show(text, now, Self::DEFAULT_SECS);
    }

    /// El mensaje activo en `now`, si no expiró.
    pub fn active(&self, now: f64) -> Option<&str> {
        match &self.msg {
            Some(m) if now < self.deadline => Some(m),
            _ => None,
        }
    }

    /// Opacidad `[0,1]` del mensaje en `now`: `1.0` mientras dura, rampa lineal
    /// a `0.0` en los últimos `fade` segundos, `0.0` si expiró o no hay mensaje.
    pub fn alpha(&self, now: f64, fade: f64) -> f32 {
        if self.msg.is_none() || now >= self.deadline {
            return 0.0;
        }
        let fade = fade.max(0.0);
        if fade <= 0.0 {
            return 1.0;
        }
        let remaining = self.deadline - now;
        if remaining >= fade {
            1.0
        } else {
            (remaining / fade).clamp(0.0, 1.0) as f32
        }
    }

    /// Opacidad con el fade por defecto.
    pub fn alpha_default(&self, now: f64) -> f32 {
        self.alpha(now, Self::DEFAULT_FADE)
    }

    pub fn clear(&mut self) {
        self.msg = None;
        self.deadline = 0.0;
    }
}

/// Formatea segundos como `M:SS` (corto) o `H:MM:SS` (≥ 1 h). Negativos → 0.
pub fn format_hms(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Línea de volumen: `"Volumen 80%"` (o `"Silencio"` en 0 / mute).
pub fn format_volume(vol: f32) -> String {
    let pct = (vol.clamp(0.0, 2.0) * 100.0).round() as i32;
    if pct == 0 {
        "Silencio".to_string()
    } else {
        format!("Volumen {pct}%")
    }
}

/// Línea de velocidad: `"Velocidad 1.5×"` (sin decimales sobrantes).
pub fn format_speed(speed: f32) -> String {
    if (speed - speed.round()).abs() < 1e-3 {
        format!("Velocidad {}×", speed.round() as i32)
    } else {
        format!("Velocidad {speed:.2}×")
    }
}

/// Línea de seek: `"0:42 / 45:00"`.
pub fn format_seek(pos: f64, total: f64) -> String {
    format!("{} / {}", format_hms(pos), format_hms(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muestra_y_expira() {
        let mut osd = Osd::new();
        assert!(osd.active(0.0).is_none());
        osd.show("hola", 10.0, 2.0);
        assert_eq!(osd.active(10.0), Some("hola"));
        assert_eq!(osd.active(11.9), Some("hola"));
        assert!(osd.active(12.0).is_none()); // expiró (now >= deadline)
        assert!(osd.active(20.0).is_none());
    }

    #[test]
    fn nuevo_reemplaza_anterior() {
        let mut osd = Osd::new();
        osd.show("uno", 0.0, 5.0);
        osd.show("dos", 1.0, 2.0);
        assert_eq!(osd.active(1.5), Some("dos"));
        assert!(osd.active(3.5).is_none()); // domina el deadline del nuevo
    }

    #[test]
    fn alpha_rampa_en_el_fade() {
        let mut osd = Osd::new();
        osd.show("x", 0.0, 2.0); // deadline = 2.0
        assert_eq!(osd.alpha(0.5, 0.4), 1.0); // remaining 1.5 ≥ fade
        assert_eq!(osd.alpha(1.6, 0.4), 1.0); // remaining 0.4 = fade
        // remaining 0.2, fade 0.4 → 0.5
        assert!((osd.alpha(1.8, 0.4) - 0.5).abs() < 1e-6);
        assert_eq!(osd.alpha(2.0, 0.4), 0.0); // expiró
        assert_eq!(osd.alpha(0.5, 0.0), 1.0); // sin fade
    }

    #[test]
    fn clear_y_duracion_invalida() {
        let mut osd = Osd::new();
        osd.show("x", 0.0, 0.0); // duración inválida → nada
        assert!(osd.active(0.0).is_none());
        osd.flash("y", 0.0);
        assert_eq!(osd.active(0.0), Some("y"));
        osd.clear();
        assert!(osd.active(0.0).is_none());
        assert_eq!(osd.alpha_default(0.0), 0.0);
    }

    #[test]
    fn formato_hms() {
        assert_eq!(format_hms(0.0), "0:00");
        assert_eq!(format_hms(42.0), "0:42");
        assert_eq!(format_hms(125.0), "2:05");
        assert_eq!(format_hms(3661.0), "1:01:01");
        assert_eq!(format_hms(-5.0), "0:00");
    }

    #[test]
    fn formato_volumen_velocidad_seek() {
        assert_eq!(format_volume(0.8), "Volumen 80%");
        assert_eq!(format_volume(0.0), "Silencio");
        assert_eq!(format_volume(1.5), "Volumen 150%");
        assert_eq!(format_speed(1.0), "Velocidad 1×");
        assert_eq!(format_speed(1.5), "Velocidad 1.50×");
        assert_eq!(format_seek(42.0, 2700.0), "0:42 / 45:00");
    }
}
