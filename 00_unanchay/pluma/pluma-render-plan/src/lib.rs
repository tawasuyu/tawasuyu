//! `fana-render-plan` — el plan de dibujo del editor DAG, agnóstico.
//!
//! Traduce un [`NarrativeGraph`] a una geometría 2D lista para pintar
//! sin saber nada del backend (`fana-editor-gpui`, `fana-editor-web`):
//!
//! - **Editor** — un [`AtomBlock`] por átomo, apilados verticalmente en
//!   orden topológico; cada rama ocupa su propia columna.
//! - **Conectores** — un [`Edge`] por arista de dependencia, del borde
//!   inferior del prerrequisito al superior del dependiente.
//! - **Osciloscopio** — un [`SidepaneMark`] por átomo en el sidepane,
//!   coloreado por coherencia y con altura según la intensidad
//!   semántica acumulada.
//!
//! Todo es determinista: el orden de layout se desempata por
//! `(profundidad, columna, id)`, sin depender de la iteración de
//! `HashMap`.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use fana_core::CoherenceState;
use fana_graph::NarrativeGraph;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Coherencia de un átomo reducida a un tono de presentación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoherenceTone {
    /// `Valid` — consistente.
    Valid,
    /// `PendingEvaluation` — una dependencia mutó, falta verificar.
    Pending,
    /// `InConflict` — una dependencia lo contradice.
    Conflict,
}

impl CoherenceTone {
    fn of(state: &CoherenceState) -> Self {
        match state {
            CoherenceState::Valid => CoherenceTone::Valid,
            CoherenceState::PendingEvaluation => CoherenceTone::Pending,
            CoherenceState::InConflict { .. } => CoherenceTone::Conflict,
        }
    }
}

/// Un bloque del editor — la caja visual de un átomo narrativo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomBlock {
    pub id: Uuid,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Rama / línea temporal del átomo.
    pub branch: String,
    /// Profundidad topológica (0 = raíz).
    pub depth: usize,
    pub tone: CoherenceTone,
    /// Primeros caracteres del contenido (con `…` si se truncó).
    pub preview: String,
}

/// Un conector de dependencia entre dos bloques.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: Uuid,
    pub to: Uuid,
    /// Punto de salida (borde inferior del prerrequisito).
    pub x1: f32,
    pub y1: f32,
    /// Punto de llegada (borde superior del dependiente).
    pub x2: f32,
    pub y2: f32,
}

/// Una marca del osciloscopio de coherencia, en el sidepane.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SidepaneMark {
    pub id: Uuid,
    pub y: f32,
    pub h: f32,
    pub tone: CoherenceTone,
    /// Intensidad semántica normalizada a `0.0..=1.0` sobre el documento.
    pub intensity: f32,
}

/// Parámetros de layout — lo que un panel de presentación ajustaría.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LayoutConfig {
    pub block_w: f32,
    pub block_h: f32,
    /// Espacio vertical entre bloques consecutivos.
    pub gap: f32,
    /// Margen alrededor del lienzo.
    pub margin: f32,
    /// Desplazamiento horizontal entre columnas de ramas.
    pub column_stride: f32,
    pub sidepane_width: f32,
    /// Cuántos caracteres del contenido entran en el `preview`.
    pub preview_chars: usize,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            block_w: 360.0,
            block_h: 64.0,
            gap: 14.0,
            margin: 24.0,
            column_stride: 400.0,
            sidepane_width: 120.0,
            preview_chars: 80,
        }
    }
}

/// El plan de dibujo completo del documento.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderPlan {
    pub blocks: Vec<AtomBlock>,
    pub edges: Vec<Edge>,
    pub sidepane: Vec<SidepaneMark>,
    /// Alto total del contenido — el backend lo usa para el scroll.
    pub content_height: f32,
    /// Config con la que se construyó — el backend lee de aquí los
    /// márgenes y el ancho del sidepane sin recibirlos por separado.
    pub config: LayoutConfig,
}

/// Profundidad topológica de cada átomo: 0 para las raíces, `1 + máx`
/// de las profundidades de sus dependencias. Si el grafo tiene un ciclo
/// (no debería) todos caen a 0.
fn depths(graph: &NarrativeGraph) -> HashMap<Uuid, usize> {
    let mut depth: HashMap<Uuid, usize> = HashMap::new();
    let Some(order) = graph.topological_order() else {
        // Grafo con ciclo: layout plano defensivo.
        return graph.atoms().map(|a| (a.id, 0)).collect();
    };
    for id in order {
        let Some(atom) = graph.get(id) else { continue };
        let d = atom
            .dependencies
            .iter()
            .filter_map(|dep| depth.get(dep))
            .map(|&d| d + 1)
            .max()
            .unwrap_or(0);
        depth.insert(id, d);
    }
    depth
}

/// Recorta `content` a `max` caracteres, añadiendo `…` si se truncó.
fn preview(content: &str, max: usize) -> String {
    let mut out: String = content.chars().take(max).collect();
    if content.chars().count() > max {
        out.push('…');
    }
    out
}

/// Intensidad semántica de un átomo: suma de los valores absolutos de
/// sus vectores concepto→intensidad.
fn raw_intensity(atom: &fana_core::NarrativeAtom) -> f32 {
    atom.semantic_vectors.values().map(|v| v.abs()).sum()
}

/// Construye el plan de dibujo de un `NarrativeGraph`.
pub fn build_plan(graph: &NarrativeGraph, cfg: &LayoutConfig) -> RenderPlan {
    if graph.is_empty() {
        return RenderPlan::default();
    }

    let depth = depths(graph);

    // Columnas: una por rama, ordenadas alfabéticamente para estabilidad.
    let mut branches: Vec<&str> = graph.atoms().map(|a| a.branch_id.as_str()).collect();
    branches.sort_unstable();
    branches.dedup();
    let column: HashMap<&str, usize> =
        branches.iter().enumerate().map(|(i, &b)| (b, i)).collect();

    // Orden de layout determinista: (profundidad, columna, id).
    let mut order: Vec<&fana_core::NarrativeAtom> = graph.atoms().collect();
    order.sort_by(|a, b| {
        let da = depth.get(&a.id).copied().unwrap_or(0);
        let db = depth.get(&b.id).copied().unwrap_or(0);
        let ca = column[a.branch_id.as_str()];
        let cb = column[b.branch_id.as_str()];
        da.cmp(&db).then(ca.cmp(&cb)).then(a.id.cmp(&b.id))
    });

    let row_stride = cfg.block_h + cfg.gap;
    let max_intensity = order
        .iter()
        .map(|a| raw_intensity(a))
        .fold(0.0f32, f32::max);

    let mut blocks = Vec::with_capacity(order.len());
    let mut sidepane = Vec::with_capacity(order.len());
    let mut rect: HashMap<Uuid, (f32, f32, f32, f32)> = HashMap::new();

    for (row, atom) in order.iter().enumerate() {
        let col = column[atom.branch_id.as_str()];
        let x = cfg.margin + cfg.sidepane_width + col as f32 * cfg.column_stride;
        let y = cfg.margin + row as f32 * row_stride;
        let d = depth.get(&atom.id).copied().unwrap_or(0);
        rect.insert(atom.id, (x, y, cfg.block_w, cfg.block_h));

        blocks.push(AtomBlock {
            id: atom.id,
            x,
            y,
            w: cfg.block_w,
            h: cfg.block_h,
            branch: atom.branch_id.clone(),
            depth: d,
            tone: CoherenceTone::of(&atom.coherence),
            preview: preview(&atom.content, cfg.preview_chars),
        });

        let intensity = if max_intensity > 0.0 {
            raw_intensity(atom) / max_intensity
        } else {
            0.0
        };
        sidepane.push(SidepaneMark {
            id: atom.id,
            y,
            h: cfg.block_h,
            tone: CoherenceTone::of(&atom.coherence),
            intensity,
        });
    }

    // Conectores: una arista por dependencia presente en el grafo.
    let mut edges = Vec::new();
    for atom in &order {
        let Some(&(tx, ty, tw, _)) = rect.get(&atom.id) else { continue };
        for dep in &atom.dependencies {
            let Some(&(fx, fy, fw, fh)) = rect.get(dep) else { continue };
            edges.push(Edge {
                from: *dep,
                to: atom.id,
                x1: fx + fw * 0.5,
                y1: fy + fh,
                x2: tx + tw * 0.5,
                y2: ty,
            });
        }
    }

    RenderPlan {
        blocks,
        edges,
        sidepane,
        content_height: cfg.margin * 2.0 + order.len() as f32 * row_stride,
        config: *cfg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fana_core::NarrativeAtom;

    /// Cadena a → b → c en la rama `main`.
    fn chain() -> (NarrativeGraph, Uuid, Uuid, Uuid) {
        let a = NarrativeAtom::new("primero", "main");
        let a_id = a.id;
        let b = NarrativeAtom::new("segundo", "main").depends_on(a_id);
        let b_id = b.id;
        let c = NarrativeAtom::new("tercero", "main").depends_on(b_id);
        let c_id = c.id;
        (NarrativeGraph::from_atoms([a, b, c]), a_id, b_id, c_id)
    }

    #[test]
    fn empty_graph_yields_empty_plan() {
        let plan = build_plan(&NarrativeGraph::new(), &LayoutConfig::default());
        assert!(plan.blocks.is_empty() && plan.edges.is_empty());
        assert_eq!(plan.content_height, 0.0);
    }

    #[test]
    fn one_block_and_one_mark_per_atom() {
        let (g, ..) = chain();
        let plan = build_plan(&g, &LayoutConfig::default());
        assert_eq!(plan.blocks.len(), 3);
        assert_eq!(plan.sidepane.len(), 3);
    }

    #[test]
    fn chain_has_one_edge_per_dependency() {
        let (g, ..) = chain();
        let plan = build_plan(&g, &LayoutConfig::default());
        assert_eq!(plan.edges.len(), 2);
    }

    #[test]
    fn blocks_are_stacked_by_topological_depth() {
        let (g, a, b, c) = chain();
        let plan = build_plan(&g, &LayoutConfig::default());
        let y = |id: Uuid| plan.blocks.iter().find(|bl| bl.id == id).unwrap().y;
        assert!(y(a) < y(b), "a antes que b");
        assert!(y(b) < y(c), "b antes que c");
        let depth = |id: Uuid| plan.blocks.iter().find(|bl| bl.id == id).unwrap().depth;
        assert_eq!((depth(a), depth(b), depth(c)), (0, 1, 2));
    }

    #[test]
    fn edge_connects_dependency_bottom_to_dependent_top() {
        let (g, a, b, _) = chain();
        let plan = build_plan(&g, &LayoutConfig::default());
        let e = plan.edges.iter().find(|e| e.from == a && e.to == b).unwrap();
        let block_a = plan.blocks.iter().find(|bl| bl.id == a).unwrap();
        let block_b = plan.blocks.iter().find(|bl| bl.id == b).unwrap();
        assert_eq!(e.y1, block_a.y + block_a.h); // borde inferior de a
        assert_eq!(e.y2, block_b.y); // borde superior de b
    }

    #[test]
    fn coherence_states_map_to_tones() {
        let mut conflicted = NarrativeAtom::new("roto", "main");
        conflicted.coherence = CoherenceState::InConflict {
            origin: Uuid::new_v4(),
            reason: "contradice el origen".into(),
        };
        let mut pending = NarrativeAtom::new("dudoso", "main");
        pending.coherence = CoherenceState::PendingEvaluation;
        let valid = NarrativeAtom::new("ok", "main");
        let (cid, pid, vid) = (conflicted.id, pending.id, valid.id);
        let g = NarrativeGraph::from_atoms([conflicted, pending, valid]);
        let plan = build_plan(&g, &LayoutConfig::default());
        let tone = |id: Uuid| plan.blocks.iter().find(|b| b.id == id).unwrap().tone;
        assert_eq!(tone(cid), CoherenceTone::Conflict);
        assert_eq!(tone(pid), CoherenceTone::Pending);
        assert_eq!(tone(vid), CoherenceTone::Valid);
    }

    #[test]
    fn branches_land_in_separate_columns() {
        let main = NarrativeAtom::new("línea principal", "main");
        let alt = NarrativeAtom::new("línea alterna", "alt");
        let (mid, aid) = (main.id, alt.id);
        let g = NarrativeGraph::from_atoms([main, alt]);
        let plan = build_plan(&g, &LayoutConfig::default());
        let x = |id: Uuid| plan.blocks.iter().find(|b| b.id == id).unwrap().x;
        assert_ne!(x(mid), x(aid), "ramas distintas → columnas distintas");
    }

    #[test]
    fn preview_truncates_long_content() {
        let long = "x".repeat(500);
        let atom = NarrativeAtom::new(long, "main");
        let g = NarrativeGraph::from_atoms([atom]);
        let cfg = LayoutConfig { preview_chars: 20, ..LayoutConfig::default() };
        let plan = build_plan(&g, &cfg);
        let p = &plan.blocks[0].preview;
        assert!(p.ends_with('…'));
        assert_eq!(p.chars().count(), 21); // 20 + el elipsis
    }

    #[test]
    fn intensity_is_normalized_to_unit_range() {
        let mut weak = NarrativeAtom::new("tenue", "main");
        weak.semantic_vectors.insert("miedo".into(), 0.2);
        let mut strong = NarrativeAtom::new("intenso", "main");
        strong.semantic_vectors.insert("miedo".into(), 0.8);
        strong.semantic_vectors.insert("ira".into(), 1.2);
        let (wid, sid) = (weak.id, strong.id);
        let g = NarrativeGraph::from_atoms([weak, strong]);
        let plan = build_plan(&g, &LayoutConfig::default());
        let inten = |id: Uuid| plan.sidepane.iter().find(|m| m.id == id).unwrap().intensity;
        // El más intenso queda en 1.0; el resto, proporcional.
        assert!((inten(sid) - 1.0).abs() < 1e-6);
        assert!(inten(wid) < inten(sid));
    }

    #[test]
    fn plan_is_deterministic() {
        let (g, ..) = chain();
        let a = build_plan(&g, &LayoutConfig::default());
        let b = build_plan(&g, &LayoutConfig::default());
        assert_eq!(a.blocks, b.blocks);
        assert_eq!(a.edges, b.edges);
        assert_eq!(a.sidepane, b.sidepane);
    }
}
