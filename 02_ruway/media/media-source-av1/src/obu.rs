//! Splitter de OBUs puro-Rust.
//!
//! Una temporal unit de AV1 es una secuencia de **OBU** (Open Bitstream
//! Unit). Cada OBU arranca con un header de 1 byte (+ 1 byte de extensión
//! opcional) y, si trae el bit `has_size_field`, un tamaño LEB128. Este
//! módulo parte el paquete en sus OBUs sin decodificar el contenido —
//! sirve para inspección, filtrado (p.ej. quitar metadata) o para
//! alimentar un decoder OBU-a-OBU. Especificación: AV1 §5.3.
//!
//! No depende del decoder: es bitstream parsing trivial y compila a WASM.

/// Tipos de OBU relevantes (AV1 §6.2.2). Los no listados caen a
/// [`ObuKind::Otro`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObuKind {
    SequenceHeader,
    TemporalDelimiter,
    FrameHeader,
    TileGroup,
    Metadata,
    Frame,
    RedundantFrameHeader,
    TileList,
    Padding,
    Otro(u8),
}

impl ObuKind {
    fn from_type(t: u8) -> Self {
        match t {
            1 => Self::SequenceHeader,
            2 => Self::TemporalDelimiter,
            3 => Self::FrameHeader,
            4 => Self::TileGroup,
            5 => Self::Metadata,
            6 => Self::Frame,
            7 => Self::RedundantFrameHeader,
            8 => Self::TileList,
            15 => Self::Padding,
            other => Self::Otro(other),
        }
    }
}

/// Un OBU localizado dentro del paquete: su tipo y el slice de payload
/// (sin el header ni el campo de tamaño).
#[derive(Debug, Clone, Copy)]
pub struct Obu<'a> {
    pub kind: ObuKind,
    /// `true` si traía bit de extensión (capa temporal/espacial).
    pub has_extension: bool,
    pub payload: &'a [u8],
}

/// Lee un entero LEB128 sin signo (AV1 §4.10.5). Devuelve
/// `(valor, bytes_consumidos)` o `None` si se trunca / excede 8 bytes.
pub fn read_leb128(data: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    for i in 0..8 {
        let byte = *data.get(i)?;
        value |= ((byte & 0x7f) as u64) << (i * 7);
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None // más de 8 bytes sin terminar = inválido
}

/// Parte una temporal unit en sus OBUs. Tolerante: ante un OBU
/// malformado corta y devuelve lo parseado hasta ahí (no panickea).
pub fn split_obus(tu: &[u8]) -> Vec<Obu<'_>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < tu.len() {
        let header = tu[pos];
        // bit 7 forbidden debe ser 0; si no, bitstream roto.
        if header & 0x80 != 0 {
            break;
        }
        let obu_type = (header >> 3) & 0x0f;
        let has_extension = header & 0x04 != 0;
        let has_size = header & 0x02 != 0;
        let mut cursor = pos + 1;
        if has_extension {
            if cursor >= tu.len() {
                break;
            }
            cursor += 1; // byte de extensión
        }
        let payload_len = if has_size {
            match read_leb128(&tu[cursor.min(tu.len())..]) {
                Some((len, consumed)) => {
                    cursor += consumed;
                    len as usize
                }
                None => break,
            }
        } else {
            // Sin campo de tamaño: el OBU ocupa el resto del paquete.
            tu.len().saturating_sub(cursor)
        };
        let end = cursor.saturating_add(payload_len);
        if end > tu.len() {
            break;
        }
        out.push(Obu {
            kind: ObuKind::from_type(obu_type),
            has_extension,
            payload: &tu[cursor..end],
        });
        pos = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leb128_basico() {
        assert_eq!(read_leb128(&[0x00]), Some((0, 1)));
        assert_eq!(read_leb128(&[0x7f]), Some((127, 1)));
        // 128 = 0x80 0x01
        assert_eq!(read_leb128(&[0x80, 0x01]), Some((128, 2)));
        // 300 = 0xac 0x02
        assert_eq!(read_leb128(&[0xac, 0x02]), Some((300, 2)));
        // truncado
        assert_eq!(read_leb128(&[0x80]), None);
    }

    #[test]
    fn split_temporal_delimiter_y_payload() {
        // Temporal delimiter: type=2, has_size=1, size=0.
        //   header = (2<<3)|0x02 = 0x12, leb128 size = 0x00.
        // Seguido de un OBU type=6 (Frame) has_size=1 size=2 payload [0xaa,0xbb].
        //   header = (6<<3)|0x02 = 0x32, size=0x02.
        let tu = [0x12, 0x00, 0x32, 0x02, 0xaa, 0xbb];
        let obus = split_obus(&tu);
        assert_eq!(obus.len(), 2);
        assert_eq!(obus[0].kind, ObuKind::TemporalDelimiter);
        assert_eq!(obus[0].payload.len(), 0);
        assert_eq!(obus[1].kind, ObuKind::Frame);
        assert_eq!(obus[1].payload, &[0xaa, 0xbb]);
    }

    #[test]
    fn real_fixture_primera_tu_tiene_seq_header() {
        use crate::ivf::IvfReader;
        let bytes = include_bytes!("../tests/fixtures/testsrc_64x48.ivf");
        let mut r = IvfReader::new(&bytes[..]).unwrap();
        let tu = r.next_unit().unwrap().unwrap();
        let obus = split_obus(&tu.data);
        // La primera TU de un stream AV1 trae el sequence header.
        assert!(
            obus.iter().any(|o| o.kind == ObuKind::SequenceHeader),
            "esperaba un sequence header en la primera TU, obus={:?}",
            obus.iter().map(|o| o.kind).collect::<Vec<_>>()
        );
    }
}
