//! Índice espacial determinista — bin de agentes por celda.
//!
//! El contagio social ingenuo es O(N²) — perfectamente aceptable hasta ~5k
//! agentes pero se vuelve cuello de botella en sweeps Monte Carlo o en
//! poblaciones grandes (`abundance_threshold` bajo + `regrowth_rate` alto
//! pueden empujar N por encima de 10k).
//!
//! Este módulo bin'ea los agentes en una grilla regular de paso `cell_size`
//! y permite recoger los índices vecinos en un radio `r` en O(K) donde K es
//! la cantidad real de vecinos en las 9 celdas adyacentes. La salida se
//! devuelve **ordenada ascendentemente** — esto vuelve la suma del contagio
//! social bit-exacta respecto a la versión O(N²) (que también suma en orden
//! ascendente de índice), aún cuando la población crezca.
//!
//! Determinismo total: sin RNG, sin paralelismo, sin HashMap. El sort interno
//! es `sort_unstable` sobre `u32` — comparación total y estable
//! cross-platform.

/// Índice por celda. `cells[id]` contiene los índices de los agentes cuyo
/// `(pos_x, pos_y)` cae dentro de esa celda. El id se mapea `(cx, cy)` →
/// `cy * nx + cx`. Los agentes con posición fuera del rango cubierto por la
/// grilla se clampean a la celda de borde más cercana — mantiene la
/// invariante "todos los agentes están en exactamente una celda".
#[derive(Debug, Clone)]
pub struct CellIndex {
    pub cells: Vec<Vec<u32>>,
    pub nx: usize,
    pub ny: usize,
    pub cell_size: f32,
}

impl CellIndex {
    /// Construye el índice. `cell_size` debe ser positivo; un valor sano es
    /// el radio social del contagio (con celdas más grandes habrá más vecinos
    /// candidatos por celda pero menos celdas visitadas; lo contrario con
    /// celdas más chicas — el trade-off típico es `cell_size ≈ radius`).
    ///
    /// `min_x/y` y `max_x/y` definen el rectángulo que el índice cubre.
    /// Posiciones fuera quedan clampedas. Un grilla 80×80 normalmente pasa
    /// `0.0, 0.0, 79.0, 79.0`.
    pub fn build(
        xs: &[f32],
        ys: &[f32],
        min_x: f32,
        min_y: f32,
        max_x: f32,
        max_y: f32,
        cell_size: f32,
    ) -> Self {
        assert!(cell_size > 0.0, "cell_size debe ser positivo");
        let span_x = (max_x - min_x).max(cell_size);
        let span_y = (max_y - min_y).max(cell_size);
        let nx = ((span_x / cell_size).ceil() as usize).max(1);
        let ny = ((span_y / cell_size).ceil() as usize).max(1);
        let total = nx * ny;
        let mut cells: Vec<Vec<u32>> = vec![Vec::new(); total];
        let n = xs.len();
        for i in 0..n {
            let cx_raw = ((xs[i] - min_x) / cell_size).floor() as i64;
            let cy_raw = ((ys[i] - min_y) / cell_size).floor() as i64;
            let cx = cx_raw.clamp(0, nx as i64 - 1) as usize;
            let cy = cy_raw.clamp(0, ny as i64 - 1) as usize;
            let id = cy * nx + cx;
            cells[id].push(i as u32);
        }
        Self { cells, nx, ny, cell_size }
    }

    /// Vecinos candidatos del agente en `(x, y)` dentro de las 9 celdas
    /// adyacentes (la propia + las 8 alrededor). El llamador debe filtrar
    /// por distancia real (este método no la mide) y opcionalmente excluir
    /// el propio índice del agente.
    ///
    /// Los índices se devuelven **ordenados ascendentemente**. Esto preserva
    /// la igualdad bit-exacta con un sweep ingenuo O(N²) que itera `0..N`
    /// en orden lineal — la suma de `f32` depende del orden y vamos a sumar
    /// `psi_j` sobre estos índices.
    pub fn candidates_sorted(&self, x: f32, y: f32, min_x: f32, min_y: f32, out: &mut Vec<u32>) {
        out.clear();
        let cx = (((x - min_x) / self.cell_size).floor() as i64)
            .clamp(0, self.nx as i64 - 1) as usize;
        let cy = (((y - min_y) / self.cell_size).floor() as i64)
            .clamp(0, self.ny as i64 - 1) as usize;
        let cx_lo = cx.saturating_sub(1);
        let cx_hi = (cx + 1).min(self.nx - 1);
        let cy_lo = cy.saturating_sub(1);
        let cy_hi = (cy + 1).min(self.ny - 1);
        for ccy in cy_lo..=cy_hi {
            for ccx in cx_lo..=cx_hi {
                let id = ccy * self.nx + ccx;
                out.extend_from_slice(&self.cells[id]);
            }
        }
        out.sort_unstable();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_distributes_agents_to_correct_cells() {
        let xs = vec![0.5, 1.5, 5.0, 9.9];
        let ys = vec![0.5, 0.5, 5.0, 9.9];
        let idx = CellIndex::build(&xs, &ys, 0.0, 0.0, 10.0, 10.0, 2.0);
        // 5 celdas × 5 = 25 bins (span 10 / cell 2 = 5).
        assert_eq!(idx.nx, 5);
        assert_eq!(idx.ny, 5);
        // Agentes 0,1 caen en cell (0,0); agente 2 en (2,2); agente 3 en (4,4).
        let count: usize = idx.cells.iter().map(|c| c.len()).sum();
        assert_eq!(count, 4);
        assert_eq!(idx.cells[0], vec![0, 1]);
    }

    #[test]
    fn candidates_sorted_returns_ascending_indices() {
        // Agentes alineados en y=5, x=0..9. Query en x=5, y=5, cell 2.
        // Sólo las 3 celdas adyacentes (cx 1..=3) → x=2..7 → idxs 2..=7.
        let xs: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let ys: Vec<f32> = vec![5.0; 10];
        let idx = CellIndex::build(&xs, &ys, 0.0, 0.0, 10.0, 10.0, 2.0);
        let mut buf = Vec::new();
        idx.candidates_sorted(5.0, 5.0, 0.0, 0.0, &mut buf);
        // Ordenado ascendente.
        for w in buf.windows(2) {
            assert!(w[0] < w[1]);
        }
        // Contiene al menos los vecinos directos del centro (id 5).
        assert!(buf.contains(&5));
    }

    #[test]
    fn out_of_bounds_positions_clamp_to_edge_cell() {
        let xs = vec![-100.0, 100.0];
        let ys = vec![-100.0, 100.0];
        let idx = CellIndex::build(&xs, &ys, 0.0, 0.0, 10.0, 10.0, 2.0);
        let total: usize = idx.cells.iter().map(|c| c.len()).sum();
        assert_eq!(total, 2, "los 2 agentes deben estar bin'ados igual");
    }
}
