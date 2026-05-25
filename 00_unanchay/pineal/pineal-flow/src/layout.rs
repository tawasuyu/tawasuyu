//! Layout de un diagrama Sankey.
//!
//! Pipeline: columnas por longest-path en el DAG (back-edges descartadas)
//! → valor de nodo = max(entrada, salida) → apilado vertical por columna
//! con una pasada de barycenter para reducir cruces → anclas de cada
//! banda (link) en los bordes de sus nodos.

use pineal_render::{Point, Rect};

/// Un nodo del Sankey.
#[derive(Debug, Clone)]
pub struct SankeyNode {
    pub label: String,
}

impl SankeyNode {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into() }
    }
}

/// Un flujo dirigido `source → target` con un caudal `value`.
#[derive(Debug, Clone, Copy)]
pub struct SankeyLink {
    pub source: usize,
    pub target: usize,
    pub value: f64,
}

/// Caja de un nodo ya ubicada en el lienzo.
#[derive(Debug, Clone)]
pub struct NodeBox {
    pub rect: Rect,
    pub column: usize,
}

/// Banda de un link: cuatro anclas (arriba/abajo en origen y destino).
#[derive(Debug, Clone, Copy)]
pub struct LinkBand {
    pub link: usize,
    pub src_top: Point,
    pub src_bot: Point,
    pub dst_top: Point,
    pub dst_bot: Point,
}

/// Layout completo: cajas de nodos + bandas de links.
#[derive(Debug, Clone, Default)]
pub struct SankeyLayout {
    pub nodes: Vec<NodeBox>,
    pub links: Vec<LinkBand>,
}

/// Calcula el layout de un Sankey dentro de `area`.
pub fn compute_layout(
    nodes: &[SankeyNode],
    links: &[SankeyLink],
    area: Rect,
    node_width: f32,
    node_gap: f32,
) -> SankeyLayout {
    let n = nodes.len();
    if n == 0 || area.w <= 0.0 || area.h <= 0.0 {
        return SankeyLayout::default();
    }
    let valid: Vec<&SankeyLink> = links
        .iter()
        .filter(|l| l.source < n && l.target < n && l.source != l.target && l.value > 0.0)
        .collect();

    let columns = assign_columns(n, &valid);
    let n_cols = columns.iter().copied().max().unwrap_or(0) + 1;

    // Valor de cada nodo = max(suma entrante, suma saliente).
    let mut in_sum = vec![0.0f64; n];
    let mut out_sum = vec![0.0f64; n];
    for l in &valid {
        in_sum[l.target] += l.value;
        out_sum[l.source] += l.value;
    }
    let node_value: Vec<f64> = (0..n).map(|i| in_sum[i].max(out_sum[i]).max(0.0)).collect();

    // Nodos por columna.
    let mut by_col: Vec<Vec<usize>> = vec![Vec::new(); n_cols];
    for (i, &c) in columns.iter().enumerate() {
        by_col[c].push(i);
    }

    // Escala vertical: la columna más cargada llena `area.h` (con gaps).
    let max_col_value = by_col
        .iter()
        .map(|col| col.iter().map(|&i| node_value[i]).sum::<f64>())
        .fold(0.0f64, f64::max)
        .max(1e-9);
    let max_col_count = by_col.iter().map(|c| c.len()).max().unwrap_or(1).max(1);
    let usable_h = (area.h - node_gap * (max_col_count.saturating_sub(1)) as f32).max(1.0);
    let v_scale = usable_h as f64 / max_col_value;

    // Una pasada de barycenter para ordenar cada columna.
    barycenter_pass(&mut by_col, &valid, &columns);

    // Geometría de cada nodo.
    let col_step = if n_cols > 1 {
        (area.w - node_width) / (n_cols - 1) as f32
    } else {
        0.0
    };
    let mut boxes = vec![
        NodeBox { rect: Rect::new(0.0, 0.0, 0.0, 0.0), column: 0 };
        n
    ];
    for (c, col) in by_col.iter().enumerate() {
        let mut y = area.y;
        for &i in col {
            let h = (node_value[i] * v_scale) as f32;
            let x = area.x + c as f32 * col_step;
            boxes[i] = NodeBox {
                rect: Rect::new(x, y, node_width, h.max(1.0)),
                column: c,
            };
            y += h + node_gap;
        }
    }

    // Bandas de links: apiladas en el borde derecho del origen y el
    // borde izquierdo del destino, en el orden de aparición.
    let mut src_cursor = vec![0.0f32; n];
    let mut dst_cursor = vec![0.0f32; n];
    let mut bands = Vec::with_capacity(valid.len());
    for (vi, l) in valid.iter().enumerate() {
        let sb = &boxes[l.source].rect;
        let tb = &boxes[l.target].rect;
        let thick_s = (l.value * v_scale) as f32;
        let s_y0 = sb.y + src_cursor[l.source];
        let t_y0 = tb.y + dst_cursor[l.target];
        src_cursor[l.source] += thick_s;
        dst_cursor[l.target] += thick_s;
        // El índice real del link en el slice original.
        let link_idx = links
            .iter()
            .position(|x| {
                x.source == l.source && x.target == l.target && x.value == l.value
            })
            .unwrap_or(vi);
        bands.push(LinkBand {
            link: link_idx,
            src_top: Point::new(sb.right(), s_y0),
            src_bot: Point::new(sb.right(), s_y0 + thick_s),
            dst_top: Point::new(tb.x, t_y0),
            dst_bot: Point::new(tb.x, t_y0 + thick_s),
        });
    }

    SankeyLayout { nodes: boxes, links: bands }
}

/// Columna de cada nodo = longest-path desde una fuente. Las back-edges
/// (detectadas por DFS) se descartan para romper ciclos.
fn assign_columns(n: usize, links: &[&SankeyLink]) -> Vec<usize> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for l in links {
        adj[l.source].push(l.target);
    }
    // DFS marcando back-edges (destino en la pila actual).
    let mut state = vec![0u8; n]; // 0=blanco 1=en-pila 2=hecho
    let mut back: Vec<(usize, usize)> = Vec::new();
    for s in 0..n {
        if state[s] != 0 {
            continue;
        }
        let mut stack = vec![(s, 0usize)];
        state[s] = 1;
        while let Some(&mut (u, ref mut i)) = stack.last_mut() {
            if *i < adj[u].len() {
                let v = adj[u][*i];
                *i += 1;
                match state[v] {
                    0 => {
                        state[v] = 1;
                        stack.push((v, 0));
                    }
                    1 => back.push((u, v)),
                    _ => {}
                }
            } else {
                state[u] = 2;
                stack.pop();
            }
        }
    }
    // Longest-path en el DAG (sin back-edges) vía relajación topológica.
    let mut indeg = vec![0usize; n];
    let mut dag: Vec<Vec<usize>> = vec![Vec::new(); n];
    for l in links {
        if !back.contains(&(l.source, l.target)) {
            dag[l.source].push(l.target);
            indeg[l.target] += 1;
        }
    }
    let mut col = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        for &v in &dag[u] {
            col[v] = col[v].max(col[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }
    col
}

/// Reordena los nodos de cada columna por el promedio de las posiciones
/// de sus vecinos (barycenter heuristic), una pasada izquierda→derecha.
fn barycenter_pass(by_col: &mut [Vec<usize>], links: &[&SankeyLink], columns: &[usize]) {
    let n = columns.len();
    let mut order_in_col = vec![0usize; n];
    for col in by_col.iter() {
        for (pos, &i) in col.iter().enumerate() {
            order_in_col[i] = pos;
        }
    }
    for c in 1..by_col.len() {
        let bary: Vec<(usize, f64)> = by_col[c]
            .iter()
            .map(|&node| {
                let mut sum = 0.0;
                let mut cnt = 0.0;
                for l in links {
                    if l.target == node && columns[l.source] == c - 1 {
                        sum += order_in_col[l.source] as f64;
                        cnt += 1.0;
                    }
                }
                (node, if cnt > 0.0 { sum / cnt } else { f64::MAX })
            })
            .collect();
        let mut sorted = bary;
        sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        by_col[c] = sorted.into_iter().map(|(node, _)| node).collect();
        for (pos, &i) in by_col[c].iter().enumerate() {
            order_in_col[i] = pos;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(n: usize) -> Vec<SankeyNode> {
        (0..n).map(|i| SankeyNode::new(format!("n{i}"))).collect()
    }

    #[test]
    fn empty_input() {
        let l = compute_layout(&[], &[], Rect::new(0.0, 0.0, 100.0, 100.0), 20.0, 4.0);
        assert!(l.nodes.is_empty());
    }

    #[test]
    fn chain_assigns_increasing_columns() {
        // 0 → 1 → 2
        let links = [
            SankeyLink { source: 0, target: 1, value: 5.0 },
            SankeyLink { source: 1, target: 2, value: 5.0 },
        ];
        let l = compute_layout(&nodes(3), &links, Rect::new(0.0, 0.0, 300.0, 100.0), 20.0, 4.0);
        assert_eq!(l.nodes[0].column, 0);
        assert_eq!(l.nodes[1].column, 1);
        assert_eq!(l.nodes[2].column, 2);
        assert_eq!(l.links.len(), 2);
    }

    #[test]
    fn back_edge_does_not_loop_forever() {
        // ciclo 0 → 1 → 0 ; debe terminar y no panickear.
        let links = [
            SankeyLink { source: 0, target: 1, value: 3.0 },
            SankeyLink { source: 1, target: 0, value: 1.0 },
        ];
        let l = compute_layout(&nodes(2), &links, Rect::new(0.0, 0.0, 200.0, 100.0), 20.0, 4.0);
        assert_eq!(l.nodes.len(), 2);
    }

    #[test]
    fn node_height_proportional_to_flow() {
        // 0 manda 10 a 1 y 1 a 2 ; nodo 0 más "grueso" que nodo 2.
        let links = [
            SankeyLink { source: 0, target: 1, value: 10.0 },
            SankeyLink { source: 0, target: 2, value: 2.0 },
        ];
        let l = compute_layout(&nodes(3), &links, Rect::new(0.0, 0.0, 200.0, 200.0), 20.0, 4.0);
        assert!(l.nodes[0].rect.h > l.nodes[2].rect.h);
    }
}
