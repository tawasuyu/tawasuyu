//! `HeatmapMatrix` — matriz densa `width × height` de `f32`.

/// Matriz de valores para un heatmap. `revision` se incrementa en cada
/// mutación — los backends lo usan para invalidar la textura cacheada.
#[derive(Debug, Clone)]
pub struct HeatmapMatrix {
    data: Vec<f32>,
    width: usize,
    height: usize,
    revision: u64,
}

impl HeatmapMatrix {
    /// Matriz de ceros de `width × height`.
    pub fn new(width: usize, height: usize) -> Self {
        Self { data: vec![0.0; width * height], width, height, revision: 0 }
    }

    /// Construye desde datos crudos. `None` si `data.len() != width*height`.
    pub fn from_data(data: Vec<f32>, width: usize, height: usize) -> Option<Self> {
        if data.len() != width * height {
            return None;
        }
        Some(Self { data, width, height, revision: 0 })
    }

    pub fn width(&self) -> usize { self.width }
    pub fn height(&self) -> usize { self.height }
    pub fn revision(&self) -> u64 { self.revision }
    pub fn data(&self) -> &[f32] { &self.data }

    /// Valor en `(x, y)`. `0.0` si está fuera de rango.
    pub fn get(&self, x: usize, y: usize) -> f32 {
        if x >= self.width || y >= self.height {
            return 0.0;
        }
        self.data[y * self.width + x]
    }

    /// Fija el valor en `(x, y)` e incrementa `revision`. No-op si está
    /// fuera de rango.
    pub fn set(&mut self, x: usize, y: usize, v: f32) {
        if x >= self.width || y >= self.height {
            return;
        }
        self.data[y * self.width + x] = v;
        self.revision += 1;
    }

    /// Reemplaza todos los datos (mismas dimensiones) e incrementa
    /// `revision`. No-op si la longitud no coincide.
    pub fn replace_data(&mut self, data: Vec<f32>) {
        if data.len() == self.width * self.height {
            self.data = data;
            self.revision += 1;
        }
    }

    /// `(min, max)` de los valores. `(0.0, 0.0)` si la matriz está vacía.
    pub fn min_max(&self) -> (f32, f32) {
        let mut it = self.data.iter().copied();
        let Some(first) = it.next() else { return (0.0, 0.0) };
        it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_data_checks_length() {
        assert!(HeatmapMatrix::from_data(vec![1.0; 6], 2, 3).is_some());
        assert!(HeatmapMatrix::from_data(vec![1.0; 5], 2, 3).is_none());
    }

    #[test]
    fn get_set_and_revision() {
        let mut m = HeatmapMatrix::new(3, 2);
        assert_eq!(m.revision(), 0);
        m.set(1, 1, 4.5);
        assert_eq!(m.get(1, 1), 4.5);
        assert_eq!(m.revision(), 1);
        m.set(99, 99, 1.0); // fuera de rango → no-op
        assert_eq!(m.revision(), 1);
    }

    #[test]
    fn min_max_over_values() {
        let m = HeatmapMatrix::from_data(vec![3.0, -1.0, 7.0, 2.0], 2, 2).unwrap();
        assert_eq!(m.min_max(), (-1.0, 7.0));
    }
}
