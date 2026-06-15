//! El tiquet de sesión: lo que el greeter le entrega al compositor tras
//! una autenticación exitosa.
//!
//! El greeter de mirada corre como proceso hijo del compositor. Cuando
//! el login tiene éxito, imprime **una línea** de tiquet a su stdout; el
//! compositor escanea las líneas del hijo buscando el prefijo
//! [`TICKET_TAG`] y, al encontrarlo, hace el traspaso a modo sesión.

use std::path::PathBuf;

use crate::UserInfo;

/// Etiqueta + versión de la línea de tiquet. El compositor sólo trata
/// como tiquet las líneas que empiezan con esto — el resto del stdout
/// del greeter (logs, ruido) se ignora.
pub const TICKET_TAG: &str = "MIRADA-SESSION-TICKET-v1";

/// Resultado de un login: la identidad autenticada más, opcionalmente,
/// el comando de sesión elegido. El greeter lo produce; el compositor lo
/// consume para arrancar la sesión (setuid al usuario + spawn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTicket {
    /// Identidad del usuario autenticado.
    pub user: UserInfo,
    /// Comando de sesión a ejecutar como el usuario. Vacío = que el
    /// compositor decida (su autostart por defecto).
    pub session: String,
    /// `true` si la sesión es un compositor **ajeno** (sway, Plasma…): el
    /// servidor actual debe soltar el DRM y hacer `exec`, no correrla como
    /// cliente. `false` para sesiones nativas de mirada (pata, autostart),
    /// que sí corren como clientes del mismo compositor.
    pub foreign: bool,
}

impl SessionTicket {
    /// Crea un tiquet sin comando de sesión explícito.
    pub fn new(user: UserInfo) -> Self {
        Self {
            user,
            session: String::new(),
            foreign: false,
        }
    }

    /// Fija el comando de sesión. Encadenable.
    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = session.into();
        self
    }

    /// Marca la sesión como compositor ajeno (handoff por `exec`).
    /// Encadenable.
    pub fn foreign(mut self, foreign: bool) -> Self {
        self.foreign = foreign;
        self
    }

    /// Serializa el tiquet a una línea única, apta para stdout. Campos
    /// separados por tabulador: ni los nombres de usuario, ni los paths,
    /// ni los comandos de sesión suelen contener tabuladores.
    pub fn to_line(&self) -> String {
        format!(
            "{TICKET_TAG}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.user.name,
            self.user.uid,
            self.user.gid,
            self.user.home.display(),
            self.user.shell.display(),
            self.session,
            if self.foreign { "1" } else { "0" },
        )
    }

    /// Parsea una línea producida por [`to_line`]. `None` si la línea no
    /// es un tiquet (otra salida del greeter) o está malformada.
    pub fn from_line(line: &str) -> Option<SessionTicket> {
        let mut f = line.trim_end_matches(['\r', '\n']).split('\t');
        if f.next()? != TICKET_TAG {
            return None;
        }
        let name = f.next()?.to_string();
        let uid = f.next()?.parse().ok()?;
        let gid = f.next()?.parse().ok()?;
        let home = PathBuf::from(f.next()?);
        let shell = PathBuf::from(f.next()?);
        // El comando de sesión puede venir vacío.
        let session = f.next().unwrap_or("").to_string();
        // El flag `foreign` es opcional (tiquets viejos no lo traen).
        let foreign = matches!(f.next(), Some("1"));
        Some(SessionTicket {
            user: UserInfo {
                name,
                uid,
                gid,
                home,
                shell,
            },
            session,
            foreign,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UserInfo {
        UserInfo {
            name: "sergio".into(),
            uid: 1000,
            gid: 1000,
            home: PathBuf::from("/home/sergio"),
            shell: PathBuf::from("/usr/bin/bash"),
        }
    }

    #[test]
    fn round_trip_without_session() {
        let t = SessionTicket::new(sample());
        let back = SessionTicket::from_line(&t.to_line()).expect("parsea");
        assert_eq!(back, t);
        assert!(back.session.is_empty());
    }

    #[test]
    fn round_trip_with_session() {
        let t = SessionTicket::new(sample()).with_session("shuma-shell --launcher");
        let back = SessionTicket::from_line(&t.to_line()).expect("parsea");
        assert_eq!(back, t);
        assert_eq!(back.session, "shuma-shell --launcher");
    }

    #[test]
    fn round_trip_with_foreign() {
        let t = SessionTicket::new(sample())
            .with_session("sway")
            .foreign(true);
        let back = SessionTicket::from_line(&t.to_line()).expect("parsea");
        assert_eq!(back, t);
        assert!(back.foreign);
    }

    #[test]
    fn foreign_defaults_false_sin_campo() {
        // Una línea estilo v1 (sin el campo foreign) parsea con foreign=false.
        let line = format!("{TICKET_TAG}\tsergio\t1000\t1000\t/home/sergio\t/usr/bin/bash\tsway");
        let back = SessionTicket::from_line(&line).expect("parsea");
        assert!(!back.foreign);
        assert_eq!(back.session, "sway");
    }

    #[test]
    fn from_line_ignores_non_ticket() {
        assert!(SessionTicket::from_line("[INFO] arrancando greeter").is_none());
        assert!(SessionTicket::from_line("").is_none());
    }

    #[test]
    fn from_line_rejects_malformed() {
        // Prefijo correcto pero faltan campos.
        assert!(SessionTicket::from_line(&format!("{TICKET_TAG}\tsergio")).is_none());
        // uid no numérico.
        assert!(
            SessionTicket::from_line(&format!("{TICKET_TAG}\tsergio\tXX\t1000\t/h\t/sh\t"))
                .is_none()
        );
    }

    #[test]
    fn tolerates_trailing_newline() {
        let line = format!("{}\n", SessionTicket::new(sample()).to_line());
        assert!(SessionTicket::from_line(&line).is_some());
    }
}
