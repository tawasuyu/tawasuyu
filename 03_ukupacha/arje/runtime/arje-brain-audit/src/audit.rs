//! Audit log: cada acción mutadora del cerebro deja una entry inmutable
//! con su predecesor encadenado por SHA256 (estilo Merkle). Verificable a
//! posteriori sin confianza en quien escribe.
//!
//! Los entries viven en memoria. Para persistencia, `flush_to_cas()` los
//! escribe al content-addressable store y devuelve el SHA del head, que
//! puede guardarse en un archivo de "head pointer" (fuera de scope aquí).

use arje_brain_cognitive::crystallize::Crystal;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use ulid::Ulid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Sequence number monotónico desde el inicio del log.
    pub seq: u64,
    /// Wall-clock al insertar.
    pub timestamp_ms: u64,
    /// SHA256 del entry anterior. None para el primer entry.
    pub prev_sha: Option<[u8; 32]>,
    /// SHA256 de este entry (auto-calculado al construir).
    pub sha: [u8; 32],
    /// Acción registrada.
    pub action: AuditAction,
}

/// Sin `#[serde(tag)]`: bincode requiere external tagging (default serde
/// para enums) para no usar `deserialize_any`. JSON sigue legible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditAction {
    PromoteCrystal { rule_id: Ulid, crystal: Crystal },
    RemoveRule { rule_id: Ulid },
    LoadRulesFile { path: String, count: usize },
    /// Un peer del bus pidió enviar una señal a un Ente del fractal.
    /// El registro precede al `kill(2)` — capturamos la intención sin
    /// importar si el syscall succede.
    KillEnte { caller: Ulid, target: Ulid, signal: i32 },
    /// Un peer del bus pidió cargar una Card por nombre desde el card store.
    /// `caller` es la identidad autenticada del peer; `name` el filename
    /// sin la extensión `.json`.
    SpawnCardFromDisk { caller: Ulid, name: String },
    /// Un peer del bus pidió encarnar una Card **transmitida por el wire**
    /// (no del store). `caller` es la identidad autenticada; `label` el de la
    /// Card. La card se encarna con las capacidades del caller (ver
    /// `arje_bus::BusRequest::RunCard`).
    RunCard { caller: Ulid, label: String },
    /// El cerebro declaró una inhibición. Mientras la entrada esté viva
    /// (TTL en el grafo), las acciones escalatorias quedan bloqueadas;
    /// auditar la entrada permite reconstruir por qué.
    BrainInhibit { reason: String },
    /// Un peer del bus pidió shutdown/reboot/suspend/hibernate. `caller`
    /// es None si vino anónimo (no autenticado contra el grafo).
    PowerMgmt {
        caller: Option<Ulid>,
        peer_pid: i32,
        kind: String,
        interactive: bool,
    },
    /// Veredicto de la atestación al arranque (A2) para un binario crítico:
    /// `arje-zero` computó el BLAKE3 del binario vivo y lo contrastó contra el
    /// manifiesto firmado del seed. Queda en la cadena anclada al CAS para que
    /// el boot sea auditable. `verdict`/`policy` son las formas legibles de
    /// `arje_attest::Veredicto` y `card_core::AttestPolicy`.
    AttestationCheck {
        binary: String,
        got_hash: [u8; 32],
        verdict: String,
        policy: String,
    },
}

/// Tag plano de un `AuditAction`, serializable y comparable sin payload.
/// Los filtros se expresan en términos de esta tag — el payload de la
/// acción es ruido para una query "muéstrame sólo los KillEnte".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuditActionKind {
    PromoteCrystal,
    RemoveRule,
    LoadRulesFile,
    KillEnte,
    SpawnCardFromDisk,
    RunCard,
    BrainInhibit,
    PowerMgmt,
    AttestationCheck,
}

impl AuditActionKind {
    /// Tag canónico en kebab-case. Es la forma esperada por el CLI:
    /// `brainctl audit --kind kill-ente`. Estable en el tiempo —
    /// no renombrar sin actualizar el parser.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PromoteCrystal => "promote-crystal",
            Self::RemoveRule => "remove-rule",
            Self::LoadRulesFile => "load-rules-file",
            Self::KillEnte => "kill-ente",
            Self::SpawnCardFromDisk => "spawn-card-from-disk",
            Self::RunCard => "run-card",
            Self::BrainInhibit => "brain-inhibit",
            Self::PowerMgmt => "power-mgmt",
            Self::AttestationCheck => "attestation-check",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "promote-crystal" => Some(Self::PromoteCrystal),
            "remove-rule" => Some(Self::RemoveRule),
            "load-rules-file" => Some(Self::LoadRulesFile),
            "kill-ente" => Some(Self::KillEnte),
            "spawn-card-from-disk" => Some(Self::SpawnCardFromDisk),
            "run-card" => Some(Self::RunCard),
            "brain-inhibit" => Some(Self::BrainInhibit),
            "power-mgmt" => Some(Self::PowerMgmt),
            "attestation-check" => Some(Self::AttestationCheck),
            _ => None,
        }
    }
}

impl AuditAction {
    pub fn kind(&self) -> AuditActionKind {
        match self {
            Self::PromoteCrystal { .. } => AuditActionKind::PromoteCrystal,
            Self::RemoveRule { .. } => AuditActionKind::RemoveRule,
            Self::LoadRulesFile { .. } => AuditActionKind::LoadRulesFile,
            Self::KillEnte { .. } => AuditActionKind::KillEnte,
            Self::SpawnCardFromDisk { .. } => AuditActionKind::SpawnCardFromDisk,
            Self::RunCard { .. } => AuditActionKind::RunCard,
            Self::BrainInhibit { .. } => AuditActionKind::BrainInhibit,
            Self::PowerMgmt { .. } => AuditActionKind::PowerMgmt,
            Self::AttestationCheck { .. } => AuditActionKind::AttestationCheck,
        }
    }
}

/// Predicado sobre `AuditEntry`. Vacío == identidad (todo pasa).
/// Las dos coordenadas se combinan en AND: kind ∈ kinds && seq > since_seq.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditFilter {
    /// Si no está vacío, sólo entries cuya `action.kind()` pertenezca al set
    /// pasan. Vacío = no filtra por kind (acepta todo).
    pub kinds: Vec<AuditActionKind>,
    /// Si Some(s), sólo entries con `seq > s` pasan — pensado para que un
    /// cliente que ya tiene hasta seq=N pida "lo nuevo desde entonces" sin
    /// re-recibir lo ya visto. None = no filtra por seq.
    pub since_seq: Option<u64>,
}

impl AuditFilter {
    pub fn matches(&self, entry: &AuditEntry) -> bool {
        if let Some(s) = self.since_seq {
            if entry.seq <= s { return false; }
        }
        if !self.kinds.is_empty() && !self.kinds.contains(&entry.action.kind()) {
            return false;
        }
        true
    }

    pub fn is_identity(&self) -> bool {
        self.kinds.is_empty() && self.since_seq.is_none()
    }
}

pub struct AuditLog {
    entries: VecDeque<AuditEntry>,
    next_seq: u64,
    /// Cap del log en memoria. Entries más viejos se descartan tras flush.
    cap: usize,
    /// Total acumulado de entries flusheadas a CAS.
    flushed_count: u64,
    /// SHA del último entry persistido a CAS — el "head pointer" del log.
    last_flushed_sha: Option<[u8; 32]>,
    /// Path opcional donde escribir el head pointer tras cada flush.
    head_pointer_path: Option<std::path::PathBuf>,
    /// Subscribers a entries en tiempo real. Cada `append` empuja a todos.
    /// Subscribers cuyo receiver se dropeó se purgan en el siguiente push.
    subscribers: Vec<tokio::sync::mpsc::UnboundedSender<AuditEntry>>,
    /// Wall-clock del último flush exitoso a CAS. None si aún no se flush.
    last_flush_at_ms: Option<u64>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self::with_cap(512)
    }

    pub fn with_cap(cap: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            next_seq: 0,
            cap,
            flushed_count: 0,
            last_flushed_sha: None,
            head_pointer_path: None,
            subscribers: Vec::new(),
            last_flush_at_ms: None,
        }
    }

    /// Registra un nuevo subscriber. El receiver recibe cada `AuditEntry`
    /// futuro hasta que el receiver se dropee (subscriber se purga al
    /// siguiente `append`).
    pub fn subscribe(&mut self) -> tokio::sync::mpsc::UnboundedReceiver<AuditEntry> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.subscribers.push(tx);
        rx
    }

    pub fn subscriber_count(&self) -> usize { self.subscribers.len() }

    pub fn with_head_pointer(mut self, path: std::path::PathBuf) -> Self {
        self.head_pointer_path = Some(path);
        self
    }

    /// Apendea una acción. Calcula el SHA encadenado contra el último entry.
    pub fn append(&mut self, action: AuditAction) -> AuditEntry {
        let prev_sha = self.entries.back().map(|e| e.sha);
        let timestamp_ms = now_ms();
        let seq = self.next_seq;
        self.next_seq += 1;

        // Pre-construct con sha en cero, luego calcular sha sobre el
        // serializado canónico, luego sobreescribir el campo.
        let mut entry = AuditEntry {
            seq, timestamp_ms, prev_sha, sha: [0u8; 32], action,
        };
        entry.sha = compute_sha(&entry);

        if self.entries.len() >= self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(entry.clone());
        // Empujar a subscribers, purgando los muertos in-place.
        self.subscribers.retain(|tx| tx.send(entry.clone()).is_ok());
        entry
    }

    pub fn recent(&self, limit: usize) -> impl Iterator<Item = &AuditEntry> {
        let n = if limit == 0 { self.entries.len() } else { limit.min(self.entries.len()) };
        self.entries.iter().skip(self.entries.len() - n)
    }

    /// Filtra primero, recorta después. El orden importa: si `cap` evictó
    /// los entries que el filtro habría descartado igual, igual queremos
    /// `limit` entries del *resultado* filtrado, no de la ventana cruda.
    /// `limit = 0` devuelve todos los matches.
    pub fn recent_filtered<'a>(
        &'a self,
        limit: usize,
        filter: &'a AuditFilter,
    ) -> Vec<&'a AuditEntry> {
        let matched: Vec<&AuditEntry> = self.entries.iter()
            .filter(|e| filter.matches(e))
            .collect();
        if limit == 0 || matched.len() <= limit {
            matched
        } else {
            // Conservamos los más recientes: drop al frente.
            matched[matched.len() - limit..].to_vec()
        }
    }

    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    pub fn head_sha(&self) -> Option<[u8; 32]> {
        self.entries.back().map(|e| e.sha)
    }

    /// Persiste el entry pasado al CAS y devuelve su SHA. Pensado para
    /// snapshots externos — el log en memoria sigue intacto.
    pub fn persist_to_cas(entry: &AuditEntry) -> anyhow::Result<[u8; 32]> {
        let bytes = serde_json::to_vec(entry)?;
        let sha = arje_cas::store(&bytes)?;
        Ok(sha)
    }

    /// Persiste TODOS los entries actuales al CAS y actualiza el head pointer.
    /// Idempotente: re-flushar dos veces da los mismos SHAs (CAS dedup).
    /// Devuelve cuántas entries se flushearon en esta pasada.
    ///
    /// Forma canónica: serializamos `entry` con `sha = [0; 32]` (format
    /// pre-hash). El CAS computa sha256 sobre esos bytes y devuelve un SHA
    /// que por construcción coincide con `entry.sha` calculado al append.
    pub fn flush_to_cas(&mut self) -> anyhow::Result<usize> {
        let mut written = 0;
        let mut last_sha = self.last_flushed_sha;
        for entry in &self.entries {
            if entry.seq < self.flushed_count { continue; }
            let bytes = canonical_bytes(entry);
            let sha = arje_cas::store(&bytes)?;
            debug_assert_eq!(sha, entry.sha,
                "CAS sha != entry.sha — fórmula canónica rota");
            last_sha = Some(sha);
            written += 1;
        }
        self.flushed_count += written as u64;
        self.last_flushed_sha = last_sha;
        if written > 0 {
            self.last_flush_at_ms = Some(now_ms());
        }
        // Persistir head pointer si está configurado.
        if let (Some(path), Some(sha)) = (&self.head_pointer_path, last_sha) {
            let pointer = AuditHeadPointer {
                last_seq: self.next_seq.saturating_sub(1),
                last_sha: sha,
                flushed_count: self.flushed_count,
                timestamp_ms: now_ms(),
            };
            let json = serde_json::to_vec_pretty(&pointer)?;
            // Escritura atómica: tmp + rename
            let tmp = path.with_extension("tmp");
            if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
            std::fs::write(&tmp, json)?;
            std::fs::rename(&tmp, path)?;
        }
        Ok(written)
    }

    pub fn flushed_count(&self) -> u64 { self.flushed_count }
    pub fn last_flushed_sha(&self) -> Option<[u8; 32]> { self.last_flushed_sha }
    pub fn last_flush_at_ms(&self) -> Option<u64> { self.last_flush_at_ms }

    /// Segundos transcurridos desde el último flush. None si nunca se flush.
    pub fn last_flush_age_secs(&self) -> Option<f64> {
        let then = self.last_flush_at_ms?;
        let now = now_ms();
        Some((now.saturating_sub(then)) as f64 / 1000.0)
    }
}

/// Pointer al head del audit log — escrito atómicamente en disco tras cada
/// flush. Permite verificar la integridad del log sin escanearlo entero:
/// el cliente lee el head, recupera el blob desde CAS, valida `prev_sha`
/// recursivamente hasta el genesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditHeadPointer {
    pub last_seq: u64,
    pub last_sha: [u8; 32],
    pub flushed_count: u64,
    pub timestamp_ms: u64,
}

/// Reporte de un replay: número de actions aplicadas + reglas finales.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayReport {
    pub applied: u64,
    pub final_rule_count: usize,
    pub error: Option<String>,
}

/// Reporte de verificación de la cadena audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    /// Cuántas entries se recorrieron y verificaron exitosamente.
    pub verified: u64,
    /// Si hubo error, el seq donde se detectó.
    pub broken_at_seq: Option<u64>,
    /// Detalles del error si hubo.
    pub error: Option<String>,
    /// SHA del genesis (primer entry; prev_sha = None).
    pub genesis_sha: Option<[u8; 32]>,
}

/// Recorre la cadena del audit log desde `start_sha` hacia atrás vía `prev_sha`
/// hasta el genesis. Para cada entry valida:
///   1. CAS contiene un blob bajo ese SHA
///   2. sha256(blob) == SHA esperado (defensa contra tampering del CAS)
///   3. El blob deserializa a AuditEntry con sha=[0;32] (forma canónica)
///
/// Devuelve un VerificationReport con el conteo, posibles errores y
/// el SHA del genesis (útil para clientes que quieren cachearlo).
pub fn verify_chain_from_cas(start_sha: [u8; 32]) -> VerificationReport {
    let mut current = Some(start_sha);
    let mut verified = 0u64;
    let mut last_seen: Option<AuditEntry> = None;

    while let Some(sha) = current {
        let path = arje_cas::cas_root().join(arje_cas::hex(&sha));
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return VerificationReport {
                verified,
                broken_at_seq: last_seen.as_ref().map(|e| e.seq),
                error: Some(format!("CAS read {}: {e}", path.display())),
                genesis_sha: None,
            },
        };
        // Verificación 1: el blob hashea al hash esperado (CAS contract, BLAKE3).
        let actual = arje_cas::blake3_of(&bytes);
        if actual != sha {
            return VerificationReport {
                verified,
                broken_at_seq: last_seen.as_ref().map(|e| e.seq),
                error: Some(format!(
                    "CAS tamper en {}: expected {} got {}",
                    path.display(), arje_cas::hex(&sha), arje_cas::hex(&actual)
                )),
                genesis_sha: None,
            };
        }
        // Verificación 2: deserialize. El blob canónico tiene sha=[0;32].
        let mut entry: AuditEntry = match serde_json::from_slice(&bytes) {
            Ok(e) => e,
            Err(e) => return VerificationReport {
                verified,
                broken_at_seq: last_seen.as_ref().map(|e| e.seq),
                error: Some(format!("deserialize: {e}")),
                genesis_sha: None,
            },
        };
        // Re-poblar el sha en el entry para reportar coherentemente.
        entry.sha = sha;
        verified += 1;

        let prev = entry.prev_sha;
        last_seen = Some(entry);
        current = prev;
    }

    VerificationReport {
        verified,
        broken_at_seq: None,
        error: None,
        genesis_sha: last_seen.as_ref().map(|e| e.sha),
    }
}

/// Devuelve el set de SHAs alcanzables desde `start_sha` siguiendo
/// `prev_sha` hasta el genesis. Usado por el GC del CAS para construir
/// las "raíces vivas" del audit log.
pub fn reachable_from_head(start_sha: [u8; 32]) -> std::collections::HashSet<[u8; 32]> {
    let mut set = std::collections::HashSet::new();
    let mut current = Some(start_sha);
    while let Some(sha) = current {
        if !set.insert(sha) { break; } // ciclo (no debería pasar) — corta
        let path = arje_cas::cas_root().join(arje_cas::hex(&sha));
        let bytes = match std::fs::read(&path) { Ok(b) => b, Err(_) => break };
        let entry: AuditEntry = match serde_json::from_slice(&bytes) {
            Ok(e) => e, Err(_) => break,
        };
        current = entry.prev_sha;
    }
    set
}

/// Recorre la cadena entera (head→genesis) y reconstruye la lista de
/// actions en orden cronológico (oldest first). Útil tanto para replay
/// como para auditoría retrospectiva.
pub fn collect_chain_from_cas(start_sha: [u8; 32]) -> anyhow::Result<Vec<AuditEntry>> {
    let mut entries = Vec::new();
    let mut current = Some(start_sha);
    while let Some(sha) = current {
        let path = arje_cas::cas_root().join(arje_cas::hex(&sha));
        let bytes = std::fs::read(&path)?;
        let mut entry: AuditEntry = serde_json::from_slice(&bytes)?;
        entry.sha = sha;
        let prev = entry.prev_sha;
        entries.push(entry);
        current = prev;
    }
    // entries está en orden head→genesis. Reverse para chronological.
    entries.reverse();
    Ok(entries)
}

/// Aplica las actions de la cadena en orden cronológico contra un engine
/// fresco. PromoteCrystal → insert. RemoveRule → remove. LoadRulesFile →
/// log informativo (los archivos pueden no existir en el ambiente actual).
pub fn replay_chain(
    start_sha: [u8; 32],
    engine: &mut arje_brain_rules::engine::RuleEngine,
) -> ReplayReport {
    let entries = match collect_chain_from_cas(start_sha) {
        Ok(es) => es,
        Err(e) => return ReplayReport {
            applied: 0, final_rule_count: engine.len(),
            error: Some(format!("collect chain: {e}")),
        },
    };
    let mut applied = 0u64;
    for entry in &entries {
        match &entry.action {
            AuditAction::PromoteCrystal { rule_id, crystal } => {
                let mut rule = arje_brain_cognitive::crystallize::crystal_to_rule(crystal);
                rule.id = *rule_id; // preservar identidad histórica
                engine.insert(rule);
            }
            AuditAction::RemoveRule { rule_id } => {
                engine.remove(*rule_id);
            }
            AuditAction::LoadRulesFile { path: _, count: _ } => {
                // Los archivos referenciados por path pueden haber cambiado
                // o no existir. Log y skip — el replay sólo reconstruye
                // promotes/removes que tienen estado en CAS.
            }
            AuditAction::KillEnte { .. }
            | AuditAction::SpawnCardFromDisk { .. }
            | AuditAction::RunCard { .. }
            | AuditAction::BrainInhibit { .. }
            | AuditAction::PowerMgmt { .. }
            | AuditAction::AttestationCheck { .. } => {
                // Acciones del bus son auditoría narrativa, no estado del
                // motor de reglas. El replay las preserva en la cadena CAS
                // (vía sha encadenado) pero no las aplica — no tienen
                // contraparte estructural en `RuleEngine`.
            }
        }
        applied += 1;
    }
    ReplayReport {
        applied,
        final_rule_count: engine.len(),
        error: None,
    }
}

impl Default for AuditLog {
    fn default() -> Self { Self::new() }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// BLAKE3 sobre el entry en forma canónica (sha=[0;32]). Hash y CAS storage
/// ven los mismos bytes, así que `arje_cas::store(canonical)` devuelve el
/// mismo hash que `compute_sha(entry)`.
fn compute_sha(entry: &AuditEntry) -> [u8; 32] {
    let bytes = canonical_bytes(entry);
    arje_cas::blake3_of(&bytes)
}

/// Forma canónica: el entry serializado JSON con `sha = [0; 32]`.
/// JSON sin pretty-print es determinístico para nuestros tipos.
fn canonical_bytes(entry: &AuditEntry) -> Vec<u8> {
    let canonical = AuditEntry {
        sha: [0u8; 32],
        ..entry.clone()
    };
    serde_json::to_vec(&canonical).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_links_consecutive_entries() {
        let mut log = AuditLog::new();
        let e1 = log.append(AuditAction::RemoveRule { rule_id: Ulid::new() });
        let e2 = log.append(AuditAction::RemoveRule { rule_id: Ulid::new() });
        assert!(e1.prev_sha.is_none());
        assert_eq!(e2.prev_sha, Some(e1.sha));
        assert_ne!(e1.sha, e2.sha);
    }

    #[test]
    fn seq_monotonic() {
        let mut log = AuditLog::new();
        let e1 = log.append(AuditAction::RemoveRule { rule_id: Ulid::new() });
        let e2 = log.append(AuditAction::RemoveRule { rule_id: Ulid::new() });
        assert_eq!(e2.seq, e1.seq + 1);
    }

    #[test]
    fn cap_evicts_oldest() {
        let mut log = AuditLog::with_cap(3);
        for _ in 0..5 {
            log.append(AuditAction::RemoveRule { rule_id: Ulid::new() });
        }
        assert_eq!(log.len(), 3);
        // El primer seq superviviente debe ser 2.
        assert_eq!(log.recent(0).next().unwrap().seq, 2);
    }

    // ---------- Tests de integración con CAS real (en directorio temporal) ----------

    use arje_brain_rules::engine::RuleEngine;
    use std::sync::Mutex;

    /// Lock para serializar tests que mutan ENTE_CAS_ROOT (test threads
    /// comparten env vars). Sin esto, dos tests en paralelo pisan el path.
    static CAS_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_cas<F: FnOnce()>(f: F) {
        let _guard = CAS_TEST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("ente-cas-test-{}", Ulid::new()));
        std::env::set_var("ENTE_CAS_ROOT", &dir);
        let _cleanup = scopeguard(&dir);
        f();
    }

    fn scopeguard(dir: &std::path::Path) -> impl Drop + '_ {
        struct G<'a>(&'a std::path::Path);
        impl<'a> Drop for G<'a> {
            fn drop(&mut self) {
                std::env::remove_var("ENTE_CAS_ROOT");
                let _ = std::fs::remove_dir_all(self.0);
            }
        }
        G(dir)
    }

    fn dummy_crystal(ant: EventKind, con: EventKind) -> Crystal {
        Crystal {
            antecedent: ant,
            consequent: con,
            conditional_prob: 0.9,
            pmi: 1.5,
            support: 7,
            gap_stats: None,
        }
    }

    use arje_brain_rules::rules::EventKind;

    #[test]
    fn flush_round_trip_preserves_chain() {
        with_temp_cas(|| {
            let mut log = AuditLog::new();
            let id1 = Ulid::new();
            let id2 = Ulid::new();
            log.append(AuditAction::PromoteCrystal {
                rule_id: id1,
                crystal: dummy_crystal(EventKind::EnteSpawned, EventKind::EnteDied),
            });
            log.append(AuditAction::PromoteCrystal {
                rule_id: id2,
                crystal: dummy_crystal(EventKind::BusAnnounce, EventKind::BusInvoke),
            });
            log.append(AuditAction::RemoveRule { rule_id: id1 });

            assert_eq!(log.flush_to_cas().unwrap(), 3);
            let head = log.last_flushed_sha().expect("head set");
            let report = verify_chain_from_cas(head);
            assert!(report.error.is_none(), "verification failed: {:?}", report.error);
            assert_eq!(report.verified, 3);
        });
    }

    #[test]
    fn replay_reconstructs_engine_state() {
        with_temp_cas(|| {
            let mut log = AuditLog::new();
            let id1: Ulid = "01KQR3000000000000000000A1".parse().unwrap();
            let id2: Ulid = "01KQR3000000000000000000A2".parse().unwrap();
            let id3: Ulid = "01KQR3000000000000000000A3".parse().unwrap();
            log.append(AuditAction::PromoteCrystal {
                rule_id: id1,
                crystal: dummy_crystal(EventKind::EnteSpawned, EventKind::EnteDied),
            });
            log.append(AuditAction::PromoteCrystal {
                rule_id: id2,
                crystal: dummy_crystal(EventKind::BusAnnounce, EventKind::BusInvoke),
            });
            log.append(AuditAction::PromoteCrystal {
                rule_id: id3,
                crystal: dummy_crystal(EventKind::DeviceAdded, EventKind::DeviceRemoved),
            });
            log.append(AuditAction::RemoveRule { rule_id: id2 });
            log.flush_to_cas().unwrap();
            let head = log.last_flushed_sha().unwrap();

            let mut engine = RuleEngine::empty();
            let rep = replay_chain(head, &mut engine);
            assert!(rep.error.is_none(), "replay error: {:?}", rep.error);
            assert_eq!(rep.applied, 4);
            assert_eq!(engine.len(), 2, "id2 should be removed, id1 + id3 remain");
            // Ulids preservados
            let ids: Vec<Ulid> = engine.rules().map(|r| r.id).collect();
            assert!(ids.contains(&id1));
            assert!(!ids.contains(&id2));
            assert!(ids.contains(&id3));
        });
    }

    // ---------- AuditFilter ----------

    fn killente_action() -> AuditAction {
        AuditAction::KillEnte { caller: Ulid::new(), target: Ulid::new(), signal: 15 }
    }

    fn powermgmt_action() -> AuditAction {
        AuditAction::PowerMgmt {
            caller: None, peer_pid: 42, kind: "reboot".into(), interactive: false,
        }
    }

    #[test]
    fn filter_identity_acepta_todo() {
        let mut log = AuditLog::new();
        log.append(AuditAction::RemoveRule { rule_id: Ulid::new() });
        log.append(killente_action());
        log.append(powermgmt_action());
        let f = AuditFilter::default();
        assert!(f.is_identity());
        assert_eq!(log.recent_filtered(0, &f).len(), 3);
    }

    #[test]
    fn filter_por_kind_unico() {
        let mut log = AuditLog::new();
        log.append(AuditAction::RemoveRule { rule_id: Ulid::new() });
        log.append(killente_action());
        log.append(powermgmt_action());
        log.append(killente_action());
        let f = AuditFilter { kinds: vec![AuditActionKind::KillEnte], since_seq: None };
        let out = log.recent_filtered(0, &f);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|e| e.action.kind() == AuditActionKind::KillEnte));
    }

    #[test]
    fn filter_por_kinds_multiples() {
        let mut log = AuditLog::new();
        log.append(AuditAction::RemoveRule { rule_id: Ulid::new() }); // pasa
        log.append(killente_action()); // pasa
        log.append(powermgmt_action()); // descarta
        log.append(AuditAction::BrainInhibit { reason: "x".into() }); // descarta
        let f = AuditFilter {
            kinds: vec![AuditActionKind::RemoveRule, AuditActionKind::KillEnte],
            since_seq: None,
        };
        let out = log.recent_filtered(0, &f);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn filter_since_seq_estricto() {
        let mut log = AuditLog::new();
        for _ in 0..5 {
            log.append(AuditAction::RemoveRule { rule_id: Ulid::new() });
        }
        // since_seq = 2 → seq > 2, es decir seq ∈ {3, 4}
        let f = AuditFilter { kinds: vec![], since_seq: Some(2) };
        let out = log.recent_filtered(0, &f);
        let seqs: Vec<u64> = out.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![3, 4]);
    }

    #[test]
    fn filter_combina_kind_y_since_seq() {
        let mut log = AuditLog::new();
        log.append(AuditAction::RemoveRule { rule_id: Ulid::new() }); // seq=0, RR
        log.append(killente_action());                                  // seq=1, KE — descarta (seq)
        log.append(AuditAction::RemoveRule { rule_id: Ulid::new() }); // seq=2, RR — descarta (seq)
        log.append(killente_action());                                  // seq=3, KE ✓
        log.append(AuditAction::RemoveRule { rule_id: Ulid::new() }); // seq=4, RR — descarta (kind)
        log.append(killente_action());                                  // seq=5, KE ✓
        let f = AuditFilter {
            kinds: vec![AuditActionKind::KillEnte],
            since_seq: Some(2),
        };
        let out = log.recent_filtered(0, &f);
        let seqs: Vec<u64> = out.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![3, 5]);
    }

    #[test]
    fn filter_respeta_limit_sobre_resultado_filtrado() {
        let mut log = AuditLog::new();
        // 6 KillEnte intercalados con 6 RemoveRule. Filtramos KE, pedimos limit=3.
        for _ in 0..6 {
            log.append(AuditAction::RemoveRule { rule_id: Ulid::new() });
            log.append(killente_action());
        }
        let f = AuditFilter { kinds: vec![AuditActionKind::KillEnte], since_seq: None };
        let out = log.recent_filtered(3, &f);
        assert_eq!(out.len(), 3);
        // Los 3 KE más recientes son seq 7, 9, 11 (cada KE va detrás de su RR).
        let seqs: Vec<u64> = out.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![7, 9, 11]);
    }

    #[test]
    fn action_kind_str_round_trip_cubre_todas() {
        for k in [
            AuditActionKind::PromoteCrystal,
            AuditActionKind::RemoveRule,
            AuditActionKind::LoadRulesFile,
            AuditActionKind::KillEnte,
            AuditActionKind::SpawnCardFromDisk,
            AuditActionKind::RunCard,
            AuditActionKind::BrainInhibit,
            AuditActionKind::PowerMgmt,
        ] {
            assert_eq!(AuditActionKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(AuditActionKind::parse("desconocido"), None);
    }

    #[test]
    fn replay_after_eviction_still_works() {
        with_temp_cas(|| {
            // Cap pequeño: la mayoría de entries se evictan de memoria pero
            // siguen en CAS. Replay debe poder reconstruir desde CAS solo.
            let mut log = AuditLog::with_cap(2);
            let mut ids = Vec::new();
            for _ in 0..6 {
                let id = Ulid::new();
                ids.push(id);
                log.append(AuditAction::PromoteCrystal {
                    rule_id: id,
                    crystal: dummy_crystal(EventKind::EnteSpawned, EventKind::EnteDied),
                });
                log.flush_to_cas().unwrap();
            }
            assert_eq!(log.len(), 2, "cap eviction limita memoria");
            let head = log.last_flushed_sha().unwrap();

            let mut engine = RuleEngine::empty();
            let rep = replay_chain(head, &mut engine);
            assert!(rep.error.is_none());
            assert_eq!(rep.applied, 6);
            assert_eq!(engine.len(), 6);
        });
    }
}
