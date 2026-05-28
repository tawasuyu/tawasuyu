//! FDEB-lite — Force-Directed Edge Bundling (Holten & van Wijk 2009,
//! versión simplificada).
//!
//! Cuando un grafo tiene muchas aristas paralelas o casi-paralelas el
//! resultado del force-directed se ve como "spaghetti". FDEB agrupa
//! las aristas compatibles en haces curvos que comparten trayectoria,
//! revelando la estructura macroscópica del flujo (mismo principio que
//! los mapas de migración o las rutas aéreas).
//!
//! Pipeline:
//! 1. Cada arista se subdivide en `subdivisions` puntos intermedios
//!    (incluyendo los endpoints que quedan fijos).
//! 2. Para cada par de aristas se calcula una *compatibility* en
//!    `[0, 1]` (combinación de paralelismo + escala + cercanía).
//! 3. En cada iteración los puntos intermedios se mueven por:
//!    - Spring force a sus vecinos dentro de la misma arista (mantiene
//!      la integridad del path).
//!    - Electric force atrayendo a puntos correspondientes de aristas
//!      compatibles (bundling).
//! 4. La step size baja en cada iteración (cooling).
//!
//! Los endpoints se mantienen fijos (los nodos no se mueven). El output
//! son los paths como `Vec<Vec<(f32, f32)>>`, listos para `stroke_polyline`.

use crate::buffers::{EdgeBuffer, NodeBuffer};

/// Parámetros del bundling. Defaults razonables para grafos medianos.
#[derive(Debug, Clone, Copy)]
pub struct FdebParams {
    /// Puntos intermedios por edge (sin contar endpoints).
    pub subdivisions: usize,
    /// Iteraciones totales.
    pub iterations: usize,
    /// Step size inicial (px por iteración). Decae linealmente a 0.
    pub step: f32,
    /// Rigidez del spring intra-edge.
    pub spring_k: f32,
    /// Fuerza eléctrica entre edges compatibles.
    pub electric_k: f32,
    /// Umbral de compatibility a partir del cual dos edges interactúan.
    pub compat_threshold: f32,
}

impl Default for FdebParams {
    fn default() -> Self {
        Self {
            subdivisions: 8,
            iterations: 30,
            step: 4.0,
            spring_k: 0.1,
            electric_k: 0.05,
            compat_threshold: 0.6,
        }
    }
}

/// Path bundleado: secuencia de puntos `(x, y)` desde el endpoint
/// origen al destino, incluyendo ambos.
pub type Path = Vec<(f32, f32)>;

/// Genera paths bundleados para cada arista de `edges`. Endpoints fijos
/// (las posiciones de los nodos no se modifican).
pub fn bundle(nodes: &NodeBuffer, edges: &EdgeBuffer, params: FdebParams) -> Vec<Path> {
    let n_edges = edges.len();
    if n_edges == 0 {
        return Vec::new();
    }
    let subs = params.subdivisions.max(1);

    // Inicializa cada path como la línea recta entre los endpoints,
    // muestreada en `subs + 2` puntos.
    let mut paths: Vec<Path> = Vec::with_capacity(n_edges);
    let endpoints: Vec<((f32, f32), (f32, f32))> = (0..n_edges)
        .map(|i| {
            let (u, v) = edges.edge(i);
            let pu = if u < nodes.len() { nodes.pos(u) } else { (0.0, 0.0) };
            let pv = if v < nodes.len() { nodes.pos(v) } else { (0.0, 0.0) };
            (pu, pv)
        })
        .collect();
    for (pu, pv) in &endpoints {
        let mut path = Vec::with_capacity(subs + 2);
        for i in 0..=(subs + 1) {
            let t = i as f32 / (subs + 1) as f32;
            path.push((pu.0 + (pv.0 - pu.0) * t, pu.1 + (pv.1 - pu.1) * t));
        }
        paths.push(path);
    }

    // Matriz de compatibilidad — sparse: sólo guardamos pares con
    // compat ≥ threshold para acelerar el lazo interno.
    let mut compat: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n_edges];
    for i in 0..n_edges {
        let ei = endpoints[i];
        for j in (i + 1)..n_edges {
            let ej = endpoints[j];
            let c = compatibility(ei, ej);
            if c >= params.compat_threshold {
                compat[i].push((j, c));
                compat[j].push((i, c));
            }
        }
    }

    // Iteraciones con cooling lineal.
    for it in 0..params.iterations {
        let cool = 1.0 - (it as f32 / params.iterations as f32);
        let step = params.step * cool.max(0.05);
        let snapshot = paths.clone();

        for e in 0..n_edges {
            // Endpoints fijos: i = 0 y i = subs+1.
            for i in 1..=subs {
                let p = snapshot[e][i];
                let prev = snapshot[e][i - 1];
                let next = snapshot[e][i + 1];

                // Spring: tira hacia el promedio de los vecinos.
                let target = ((prev.0 + next.0) * 0.5, (prev.1 + next.1) * 0.5);
                let dx_sp = (target.0 - p.0) * params.spring_k;
                let dy_sp = (target.1 - p.1) * params.spring_k;

                // Electric: atrae al punto i de cada edge compatible.
                let mut dx_el = 0.0_f32;
                let mut dy_el = 0.0_f32;
                for &(other, c) in &compat[e] {
                    let q = snapshot[other][i];
                    let dx = q.0 - p.0;
                    let dy = q.1 - p.1;
                    let d2 = dx * dx + dy * dy + 1.0;
                    let inv_d = 1.0 / d2.sqrt();
                    let f = params.electric_k * c * inv_d;
                    dx_el += dx * f;
                    dy_el += dy * f;
                }

                let total_dx = dx_sp + dx_el;
                let total_dy = dy_sp + dy_el;
                let len = (total_dx * total_dx + total_dy * total_dy).sqrt();
                let (mx, my) = if len > step {
                    (total_dx / len * step, total_dy / len * step)
                } else {
                    (total_dx, total_dy)
                };
                paths[e][i].0 += mx;
                paths[e][i].1 += my;
            }
        }
    }

    paths
}

/// Compatibility entre dos edges (Holten-lite): producto de angle,
/// scale y position. Devuelve `[0, 1]`.
fn compatibility(a: ((f32, f32), (f32, f32)), b: ((f32, f32), (f32, f32))) -> f32 {
    let va = (a.1 .0 - a.0 .0, a.1 .1 - a.0 .1);
    let vb = (b.1 .0 - b.0 .0, b.1 .1 - b.0 .1);
    let len_a = (va.0 * va.0 + va.1 * va.1).sqrt().max(1e-3);
    let len_b = (vb.0 * vb.0 + vb.1 * vb.1).sqrt().max(1e-3);

    // Angle compat: |cos(theta)|. Edges anti-paralelas también son compatibles.
    let cos_t = (va.0 * vb.0 + va.1 * vb.1) / (len_a * len_b);
    let angle_c = cos_t.abs();

    // Scale compat: 2 * min(la, lb) / ((la + lb) * max(la, lb) / min) — simplificado.
    let lmin = len_a.min(len_b);
    let lmax = len_a.max(len_b);
    let l_avg = (len_a + len_b) * 0.5;
    let scale_c = 2.0 / (l_avg / lmin + lmax / l_avg);

    // Position compat: cercanía de midpoints relativa a length promedio.
    let mid_a = ((a.0 .0 + a.1 .0) * 0.5, (a.0 .1 + a.1 .1) * 0.5);
    let mid_b = ((b.0 .0 + b.1 .0) * 0.5, (b.0 .1 + b.1 .1) * 0.5);
    let d_mid =
        ((mid_a.0 - mid_b.0).powi(2) + (mid_a.1 - mid_b.1).powi(2)).sqrt();
    let position_c = l_avg / (l_avg + d_mid);

    angle_c * scale_c * position_c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_edges(positions: &[(f32, f32)], pairs: &[(usize, usize)]) -> (NodeBuffer, EdgeBuffer) {
        let mut nb = NodeBuffer::new();
        for &(x, y) in positions {
            nb.push(x, y, 3.0);
        }
        let mut eb = EdgeBuffer::new();
        for &(u, v) in pairs {
            eb.push(u, v);
        }
        (nb, eb)
    }

    #[test]
    fn empty_input_returns_empty() {
        let nb = NodeBuffer::new();
        let eb = EdgeBuffer::new();
        let paths = bundle(&nb, &eb, FdebParams::default());
        assert!(paths.is_empty());
    }

    #[test]
    fn endpoints_stay_fixed_after_bundle() {
        // 4 nodos, 2 edges paralelas (0→1, 2→3).
        let (nb, eb) = make_edges(
            &[(0.0, 0.0), (100.0, 0.0), (0.0, 20.0), (100.0, 20.0)],
            &[(0, 1), (2, 3)],
        );
        let paths = bundle(&nb, &eb, FdebParams::default());
        // Endpoints intactos.
        assert!((paths[0][0].0 - 0.0).abs() < 1e-3 && (paths[0][0].1 - 0.0).abs() < 1e-3);
        let last0 = *paths[0].last().unwrap();
        assert!((last0.0 - 100.0).abs() < 1e-3);
    }

    #[test]
    fn parallel_edges_get_pulled_together() {
        // Dos edges horizontales separadas verticalmente: tras bundle
        // los midpoints deberían acercarse.
        let (nb, eb) = make_edges(
            &[(0.0, 0.0), (200.0, 0.0), (0.0, 40.0), (200.0, 40.0)],
            &[(0, 1), (2, 3)],
        );
        let mut params = FdebParams::default();
        params.iterations = 60;
        params.electric_k = 0.2;
        let paths = bundle(&nb, &eb, params);
        let mid_idx = paths[0].len() / 2;
        let m0 = paths[0][mid_idx];
        let m1 = paths[1][mid_idx];
        let dy = (m0.1 - m1.1).abs();
        assert!(dy < 40.0, "esperaba bundling, midpoints siguen a dy={dy}");
    }

    #[test]
    fn orthogonal_edges_do_not_bundle() {
        // Edge horizontal + edge vertical: compat angular ≈ 0.
        let (nb, eb) = make_edges(
            &[(0.0, 0.0), (200.0, 0.0), (100.0, -100.0), (100.0, 100.0)],
            &[(0, 1), (2, 3)],
        );
        let paths = bundle(&nb, &eb, FdebParams::default());
        // El path original era recto y debería seguir cerca de recto.
        let mid = paths[0][paths[0].len() / 2];
        // Midpoint del path original recto: (100, 0). Si bundleó con el
        // vertical, se habría movido lejos. Tolerancia generosa.
        assert!(mid.1.abs() < 5.0, "edge horizontal se torció: {mid:?}");
    }

    #[test]
    fn path_has_subdivisions_plus_two_points() {
        let (nb, eb) = make_edges(&[(0.0, 0.0), (100.0, 0.0)], &[(0, 1)]);
        let mut params = FdebParams::default();
        params.subdivisions = 10;
        let paths = bundle(&nb, &eb, params);
        assert_eq!(paths[0].len(), 12);
    }
}
