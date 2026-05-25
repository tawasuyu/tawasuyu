//! Buffers planos de nodos y aristas — `Vec` contiguos con stride fijo.

/// Nodos: stride 3 = `[x, y, radius]` por nodo.
#[derive(Debug, Clone, Default)]
pub struct NodeBuffer {
    data: Vec<f32>,
}

impl NodeBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(n: usize) -> Self {
        Self { data: Vec::with_capacity(n * 3) }
    }

    /// Agrega un nodo y devuelve su índice.
    pub fn push(&mut self, x: f32, y: f32, radius: f32) -> usize {
        let idx = self.len();
        self.data.extend_from_slice(&[x, y, radius]);
        idx
    }

    pub fn len(&self) -> usize {
        self.data.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn pos(&self, i: usize) -> (f32, f32) {
        (self.data[i * 3], self.data[i * 3 + 1])
    }

    pub fn radius(&self, i: usize) -> f32 {
        self.data[i * 3 + 2]
    }

    pub fn set_pos(&mut self, i: usize, x: f32, y: f32) {
        self.data[i * 3] = x;
        self.data[i * 3 + 1] = y;
    }

    /// Acceso crudo al `Vec<f32>` interleaved — para subir como buffer GPU.
    pub fn raw(&self) -> &[f32] {
        &self.data
    }
}

/// Aristas: stride 2 = `[from, to]` (índices de nodo).
#[derive(Debug, Clone, Default)]
pub struct EdgeBuffer {
    data: Vec<u32>,
}

impl EdgeBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, from: usize, to: usize) {
        self.data.push(from as u32);
        self.data.push(to as u32);
    }

    pub fn len(&self) -> usize {
        self.data.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn edge(&self, i: usize) -> (usize, usize) {
        (self.data[i * 2] as usize, self.data[i * 2 + 1] as usize)
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..self.len()).map(move |i| self.edge(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_buffer_push_and_access() {
        let mut nb = NodeBuffer::new();
        let a = nb.push(1.0, 2.0, 5.0);
        let b = nb.push(3.0, 4.0, 6.0);
        assert_eq!((a, b), (0, 1));
        assert_eq!(nb.len(), 2);
        assert_eq!(nb.pos(1), (3.0, 4.0));
        assert_eq!(nb.radius(0), 5.0);
        nb.set_pos(0, 9.0, 9.0);
        assert_eq!(nb.pos(0), (9.0, 9.0));
    }

    #[test]
    fn edge_buffer_roundtrip() {
        let mut eb = EdgeBuffer::new();
        eb.push(0, 1);
        eb.push(1, 2);
        assert_eq!(eb.len(), 2);
        assert_eq!(eb.edge(1), (1, 2));
        assert_eq!(eb.iter().collect::<Vec<_>>(), vec![(0, 1), (1, 2)]);
    }
}
