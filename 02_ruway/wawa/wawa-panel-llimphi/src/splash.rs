//! Config del **splash de arranque** (`arje-splash`) editable desde el panel.
//! Espejo de `arje-splash::config::SplashCfg` (mismo formato `clave = valor`).
//!
//! El panel corre como usuario, así que escribe en `$XDG_CONFIG_HOME/arje/` (o
//! `~/.config/arje/`). El **instalador** (`scripts/install-arje-splash.sh`) lee
//! ese archivo (y la imagen referenciada) y los hornea en el initramfs/ESP como
//! `/etc/arje/splash.conf`, que es de donde `arje-splash` lo lee al arrancar.
//! Editar acá ⇒ re-instalar para que el próximo arranque lo tome.

use std::path::PathBuf;

/// Fuentes posibles del splash: `(tag, etiqueta)`. Espejo de
/// `arje-splash::config::Source`.
pub const SOURCES: &[(&str, &str)] = &[
    ("chakana", "Chakana animada (marca)"),
    ("builtin", "Logo nativo (respiración)"),
    ("image", "Imagen PNG"),
    ("frames", "Animación (carpeta de PNG)"),
    ("lottie", "Lottie (.json, bakeado)"),
    ("rive", "rive (.ron del studio, bakeado)"),
];

/// Política del panel de logs de arranque: `(tag, etiqueta)`.
pub const LOG_MODES: &[(&str, &str)] = &[
    ("auto", "Automático (sólo si tarda o falla)"),
    ("off", "Nunca"),
];

/// Config del splash editable desde el panel.
#[derive(Clone, Debug, PartialEq)]
pub struct SplashCfg {
    pub source: String, // chakana | builtin | image | frames | lottie | rive
    pub image: String,  // ruta del PNG (source = image)
    pub frames: String, // ruta de la carpeta (source = frames)
    pub lottie: String, // ruta del .json (source = lottie)
    pub rive: String,   // ruta del .ron (source = rive)
    pub fps: u32,
    pub bg: String,     // #rrggbb
    pub accent: String, // #rrggbb
    pub logs: String,   // auto | off
}

impl Default for SplashCfg {
    fn default() -> Self {
        Self {
            // La chakana de marca es el default unificado de las tres superficies.
            source: "chakana".into(),
            image: String::new(),
            frames: String::new(),
            lottie: String::new(),
            rive: String::new(),
            fps: 30,
            bg: "#121218".into(),
            accent: "#7c83f7".into(),
            logs: "auto".into(),
        }
    }
}

impl SplashCfg {
    /// Carga del primer candidato que exista; cae al default.
    pub fn load() -> Self {
        let mut c = Self::default();
        for p in candidate_paths() {
            if let Ok(text) = std::fs::read_to_string(&p) {
                c.merge_text(&text);
                break;
            }
        }
        c
    }

    fn merge_text(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "source" => self.source = v.to_string(),
                "image" => {
                    self.image = v.to_string();
                    if !v.is_empty() {
                        self.source = "image".into();
                    }
                }
                "frames" => {
                    self.frames = v.to_string();
                    if !v.is_empty() {
                        self.source = "frames".into();
                    }
                }
                "lottie" => {
                    self.lottie = v.to_string();
                    if !v.is_empty() {
                        self.source = "lottie".into();
                    }
                }
                "rive" => {
                    self.rive = v.to_string();
                    if !v.is_empty() {
                        self.source = "rive".into();
                    }
                }
                "fps" => if let Ok(n) = v.parse() { self.fps = n },
                "bg" => self.bg = v.to_string(),
                "accent" => self.accent = v.to_string(),
                "logs" => self.logs = v.to_string(),
                _ => {}
            }
        }
    }

    fn to_text(&self) -> String {
        let path_line = match self.source.as_str() {
            "image" => format!("image = {}\n", self.image),
            "frames" => format!("frames = {}\n", self.frames),
            "lottie" => format!("lottie = {}\n", self.lottie),
            "rive" => format!("rive = {}\n", self.rive),
            _ => String::new(),
        };
        format!(
            "# arje-splash — config del arranque (la escribe wawa-panel)\n\
             source = {}\n\
             {path_line}\
             fps = {}\n\
             bg = {}\n\
             accent = {}\n\
             logs = {}\n",
            self.source, self.fps, self.bg, self.accent, self.logs,
        )
    }

    /// Persiste en el primer candidato escribible.
    pub fn save(&self) -> std::io::Result<()> {
        let body = self.to_text();
        let mut last = std::io::Error::new(std::io::ErrorKind::Other, "sin destino para splash.conf");
        for p in candidate_paths() {
            if let Some(dir) = p.parent() {
                if !dir.as_os_str().is_empty() && std::fs::create_dir_all(dir).is_err() {
                    continue;
                }
            }
            match std::fs::write(&p, &body) {
                Ok(()) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

/// Rutas candidatas, en orden de preferencia. El panel (usuario) suele escribir
/// la de `~/.config/arje/`; el instalador hornea desde ahí a `/etc/arje/`.
fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("ARJE_SPLASH_CONFIG").filter(|p| !p.is_empty()) {
        out.push(PathBuf::from(p));
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        out.push(PathBuf::from(x).join("arje/splash.conf"));
    }
    if let Some(h) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        out.push(PathBuf::from(h).join(".config/arje/splash.conf"));
    }
    out.push(PathBuf::from("/etc/arje/splash.conf"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_imagen() {
        let mut c = SplashCfg::default();
        c.source = "image".into();
        c.image = "/home/yo/logo.png".into();
        c.fps = 60;
        c.logs = "off".into();
        let mut back = SplashCfg::default();
        back.merge_text(&c.to_text());
        assert_eq!(back.source, "image");
        assert_eq!(back.image, "/home/yo/logo.png");
        assert_eq!(back.fps, 60);
        assert_eq!(back.logs, "off");
    }

    #[test]
    fn image_no_vacia_fija_source() {
        let mut c = SplashCfg::default();
        c.merge_text("image = /x/y.png\n");
        assert_eq!(c.source, "image");
    }
}
