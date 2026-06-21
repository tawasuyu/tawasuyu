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

use matilda_core::{Container, Inventory, Service, VHost};
use serde::{Deserialize, Serialize};

/// Estado declarativo observado de un servicio systemd administrado:
/// su unidad y si está habilitado/activo *ahora*. A diferencia de los
/// contenedores, sólo se observan los servicios **declarados** (matilda no
/// administra las cientos de unidades del sistema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedService {
    pub unit: String,
    pub enabled: bool,
    pub active: bool,
}

/// El estado observado de un servidor — los nombres de lo que existe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerState {
    /// Nombres de los contenedores presentes.
    pub containers: Vec<String>,
    /// Dominios de los vhosts presentes.
    pub vhosts: Vec<String>,
    /// Estado declarativo de los servicios administrados (sólo los
    /// declarados; vacío en el discover remoto v1).
    #[serde(default)]
    pub services: Vec<ObservedService>,
}

/// Estado de ejecución observado de un contenedor — el campo `{{.State}}`
/// de Docker, normalizado. Lo que distingue "monitoreo" de "inventario":
/// no *qué debería haber* sino *qué está pasando ahora*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    Created,
    Restarting,
    Running,
    Paused,
    Exited,
    Dead,
    Unknown,
}

impl RunState {
    /// Mapea el `{{.State}}` de Docker/Podman a la variante.
    pub fn from_docker(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "created" => RunState::Created,
            "restarting" => RunState::Restarting,
            "running" | "up" => RunState::Running,
            "paused" => RunState::Paused,
            "exited" | "stopped" => RunState::Exited,
            "dead" => RunState::Dead,
            _ => RunState::Unknown,
        }
    }

    /// `true` si el contenedor está vivo (corriendo o reiniciándose).
    pub fn is_up(self) -> bool {
        matches!(self, RunState::Running | RunState::Restarting)
    }

    /// Glifo de semáforo para la UI: ● vivo, ◐ transición, ○ parado.
    pub fn glyph(self) -> char {
        match self {
            RunState::Running => '●',
            RunState::Restarting | RunState::Paused | RunState::Created => '◐',
            RunState::Exited | RunState::Dead => '○',
            RunState::Unknown => '◌',
        }
    }
}

/// Estado runtime observado de un contenedor — la fila de `docker ps`
/// rica (no sólo el nombre). Es la unidad del monitoreo en vivo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerStatus {
    pub name: String,
    pub image: String,
    pub state: RunState,
    /// Texto crudo de Docker: `Up 2 hours`, `Exited (0) 3 days ago`.
    pub status: String,
    /// Mapeos de puerto tal como los reporta Docker.
    pub ports: String,
}

/// Estado `ACTIVE` de un servicio systemd, normalizado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceState {
    Active,
    Inactive,
    Activating,
    Deactivating,
    Failed,
    Unknown,
}

impl ServiceState {
    pub fn from_systemd(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "active" => ServiceState::Active,
            "inactive" => ServiceState::Inactive,
            "activating" => ServiceState::Activating,
            "deactivating" => ServiceState::Deactivating,
            "failed" => ServiceState::Failed,
            _ => ServiceState::Unknown,
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, ServiceState::Active | ServiceState::Activating)
    }

    /// Glifo de semáforo: ● activo, ◐ transición, ✖ fallado, ○ parado.
    pub fn glyph(self) -> char {
        match self {
            ServiceState::Active => '●',
            ServiceState::Activating | ServiceState::Deactivating => '◐',
            ServiceState::Failed => '✖',
            ServiceState::Inactive => '○',
            ServiceState::Unknown => '◌',
        }
    }
}

/// Estado runtime de un servicio systemd (una fila de `systemctl
/// list-units --type=service`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceStatus {
    /// Nombre de la unidad — `sshd.service`.
    pub name: String,
    pub state: ServiceState,
    /// El campo `SUB` de systemd: `running`, `exited`, `dead`, `failed`.
    pub sub: String,
    pub description: String,
}

/// Foto runtime del servidor: contenedores + servicios + vhosts.
/// Distinta del `Inventory` declarativo — esto es lo *observado vivo*.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeState {
    pub containers: Vec<ContainerStatus>,
    pub services: Vec<ServiceStatus>,
    pub vhosts: Vec<String>,
}

impl RuntimeState {
    /// Cuenta de contenedores vivos.
    pub fn up_count(&self) -> usize {
        self.containers.iter().filter(|c| c.state.is_up()).count()
    }

    /// Cuenta de contenedores parados/muertos.
    pub fn down_count(&self) -> usize {
        self.containers.iter().filter(|c| !c.state.is_up()).count()
    }

    /// Busca el estado runtime de un contenedor por nombre.
    pub fn container(&self, name: &str) -> Option<&ContainerStatus> {
        self.containers.iter().find(|c| c.name == name)
    }

    /// Cuenta de servicios activos.
    pub fn services_active(&self) -> usize {
        self.services.iter().filter(|s| s.state.is_active()).count()
    }

    /// Cuenta de servicios fallados.
    pub fn services_failed(&self) -> usize {
        self.services
            .iter()
            .filter(|s| s.state == ServiceState::Failed)
            .count()
    }
}

/// Parsea `systemctl list-units --type=service --no-legend --plain`: una
/// fila `UNIT LOAD ACTIVE SUB DESCRIPTION…` por servicio. La descripción
/// (resto de la línea) puede tener espacios. Puro y testeable.
pub fn parse_systemctl_units(text: &str) -> Vec<ServiceStatus> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut f = line.split_whitespace();
            let name = f.next()?.to_string();
            let _load = f.next()?;
            let active = f.next()?;
            let sub = f.next()?.to_string();
            let description = f.collect::<Vec<_>>().join(" ");
            Some(ServiceStatus {
                name,
                state: ServiceState::from_systemd(active),
                sub,
                description,
            })
        })
        .collect()
}

/// Formato rico que pedimos a Docker/Podman para el monitoreo: una fila
/// tab-separada por contenedor. Reutilizable por el discover local y el
/// remoto (SSH).
pub const DOCKER_PS_FORMAT: &str = "{{.Names}}\t{{.Image}}\t{{.State}}\t{{.Status}}\t{{.Ports}}";

/// Parsea la salida de `docker ps -a --format DOCKER_PS_FORMAT`: una fila
/// tab-separada por contenedor. Tolera campos faltantes (los rellena
/// vacíos) y descarta líneas sin nombre. Puro y testeable.
pub fn parse_docker_ps(text: &str) -> Vec<ContainerStatus> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.trim().is_empty() {
                return None;
            }
            let mut f = line.split('\t');
            let name = f.next()?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let image = f.next().unwrap_or("").trim().to_string();
            let state = RunState::from_docker(f.next().unwrap_or(""));
            let status = f.next().unwrap_or("").trim().to_string();
            let ports = f.next().unwrap_or("").trim().to_string();
            Some(ContainerStatus { name, image, state, status, ports })
        })
        .collect()
}

/// Muestra de uso de un contenedor: CPU y memoria como porcentaje. La unidad
/// del histograma CPU/mem del monitoreo (M2). Numérico (no el texto crudo de
/// Docker) para alimentar la sparkline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContainerStats {
    pub cpu_pct: f32,
    pub mem_pct: f32,
}

/// Formato que pedimos a `docker stats --no-stream` para el muestreo: nombre
/// + CPU% + MEM%. Tab-separado, reutilizable local y remoto (SSH).
pub const DOCKER_STATS_FORMAT: &str = "{{.Name}}\t{{.CPUPerc}}\t{{.MemPerc}}";

/// Parsea un porcentaje de Docker (`12.34%`, `0.00%`) a `f32`. Tolera el
/// sufijo `%`, espacios y `--` (sin dato → 0.0).
fn parse_percent(s: &str) -> f32 {
    s.trim().trim_end_matches('%').trim().parse::<f32>().unwrap_or(0.0)
}

/// Parsea la salida de `docker stats --no-stream --format DOCKER_STATS_FORMAT`:
/// una fila `nombre<TAB>cpu%<TAB>mem%` por contenedor. Devuelve un mapa
/// `nombre → ContainerStats`. Puro y testeable.
pub fn parse_docker_stats(text: &str) -> std::collections::BTreeMap<String, ContainerStats> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.trim().is_empty() {
                return None;
            }
            let mut f = line.split('\t');
            let name = f.next()?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let cpu_pct = parse_percent(f.next().unwrap_or(""));
            let mem_pct = parse_percent(f.next().unwrap_or(""));
            Some((name, ContainerStats { cpu_pct, mem_pct }))
        })
        .collect()
}

/// Observa el uso CPU/mem de los contenedores corriendo en *esta* máquina
/// (`docker stats --no-stream`). Vacío si docker no está. Bloqueante (~1-2 s:
/// docker muestrea un intervalo), pensado para correr en un thread de polling.
pub fn discover_stats() -> std::collections::BTreeMap<String, ContainerStats> {
    run_local(
        "docker",
        &["stats", "--no-stream", "--format", DOCKER_STATS_FORMAT],
    )
    .map(|t| parse_docker_stats(&t))
    .unwrap_or_default()
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
    // Servicios: sólo los declarados (matilda no administra todo systemd).
    // Reflejamos su estado observado → el plan emite Update si difiere del
    // deseado, o nada si coincide.
    for svc in &state.services {
        inv.add_service(
            Service::new(svc.unit.as_str())
                .with_enabled(svc.enabled)
                .with_active(svc.active),
        );
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
    // Servicios: sólo los declarados — consultamos su estado actual
    // (`is-enabled`/`is-active`) para que el plan emita Update si difieren.
    for svc in desired.services() {
        let (enabled, active) = service_actual_state(&svc.unit);
        inv.add_service(
            Service::new(svc.unit.as_str())
                .with_enabled(enabled)
                .with_active(active),
        );
    }
    inv
}

/// Consulta el estado actual de un servicio systemd: `(enabled, active)`.
/// `systemctl is-enabled`/`is-active` salen con código 0 sólo cuando lo
/// están; si systemctl no existe, ambos son `false`.
fn service_actual_state(unit: &str) -> (bool, bool) {
    let enabled = run_local("systemctl", &["is-enabled", unit])
        .map(|s| s.trim() == "enabled")
        .unwrap_or(false);
    let active = run_local("systemctl", &["is-active", unit])
        .map(|s| s.trim() == "active")
        .unwrap_or(false);
    (enabled, active)
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
    ServerState { containers, vhosts, services: Vec::new() }
}

/// Observa el estado **runtime** de esta máquina: `docker ps -a` con el
/// formato rico (estado + status + puertos) y los sitios de nginx. Es la
/// fuente del monitoreo en vivo del bloque de matilda. Si docker no está,
/// la lista de contenedores queda vacía (no es error).
pub fn discover_runtime() -> RuntimeState {
    let containers = run_local("docker", &["ps", "-a", "--format", DOCKER_PS_FORMAT])
        .map(|t| parse_docker_ps(&t))
        .unwrap_or_default();
    let services = discover_services();
    let vhosts = run_local("ls", &["-1", "/etc/nginx/sites-enabled"])
        .map(|t| parse_nginx_sites(&t))
        .unwrap_or_default();
    RuntimeState { containers, services, vhosts }
}

/// Observa los servicios systemd **operativamente interesantes**: los que
/// están corriendo o fallaron (no las cientos de unidades inactivas). Es
/// la base del monitoreo de servicios. Vacío si no hay systemctl.
pub fn discover_services() -> Vec<ServiceStatus> {
    run_local(
        "systemctl",
        &[
            "list-units",
            "--type=service",
            "--state=running,failed",
            "--no-legend",
            "--plain",
        ],
    )
    .map(|t| parse_systemctl_units(&t))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use matilda_plan::{plan, Op};

    #[test]
    fn parse_docker_ps_rico() {
        let text = "web\tnginx:1.27\trunning\tUp 2 hours\t0.0.0.0:80->80/tcp\n\
                    db\tpostgres:16\texited\tExited (0) 3 days ago\t\n";
        let cs = parse_docker_ps(text);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].name, "web");
        assert_eq!(cs[0].state, RunState::Running);
        assert!(cs[0].state.is_up());
        assert_eq!(cs[0].status, "Up 2 hours");
        assert_eq!(cs[0].ports, "0.0.0.0:80->80/tcp");
        assert_eq!(cs[1].state, RunState::Exited);
        assert!(!cs[1].state.is_up());
        // Campos faltantes (sin ports) no rompen el parseo.
        assert_eq!(cs[1].ports, "");
    }

    #[test]
    fn parse_docker_stats_porcentajes() {
        let text = "web\t12.34%\t5.67%\ndb\t0.00%\t40.10%\nbad\t--\t--\n";
        let m = parse_docker_stats(text);
        assert_eq!(m.len(), 3);
        assert!((m["web"].cpu_pct - 12.34).abs() < 0.01);
        assert!((m["web"].mem_pct - 5.67).abs() < 0.01);
        assert_eq!(m["db"].cpu_pct, 0.0);
        // `--` (sin dato) cae a 0.0 sin romper.
        assert_eq!(m["bad"].cpu_pct, 0.0);
        assert_eq!(m["bad"].mem_pct, 0.0);
    }

    #[test]
    fn runtime_state_cuenta_up_down() {
        let rs = RuntimeState {
            containers: vec![
                ContainerStatus {
                    name: "a".into(),
                    image: "x".into(),
                    state: RunState::Running,
                    status: "Up".into(),
                    ports: String::new(),
                },
                ContainerStatus {
                    name: "b".into(),
                    image: "y".into(),
                    state: RunState::Exited,
                    status: "Exited".into(),
                    ports: String::new(),
                },
                ContainerStatus {
                    name: "c".into(),
                    image: "z".into(),
                    state: RunState::Restarting,
                    status: "Restarting".into(),
                    ports: String::new(),
                },
            ],
            services: vec![],
            vhosts: vec![],
        };
        assert_eq!(rs.up_count(), 2); // running + restarting
        assert_eq!(rs.down_count(), 1);
        assert_eq!(rs.container("b").unwrap().state, RunState::Exited);
        assert!(rs.container("nope").is_none());
    }

    #[test]
    fn parse_systemctl_units_y_conteos() {
        let text = "sshd.service        loaded active running OpenSSH server daemon\n\
                    nginx.service       loaded active running A high performance web server\n\
                    backup.service      loaded failed failed  Nightly backup\n";
        let svcs = parse_systemctl_units(text);
        assert_eq!(svcs.len(), 3);
        assert_eq!(svcs[0].name, "sshd.service");
        assert_eq!(svcs[0].state, ServiceState::Active);
        assert_eq!(svcs[0].description, "OpenSSH server daemon");
        assert_eq!(svcs[2].state, ServiceState::Failed);
        let rs = RuntimeState { containers: vec![], services: svcs, vhosts: vec![] };
        assert_eq!(rs.services_active(), 2);
        assert_eq!(rs.services_failed(), 1);
    }

    #[test]
    fn run_state_glyphs_y_mapeo() {
        assert_eq!(RunState::from_docker("RUNNING"), RunState::Running);
        assert_eq!(RunState::from_docker("up"), RunState::Running);
        assert_eq!(RunState::from_docker("dead"), RunState::Dead);
        assert_eq!(RunState::from_docker("???"), RunState::Unknown);
        assert_eq!(RunState::Running.glyph(), '●');
        assert_eq!(RunState::Exited.glyph(), '○');
    }

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
        let state = ServerState { containers: vec!["web".into()], vhosts: vec![], services: vec![] };
        let current = observed_inventory(&state, &desired);
        let p = plan(&current, &desired);
        assert!(p.is_empty(), "presente y deseado → sin acciones");
    }

    #[test]
    fn observed_orphan_becomes_a_removal() {
        // Un contenedor presente que NO se desea → se elimina.
        let desired = Inventory::new();
        let state = ServerState { containers: vec!["viejo".into()], vhosts: vec![], services: vec![] };
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
        let state = ServerState { containers: vec!["viejo".into()], vhosts: vec![], services: vec![] };
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
