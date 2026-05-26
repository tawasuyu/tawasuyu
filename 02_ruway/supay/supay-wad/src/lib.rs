//! `supay-wad` — parser mínimo del formato WAD de id Software.
//!
//! ## Formato (resumen)
//!
//! ```text
//! Header (12 bytes):
//!   0..4   "IWAD" o "PWAD" ASCII
//!   4..8   num_lumps        (u32 LE)
//!   8..12  info_table_off   (u32 LE)
//!
//! Directorio (num_lumps × 16 bytes, en info_table_off):
//!   0..4   file_pos         (u32 LE)
//!   4..8   size             (u32 LE)
//!   8..16  name             (8 ASCII, padded con \0)
//! ```
//!
//! ## Qué soporta esta crate
//!
//! - Lectura completa del WAD a memoria + parseo del directorio.
//! - Lookup por nombre (`lump(name)`), case-insensitive, longitud ≤ 8.
//! - [`Wad::palette`]: parseo de PLAYPAL (256×RGB de la primera de las
//!   14 paletas que el motor usa para distintos states de daño/pickup).
//! - [`Wad::flat`]: lump de 4096 bytes (64×64 indexed) leído crudo.
//! - [`Wad::flat_rgba`]: flat convertido a RGBA8 listo para Image.
//! - [`Wad::flat_center_color`]: pixel central del flat resuelto a RGB
//!   con la paleta — útil para "color promedio" sin samplear texturas.
//!
//! ## Qué NO está
//!
//! - Patches column-format (lumps PATCH/SPRITE) — defer a 3.4 cuando
//!   agreguemos texturing de paredes.
//! - TEXTURE1/TEXTURE2 + PNAMES (composites de pared) — idem.
//! - Niveles (lumps THINGS/LINEDEFS/SIDEDEFS/...) — fuera de scope; el
//!   motor de doomgeneric ya los parsea internamente.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

pub const FLAT_SIZE: usize = 64;
pub const FLAT_BYTES: usize = FLAT_SIZE * FLAT_SIZE;
pub const PALETTE_ENTRIES: usize = 256;
pub const PLAYPAL_BYTES: usize = PALETTE_ENTRIES * 3;

#[derive(Debug)]
pub enum WadError {
    Io(io::Error),
    BadMagic,
    Truncated,
    InvalidDirectory,
    LumpNotFound(String),
    LumpWrongSize { name: String, want: usize, got: usize },
}

impl std::fmt::Display for WadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WadError::Io(e) => write!(f, "io: {e}"),
            WadError::BadMagic => write!(f, "not a WAD (magic != IWAD/PWAD)"),
            WadError::Truncated => write!(f, "WAD truncado"),
            WadError::InvalidDirectory => write!(f, "directorio WAD inconsistente"),
            WadError::LumpNotFound(n) => write!(f, "lump no encontrado: {n}"),
            WadError::LumpWrongSize { name, want, got } => {
                write!(f, "lump {name} tamaño {got} bytes, esperado {want}")
            }
        }
    }
}

impl std::error::Error for WadError {}

impl From<io::Error> for WadError {
    fn from(e: io::Error) -> Self {
        WadError::Io(e)
    }
}

#[derive(Clone, Debug)]
struct DirEntry {
    pos: u32,
    size: u32,
    // Nombre normalizado a uppercase para lookup case-insensitive.
    name_upper: String,
}

pub struct Wad {
    bytes: Vec<u8>,
    // El primer lump que matchea cada nombre (Doom permite duplicados;
    // el primero gana en general, salvo para sprites/flats que pueden
    // venir entre F_START/F_END o S_START/S_END como marcadores).
    index: HashMap<String, usize>,
    entries: Vec<DirEntry>,
}

impl Wad {
    /// Abre y parsea un WAD desde disco. Lee el archivo completo a
    /// memoria — DOOM1.WAD ≈ 4 MB, no es problema; permite que `lump`
    /// devuelva `&[u8]` directo sin manejar I/O por lookup.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WadError> {
        let bytes = fs::read(path)?;
        Self::parse(bytes)
    }

    /// Parsea un WAD desde bytes ya en memoria. Útil para tests o para
    /// hosts que cachean el WAD en otro formato. Equivalente a
    /// `Wad::open` salvo la fuente.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, WadError> {
        if bytes.len() < 12 {
            return Err(WadError::Truncated);
        }
        let magic = &bytes[0..4];
        if magic != b"IWAD" && magic != b"PWAD" {
            return Err(WadError::BadMagic);
        }
        let num_lumps = u32_le(&bytes[4..8]) as usize;
        let dir_off = u32_le(&bytes[8..12]) as usize;
        let dir_end = dir_off
            .checked_add(num_lumps.checked_mul(16).ok_or(WadError::InvalidDirectory)?)
            .ok_or(WadError::InvalidDirectory)?;
        if dir_end > bytes.len() {
            return Err(WadError::InvalidDirectory);
        }
        let mut entries: Vec<DirEntry> = Vec::with_capacity(num_lumps);
        let mut index: HashMap<String, usize> = HashMap::with_capacity(num_lumps);
        for i in 0..num_lumps {
            let off = dir_off + i * 16;
            let pos = u32_le(&bytes[off..off + 4]);
            let size = u32_le(&bytes[off + 4..off + 8]);
            let raw_name = &bytes[off + 8..off + 16];
            let name = parse_lump_name(raw_name);
            let name_upper = name.to_ascii_uppercase();
            // pos/size válidos (un lump puede ser size=0 como marcador F_START).
            if size != 0 && (pos as usize).saturating_add(size as usize) > bytes.len() {
                return Err(WadError::InvalidDirectory);
            }
            // Primer lump por nombre gana.
            index.entry(name_upper.clone()).or_insert(entries.len());
            entries.push(DirEntry {
                pos,
                size,
                name_upper,
            });
        }
        Ok(Self { bytes, index, entries })
    }

    /// Total de lumps en el directorio (incluye markers).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Recupera un lump por nombre (case-insensitive, longitud ≤ 8).
    /// Devuelve `None` si no existe.
    pub fn lump(&self, name: &str) -> Option<&[u8]> {
        let key = name.to_ascii_uppercase();
        let idx = *self.index.get(&key)?;
        self.lump_at(idx)
    }

    /// Recupera por índice en el directorio. Útil para iterar entre
    /// markers (e.g. `F_START`..`F_END`).
    pub fn lump_at(&self, idx: usize) -> Option<&[u8]> {
        let e = self.entries.get(idx)?;
        let start = e.pos as usize;
        let end = start + e.size as usize;
        self.bytes.get(start..end)
    }

    /// Índice del lump por nombre (case-insensitive).
    pub fn lump_index(&self, name: &str) -> Option<usize> {
        let key = name.to_ascii_uppercase();
        self.index.get(&key).copied()
    }

    /// Nombre normalizado (uppercase, 8 chars max) del lump en `idx`.
    pub fn lump_name_at(&self, idx: usize) -> Option<&str> {
        self.entries.get(idx).map(|e| e.name_upper.as_str())
    }

    /// Parsea la primera paleta de PLAYPAL (Doom carga 14 paletas; la 0
    /// es "normal", las demás corresponden a daño/pickup/radsuit). Si
    /// PLAYPAL no existe o es corto, devuelve una grayscale de
    /// fallback para no panickear.
    pub fn palette(&self) -> [(u8, u8, u8); PALETTE_ENTRIES] {
        let mut out = [(0u8, 0u8, 0u8); PALETTE_ENTRIES];
        let bytes = self.lump("PLAYPAL").unwrap_or(&[]);
        if bytes.len() < PLAYPAL_BYTES {
            // Fallback grayscale: índice i → (i, i, i).
            for (i, slot) in out.iter_mut().enumerate() {
                let v = i as u8;
                *slot = (v, v, v);
            }
            return out;
        }
        for i in 0..PALETTE_ENTRIES {
            let o = i * 3;
            out[i] = (bytes[o], bytes[o + 1], bytes[o + 2]);
        }
        out
    }

    /// Devuelve los 4096 bytes raw (palette-indexed) de un flat 64×64.
    /// `None` si no existe o tamaño incorrecto.
    pub fn flat(&self, name: &str) -> Option<&[u8]> {
        let l = self.lump(name)?;
        if l.len() == FLAT_BYTES {
            Some(l)
        } else {
            None
        }
    }

    /// Color RGB del píxel central del flat resuelto contra la paleta
    /// dada. Si el flat no existe, devuelve `None`. Útil para "color
    /// dominante" cheap sin compute average sobre los 4096 píxeles.
    pub fn flat_center_color(
        &self,
        name: &str,
        palette: &[(u8, u8, u8); PALETTE_ENTRIES],
    ) -> Option<(u8, u8, u8)> {
        let flat = self.flat(name)?;
        // Centro = (32, 32) — offset 32*64 + 32 = 2080.
        let center = flat.get(32 * FLAT_SIZE + 32).copied()?;
        Some(palette[center as usize])
    }

    /// Promedio aritmético de los 4096 píxeles del flat — más estable
    /// que `flat_center_color` para flats con detalle alto. O(4096).
    pub fn flat_average_color(
        &self,
        name: &str,
        palette: &[(u8, u8, u8); PALETTE_ENTRIES],
    ) -> Option<(u8, u8, u8)> {
        let flat = self.flat(name)?;
        let mut r = 0u64;
        let mut g = 0u64;
        let mut b = 0u64;
        for &idx in flat {
            let (pr, pg, pb) = palette[idx as usize];
            r += pr as u64;
            g += pg as u64;
            b += pb as u64;
        }
        let n = flat.len() as u64;
        Some(((r / n) as u8, (g / n) as u8, (b / n) as u8))
    }

    /// Flat resuelto a RGBA8 (4096 px × 4 bytes = 16 KB). Listo para
    /// envolver en `peniko::Image` y samplear como texture.
    pub fn flat_rgba(
        &self,
        name: &str,
        palette: &[(u8, u8, u8); PALETTE_ENTRIES],
    ) -> Option<Vec<u8>> {
        let flat = self.flat(name)?;
        let mut out = Vec::with_capacity(FLAT_BYTES * 4);
        for &idx in flat {
            let (r, g, b) = palette[idx as usize];
            out.extend_from_slice(&[r, g, b, 0xFF]);
        }
        Some(out)
    }
}

#[inline]
fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn parse_lump_name(raw: &[u8]) -> String {
    // 8 bytes, null-terminated. Recortamos en el primer 0.
    let mut end = raw.len();
    for (i, &c) in raw.iter().enumerate() {
        if c == 0 {
            end = i;
            break;
        }
    }
    // Filtramos non-ASCII por si llega basura — devolvemos as utf-8 lossy.
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construye un WAD mínimo válido en memoria: header + dos lumps
    /// (uno PLAYPAL grayscale, uno FLAT64 con un patrón checker).
    fn build_synth_wad() -> Vec<u8> {
        let playpal: Vec<u8> = (0..PALETTE_ENTRIES)
            .flat_map(|i| {
                let v = i as u8;
                [v, v, v]
            })
            .collect();
        let mut flat = vec![0u8; FLAT_BYTES];
        for y in 0..FLAT_SIZE {
            for x in 0..FLAT_SIZE {
                flat[y * FLAT_SIZE + x] = if (x / 8 + y / 8) % 2 == 0 { 100 } else { 200 };
            }
        }
        // Header (12) + lumps + dir.
        let mut out = Vec::new();
        out.extend_from_slice(b"IWAD");
        out.extend_from_slice(&2u32.to_le_bytes());
        let dir_off_placeholder = out.len();
        out.extend_from_slice(&0u32.to_le_bytes()); // patched later
        // Lump 1: PLAYPAL.
        let p1 = out.len();
        out.extend_from_slice(&playpal);
        // Lump 2: F_TEST flat.
        let p2 = out.len();
        out.extend_from_slice(&flat);
        // Directorio.
        let dir_off = out.len() as u32;
        out.extend_from_slice(&(p1 as u32).to_le_bytes());
        out.extend_from_slice(&(playpal.len() as u32).to_le_bytes());
        out.extend_from_slice(b"PLAYPAL\0");
        out.extend_from_slice(&(p2 as u32).to_le_bytes());
        out.extend_from_slice(&(flat.len() as u32).to_le_bytes());
        out.extend_from_slice(b"F_TEST\0\0");
        // Parchear el dir_off.
        out[dir_off_placeholder..dir_off_placeholder + 4].copy_from_slice(&dir_off.to_le_bytes());
        out
    }

    #[test]
    fn parses_synth_wad_header_and_directory() {
        let bytes = build_synth_wad();
        let wad = Wad::parse(bytes).expect("parse");
        assert_eq!(wad.len(), 2);
        assert!(wad.lump("PLAYPAL").is_some());
        assert!(wad.lump("F_TEST").is_some());
        // Case-insensitive lookup.
        assert!(wad.lump("playpal").is_some());
        // Lump no existente.
        assert!(wad.lump("NOPE").is_none());
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = build_synth_wad();
        bytes[0] = b'X';
        assert!(matches!(Wad::parse(bytes), Err(WadError::BadMagic)));
    }

    #[test]
    fn rejects_truncated_header() {
        let bytes = vec![b'I', b'W', b'A', b'D', 0, 0];
        assert!(matches!(Wad::parse(bytes), Err(WadError::Truncated)));
    }

    #[test]
    fn palette_is_grayscale_in_synth() {
        let bytes = build_synth_wad();
        let wad = Wad::parse(bytes).unwrap();
        let pal = wad.palette();
        assert_eq!(pal[0], (0, 0, 0));
        assert_eq!(pal[128], (128, 128, 128));
        assert_eq!(pal[255], (255, 255, 255));
    }

    #[test]
    fn flat_lookup_returns_4096_bytes() {
        let bytes = build_synth_wad();
        let wad = Wad::parse(bytes).unwrap();
        let flat = wad.flat("F_TEST").unwrap();
        assert_eq!(flat.len(), FLAT_BYTES);
    }

    #[test]
    fn flat_center_color_resolves_via_palette() {
        let bytes = build_synth_wad();
        let wad = Wad::parse(bytes).unwrap();
        let pal = wad.palette();
        let center = wad.flat_center_color("F_TEST", &pal).unwrap();
        // Centro (32, 32) → patrón checker: x/8=4, y/8=4, suma=8 par
        // → índice 100 → grayscale = (100, 100, 100).
        assert_eq!(center, (100, 100, 100));
    }

    #[test]
    fn flat_average_color_correct_for_checker() {
        let bytes = build_synth_wad();
        let wad = Wad::parse(bytes).unwrap();
        let pal = wad.palette();
        let avg = wad.flat_average_color("F_TEST", &pal).unwrap();
        // 50% pixels = 100, 50% = 200 → promedio 150.
        assert_eq!(avg, (150, 150, 150));
    }

    #[test]
    fn flat_rgba_has_expected_size() {
        let bytes = build_synth_wad();
        let wad = Wad::parse(bytes).unwrap();
        let pal = wad.palette();
        let rgba = wad.flat_rgba("F_TEST", &pal).unwrap();
        assert_eq!(rgba.len(), FLAT_BYTES * 4);
        // Píxel (0,0) → x/8=0, y/8=0, par → índice 100 → (100,100,100,255).
        assert_eq!(&rgba[0..4], &[100, 100, 100, 255]);
    }
}
