//! `CobFile` — un fichero COBOL en tiempo de ejecución.
//!
//! Soporta dos familias de organización:
//!
//! - **`LINE SEQUENTIAL`** (el default): cada registro es una línea de
//!   texto. La lectura carga el fichero entero a memoria; la escritura
//!   acumula líneas y las vuelca al cerrar.
//! - **`INDEXED` / `RELATIVE`**: un almacén por clave (`BTreeMap`) que
//!   guarda cada registro indexado por su clave (la `RECORD KEY` para
//!   indexado, el número de registro para relativo). Soporta acceso
//!   directo (`READ`/`REWRITE`/`DELETE` por clave), posicionamiento
//!   (`START`) y lectura secuencial en orden de clave (`READ NEXT`).
//!   En disco se persiste como líneas `clave<TAB>registro`, en orden de
//!   clave, para que un `OPEN` posterior reconstruya el índice.

use std::collections::{BTreeMap, VecDeque};

/// La organización física de un fichero, tal como la declara el
/// `SELECT`. Determina qué familia de operaciones admite el `CobFile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Organization {
    /// Registros = líneas de texto; acceso secuencial.
    LineSequential,
    /// Registros indexados por una `RECORD KEY`.
    Indexed,
    /// Registros numerados por una `RELATIVE KEY`.
    Relative,
}

/// El operador de un `START`: posiciona el cursor en el primer registro
/// cuya clave satisface `clave-del-registro OP clave-buscada`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartCmp {
    Eq,
    Gt,
    Ge,
    Lt,
    Le,
}

/// Un fichero COBOL. La organización se fija al construirlo y no cambia.
#[derive(Debug)]
pub struct CobFile {
    path: String,
    org: Organization,
    state: State,
}

#[derive(Debug)]
enum State {
    Closed,
    /// Abierto para lectura secuencial: las líneas que faltan por leer.
    Reading(VecDeque<String>),
    /// Abierto para escritura secuencial: las líneas acumuladas.
    Writing(Vec<String>),
    /// Abierto sobre un almacén por clave (indexado/relativo).
    Keyed(Keyed),
}

/// El estado de un fichero por clave mientras está abierto.
#[derive(Debug)]
struct Keyed {
    /// El índice: clave → registro, ordenado por clave.
    map: BTreeMap<String, String>,
    /// La posición para la próxima lectura secuencial (`READ NEXT`).
    cursor: Cursor,
    /// Si el modo de apertura permite mutar (`OUTPUT`/`I-O`/`EXTEND`).
    writable: bool,
}

/// Dónde lee la próxima lectura secuencial de un fichero por clave.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cursor {
    /// Desde el principio (la clave mínima).
    Start,
    /// La próxima lectura devuelve el primer registro con clave ≥ esta.
    At(String),
    /// Agotado: no quedan registros por leer.
    End,
}

impl CobFile {
    /// Un fichero `LINE SEQUENTIAL` nuevo, cerrado, asignado a `path`.
    pub fn new(path: &str) -> Self {
        Self::with_org(path, Organization::LineSequential)
    }

    /// Un fichero nuevo con una organización explícita.
    pub fn with_org(path: &str, org: Organization) -> Self {
        Self {
            path: path.to_string(),
            org,
            state: State::Closed,
        }
    }

    /// `OPEN INPUT`: carga el fichero a memoria. Si no existe, queda
    /// abierto y vacío (la primera lectura dará fin de fichero).
    pub fn open_input(&mut self) {
        self.state = match self.org {
            Organization::LineSequential => State::Reading(self.load_lines()),
            _ => State::Keyed(Keyed {
                map: self.load_keyed(),
                cursor: Cursor::Start,
                writable: false,
            }),
        };
    }

    /// `OPEN OUTPUT`: empieza un fichero nuevo y vacío.
    pub fn open_output(&mut self) {
        self.state = match self.org {
            Organization::LineSequential => State::Writing(Vec::new()),
            _ => State::Keyed(Keyed {
                map: BTreeMap::new(),
                cursor: Cursor::Start,
                writable: true,
            }),
        };
    }

    /// `OPEN I-O`: abre el fichero existente para lectura **y** escritura
    /// (lo que requieren `REWRITE`/`DELETE`/`START`). En line-sequential,
    /// que no soporta acceso mixto, se comporta como `OPEN INPUT`.
    pub fn open_io(&mut self) {
        self.state = match self.org {
            Organization::LineSequential => State::Reading(self.load_lines()),
            _ => State::Keyed(Keyed {
                map: self.load_keyed(),
                cursor: Cursor::Start,
                writable: true,
            }),
        };
    }

    /// `OPEN EXTEND`: abre para añadir al final. En un fichero por clave
    /// equivale a `I-O` (las claves se insertan donde corresponda); en
    /// line-sequential, carga lo existente para que los `WRITE` añadan.
    pub fn open_extend(&mut self) {
        self.state = match self.org {
            Organization::LineSequential => {
                State::Writing(self.load_lines().into_iter().collect())
            }
            _ => State::Keyed(Keyed {
                map: self.load_keyed(),
                cursor: Cursor::Start,
                writable: true,
            }),
        };
    }

    /// `READ` secuencial: la siguiente línea (line-sequential) o el
    /// siguiente registro en orden de clave (keyed), o `None` en fin de
    /// fichero. Para keyed equivale a `READ NEXT`.
    pub fn read(&mut self) -> Option<String> {
        match &mut self.state {
            State::Reading(lines) => lines.pop_front(),
            State::Keyed(k) => k.read_next(),
            _ => None,
        }
    }

    /// `WRITE` secuencial: agrega una línea (sólo line-sequential abierto
    /// para escritura).
    pub fn write(&mut self, line: &str) {
        if let State::Writing(buf) = &mut self.state {
            buf.push(line.to_string());
        }
    }

    /// `WRITE` por clave: inserta `record` bajo `key`. Devuelve `false`
    /// si la clave ya existe (la condición `INVALID KEY` de COBOL para un
    /// registro duplicado).
    pub fn write_keyed(&mut self, key: &str, record: &str) -> bool {
        match &mut self.state {
            State::Keyed(k) if k.writable => {
                if k.map.contains_key(key) {
                    false
                } else {
                    k.map.insert(key.to_string(), record.to_string());
                    true
                }
            }
            _ => false,
        }
    }

    /// `READ` por clave (acceso directo): devuelve el registro de `key`,
    /// o `None` si no existe (`INVALID KEY`). Deja el cursor posicionado
    /// tras la clave leída, para un `READ NEXT` posterior.
    pub fn read_keyed(&mut self, key: &str) -> Option<String> {
        match &mut self.state {
            State::Keyed(k) => {
                let rec = k.map.get(key).cloned();
                if rec.is_some() {
                    k.cursor = k.after(key);
                }
                rec
            }
            _ => None,
        }
    }

    /// `REWRITE` por clave: reemplaza el registro de `key`. Devuelve
    /// `false` si la clave no existe (`INVALID KEY`).
    pub fn rewrite_keyed(&mut self, key: &str, record: &str) -> bool {
        match &mut self.state {
            State::Keyed(k) if k.writable && k.map.contains_key(key) => {
                k.map.insert(key.to_string(), record.to_string());
                true
            }
            _ => false,
        }
    }

    /// `DELETE` por clave: elimina el registro de `key`. Devuelve `false`
    /// si la clave no existe (`INVALID KEY`).
    pub fn delete_keyed(&mut self, key: &str) -> bool {
        match &mut self.state {
            State::Keyed(k) if k.writable => k.map.remove(key).is_some(),
            _ => false,
        }
    }

    /// `START`: posiciona el cursor en el primer registro cuya clave
    /// satisface `clave OP key`. Devuelve `false` si no hay ninguno
    /// (`INVALID KEY`); en ese caso el cursor queda agotado.
    pub fn start(&mut self, key: &str, cmp: StartCmp) -> bool {
        let State::Keyed(k) = &mut self.state else {
            return false;
        };
        let target = match cmp {
            StartCmp::Ge => k.map.range(key.to_string()..).next().map(|(k, _)| k.clone()),
            StartCmp::Gt => k
                .map
                .range(exclusive(key)..)
                .next()
                .map(|(k, _)| k.clone()),
            StartCmp::Eq => k.map.get_key_value(key).map(|(k, _)| k.clone()),
            StartCmp::Le => k
                .map
                .range(..=key.to_string())
                .next_back()
                .map(|(k, _)| k.clone()),
            StartCmp::Lt => k
                .map
                .range(..key.to_string())
                .next_back()
                .map(|(k, _)| k.clone()),
        };
        match target {
            Some(t) => {
                k.cursor = Cursor::At(t);
                true
            }
            None => {
                k.cursor = Cursor::End;
                false
            }
        }
    }

    /// `CLOSE`: vuelca el contenido al disco (escritura secuencial o
    /// almacén por clave) y deja el fichero cerrado.
    pub fn close(&mut self) {
        match &self.state {
            State::Writing(buf) => {
                let body: String = buf.iter().map(|l| format!("{l}\n")).collect();
                let _ = std::fs::write(&self.path, body);
            }
            State::Keyed(k) if k.writable => {
                let body: String = k
                    .map
                    .iter()
                    .map(|(key, rec)| format!("{key}\t{rec}\n"))
                    .collect();
                let _ = std::fs::write(&self.path, body);
            }
            _ => {}
        }
        self.state = State::Closed;
    }

    /// Carga el fichero como líneas de texto (line-sequential).
    fn load_lines(&self) -> VecDeque<String> {
        std::fs::read_to_string(&self.path)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Reconstruye el índice de un fichero por clave desde el disco. Cada
    /// línea es `clave<TAB>registro`; las líneas malformadas se omiten.
    fn load_keyed(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(&self.path)
            .map(|s| {
                s.lines()
                    .filter_map(|l| l.split_once('\t'))
                    .map(|(k, r)| (k.to_string(), r.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Keyed {
    /// La siguiente lectura secuencial según el cursor, avanzándolo.
    fn read_next(&mut self) -> Option<String> {
        let found = match &self.cursor {
            Cursor::End => None,
            Cursor::Start => self.map.iter().next().map(|(k, r)| (k.clone(), r.clone())),
            Cursor::At(key) => self
                .map
                .range(key.clone()..)
                .next()
                .map(|(k, r)| (k.clone(), r.clone())),
        };
        match found {
            Some((k, r)) => {
                self.cursor = self.after(&k);
                Some(r)
            }
            None => {
                self.cursor = Cursor::End;
                None
            }
        }
    }

    /// El cursor que apunta al primer registro con clave estrictamente
    /// mayor que `key`, o `End` si `key` es la última.
    fn after(&self, key: &str) -> Cursor {
        match self.map.range(exclusive(key)..).next() {
            Some((next, _)) => Cursor::At(next.clone()),
            None => Cursor::End,
        }
    }
}

/// La menor cadena estrictamente mayor que `key` con el mismo prefijo
/// (`key` + un carácter nulo), para construir un rango `(key, ..)`
/// excluyente sobre claves `String`.
fn exclusive(key: &str) -> String {
    let mut s = key.to_string();
    s.push('\0');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let path = std::env::temp_dir().join("chaka-cobfile-test.dat");
        let path = path.to_str().unwrap();

        let mut f = CobFile::new(path);
        f.open_output();
        f.write("PRIMERA");
        f.write("SEGUNDA");
        f.close();

        let mut g = CobFile::new(path);
        g.open_input();
        assert_eq!(g.read().as_deref(), Some("PRIMERA"));
        assert_eq!(g.read().as_deref(), Some("SEGUNDA"));
        assert_eq!(g.read(), None); // fin de fichero
        g.close();

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let mut f = CobFile::new("/chaka/no/existe/jamas.dat");
        f.open_input();
        assert_eq!(f.read(), None);
    }

    #[test]
    fn indexed_write_read_by_key() {
        let path = std::env::temp_dir().join("chaka-cobfile-idx.dat");
        let path = path.to_str().unwrap();

        let mut f = CobFile::with_org(path, Organization::Indexed);
        f.open_output();
        assert!(f.write_keyed("B002", "B002 BETA"));
        assert!(f.write_keyed("A001", "A001 ALFA"));
        // Clave duplicada → INVALID KEY.
        assert!(!f.write_keyed("A001", "A001 OTRO"));
        f.close();

        let mut g = CobFile::with_org(path, Organization::Indexed);
        g.open_io();
        // Acceso directo.
        assert_eq!(g.read_keyed("A001").as_deref(), Some("A001 ALFA"));
        assert_eq!(g.read_keyed("ZZZZ"), None); // INVALID KEY
        g.close();

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn indexed_read_next_is_in_key_order() {
        let path = std::env::temp_dir().join("chaka-cobfile-idx-seq.dat");
        let path = path.to_str().unwrap();

        let mut f = CobFile::with_org(path, Organization::Indexed);
        f.open_output();
        f.write_keyed("C", "tres");
        f.write_keyed("A", "uno");
        f.write_keyed("B", "dos");
        f.close();

        let mut g = CobFile::with_org(path, Organization::Indexed);
        g.open_input();
        // READ NEXT recorre en orden de clave, no de inserción.
        assert_eq!(g.read().as_deref(), Some("uno"));
        assert_eq!(g.read().as_deref(), Some("dos"));
        assert_eq!(g.read().as_deref(), Some("tres"));
        assert_eq!(g.read(), None);
        g.close();

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rewrite_and_delete() {
        let path = std::env::temp_dir().join("chaka-cobfile-rw.dat");
        let path = path.to_str().unwrap();

        let mut f = CobFile::with_org(path, Organization::Indexed);
        f.open_output();
        f.write_keyed("K1", "viejo");
        f.write_keyed("K2", "otro");
        f.close();

        let mut g = CobFile::with_org(path, Organization::Indexed);
        g.open_io();
        assert!(g.rewrite_keyed("K1", "nuevo"));
        assert!(!g.rewrite_keyed("NOPE", "x")); // INVALID KEY
        assert!(g.delete_keyed("K2"));
        assert!(!g.delete_keyed("K2")); // ya borrado → INVALID KEY
        assert_eq!(g.read_keyed("K1").as_deref(), Some("nuevo"));
        assert_eq!(g.read_keyed("K2"), None);
        g.close();

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn start_positions_for_sequential_read() {
        let path = std::env::temp_dir().join("chaka-cobfile-start.dat");
        let path = path.to_str().unwrap();

        let mut f = CobFile::with_org(path, Organization::Indexed);
        f.open_output();
        for k in ["10", "20", "30", "40"] {
            f.write_keyed(k, &format!("rec-{k}"));
        }
        f.close();

        let mut g = CobFile::with_org(path, Organization::Indexed);
        g.open_io();
        // START KEY >= 25 → posiciona en 30.
        assert!(g.start("25", StartCmp::Ge));
        assert_eq!(g.read().as_deref(), Some("rec-30"));
        assert_eq!(g.read().as_deref(), Some("rec-40"));
        assert_eq!(g.read(), None);
        // START > 40 → no hay → INVALID KEY.
        assert!(!g.start("40", StartCmp::Gt));
        // START = 20 → exacto.
        assert!(g.start("20", StartCmp::Eq));
        assert_eq!(g.read().as_deref(), Some("rec-20"));
        g.close();

        let _ = std::fs::remove_file(path);
    }
}
