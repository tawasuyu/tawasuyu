//! Lector del contenedor **PMTiles v3** — el "single-file vector tiles" que
//! ubica los bytes de cada tile *sin red ni servidor*: un archivo que leés
//! local (o desde tu propio bucket). Es la pieza soberana que faltaba para el
//! basemap de calles, complementando el decoder MVT de [`super::vt`].
//!
//! Implementa lo intrincado del formato a mano: IDs de tile en **curva de
//! Hilbert**, **directorios** de dos niveles (root + leaf) con entradas
//! columnar delta-encoded, y descompresión (none/gzip). Spec:
//! <https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md>.
//!
//! Verificado con tests sintéticos (Hilbert contra valores conocidos +
//! round-trip de un archivo mínimo construido a mano). La validación contra
//! un `.pmtiles` real es el último paso pendiente.

use std::io::Read;

/// Cabecera PMTiles v3 (campos que usamos).
#[derive(Debug, Clone)]
pub struct Header {
    pub root_offset: u64,
    pub root_length: u64,
    pub leaf_offset: u64,
    pub leaf_length: u64,
    pub tile_offset: u64,
    pub tile_length: u64,
    pub min_zoom: u8,
    pub max_zoom: u8,
    /// Compresión de directorios/metadata: 1 none, 2 gzip.
    pub internal_compression: u8,
    /// Compresión de los tiles: 1 none, 2 gzip.
    pub tile_compression: u8,
    /// Tipo de tile: 1 = MVT.
    pub tile_type: u8,
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

/// Una entrada de directorio: cubre los tile-ids `[tile_id, tile_id+run_length)`.
/// `run_length == 0` marca un puntero a un directorio *leaf*.
#[derive(Debug, Clone, Copy)]
struct Entry {
    tile_id: u64,
    offset: u64,
    length: u64,
    run_length: u32,
}

/// Archivo PMTiles cargado en memoria (MVP; mmap/stream sería el paso fino
/// para planetas de varios GB).
pub struct PmTiles {
    data: Vec<u8>,
    pub header: Header,
}

const MAGIC: &[u8; 7] = b"PMTiles";
const HEADER_LEN: usize = 127;

impl PmTiles {
    /// Abre y parsea un `.pmtiles` desde disco.
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        Self::from_bytes(data)
    }

    /// Parsea un `.pmtiles` ya en memoria (también el camino de los tests).
    pub fn from_bytes(data: Vec<u8>) -> Result<Self, String> {
        if data.len() < HEADER_LEN {
            return Err("pmtiles: archivo más corto que la cabecera".into());
        }
        if &data[0..7] != MAGIC {
            return Err("pmtiles: magic inválido".into());
        }
        if data[7] != 3 {
            return Err(format!(
                "pmtiles: versión {} no soportada (sólo v3)",
                data[7]
            ));
        }
        let header = Header {
            root_offset: u64le(&data, 8),
            root_length: u64le(&data, 16),
            leaf_offset: u64le(&data, 40),
            leaf_length: u64le(&data, 48),
            tile_offset: u64le(&data, 56),
            tile_length: u64le(&data, 64),
            internal_compression: data[97],
            tile_compression: data[98],
            tile_type: data[99],
            min_zoom: data[100],
            max_zoom: data[101],
            min_lon: i32le(&data, 102) as f64 / 1e7,
            min_lat: i32le(&data, 106) as f64 / 1e7,
            max_lon: i32le(&data, 110) as f64 / 1e7,
            max_lat: i32le(&data, 114) as f64 / 1e7,
        };
        Ok(PmTiles { data, header })
    }

    /// Devuelve los bytes (ya descomprimidos) del tile `z/x/y`, o `None` si no
    /// existe. Desciende por hasta unos niveles de directorios leaf.
    pub fn tile(&self, z: u32, x: u32, y: u32) -> Option<Vec<u8>> {
        let tid = zxy_to_tile_id(z, x, y);
        let mut dir = self.read_dir(self.header.root_offset, self.header.root_length)?;
        for _ in 0..4 {
            let e = find_entry(&dir, tid)?;
            if e.run_length == 0 {
                // Puntero a directorio leaf.
                dir = self.read_dir(self.header.leaf_offset + e.offset, e.length)?;
            } else if tid < e.tile_id + e.run_length as u64 {
                let start = (self.header.tile_offset + e.offset) as usize;
                let end = start.checked_add(e.length as usize)?;
                let raw = self.data.get(start..end)?;
                return decompress(raw, self.header.tile_compression);
            } else {
                return None;
            }
        }
        None
    }

    /// Lee y descomprime un directorio en `[offset, offset+length)`.
    fn read_dir(&self, offset: u64, length: u64) -> Option<Vec<Entry>> {
        let start = offset as usize;
        let end = start.checked_add(length as usize)?;
        let raw = self.data.get(start..end)?;
        let bytes = decompress(raw, self.header.internal_compression)?;
        parse_directory(&bytes)
    }
}

/// Parsea un directorio (ya descomprimido) en sus entradas.
fn parse_directory(b: &[u8]) -> Option<Vec<Entry>> {
    let mut p = VarintReader::new(b);
    let n = p.read()? as usize;
    if n > 10_000_000 {
        return None; // guardia anti-corrupción
    }
    let mut entries = vec![
        Entry {
            tile_id: 0,
            offset: 0,
            length: 0,
            run_length: 0
        };
        n
    ];
    // tile_ids (delta-encoded).
    let mut last = 0u64;
    for e in entries.iter_mut() {
        last += p.read()?;
        e.tile_id = last;
    }
    for e in entries.iter_mut() {
        e.run_length = p.read()? as u32;
    }
    for e in entries.iter_mut() {
        e.length = p.read()?;
    }
    // offsets: 0 = contiguo al anterior; v>0 = v-1.
    for i in 0..n {
        let v = p.read()?;
        entries[i].offset = if v == 0 {
            if i == 0 {
                return None;
            }
            entries[i - 1].offset + entries[i - 1].length
        } else {
            v - 1
        };
    }
    Some(entries)
}

/// Mayor entrada con `tile_id <= target` (búsqueda binaria; las entradas están
/// ordenadas por tile_id).
fn find_entry(entries: &[Entry], target: u64) -> Option<Entry> {
    if entries.is_empty() || entries[0].tile_id > target {
        return None;
    }
    let mut lo = 0usize;
    let mut hi = entries.len(); // hi exclusivo
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if entries[mid].tile_id <= target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(entries[lo])
}

/// `z/x/y` → tile-id PMTiles: offset acumulado de zooms menores + índice de
/// Hilbert dentro del zoom.
pub fn zxy_to_tile_id(z: u32, x: u32, y: u32) -> u64 {
    // Tiles en zooms 0..z = (4^z - 1) / 3.
    let base = ((1u64 << (2 * z)) - 1) / 3;
    base + hilbert_xy2d(z, x, y)
}

/// Índice de Hilbert de `(x, y)` en una grilla `2^z × 2^z`.
fn hilbert_xy2d(z: u32, x: u32, y: u32) -> u64 {
    let n: u64 = 1 << z;
    let (mut x, mut y) = (x as u64, y as u64);
    let mut d: u64 = 0;
    let mut s = n / 2;
    while s > 0 {
        let rx = if (x & s) > 0 { 1u64 } else { 0 };
        let ry = if (y & s) > 0 { 1u64 } else { 0 };
        d += s * s * ((3 * rx) ^ ry);
        // Rotar el cuadrante.
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    d
}

/// Descomprime según el código de compresión PMTiles (1 none, 2 gzip).
fn decompress(raw: &[u8], compression: u8) -> Option<Vec<u8>> {
    match compression {
        1 => Some(raw.to_vec()),
        2 => {
            let mut d = flate2::read::GzDecoder::new(raw);
            let mut out = Vec::new();
            d.read_to_end(&mut out).ok()?;
            Some(out)
        }
        _ => None, // brotli/zstd: fuera de alcance (evitamos deps pesadas)
    }
}

fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn i32le(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// Lector de varints LEB128 sobre un buffer.
struct VarintReader<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> VarintReader<'a> {
    fn new(b: &'a [u8]) -> Self {
        VarintReader { b, i: 0 }
    }
    fn read(&mut self) -> Option<u64> {
        let mut shift = 0u32;
        let mut out = 0u64;
        loop {
            let byte = *self.b.get(self.i)?;
            self.i += 1;
            out |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Some(out);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hilbert_valores_conocidos() {
        // Grilla 2×2 (z=1), orden Hilbert canónico.
        assert_eq!(hilbert_xy2d(1, 0, 0), 0);
        assert_eq!(hilbert_xy2d(1, 0, 1), 1);
        assert_eq!(hilbert_xy2d(1, 1, 1), 2);
        assert_eq!(hilbert_xy2d(1, 1, 0), 3);
    }

    #[test]
    fn tile_id_acumula_zooms() {
        assert_eq!(zxy_to_tile_id(0, 0, 0), 0);
        // z=1 arranca en 1 (tras el único tile de z0).
        assert_eq!(zxy_to_tile_id(1, 0, 0), 1);
        assert_eq!(zxy_to_tile_id(1, 1, 0), 4);
        // z=2 arranca en 5 (1 + 4).
        assert_eq!(zxy_to_tile_id(2, 0, 0), 5);
    }

    // --- Round-trip: construir un .pmtiles mínimo y leerlo ---

    fn varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    }

    /// Serializa un directorio de una sola entrada (sin compresión).
    fn write_dir(tile_id: u64, offset: u64, length: u64, run: u64) -> Vec<u8> {
        let mut d = Vec::new();
        varint(&mut d, 1); // num_entries
        varint(&mut d, tile_id); // delta desde 0
        varint(&mut d, run);
        varint(&mut d, length);
        varint(&mut d, offset + 1); // evita el atajo "contiguo"
        d
    }

    fn build_pmtiles(tile_bytes: &[u8]) -> Vec<u8> {
        let dir = write_dir(0, 0, tile_bytes.len() as u64, 1);
        let root_off = HEADER_LEN as u64;
        let root_len = dir.len() as u64;
        let tile_off = root_off + root_len; // metadata len 0
        let mut h = vec![0u8; HEADER_LEN];
        h[0..7].copy_from_slice(MAGIC);
        h[7] = 3;
        h[8..16].copy_from_slice(&root_off.to_le_bytes());
        h[16..24].copy_from_slice(&root_len.to_le_bytes());
        // metadata 24..40 = 0
        h[40..48].copy_from_slice(&tile_off.to_le_bytes()); // leaf offset (sin leaves)
                                                            // leaf length 48..56 = 0
        h[56..64].copy_from_slice(&tile_off.to_le_bytes());
        h[64..72].copy_from_slice(&(tile_bytes.len() as u64).to_le_bytes());
        h[97] = 1; // internal: none
        h[98] = 1; // tile: none
        h[99] = 1; // mvt
        h[100] = 0; // min zoom
        h[101] = 0; // max zoom
        let mut file = h;
        file.extend_from_slice(&dir);
        file.extend_from_slice(tile_bytes);
        file
    }

    #[test]
    fn roundtrip_lee_el_tile() {
        let tile_payload = b"\x01\x02\x03 soy un tile";
        let file = build_pmtiles(tile_payload);
        let pm = PmTiles::from_bytes(file).expect("parsea");
        assert_eq!(pm.header.min_zoom, 0);
        assert_eq!(pm.tile(0, 0, 0).as_deref(), Some(&tile_payload[..]));
        // Un tile inexistente → None.
        assert_eq!(pm.tile(1, 0, 0), None);
    }

    #[test]
    fn gzip_de_directorio() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        // Mismo archivo pero con el directorio gzip e internal_compression=2.
        let tile_payload = b"tile-gz";
        let dir = write_dir(0, 0, tile_payload.len() as u64, 1);
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&dir).unwrap();
        let dir_gz = enc.finish().unwrap();
        let root_off = HEADER_LEN as u64;
        let root_len = dir_gz.len() as u64;
        let tile_off = root_off + root_len;
        let mut h = vec![0u8; HEADER_LEN];
        h[0..7].copy_from_slice(MAGIC);
        h[7] = 3;
        h[8..16].copy_from_slice(&root_off.to_le_bytes());
        h[16..24].copy_from_slice(&root_len.to_le_bytes());
        h[40..48].copy_from_slice(&tile_off.to_le_bytes());
        h[56..64].copy_from_slice(&tile_off.to_le_bytes());
        h[64..72].copy_from_slice(&(tile_payload.len() as u64).to_le_bytes());
        h[97] = 2; // internal: gzip
        h[98] = 1; // tile: none
        h[99] = 1;
        let mut file = h;
        file.extend_from_slice(&dir_gz);
        file.extend_from_slice(tile_payload);
        let pm = PmTiles::from_bytes(file).expect("parsea");
        assert_eq!(pm.tile(0, 0, 0).as_deref(), Some(&tile_payload[..]));
    }

    #[test]
    fn magic_invalido_falla() {
        assert!(PmTiles::from_bytes(vec![0u8; 200]).is_err());
        assert!(PmTiles::from_bytes(b"NOPE".to_vec()).is_err());
    }
}
