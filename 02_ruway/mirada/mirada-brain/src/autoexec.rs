//! `autoexec` — apps de **arranque por vista**, con vida opcional **efímera**.
//!
//! Cada [`Vista`](crate::vistas::Vista) puede declarar comandos que se lanzan al
//! aplicarla. Un comando **efímero** *pertenece* a la vista: cuando se cambia de
//! vista, el compositor lo termina (`SIGTERM`). Uno **persistente** sobrevive al
//! cambio (se lanza una vez y queda).
//!
//! El uso que lo motivó: traer de vuelta **Windows 3.1** como guiño — su
//! *Program Manager* sería una app cliente real (una ventana movible, no la
//! barra), declarada como autoexec **efímero** de esa vista: aparece con ella y
//! se va al salir.
//!
//! Este módulo es la **política pura** (qué matar / qué lanzar); los efectos
//! (spawnear, trackear PIDs, matar) los hace el compositor, que es dueño de los
//! procesos hijos.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Un comando de arranque de una vista.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoExec {
    /// La línea de comando (se pasa a `sh -c`).
    pub command: String,
    /// `true` = **efímero**: el compositor lo termina al cambiar de vista (la app
    /// pertenece a la vista, como el Program Manager de Win3.1). `false`
    /// (default) = persistente: sobrevive al cambio.
    #[serde(default)]
    pub ephemeral: bool,
}

impl AutoExec {
    /// Un comando persistente (sobrevive al cambio de vista).
    pub fn persistent(command: impl Into<String>) -> Self {
        Self { command: command.into(), ephemeral: false }
    }
    /// Un comando efímero (muere al cambiar de vista).
    pub fn ephemeral(command: impl Into<String>) -> Self {
        Self { command: command.into(), ephemeral: true }
    }
}

/// Decide, dado lo que el compositor **ya lanzó** (`running`: comando → ¿es
/// efímero?) y el autoexec `desired` de la vista actual, **qué efímeros terminar**
/// y **qué comandos lanzar**.
///
/// Reglas:
/// - **No relanza** lo que ya está en `running` aunque el usuario lo haya cerrado
///   a mano — respeta el cierre manual dentro de la misma sesión de vista.
/// - **Mata** sólo los efímeros que ya **no** están en el autoexec nuevo (cambio
///   de vista). Los persistentes nunca se matan.
/// - **Lanza** los del autoexec nuevo que no estén ya corriendo.
///
/// Pura y testeable; el compositor ejecuta el plan (spawn/SIGTERM).
pub fn autoexec_plan<'a>(
    running: &HashMap<String, bool>,
    desired: &'a [AutoExec],
) -> (Vec<String>, Vec<&'a AutoExec>) {
    let want: HashSet<&str> = desired.iter().map(|a| a.command.as_str()).collect();
    let kill: Vec<String> = running
        .iter()
        .filter(|(cmd, &eph)| eph && !want.contains(cmd.as_str()))
        .map(|(cmd, _)| cmd.clone())
        .collect();
    let launch: Vec<&AutoExec> =
        desired.iter().filter(|a| !running.contains_key(&a.command)).collect();
    (kill, launch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(pairs: &[(&str, bool)]) -> HashMap<String, bool> {
        pairs.iter().map(|(c, e)| (c.to_string(), *e)).collect()
    }

    #[test]
    fn lanza_lo_que_falta_y_no_relanza_lo_que_ya_corre() {
        let run = running(&[("pm", true)]);
        let desired = vec![AutoExec::ephemeral("pm"), AutoExec::persistent("dock")];
        let (kill, launch) = autoexec_plan(&run, &desired);
        assert!(kill.is_empty(), "nada que matar: pm sigue deseado");
        assert_eq!(launch.len(), 1);
        assert_eq!(launch[0].command, "dock", "sólo lanza lo nuevo");
    }

    #[test]
    fn al_cambiar_de_vista_mata_los_efimeros_que_se_fueron() {
        // Estaba corriendo el PM (efímero) y un dock (persistente). La vista nueva
        // no los incluye: el PM se mata, el dock sobrevive.
        let run = running(&[("pm", true), ("dock", false)]);
        let desired = vec![AutoExec::ephemeral("front-panel")];
        let (kill, launch) = autoexec_plan(&run, &desired);
        assert_eq!(kill, vec!["pm".to_string()], "sólo el efímero muere");
        assert_eq!(launch.len(), 1);
        assert_eq!(launch[0].command, "front-panel");
    }

    #[test]
    fn no_relanza_un_efimero_cerrado_a_mano_en_la_misma_vista() {
        // El usuario cerró el PM, pero sigue en `running` (el compositor no lo
        // saca hasta el cambio de vista). Re-aplicar la misma vista NO lo relanza.
        let run = running(&[("pm", true)]);
        let desired = vec![AutoExec::ephemeral("pm")];
        let (kill, launch) = autoexec_plan(&run, &desired);
        assert!(kill.is_empty() && launch.is_empty(), "idempotente: ni mata ni relanza");
    }
}
