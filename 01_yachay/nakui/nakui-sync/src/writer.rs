//! El escritor autoritativo: dueño único del [`EventLog`] y del store
//! canónico.
//!
//! Es el punto de serialización del modelo multi-cliente. Toda mutación
//! pasa por [`Writer::commit`], que valida (dry-run del kernel), anexa la
//! entrada al log asignándole un `seq` monótono, materializa el cambio en
//! el store autoritativo, y devuelve el [`Commit`] con las entradas
//! anexadas para que el transporte las difunda.
//!
//! Un solo escritor ⇒ orden total gratis: no hacen falta CRDTs ni
//! consenso para la coherencia de un ERP (la partida doble exige
//! serializabilidad estricta, justamente lo que un escritor único da).
//!
//! Es la mudanza del cuerpo de `NakuiBackend` a una capa UI-agnóstica:
//! `nakui-backend` ahora es un cliente co-locado delgado sobre esto.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};
use uuid::Uuid;

use nakui_core::delta::{FieldOp, FieldPath};
use nakui_core::event_log::{
    execute_and_log_with_recovery, replay_with_snapshot_into, EventLog, LogEntry, Snapshot,
};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};

use crate::intent::{Commit, Intent};

/// Path del snapshot sibling del log:
/// `nakui-ui-state.jsonl` ↔ `nakui-ui-state.snap.json`.
pub fn snapshot_path_for(log_path: &Path) -> PathBuf {
    log_path.with_extension("snap.json")
}

/// Si el log file tiene >= `threshold` entries, captura un snapshot del
/// store actual y compacta el log dejando 1 entry como anchor del cursor.
/// Idempotente abajo del threshold o con < 2 entries.
pub fn maybe_compact_log(
    log: &mut EventLog,
    snap_path: &Path,
    store: &MemoryStore,
    threshold: usize,
) -> Result<Option<String>, String> {
    if threshold == 0 {
        return Ok(None);
    }
    let entry_count = log
        .entries()
        .map_err(|e| format!("read entries: {e}"))?
        .len();
    if entry_count < threshold || entry_count < 2 {
        return Ok(None);
    }
    let snap_seq = log.next_seq() - 1;
    let through = log.next_seq() - 2;
    let snap = Snapshot::from_memory_store(store, snap_seq);
    snap.write(snap_path)
        .map_err(|e| format!("write snapshot {}: {e}", snap_path.display()))?;
    log.compact_through(through)
        .map_err(|e| format!("compact_through({through}): {e}"))?;
    Ok(Some(format!(
        "auto-compact: snapshot @ seq {snap_seq}, {} entries dropped (1 anchor kept)",
        entry_count - 1
    )))
}

/// Estado inicial tras abrir el log + cargar snapshot + replay. Lo
/// devuelve [`Writer::open`] para que el caller acumule mensajes al banner.
pub struct OpenStatus {
    /// Mensaje "log X cargado: next_seq=N (snapshot @ seq K)" o similar.
    pub init_toast: Option<String>,
    /// Errores no-fatales acumulados (snapshot corrupto, replay falló, log
    /// inaccesible). El escritor igualmente queda usable (eventualmente
    /// in-memory only si el log no abrió).
    pub load_error: Option<String>,
}

/// Escritor autoritativo Nakui: WAL persistente + store canónico +
/// executors por módulo + auto-compaction.
///
/// El store y el log se exponen tras `Arc<Mutex>` para que un cliente
/// co-locado (misma máquina que el escritor) pueda leer el store
/// autoritativo sin bloquear el path de commit. El propio `Writer` se
/// envuelve en un `Mutex` (en el transporte) para serializar los commits.
pub struct Writer {
    /// Store autoritativo. Compartido por handle con clientes co-locados
    /// para reads sin tomar el lock del escritor.
    store: Arc<Mutex<MemoryStore>>,
    /// Log persistente. `None` si abrir falló — degrada a in-memory only
    /// (los writes no se persisten ni se difunden; los reads siguen).
    log: Option<Arc<Mutex<EventLog>>>,
    /// Executors indexados por `module.id`.
    executors: BTreeMap<String, Arc<Executor>>,
    /// Path del snapshot (cacheado del init).
    snap_path: PathBuf,
    /// Threshold de auto-compaction. `0` = desactivado.
    snapshot_threshold: usize,
    /// Contador de writes desde el último compact.
    writes_since_compact: u64,
}

impl Writer {
    /// Abre/crea el log en `log_path`, intenta cargar el snapshot sibling,
    /// hace replay al store. Si el log no abre, degrada a in-memory only.
    /// Ningún error es fatal — los mensajes van en `OpenStatus`.
    pub fn open(
        log_path: PathBuf,
        snapshot_threshold: usize,
        executors: BTreeMap<String, Arc<Executor>>,
    ) -> (Self, OpenStatus) {
        let snap_path = snapshot_path_for(&log_path);
        let mut store = MemoryStore::new();
        let mut init_toast: Option<String> = None;
        let mut load_error: Option<String> = None;

        let snapshot: Option<Snapshot> = match Snapshot::load(&snap_path) {
            Ok(s) => s,
            Err(e) => {
                load_error = Some(format!("snapshot {}: {e} — full replay", snap_path.display()));
                None
            }
        };

        let log = match EventLog::open(&log_path) {
            Ok(mut log) => match replay_with_snapshot_into(&log, snapshot.as_ref(), &mut store) {
                Ok(()) => {
                    let n = log.next_seq();
                    let from_snap = snapshot
                        .as_ref()
                        .map(|s| format!(" (snapshot @ seq {})", s.seq))
                        .unwrap_or_default();
                    init_toast = Some(if n > 0 {
                        format!("log {} cargado: next_seq={n}{from_snap}", log_path.display())
                    } else {
                        format!("log nuevo en {}", log_path.display())
                    });

                    match maybe_compact_log(&mut log, &snap_path, &store, snapshot_threshold) {
                        Ok(Some(msg)) => {
                            let prev = init_toast.unwrap_or_default();
                            init_toast = Some(format!("{prev}; {msg}"));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let msg = format!("auto-compact: {e}");
                            load_error = Some(match load_error {
                                Some(p) => format!("{p}; {msg}"),
                                None => msg,
                            });
                        }
                    }
                    Some(Arc::new(Mutex::new(log)))
                }
                Err(e) => {
                    let msg =
                        format!("replay del log {} falló: {e} — running in-memory", log_path.display());
                    load_error = Some(match load_error {
                        Some(p) => format!("{p}; {msg}"),
                        None => msg,
                    });
                    None
                }
            },
            Err(e) => {
                let msg = format!("abrir log {}: {e} — running in-memory only", log_path.display());
                load_error = Some(match load_error {
                    Some(p) => format!("{p}; {msg}"),
                    None => msg,
                });
                None
            }
        };

        let writer = Writer {
            store: Arc::new(Mutex::new(store)),
            log,
            executors,
            snap_path,
            snapshot_threshold,
            writes_since_compact: 0,
        };
        (writer, OpenStatus { init_toast, load_error })
    }

    /// Handle al store autoritativo para reads co-locados (un cliente en
    /// la misma máquina lee de acá sin tocar el lock del escritor).
    pub fn store_handle(&self) -> Arc<Mutex<MemoryStore>> {
        self.store.clone()
    }

    /// El executor de un módulo, si está cargado. Lo usa la UI para
    /// derivar el grafo de morfismos sin que esos tipos vivan acá.
    pub fn executor(&self, module_id: &str) -> Option<Arc<Executor>> {
        self.executors.get(module_id).cloned()
    }

    /// ¿Hay log activo? `false` ⇒ modo in-memory (writes no persisten ni
    /// se difunden).
    pub fn has_log(&self) -> bool {
        self.log.is_some()
    }

    /// Punto de entrada autoritativo: valida, ordena, materializa y
    /// devuelve el delta.
    pub fn commit(&mut self, intent: Intent) -> Result<Commit, String> {
        match intent {
            Intent::Seed { entity, data } => self.commit_seed(&entity, data),
            Intent::Update { entity, id, set, clear } => self.commit_update(&entity, id, set, clear),
            Intent::Delete { entity, id } => self.commit_delete(&entity, id),
            Intent::Morphism { module_id, name, inputs, params } => {
                self.commit_morphism(&module_id, &name, inputs, params)
            }
        }
    }

    // ---- helpers internos ------------------------------------------------

    fn next_seq(&self) -> Result<u64, String> {
        let log_arc = self.log.as_ref().expect("checked by caller").clone();
        let log = log_arc.lock().map_err(|_| "log mutex envenenado".to_string())?;
        Ok(log.next_seq())
    }

    fn append_log(&self, entry: LogEntry) -> Result<(), String> {
        let Some(log_arc) = self.log.as_ref() else {
            return Ok(()); // in-memory mode, no log.
        };
        let mut log = log_arc.lock().map_err(|_| "log mutex envenenado".to_string())?;
        log.append(entry).map_err(|e| format!("append al log: {e}"))
    }

    fn commit_seed(&mut self, entity: &str, data: Map<String, Value>) -> Result<Commit, String> {
        let id = Uuid::new_v4();
        // El `id` de la entity = la clave del store. Inyectarlo en el record
        // hace que `data.id` y la clave coincidan.
        let mut data = data;
        data.insert("id".to_string(), Value::String(id.to_string()));
        let value = Value::Object(data);

        let mut entries = Vec::new();
        if self.has_log() {
            let seq = self.next_seq()?;
            let entry = LogEntry::Seed {
                seq,
                entity: entity.to_string(),
                id,
                data: value.clone(),
                schema_hash: None,
            };
            self.append_log(entry.clone())?;
            entries.push(entry);
        }
        self.store
            .lock()
            .map_err(|_| "store mutex envenenado".to_string())?
            .seed(entity, id, value);
        let post_status = self.tick_compact();
        Ok(Commit { entries, primary_id: Some(id), changed: 1, post_status })
    }

    fn commit_update(
        &mut self,
        entity: &str,
        id: Uuid,
        set: Map<String, Value>,
        clear: Vec<String>,
    ) -> Result<Commit, String> {
        if set.is_empty() && clear.is_empty() {
            return Ok(Commit::no_op(id));
        }
        let mut ops: Vec<FieldOp> = set
            .iter()
            .map(|(field, value)| FieldOp::Set {
                path: FieldPath { entity: entity.to_string(), id, field: field.clone() },
                value: value.clone(),
            })
            .collect();
        for field in &clear {
            ops.push(FieldOp::Clear {
                path: FieldPath { entity: entity.to_string(), id, field: field.clone() },
            });
        }
        let changed = set.len() + clear.len();

        let mut entries = Vec::new();
        if self.has_log() {
            let seq = self.next_seq()?;
            let mut params = serde_json::Map::new();
            params.insert("entity".into(), json!(entity));
            params.insert("id".into(), json!(id.to_string()));
            if !set.is_empty() {
                params.insert("fields".into(), Value::Object(set.clone()));
            }
            if !clear.is_empty() {
                params.insert("cleared".into(), Value::Array(clear.iter().map(|s| json!(s)).collect()));
            }
            let entry = LogEntry::Morphism {
                seq,
                morphism: "ui.edit_record".into(),
                inputs: Default::default(),
                params: Value::Object(params),
                ops: ops.clone(),
                schema_hash: None,
            };
            self.append_log(entry.clone())?;
            entries.push(entry);
        }
        self.store
            .lock()
            .map_err(|_| "store mutex envenenado".to_string())?
            .apply(&ops)
            .map_err(|e| format!("apply edit ops: {e}"))?;
        let post_status = self.tick_compact();
        Ok(Commit { entries, primary_id: Some(id), changed, post_status })
    }

    fn commit_delete(&mut self, entity: &str, id: Uuid) -> Result<Commit, String> {
        let ops = vec![FieldOp::Delete { entity: entity.to_string(), id }];
        let mut entries = Vec::new();
        if self.has_log() {
            let seq = self.next_seq()?;
            let entry = LogEntry::Morphism {
                seq,
                morphism: "ui.delete_record".into(),
                inputs: Default::default(),
                params: json!({ "entity": entity, "id": id.to_string() }),
                ops: ops.clone(),
                schema_hash: None,
            };
            self.append_log(entry.clone())?;
            entries.push(entry);
        }
        self.store
            .lock()
            .map_err(|_| "store mutex envenenado".to_string())?
            .apply(&ops)
            .map_err(|e| format!("apply Delete: {e}"))?;
        let post_status = self.tick_compact();
        Ok(Commit { entries, primary_id: Some(id), changed: 1, post_status })
    }

    fn commit_morphism(
        &mut self,
        module_id: &str,
        name: &str,
        inputs: Vec<(String, Uuid)>,
        params: Value,
    ) -> Result<Commit, String> {
        let executor = self
            .executors
            .get(module_id)
            .ok_or_else(|| {
                format!(
                    "módulo '{module_id}' no tiene executor nakui (falta nakui_module_dir o falló la carga)"
                )
            })?
            .clone();
        let log_arc = self
            .log
            .as_ref()
            .ok_or_else(|| "morphism requiere event log activo".to_string())?
            .clone();

        let inputs_ref: Vec<(&str, Uuid)> = inputs.iter().map(|(r, id)| (r.as_str(), *id)).collect();
        let schema_hash = executor.schema_hash(name);
        let params_for_entry = params.clone();

        let mut log = log_arc.lock().map_err(|_| "log mutex envenenado".to_string())?;
        let mut store = self.store.lock().map_err(|_| "store mutex envenenado".to_string())?;

        // Capturamos el seq que `execute_and_log_with_recovery` va a usar
        // (es `log.next_seq()` justo antes del append) para reconstruir la
        // entrada exacta que se logueó y poder difundirla.
        let seq = log.next_seq();
        let ops = execute_and_log_with_recovery(
            &executor,
            &mut *store,
            &mut *log,
            name,
            &inputs_ref,
            params,
        )
        .map_err(|e| format!("{e}"))?;
        drop(store);
        drop(log);

        let entry = LogEntry::Morphism {
            seq,
            morphism: name.to_string(),
            inputs: inputs.iter().map(|(r, id)| (r.clone(), *id)).collect(),
            params: params_for_entry,
            ops: ops.clone(),
            schema_hash,
        };
        let post_status = self.tick_compact();
        Ok(Commit { entries: vec![entry], primary_id: None, changed: ops.len(), post_status })
    }

    /// Increment + check del threshold; si cruza, captura snapshot +
    /// compacta. Devuelve el mensaje de status.
    fn tick_compact(&mut self) -> Option<String> {
        if self.snapshot_threshold == 0 {
            return None;
        }
        self.writes_since_compact += 1;
        if self.writes_since_compact < self.snapshot_threshold as u64 {
            return None;
        }
        let log_arc = self.log.as_ref()?.clone();
        let mut log = match log_arc.lock() {
            Ok(l) => l,
            Err(_) => return Some("auto-compact skip: log mutex envenenado".into()),
        };
        let store = match self.store.lock() {
            Ok(s) => s,
            Err(_) => return Some("auto-compact skip: store mutex envenenado".into()),
        };
        match maybe_compact_log(&mut log, &self.snap_path, &store, self.snapshot_threshold) {
            Ok(Some(msg)) => {
                self.writes_since_compact = 0;
                Some(msg)
            }
            Ok(None) => {
                self.writes_since_compact = 0;
                None
            }
            Err(e) => Some(format!("auto-compact: {e}")),
        }
    }
}
