//! Implementación de [`MetaBackend`] para Nakui.
//!
//! Tras el split multi-cliente (fase 1 de "nakui en red"), este crate es un
//! **cliente co-locado delgado** sobre [`nakui_sync`]: la propiedad del
//! `EventLog` + store + executors + compaction vive ahora en
//! [`nakui_sync::Writer`] (el escritor autoritativo, UI-agnóstico). Acá sólo
//! queda el adaptador que proyecta el contrato `MetaBackend` del widget a
//! intenciones [`nakui_sync::Intent`] entregadas por un
//! [`nakui_sync::LocalTransport`].
//!
//! "Co-locado" = el escritor vive en el mismo proceso, así que los reads van
//! directo al store autoritativo (handle compartido) sin tomar el lock del
//! escritor. Un cliente *remoto* (card-net, fase 2) tendría su propia
//! proyección puesta al día con [`nakui_sync::apply_commit`]; el contrato
//! `MetaBackend` no cambia.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use uuid::Uuid;

use nahual_meta_runtime::{MetaBackend, WriteOutcome};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use nakui_sync::{Intent, LocalTransport, Transport, Writer};

// Re-export de la superficie que el resto de Nakui (UI, explorer, tests)
// importa históricamente desde `nakui_backend`. La lógica se mudó a
// `nakui-sync`; estos nombres siguen resolviendo acá.
pub use nakui_sync::{maybe_compact_log, snapshot_path_for, OpenStatus};

/// Backend Nakui: cliente co-locado sobre el escritor autoritativo.
///
/// Implementa [`MetaBackend`] proyectando cada operación a una
/// [`Intent`] que entrega vía [`LocalTransport`]. Los reads van al store
/// autoritativo compartido (handle del escritor).
pub struct NakuiBackend {
    /// Transporte al escritor autoritativo (in-process).
    transport: LocalTransport,
    /// Handle al store autoritativo, para reads sin tocar el lock del
    /// escritor. Es el MISMO store que el escritor muta en cada commit,
    /// así que un read-after-write (propio o de otro cliente co-locado)
    /// es consistente sin re-aplicar el delta acá.
    store: Arc<Mutex<MemoryStore>>,
}

impl NakuiBackend {
    /// Abre/crea el log en `log_path`, hace replay, y monta el escritor
    /// autoritativo detrás de un [`LocalTransport`]. Firma y semántica
    /// idénticas a la versión pre-split — el caller (`main.rs`) no cambia.
    pub fn open(
        log_path: std::path::PathBuf,
        snapshot_threshold: usize,
        executors: BTreeMap<String, Arc<Executor>>,
    ) -> (Self, OpenStatus) {
        let (writer, status) = Writer::open(log_path, snapshot_threshold, executors);
        let store = writer.store_handle();
        let transport = LocalTransport::new(writer);
        (NakuiBackend { transport, store }, status)
    }

    /// Deriva el grafo de morfismos del módulo `module_id` a partir de su
    /// `Executor` (vía el escritor): cada morfismo es un nodo (con los
    /// tokens que lee y escribe), cada par escritura→lectura del mismo
    /// token una arista. `None` si el módulo no tiene executor.
    pub fn morphism_graph(&self, module_id: &str) -> Option<MorphismGraphData> {
        let exec = {
            let w = self.transport.writer();
            let guard = w.lock().ok()?;
            guard.executor(module_id)?
        };
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

    /// Map de un [`nakui_sync::Commit`] al `WriteOutcome` que espera la UI.
    fn outcome(commit: nakui_sync::Commit) -> WriteOutcome {
        WriteOutcome {
            id: commit.primary_id,
            changed: commit.changed,
            post_status: commit.post_status,
        }
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

/// Una arista de flujo de datos: el morfismo `from` escribe `token`, que
/// el morfismo `to` lee — por eso `to` está aguas abajo de `from`.
#[derive(Debug, Clone)]
pub struct DataFlowEdge {
    pub from: String,
    pub to: String,
    pub token: String,
}

/// El grafo de morfismos de un módulo: nodos + aristas de flujo de datos.
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
        let commit = self.transport.submit(Intent::Seed {
            entity: entity.to_string(),
            data,
        })?;
        Ok(Self::outcome(commit))
    }

    fn update(
        &mut self,
        entity: &str,
        id: Uuid,
        set: serde_json::Map<String, Value>,
        clear: Vec<String>,
    ) -> Result<WriteOutcome, String> {
        let commit = self.transport.submit(Intent::Update {
            entity: entity.to_string(),
            id,
            set,
            clear,
        })?;
        Ok(Self::outcome(commit))
    }

    fn delete(&mut self, entity: &str, id: Uuid) -> Result<WriteOutcome, String> {
        let commit = self.transport.submit(Intent::Delete {
            entity: entity.to_string(),
            id,
        })?;
        Ok(Self::outcome(commit))
    }

    fn morphism_n(
        &mut self,
        module_id: &str,
        name: &str,
        inputs: Vec<(String, Uuid)>,
        params: Value,
    ) -> Result<WriteOutcome, String> {
        let commit = self.transport.submit(Intent::Morphism {
            module_id: module_id.to_string(),
            name: name.to_string(),
            inputs,
            params,
        })?;
        Ok(Self::outcome(commit))
    }
}

#[cfg(test)]
mod tests {
    //! Tests del impl `NakuiBackend` contra el contrato del trait.
    //! Exercises seed/load/list/update/delete sin GPUI ni morphism.
    //! El path de morphism está cubierto por
    //! `morphism_pipeline_executes_real_sales_vender` en main.rs y por los
    //! tests multi-cliente en `nakui-sync`.

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
        // Carga el módulo demo `tesoro` y verifica que el grafo de morfismos
        // sale del manifest: 5 nodos y las aristas de flujo de datos. El
        // módulo demo vive en `nakui-ui-llimphi/examples/` tras el refactor.
        let module_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../nakui-ui-llimphi/examples/nakui-modules/tesoro/nakui");
        let exec = Executor::load_module(&module_dir).expect("tesoro carga");
        let mut execs: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
        execs.insert("tesoro".into(), Arc::new(exec));
        let dir = tempfile::tempdir().unwrap();
        let (b, _status) = NakuiBackend::open(dir.path().join("log.jsonl"), 0, execs);

        let g = b.morphism_graph("tesoro").expect("hay grafo");
        assert_eq!(g.nodes.len(), 5, "5 morfismos");

        let edge = |from: &str, to: &str| g.edges.iter().any(|e| e.from == from && e.to == to);
        assert!(edge("registrar_movimiento", "aplicar_movimiento"));
        assert!(edge("aplicar_movimiento", "asentar_libro"));
        assert!(edge("aplicar_movimiento", "cerrar_periodo"));
        assert!(edge("asentar_libro", "cerrar_periodo"));
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
        assert!(snap_path.exists(), "snap debería haberse escrito");
    }
}
