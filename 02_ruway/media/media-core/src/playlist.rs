//! playlist — modelo de orden de la cola de reproducción (U1 de
//! `PARIDAD.md`), agnóstico de cómo se decodifica cada entrada.
//!
//! Hoy la `Playlist` "viva" de `media-app` mezcla el orden con los
//! decoders cargados; este módulo extrae la parte que el **editor de
//! playlist** necesita (regla #2: la lógica de dominio no sabe quién la
//! pinta): la lista de entradas, el cursor de la que suena, y las
//! operaciones de edición —reordenar arrastrando, encolar "reproducir a
//! continuación", quitar, guardar/cargar— manteniendo el cursor apuntando
//! a la **misma entrada lógica** tras cada permutación.
//!
//! Cada entrada se identifica por una `String` (ruta/URL/clave — misma
//! convención que [`crate::library`]). El modo repeat y el shuffle viven
//! acá como estado serializable; la permutación de shuffle se deriva de
//! una semilla ([`Playlist::shuffle_order`]) para ser determinista y
//! testeable (la app pasa una semilla derivada del reloj).

use serde::{Deserialize, Serialize};

/// Política de fin de cola, espejo del control de VLC/mpv.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Repeat {
    /// Para al terminar la última entrada.
    #[default]
    Off,
    /// Repite la entrada actual indefinidamente.
    One,
    /// Vuelve al principio al pasar la última.
    All,
}

impl Repeat {
    /// Ciclo del botón, estilo VLC: Off → All → One → Off.
    pub fn cycle(self) -> Repeat {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }

    /// Slug estable (etiqueta de UI / forma en disco).
    pub fn slug(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::One => "one",
            Repeat::All => "all",
        }
    }
}

/// Cola de reproducción editable. El cursor [`Playlist::current_index`]
/// señala la entrada que suena (o `None` si no hay ninguna cargada).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    entries: Vec<String>,
    current: Option<usize>,
    repeat: Repeat,
    shuffle: bool,
}

impl Default for Playlist {
    fn default() -> Self {
        Playlist {
            entries: Vec::new(),
            current: None,
            repeat: Repeat::Off,
            shuffle: false,
        }
    }
}

impl Playlist {
    /// Construye desde una lista de entradas; el cursor arranca en la
    /// primera (o `None` si está vacía).
    pub fn from_entries(entries: Vec<String>) -> Self {
        let current = if entries.is_empty() { None } else { Some(0) };
        Playlist {
            entries,
            current,
            repeat: Repeat::Off,
            shuffle: false,
        }
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    /// Clave de la entrada actual, si hay alguna cargada.
    pub fn current_key(&self) -> Option<&str> {
        self.current.and_then(|i| self.entries.get(i)).map(|s| s.as_str())
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn set_repeat(&mut self, r: Repeat) {
        self.repeat = r;
    }

    /// Avanza el modo repeat un paso (botón de la UI).
    pub fn cycle_repeat(&mut self) -> Repeat {
        self.repeat = self.repeat.cycle();
        self.repeat
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
    }

    pub fn toggle_shuffle(&mut self) -> bool {
        self.shuffle = !self.shuffle;
        self.shuffle
    }

    /// Mueve el cursor a `index` si es válido.
    pub fn set_current(&mut self, index: usize) {
        if index < self.entries.len() {
            self.current = Some(index);
        }
    }

    // ---------- edición ----------

    /// Agrega una entrada al final. Si la cola estaba vacía, el cursor
    /// queda en ella.
    pub fn push(&mut self, key: impl Into<String>) {
        self.entries.push(key.into());
        if self.current.is_none() {
            self.current = Some(self.entries.len() - 1);
        }
    }

    /// Inserta `key` en `index` (clampado a `[0, len]`), corriendo el
    /// cursor si la inserción cae en o antes de él.
    pub fn insert(&mut self, index: usize, key: impl Into<String>) {
        let to = index.min(self.entries.len());
        self.entries.insert(to, key.into());
        match self.current {
            Some(c) if c >= to => self.current = Some(c + 1),
            None => self.current = Some(to),
            _ => {}
        }
    }

    /// Encola `key` para que suene **justo después** de la actual
    /// ("reproducir a continuación"). Sin actual, va al final.
    pub fn enqueue_next(&mut self, key: impl Into<String>) {
        let pos = self.current.map(|c| c + 1).unwrap_or(self.entries.len());
        self.insert(pos, key);
    }

    /// Quita la entrada en `index` y devuelve su clave. Reajusta el
    /// cursor: si se borró la actual queda en la que ocupó su lugar (o
    /// `None` si la cola quedó vacía); si se borró antes, se corre.
    pub fn remove(&mut self, index: usize) -> Option<String> {
        if index >= self.entries.len() {
            return None;
        }
        let removed = self.entries.remove(index);
        let len = self.entries.len();
        self.current = match self.current {
            _ if len == 0 => None,
            Some(c) if c == index => Some(c.min(len - 1)),
            Some(c) if c > index => Some(c - 1),
            other => other,
        };
        Some(removed)
    }

    /// Reordena: saca la entrada de `from` y la reinserta de modo que
    /// quede en el índice `to` (drag-to-reorder del editor). El cursor
    /// sigue a la **misma entrada lógica**.
    pub fn move_item(&mut self, from: usize, to: usize) {
        let len = self.entries.len();
        if from >= len || len == 0 {
            return;
        }
        let to = to.min(len - 1);
        if from == to {
            return;
        }
        let item = self.entries.remove(from);
        self.entries.insert(to, item);
        if let Some(c) = self.current {
            self.current = Some(adjust_index(c, from, to));
        }
    }

    /// Vacía la cola.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.current = None;
    }

    /// Reconcilia un estado cargado de disco: clampea el cursor al rango
    /// (o `None` si la cola está vacía / el índice quedó fuera).
    pub fn sanitized(mut self) -> Playlist {
        self.current = match self.current {
            Some(c) if c < self.entries.len() => Some(c),
            _ if self.entries.is_empty() => None,
            _ => Some(0),
        };
        self
    }

    // ---------- navegación ----------

    /// Próxima entrada según [`Repeat`] (orden lineal). Mueve el cursor y
    /// devuelve su clave, o `None` si la cola terminó (`Off` en la última)
    /// — en ese caso el cursor no se mueve. Para shuffle, navegar sobre
    /// [`Playlist::shuffle_order`].
    pub fn next(&mut self) -> Option<&str> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        let c = self.current.unwrap_or(0);
        let nxt = match self.repeat {
            Repeat::One => c,
            _ if c + 1 < len => c + 1,
            Repeat::All => 0,
            Repeat::Off => return None,
        };
        self.current = Some(nxt);
        self.entries.get(nxt).map(|s| s.as_str())
    }

    /// Entrada anterior según [`Repeat`] (orden lineal); simétrica de
    /// [`Playlist::next`].
    pub fn prev(&mut self) -> Option<&str> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        let c = self.current.unwrap_or(0);
        let prv = match self.repeat {
            Repeat::One => c,
            _ if c > 0 => c - 1,
            Repeat::All => len - 1,
            Repeat::Off => return None,
        };
        self.current = Some(prv);
        self.entries.get(prv).map(|s| s.as_str())
    }

    /// Permutación de los índices `0..len` para reproducción aleatoria,
    /// derivada de `seed` (Fisher-Yates con un LCG): determinista para una
    /// misma semilla — la app pasa una derivada del reloj para variar. Si
    /// hay actual, la deja **primera** en el orden (suena lo que ya está
    /// cargado y de ahí baraja el resto).
    pub fn shuffle_order(&self, seed: u64) -> Vec<usize> {
        let len = self.entries.len();
        let mut order: Vec<usize> = (0..len).collect();
        // Fisher-Yates con LCG (constantes de Numerical Recipes).
        let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
        let mut next_rand = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for i in (1..len).rev() {
            let j = next_rand() % (i + 1);
            order.swap(i, j);
        }
        // Asegura que la actual quede primera.
        if let Some(c) = self.current {
            if let Some(pos) = order.iter().position(|&x| x == c) {
                order.swap(0, pos);
            }
        }
        order
    }
}

/// Nuevo índice de un cursor `c` tras mover el elemento de `from` a `to`
/// (semántica remove-then-insert). Ver tests para los casos límite.
fn adjust_index(c: usize, from: usize, to: usize) -> usize {
    if c == from {
        return to;
    }
    // Tras quitar `from`, los índices mayores bajan uno.
    let after_remove = if c > from { c - 1 } else { c };
    // Tras insertar en `to`, los índices ≥ to suben uno.
    if after_remove >= to {
        after_remove + 1
    } else {
        after_remove
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(items: &[&str]) -> Playlist {
        Playlist::from_entries(items.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn from_entries_arranca_en_la_primera() {
        let p = pl(&["a", "b", "c"]);
        assert_eq!(p.current_index(), Some(0));
        assert_eq!(p.current_key(), Some("a"));
        assert_eq!(Playlist::default().current_index(), None);
    }

    #[test]
    fn push_carga_cursor_si_vacia() {
        let mut p = Playlist::default();
        p.push("a");
        assert_eq!(p.current_key(), Some("a"));
        p.push("b");
        // El cursor no se mueve al agregar con cola ya cargada.
        assert_eq!(p.current_key(), Some("a"));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn insert_corre_el_cursor() {
        let mut p = pl(&["a", "b", "c"]);
        p.set_current(1); // "b"
        p.insert(0, "z"); // antes del cursor
        assert_eq!(p.entries(), &["z", "a", "b", "c"]);
        assert_eq!(p.current_key(), Some("b")); // sigue en "b"
        p.insert(99, "w"); // al final, después del cursor
        assert_eq!(p.current_key(), Some("b"));
        assert_eq!(p.entries().last().unwrap(), "w");
    }

    #[test]
    fn enqueue_next_va_tras_el_actual() {
        let mut p = pl(&["a", "b", "c"]);
        p.set_current(1); // "b"
        p.enqueue_next("x");
        assert_eq!(p.entries(), &["a", "b", "x", "c"]);
        // El cursor se queda en lo que suena.
        assert_eq!(p.current_key(), Some("b"));
    }

    #[test]
    fn remove_reajusta_cursor() {
        // Borrar antes del cursor lo corre.
        let mut p = pl(&["a", "b", "c"]);
        p.set_current(2); // "c"
        assert_eq!(p.remove(0).as_deref(), Some("a"));
        assert_eq!(p.current_key(), Some("c"));

        // Borrar la actual deja en la que ocupa su lugar.
        let mut p = pl(&["a", "b", "c"]);
        p.set_current(1); // "b"
        p.remove(1);
        assert_eq!(p.current_key(), Some("c"));

        // Borrar la última (siendo la actual) cae a la nueva última.
        let mut p = pl(&["a", "b"]);
        p.set_current(1);
        p.remove(1);
        assert_eq!(p.current_key(), Some("a"));

        // Vaciar.
        let mut p = pl(&["a"]);
        p.remove(0);
        assert_eq!(p.current_index(), None);
    }

    #[test]
    fn move_item_sigue_la_entrada_logica() {
        // Mover una entrada delante del cursor: el cursor sigue a "c".
        let mut p = pl(&["a", "b", "c", "d"]);
        p.set_current(2); // "c"
        p.move_item(0, 2); // a → posición 2
        assert_eq!(p.entries(), &["b", "c", "a", "d"]);
        assert_eq!(p.current_key(), Some("c"));

        // Mover desde después del cursor hacia el frente.
        let mut p = pl(&["a", "b", "c", "d"]);
        p.set_current(2); // "c"
        p.move_item(3, 0); // d → frente
        assert_eq!(p.entries(), &["d", "a", "b", "c"]);
        assert_eq!(p.current_key(), Some("c"));

        // Mover la propia entrada actual: el cursor va con ella.
        let mut p = pl(&["a", "b", "c", "d"]);
        p.set_current(1); // "b"
        p.move_item(1, 3);
        assert_eq!(p.entries(), &["a", "c", "d", "b"]);
        assert_eq!(p.current_key(), Some("b"));
    }

    #[test]
    fn next_prev_respetan_repeat() {
        let mut p = pl(&["a", "b", "c"]);
        assert_eq!(p.next(), Some("b"));
        assert_eq!(p.next(), Some("c"));
        // Off en la última → None, cursor no se mueve.
        assert_eq!(p.next(), None);
        assert_eq!(p.current_key(), Some("c"));

        // All envuelve.
        p.set_repeat(Repeat::All);
        assert_eq!(p.next(), Some("a"));
        p.set_current(0);
        assert_eq!(p.prev(), Some("c")); // envuelve hacia atrás

        // One se queda.
        p.set_repeat(Repeat::One);
        p.set_current(1);
        assert_eq!(p.next(), Some("b"));
        assert_eq!(p.prev(), Some("b"));
    }

    #[test]
    fn cycle_repeat_orden_vlc() {
        let mut p = Playlist::default();
        assert_eq!(p.repeat(), Repeat::Off);
        assert_eq!(p.cycle_repeat(), Repeat::All);
        assert_eq!(p.cycle_repeat(), Repeat::One);
        assert_eq!(p.cycle_repeat(), Repeat::Off);
    }

    #[test]
    fn shuffle_order_es_permutacion_determinista() {
        let p = pl(&["a", "b", "c", "d", "e"]);
        let o1 = p.shuffle_order(42);
        let o2 = p.shuffle_order(42);
        assert_eq!(o1, o2, "misma semilla → mismo orden");
        // Es una permutación válida de 0..5.
        let mut sorted = o1.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4]);
        // La actual (0) queda primera.
        assert_eq!(o1[0], 0);
        // Otra semilla suele dar otro orden (no es garantía dura, pero con
        // estas dos semillas difiere).
        assert_ne!(p.shuffle_order(7), o1);
    }

    #[test]
    fn shuffle_order_pone_la_actual_primera() {
        let mut p = pl(&["a", "b", "c", "d"]);
        p.set_current(2); // "c"
        let o = p.shuffle_order(123);
        assert_eq!(o[0], 2, "la entrada actual va primera en el shuffle");
    }

    #[test]
    fn sanitized_clampa_cursor() {
        // Un .ron con cursor fuera de rango.
        let p = Playlist {
            entries: vec!["a".into(), "b".into()],
            current: Some(9),
            repeat: Repeat::All,
            shuffle: true,
        };
        let s = p.sanitized();
        assert_eq!(s.current_index(), Some(0));

        // Cola vacía → cursor None.
        let p = Playlist {
            entries: vec![],
            current: Some(0),
            repeat: Repeat::Off,
            shuffle: false,
        };
        assert_eq!(p.sanitized().current_index(), None);
    }

    #[test]
    fn round_trip_ron() {
        let mut p = pl(&["a.mp3", "b.flac", "c.opus"]);
        p.set_current(1);
        p.set_repeat(Repeat::All);
        p.set_shuffle(true);
        let txt = ron::ser::to_string(&p).expect("serializa");
        let back: Playlist = ron::from_str(&txt).expect("deserializa");
        assert_eq!(p, back);
    }

    #[test]
    fn adjust_index_casos_limite() {
        // c == from → to
        assert_eq!(adjust_index(2, 2, 0), 0);
        // c > from, c >= to
        assert_eq!(adjust_index(2, 0, 2), 1); // remove baja a 1, insert en 2 no sube
        // d=3 con from0 to2: remove→2, insert@2→3
        assert_eq!(adjust_index(3, 0, 2), 3);
        // c < from, insert antes
        assert_eq!(adjust_index(0, 3, 0), 1);
    }
}
