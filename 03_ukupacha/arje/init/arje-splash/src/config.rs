//! Config del splash — **el contrato** entre quien la escribe (wawa-panel,
//! sección «Arranque») y quien la lee (este binario, en el initramfs).
//!
//! Formato `clave = valor` en texto plano (igual que `mirada-greeter`/greeter.conf
//! y el resto del panel): sin dependencias de parser, trivial de leer en el
//! initramfs y de escribir desde el panel. wawa-panel mantiene un espejo de
//! este esquema en `wawa-panel-llimphi::splash`.
//!
//! Rutas candidatas (primera que exista): `$ARJE_SPLASH_CONFIG`, luego
//! `/etc/arje/splash.conf`. El instalador hornea el archivo (y la imagen) en el
//! initramfs/ESP. Si no hay config, todo cae a los defaults (splash builtin).

use std::path::PathBuf;

/// De dónde sale lo que se pinta.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// El splash nativo (logo de marca respirando + barra). Sin archivos.
    Builtin,
    /// Una imagen PNG estática, centrada y escalada a la pantalla.
    Image(PathBuf),
    /// Una carpeta con cuadros `*.png` (orden alfabético) reproducidos en loop.
    Frames(PathBuf),
}

/// Política del panel de logs de arranque.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LogMode {
    /// Nunca mostrar logs sobre el splash.
    Off,
    /// Mostrarlos sólo si el arranque tarda demasiado o hay un error.
    Auto,
}

/// Config completa del splash.
#[derive(Clone, Debug, PartialEq)]
pub struct SplashCfg {
    pub source: Source,
    pub fps: u64,
    pub max_ms: u64,
    pub bg: (u8, u8, u8),
    pub accent: (u8, u8, u8),
    pub logs: LogMode,
    /// Ms tras los que, si el arranque sigue, aparece el panel de logs
    /// (también aparece ante un error, sin esperar). Sólo con `logs = auto`.
    pub log_after_ms: u64,
}

impl Default for SplashCfg {
    fn default() -> Self {
        Self {
            source: Source::Builtin,
            fps: 30,
            max_ms: 8000,
            bg: crate::render::BG,
            accent: crate::render::ACCENT,
            logs: LogMode::Auto,
            log_after_ms: 6000,
        }
    }
}

impl SplashCfg {
    /// Carga del primer candidato que exista; cae al default.
    pub fn load() -> Self {
        let mut cfg = Self::default();
        for p in candidate_paths() {
            if let Ok(text) = std::fs::read_to_string(&p) {
                cfg.merge_text(&text);
                break;
            }
        }
        cfg
    }

    /// Mezcla las claves presentes en `text` sobre la config actual.
    pub fn merge_text(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "source" => match v.to_ascii_lowercase().as_str() {
                    "builtin" => self.source = Source::Builtin,
                    "image" => {} // la ruta llega en `image =`
                    "frames" => {}
                    _ => {}
                },
                "image" if !v.is_empty() => self.source = Source::Image(PathBuf::from(v)),
                "frames" if !v.is_empty() => self.source = Source::Frames(PathBuf::from(v)),
                "fps" => if let Ok(n) = v.parse() { self.fps = n },
                "max_ms" => if let Ok(n) = v.parse() { self.max_ms = n },
                "bg" => if let Some(c) = parse_hex(v) { self.bg = c },
                "accent" => if let Some(c) = parse_hex(v) { self.accent = c },
                "logs" => self.logs = match v.to_ascii_lowercase().as_str() {
                    "off" | "no" | "0" => LogMode::Off,
                    _ => LogMode::Auto,
                },
                "log_after_ms" => if let Ok(n) = v.parse() { self.log_after_ms = n },
                _ => {}
            }
        }
    }

    /// Serializa a texto `clave = valor` (lo que escribe wawa-panel).
    pub fn to_text(&self) -> String {
        let (src, path) = match &self.source {
            Source::Builtin => ("builtin", String::new()),
            Source::Image(p) => ("image", p.display().to_string()),
            Source::Frames(p) => ("frames", p.display().to_string()),
        };
        let key = match &self.source {
            Source::Image(_) => format!("image = {path}\n"),
            Source::Frames(_) => format!("frames = {path}\n"),
            Source::Builtin => String::new(),
        };
        format!(
            "# arje-splash — config del arranque (la escribe wawa-panel)\n\
             source = {src}\n\
             {key}\
             fps = {}\n\
             max_ms = {}\n\
             bg = {}\n\
             accent = {}\n\
             logs = {}\n\
             log_after_ms = {}\n",
            self.fps,
            self.max_ms,
            hex(self.bg),
            hex(self.accent),
            match self.logs { LogMode::Off => "off", LogMode::Auto => "auto" },
            self.log_after_ms,
        )
    }
}

/// `#rrggbb` (o `rrggbb`) → `(r,g,b)`.
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

/// `(r,g,b)` → `#rrggbb`.
pub fn hex(c: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// Rutas candidatas, en orden de preferencia.
fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("ARJE_SPLASH_CONFIG").filter(|p| !p.is_empty()) {
        out.push(PathBuf::from(p));
    }
    out.push(PathBuf::from("/etc/arje/splash.conf"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_y_vuelta() {
        assert_eq!(parse_hex("#7c83f7"), Some((124, 131, 247)));
        assert_eq!(parse_hex("121218"), Some((18, 18, 24)));
        assert_eq!(parse_hex("#xyz"), None);
        assert_eq!(hex((124, 131, 247)), "#7c83f7");
    }

    #[test]
    fn merge_image_y_round_trip() {
        let mut c = SplashCfg::default();
        c.merge_text(
            "source = image\nimage = /etc/arje/splash.png\nfps = 60\nbg = #0a0a0a\nlogs = off\n",
        );
        assert_eq!(c.source, Source::Image(PathBuf::from("/etc/arje/splash.png")));
        assert_eq!(c.fps, 60);
        assert_eq!(c.bg, (10, 10, 10));
        assert_eq!(c.logs, LogMode::Off);
        // Round-trip: to_text → merge → mismo valor.
        let mut back = SplashCfg::default();
        back.merge_text(&c.to_text());
        assert_eq!(back, c);
    }

    #[test]
    fn frames_y_default_builtin() {
        let mut c = SplashCfg::default();
        assert_eq!(c.source, Source::Builtin);
        c.merge_text("frames = /etc/arje/boot-anim\n");
        assert_eq!(c.source, Source::Frames(PathBuf::from("/etc/arje/boot-anim")));
    }
}
