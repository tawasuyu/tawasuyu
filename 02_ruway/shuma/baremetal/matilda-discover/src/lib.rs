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

// --- Detección de drift por `docker inspect` ----------------------------

/// Subconjunto de la salida de `docker inspect` que importa para el drift.
#[derive(Debug, Deserialize)]
struct DockerInspect {
    #[serde(rename = "Config")]
    config: DockerConfig,
    #[serde(rename = "HostConfig")]
    host_config: DockerHostConfig,
}

#[derive(Debug, Deserialize)]
struct DockerConfig {
    #[serde(rename = "Image")]
    image: String,
    #[serde(default, rename = "Env")]
    env: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct DockerHostConfig {
    #[serde(default, rename = "Binds")]
    binds: Option<Vec<String>>,
    #[serde(default, rename = "PortBindings")]
    port_bindings: std::collections::HashMap<String, Option<Vec<PortBinding>>>,
    #[serde(default, rename = "RestartPolicy")]
    restart_policy: DockerRestart,
}

#[derive(Debug, Default, Deserialize)]
struct DockerRestart {
    #[serde(default, rename = "Name")]
    name: String,
}

#[derive(Debug, Deserialize)]
struct PortBinding {
    #[serde(rename = "HostPort")]
    host_port: String,
}

/// `true` si el contenedor que está corriendo **se desvió** de lo que
/// declara `desired` — distinta imagen, puerto, env o volumen.
///
/// La comparación es por *satisfacción*: lo que el spec declara debe
/// estar; lo extra que traiga la imagen (su `PATH`, etc.) se ignora.
/// Si el JSON no se puede leer, se asume que no hay drift (no se marca
/// un cambio espurio).
pub fn container_drift(desired: &Container, inspect_json: &str) -> bool {
    let parsed: Vec<DockerInspect> = match serde_json::from_str(inspect_json) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let Some(d) = parsed.first() else {
        return false;
    };

    // Imagen.
    if d.config.image != desired.image {
        return true;
    }
    // Política de reinicio (docker reporta "" cuando no hay → "no").
    let actual = if d.host_config.restart_policy.name.is_empty() {
        "no"
    } else {
        d.host_config.restart_policy.name.as_str()
    };
    if actual != desired.restart.docker_flag() {
        return true;
    }
    // Cada puerto declarado debe estar publicado al host correcto.
    for p in &desired.ports {
        let key = format!("{}/tcp", p.container);
        let published = d
            .host_config
            .port_bindings
            .get(&key)
            .and_then(|b| b.as_ref())
            .map(|bs| bs.iter().any(|b| b.host_port == p.host.to_string()))
            .unwrap_or(false);
        if !published {
            return true;
        }
    }
    // Cada variable de entorno declarada debe estar presente.
    for (k, v) in &desired.env {
        let want = format!("{k}={v}");
        if !d.config.env.iter().any(|e| e == &want) {
            return true;
        }
    }
    // Cada volumen declarado debe estar montado.
    for (h, c) in &desired.volumes {
        let want = format!("{h}:{c}");
        if !d.host_config.binds.iter().flatten().any(|b| b.starts_with(&want)) {
            return true;
        }
    }
    false
}

/// Descubre el inventario actual **con detección de drift**: corre
/// `docker inspect` en cada contenedor y, si se desvió del spec deseado,
/// lo marca para que el `plan` emita un `Update`. Los contenedores al
/// día se copian del deseado (sin cambio); los huérfanos quedan marcados
/// para `Remove`. Los vhosts se descubren por nombre.
pub fn discover_inventory(desired: &Inventory) -> Inventory {
    let mut inv = Inventory::new();
    let names = run_local("docker", &["ps", "-a", "--format", "{{.Names}}"])
        .map(|t| parse_docker_names(&t))
        .unwrap_or_default();
    for name in names {
        match desired.container(&name) {
            Some(d) => {
                let drifted = run_local("docker", &["inspect", &name])
                    .map(|json| container_drift(d, &json))
                    .unwrap_or(false);
                if drifted {
                    // Marcador distinto del deseado → el plan verá `Update`.
                    inv.add_container(Container::new(&name, "(desviado)"));
                } else {
                    inv.add_container(d.clone());
                }
            }
            None => inv.add_container(Container::new(&name, "(huérfano)")),
        }
    }
    for domain in run_local("ls", &["-1", "/etc/nginx/sites-enabled"])
        .map(|t| parse_nginx_sites(&t))
        .unwrap_or_default()
    {
        match desired.vhost(&domain) {
            Some(v) => inv.add_vhost(v.clone()),
            None => inv.add_vhost(VHost::to_address(&domain, "(huérfano)")),
        }
    }
    inv
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

    /// `docker inspect` de un `web` con nginx:1.27, 8080→80, un volumen,
    /// la env TZ y reinicio `always`.
    const INSPECT_WEB: &str = r#"[{
        "Config": {
            "Image": "nginx:1.27",
            "Env": ["PATH=/usr/local/sbin", "TZ=UTC"]
        },
        "HostConfig": {
            "Binds": ["/srv/web:/usr/share/nginx/html"],
            "PortBindings": {"80/tcp": [{"HostPort": "8080"}]},
            "RestartPolicy": {"Name": "always"}
        }
    }]"#;

    fn web_spec() -> matilda_core::Container {
        Container::new("web", "nginx:1.27")
            .with_port(8080, 80)
            .with_volume("/srv/web", "/usr/share/nginx/html")
            .with_env("TZ", "UTC")
            .with_restart(matilda_core::RestartPolicy::Always)
    }

    #[test]
    fn no_drift_when_running_matches_the_spec() {
        assert!(!container_drift(&web_spec(), INSPECT_WEB));
    }

    #[test]
    fn drift_when_image_changed() {
        let mut spec = web_spec();
        spec.image = "nginx:1.25".into();
        assert!(container_drift(&spec, INSPECT_WEB));
    }

    #[test]
    fn drift_when_a_declared_port_is_missing() {
        let spec = web_spec().with_port(9000, 9000);
        assert!(container_drift(&spec, INSPECT_WEB));
    }

    #[test]
    fn drift_when_a_declared_env_is_missing() {
        let spec = web_spec().with_env("DEBUG", "1");
        assert!(container_drift(&spec, INSPECT_WEB));
    }

    #[test]
    fn drift_when_restart_policy_differs() {
        let spec = web_spec().with_restart(matilda_core::RestartPolicy::No);
        assert!(container_drift(&spec, INSPECT_WEB));
    }

    #[test]
    fn unreadable_json_is_not_treated_as_drift() {
        assert!(!container_drift(&web_spec(), "no es json"));
    }
}
