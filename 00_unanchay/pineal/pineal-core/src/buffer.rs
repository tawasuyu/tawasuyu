//! `DataBuffer` — buffer interleaved `[x0, y0, x1, y1, ...]` con
//! revision counter para invalidación de cachés.
//!
//! Es la primitiva universal de Lapaloma: todo serie cartesiana,
//! todo grafo de nodos, todo OHLC vive en uno de estos (o en una
//! variante con stride distinto). El layout `f32` x `f32` es lo
//! que el GPU consume sin transformación.

/// Buffer de coordenadas planas `[x, y]` empacadas.
///
/// La longitud lógica (número de puntos) es `coords.len() / 2`.
/// Mutar in-place (`set_xy`, `push`) bumpea `revision` — los
/// painters comparan su `last_seen_revision` para decidir si
/// rebuilear su caché.
#[derive(Debug, Clone, Default)]
pub struct DataBuffer {
    coords: Vec<f32>,
    revision: u64,
}

impl DataBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserva espacio para `n` puntos sin agregarlos. Usalo al
    /// montar el widget para que `push` no realloque después.
    pub fn with_capacity(n: usize) -> Self {
        Self {
            coords: Vec::with_capacity(n * 2),
            revision: 0,
        }
    }

    /// Construye a partir de coords interleaved ya armadas.
    /// Útil en tests y carga inicial.
    pub fn from_interleaved(coords: Vec<f32>) -> Self {
        assert!(coords.len() % 2 == 0, "interleaved coords deben ser pares");
        Self {
            coords,
            revision: 0,
        }
    }

    pub fn push(&mut self, x: f32, y: f32) {
        self.coords.push(x);
        self.coords.push(y);
        self.revision = self.revision.wrapping_add(1);
    }

    /// Sobrescribe un punto existente. `i` es el índice de punto
    /// (no de float), 0-based.
    pub fn set_xy(&mut self, i: usize, x: f32, y: f32) {
        self.coords[i * 2] = x;
        self.coords[i * 2 + 1] = y;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Pisa el contenido completo con la nueva slice.
    /// Útil para hidratar el buffer en un solo memcpy.
    pub fn replace_from(&mut self, src: &[f32]) {
        assert!(src.len() % 2 == 0);
        self.coords.clear();
        self.coords.extend_from_slice(src);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn clear(&mut self) {
        self.coords.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn len(&self) -> usize {
        self.coords.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.coords.is_empty()
    }

    pub fn xy(&self, i: usize) -> (f32, f32) {
        (self.coords[i * 2], self.coords[i * 2 + 1])
    }

    /// Slice plana lista para `drawRawPoints` / `wgpu::Buffer`
    /// / `<polyline points>`. No realiza copia.
    pub fn coords(&self) -> &[f32] {
        &self.coords
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_y_len() {
        let mut b = DataBuffer::with_capacity(4);
        b.push(0.0, 1.0);
        b.push(1.0, 2.0);
        assert_eq!(b.len(), 2);
        assert_eq!(b.xy(1), (1.0, 2.0));
    }

    #[test]
    fn revision_bumps() {
        let mut b = DataBuffer::new();
        let r0 = b.revision();
        b.push(0.0, 0.0);
        let r1 = b.revision();
        b.set_xy(0, 1.0, 1.0);
        let r2 = b.revision();
        assert_ne!(r0, r1);
        assert_ne!(r1, r2);
    }

    #[test]
    fn coords_slice_is_zero_copy() {
        let raw = vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0];
        let b = DataBuffer::from_interleaved(raw);
        assert_eq!(b.coords(), &[0.0, 0.0, 1.0, 1.0, 2.0, 2.0]);
        assert_eq!(b.len(), 3);
    }
}
