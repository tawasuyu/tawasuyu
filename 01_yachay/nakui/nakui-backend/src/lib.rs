//! Implementación de [`MetaBackend`] para Nakui — compone
//! `nakui_core::store::MemoryStore`, `event_log::EventLog`, los
//! `Executor`s por módulo, y la lógica de auto-compaction.
//!
//! Es lo único que sabe de Nakui en el binario nuevo. El widget de
//! UI no toca ninguno de estos tipos directamente.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use uuid::Uuid;

use nahual_meta_runtime::{MetaBackend, WriteOutcome};
use nakui_core::delta::{FieldOp, FieldPath};
use nakui_core::event_log::{
    execute_and_log_with_recovery, replay_with_snapshot_into, EventLog, LogEntry, Snapshot,
};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};

/// Path del snapshot sibling del log:
/// `nakui-ui-state.jsonl` ↔ `nakui-ui-state.snap.json`.
pub fn snapshot_path_for(log_path: &Path) -> PathBuf {
    log_path.with_extension("snap.json")
}

/// Si el log file tiene >= `threshold` entries, captura un snapshot
/// del store actual y compacta el log dejando 1 entry como anchor del
/// cursor. Idempotente abajo del threshold o con < 2 entries.
///
/// Ver el doc original (commit del runtime compact) para detalles
/// sobre el anchor invariant. Re-locado acá porque es detalle del
/// backend, no del widget.
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

/// Estado inicial del backend tras abrir el log + cargar snapshot
/// + replay. Devuelto desde [`NakuiBackend::open`] para que el caller
/// (typicamente `main.rs`) acumule mensajes informativos al banner.
pub struct OpenStatus {
    /// Mensaje "log X cargado: next_seq=N (snapshot @ seq K)" o similar.
    pub init_toast: Option<String>,
    /// Errores no-fatales acumulados (snapshot corrupto, replay falló,
    /// log inaccesible). El backend igualmente queda usable
    /// (eventualmente in-memory only si log_arc es None).
    pub load_error: Option<String>,
}

/// Backend Nakui: WAL persistente + MemoryStore + executors por
/// módulo + auto-compaction.
///
/// Implementa [`MetaBackend`] proyectando cada operación al
/// pipeline de nakui-core (compute → log → apply para morphisms;
/// log → apply para seed/edit/delete).
pub struct NakuiBackend {
    /// Store compartido (Arc para que el render pueda hacer reads
    /// sin bloquear writes; el lock interno serializa).
    store: Arc<Mutex<MemoryStore>>,
    /// Log persistente. `None` si abrir falló — el backend degrada
    /// a in-memory only (writes no se persisten; reads siguen).
    event_log: Option<Arc<Mutex<EventLog>>>,
    /// Executors indexados por `module.id`. Los módulos sin
    /// `nakui_module_dir` no aparecen acá; sus llamadas a
    /// `morphism()` rebotan con error claro.
    executors: BTreeMap<String, Arc<Executor>>,
    /// Path del snapshot (cacheado del init).
    snap_path: PathBuf,
    /// Threshold de auto-compaction. `0` = desactivado.
    snapshot_threshold: usize,
    /// Contador de writes desde el último compact. Se resetea al
    /// disparar compact.
    writes_since_compact: u64,
}

impl NakuiBackend {
    /// Abre/crea el log en `log_path`, intenta cargar el snapshot
    /// sibling, hace replay al store. Si el log no abre, degrada a
    /// in-memory only. Ningún error es fatal — los mensajes se
    /// devuelven en `OpenStatus` para que el caller los acumule.
    ///
    /// `executors` se pasan ya cargados (la lógica de qué módulos
    /// declaran `nakui_module_dir` es responsabilidad del caller).
    pub fn open(
        log_path: PathBuf,
        snapshot_threshold: usize,
        executors: BTreeMap<String, Arc<Executor>>,
    ) -> (Self, OpenStatus) {
        let snap_path = snapshot_path_for(&log_path);
        let mut store = MemoryStore::new();
        let mut init_toast: Option<String> = None;
        let mut load_error: Option<String> = None;

        // Cargar snapshot (si existe).
        let snapshot: Option<Snapshot> = match Snapshot::load(&snap_path) {
            Ok(s) => s,
            Err(e) => {
                load_error = Some(format!(
                    "snapshot {}: {e} — full replay",
                    snap_path.display()
                ));
                None
            }
        };

        let event_log = match EventLog::open(&log_path) {
            Ok(mut log) => {
                match replay_with_snapshot_into(&log, snapshot.as_ref(), &mut store) {
                    Ok(()) => {
                        let n = log.next_seq();
                        let from_snap = snapshot
                            .as_ref()
                            .map(|s| format!(" (snapshot @ seq {})", s.seq))
                            .unwrap_or_default();
                        if n > 0 {
                            init_toast = Some(format!(
                                "log {} cargado: next_seq={n}{from_snap}",
                                log_path.display()
                            ));
                        } else {
                            init_toast = Some(format!("log nuevo en {}", log_path.display()));
                        }

                        // Auto-compact si pasamos el threshold.
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
                        let msg = format!(
                            "replay del log {} falló: {e} — running in-memory",
                            log_path.display()
                        );
                        load_error = Some(match load_error {
                            Some(p) => format!("{p}; {msg}"),
                            None => msg,
                        });
                        None
                    }
                }
            }
            Err(e) => {
                let msg = format!(
                    "abrir log {}: {e} — running in-memory only",
                    log_path.display()
                );
                load_error = Some(match load_error {
                    Some(p) => format!("{p}; {msg}"),
                    None => msg,
                });
                None
            }
        };

        let backend = NakuiBackend {
            store: Arc::new(Mutex::new(store)),
            event_log,
            executors,
            snap_path,
            snapshot_threshold,
            writes_since_compact: 0,
        };
        (
            backend,
            OpenStatus {
                init_toast,
                load_error,
            },
        )
    }

    /// Increment + check del threshold; si cruza, captura snapshot
    /// + compacta. Devuelve el mensaje de status para concatenar al
    /// `WriteOutcome.post_status`.
    fn tick_compact(&mut self) -> Option<String> {
        if self.snapshot_threshold == 0 {
            return None;
        }
        self.writes_since_compact += 1;
        if self.writes_since_compact < self.snapshot_threshold as u64 {
            return None;
        }
        let log_arc = self.event_log.as_ref()?.clone();
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

    /// Helper: append una entry al log si está disponible. Errors si
    /// el lock falla o el append falla.
    fn append_log(&self, entry: LogEntry) -> Result<(), String> {
        let Some(log_arc) = self.event_log.as_ref() else {
            return Ok(()); // in-memory mode, no log.
        };
        let mut log = log_arc
            .lock()
            .map_err(|_| "log mutex envenenado".to_string())?;
        log.append(entry).map_err(|e| format!("append al log: {e}"))
    }

    /// Deriva el grafo de morfismos del módulo `module_id` a partir de
    /// su `Executor`: cada morfismo es un nodo (con los tokens que lee y
    /// escribe), y cada par escritura→lectura del mismo token es una
    /// arista de flujo de datos. `None` si el módulo no tiene executor
    /// (no declara `nakui_module_dir` o falló la carga).
    pub fn morphism_graph(&self, module_id: &str) -> Option<MorphismGraphData> {
        let exec = self.executors.get(module_id)?;
        let g = &exec.graph;
        let order = g.topological_order();
        let nodes: Vec<MorphismNode> = order
            .iter()
            .map(|name| MorphismNode {
                name: name.clone(),
                reads: g.morphism_reads(name).to_vec(),
                writes: g.morphism_writes(name).to_vec(),
            })
            .collect();
        let mut edges: Vec<DataFlowEdge> = Vec::new();
        for name in &order {
            for token in g.morphism_writes(name) {
                for reader in g.readers_of(token) {
                    // Self-loops (un morfismo que lee lo que escribe) no
                    // aportan al grafo de cascada — se omiten.
                    if reader != name {
                        edges.push(DataFlowEdge {
                            from: name.clone(),
                            to: reader.clone(),
                            token: token.clone(),
                        });
                    }
                }
            }
        }
        Some(MorphismGraphData { nodes, edges })
    }
}

/// Un nodo del grafo de morfismos: el morfismo y los tokens que lee
/// (pins de entrada) / escribe (pins de salida).
#[derive(Debug, Clone)]
pub struct MorphismNode {
    pub name: String,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
}

/// Una arista de flujo de datos: el morfismo `from` escribe `token`,
/// que el morfismo `to` lee — por eso `to` está aguas abajo de `from`.
#[derive(Debug, Clone)]
pub struct DataFlowEdge {
    pub from: String,
    pub to: String,
    pub token: String,
}

/// El grafo de morfismos de un módulo: nodos (morfismos con sus tokens)
/// + aristas de flujo de datos.
#[derive(Debug, Clone)]
pub struct MorphismGraphData {
    pub nodes: Vec<MorphismNode>,
    pub edges: Vec<DataFlowEdge>,
}

impl MetaBackend for NakuiBackend {
    fn list_records(&self, entity: &str) -> Vec<(Uuid, Value)> {
        let store = match self.store.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let it = match store.iter() {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<(Uuid, Value)> = it
            .filter(|(e, _, _)| e == entity)
            .map(|(_, id, v)| (id, v))
            .collect();
        out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        out
    }

    fn load_record(&self, entity: &str, id: Uuid) -> Option<Value> {
        self.store.lock().ok()?.load(entity, id)
    }

    fn seed(
        &mut self,
        entity: &str,
        data: serde_json::Map<String, Value>,
    ) -> Result<WriteOutcome, String> {
        let id = Uuid::new_v4();
        // El `id` de la entity = la clave del store. Inyectarlo en el
        // record hace que `data.id` y la clave coincidan — los schemas
        // Nickel suelen declarar `id | String` y los morfismos lo leen.
        let mut data = data;
        data.insert("id".to_string(), Value::String(id.to_string()));
        let value = Value::Object(data);
        // WAL: log primero, store después.
        if self.event_log.is_some() {
            let seq = {
                let log_arc = self.event_log.as_ref().expect("checked above").clone();
                let log = log_arc
                    .lock()
                    .map_err(|_| "log mutex envenenado".to_string())?;
                log.next_seq()
            };
            self.append_log(LogEntry::Seed {
                seq,
                entity: entity.to_string(),
                id,
                data: value.clone(),
                schema_hash: None,
            })?;
        }
        let mut store = self
            .store
            .lock()
            .map_err(|_| "store mutex envenenado".to_string())?;
        store.seed(entity, id, value);
        drop(store);
        let post_status = self.tick_compact();
        Ok(WriteOutcome {
            id: Some(id),
            changed: 1,
            post_status,
        })
    }

    fn update(
        &mut self,
        entity: &str,
        id: Uuid,
        set: serde_json::Map<String, Value>,
        clear: Vec<String>,
    ) -> Result<WriteOutcome, String> {
        if set.is_empty() && clear.is_empty() {
            return Ok(WriteOutcome::no_change(id));
        }
        // Construir ops: Set primero, después Clear (la sem es
        // independiente del orden, pero estable mejor para diff).
        let mut ops: Vec<FieldOp> = set
            .iter()
            .map(|(field, value)| FieldOp::Set {
                path: FieldPath {
                    entity: entity.to_string(),
                    id,
                    field: field.clone(),
                },
                value: value.clone(),
            })
            .collect();
        for field in &clear {
            ops.push(FieldOp::Clear {
                path: FieldPath {
                    entity: entity.to_string(),
                    id,
                    field: field.clone(),
                },
            });
        }
        let changed = set.len() + clear.len();

        // Log: Morphism { ui.edit_record, ops, params: {entity, id, fields, cleared} }.
        if self.event_log.is_some() {
            let seq = {
                let log_arc = self.event_log.as_ref().expect("checked").clone();
                let log = log_arc
                    .lock()
                    .map_err(|_| "log mutex envenenado".to_string())?;
                log.next_seq()
            };
            let mut params = serde_json::Map::new();
            params.insert("entity".into(), json!(entity));
            params.insert("id".into(), json!(id.to_string()));
            if !set.is_empty() {
                params.insert("fields".into(), Value::Object(set.clone()));
            }
            if !clear.is_empty() {
                params.insert(
                    "cleared".into(),
                    Value::Array(clear.iter().map(|s| json!(s)).collect()),
                );
            }
            self.append_log(LogEntry::Morphism {
                seq,
                morphism: "ui.edit_record".into(),
                inputs: Default::default(),
                params: Value::Object(params),
                ops: ops.clone(),
                schema_hash: None,
            })?;
        }
        let mut store = self
            .store
            .lock()
            .map_err(|_| "store mutex envenenado".to_string())?;
        store
            .apply(&ops)
            .map_err(|e| format!("apply edit ops: {e}"))?;
        drop(store);
        let post_status = self.tick_compact();
        Ok(WriteOutcome {
            id: Some(id),
            changed,
            post_status,
        })
    }

    fn delete(&mut self, entity: &str, id: Uuid) -> Result<WriteOutcome, String> {
        let ops = vec![FieldOp::Delete {
            entity: entity.to_string(),
            id,
        }];
        if self.event_log.is_some() {
            let seq = {
                let log_arc = self.event_log.as_ref().expect("checked").clone();
                let log = log_arc
                    .lock()
                    .map_err(|_| "log mutex envenenado".to_string())?;
                log.next_seq()
            };
            self.append_log(LogEntry::Morphism {
                seq,
                morphism: "ui.delete_record".into(),
                inputs: Default::default(),
                params: json!({ "entity": entity, "id": id.to_string() }),
                ops: ops.clone(),
                schema_hash: None,
            })?;
        }
        let mut store = self
            .store
            .lock()
            .map_err(|_| "store mutex envenenado".to_string())?;
        store
            .apply(&ops)
            .map_err(|e| format!("apply Delete: {e}"))?;
        drop(store);
        let post_status = self.tick_compact();
        Ok(WriteOutcome {
            id: Some(id),
            changed: 1,
            post_status,
        })
    }

    fn morphism(
        &mut self,
        module_id: &str,
        name: &str,
        inputs: BTreeMap<String, Uuid>,
        params: Value,
    ) -> Result<WriteOutcome, String> {
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
            .event_log
            .as_ref()
            .ok_or_else(|| "morphism requiere event log activo".to_string())?
            .clone();

        let inputs_owned: Vec<(String, Uuid)> = inputs.into_iter().collect();
        let inputs_ref: Vec<(&str, Uuid)> = inputs_owned
            .iter()
            .map(|(r, id)| (r.as_str(), *id))
            .collect();

        let mut log = log_arc
            .lock()
            .map_err(|_| "log mutex envenenado".to_string())?;
        let mut store = self
            .store
            .lock()
            .map_err(|_| "store mutex envenenado".to_string())?;

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
        let post_status = self.tick_compact();
        Ok(WriteOutcome {
            id: None,
            changed: ops.len(),
            post_status,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Tests del impl `NakuiBackend` contra el contrato del trait.
    //! Exercises seed/load/list/update/delete sin GPUI ni morphism.
    //! El path de morphism está cubierto por
    //! `morphism_pipeline_executes_real_sales_vender` en main.rs.

    use super::*;
    use serde_json::json;

    fn open_in_tempdir() -> (NakuiBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("log.jsonl");
        let (backend, _status) = NakuiBackend::open(log_path, 0, BTreeMap::new());
        (backend, dir)
    }

    fn map_of(items: &[(&str, Value)]) -> serde_json::Map<String, Value> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn seed_then_load_round_trip_via_trait() {
        let (mut b, _dir) = open_in_tempdir();
        let out = b
            .seed("Customer", map_of(&[("name", json!("Acme"))]))
            .unwrap();
        let id = out.id.unwrap();
        assert_eq!(out.changed, 1);
        let rec = b.load_record("Customer", id).unwrap();
        assert_eq!(rec.get("name"), Some(&json!("Acme")));
    }

    #[test]
    fn update_set_then_clear_via_trait() {
        let (mut b, _dir) = open_in_tempdir();
        let id = b
            .seed("X", map_of(&[("a", json!(1)), ("b", json!(2))]))
            .unwrap()
            .id
            .unwrap();

        let out = b
            .update("X", id, map_of(&[("a", json!(10))]), vec!["b".into()])
            .unwrap();
        assert_eq!(out.changed, 2, "1 set + 1 clear = 2 cambios");

        let rec = b.load_record("X", id).unwrap();
        assert_eq!(rec.get("a"), Some(&json!(10)));
        assert!(rec.get("b").is_none());
    }

    #[test]
    fn update_no_op_returns_no_change() {
        let (mut b, _dir) = open_in_tempdir();
        let id = b.seed("X", map_of(&[("a", json!(1))])).unwrap().id.unwrap();
        let out = b.update("X", id, serde_json::Map::new(), vec![]).unwrap();
        assert_eq!(out, WriteOutcome::no_change(id));
    }

    #[test]
    fn delete_via_trait_then_load_returns_none() {
        let (mut b, _dir) = open_in_tempdir();
        let id = b.seed("X", map_of(&[("a", json!(1))])).unwrap().id.unwrap();
        b.delete("X", id).unwrap();
        assert!(b.load_record("X", id).is_none());
    }

    #[test]
    fn list_records_returns_seeded_in_id_order() {
        let (mut b, _dir) = open_in_tempdir();
        let _ = b.seed("X", map_of(&[("k", json!(1))])).unwrap();
        let _ = b.seed("X", map_of(&[("k", json!(2))])).unwrap();
        let _ = b.seed("Y", map_of(&[("k", json!(3))])).unwrap();
        assert_eq!(b.list_records("X").len(), 2);
        assert_eq!(b.list_records("Y").len(), 1);
        assert!(b.list_records("Z").is_empty());
    }

    #[test]
    fn morphism_without_executor_errors_clearly() {
        let (mut b, _dir) = open_in_tempdir();
        let err = b
            .morphism("missing", "vender", BTreeMap::new(), json!({}))
            .unwrap_err();
        assert!(
            err.contains("missing"),
            "msg debe mencionar el módulo: {err}"
        );
        assert!(err.contains("nakui_module_dir") || err.contains("executor"));
    }

    #[test]
    fn morphism_graph_derives_nodes_and_data_flow_edges() {
        // Carga el módulo demo `tesoro` y verifica que el grafo de
        // morfismos sale del manifest: 5 nodos y las aristas de flujo
        // de datos (escritura→lectura del mismo token canónico).
        let module_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/nakui-modules/tesoro/nakui");
        let exec = Executor::load_module(&module_dir).expect("tesoro carga");
        let mut execs: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
        execs.insert("tesoro".into(), Arc::new(exec));
        let dir = tempfile::tempdir().unwrap();
        let (b, _status) = NakuiBackend::open(dir.path().join("log.jsonl"), 0, execs);

        let g = b.morphism_graph("tesoro").expect("hay grafo");
        assert_eq!(g.nodes.len(), 5, "5 morfismos");

        let edge = |from: &str, to: &str| {
            g.edges
                .iter()
                .any(|e| e.from == from && e.to == to)
        };
        // registrar_movimiento escribe Movimiento → aplicar_movimiento lo lee.
        assert!(edge("registrar_movimiento", "aplicar_movimiento"));
        // aplicar_movimiento escribe Caja.saldo → asentar_libro y cerrar lo leen.
        assert!(edge("aplicar_movimiento", "asentar_libro"));
        assert!(edge("aplicar_movimiento", "cerrar_periodo"));
        // asentar_libro escribe Asiento → cerrar_periodo lo lee.
        assert!(edge("asentar_libro", "cerrar_periodo"));
        // abrir_caja escribe la entity Caja (Create), nadie lee "Caja" suelto:
        // queda como nodo fuente sin aristas salientes de ese token.
        assert!(
            !g.edges.iter().any(|e| e.from == "abrir_caja"),
            "abrir_caja no alimenta a nadie por flujo de datos"
        );
    }

    #[test]
    fn tick_compact_writes_snapshot_after_threshold() {
        // threshold=3: tras 3 writes debería haber compactado.
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("log.jsonl");
        let snap_path = snapshot_path_for(&log_path);
        let (mut b, _) = NakuiBackend::open(log_path, 3, BTreeMap::new());

        for _ in 0..3 {
            let _ = b.seed("X", map_of(&[("k", json!(1))])).unwrap();
        }
        // El último seed debería traer un post_status del compact.
        // (En la 3ra llamada el contador llega a 3 y dispara.)
        // Verificamos que el snapshot file exists.
        assert!(snap_path.exists(), "snap debería haberse escrito");
    }
}
