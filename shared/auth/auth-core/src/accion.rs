//! La acción que el *shell de credenciales* (greeter / lock) le entrega al
//! compositor por su stdout.
//!
//! Generaliza al [`SessionTicket`]: el greeter ya no sólo dice «arrancá esta
//! sesión», sino que el mismo canal sirve para el **lock** y, más adelante,
//! para el *fast user switching* (saltar entre sesiones). El compositor corre
//! el shell como proceso hijo y escanea su stdout; cada línea que matchea una
//! de las etiquetas de aquí es una [`ShellAction`], el resto es ruido (logs).
//!
//! Por qué un enum y no sólo el ticket: el compositor hostea N sesiones (hoy
//! 0..1) y muestra el shell *encima* de ellas. El shell necesita poder pedir
//! cosas distintas — arrancar una sesión nueva, **desbloquear** la activa, o
//! cancelar — sin que cada una sea un canal aparte. Es el seam que deja crecer
//! a multisesión sin reescribir el contrato greeter↔compositor.

use crate::SessionTicket;

/// Etiqueta de la acción «desbloquear la sesión activa».
pub const UNLOCK_TAG: &str = "MIRADA-SHELL-UNLOCK-v1";
/// Etiqueta de la acción «cancelar el shell sin hacer nada».
pub const CANCEL_TAG: &str = "MIRADA-SHELL-CANCEL-v1";
/// Prefijo de la acción «saltar a la sesión hosteada con este id». Lleva el id
/// de sesión (un `u32`, el `SessionId` del roster del compositor) separado por
/// un espacio.
pub const SWITCH_TAG: &str = "MIRADA-SHELL-SWITCH-v1";
/// Etiqueta de la acción «abrir el login para hostear una sesión nueva».
pub const NEWSESSION_TAG: &str = "MIRADA-SHELL-NEWSESSION-v1";

/// Lo que el shell de credenciales le pide al compositor.
///
/// El *fast user switching* (FUS) cableó dos acciones más sobre el mismo canal:
/// [`SwitchTo`](ShellAction::SwitchTo) (saltar a otra sesión ya hosteada) y
/// [`NewSession`](ShellAction::NewSession) (volver al login para hostear una
/// sesión nueva sin tirar la actual). Por eso el canal es un enum y no un
/// [`SessionTicket`] pelado: crece sin reescribir el contrato greeter↔compositor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellAction {
    /// Autenticación de login exitosa: arrancá/elegí la sesión del ticket.
    /// Es el camino del greeter (0 sesiones → 1 sesión, o una sesión más en FUS).
    StartSession(SessionTicket),
    /// El lock se resolvió con la contraseña correcta: volvé a la sesión
    /// activa. Sin destino explícito (el compositor sabe cuál es la activa);
    /// para volver a *otra* sesión existe [`SwitchTo`](ShellAction::SwitchTo).
    Unlock,
    /// FUS: saltar a la sesión hosteada con este id (la elegida en el selector
    /// «cambiar usuario» del lock). El id es el `SessionId` del roster del
    /// compositor.
    SwitchTo(u32),
    /// FUS: «cambiar usuario» abriendo el login para hostear una sesión nueva.
    /// La sesión actual queda residente (suspendida) debajo; el compositor
    /// vuelve a modo greeter y, tras el login, le da de alta otra sesión.
    NewSession,
    /// Cerrá el shell sin acción (reservado: salida del lock sin desbloquear).
    Cancel,
}

impl ShellAction {
    /// Serializa la acción a una línea única apta para stdout. `StartSession`
    /// reusa la línea del [`SessionTicket`] — así un ticket «pelado» (formato
    /// viejo) sigue siendo una acción válida y nada que ya emita tickets se
    /// rompe.
    pub fn to_line(&self) -> String {
        match self {
            ShellAction::StartSession(t) => t.to_line(),
            ShellAction::Unlock => UNLOCK_TAG.to_string(),
            ShellAction::SwitchTo(id) => format!("{SWITCH_TAG} {id}"),
            ShellAction::NewSession => NEWSESSION_TAG.to_string(),
            ShellAction::Cancel => CANCEL_TAG.to_string(),
        }
    }

    /// Parsea una línea producida por [`to_line`]. `None` si la línea no es una
    /// acción del shell (cualquier otra salida del hijo). Un ticket bien
    /// formado se interpreta como [`StartSession`](ShellAction::StartSession),
    /// conservando compatibilidad con el canal anterior.
    pub fn from_line(line: &str) -> Option<ShellAction> {
        if let Some(ticket) = SessionTicket::from_line(line) {
            return Some(ShellAction::StartSession(ticket));
        }
        let line = line.trim_end_matches(['\r', '\n']);
        // `SWITCH_TAG <id>`: el destino del salto va tras un espacio.
        if let Some(rest) = line.strip_prefix(SWITCH_TAG) {
            return rest.trim().parse::<u32>().ok().map(ShellAction::SwitchTo);
        }
        match line {
            UNLOCK_TAG => Some(ShellAction::Unlock),
            NEWSESSION_TAG => Some(ShellAction::NewSession),
            CANCEL_TAG => Some(ShellAction::Cancel),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UserInfo;
    use std::path::PathBuf;

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
    fn round_trip_start_session() {
        let a = ShellAction::StartSession(SessionTicket::new(sample()).with_session("pata"));
        assert_eq!(ShellAction::from_line(&a.to_line()), Some(a));
    }

    #[test]
    fn round_trip_unlock_y_cancel() {
        assert_eq!(ShellAction::from_line(&ShellAction::Unlock.to_line()), Some(ShellAction::Unlock));
        assert_eq!(ShellAction::from_line(&ShellAction::Cancel.to_line()), Some(ShellAction::Cancel));
    }

    #[test]
    fn round_trip_switch_y_new_session() {
        for id in [0u32, 1, 42, u32::MAX] {
            let a = ShellAction::SwitchTo(id);
            assert_eq!(ShellAction::from_line(&a.to_line()), Some(a));
        }
        assert_eq!(
            ShellAction::from_line(&ShellAction::NewSession.to_line()),
            Some(ShellAction::NewSession)
        );
    }

    #[test]
    fn switch_con_id_invalido_es_ruido() {
        // Sin id, o con un id no numérico, la línea no es una acción válida.
        assert!(ShellAction::from_line(SWITCH_TAG).is_none());
        assert!(ShellAction::from_line(&format!("{SWITCH_TAG} abc")).is_none());
    }

    #[test]
    fn ticket_pelado_es_start_session() {
        // Una línea de ticket directa (canal viejo) parsea como StartSession.
        let line = SessionTicket::new(sample()).to_line();
        assert!(matches!(ShellAction::from_line(&line), Some(ShellAction::StartSession(_))));
    }

    #[test]
    fn ignora_ruido() {
        assert!(ShellAction::from_line("[INFO] arrancando").is_none());
        assert!(ShellAction::from_line("").is_none());
    }

    #[test]
    fn tolera_newline_final() {
        let line = format!("{UNLOCK_TAG}\n");
        assert_eq!(ShellAction::from_line(&line), Some(ShellAction::Unlock));
    }
}
