//! `matilda-discover` — qué hay realmente en el servidor.
//!
//! Para reconciliar de verdad hace falta saber el estado *actual*: qué
//! contenedores y vhosts existen. Este crate lo observa y lo reconstruye
//! como un [`Inventory`] que `matilda-plan` puede diferenciar contra el
//! deseado.
//!
//! Alcance v1: descubre por **nombre**. Detecta correctamente lo que hay
//! que **crear** y lo que hay que **eliminar** (huérfanos). No detecta
//! cambios de configuración de un recurso existente — eso necesita
//! inspección detallada (`docker inspect`), aún no implementada; un
//! recurso presente y deseado se asume sin cambios.
//!
//! El parseo es puro y testeable; sólo [`discover_local`] toca el sistema.

#![forbid(unsafe_code)]

use matilda_core::{Container, Inventory, VHost};
use serde::{Deserialize, Serialize};

/// El estado observado de un servidor — los nombres de lo que existe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerState {
    /// Nombres de los contenedores presentes.
    pub containers: Vec<String>,
    /// Dominios de los vhosts presentes.
    pub vhosts: Vec<String>,
}

/// Parsea la salida de `docker ps -a --format '{{.Names}}'` — un nombre
/// por línea.
pub fn parse_docker_names(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parsea un listado de `/etc/nginx/sites-enabled` — un archivo por
/// línea; el sufijo `.conf` se quita para quedarse con el dominio.
pub fn parse_nginx_sites(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.strip_suffix(".conf").unwrap_or(l).to_string())
        .collect()
}

/// Reconstruye el inventario "actual" a partir de los nombres observados.
///
/// Un recurso presente que también está en `desired` se copia de ahí —
/// así el `plan` no marca cambios espurios (la detección real de drift
/// necesita inspección detallada). Un recurso presente que **no** está
/// en `desired` entra como un marcador, y el `plan` lo verá como un
/// `Remove`.
pub fn observed_inventory(state: &ServerState, desired: &Inventory) -> Inventory {
    let mut inv = Inventory::new();
    for name in &state.containers {
        match desired.container(name) {
            Some(c) => inv.add_container(c.clone()),
            None => inv.add_container(Container::new(name, "(desconocido)")),
        }
    }
    for domain in &state.vhosts {
        match desired.vhost(domain) {
            Some(v) => inv.add_vhost(v.clone()),
            None => inv.add_vhost(VHost::to_address(domain, "(desconocido)")),
        }
    }
    inv
}

/// Ejecuta un comando local y devuelve su stdout, o `None` si falla.
fn run_local(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Observa el estado de *esta* máquina: `docker ps` + los sitios de
/// nginx. Si docker no está o el directorio no existe, esa parte queda
/// vacía (no es un error — quizá el servidor aún no tiene nada).
pub fn discover_local() -> ServerState {
    let containers = run_local("docker", &["ps", "-a", "--format", "{{.Names}}"])
        .map(|t| parse_docker_names(&t))
        .unwrap_or_default();
    let vhosts = run_local("ls", &["-1", "/etc/nginx/sites-enabled"])
        .map(|t| parse_nginx_sites(&t))
        .unwrap_or_default();
    ServerState { containers, vhosts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matilda_plan::{plan, Op};

    #[test]
    fn parses_docker_names() {
        let names = parse_docker_names("web\napi\n\n  db  \n");
        assert_eq!(names, vec!["web", "api", "db"]);
    }

    #[test]
    fn parses_nginx_sites_stripping_conf() {
        let sites = parse_nginx_sites("sitio.com.conf\napi.sitio.com.conf\n");
        assert_eq!(sites, vec!["sitio.com", "api.sitio.com"]);
    }

    #[test]
    fn observed_present_and_desired_diffs_clean() {
        // Un contenedor presente que también se desea → sin cambios.
        let mut desired = Inventory::new();
        desired.add_container(Container::new("web", "nginx:1.27"));
        let state = ServerState { containers: vec!["web".into()], vhosts: vec![] };
        let current = observed_inventory(&state, &desired);
        let p = plan(&current, &desired);
        assert!(p.is_empty(), "presente y deseado → sin acciones");
    }

    #[test]
    fn observed_orphan_becomes_a_removal() {
        // Un contenedor presente que NO se desea → se elimina.
        let desired = Inventory::new();
        let state = ServerState { containers: vec!["viejo".into()], vhosts: vec![] };
        let current = observed_inventory(&state, &desired);
        let p = plan(&current, &desired);
        assert_eq!(p.count(Op::Remove), 1);
        assert_eq!(p.actions[0].name, "viejo");
    }

    #[test]
    fn missing_desired_resource_becomes_a_creation() {
        let mut desired = Inventory::new();
        desired.add_container(Container::new("nuevo", "img:1"));
        // El servidor no tiene nada.
        let current = observed_inventory(&ServerState::default(), &desired);
        let p = plan(&current, &desired);
        assert_eq!(p.count(Op::Create), 1);
    }

    #[test]
    fn create_and_remove_together() {
        let mut desired = Inventory::new();
        desired.add_container(Container::new("nuevo", "img:1"));
        let state = ServerState { containers: vec!["viejo".into()], vhosts: vec![] };
        let p = plan(&observed_inventory(&state, &desired), &desired);
        assert_eq!(p.count(Op::Create), 1);
        assert_eq!(p.count(Op::Remove), 1);
    }
}
