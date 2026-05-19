//! `shuma-card` — tipos del runtime shuma.
//!
//! Tres entidades nuevas encima del `brahman-card::Card`:
//!
//! - [`WorkspaceSpec`] — espacio aislado raíz con su propio `SomaSpec`.
//! - [`CommandRef`] — un comando dentro de un workspace.
//! - [`PipelineSpec`] — DAG de `CommandRef` conectados por `FlowEdge`.
//!
//! Cada `WorkspaceSpec`/`CommandRef` se **compila** a una o varias
//! [`brahman_card::Card`] que el daemon entrega al [`Incarnator`] de
//! `ente-incarnate`. Esto preserva el contrato canónico del fractal.

#![forbid(unsafe_code)]

use brahman_card::{Card, Payload, Permissions, SomaSpec, Supervision};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use ulid::Ulid;

// =====================================================================
// Identidades
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(pub Ulid);

impl WorkspaceId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for WorkspaceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipelineId(pub Ulid);

impl PipelineId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for PipelineId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PipelineId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// =====================================================================
// Workspace
// =====================================================================

/// Espacio aislado de shuma. Es la raíz de aislamiento — cualquier comando
/// que corre dentro hereda restricciones y no puede aflojarlas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSpec {
    pub label: String,

    /// Aislamiento del workspace mismo (cuando se materializa como Card raíz).
    #[serde(default)]
    pub soma: SomaSpec,

    /// Permisos máximos para hijas. Hijas pueden bajar pero no subir.
    #[serde(default)]
    pub permissions: Permissions,

    /// `None` = vive hasta `stop`. `Some(d)` = el daemon lo termina tras d.
    #[serde(default, with = "opt_duration_millis")]
    pub ttl: Option<Duration>,

    /// Slots de flow pre-declarados. Limitan qué consumidores externos al
    /// workspace pueden empatar contra los productores internos.
    #[serde(default)]
    pub flow_dirs: Vec<FlowSlot>,

    /// Política al terminar el workspace.
    #[serde(default)]
    pub on_exit: ExitPolicy,

    /// Política de enforcement automático cuando un recurso excede su
    /// rlimit declarado en `soma.rlimits`. Default = sólo accounting
    /// (None) — el quota report sigue funcionando, pero no hay kill.
    #[serde(default)]
    pub quota_enforce: QuotaEnforcement,
}

/// Acción cuando un recurso excede su límite. Aplica por recurso (mem,
/// nproc, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuotaAction {
    /// Sólo accounting: la breach aparece en `workspace_quota`.
    #[default]
    None,
    /// Loguear la breach (info-level del daemon).
    Log,
    /// Matar todos los comandos vivos del workspace (SIGKILL, sin grace).
    Kill,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuotaEnforcement {
    #[serde(default)]
    pub mem: QuotaAction,
    #[serde(default)]
    pub nproc: QuotaAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowSlot {
    pub name: String,
    pub direction: FlowDirection,
    /// Si `Workspace`, sólo otros nodos del mismo workspace pueden empatar.
    /// Si `Public`, el broker global puede emparejar.
    #[serde(default)]
    pub scope: FlowScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowScope {
    #[default]
    Workspace,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExitPolicy {
    /// Reapear procesos hijos y descartar estado.
    #[default]
    Reap,
    /// Mantener el workspace en `stopped` para inspección.
    Keep,
    /// Tomar snapshot del estado (para restart posterior).
    Snapshot,
}

mod opt_duration_millis {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        d.map(|x| x.as_millis() as u64).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let v: Option<u64> = Option::deserialize(d)?;
        Ok(v.map(Duration::from_millis))
    }
}

// =====================================================================
// CommandRef
// =====================================================================

/// Un comando que vive dentro de un workspace. Se compila a una `Card` con
/// `pin_to` apuntando al workspace padre (label) y su `SomaSpec`
/// intersectado con el del workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRef {
    pub label: String,
    pub payload: Payload,

    /// SomaSpec del comando. El compilador lo intersecta con el del workspace.
    #[serde(default)]
    pub soma: SomaSpec,

    /// Inputs/outputs tipados (mismos `Flow` de brahman-card).
    #[serde(default)]
    pub flows: brahman_card::Flows,

    /// Política de supervisión. Default `OneShot` (un comando se ejecuta y muere).
    #[serde(default = "default_oneshot")]
    pub supervision: Supervision,
}

fn default_oneshot() -> Supervision {
    Supervision::OneShot
}

// =====================================================================
// Pipeline
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineSpec {
    pub label: String,
    pub workspace: WorkspaceId,
    pub nodes: Vec<CommandRef>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
    #[serde(default)]
    pub discern: DiscernPolicy,
    /// Si `true` y cualquier comando del pipeline termina con exit!=0,
    /// el daemon relaunch el pipeline ENTERO (stop + nuevo run_pipeline).
    /// Útil para pipelines de procesamiento continuo.
    #[serde(default)]
    pub restart_on_failure: bool,
    /// Backoff inicial entre restarts (ms). Crece exponencialmente
    /// hasta `restart_max_backoff_ms`. Default 200ms = ~5 restarts/s
    /// inicial, escalando rápido.
    #[serde(default = "default_restart_backoff")]
    pub restart_backoff_ms: u64,
    /// Backoff máximo (ms). Default 30s. El backoff no crece más allá.
    #[serde(default = "default_restart_max_backoff")]
    pub restart_max_backoff_ms: u64,
    /// Máximo de restarts antes de dar up. `0` = infinito. Default 0.
    /// Útil para fail-loud cuando un pipeline siempre falla.
    #[serde(default)]
    pub restart_max: u32,
}

fn default_restart_backoff() -> u64 {
    200
}
fn default_restart_max_backoff() -> u64 {
    30_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    /// Índice en `PipelineSpec.nodes` del productor.
    pub from: usize,
    /// Nombre del Flow output del productor.
    pub from_output: String,
    /// Índice en `PipelineSpec.nodes` del consumidor.
    pub to: usize,
    /// Nombre del Flow input del consumidor.
    pub to_input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscernPolicy {
    /// Bytes a samplear por flow para el discernidor. Default 4 KiB.
    #[serde(default = "default_sample_bytes")]
    pub sample_bytes: usize,
    /// Si `true`, enriquece la Card del producer con el TypeRef detectado.
    #[serde(default = "default_true")]
    pub enrich_producer: bool,
    /// Chunks que el FlowChannel guarda en replay buffer para subscribers
    /// tarde. Default 32. Subir si los productores escriben en ráfagas y
    /// querés que los consumidores tardíos vean toda la salida.
    #[serde(default = "default_replay_chunks")]
    pub replay_chunks: usize,
    /// Tope adicional por **bytes** acumulados en el replay buffer. Lo
    /// que se exceda primero (chunks o bytes) drop-ea el chunk más viejo.
    /// `0` = sin tope por bytes (sólo aplica `replay_chunks`). Útil para
    /// productores con chunks de tamaño variable.
    #[serde(default)]
    pub replay_bytes: usize,
    /// Rate-limit del flow channel (bytes/s). `0` = sin límite. Si está
    /// definido, el splitter sleeps proporcional al tamaño del chunk
    /// antes de re-broadcastear. Protege subscribers lentos.
    #[serde(default)]
    pub max_bytes_per_sec: u64,
}

impl Default for DiscernPolicy {
    fn default() -> Self {
        Self {
            sample_bytes: default_sample_bytes(),
            enrich_producer: default_true(),
            replay_chunks: default_replay_chunks(),
            replay_bytes: 0,
            max_bytes_per_sec: 0,
        }
    }
}

fn default_sample_bytes() -> usize {
    4096
}
fn default_true() -> bool {
    true
}
fn default_replay_chunks() -> usize {
    32
}

// =====================================================================
// Compilación a Card
// =====================================================================

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("workspace label vacío")]
    EmptyWorkspaceLabel,
    #[error("comando con label vacío en posición {0}")]
    EmptyCommandLabel(usize),
    #[error("edge fuera de rango: from={from}, to={to}, nodes={nodes}")]
    EdgeOutOfBounds { from: usize, to: usize, nodes: usize },
}

impl WorkspaceSpec {
    /// Compila el WorkspaceSpec a una Card raíz que el Incarnator puede
    /// encarnar. Usa `Payload::Virtual` (el workspace no es un proceso por
    /// sí solo; sólo aloja hijos).
    pub fn to_card(&self, id: WorkspaceId) -> Result<Card, CompileError> {
        if self.label.trim().is_empty() {
            return Err(CompileError::EmptyWorkspaceLabel);
        }
        let mut c = Card::new(format!("shuma.workspace.{}", self.label));
        c.id = id.0;
        c.soma = self.soma.clone();
        c.permissions = self.permissions.clone();
        c.payload = Payload::Virtual;
        c.supervision = Supervision::OneShot;
        Ok(c)
    }
}

impl CommandRef {
    /// Compila un CommandRef a Card hija de un workspace. La Card resultante
    /// referencia al workspace por label en `pin_to` de cada Flow.
    pub fn to_card(&self, idx: usize, workspace_label: &str) -> Result<Card, CompileError> {
        if self.label.trim().is_empty() {
            return Err(CompileError::EmptyCommandLabel(idx));
        }
        let mut c = Card::new(format!("shuma.cmd.{}.{}", workspace_label, self.label));
        c.payload = self.payload.clone();
        c.soma = intersect_soma(&self.soma, /*workspace*/ &SomaSpec::default());
        c.supervision = self.supervision.clone();
        c.flow = self.flows.clone();
        // pin_to del workspace en cada Flow input/output → el broker prefiere
        // resolver dentro del mismo workspace cuando hay candidatos múltiples.
        let pin = format!("shuma.workspace.{}", workspace_label);
        for f in c.flow.input.iter_mut().chain(c.flow.output.iter_mut()) {
            if f.pin_to.is_none() {
                f.pin_to = Some(pin.clone());
            }
        }
        Ok(c)
    }
}

/// Intersección conservadora: si el workspace pidió aislamiento, la hija
/// también lo tiene (no puede aflojar). Si la hija pidió aislamiento extra,
/// se respeta.
fn intersect_soma(child: &SomaSpec, ws: &SomaSpec) -> SomaSpec {
    let mut out = child.clone();
    out.namespaces.mount  |= ws.namespaces.mount;
    out.namespaces.pid    |= ws.namespaces.pid;
    out.namespaces.net    |= ws.namespaces.net;
    out.namespaces.uts    |= ws.namespaces.uts;
    out.namespaces.ipc    |= ws.namespaces.ipc;
    out.namespaces.user   |= ws.namespaces.user;
    out.namespaces.cgroup |= ws.namespaces.cgroup;
    // rlimits: el menor (más restrictivo) gana.
    out.rlimits.mem_bytes = min_opt(out.rlimits.mem_bytes, ws.rlimits.mem_bytes);
    out.rlimits.nproc = min_opt(out.rlimits.nproc, ws.rlimits.nproc);
    out.rlimits.nofile = min_opt(out.rlimits.nofile, ws.rlimits.nofile);
    out
}

fn min_opt<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

impl PipelineSpec {
    pub fn validate(&self) -> Result<(), CompileError> {
        let n = self.nodes.len();
        for (i, c) in self.nodes.iter().enumerate() {
            if c.label.trim().is_empty() {
                return Err(CompileError::EmptyCommandLabel(i));
            }
        }
        for e in &self.edges {
            if e.from >= n || e.to >= n {
                return Err(CompileError::EdgeOutOfBounds {
                    from: e.from,
                    to: e.to,
                    nodes: n,
                });
            }
        }
        Ok(())
    }
}

// =====================================================================
// I/O conveniencia (TOML + JSON)
// =====================================================================

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("formato desconocido (esperado .toml o .json)")]
    UnknownFormat,
}

pub fn load_workspace_spec(path: &std::path::Path) -> Result<WorkspaceSpec, LoadError> {
    let raw = std::fs::read_to_string(path)?;
    match path.extension().and_then(|s| s.to_str()) {
        Some("toml") => Ok(toml::from_str(&raw)?),
        Some("json") => Ok(serde_json::from_str(&raw)?),
        _ => Err(LoadError::UnknownFormat),
    }
}

pub fn load_pipeline_spec(path: &std::path::Path) -> Result<PipelineSpec, LoadError> {
    let raw = std::fs::read_to_string(path)?;
    match path.extension().and_then(|s| s.to_str()) {
        Some("toml") => Ok(toml::from_str(&raw)?),
        Some("json") => Ok(serde_json::from_str(&raw)?),
        _ => Err(LoadError::UnknownFormat),
    }
}

/// Sustituye `${KEY}` en todos los strings del spec por el valor de
/// `vars["KEY"]`. Variables sin match quedan intactas (no se borra el
/// placeholder — útil para detectar olvidos).
///
/// Walk recursivo sobre la representación JSON intermedia para cubrir
/// labels, argv, envp, paths y cualquier String del schema.
pub fn substitute_vars(
    spec: &PipelineSpec,
    vars: &std::collections::HashMap<String, String>,
) -> Result<PipelineSpec, serde_json::Error> {
    if vars.is_empty() {
        return Ok(spec.clone());
    }
    let mut v = serde_json::to_value(spec)?;
    walk_subst(&mut v, vars);
    serde_json::from_value(v)
}

fn walk_subst(v: &mut serde_json::Value, vars: &std::collections::HashMap<String, String>) {
    match v {
        serde_json::Value::String(s) => {
            *s = subst_str(s, vars);
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                walk_subst(item, vars);
            }
        }
        serde_json::Value::Object(obj) => {
            for (_, val) in obj.iter_mut() {
                walk_subst(val, vars);
            }
        }
        _ => {}
    }
}

fn subst_str(s: &str, vars: &std::collections::HashMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'$' && bytes[i + 1] == b'{' {
            // Buscar el cierre `}`.
            if let Some(close) = bytes[i + 2..].iter().position(|&b| b == b'}') {
                let key = std::str::from_utf8(&bytes[i + 2..i + 2 + close]).unwrap_or("");
                if let Some(val) = vars.get(key) {
                    out.push_str(val);
                    i += 2 + close + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod subst_tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn substitute_in_argv_and_label() {
        let mut vars = HashMap::new();
        vars.insert("MSG".into(), "hola-mundo".into());
        vars.insert("LABEL".into(), "renamed".into());
        let spec = PipelineSpec {
            label: "p-${LABEL}".into(),
            workspace: WorkspaceId::new(),
            nodes: vec![CommandRef {
                label: "node-${LABEL}".into(),
                payload: Payload::Native {
                    exec: "/bin/echo".into(),
                    argv: vec!["${MSG}".into()],
                    envp: vec![],
                },
                soma: Default::default(),
                flows: Default::default(),
                supervision: Supervision::OneShot,
            }],
            edges: vec![],
            discern: DiscernPolicy::default(),
            restart_on_failure: false,
            restart_backoff_ms: 200,
            restart_max_backoff_ms: 30_000,
            restart_max: 0,
        };
        let out = substitute_vars(&spec, &vars).unwrap();
        assert_eq!(out.label, "p-renamed");
        assert_eq!(out.nodes[0].label, "node-renamed");
        match &out.nodes[0].payload {
            Payload::Native { argv, .. } => assert_eq!(argv[0], "hola-mundo"),
            _ => panic!("wrong payload"),
        }
    }

    #[test]
    fn unknown_var_left_intact() {
        let vars = HashMap::new();
        let spec = PipelineSpec {
            label: "p-${UNDEFINED}".into(),
            workspace: WorkspaceId::new(),
            nodes: vec![],
            edges: vec![],
            discern: DiscernPolicy::default(),
            restart_on_failure: false,
            restart_backoff_ms: 200,
            restart_max_backoff_ms: 30_000,
            restart_max: 0,
        };
        let out = substitute_vars(&spec, &vars).unwrap();
        assert_eq!(out.label, "p-${UNDEFINED}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_workspace() -> WorkspaceSpec {
        WorkspaceSpec {
            label: "demo".into(),
            soma: SomaSpec::default(),
            permissions: Permissions::default(),
            ttl: Some(Duration::from_secs(60)),
            flow_dirs: vec![FlowSlot {
                name: "out".into(),
                direction: FlowDirection::Output,
                scope: FlowScope::Public,
            }],
            on_exit: ExitPolicy::Reap,
            quota_enforce: Default::default(),
        }
    }

    #[test]
    fn workspace_toml_roundtrip() {
        let ws = sample_workspace();
        let s = toml::to_string(&ws).unwrap();
        let back: WorkspaceSpec = toml::from_str(&s).unwrap();
        assert_eq!(back.label, ws.label);
        assert_eq!(back.ttl, ws.ttl);
        assert_eq!(back.flow_dirs.len(), 1);
    }

    #[test]
    fn workspace_json_roundtrip() {
        let ws = sample_workspace();
        let s = serde_json::to_string(&ws).unwrap();
        let back: WorkspaceSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(back.label, ws.label);
    }

    #[test]
    fn workspace_compiles_to_card() {
        let ws = sample_workspace();
        let id = WorkspaceId::new();
        let c = ws.to_card(id).unwrap();
        assert_eq!(c.id, id.0);
        assert!(c.label.starts_with("shuma.workspace."));
        assert!(matches!(c.payload, Payload::Virtual));
    }

    #[test]
    fn empty_label_rejected() {
        let mut ws = sample_workspace();
        ws.label = String::new();
        assert!(ws.to_card(WorkspaceId::new()).is_err());
    }

    #[test]
    fn pipeline_validates_edges() {
        let p = PipelineSpec {
            label: "p".into(),
            workspace: WorkspaceId::new(),
            nodes: vec![CommandRef {
                label: "a".into(),
                payload: Payload::Virtual,
                soma: SomaSpec::default(),
                flows: brahman_card::Flows::default(),
                supervision: Supervision::OneShot,
            }],
            edges: vec![FlowEdge {
                from: 0,
                from_output: "x".into(),
                to: 5,
                to_input: "y".into(),
            }],
            discern: DiscernPolicy::default(),
            restart_on_failure: false,
            restart_backoff_ms: 200,
            restart_max_backoff_ms: 30_000,
            restart_max: 0,
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn intersect_soma_takes_more_restrictive() {
        let mut child = SomaSpec::default();
        child.rlimits.mem_bytes = Some(1_000_000);
        let mut ws = SomaSpec::default();
        ws.rlimits.mem_bytes = Some(500_000);
        ws.namespaces.user = true;
        let r = intersect_soma(&child, &ws);
        assert_eq!(r.rlimits.mem_bytes, Some(500_000));
        assert!(r.namespaces.user);
    }
}
