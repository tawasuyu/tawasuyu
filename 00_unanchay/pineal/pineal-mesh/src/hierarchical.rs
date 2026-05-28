//! Sugiyama-lite — layout layered para DAGs (o casi-DAGs).
//!
//! Pipeline en tres etapas, todas O(V + E):
//! 1. **Cycle removal** (DFS): cualquier back-edge se invierte
//!    lógicamente para producir un DAG con el que trabajar.
//! 2. **Layering** (Kahn longest-path): asigna a cada nodo una capa
//!    `0..L` tal que cada arista va de capa `i` a capa `j > i`.
//! 3. **Barycenter ordering** (2 pasadas): minimiza cruces sumando
//!    la posición promedio de los vecinos de la capa anterior y
//!    re-ordenando.
//!
//! Devuelve la posición final `(x, y)` de cada nodo en pixels,
//! contenida en el `Rect` provisto. La integración con `NodeBuffer`
//! la hace el caller (`set_pos` por nodo).

use pineal_render::Rect;

/// Resultado del layout: posición por nodo (mismo índice que la
/// entrada) + información de capas para el caller que quiera dibujar
/// guías o agrupaciones.
#[derive(Debug, Clone)]
pub struct HierarchicalLayout {
    pub positions: Vec<(f32, f32)>,
    pub layers: Vec<Vec<usize>>,
}

/// Calcula el layout. `edges` son pares `(from, to)`; los nodos
/// existen para todos los índices `0..n`. `area` es el rectángulo
/// destino; nodos se distribuyen uniformemente dentro.
pub fn sugiyama_layout(n: usize, edges: &[(usize, usize)], area: Rect) -> HierarchicalLayout {
    if n == 0 {
        return HierarchicalLayout { positions: Vec::new(), layers: Vec::new() };
    }

    // === 1. Cycle removal por DFS — marca back-edges.
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(u, v) in edges {
        if u < n && v < n && u != v {
            adj[u].push(v);
        }
    }
    let mut state = vec![0u8; n]; // 0=blanco 1=en pila 2=hecho
    let mut back: Vec<(usize, usize)> = Vec::new();
    for s in 0..n {
        if state[s] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
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

    // === 2. Longest-path layering (Kahn) sobre el DAG (sin back-edges).
    let mut dag: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(u, v) in edges {
        if u < n && v < n && u != v && !back.contains(&(u, v)) {
            dag[u].push(v);
            indeg[v] += 1;
        }
    }
    let mut layer = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        for &v in &dag[u] {
            layer[v] = layer[v].max(layer[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }
    let n_layers = layer.iter().copied().max().unwrap_or(0) + 1;
    let mut by_layer: Vec<Vec<usize>> = vec![Vec::new(); n_layers];
    for (i, &l) in layer.iter().enumerate() {
        by_layer[l].push(i);
    }

    // === 3. Barycenter — dos pasadas (down + up) para reducir cruces.
    let mut order = vec![0usize; n];
    for col in by_layer.iter() {
        for (pos, &node) in col.iter().enumerate() {
            order[node] = pos;
        }
    }
    for _ in 0..2 {
        // Down (capa i mira a sus padres en capa i-1).
        for c in 1..n_layers {
            let mut paired: Vec<(usize, f64)> = by_layer[c]
                .iter()
                .map(|&node| {
                    let mut sum = 0.0;
                    let mut cnt = 0.0;
                    for &(u, v) in edges {
                        if v == node && u < n && layer[u] + 1 == c {
                            sum += order[u] as f64;
                            cnt += 1.0;
                        }
                    }
                    (node, if cnt > 0.0 { sum / cnt } else { f64::MAX })
                })
                .collect();
            paired.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            by_layer[c] = paired.into_iter().map(|(n, _)| n).collect();
            for (pos, &i) in by_layer[c].iter().enumerate() {
                order[i] = pos;
            }
        }
        // Up (capa i mira a sus hijos en capa i+1).
        for c in (0..n_layers.saturating_sub(1)).rev() {
            let mut paired: Vec<(usize, f64)> = by_layer[c]
                .iter()
                .map(|&node| {
                    let mut sum = 0.0;
                    let mut cnt = 0.0;
                    for &(u, v) in edges {
                        if u == node && v < n && layer[v] == c + 1 {
                            sum += order[v] as f64;
                            cnt += 1.0;
                        }
                    }
                    (node, if cnt > 0.0 { sum / cnt } else { f64::MAX })
                })
                .collect();
            paired.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            by_layer[c] = paired.into_iter().map(|(n, _)| n).collect();
            for (pos, &i) in by_layer[c].iter().enumerate() {
                order[i] = pos;
            }
        }
    }

    // === 4. Asignar posiciones — `x` por capa, `y` por orden dentro.
    let mut positions = vec![(0.0f32, 0.0); n];
    let dx = if n_layers > 1 {
        area.w / (n_layers - 1) as f32
    } else {
        0.0
    };
    for (c, col) in by_layer.iter().enumerate() {
        let dy = if col.len() > 1 {
            area.h / (col.len() - 1) as f32
        } else {
            0.0
        };
        let cx = area.x + c as f32 * dx;
        for (pos, &node) in col.iter().enumerate() {
            let cy = if col.len() == 1 {
                area.y + area.h * 0.5
            } else {
                area.y + pos as f32 * dy
            };
            positions[node] = (cx, cy);
        }
    }

    HierarchicalLayout { positions, layers: by_layer }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty() {
        let l = sugiyama_layout(0, &[], Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(l.positions.is_empty());
    }

    #[test]
    fn chain_assigns_increasing_x() {
        // 0 → 1 → 2 → 3
        let edges = [(0, 1), (1, 2), (2, 3)];
        let l = sugiyama_layout(4, &edges, Rect::new(0.0, 0.0, 300.0, 100.0));
        assert_eq!(l.layers.len(), 4);
        let xs: Vec<f32> = l.positions.iter().map(|p| p.0).collect();
        assert!(xs[0] < xs[1] && xs[1] < xs[2] && xs[2] < xs[3]);
    }

    #[test]
    fn cycle_does_not_loop() {
        // 0 → 1 → 0
        let edges = [(0, 1), (1, 0)];
        let l = sugiyama_layout(2, &edges, Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(l.positions.len(), 2);
        assert!(l.positions[0].0.is_finite() && l.positions[0].1.is_finite());
    }

    #[test]
    fn fan_out_distributes_children_vertically() {
        // 0 → {1, 2, 3, 4}
        let edges = [(0, 1), (0, 2), (0, 3), (0, 4)];
        let l = sugiyama_layout(5, &edges, Rect::new(0.0, 0.0, 200.0, 200.0));
        let ys: Vec<f32> = (1..=4).map(|i| l.positions[i].1).collect();
        let unique_ys: std::collections::HashSet<i32> =
            ys.iter().map(|y| (*y * 10.0) as i32).collect();
        assert!(unique_ys.len() >= 3, "ys deberían ser distintos: {ys:?}");
    }

    #[test]
    fn positions_fall_within_area() {
        let edges = [(0, 1), (1, 2), (0, 3), (3, 4)];
        let area = Rect::new(50.0, 100.0, 400.0, 300.0);
        let l = sugiyama_layout(5, &edges, area);
        for &(x, y) in &l.positions {
            assert!(x >= area.x && x <= area.x + area.w + 1.0);
            assert!(y >= area.y && y <= area.y + area.h + 1.0);
        }
    }
}
