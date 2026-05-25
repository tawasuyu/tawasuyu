//! Layout de árbol por ancho de subárbol.
//!
//! Post-order: las hojas se ubican en columnas consecutivas; cada nodo
//! interno se centra sobre sus hijos. `y` es la profundidad. Soporta
//! bosque (múltiples raíces). Ciclos en los punteros `parent` se ignoran
//! con gracia (esos nodos quedan en el origen).

/// Calcula `(x, y)` por nodo. `parent[i] = None` marca una raíz.
pub fn tree_layout(parent: &[Option<usize>], x_gap: f32, y_gap: f32) -> Vec<(f32, f32)> {
    let n = parent.len();
    let mut pos = vec![(0.0f32, 0.0f32); n];
    if n == 0 {
        return pos;
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots: Vec<usize> = Vec::new();
    for i in 0..n {
        match parent[i] {
            Some(p) if p < n && p != i => children[p].push(i),
            _ => roots.push(i),
        }
    }

    // Profundidad por BFS desde las raíces.
    let mut depth = vec![0usize; n];
    let mut visited = vec![false; n];
    let mut queue: Vec<usize> = roots.clone();
    for &r in &roots {
        visited[r] = true;
    }
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        for &c in &children[u] {
            if !visited[c] {
                visited[c] = true;
                depth[c] = depth[u] + 1;
                queue.push(c);
            }
        }
    }

    // Asignación de `x` por post-order iterativo, raíz por raíz.
    let mut next_leaf = 0.0f32;
    for &root in &roots {
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&mut (u, ref mut ci)) = stack.last_mut() {
            if *ci < children[u].len() {
                let c = children[u][*ci];
                *ci += 1;
                stack.push((c, 0));
            } else {
                if children[u].is_empty() {
                    pos[u].0 = next_leaf;
                    next_leaf += x_gap;
                } else {
                    let sum: f32 = children[u].iter().map(|&c| pos[c].0).sum();
                    pos[u].0 = sum / children[u].len() as f32;
                }
                pos[u].1 = depth[u] as f32 * y_gap;
                stack.pop();
            }
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_node_at_origin() {
        let pos = tree_layout(&[None], 10.0, 20.0);
        assert_eq!(pos, vec![(0.0, 0.0)]);
    }

    #[test]
    fn parent_centered_over_two_children() {
        // 0 raíz; 1 y 2 hijos.
        let pos = tree_layout(&[None, Some(0), Some(0)], 10.0, 20.0);
        assert_eq!(pos[1], (0.0, 20.0));
        assert_eq!(pos[2], (10.0, 20.0));
        // padre centrado en x = (0+10)/2 = 5, depth 0.
        assert_eq!(pos[0], (5.0, 0.0));
    }

    #[test]
    fn depth_increases_down_the_tree() {
        // cadena 0 → 1 → 2
        let pos = tree_layout(&[None, Some(0), Some(1)], 10.0, 20.0);
        assert_eq!(pos[0].1, 0.0);
        assert_eq!(pos[1].1, 20.0);
        assert_eq!(pos[2].1, 40.0);
    }

    #[test]
    fn cycle_in_parents_does_not_hang() {
        // 0 ↔ 1 sin raíz: no debe colgar.
        let pos = tree_layout(&[Some(1), Some(0)], 10.0, 20.0);
        assert_eq!(pos.len(), 2);
    }
}
