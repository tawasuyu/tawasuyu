//! Quadtree Barnes-Hut para aproximar la fuerza repulsiva del
//! force-directed en O(n log n) en lugar de O(n²).
//!
//! La idea: si un nodo lejano y un grupo de nodos cumplen
//! `half_size / dist < theta`, el grupo se aproxima por su centro de masa.
//! Theta típico: 0.5 — más bajo = más preciso, más lento.
//!
//! Representación: `Vec<QuadNode>` indexado. El doc original de
//! `pineal-core::barnes_hut` propone stride 7 plano; acá optamos por la
//! versión idiomática — clara y suficiente para los rangos en que el
//! force-directed corre interactivo (hasta ~50 K nodos).

/// Un nodo del árbol — puede ser hoja con una partícula, hoja vacía, o
/// nodo interno con hasta 4 hijos.
#[derive(Debug, Clone, Copy)]
struct QuadNode {
    /// Centro del cuadrante (no es el center of mass; eso vive en `cm`).
    cx: f32,
    cy: f32,
    /// Medio-lado del cuadrante.
    half: f32,
    /// Center of mass acumulado.
    cm_x: f32,
    cm_y: f32,
    /// Masa total (suma de pesos — acá usamos 1.0 por nodo).
    mass: f32,
    /// Hoja con exactamente 1 cuerpo: índice del cuerpo en `positions`.
    body: Option<usize>,
    /// Índices a los 4 hijos (NW, NE, SW, SE) en `Quadtree::nodes`.
    children: [Option<usize>; 4],
}

impl QuadNode {
    fn new(cx: f32, cy: f32, half: f32) -> Self {
        Self {
            cx,
            cy,
            half,
            cm_x: 0.0,
            cm_y: 0.0,
            mass: 0.0,
            body: None,
            children: [None; 4],
        }
    }

    fn is_internal(&self) -> bool {
        self.children.iter().any(Option::is_some)
    }

    /// Cuadrante (0..=3) al que cae `(x, y)` respecto al centro.
    /// 0 = NW (x<cx, y<cy), 1 = NE (x>=cx, y<cy), 2 = SW, 3 = SE.
    fn quadrant_of(&self, x: f32, y: f32) -> usize {
        let east = x >= self.cx;
        let south = y >= self.cy;
        match (east, south) {
            (false, false) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        }
    }

    /// Centro del cuadrante hijo `q`.
    fn child_center(&self, q: usize) -> (f32, f32) {
        let h = self.half * 0.5;
        match q {
            0 => (self.cx - h, self.cy - h),
            1 => (self.cx + h, self.cy - h),
            2 => (self.cx - h, self.cy + h),
            3 => (self.cx + h, self.cy + h),
            _ => unreachable!(),
        }
    }
}

/// Quadtree construido sobre un set fijo de posiciones.
pub struct Quadtree {
    nodes: Vec<QuadNode>,
    root: Option<usize>,
}

impl Quadtree {
    /// Construye el árbol cubriendo todas las posiciones con una raíz
    /// cuadrada ajustada al bounding box (con margen de seguridad).
    pub fn build(positions: &[(f32, f32)]) -> Self {
        if positions.is_empty() {
            return Self { nodes: Vec::new(), root: None };
        }
        let (mut min_x, mut min_y) = positions[0];
        let (mut max_x, mut max_y) = positions[0];
        for &(x, y) in &positions[1..] {
            if x < min_x {
                min_x = x;
            }
            if x > max_x {
                max_x = x;
            }
            if y < min_y {
                min_y = y;
            }
            if y > max_y {
                max_y = y;
            }
        }
        let cx = (min_x + max_x) * 0.5;
        let cy = (min_y + max_y) * 0.5;
        // `half` cubre la dimensión más grande con margen — evita que un
        // body recién en el borde se pierda por error de redondeo.
        let half = ((max_x - min_x).max(max_y - min_y) * 0.5).max(1e-3) + 1.0;

        let mut tree = Self { nodes: Vec::with_capacity(positions.len() * 4), root: None };
        tree.nodes.push(QuadNode::new(cx, cy, half));
        tree.root = Some(0);
        for (i, &p) in positions.iter().enumerate() {
            tree.insert(0, i, p, 0);
        }
        tree
    }

    /// Inserta el body `body_idx` en el subárbol enraizado en `node_idx`.
    /// `depth` es un guardabarros: tras un umbral abandonamos la
    /// subdivisión (bodies degenerados muy próximos sólo se ven en
    /// patológicos — el integrador con jitter ya los separa).
    fn insert(&mut self, node_idx: usize, body_idx: usize, p: (f32, f32), depth: u32) {
        const MAX_DEPTH: u32 = 32;
        let (mass, cm) = (self.nodes[node_idx].mass, (self.nodes[node_idx].cm_x, self.nodes[node_idx].cm_y));
        // Actualiza COM acumulado: weighted mean.
        let new_mass = mass + 1.0;
        let cm_x = (cm.0 * mass + p.0) / new_mass;
        let cm_y = (cm.1 * mass + p.1) / new_mass;
        self.nodes[node_idx].mass = new_mass;
        self.nodes[node_idx].cm_x = cm_x;
        self.nodes[node_idx].cm_y = cm_y;

        // Hoja vacía: ocupar.
        if self.nodes[node_idx].body.is_none() && !self.nodes[node_idx].is_internal() {
            self.nodes[node_idx].body = Some(body_idx);
            return;
        }
        // Hoja con body previo: hay que subdividir.
        if let Some(prev_body) = self.nodes[node_idx].body.take() {
            // Reubicar el body previo (necesitamos su posición — la
            // recuperamos vía las componentes que metimos en cm).
            // Para evitar recomputar todo, asumimos que el caller no
            // muta `positions` entre inserts y nos basta la posición
            // que reconstruimos del cm previo (mass == 1 antes del
            // update significa que cm == posición del previo body).
            let prev_pos = if mass == 1.0 { cm } else { (cm.0, cm.1) };
            if depth >= MAX_DEPTH {
                // Bodies coincidentes en profundidad máxima: dejamos el
                // body re-asignado al mismo nodo. El cm sigue siendo
                // válido, sólo perdemos la separación. Ralo en practice.
                self.nodes[node_idx].body = Some(prev_body);
                return;
            }
            self.descend_and_insert(node_idx, prev_body, prev_pos, depth);
        }
        // Insertar el nuevo body en el hijo apropiado.
        self.descend_and_insert(node_idx, body_idx, p, depth);
    }

    fn descend_and_insert(&mut self, node_idx: usize, body_idx: usize, p: (f32, f32), depth: u32) {
        let q = self.nodes[node_idx].quadrant_of(p.0, p.1);
        let child_idx = match self.nodes[node_idx].children[q] {
            Some(ix) => ix,
            None => {
                let (ccx, ccy) = self.nodes[node_idx].child_center(q);
                let ch = QuadNode::new(ccx, ccy, self.nodes[node_idx].half * 0.5);
                let new_idx = self.nodes.len();
                self.nodes.push(ch);
                self.nodes[node_idx].children[q] = Some(new_idx);
                new_idx
            }
        };
        self.insert(child_idx, body_idx, p, depth + 1);
    }

    /// Fuerza repulsiva `f_r = k² / d` sobre la partícula en `target_pos`
    /// (con índice `target_idx`, para que no se atraiga a sí misma).
    /// Aproxima clusters lejanos usando `theta`.
    pub fn force_on(&self, target_pos: (f32, f32), target_idx: usize, k: f32, theta: f32) -> (f32, f32) {
        let Some(root) = self.root else { return (0.0, 0.0) };
        let mut acc = (0.0f32, 0.0f32);
        self.accumulate(root, target_pos, target_idx, k, theta, &mut acc);
        acc
    }

    fn accumulate(
        &self,
        node_idx: usize,
        p: (f32, f32),
        target_idx: usize,
        k: f32,
        theta: f32,
        acc: &mut (f32, f32),
    ) {
        let n = &self.nodes[node_idx];
        if n.mass <= 0.0 {
            return;
        }
        // Si es hoja con el mismo body, skip.
        if let Some(body) = n.body {
            if body == target_idx {
                return;
            }
        }
        let dx = p.0 - n.cm_x;
        let dy = p.1 - n.cm_y;
        let dist2 = dx * dx + dy * dy;
        let dist = dist2.sqrt().max(1e-3);
        let s = n.half * 2.0; // lado completo del cuadrante
        // Criterio MAC: hoja, o lejano enough → aproximamos.
        if n.body.is_some() || (s / dist) < theta {
            let f = k * k * n.mass / dist;
            acc.0 += (dx / dist) * f;
            acc.1 += (dy / dist) * f;
            return;
        }
        for c in n.children.iter().flatten() {
            self.accumulate(*c, p, target_idx, k, theta, acc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tree_has_no_force() {
        let qt = Quadtree::build(&[]);
        assert_eq!(qt.force_on((0.0, 0.0), 0, 10.0, 0.5), (0.0, 0.0));
    }

    #[test]
    fn single_body_self_skipped() {
        let qt = Quadtree::build(&[(0.0, 0.0)]);
        assert_eq!(qt.force_on((0.0, 0.0), 0, 10.0, 0.5), (0.0, 0.0));
    }

    #[test]
    fn two_bodies_repel_along_axis() {
        let qt = Quadtree::build(&[(0.0, 0.0), (100.0, 0.0)]);
        let f = qt.force_on((0.0, 0.0), 0, 10.0, 0.5);
        // Body en (100,0): empuja a body 0 hacia -x.
        assert!(f.0 < 0.0, "esperaba fuerza hacia -x, got {f:?}");
        assert!(f.1.abs() < 1e-3);
    }

    #[test]
    fn bh_approximates_naive_for_distant_clusters() {
        // Cluster lejano de 50 nodos vs un body cercano. BH con theta
        // generoso debería dar fuerza similar a la suma naïve.
        let mut positions = Vec::with_capacity(51);
        positions.push((0.0, 0.0)); // target
        for i in 0..50 {
            let a = (i as f32 / 50.0) * std::f32::consts::TAU;
            positions.push((500.0 + 5.0 * a.cos(), 500.0 + 5.0 * a.sin()));
        }
        let qt = Quadtree::build(&positions);
        let f_bh = qt.force_on(positions[0], 0, 10.0, 0.7);

        // Naïve para comparar.
        let k = 10.0;
        let mut f_n = (0.0f32, 0.0);
        for (i, &p) in positions.iter().enumerate() {
            if i == 0 {
                continue;
            }
            let dx = positions[0].0 - p.0;
            let dy = positions[0].1 - p.1;
            let dist = (dx * dx + dy * dy).sqrt().max(1e-3);
            let f = k * k / dist;
            f_n.0 += (dx / dist) * f;
            f_n.1 += (dy / dist) * f;
        }
        // Comparación: misma dirección y magnitud dentro del 30%.
        assert!(f_bh.0 * f_n.0 > 0.0, "signo discrepa: bh={f_bh:?} naive={f_n:?}");
        let mag_bh = (f_bh.0 * f_bh.0 + f_bh.1 * f_bh.1).sqrt();
        let mag_n = (f_n.0 * f_n.0 + f_n.1 * f_n.1).sqrt();
        assert!(
            (mag_bh - mag_n).abs() / mag_n < 0.30,
            "magnitud lejos: bh={mag_bh}, naive={mag_n}"
        );
    }

    #[test]
    fn coincident_bodies_do_not_explode() {
        let qt = Quadtree::build(&[(10.0, 10.0), (10.0, 10.0), (10.0, 10.0)]);
        let f = qt.force_on((10.0, 10.0), 0, 10.0, 0.5);
        assert!(f.0.is_finite() && f.1.is_finite());
    }
}
