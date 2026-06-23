//! Estado persistente del greeter: recuerda el último usuario y el último
//! escritorio elegidos, y lleva la config del fondo (rusty rain).
//!
//! Formato: un archivo de texto con líneas `clave = valor` (igual de simple
//! que el parser de `.desktop` de [`crate::sessions`]; no metemos `toml` por
//! un puñado de claves). Se lee en `init` y se reescribe tras un login válido.
//!
//! Ruta: se prueban candidatas en orden (override por entorno, XDG, `$HOME`,
//! y `/var/lib` para arranques de sistema sin home de usuario). La lectura usa
//! la primera que exista; la escritura, la primera donde se pueda crear el
//! directorio y escribir.

use std::path::PathBuf;

/// Paleta del fondo de lluvia.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RainColor {
    Green,
    Red,
    Amber,
    Cyan,
    Accent,
}

impl RainColor {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "green" | "verde" => Some(Self::Green),
            "red" | "rojo" => Some(Self::Red),
            "amber" | "ambar" | "ámbar" => Some(Self::Amber),
            "cyan" | "cian" => Some(Self::Cyan),
            "accent" | "acento" => Some(Self::Accent),
            _ => None,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::Red => "red",
            Self::Amber => "amber",
            Self::Cyan => "cyan",
            Self::Accent => "accent",
        }
    }
}

/// Animación de fondo elegida. El fondo es enchufable: cada variante es una
/// función pura `paint(scene, ts, rect, t, color)`. `rain_enabled` es el
/// interruptor maestro; `anim` elige cuál se pinta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BgAnim {
    /// Lluvia de glifos estilo Matrix (`rain`).
    Matrix,
    /// Campo de estrellas en warp (`stars`).
    Stars,
    /// Ondas/plasma sinusoidal (`waves`).
    Waves,
}

impl BgAnim {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "matrix" | "rain" | "lluvia" => Some(Self::Matrix),
            "stars" | "estrellas" | "starfield" => Some(Self::Stars),
            "waves" | "ondas" | "plasma" => Some(Self::Waves),
            _ => None,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::Matrix => "matrix",
            Self::Stars => "stars",
            Self::Waves => "waves",
        }
    }
}

/// Estado persistido del greeter.
#[derive(Clone, Debug)]
pub struct GreeterState {
    /// Último usuario que entró (para prerellenar el campo).
    pub last_user: String,
    /// Nombre del último escritorio elegido (se matchea contra `sessions`).
    pub last_session: String,
    /// ¿Pintar el fondo animado?
    pub rain_enabled: bool,
    /// Paleta del fondo.
    pub rain_color: RainColor,
    /// Qué animación de fondo pintar.
    pub anim: BgAnim,
}

impl Default for GreeterState {
    fn default() -> Self {
        Self {
            last_user: String::new(),
            last_session: String::new(),
            // El fondo viene encendido por defecto; se apaga por archivo o
            // por `MIRADA_GREETER_RAIN=0`.
            rain_enabled: true,
            rain_color: RainColor::Green,
            anim: BgAnim::Matrix,
        }
    }
}

/// `true` si el valor representa un booleano afirmativo.
fn truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes" | "si" | "sí"
    )
}

impl GreeterState {
    /// Carga el estado del primer archivo candidato que exista, y aplica los
    /// overrides de entorno por encima. Nunca falla: cae al default.
    pub fn load() -> Self {
        let mut st = Self::default();
        for p in candidate_paths() {
            if let Ok(text) = std::fs::read_to_string(&p) {
                st.merge_text(&text);
                break;
            }
        }
        st.apply_env();
        st
    }

    /// Mezcla las claves de un archivo `clave = valor` sobre `self`.
    fn merge_text(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "last_user" => self.last_user = v.to_string(),
                "last_session" => self.last_session = v.to_string(),
                "rain" => self.rain_enabled = truthy(v),
                "rain_color" => {
                    if let Some(c) = RainColor::parse(v) {
                        self.rain_color = c;
                    }
                }
                "bg" | "anim" => {
                    if let Some(a) = BgAnim::parse(v) {
                        self.anim = a;
                    }
                }
                _ => {}
            }
        }
    }

    /// Overrides de entorno: `MIRADA_GREETER_RAIN` (bool) y
    /// `MIRADA_GREETER_RAIN_COLOR` (paleta).
    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("MIRADA_GREETER_RAIN") {
            self.rain_enabled = truthy(&v);
        }
        if let Ok(v) = std::env::var("MIRADA_GREETER_RAIN_COLOR") {
            if let Some(c) = RainColor::parse(&v) {
                self.rain_color = c;
            }
        }
        if let Ok(v) = std::env::var("MIRADA_GREETER_BG") {
            if let Some(a) = BgAnim::parse(&v) {
                self.anim = a;
            }
        }
    }

    /// Serializa a `clave = valor`.
    fn to_text(&self) -> String {
        format!(
            "# mirada-greeter — estado recordado\n\
             last_user = {}\n\
             last_session = {}\n\
             rain = {}\n\
             rain_color = {}\n\
             bg = {}\n",
            self.last_user,
            self.last_session,
            self.rain_enabled,
            self.rain_color.tag(),
            self.anim.tag(),
        )
    }

    /// Persiste el estado en el primer candidato escribible. Silencioso: si
    /// ningún destino acepta la escritura (greeter sin home, FS de sólo
    /// lectura) no es fatal — sólo se pierde la memoria entre logins.
    pub fn save(&self) {
        let body = self.to_text();
        for p in candidate_paths() {
            if let Some(dir) = p.parent() {
                if !dir.as_os_str().is_empty() && std::fs::create_dir_all(dir).is_err() {
                    continue;
                }
            }
            if std::fs::write(&p, &body).is_ok() {
                return;
            }
        }
    }
}

/// Rutas candidatas para el archivo de estado, en orden de preferencia.
fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("MIRADA_GREETER_STATE") {
        if !p.is_empty() {
            out.push(PathBuf::from(p));
        }
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        out.push(PathBuf::from(x).join("mirada/greeter.conf"));
    }
    if let Some(h) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        out.push(PathBuf::from(h).join(".config/mirada/greeter.conf"));
    }
    out.push(PathBuf::from("/var/lib/mirada/greeter.conf"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_claves() {
        let mut st = GreeterState::default();
        st.merge_text(
            "# comentario\nlast_user = ana\nlast_session = Sway\nrain = off\nrain_color = cyan\n",
        );
        assert_eq!(st.last_user, "ana");
        assert_eq!(st.last_session, "Sway");
        assert!(!st.rain_enabled);
        assert_eq!(st.rain_color, RainColor::Cyan);
    }

    #[test]
    fn round_trip() {
        let st = GreeterState {
            last_user: "bob".into(),
            last_session: "mirada · pata".into(),
            rain_enabled: true,
            rain_color: RainColor::Amber,
            anim: BgAnim::Stars,
        };
        let mut back = GreeterState::default();
        back.merge_text(&st.to_text());
        assert_eq!(back.last_user, "bob");
        assert_eq!(back.last_session, "mirada · pata");
        assert!(back.rain_enabled);
        assert_eq!(back.rain_color, RainColor::Amber);
        assert_eq!(back.anim, BgAnim::Stars);
    }
}
