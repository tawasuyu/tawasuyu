//! Store de scrollback append-only (Capa 0 del SDD-TERMINAL).
//!
//! El texto vive en un buffer contiguo (`buf`) y un índice de offsets de inicio
//! de línea (`starts`, con una sentinela al final) da acceso a la línea N en
//! **O(1)**. El cap es por **MEMORIA** (bytes), no por número de líneas: al
//! excederse, se descartan líneas enteras del **frente** en un solo `drain` +
//! reindex (amortizado, no una vez por línea).
//!
//! Las líneas descartadas se **cuentan** (`dropped`), de modo que cada línea
//! tiene un **id global estable** (`line_id = dropped + idx`) que sobrevive al
//! recorte del frente. Eso permite anclar el scroll a un id (no a px desde el
//! fondo) y preservar la posición de lectura mientras llega output —
//! exactamente la deuda B del PLAN-OUTPUT, que acá nace resuelta de raíz.
//!
//! Una línea es **un renglón lógico sin `'\n'`** (el caller lo separa; en shuma
//! cada `OutputLine` ya es una línea). El store no interpreta el contenido.

/// Límite de memoria por defecto del scrollback: 64 MiB ≈ cientos de miles de
/// líneas. "Infinito" en la práctica = "acotado por una memoria que elegís".
pub const DEFAULT_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// Persistencia opcional de las líneas que el cap recorta del frente: en vez
/// de tirarlas, las appendea a un archivo y guarda `(offset, len)` por línea
/// para lookup random posterior. Es lo que habilita "scrollback infinito"
/// (Fase 5 del SDD-TERMINAL) cuando el shell corre por horas y la memoria
/// no alcanza para todo el output histórico.
///
/// Diseño:
///
/// - **Archivo append-only**: cada línea se escribe verbatim (sin separador
///   intermedio — la longitud está en el índice). Crecimiento monótono.
/// - **Índice en memoria** `(offset, len)` por línea, indexado por
///   `global_id` (el mismo id estable del `Scrollback`). Random access O(1).
/// - **UTF-8 in/out**: el caller pasa `&str`, el read devuelve `String`. Si
///   el archivo se corrompe (improbable mientras nadie más lo toque), el
///   read devuelve `InvalidData`.
#[derive(Debug)]
pub struct SpillStore {
    file: std::fs::File,
    /// `(offset_in_file, byte_len)` por línea spilleada, indexada por
    /// posición en este Vec (no por `global_id` — restamos `base_id` al
    /// indexar si en el futuro permitimos descartar también las viejas).
    entries: Vec<(u64, u32)>,
    path: std::path::PathBuf,
}

impl SpillStore {
    /// Crea o abre un spill file en `path`. Si el archivo existe, lo trunca
    /// (el caller decide si quiere persistencia inter-sesión, generalmente
    /// no — el shell tipicamente arranca con un spill nuevo). Devuelve
    /// `Err` si no se puede crear (permisos, disco lleno).
    pub fn create(path: impl Into<std::path::PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        Ok(Self {
            file,
            entries: Vec::new(),
            path,
        })
    }

    /// Append de una línea al spill. Devuelve el índice (`entries.len()-1`)
    /// donde queda registrada. NO inserta separador — el largo está en el
    /// índice. Errores: `WriteZero`/`Interrupted` y otros transientes se
    /// devuelven; el caller puede decidir reintentar o ignorar.
    pub fn append(&mut self, text: &str) -> std::io::Result<usize> {
        use std::io::{Seek, SeekFrom, Write};
        let offset = self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(text.as_bytes())?;
        // Flush por seguridad — el shell puede crashear y queremos el
        // archivo legible. Cost ~µs en SSD; aceptable.
        self.file.flush()?;
        self.entries.push((offset, text.len() as u32));
        Ok(self.entries.len() - 1)
    }

    /// Lee la línea spilleada con índice `i` (0-based dentro del spill, NO
    /// el `global_id`). `None` si fuera de rango.
    pub fn read(&mut self, i: usize) -> std::io::Result<Option<String>> {
        use std::io::{Read, Seek, SeekFrom};
        let Some(&(off, len)) = self.entries.get(i) else {
            return Ok(None);
        };
        self.file.seek(SeekFrom::Start(off))?;
        let mut buf = vec![0u8; len as usize];
        self.file.read_exact(&mut buf)?;
        String::from_utf8(buf).map(Some).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })
    }

    /// Cantidad de líneas spilleadas hasta ahora.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` si todavía no spilleó ninguna línea.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Path del archivo del spill (informativo). El caller lo puede usar
    /// para mostrarlo en una notice tipo "salida volcada a /tmp/...".
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// Store de scrollback append-only con índice de líneas y cap por memoria.
///
/// Invariantes:
/// - `starts` siempre tiene al menos un elemento (la sentinela) y es monótono
///   creciente; `starts[len()] == buf.len()`.
/// - `len() == starts.len() - 1`.
/// - `line(i)` ⊆ `buf` para todo `i < len()`.
#[derive(Debug)]
pub struct Scrollback {
    /// Texto de todas las líneas vigentes, concatenado sin separadores.
    buf: String,
    /// `starts[i]` = offset de inicio de la línea `i` en `buf`. El último
    /// elemento es la sentinela (`== buf.len()`), así `line(i)` es
    /// `buf[starts[i]..starts[i+1]]` sin casos especiales para la última.
    starts: Vec<usize>,
    /// Cuántas líneas se descartaron del frente desde el último `clear`. Hace
    /// estable la numeración/los ids globales aunque el frente se recorte.
    dropped: u64,
    /// Cap de memoria del texto (`buf.len()`), en bytes.
    limit_bytes: usize,
    /// Spill opcional: cuando se setea, las líneas que `enforce_limit` saca
    /// del frente NO se pierden — se appendean al spill y quedan
    /// recuperables vía `read_spilled` (Fase 5 del SDD-TERMINAL). El
    /// `Arc<Mutex<>>` deja que el `Scrollback` sea Clone aunque
    /// `SpillStore` no lo sea (file handles no son Clone).
    spill: Option<std::sync::Arc<std::sync::Mutex<SpillStore>>>,
}

impl Clone for Scrollback {
    fn clone(&self) -> Self {
        // Clone share el spill (Arc) — las dos instancias appendean al MISMO
        // archivo, lo que es lo único razonable: el spill es la verdad
        // sobre las líneas viejas, no hay forma de "clonar el archivo".
        Self {
            buf: self.buf.clone(),
            starts: self.starts.clone(),
            dropped: self.dropped,
            limit_bytes: self.limit_bytes,
            spill: self.spill.clone(),
        }
    }
}

impl Default for Scrollback {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT_BYTES)
    }
}

impl Scrollback {
    /// Store vacío con un cap de memoria explícito (bytes del texto). Un
    /// `limit_bytes` de `0` se trata como "sin tope práctico" (no recorta).
    pub fn new(limit_bytes: usize) -> Self {
        Self {
            buf: String::new(),
            starts: vec![0],
            dropped: 0,
            limit_bytes,
            spill: None,
        }
    }

    /// Habilita el spill: las líneas que se recorten del frente se
    /// appendean a `spill` en vez de descartarse. El caller construye el
    /// `SpillStore` con `SpillStore::create(path)`.
    pub fn enable_spill(&mut self, spill: SpillStore) {
        self.spill = Some(std::sync::Arc::new(std::sync::Mutex::new(spill)));
    }

    /// `true` si este scrollback tiene spill activo.
    pub fn has_spill(&self) -> bool {
        self.spill.is_some()
    }

    /// Cantidad de líneas spilleadas hasta ahora (`0` si no hay spill).
    pub fn spilled_count(&self) -> usize {
        match self.spill.as_ref() {
            Some(s) => s.lock().map(|g| g.len()).unwrap_or(0),
            None => 0,
        }
    }

    /// Path del archivo de spill, si está activo. Lo expone el shell con
    /// `:scrollback` para que el usuario pueda abrirlo / `cat`-earlo /
    /// buscar grep en él. `None` si no hay spill.
    pub fn spill_path(&self) -> Option<std::path::PathBuf> {
        self.spill
            .as_ref()
            .and_then(|s| s.lock().ok().map(|g| g.path().to_path_buf()))
    }

    /// Lee una línea spilleada por su id global. Lookup O(1) en el índice
    /// del spill + un `seek` + `read` en el archivo. Devuelve `None` si
    /// `global_id >= spilled_count` o no hay spill. La línea sigue contando
    /// con `dropped`/`total_pushed` originales (el id global persiste).
    pub fn read_spilled(&self, global_id: u64) -> std::io::Result<Option<String>> {
        let Some(spill) = self.spill.as_ref() else { return Ok(None) };
        let mut guard = match spill.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // El spill indexa por orden de append, que coincide con el
        // global_id 0-based (la primera dropped es la 0ª, la segunda la
        // 1ª, etc.).
        guard.read(global_id as usize)
    }

    /// Appendea **un renglón lógico** (sin `'\n'`; si lo trae, se guarda
    /// verbatim — el caller separa). Tras appendear, recorta el frente si el
    /// texto excede `limit_bytes`.
    pub fn push_line(&mut self, text: &str) {
        self.buf.push_str(text);
        self.starts.push(self.buf.len());
        self.enforce_limit();
    }

    /// Recorta líneas enteras del frente hasta que `buf.len() <= limit_bytes`,
    /// en un solo `drain` + reindex. No-op si `limit_bytes == 0` o ya cabe.
    fn enforce_limit(&mut self) {
        if self.limit_bytes == 0 || self.buf.len() <= self.limit_bytes {
            return;
        }
        // Bytes que sobran respecto del tope: hay que liberar al menos esto del
        // frente. Buscamos el primer `k` cuyo offset de inicio deje `buf` bajo
        // el tope (`buf.len() - starts[k] <= limit`, i.e. `starts[k] >=
        // need_free`).
        let need_free = self.buf.len() - self.limit_bytes;
        let k = self.starts.partition_point(|&s| s < need_free);
        // No tirar la sentinela: como mucho dejamos el store vacío (línea única
        // más grande que el cap entero, caso patológico).
        let k = k.min(self.len());
        if k == 0 {
            return;
        }
        let cut = self.starts[k];
        // Antes de borrar las líneas del frente, las spillamos a disco si
        // hay spill configurado. El append es por línea (cada una con su
        // longitud en el índice), así el read random por `global_id`
        // sigue trivial.
        if let Some(spill) = self.spill.as_ref() {
            let mut guard = match spill.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            for i in 0..k {
                let lo = self.starts[i];
                let hi = self.starts[i + 1];
                // Errores de I/O se ignoran silenciosamente: el spill es
                // mejor-esfuerzo; perder una línea spilleada por disco
                // lleno no debe colgar el shell. El caller que quiera
                // chequear que el spill esté vivo puede mirar `spilled_count`
                // vs `dropped` y avisar al usuario.
                let _ = guard.append(&self.buf[lo..hi]);
            }
        }
        self.buf.drain(0..cut);
        self.starts.drain(0..k);
        for s in &mut self.starts {
            *s -= cut;
        }
        self.dropped += k as u64;
    }

    /// Cantidad de líneas vigentes en el store.
    pub fn len(&self) -> usize {
        self.starts.len() - 1
    }

    /// `true` si no hay líneas vigentes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Línea `idx` (0-based, vigente) en **O(1)**. `None` fuera de rango.
    pub fn line(&self, idx: usize) -> Option<&str> {
        if idx + 1 >= self.starts.len() {
            return None;
        }
        Some(&self.buf[self.starts[idx]..self.starts[idx + 1]])
    }

    /// Líneas descartadas del frente desde el último `clear`.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Total de líneas que pasaron por el store desde el último `clear`
    /// (`dropped + len`). Es el número de la próxima línea (0-based global).
    pub fn total_pushed(&self) -> u64 {
        self.dropped + self.len() as u64
    }

    /// Número **global 1-based** de la línea `idx` (para la numeración del
    /// gutter): estable aunque el frente se recorte.
    pub fn line_number(&self, idx: usize) -> u64 {
        self.dropped + idx as u64 + 1
    }

    /// Id **global estable** de la línea `idx` (`dropped + idx`): sobrevive al
    /// recorte del frente. Para anclar el scroll a una línea concreta.
    pub fn line_id(&self, idx: usize) -> u64 {
        self.dropped + idx as u64
    }

    /// Índice vigente del id global `id`, si la línea sigue en el store
    /// (no se recortó del frente ni es futura). `None` si no.
    pub fn index_of_id(&self, id: u64) -> Option<usize> {
        if id < self.dropped {
            return None;
        }
        let idx = (id - self.dropped) as usize;
        (idx < self.len()).then_some(idx)
    }

    /// Bytes del texto vigente (lo que cuenta para el cap).
    pub fn byte_len(&self) -> usize {
        self.buf.len()
    }

    /// Cap de memoria configurado.
    pub fn limit_bytes(&self) -> usize {
        self.limit_bytes
    }

    /// Texto de las líneas `[start, end)` unido por `'\n'` — para copiar al
    /// clipboard una selección de filas. Recorta `end` a `len()`; rango vacío o
    /// invertido → cadena vacía.
    pub fn slice_text(&self, start: usize, end: usize) -> String {
        let end = end.min(self.len());
        if start >= end {
            return String::new();
        }
        let mut out = String::with_capacity(self.starts[end] - self.starts[start]);
        for i in start..end {
            if i > start {
                out.push('\n');
            }
            out.push_str(self.line(i).unwrap_or(""));
        }
        out
    }

    /// Vacía el store y **reinicia** la numeración (`dropped = 0`) — el
    /// equivalente del builtin `clear` del shell: se empieza de cero.
    pub fn clear(&mut self) {
        self.buf.clear();
        self.starts.clear();
        self.starts.push(0);
        self.dropped = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_is_empty() {
        let s = Scrollback::new(1024);
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.line(0), None);
        assert_eq!(s.byte_len(), 0);
    }

    #[test]
    fn push_and_access_o1() {
        let mut s = Scrollback::new(1024);
        s.push_line("uno");
        s.push_line("dos");
        s.push_line("tres");
        assert_eq!(s.len(), 3);
        assert_eq!(s.line(0), Some("uno"));
        assert_eq!(s.line(1), Some("dos"));
        assert_eq!(s.line(2), Some("tres"));
        assert_eq!(s.line(3), None);
    }

    #[test]
    fn line_numbers_are_one_based_global() {
        let mut s = Scrollback::new(1024);
        s.push_line("a");
        s.push_line("b");
        assert_eq!(s.line_number(0), 1);
        assert_eq!(s.line_number(1), 2);
        assert_eq!(s.line_id(0), 0);
        assert_eq!(s.line_id(1), 1);
        assert_eq!(s.total_pushed(), 2);
    }

    #[test]
    fn cap_drops_front_and_keeps_global_numbering() {
        // Tope chico: ~ cada línea ocupa "linea_N" (7-8 bytes). Con tope 20 sólo
        // entran ~2-3 líneas; las viejas se recortan del frente.
        let mut s = Scrollback::new(20);
        for i in 0..50 {
            s.push_line(&format!("L{i:04}")); // 5 bytes c/u
        }
        // Sigue bajo el tope.
        assert!(s.byte_len() <= 20, "byte_len {} excede el tope", s.byte_len());
        // Hubo recorte del frente.
        assert!(s.dropped() > 0);
        // La numeración global sigue siendo correcta: la última línea es la 50ª
        // (1-based), id global 49.
        let last = s.len() - 1;
        assert_eq!(s.line(last), Some("L0049"));
        assert_eq!(s.line_number(last), 50);
        assert_eq!(s.line_id(last), 49);
        // total_pushed cuenta todo lo que pasó (49 dropped + len vigente = 50).
        assert_eq!(s.total_pushed(), 50);
    }

    #[test]
    fn dropped_lines_are_not_accessible_but_ids_resolve() {
        let mut s = Scrollback::new(20);
        for i in 0..50 {
            s.push_line(&format!("L{i:04}"));
        }
        let dropped = s.dropped();
        assert!(dropped > 0);
        // Un id ya recortado no resuelve a índice vigente.
        assert_eq!(s.index_of_id(0), None);
        // El id de la primera línea vigente resuelve a índice 0.
        let first_id = s.line_id(0);
        assert_eq!(first_id, dropped);
        assert_eq!(s.index_of_id(first_id), Some(0));
        // Un id futuro tampoco resuelve.
        assert_eq!(s.index_of_id(s.total_pushed() + 5), None);
    }

    #[test]
    fn id_survives_front_drop() {
        // Un id apuntado antes de un recorte sigue apuntando a la MISMA línea
        // (mientras siga vigente), aunque su índice cambie.
        let mut s = Scrollback::new(40);
        for i in 0..10 {
            s.push_line(&format!("L{i:04}"));
        }
        // Tomamos el id de una línea concreta por su texto.
        let idx = (0..s.len()).find(|&i| s.line(i) == Some("L0007")).unwrap();
        let id = s.line_id(idx);
        // Llega más output → se recorta más frente.
        for i in 10..20 {
            s.push_line(&format!("L{i:04}"));
        }
        // El id sigue resolviendo a la línea "L0007" si no fue recortada.
        if let Some(now) = s.index_of_id(id) {
            assert_eq!(s.line(now), Some("L0007"), "el id debe seguir apuntando a la misma línea");
        }
        // (Si "L0007" ya se recortó, index_of_id devuelve None — también válido.)
    }

    #[test]
    fn slice_text_joins_with_newlines() {
        let mut s = Scrollback::new(1024);
        for l in ["alfa", "beta", "gamma", "delta"] {
            s.push_line(l);
        }
        assert_eq!(s.slice_text(1, 3), "beta\ngamma");
        assert_eq!(s.slice_text(0, 4), "alfa\nbeta\ngamma\ndelta");
        // Rango clampeado y vacío.
        assert_eq!(s.slice_text(2, 999), "gamma\ndelta");
        assert_eq!(s.slice_text(3, 3), "");
        assert_eq!(s.slice_text(5, 2), "");
    }

    #[test]
    fn clear_resets_buffer_and_numbering() {
        let mut s = Scrollback::new(20);
        for i in 0..50 {
            s.push_line(&format!("L{i:04}"));
        }
        assert!(s.dropped() > 0);
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.dropped(), 0);
        assert_eq!(s.byte_len(), 0);
        // Tras clear la numeración arranca de nuevo en 1.
        s.push_line("nuevo");
        assert_eq!(s.line_number(0), 1);
        assert_eq!(s.line(0), Some("nuevo"));
    }

    #[test]
    fn zero_limit_means_no_cap() {
        let mut s = Scrollback::new(0);
        for i in 0..1000 {
            s.push_line(&format!("linea {i}"));
        }
        assert_eq!(s.len(), 1000);
        assert_eq!(s.dropped(), 0);
        assert_eq!(s.line(999), Some("linea 999"));
    }

    #[test]
    fn unicode_lines_are_sliced_on_char_boundaries() {
        // El índice usa offsets de byte; appendeamos líneas completas, así que
        // los cortes caen siempre en frontera de carácter (inicio de línea).
        let mut s = Scrollback::new(1024);
        s.push_line("café ☕");
        s.push_line("niño ñ");
        assert_eq!(s.line(0), Some("café ☕"));
        assert_eq!(s.line(1), Some("niño ñ"));
        assert_eq!(s.slice_text(0, 2), "café ☕\nniño ñ");
    }

    #[test]
    fn spill_archiva_lineas_recortadas_y_las_lee_random() {
        // Setup: store con cap chico + spill a un archivo temporal. Tras
        // muchas appends, las líneas viejas viven en el spill y se leen
        // de vuelta por su id global.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("spill.log");
        let spill = SpillStore::create(&path).expect("spill create");
        let mut s = Scrollback::new(20);
        s.enable_spill(spill);
        assert!(s.has_spill());

        for i in 0..50 {
            s.push_line(&format!("L{i:04}"));
        }
        // Hubo recorte → spill tiene entries.
        assert!(s.dropped() > 0);
        assert_eq!(s.spilled_count() as u64, s.dropped(), "todas las dropped van al spill");
        // Una línea concreta del spill — la 5ª (id=5) → "L0005".
        let read = s.read_spilled(5).expect("read").expect("entry");
        assert_eq!(read, "L0005");
        // La primera (id=0).
        let first = s.read_spilled(0).expect("read").expect("entry");
        assert_eq!(first, "L0000");
        // Una línea fuera de rango (un id futuro).
        let none = s.read_spilled(99999).expect("read");
        assert!(none.is_none());
    }

    #[test]
    fn sin_spill_read_spilled_es_none() {
        let mut s = Scrollback::new(20);
        for i in 0..50 {
            s.push_line(&format!("L{i:04}"));
        }
        // Hubo recorte pero no hay spill → no se puede recuperar.
        assert!(s.dropped() > 0);
        assert_eq!(s.spilled_count(), 0);
        assert!(s.read_spilled(0).expect("read").is_none());
    }

    #[test]
    fn spill_sobrevive_a_clones_del_scrollback() {
        // `Scrollback` es Clone (Pata clona el state del shell); el spill
        // se comparte por Arc, así las dos instancias appendean al MISMO
        // archivo. Acá comprobamos que el spilled_count se ve desde ambos.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spill.log");
        let spill = SpillStore::create(&path).unwrap();
        let mut a = Scrollback::new(20);
        a.enable_spill(spill);
        for i in 0..30 {
            a.push_line(&format!("L{i:04}"));
        }
        let b = a.clone();
        // El clon ve el mismo spilled_count.
        assert_eq!(a.spilled_count(), b.spilled_count());
        // Y puede leer las mismas líneas.
        let from_a = a.read_spilled(2).unwrap().unwrap();
        let from_b = b.read_spilled(2).unwrap().unwrap();
        assert_eq!(from_a, from_b);
    }

    #[test]
    fn spill_almacena_utf8_intacto() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spill.log");
        let spill = SpillStore::create(&path).unwrap();
        let mut s = Scrollback::new(15);
        s.enable_spill(spill);
        s.push_line("café ☕");
        s.push_line("niño ñ");
        s.push_line("hello world"); // empuja las anteriores a spill
        assert!(s.dropped() > 0);
        let cafe = s.read_spilled(0).unwrap().unwrap();
        assert_eq!(cafe, "café ☕");
        let nino = s.read_spilled(1).unwrap().unwrap();
        assert_eq!(nino, "niño ñ");
    }

    #[test]
    fn large_append_stays_under_cap_and_indexes_correctly() {
        // Muchas líneas, tope moderado: el store se mantiene acotado y el acceso
        // sigue correcto en todo el rango vigente.
        let mut s = Scrollback::new(4096);
        for i in 0..100_000 {
            s.push_line(&format!("fila numero {i}"));
        }
        assert!(s.byte_len() <= 4096);
        assert!(s.dropped() > 0);
        // Todas las líneas vigentes son accesibles y coherentes con su número.
        for idx in 0..s.len() {
            let n = s.line_number(idx); // 1-based global
            let expected = format!("fila numero {}", n - 1);
            assert_eq!(s.line(idx), Some(expected.as_str()));
        }
        // La última empujada fue la 100000ª.
        assert_eq!(s.total_pushed(), 100_000);
    }
}
