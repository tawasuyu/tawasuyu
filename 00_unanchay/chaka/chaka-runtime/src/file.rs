//! `CobFile` — un fichero secuencial de líneas para el runtime COBOL.

use std::collections::VecDeque;

/// Un fichero de organización «line sequential»: cada registro es una
/// línea de texto. La lectura carga el fichero entero a memoria; la
/// escritura acumula líneas y las vuelca al cerrar.
#[derive(Debug)]
pub struct CobFile {
    path: String,
    state: State,
}

#[derive(Debug)]
enum State {
    Closed,
    /// Abierto para lectura: las líneas que faltan por leer.
    Reading(VecDeque<String>),
    /// Abierto para escritura: las líneas acumuladas.
    Writing(Vec<String>),
}

impl CobFile {
    /// Un fichero nuevo, cerrado, asignado a la ruta `path`.
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            state: State::Closed,
        }
    }

    /// `OPEN INPUT`: carga el fichero a memoria. Si no existe, queda
    /// abierto y vacío (la primera lectura dará fin de fichero).
    pub fn open_input(&mut self) {
        let lines = std::fs::read_to_string(&self.path)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default();
        self.state = State::Reading(lines);
    }

    /// `OPEN OUTPUT`: empieza un fichero nuevo y vacío.
    pub fn open_output(&mut self) {
        self.state = State::Writing(Vec::new());
    }

    /// `READ`: la siguiente línea, o `None` en fin de fichero.
    pub fn read(&mut self) -> Option<String> {
        match &mut self.state {
            State::Reading(lines) => lines.pop_front(),
            _ => None,
        }
    }

    /// `WRITE`: agrega una línea (sólo si está abierto para escritura).
    pub fn write(&mut self, line: &str) {
        if let State::Writing(buf) = &mut self.state {
            buf.push(line.to_string());
        }
    }

    /// `CLOSE`: si estaba escribiendo, vuelca el contenido al disco.
    pub fn close(&mut self) {
        if let State::Writing(buf) = &self.state {
            let body: String = buf.iter().map(|l| format!("{l}\n")).collect();
            let _ = std::fs::write(&self.path, body);
        }
        self.state = State::Closed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let path = std::env::temp_dir().join("charka-cobfile-test.dat");
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
        let mut f = CobFile::new("/charka/no/existe/jamas.dat");
        f.open_input();
        assert_eq!(f.read(), None);
    }
}
