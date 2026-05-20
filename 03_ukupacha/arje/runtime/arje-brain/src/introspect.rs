//! Introspect API. Unix Domain Socket + framing length-prefijo + bincode.
//!
//! Una herramienta externa (ej. `brainctl`) puede consultar el estado del
//! cerebro sin tocar el bus interno del fractal. Esto separa observación de
//! ejecución — la introspección es read-only por diseño.

use crate::crystallize::{detect_crystals, Crystal, CrystallizationParams};
use crate::engine::RuleEngine;
use crate::observer::Observer;
use crate::rules::Rule;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;
use tracing::{debug, info, trace, warn};
use ulid::Ulid;

const MAX_FRAME: usize = 4 * 1024 * 1024; // 4 MiB — correlation matrices crecen

/// Estado compartido entre el bucle del Init y el servidor de introspección.
/// `Arc<RwLock<...>>` permite muchos lectores concurrentes (introspect) y
/// un escritor (el dispatcher de eventos en el bucle primordial).
#[derive(Clone)]
pub struct BrainState {
    pub engine: Arc<RwLock<RuleEngine>>,
    pub observer: Arc<RwLock<Observer>>,
    pub params: CrystallizationParams,
    /// Path opcional donde apendear reglas promovidas en JSONL. Si Some,
    /// cada PromoteCrystal añade una línea (append-only) con la Rule serializada.
    pub rules_out: Option<Arc<PathBuf>>,
    /// Audit log en memoria. Cada promote/remove deja huella aquí.
    pub audit: Arc<RwLock<crate::audit::AuditLog>>,
}

impl BrainState {
    pub fn new(window_size: usize) -> Self {
        Self::with_params(window_size, CrystallizationParams::default())
    }

    pub fn with_params(window_size: usize, params: CrystallizationParams) -> Self {
        Self {
            engine: Arc::new(RwLock::new(RuleEngine::empty())),
            observer: Arc::new(RwLock::new(Observer::new(window_size))),
            params,
            rules_out: None,
            audit: Arc::new(RwLock::new(crate::audit::AuditLog::new())),
        }
    }

    pub fn with_rules_out(mut self, path: PathBuf) -> Self {
        self.rules_out = Some(Arc::new(path));
        self
    }
}

/// Append-only writer de una `Rule` serializada a `rules_out` en formato
/// JSONL: una línea = un Rule JSON. Idempotente respecto a re-flushes
/// porque el caller se encarga de no apendar la misma rule dos veces.
/// El loader (`loader::extract_rules_from_json`) acepta tanto JSONL como
/// arrays — el archivo es legible en ambos modos.
pub fn append_rule_jsonl(path: &Path, rule: &Rule) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(rule)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(file, "{line}")?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IntrospectRequest {
    /// Lista resumida de reglas vivas.
    ListRules,
    /// Detalle de una regla concreta.
    GetRule(Ulid),
    /// Snapshot de la entropía y conteos básicos.
    EntropySnapshot,
    /// Top N pares (a, b) por co-ocurrencia.
    TopCorrelations { n: usize },
    /// Cristales detectados con los parámetros del BrainState.
    Crystals,
    /// Serializa la Rule derivada de un cristal específico como JSON
    /// (índice tras Crystals).
    CrystalJson { index: usize },
    /// Promueve el cristal #index a regla viva en el motor. Devuelve el
    /// rule_id asignado y el JSON de la Rule para auditoría/persistencia.
    PromoteCrystal { index: usize },
    /// Elimina una regla viva por id. Útil para revertir un promote.
    RemoveRule { id: Ulid },
    /// Lista las últimas N entradas del audit log. limit=0 = todas.
    ListAudit { limit: usize },
    /// Persiste todas las entries pendientes al CAS y actualiza el head
    /// pointer si el log lo tiene configurado.
    FlushAudit,
    /// Recarga reglas desde el archivo configurado por --rules-out (o el
    /// path provisto). Vacía el engine antes de cargar.
    ReloadRules { path: Option<String> },
    /// Verifica la cadena audit recorriendo prev_sha hasta el genesis,
    /// validando integridad de cada entry contra el CAS.
    VerifyAudit,
    /// Reconstruye el engine desde la cadena audit. Vacía engine y aplica
    /// PromoteCrystal/RemoveRule en orden cronológico.
    ReplayAudit,
    /// Mantiene la conexión abierta y empuja cada `AuditEntry` nuevo en
    /// frames `IntrospectResponse::AuditStreamFrame` hasta que el cliente
    /// cierra. Tras esta request no se aceptan más requests en la misma conn.
    StreamAudit,
    /// Garbage-collect el CAS. Considera reachable: todo lo alcanzable desde
    /// el head del audit log. Cualquier blob extra (Wasm modules referenciados
    /// por Cards) debe haberse pasado en `extra_roots` por el caller.
    GcCas { extra_roots: Vec<[u8; 32]> },
    /// Detecta cristales de patrones temporales (Burst, Silence).
    PatternCrystals,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IntrospectResponse {
    Rules(Vec<RuleSummary>),
    Rule(Option<Rule>),
    Entropy { value_bits: f64, sample_size: u64, distinct_kinds: usize, window_full: bool },
    Correlations(Vec<CorrelationEntry>),
    Crystals(Vec<Crystal>),
    Json(String),
    /// Resultado de PromoteCrystal: id de la regla creada + JSON de la Rule
    /// para que el operador lo persista en disco si quiere.
    Promoted { rule_id: Ulid, rule_json: String },
    /// Resultado de RemoveRule: true si existía, false si ya no.
    Removed(bool),
    /// Entradas del audit log (más recientes al final).
    AuditEntries(Vec<crate::audit::AuditEntry>),
    /// Resultado de FlushAudit: cuántas entries se escribieron y SHA del head.
    Flushed { written: usize, head_sha: Option<[u8; 32]>, total_flushed: u64 },
    /// Resultado de ReloadRules: número total de reglas tras el reload.
    Reloaded { count: usize },
    /// Resultado de VerifyAudit.
    AuditVerified(crate::audit::VerificationReport),
    /// Resultado de ReplayAudit.
    Replayed(crate::audit::ReplayReport),
    /// Frame de streaming. El cliente lee estos en bucle hasta EOF.
    AuditStreamFrame(crate::audit::AuditEntry),
    /// Resultado de GcCas: cuántos blobs eliminados y bytes liberados.
    GcResult { deleted: usize, freed_bytes: u64 },
    /// Cristales de Burst/Silence detectados.
    Patterns(Vec<crate::crystallize::PatternCrystal>),
    Error(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RuleSummary {
    pub id: Ulid,
    pub priority: u8,
    pub event_kind_tag: String,
    pub action_count: usize,
    pub scope_wildcard: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CorrelationEntry {
    pub a: String,
    pub b: String,
    pub joint_count: u64,
    pub conditional_prob: f64,
    pub pmi_bits: f64,
}

pub struct IntrospectServer {
    state: BrainState,
}

impl IntrospectServer {
    pub fn new(state: BrainState) -> Self { Self { state } }

    /// Spawn del listener. Devuelve cuando bind() falla; en caso contrario
    /// corre indefinidamente.
    pub async fn serve(self, path: &Path) -> anyhow::Result<()> {
        let _ = std::fs::remove_file(path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let listener = UnixListener::bind(path)?;
        info!(path = %path.display(), "brain introspect escuchando");
        let arc_self = Arc::new(self);
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    trace!("introspect conn aceptada");
                    let me = arc_self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = me.handle(stream).await {
                            warn!(?e, "introspect conn ended");
                        }
                    });
                }
                Err(e) => {
                    warn!(?e, "introspect accept failed");
                    return Ok(());
                }
            }
        }
    }

    async fn handle(self: Arc<Self>, mut stream: UnixStream) -> anyhow::Result<()> {
        loop {
            let mut len_buf = [0u8; 4];
            if stream.read_exact(&mut len_buf).await.is_err() {
                return Ok(()); // EOF
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            if len > MAX_FRAME {
                anyhow::bail!("frame oversize: {len}");
            }
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            let req: IntrospectRequest = bincode::deserialize(&buf)?;
            debug!(?req, "introspect request");

            // StreamAudit toma posesión de la conn — no más requests aquí.
            if matches!(req, IntrospectRequest::StreamAudit) {
                return self.stream_audit(stream).await;
            }

            let resp = self.dispatch(req).await;

            let out = bincode::serialize(&resp)?;
            stream.write_u32(out.len() as u32).await?;
            stream.write_all(&out).await?;
        }
    }

    /// Modo streaming: subscribe al audit log y empuja cada entry como
    /// frame `AuditStreamFrame`. La función retorna cuando el cliente
    /// cierra (write falla) o el subscriber se desconecta.
    async fn stream_audit(self: Arc<Self>, mut stream: UnixStream) -> anyhow::Result<()> {
        let mut rx = self.state.audit.write().await.subscribe();
        info!("audit stream client conectado");
        while let Some(entry) = rx.recv().await {
            let frame = IntrospectResponse::AuditStreamFrame(entry);
            let bytes = bincode::serialize(&frame)?;
            if stream.write_u32(bytes.len() as u32).await.is_err() { break; }
            if stream.write_all(&bytes).await.is_err() { break; }
        }
        info!("audit stream client desconectado");
        Ok(())
    }

    async fn dispatch(&self, req: IntrospectRequest) -> IntrospectResponse {
        match req {
            IntrospectRequest::ListRules => {
                let engine = self.state.engine.read().await;
                let rules = engine.rules()
                    .map(|r| RuleSummary {
                        id: r.id,
                        priority: r.priority,
                        event_kind_tag: format!("{:?}", r.when),
                        action_count: r.then.len(),
                        scope_wildcard: r.scope.is_wildcard(),
                    })
                    .collect();
                IntrospectResponse::Rules(rules)
            }
            IntrospectRequest::GetRule(id) => {
                let engine = self.state.engine.read().await;
                let found = engine.rules()
                    .find(|r| r.id == id)
                    .map(|r| Rule::clone(r));
                IntrospectResponse::Rule(found)
            }
            IntrospectRequest::EntropySnapshot => {
                let obs = self.state.observer.read().await;
                IntrospectResponse::Entropy {
                    value_bits: obs.shannon_entropy(),
                    sample_size: obs.total(),
                    distinct_kinds: obs.marginals().len(),
                    window_full: obs.current_window() >= obs.window_size(),
                }
            }
            IntrospectRequest::TopCorrelations { n } => {
                let obs = self.state.observer.read().await;
                let mut entries: Vec<CorrelationEntry> = obs.cooccurrences().iter()
                    .map(|((a, b), &joint)| CorrelationEntry {
                        a: format!("{:?}", a),
                        b: format!("{:?}", b),
                        joint_count: joint,
                        conditional_prob: obs.conditional_prob(a, b),
                        pmi_bits: obs.pmi(a, b),
                    })
                    .collect();
                entries.sort_by(|x, y| y.joint_count.cmp(&x.joint_count));
                entries.truncate(n);
                IntrospectResponse::Correlations(entries)
            }
            IntrospectRequest::Crystals => {
                let obs = self.state.observer.read().await;
                let crystals = detect_crystals(&obs, &self.state.params);
                IntrospectResponse::Crystals(crystals)
            }
            IntrospectRequest::CrystalJson { index } => {
                let obs = self.state.observer.read().await;
                let crystals = detect_crystals(&obs, &self.state.params);
                match crystals.get(index) {
                    Some(c) => IntrospectResponse::Json(crate::crystallize::crystal_to_json_pretty(c)),
                    None => IntrospectResponse::Error(format!("no crystal at index {index}")),
                }
            }
            IntrospectRequest::PromoteCrystal { index } => {
                let crystals = {
                    let obs = self.state.observer.read().await;
                    detect_crystals(&obs, &self.state.params)
                };
                match crystals.get(index) {
                    Some(c) => {
                        let rule = crate::crystallize::crystal_to_rule(c);
                        let rule_id = rule.id;
                        let rule_json = serde_json::to_string_pretty(&rule)
                            .unwrap_or_else(|_| "<serialize failed>".into());
                        self.state.engine.write().await.insert(rule.clone());
                        // Persistencia opcional al archivo JSONL.
                        if let Some(path) = self.state.rules_out.as_ref() {
                            if let Err(e) = append_rule_jsonl(path, &rule) {
                                warn!(?e, path = %path.display(), "rules_out append falló");
                            } else {
                                info!(path = %path.display(), %rule_id, "regla persistida a JSONL");
                            }
                        }
                        // Audit entry
                        self.state.audit.write().await.append(
                            crate::audit::AuditAction::PromoteCrystal {
                                rule_id, crystal: c.clone(),
                            }
                        );
                        IntrospectResponse::Promoted { rule_id, rule_json }
                    }
                    None => IntrospectResponse::Error(format!("no crystal at index {index}")),
                }
            }
            IntrospectRequest::RemoveRule { id } => {
                let removed = self.state.engine.write().await.remove(id);
                if removed {
                    self.state.audit.write().await.append(
                        crate::audit::AuditAction::RemoveRule { rule_id: id }
                    );
                }
                IntrospectResponse::Removed(removed)
            }
            IntrospectRequest::ListAudit { limit } => {
                let audit = self.state.audit.read().await;
                IntrospectResponse::AuditEntries(audit.recent(limit).cloned().collect())
            }
            IntrospectRequest::FlushAudit => {
                let mut audit = self.state.audit.write().await;
                match audit.flush_to_cas() {
                    Ok(written) => IntrospectResponse::Flushed {
                        written,
                        head_sha: audit.last_flushed_sha(),
                        total_flushed: audit.flushed_count(),
                    },
                    Err(e) => IntrospectResponse::Error(format!("flush_to_cas: {e}")),
                }
            }
            IntrospectRequest::VerifyAudit => {
                let head = self.state.audit.read().await.last_flushed_sha();
                let head = match head {
                    Some(h) => h,
                    None => return IntrospectResponse::Error(
                        "audit log sin entries flushadas — nada que verificar".into()
                    ),
                };
                let report = crate::audit::verify_chain_from_cas(head);
                IntrospectResponse::AuditVerified(report)
            }
            IntrospectRequest::StreamAudit => {
                // Inalcanzable por construcción: handle() detecta StreamAudit
                // antes de llamar a dispatch(). Pero el match exige cubrir.
                IntrospectResponse::Error(
                    "StreamAudit no debe llegar a dispatch — bug del handler".into()
                )
            }
            IntrospectRequest::PatternCrystals => {
                let obs = self.state.observer.read().await;
                let params = crate::crystallize::PatternParams::default();
                let patterns = crate::crystallize::detect_pattern_crystals(&obs, &params);
                IntrospectResponse::Patterns(patterns)
            }
            IntrospectRequest::GcCas { extra_roots } => {
                // Reachable = audit chain desde head + extra_roots provistos.
                let mut reachable = std::collections::HashSet::new();
                if let Some(head) = self.state.audit.read().await.last_flushed_sha() {
                    reachable.extend(crate::audit::reachable_from_head(head));
                }
                reachable.extend(extra_roots);
                match arje_cas::gc(&reachable) {
                    Ok((deleted, freed_bytes)) => IntrospectResponse::GcResult { deleted, freed_bytes },
                    Err(e) => IntrospectResponse::Error(format!("gc: {e}")),
                }
            }
            IntrospectRequest::ReplayAudit => {
                let head = self.state.audit.read().await.last_flushed_sha();
                let head = match head {
                    Some(h) => h,
                    None => return IntrospectResponse::Error(
                        "audit log sin entries flushadas — nada que replayar".into()
                    ),
                };
                let mut engine = self.state.engine.write().await;
                *engine = crate::engine::RuleEngine::empty();
                let report = crate::audit::replay_chain(head, &mut engine);
                IntrospectResponse::Replayed(report)
            }
            IntrospectRequest::ReloadRules { path } => {
                // Path explícito gana sobre el rules_out configurado.
                let resolved = path.map(std::path::PathBuf::from)
                    .or_else(|| self.state.rules_out.as_ref().map(|p| p.as_path().to_path_buf()));
                let path = match resolved {
                    Some(p) => p,
                    None => return IntrospectResponse::Error(
                        "ReloadRules sin path y sin rules_out configurado".into()
                    ),
                };
                let rules = match crate::loader::load_rules_file(&path) {
                    Ok(r) => r,
                    Err(e) => return IntrospectResponse::Error(format!("load: {e}")),
                };
                // Vaciamos el engine antes de re-cargar — semántica clean-slate.
                let mut engine = self.state.engine.write().await;
                *engine = crate::engine::RuleEngine::empty();
                let count = rules.len();
                for r in rules { engine.insert(r); }
                drop(engine);
                self.state.audit.write().await.append(
                    crate::audit::AuditAction::LoadRulesFile {
                        path: path.to_string_lossy().into_owned(),
                        count,
                    }
                );
                IntrospectResponse::Reloaded { count }
            }
        }
    }
}

// Cliente helper para tools externos (brainctl).
pub async fn call(path: &Path, req: IntrospectRequest) -> anyhow::Result<IntrospectResponse> {
    let mut stream = UnixStream::connect(path).await?;
    let buf = bincode::serialize(&req)?;
    stream.write_u32(buf.len() as u32).await?;
    stream.write_all(&buf).await?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        anyhow::bail!("response oversize: {len}");
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(bincode::deserialize(&buf)?)
}

/// Consume la lista marginal del observer para humanos. Suprime el detalle
/// crudo de `EventKind` (ej. payloads largos en BusInvokeOf).
pub fn marginal_summary(obs: &Observer) -> Vec<(String, u64)> {
    let mut entries: Vec<(String, u64)> = obs.marginals().iter()
        .map(|(k, &c)| (format!("{:?}", k), c))
        .collect();
    entries.sort_by(|x, y| y.1.cmp(&x.1));
    entries
}

