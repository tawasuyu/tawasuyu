//! `Container` — la especificación declarativa de un contenedor Docker.
//!
//! Es sólo el *deseo*: qué imagen, qué puertos, qué entorno. Ejecutar
//! Docker es trabajo de capas superiores; aquí el contenedor es un dato
//! comparable (`PartialEq`) para que el plan detecte cambios.

use serde::{Deserialize, Serialize};

/// Política de reinicio del contenedor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    /// Nunca reiniciar.
    #[default]
    No,
    /// Reiniciar sólo si salió con error.
    OnFailure,
    /// Reiniciar siempre.
    Always,
    /// Reiniciar salvo que se haya detenido a mano.
    UnlessStopped,
}

impl RestartPolicy {
    /// Valor tal como lo espera el flag `--restart` de Docker.
    pub fn docker_flag(self) -> &'static str {
        match self {
            RestartPolicy::No => "no",
            RestartPolicy::OnFailure => "on-failure",
            RestartPolicy::Always => "always",
            RestartPolicy::UnlessStopped => "unless-stopped",
        }
    }
}

/// Un mapeo de puerto `host → contenedor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMap {
    pub host: u16,
    pub container: u16,
}

impl PortMap {
    pub fn new(host: u16, container: u16) -> Self {
        Self { host, container }
    }
}

/// La especificación declarativa de un contenedor. Clave única: `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Container {
    pub name: String,
    /// Imagen con etiqueta — `"nginx:1.27"`, `"postgres:16"`.
    pub image: String,
    pub ports: Vec<PortMap>,
    /// Variables de entorno, ordenadas por clave para comparación estable.
    pub env: Vec<(String, String)>,
    /// Volúmenes `ruta_host → ruta_contenedor`.
    pub volumes: Vec<(String, String)>,
    pub restart: RestartPolicy,
}

impl Container {
    /// Contenedor mínimo: nombre + imagen.
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            ports: Vec::new(),
            env: Vec::new(),
            volumes: Vec::new(),
            restart: RestartPolicy::default(),
        }
    }

    /// Publica un puerto (encadenable).
    pub fn with_port(mut self, host: u16, container: u16) -> Self {
        self.ports.push(PortMap::new(host, container));
        self
    }

    /// Define una variable de entorno (encadenable). El vector se
    /// mantiene ordenado por clave para que dos contenedores con el
    /// mismo entorno comparen iguales sin importar el orden de llamada.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        self.env.retain(|(k, _)| k != &key);
        self.env.push((key, value.into()));
        self.env.sort_by(|a, b| a.0.cmp(&b.0));
        self
    }

    /// Monta un volumen (encadenable).
    pub fn with_volume(
        mut self,
        host_path: impl Into<String>,
        container_path: impl Into<String>,
    ) -> Self {
        self.volumes.push((host_path.into(), container_path.into()));
        self
    }

    /// Fija la política de reinicio (encadenable).
    pub fn with_restart(mut self, restart: RestartPolicy) -> Self {
        self.restart = restart;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_order_does_not_affect_equality() {
        let a = Container::new("c", "img").with_env("B", "2").with_env("A", "1");
        let b = Container::new("c", "img").with_env("A", "1").with_env("B", "2");
        assert_eq!(a, b);
    }

    #[test]
    fn with_env_overwrites_same_key() {
        let c = Container::new("c", "img").with_env("K", "old").with_env("K", "new");
        assert_eq!(c.env, vec![("K".to_string(), "new".to_string())]);
    }

    #[test]
    fn restart_flags_match_docker() {
        assert_eq!(RestartPolicy::UnlessStopped.docker_flag(), "unless-stopped");
        assert_eq!(RestartPolicy::default(), RestartPolicy::No);
    }
}
