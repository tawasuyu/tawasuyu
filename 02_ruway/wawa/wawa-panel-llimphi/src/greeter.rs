//! Lectura/escritura de la config del **greeter** (la pantalla de login del
//! DM) desde wawa-panel: el fondo animado y su paleta. El greeter la lee al
//! arrancar (`GreeterState::load`) — los cambios entran en el próximo login.
//!
//! Formato y rutas espejan `mirada-greeter/src/state.rs` (texto `clave =
//! valor`; candidatas `$MIRADA_GREETER_STATE`, `$XDG_CONFIG_HOME/mirada/`,
//! `$HOME/.config/mirada/`, `/var/lib/mirada/`). Guardamos las 5 claves para
//! NO pisar `last_user` / `last_session` que escribe el propio greeter.
//!
//! La lista de animaciones (`ANIMS`) debe seguir a `state::BgAnim` del greeter;
//! si sumás una variante allá (p. ej. un efecto estilo termflix), agregala acá.

use std::path::PathBuf;

/// Animaciones de fondo disponibles: `(tag, etiqueta)`. Espejo de
/// `mirada-greeter::state::BgAnim`.
pub const ANIMS: &[(&str, &str)] = &[
    ("matrix", "Matrix (lluvia de glifos)"),
    ("stars", "Estrellas (starfield)"),
    ("waves", "Ondas"),
    ("fire", "Fuego"),
    ("plasma", "Plasma"),
    ("aurora", "Aurora"),
    ("lightning", "Rayos"),
];

/// Paletas del fondo: `(tag, etiqueta)`. Espejo de
/// `mirada-greeter::state::RainColor`.
pub const COLORS: &[(&str, &str)] = &[
    ("green", "Verde"),
    ("red", "Rojo"),
    ("amber", "Ámbar"),
    ("cyan", "Cian"),
    ("accent", "Acento del tema"),
];

/// Config del greeter editable desde el panel. `last_user`/`last_session` se
/// conservan tal cual para no perderlos al reescribir.
#[derive(Clone, Debug)]
pub struct GreeterCfg {
    pub last_user: String,
    pub last_session: String,
    pub rain_enabled: bool,
    pub rain_color: String,
    pub anim: String,
    /// Ruta a un `.json` de Lottie a usar como **fondo vivo** del greeter (toma
    /// precedencia sobre la animación procedural). Vacío = sin Lottie. Se
    /// preserva al reescribir — antes el panel reescribía sin esta clave y
    /// **borraba** el Lottie que el usuario hubiera puesto a mano.
    pub lottie: String,
}

impl Default for GreeterCfg {
    fn default() -> Self {
        Self {
            last_user: String::new(),
            last_session: String::new(),
            rain_enabled: true,
            rain_color: "green".into(),
            anim: "matrix".into(),
            lottie: String::new(),
        }
    }
}

fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes" | "si" | "sí")
}

impl GreeterCfg {
    /// Carga del primer candidato que exista; cae al default.
    pub fn load() -> Self {
        let mut st = Self::default();
        for p in candidate_paths() {
            if let Ok(text) = std::fs::read_to_string(&p) {
                st.merge_text(&text);
                break;
            }
        }
        st
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
                "last_user" => self.last_user = v.to_string(),
                "last_session" => self.last_session = v.to_string(),
                "rain" => self.rain_enabled = truthy(v),
                "rain_color" => self.rain_color = v.to_string(),
                "bg" | "anim" => self.anim = v.to_string(),
                "lottie" => self.lottie = v.to_string(),
                _ => {}
            }
        }
    }

    fn to_text(&self) -> String {
        let mut s = format!(
            "# mirada-greeter — estado recordado\n\
             last_user = {}\n\
             last_session = {}\n\
             rain = {}\n\
             rain_color = {}\n\
             bg = {}\n",
            self.last_user, self.last_session, self.rain_enabled, self.rain_color, self.anim,
        );
        // Preservá el Lottie de fondo si está configurado (no lo borres).
        if !self.lottie.trim().is_empty() {
            s.push_str(&format!("lottie = {}\n", self.lottie.trim()));
        }
        s
    }

    /// Persiste en el primer candidato escribible.
    pub fn save(&self) -> std::io::Result<()> {
        let body = self.to_text();
        let mut last_err =
            std::io::Error::new(std::io::ErrorKind::Other, "sin destino escribible para greeter.conf");
        for p in candidate_paths() {
            if let Some(dir) = p.parent() {
                if !dir.as_os_str().is_empty() && std::fs::create_dir_all(dir).is_err() {
                    continue;
                }
            }
            match std::fs::write(&p, &body) {
                Ok(()) => return Ok(()),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserva_usuario_y_sesion() {
        let mut g = GreeterCfg::default();
        g.merge_text(
            "last_user = ana\nlast_session = mirada · pata\nrain = on\nrain_color = cyan\nbg = stars\n",
        );
        assert_eq!(g.last_user, "ana");
        assert!(g.rain_enabled);
        assert_eq!(g.rain_color, "cyan");
        assert_eq!(g.anim, "stars");
        // Reescritura → re-lectura conserva todo (incluido usuario/sesión).
        let mut back = GreeterCfg::default();
        back.merge_text(&g.to_text());
        assert_eq!(back.last_user, "ana");
        assert_eq!(back.last_session, "mirada · pata");
        assert_eq!(back.anim, "stars");
        assert_eq!(back.rain_color, "cyan");
    }

    #[test]
    fn anim_off_se_respeta() {
        let mut g = GreeterCfg::default();
        g.merge_text("rain = off\n");
        assert!(!g.rain_enabled);
    }
}

/// Rutas candidatas, en orden de preferencia (espejo del greeter).
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
