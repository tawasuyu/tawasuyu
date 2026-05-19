//! `RingBuffer` — buffer circular de samples para streaming tipo
//! osciloscopio.
//!
//! Capacidad fija. `push(v)` hace dos writes (uno a `values`, uno
//! a `coords[head*2+1]`) y un increment de head + revision. El
//! buffer **nunca se reasigna**; el painter consume slices del
//! mismo backing memory frame tras frame.
//!
//! Convención: `x_norm` se pre-computa una vez en construcción
//! (modo sweep). El painter aplica el escalado a píxeles via su
//! propio transform — el buffer no rota X entre frames.
//!
//! ## Trampa del pre-fill (1.0.2 fix del Flutter)
//!
//! Antes que `count >= capacity`, los slots `[head, capacity)`
//! contienen ceros iniciales. Si el painter dibuja toda la
//! ringa, aparece una línea plana sobre la mitad derecha. La
//! API expone [`RingBuffer::filled_len`] que devuelve `head` en
//! ese caso, y `capacity` después — el painter clipea a eso.

/// Ring buffer en modo sweep (x_norm de cada slot es fijo).
///
/// Para modo scroll el painter aplica un translate adicional por
/// frame; la estructura de datos es la misma.
#[derive(Debug, Clone)]
pub struct RingBuffer {
    /// Sample raw por slot.
    values: Vec<f32>,
    /// `[x_norm, y_value]` por slot. `x_norm = slot / (cap - 1)`,
    /// fijo. `y_value` = `values[slot]`.
    coords: Vec<f32>,
    capacity: usize,
    /// Próximo slot a escribir.
    head: usize,
    /// Monotonic, sobrevive wraparound. Útil para anclar
    /// anotaciones por sample index absoluto.
    count: u64,
    revision: u64,
}

impl RingBuffer {
    /// Asume `capacity >= 2` para que `x_norm` no divida por cero.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity >= 2, "RingBuffer requiere capacity >= 2");
        let mut coords = vec![0.0; capacity * 2];
        let denom = (capacity - 1) as f32;
        for slot in 0..capacity {
            coords[slot * 2] = slot as f32 / denom;
        }
        Self {
            values: vec![0.0; capacity],
            coords,
            capacity,
            head: 0,
            count: 0,
            revision: 0,
        }
    }

    pub fn push(&mut self, v: f32) {
        self.values[self.head] = v;
        self.coords[self.head * 2 + 1] = v;
        self.head = (self.head + 1) % self.capacity;
        self.count = self.count.wrapping_add(1);
        self.revision = self.revision.wrapping_add(1);
    }

    /// Inserción en batch con dos memcpys (cola + wrap-around).
    /// Para batches > capacity se queda con los últimos `capacity`
    /// samples (los anteriores se sobreescribirían igual).
    pub fn push_all(&mut self, batch: &[f32]) {
        if batch.is_empty() {
            return;
        }

        let cap = self.capacity;
        let src = if batch.len() > cap {
            &batch[batch.len() - cap..]
        } else {
            batch
        };

        let tail = cap - self.head;
        if src.len() <= tail {
            self.values[self.head..self.head + src.len()].copy_from_slice(src);
            for (i, v) in src.iter().enumerate() {
                self.coords[(self.head + i) * 2 + 1] = *v;
            }
            self.head = (self.head + src.len()) % cap;
        } else {
            let (a, b) = src.split_at(tail);
            self.values[self.head..].copy_from_slice(a);
            for (i, v) in a.iter().enumerate() {
                self.coords[(self.head + i) * 2 + 1] = *v;
            }
            self.values[..b.len()].copy_from_slice(b);
            for (i, v) in b.iter().enumerate() {
                self.coords[i * 2 + 1] = *v;
            }
            self.head = b.len();
        }

        self.count = self.count.wrapping_add(src.len() as u64);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn head(&self) -> usize {
        self.head
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_full(&self) -> bool {
        self.count >= self.capacity as u64
    }

    /// Cantidad de slots con datos reales. Antes del fill es
    /// `head`; después es `capacity`. El painter clipea a este
    /// valor para evitar el flicker del pre-fill.
    pub fn filled_len(&self) -> usize {
        if self.is_full() {
            self.capacity
        } else {
            self.head
        }
    }

    /// Slice interleaved de `[x_norm, y]`. Para render en dos
    /// segmentos: `&coords()[..head*2]` y `&coords()[head*2..]`
    /// (cuando is_full).
    pub fn coords(&self) -> &[f32] {
        &self.coords
    }

    /// Slice plana de samples raw — útil para downsample envelope
    /// min/max sin pasar por coords.
    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_norm_precomputado() {
        let r = RingBuffer::new(4);
        // x_norm en slots 0, 1, 2, 3 = 0.0, 1/3, 2/3, 1.0
        assert!((r.coords()[0] - 0.0).abs() < 1e-6);
        assert!((r.coords()[2] - 1.0 / 3.0).abs() < 1e-6);
        assert!((r.coords()[6] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn push_actualiza_y_no_x() {
        let mut r = RingBuffer::new(4);
        r.push(5.0);
        r.push(7.0);
        // slot 0 → y=5, slot 1 → y=7, x quedó igual
        assert_eq!(r.coords()[1], 5.0);
        assert_eq!(r.coords()[3], 7.0);
        assert!((r.coords()[2] - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(r.head(), 2);
        assert_eq!(r.count(), 2);
    }

    #[test]
    fn filled_len_bloquea_prefill() {
        let mut r = RingBuffer::new(4);
        assert_eq!(r.filled_len(), 0);
        r.push(1.0);
        r.push(2.0);
        assert_eq!(r.filled_len(), 2);
        r.push(3.0);
        r.push(4.0);
        assert_eq!(r.filled_len(), 4);
        r.push(5.0); // wrap
        assert_eq!(r.filled_len(), 4);
        assert!(r.is_full());
    }

    #[test]
    fn push_all_wrap_around() {
        let mut r = RingBuffer::new(4);
        r.push_all(&[1.0, 2.0, 3.0]); // head=3
        r.push_all(&[4.0, 5.0, 6.0]); // wrap: 4 en slot 3, 5 en slot 0, 6 en slot 1
        assert_eq!(r.values()[3], 4.0);
        assert_eq!(r.values()[0], 5.0);
        assert_eq!(r.values()[1], 6.0);
        assert_eq!(r.head(), 2);
        assert_eq!(r.count(), 6);
    }

    #[test]
    fn push_all_oversized_se_queda_con_la_cola() {
        let mut r = RingBuffer::new(4);
        r.push_all(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        // Sólo los últimos 4 importan: [6,7,8,9]
        assert_eq!(r.values()[0], 6.0);
        assert_eq!(r.values()[1], 7.0);
        assert_eq!(r.values()[2], 8.0);
        assert_eq!(r.values()[3], 9.0);
        assert!(r.is_full());
    }
}
