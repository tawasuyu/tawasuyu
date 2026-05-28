//! Layout force-directed (Fruchterman-Reingold).
//!
//! Repulsión entre todo par de nodos + atracción a lo largo de las
//! aristas, integrado con cooling. Dos variantes del paso:
//!
//! - [`ForceLayout::step`] — O(n²) naïve. Hasta ~1 K nodos, exacto.
//! - [`ForceLayout::step_bh`] — O(n log n) Barnes-Hut, con parámetro
//!   `theta` (típico 0.5). Apta para 10⁴–10⁵ nodos.

use crate::barnes_hut::Quadtree;
use crate::buffers::{EdgeBuffer, NodeBuffer};

/// Parámetros de la simulación.
#[derive(Debug, Clone, Copy)]
pub struct ForceParams {
    /// Distancia ideal entre nodos conectados.
    pub k: f32,
    /// Desplazamiento máximo inicial por paso (se enfría).
    pub temperature: f32,
    /// Factor de enfriamiento aplicado cada paso (`0 < cooling < 1`).
    pub cooling: f32,
}

impl Default for ForceParams {
    fn default() -> Self {
        Self { k: 50.0, temperature: 50.0, cooling: 0.95 }
    }
}

/// Estado de una simulación force-directed.
pub struct ForceLayout {
    params: ForceParams,
    temp: f32,
}

impl ForceLayout {
    pub fn new(params: ForceParams) -> Self {
        let temp = params.temperature;
        Self { params, temp }
    }

    /// Temperatura actual (baja con cada paso — útil para detectar fin).
    pub fn temperature(&self) -> f32 {
        self.temp
    }

    /// Un paso de simulación. Muta las posiciones de `nodes`. Devuelve el
    /// desplazamiento total aplicado (converge hacia 0).
    pub fn step(&mut self, nodes: &mut NodeBuffer, edges: &EdgeBuffer) -> f32 {
        let n = nodes.len();
        if n == 0 {
            return 0.0;
        }
        let k = self.params.k.max(1e-3);
        let mut disp = vec![(0.0f32, 0.0f32); n];

        // Repulsión: todo par. f_r = k² / d.
        for i in 0..n {
            let (xi, yi) = nodes.pos(i);
            for j in (i + 1)..n {
                let (xj, yj) = nodes.pos(j);
                let mut dx = xi - xj;
                let mut dy = yi - yj;
                let mut dist = (dx * dx + dy * dy).sqrt();
                if dist < 1e-3 {
                    // Jitter determinista para despegar nodos coincidentes.
                    dx = ((i as f32) - (j as f32)) * 0.01 + 0.01;
                    dy = 0.01;
                    dist = (dx * dx + dy * dy).sqrt();
                }
                let f = k * k / dist;
                let (ux, uy) = (dx / dist, dy / dist);
                disp[i].0 += ux * f;
                disp[i].1 += uy * f;
                disp[j].0 -= ux * f;
                disp[j].1 -= uy * f;
            }
        }

        // Atracción: a lo largo de cada arista. f_a = d² / k.
        for (u, v) in edges.iter() {
            if u >= n || v >= n || u == v {
                continue;
            }
            let (xu, yu) = nodes.pos(u);
            let (xv, yv) = nodes.pos(v);
            let dx = xu - xv;
            let dy = yu - yv;
            let dist = (dx * dx + dy * dy).sqrt().max(1e-3);
            let f = dist * dist / k;
            let (ux, uy) = (dx / dist, dy / dist);
            disp[u].0 -= ux * f;
            disp[u].1 -= uy * f;
            disp[v].0 += ux * f;
            disp[v].1 += uy * f;
        }

        // Integración con cap de temperatura.
        let mut total = 0.0f32;
        for i in 0..n {
            let (dx, dy) = disp[i];
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 {
                continue;
            }
            let capped = len.min(self.temp);
            let (mx, my) = (dx / len * capped, dy / len * capped);
            let (x, y) = nodes.pos(i);
            nodes.set_pos(i, x + mx, y + my);
            total += capped;
        }
        self.temp *= self.params.cooling;
        total
    }

    /// Igual semántica que [`Self::step`] pero la repulsión se
    /// aproxima con un quadtree Barnes-Hut. `theta` controla la
    /// agresividad de la aproximación (0.0 = exacto/lento, ~0.5
    /// = balance, 1.0 = grosero). La atracción y la integración
    /// quedan iguales — Barnes-Hut sólo aplica al lazo de pares.
    pub fn step_bh(&mut self, nodes: &mut NodeBuffer, edges: &EdgeBuffer, theta: f32) -> f32 {
        let n = nodes.len();
        if n == 0 {
            return 0.0;
        }
        let k = self.params.k.max(1e-3);
        let mut disp = vec![(0.0f32, 0.0f32); n];

        // Repulsión vía Barnes-Hut.
        let positions: Vec<(f32, f32)> = (0..n).map(|i| nodes.pos(i)).collect();
        let qt = Quadtree::build(&positions);
        for i in 0..n {
            let f = qt.force_on(positions[i], i, k, theta.max(0.0));
            disp[i].0 += f.0;
            disp[i].1 += f.1;
        }

        // Atracción a lo largo de aristas, idem `step`.
        for (u, v) in edges.iter() {
            if u >= n || v >= n || u == v {
                continue;
            }
            let (xu, yu) = nodes.pos(u);
            let (xv, yv) = nodes.pos(v);
            let dx = xu - xv;
            let dy = yu - yv;
            let dist = (dx * dx + dy * dy).sqrt().max(1e-3);
            let f = dist * dist / k;
            let (ux, uy) = (dx / dist, dy / dist);
            disp[u].0 -= ux * f;
            disp[u].1 -= uy * f;
            disp[v].0 += ux * f;
            disp[v].1 += uy * f;
        }

        // Integración + cooling, idéntico a `step`.
        let mut total = 0.0f32;
        for i in 0..n {
            let (dx, dy) = disp[i];
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 {
                continue;
            }
            let capped = len.min(self.temp);
            let (mx, my) = (dx / len * capped, dy / len * capped);
            let (x, y) = nodes.pos(i);
            nodes.set_pos(i, x + mx, y + my);
            total += capped;
        }
        self.temp *= self.params.cooling;
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_connected_nodes_settle_near_k() {
        let mut nb = NodeBuffer::new();
        nb.push(0.0, 0.0, 5.0);
        nb.push(500.0, 0.0, 5.0); // arrancan muy lejos
        let mut eb = EdgeBuffer::new();
        eb.push(0, 1);
        let mut fl = ForceLayout::new(ForceParams::default());
        for _ in 0..400 {
            fl.step(&mut nb, &eb);
        }
        let (x0, y0) = nb.pos(0);
        let (x1, y1) = nb.pos(1);
        let dist = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        // No deberían quedar ni pegados ni a 500 de distancia.
        assert!(dist > 5.0 && dist < 300.0, "dist tras converger = {dist}");
    }

    #[test]
    fn coincident_nodes_do_not_nan() {
        let mut nb = NodeBuffer::new();
        nb.push(10.0, 10.0, 5.0);
        nb.push(10.0, 10.0, 5.0);
        let eb = EdgeBuffer::new();
        let mut fl = ForceLayout::new(ForceParams::default());
        fl.step(&mut nb, &eb);
        let (x, y) = nb.pos(0);
        assert!(x.is_finite() && y.is_finite());
    }

    #[test]
    fn empty_graph_is_noop() {
        let mut nb = NodeBuffer::new();
        let eb = EdgeBuffer::new();
        let mut fl = ForceLayout::new(ForceParams::default());
        assert_eq!(fl.step(&mut nb, &eb), 0.0);
    }

    #[test]
    fn step_bh_converges_to_similar_layout_as_naive() {
        // 12 nodos en ciclo. Naïve y Barnes-Hut deberían converger a
        // tamaños similares (dentro del 30%).
        fn build() -> (NodeBuffer, EdgeBuffer) {
            let mut nb = NodeBuffer::new();
            for i in 0..12 {
                let a = (i as f32 / 12.0) * std::f32::consts::TAU;
                nb.push(80.0 * a.cos(), 80.0 * a.sin(), 4.0);
            }
            let mut eb = EdgeBuffer::new();
            for i in 0..12 {
                eb.push(i, (i + 1) % 12);
            }
            (nb, eb)
        }
        let (mut nb_n, eb_n) = build();
        let mut fl_n = ForceLayout::new(ForceParams::default());
        for _ in 0..200 {
            fl_n.step(&mut nb_n, &eb_n);
        }
        let (mut nb_b, eb_b) = build();
        let mut fl_b = ForceLayout::new(ForceParams::default());
        for _ in 0..200 {
            fl_b.step_bh(&mut nb_b, &eb_b, 0.5);
        }
        let radius = |nb: &NodeBuffer| -> f32 {
            let mut rmax = 0.0f32;
            for i in 0..nb.len() {
                let (x, y) = nb.pos(i);
                rmax = rmax.max((x * x + y * y).sqrt());
            }
            rmax
        };
        let r_n = radius(&nb_n);
        let r_b = radius(&nb_b);
        let ratio = (r_b - r_n).abs() / r_n.max(1.0);
        assert!(ratio < 0.30, "naive r={r_n}, BH r={r_b} (>30% off)");
    }

    #[test]
    fn step_bh_empty_is_noop() {
        let mut nb = NodeBuffer::new();
        let eb = EdgeBuffer::new();
        let mut fl = ForceLayout::new(ForceParams::default());
        assert_eq!(fl.step_bh(&mut nb, &eb, 0.5), 0.0);
    }
}
