//! Acciones de ciclo de vida **ad-hoc** sobre un contenedor existente —
//! operación viva, distinta de la reconciliación declarativa (plan/apply).
//!
//! Puro: cada acción se traduce a un comando de shell. Ejecutarlo (local
//! o por SSH) es trabajo de la capa de I/O (el bloque de shuma).

use serde::{Deserialize, Serialize};

/// Acción dirigida a un contenedor por nombre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
    /// Muestra las últimas líneas del log (lectura, no muta el contenedor).
    Logs,
    /// Detiene y elimina (`rm -f`).
    Remove,
}

impl ContainerAction {
    /// Etiqueta corta para el botón en la UI.
    pub fn label(self) -> &'static str {
        match self {
            ContainerAction::Start => "Start",
            ContainerAction::Stop => "Stop",
            ContainerAction::Restart => "Restart",
            ContainerAction::Logs => "Logs",
            ContainerAction::Remove => "Remove",
        }
    }

    /// `true` si la acción cambia el estado del contenedor (vs. sólo leer).
    /// El caller refresca el runtime después de una acción mutante.
    pub fn is_mutating(self) -> bool {
        !matches!(self, ContainerAction::Logs)
    }

    /// Comando de shell que ejecuta la acción sobre `name`. Puro.
    /// `name` se asume un nombre de contenedor válido (sin espacios); el
    /// caller no debe pasar entrada de usuario sin validar.
    pub fn command(self, name: &str) -> String {
        match self {
            ContainerAction::Start => format!("docker start {name}"),
            ContainerAction::Stop => format!("docker stop {name}"),
            ContainerAction::Restart => format!("docker restart {name}"),
            ContainerAction::Logs => format!("docker logs --tail 200 {name}"),
            ContainerAction::Remove => format!("docker rm -f {name}"),
        }
    }

    /// Todas las acciones, en el orden en que la UI las pinta.
    pub fn all() -> [ContainerAction; 5] {
        [
            ContainerAction::Start,
            ContainerAction::Stop,
            ContainerAction::Restart,
            ContainerAction::Logs,
            ContainerAction::Remove,
        ]
    }
}

/// Acción de ciclo de vida sobre un servicio systemd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
    /// Estado detallado (lectura, no muta).
    Status,
}

impl ServiceAction {
    pub fn label(self) -> &'static str {
        match self {
            ServiceAction::Start => "Start",
            ServiceAction::Stop => "Stop",
            ServiceAction::Restart => "Restart",
            ServiceAction::Enable => "Enable",
            ServiceAction::Disable => "Disable",
            ServiceAction::Status => "Status",
        }
    }

    pub fn is_mutating(self) -> bool {
        !matches!(self, ServiceAction::Status)
    }

    /// Comando `systemctl` para la acción sobre `unit`. Puro. Las acciones
    /// mutantes suelen requerir privilegios; si fallan, el caller lo loguea.
    pub fn command(self, unit: &str) -> String {
        match self {
            ServiceAction::Start => format!("systemctl start {unit}"),
            ServiceAction::Stop => format!("systemctl stop {unit}"),
            ServiceAction::Restart => format!("systemctl restart {unit}"),
            ServiceAction::Enable => format!("systemctl enable {unit}"),
            ServiceAction::Disable => format!("systemctl disable {unit}"),
            ServiceAction::Status => format!("systemctl status {unit} --no-pager --lines=20"),
        }
    }

    pub fn all() -> [ServiceAction; 6] {
        [
            ServiceAction::Start,
            ServiceAction::Stop,
            ServiceAction::Restart,
            ServiceAction::Enable,
            ServiceAction::Disable,
            ServiceAction::Status,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comandos_systemctl_por_accion() {
        assert_eq!(ServiceAction::Start.command("sshd"), "systemctl start sshd");
        assert_eq!(ServiceAction::Enable.command("nginx"), "systemctl enable nginx");
        assert!(ServiceAction::Status.command("x").contains("--no-pager"));
        assert!(!ServiceAction::Status.is_mutating());
        assert!(ServiceAction::Restart.is_mutating());
    }

    #[test]
    fn comandos_docker_por_accion() {
        assert_eq!(ContainerAction::Start.command("web"), "docker start web");
        assert_eq!(ContainerAction::Stop.command("web"), "docker stop web");
        assert_eq!(ContainerAction::Restart.command("web"), "docker restart web");
        assert_eq!(ContainerAction::Remove.command("web"), "docker rm -f web");
        assert!(ContainerAction::Logs.command("web").contains("docker logs"));
    }

    #[test]
    fn logs_no_muta_el_resto_si() {
        assert!(!ContainerAction::Logs.is_mutating());
        assert!(ContainerAction::Start.is_mutating());
        assert!(ContainerAction::Remove.is_mutating());
    }
}
