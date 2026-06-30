//! Política de **autologin**: entrar sin tipear contraseña, como preferencia.
//!
//! Vive en su **propio archivo** (`mirada/autologin.conf`), no en `greeter.conf`,
//! para que el panel (escritor) y el greeter (lector) no se pisen las claves.
//! Formato `clave = valor` (sin TOML, como el resto de configs de mirada).
//!
//! El autologin tensa con el cifrado de dotfiles de pacha: sin contraseña tipeada
//! no hay con qué desbloquear la identidad. Por eso `secretos` deja **elegir el
//! tradeoff explícitamente** ([`SecretosPolitica`]).

use std::path::PathBuf;

/// Qué pasa con los secretos (cifrado de dotfiles) bajo autologin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecretosPolitica {
    /// **Manual**: la identidad queda bloqueada; el cifrado de dotfiles se
    /// desbloquea a mano (panel → Identidad, o `agora-cli desbloquear`). Es el
    /// default seguro: ninguna frase toca el disco.
    #[default]
    Manual,
    /// **Sellada**: la frase de la identidad se guarda en un archivo privado
    /// (0600) y la sesión la usa para auto-desbloquear al entrar. Más cómodo,
    /// pero **debilita** el modelo: el "secreto para acceder a los secretos"
    /// deja de estar sólo en tu cabeza y queda en disco.
    Sellada,
}

impl SecretosPolitica {
    pub fn tag(self) -> &'static str {
        match self {
            SecretosPolitica::Manual => "manual",
            SecretosPolitica::Sellada => "sellada",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "manual" => Some(SecretosPolitica::Manual),
            "sellada" | "sealed" => Some(SecretosPolitica::Sellada),
            _ => None,
        }
    }
}

/// Configuración de autologin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutologinCfg {
    /// Si `true`, el greeter entra solo (sin pedir usuario/contraseña).
    pub enabled: bool,
    /// Usuario al que entrar.
    pub user: String,
    /// Nombre de la sesión a arrancar (se matchea contra los `.desktop`
    /// descubiertos). Vacío = que el compositor decida.
    pub session: String,
    /// Tradeoff de secretos bajo autologin.
    pub secretos: SecretosPolitica,
}

impl AutologinCfg {
    /// Carga la config del primer archivo existente. Sin archivo → default
    /// (deshabilitado).
    pub fn load() -> Self {
        for p in candidate_paths() {
            if let Ok(text) = std::fs::read_to_string(&p) {
                return Self::parse(&text);
            }
        }
        Self::default()
    }

    /// Parsea el cuerpo `clave = valor`.
    pub fn parse(text: &str) -> Self {
        let mut cfg = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "autologin" | "enabled" => cfg.enabled = truthy(v),
                "user" => cfg.user = v.to_string(),
                "session" => cfg.session = v.to_string(),
                "secretos" => {
                    if let Some(s) = SecretosPolitica::parse(v) {
                        cfg.secretos = s;
                    }
                }
                _ => {}
            }
        }
        cfg
    }

    /// Serializa a `clave = valor`.
    pub fn to_text(&self) -> String {
        format!(
            "# mirada — política de autologin (la edita el wawa-panel)\n\
             autologin = {}\n\
             user = {}\n\
             session = {}\n\
             secretos = {}\n",
            self.enabled,
            self.user,
            self.session,
            self.secretos.tag(),
        )
    }

    /// Persiste al primer destino escribible. `Err` si ninguno lo es.
    pub fn save(&self) -> std::io::Result<()> {
        let body = self.to_text();
        let mut last_err = std::io::Error::new(
            std::io::ErrorKind::Other,
            "sin destino escribible para autologin.conf",
        );
        for p in candidate_paths() {
            if let Some(dir) = p.parent() {
                if std::fs::create_dir_all(dir).is_err() {
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

/// Mismos candidatos que `greeter.conf`: config del usuario primero, luego el
/// sistema (que es el que el greeter lee ANTES del login).
fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        out.push(PathBuf::from(x).join("mirada/autologin.conf"));
    }
    if let Ok(h) = std::env::var("HOME") {
        out.push(PathBuf::from(h).join(".config/mirada/autologin.conf"));
    }
    out.push(PathBuf::from("/var/lib/mirada/autologin.conf"));
    out
}

fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on" | "sí" | "si")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_y_default() {
        assert!(!AutologinCfg::default().enabled);
        let cfg = AutologinCfg {
            enabled: true,
            user: "sergio".into(),
            session: "mirada · pata".into(),
            secretos: SecretosPolitica::Sellada,
        };
        let back = AutologinCfg::parse(&cfg.to_text());
        assert_eq!(back, cfg);
    }

    #[test]
    fn parse_tolera_basura_y_secretos_default_manual() {
        let cfg = AutologinCfg::parse("# x\nautologin=yes\nuser = ana\nbasura\nsecretos = ???\n");
        assert!(cfg.enabled);
        assert_eq!(cfg.user, "ana");
        assert_eq!(cfg.secretos, SecretosPolitica::Manual); // valor inválido → default
    }
}
