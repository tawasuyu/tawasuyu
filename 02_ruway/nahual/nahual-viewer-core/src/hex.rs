//! `hex` — núcleo agnóstico del visor de hex de nahual (parseo + tipos de preview). El render vive en `nahual-hex-viewer-llimphi`.

use std::path::Path;

/// Bytes que se vuelcan por defecto (4 KiB = 256 filas). El caller puede
/// pedir más; pasado cierto punto un dump deja de ser legible a ojo.
pub const DEFAULT_HEX_BYTES_MAX: usize = 4 * 1024;

/// Bytes por fila del dump.
const COLS: usize = 16;

/// Estado del visor.
#[derive(Debug, Clone)]
pub enum HexPreview {
    /// Sin archivo seleccionado.
    Empty,
    /// Dump listo. `total` es el tamaño real del archivo (puede exceder
    /// los bytes volcados → el header lo señala).
    Dump {
        text: String,
        total: u64,
        shown: usize,
    },
    /// `fs::read`/`metadata` falló.
    Error(String),
}

impl Default for HexPreview {
    fn default() -> Self {
        HexPreview::Empty
    }
}

/// Lee hasta `max_bytes` del inicio del archivo y arma el dump.
pub fn load_hex(path: &Path, max_bytes: usize) -> HexPreview {
    use std::io::Read;
    let total = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => return HexPreview::Error(e.to_string()),
    };
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return HexPreview::Error(e.to_string()),
    };
    let mut buf = vec![0u8; max_bytes];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(e) => return HexPreview::Error(e.to_string()),
    };
    buf.truncate(n);
    HexPreview::Dump {
        text: dump(&buf),
        total,
        shown: n,
    }
}

/// Formatea `bytes` como `OFFSET  hex(8) hex(8)  |ascii|`, 16 por fila.
fn dump(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4);
    for (row, chunk) in bytes.chunks(COLS).enumerate() {
        if row > 0 {
            out.push('\n');
        }
        // Offset.
        let offset = row * COLS;
        out.push_str(&format!("{offset:08x}  "));
        // Hex, en dos grupos de 8 separados por un espacio extra.
        for i in 0..COLS {
            if i == COLS / 2 {
                out.push(' ');
            }
            match chunk.get(i) {
                Some(b) => out.push_str(&format!("{b:02x} ")),
                None => out.push_str("   "), // relleno para alinear el ascii
            }
        }
        // ASCII.
        out.push_str(" |");
        for &b in chunk {
            let c = if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            };
            out.push(c);
        }
        out.push('|');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_basico_alinea_offset_hex_ascii() {
        let d = dump(b"Hello, world!");
        // Una sola fila: 13 bytes.
        assert!(d.starts_with("00000000  "));
        assert!(d.contains("48 65 6c 6c 6f")); // "Hello"
        assert!(d.ends_with("|Hello, world!|"));
    }

    #[test]
    fn no_imprimibles_son_punto() {
        let d = dump(&[0x00, 0x1f, 0x7f, 0x41]);
        assert!(d.ends_with("|...A|"));
    }

    #[test]
    fn dos_filas_tienen_offset_correcto() {
        let bytes: Vec<u8> = (0u8..20).collect();
        let d = dump(&bytes);
        let mut lines = d.lines();
        assert!(lines.next().unwrap().starts_with("00000000  "));
        assert!(lines.next().unwrap().starts_with("00000010  "));
    }

    #[test]
    fn load_inexistente_es_error() {
        assert!(matches!(
            load_hex(Path::new("/no/existe.bin"), DEFAULT_HEX_BYTES_MAX),
            HexPreview::Error(_)
        ));
    }
}
