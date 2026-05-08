//! Static dependency graph derived from a `Manifest`.
//!
//! Two graphs in one structure:
//!   - **Explicit graph** (`depends_on`): morphism-to-morphism edges declared
//!     by the manifest author. Cycles here are an error — the graph is built
//!     with cycle detection.
//!   - **Data-flow indexes** (`reads`/`writes`): inverted indexes from
//!     canonical entity tokens (`"Caja.saldo"` or `"Movimiento"`) to the
//!     morphisms that read or write them. Self-loops in data flow are
//!     legal (a morphism that reads a field and updates it is normal).
//!
//! Tokens are normalized at build time: a manifest's role-prefixed tokens
//! (`"caja.saldo"`) become entity-prefixed (`"Caja.saldo"`) so cross-module
//! queries work uniformly.

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Topo;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

use crate::manifest::Manifest;

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("dependency cycle in `depends_on` involving morphisms {0:?}")]
    Cycle(Vec<String>),
    #[error("morphism `{0}` referenced in depends_on but not declared in this manifest")]
    UnknownMorphism(String),
}

#[derive(Debug)]
pub struct ManifestGraph {
    /// Explicit `depends_on` graph. Edge `a -> b` means: morphism `b`
    /// depends on `a`, so `a` must be available before `b`.
    explicit: DiGraph<String, ()>,

    /// Data-flow indexes. Token form: "Entity.field" or "Entity".
    readers_of_token: HashMap<String, Vec<String>>,
    writers_of_token: HashMap<String, Vec<String>>,

    /// Per-morphism canonicalized token sets.
    morphism_reads: HashMap<String, Vec<String>>,
    morphism_writes: HashMap<String, Vec<String>>,
}

impl ManifestGraph {
    pub fn build(manifest: &Manifest) -> Result<Self, GraphError> {
        let explicit = build_explicit(manifest)?;
        if let Some(cycle) = find_cycle(&explicit) {
            return Err(GraphError::Cycle(cycle));
        }
        let (readers_of_token, writers_of_token, morphism_reads, morphism_writes) =
            build_data_flow(manifest);
        Ok(Self {
            explicit,
            readers_of_token,
            writers_of_token,
            morphism_reads,
            morphism_writes,
        })
    }

    /// Morphisms that read `token`. Token form: "Entity.field" or "Entity".
    pub fn readers_of(&self, token: &str) -> &[String] {
        self.readers_of_token
            .get(token)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Morphisms that write `token`.
    pub fn writers_of(&self, token: &str) -> &[String] {
        self.writers_of_token
            .get(token)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn morphism_reads(&self, name: &str) -> &[String] {
        self.morphism_reads
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn morphism_writes(&self, name: &str) -> &[String] {
        self.morphism_writes
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Morphisms whose `reads` overlap any of `name`'s `writes`. The
    /// dirty-marking primitive: after `name` runs successfully, these are
    /// the candidates whose derived state would be invalidated. The result
    /// excludes `name` itself even if it reads what it writes.
    pub fn affected_by(&self, name: &str) -> Vec<String> {
        let writes = match self.morphism_writes.get(name) {
            Some(w) => w,
            None => return Vec::new(),
        };
        let mut affected: HashSet<String> = HashSet::new();
        for token in writes {
            if let Some(readers) = self.readers_of_token.get(token) {
                for r in readers {
                    if r != name {
                        affected.insert(r.clone());
                    }
                }
            }
        }
        let mut out: Vec<_> = affected.into_iter().collect();
        out.sort();
        out
    }

    /// Topological order of the explicit dependency graph. If `a` is in
    /// `b.depends_on`, `a` precedes `b` in the result.
    pub fn topological_order(&self) -> Vec<String> {
        let mut topo = Topo::new(&self.explicit);
        let mut out = Vec::new();
        while let Some(idx) = topo.next(&self.explicit) {
            out.push(self.explicit[idx].clone());
        }
        out
    }
}

fn build_explicit(manifest: &Manifest) -> Result<DiGraph<String, ()>, GraphError> {
    let mut graph = DiGraph::new();
    let mut nodes: HashMap<String, NodeIndex> = HashMap::new();
    for m in &manifest.morphisms {
        let idx = graph.add_node(m.name.clone());
        nodes.insert(m.name.clone(), idx);
    }
    for m in &manifest.morphisms {
        let to = nodes[&m.name];
        for dep in &m.depends_on {
            let from = *nodes
                .get(dep)
                .ok_or_else(|| GraphError::UnknownMorphism(dep.clone()))?;
            graph.add_edge(from, to, ());
        }
    }
    Ok(graph)
}

/// Returns one cycle's nodes (sorted) if the graph has any. Self-loops
/// are returned as `[name]`; multi-node SCCs as the SCC's nodes.
fn find_cycle(graph: &DiGraph<String, ()>) -> Option<Vec<String>> {
    for scc in tarjan_scc(graph) {
        if scc.len() > 1 {
            let mut names: Vec<String> = scc.iter().map(|i| graph[*i].clone()).collect();
            names.sort();
            return Some(names);
        }
        if scc.len() == 1 && graph.find_edge(scc[0], scc[0]).is_some() {
            return Some(vec![graph[scc[0]].clone()]);
        }
    }
    None
}

fn build_data_flow(
    manifest: &Manifest,
) -> (
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<String>>,
) {
    let mut readers: HashMap<String, Vec<String>> = HashMap::new();
    let mut writers: HashMap<String, Vec<String>> = HashMap::new();
    let mut m_reads: HashMap<String, Vec<String>> = HashMap::new();
    let mut m_writes: HashMap<String, Vec<String>> = HashMap::new();

    for m in &manifest.morphisms {
        let role_to_entity: HashMap<&str, &str> = m
            .inputs
            .iter()
            .map(|i| (i.role.as_str(), i.entity.as_str()))
            .collect();

        // Dedupe per-morphism: `source.saldo` and `dest.saldo` both
        // canonicalize to `Caja.saldo` — the morphism is one writer, not
        // two.
        let mut seen_reads: HashSet<String> = HashSet::new();
        for r in &m.reads {
            if let Some(token) = canonicalize_token(r, &role_to_entity) {
                if seen_reads.insert(token.clone()) {
                    readers.entry(token.clone()).or_default().push(m.name.clone());
                    m_reads.entry(m.name.clone()).or_default().push(token);
                }
            }
        }
        let mut seen_writes: HashSet<String> = HashSet::new();
        for w in &m.writes {
            if let Some(token) = canonicalize_token(w, &role_to_entity) {
                if seen_writes.insert(token.clone()) {
                    writers.entry(token.clone()).or_default().push(m.name.clone());
                    m_writes.entry(m.name.clone()).or_default().push(token);
                }
            }
        }
    }

    (readers, writers, m_reads, m_writes)
}

/// "role.field" -> "Entity.field" via the inputs map; "Entity" -> "Entity".
fn canonicalize_token(t: &str, roles: &HashMap<&str, &str>) -> Option<String> {
    if let Some((role, field)) = t.split_once('.') {
        roles
            .get(role)
            .map(|entity| format!("{}.{}", entity, field))
    } else {
        Some(t.to_string())
    }
}

/// Tracks which morphisms have stale derived state because some morphism
/// they read from was applied. Wire it next to your `execute_and_log`
/// loop: after a successful run, call `mark_dirty_after(morphism, &graph)`;
/// then any consumer (cached view, derived report, downstream pipeline)
/// queries `is_dirty(name)` before using its cached output.
///
/// The tracker holds names only — it doesn't know what "recompute" means
/// for any particular morphism. That's deliberate: the kernel exposes the
/// invalidation primitive; what to do with the dirty set is the caller's.
#[derive(Debug, Default, Clone)]
pub struct DirtyTracker {
    dirty: HashSet<String>,
}

impl DirtyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// After `morphism_name` runs successfully, mark every morphism in
    /// `graph.affected_by(morphism_name)` as dirty.
    pub fn mark_dirty_after(&mut self, morphism_name: &str, graph: &ManifestGraph) {
        for affected in graph.affected_by(morphism_name) {
            self.dirty.insert(affected);
        }
    }

    pub fn is_dirty(&self, morphism: &str) -> bool {
        self.dirty.contains(morphism)
    }

    /// Sorted list of dirty morphisms. Stable order for UI/telemetry.
    pub fn dirty(&self) -> Vec<String> {
        let mut out: Vec<String> = self.dirty.iter().cloned().collect();
        out.sort();
        out
    }

    pub fn len(&self) -> usize {
        self.dirty.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dirty.is_empty()
    }

    /// Clear the dirty flag for a specific morphism (call after the
    /// caller has recomputed it).
    pub fn clear(&mut self, morphism: &str) {
        self.dirty.remove(morphism);
    }

    pub fn clear_all(&mut self) {
        self.dirty.clear();
    }
}
