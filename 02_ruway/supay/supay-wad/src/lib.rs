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

/// Efecto de sonido digital decodificado desde un lump `DS*` en formato
/// DMX. PCM mono normalizado a `f32 ∈ [-1, 1]`. El `sample_rate` nativo
/// (típicamente 11025 Hz) viaja con las muestras para que el mixer
/// resamplee a la frecuencia del dispositivo. Ver [`Wad::sound`].
#[derive(Clone, Debug)]
pub struct Sound {
    /// Frecuencia de muestreo nativa del lump (Hz). Doom usa 11025.
    pub sample_rate: u16,
    /// Muestras mono normalizadas a `[-1, 1]` (128 = silencio en el
    /// PCM original de 8-bit unsigned).
    pub samples: Vec<f32>,
}

/// Un evento de música decodificado del formato MUS de Doom. El timeline
/// se reproduce a 140 Hz (los `delay` están en ticks de ~1/140 s). Ver
/// [`parse_mus`]. El MVP de `supay-audio` sólo consume `NoteOn/Off` y
/// `Volume`; pitch wheel, instrumentos y eventos de sistema se descartan
/// al parsear (no hay banco GENMIDI todavía — Fase 4.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MusEvent {
    /// Nota presionada en `channel` (0-15), `note` MIDI 0-127, `vel` 0-127.
    NoteOn { channel: u8, note: u8, vel: u8 },
    /// Nota soltada.
    NoteOff { channel: u8, note: u8 },
    /// Cambio de volumen del canal (controller #3), 0-127.
    Volume { channel: u8, vol: u8 },
    /// Fin de la partitura (loop o stop según el modo de reproducción).
    End,
}

/// Un evento con el retardo (en ticks de 140 Hz) a esperar **antes** de
/// ejecutarlo, relativo al evento previo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MusStep {
    /// Ticks (1/140 s) a esperar antes de este evento.
    pub delay: u32,
    pub event: MusEvent,
}

/// Partitura MUS parseada: timeline de eventos. El canal 15 (percusión
/// MIDI) se mantiene — el synth decide qué hacer con él.
#[derive(Clone, Debug, Default)]
pub struct MusSong {
    pub steps: Vec<MusStep>,
}

/// Parsea un lump en formato MUS de Doom a un timeline de [`MusStep`].
///
/// Layout MUS: magic `MUS\x1a` | `u16 scoreLen` | `u16 scoreStart` | ...
/// header | lista de instrumentos | score. El score es un stream de
/// eventos: byte descriptor (`bit7`=último-del-grupo, `bits4-6`=tipo,
/// `bits0-3`=canal) + payload por tipo. Tras un evento con `bit7` se lee
/// un delay var-length (7 bits/byte, `bit7`=continúa) en ticks de 140 Hz.
///
/// Sólo materializamos los eventos que el synth usa (play/release/volume/
/// end); pitch wheel, system e instrumentos se saltan pero su delay se
/// preserva. Devuelve `None` si el magic o el header no validan.
pub fn parse_mus(bytes: &[u8]) -> Option<MusSong> {
    if bytes.len() < 16 || &bytes[0..4] != b"MUS\x1a" {
        return None;
    }
    let score_start = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    let score_len = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if score_start > bytes.len() {
        return None;
    }
    let end = (score_start + score_len).min(bytes.len());
    let score = &bytes[score_start..end];

    let mut steps: Vec<MusStep> = Vec::new();
    let mut pos = 0usize;
    // Delay acumulado a aplicar al PRÓXIMO evento materializado.
    let mut pending_delay: u32 = 0;
    // MUS reusa el último volumen del canal cuando una nota no trae vel.
    let mut last_vel = [100u8; 16];

    while pos < score.len() {
        let desc = score[pos];
        pos += 1;
        let last = desc & 0x80 != 0;
        let etype = (desc >> 4) & 0x07;
        let ch = desc & 0x0f;

        let mut materialized: Option<MusEvent> = None;
        match etype {
            0 => {
                // Release: 1 byte (nota).
                if pos >= score.len() {
                    break;
                }
                let note = score[pos] & 0x7f;
                pos += 1;
                materialized = Some(MusEvent::NoteOff { channel: ch, note });
            }
            1 => {
                // Play: 1 byte (nota | bit7=trae volumen), +1 si bit7.
                if pos >= score.len() {
                    break;
                }
                let nb = score[pos];
                pos += 1;
                let note = nb & 0x7f;
                let vel = if nb & 0x80 != 0 {
                    if pos >= score.len() {
                        break;
                    }
                    let v = score[pos] & 0x7f;
                    pos += 1;
                    last_vel[ch as usize] = v;
                    v
                } else {
                    last_vel[ch as usize]
                };
                materialized = Some(MusEvent::NoteOn { channel: ch, note, vel });
            }
            2 => {
                pos += 1; // pitch wheel: 1 byte, ignorado
            }
            3 => {
                pos += 1; // system event: 1 byte, ignorado
            }
            4 => {
                // Controller: 2 bytes (número, valor).
                if pos + 1 >= score.len() {
                    break;
                }
                let ctrl = score[pos];
                let val = score[pos + 1] & 0x7f;
                pos += 2;
                if ctrl == 3 {
                    materialized = Some(MusEvent::Volume { channel: ch, vol: val });
                }
            }
            6 => {
                materialized = Some(MusEvent::End);
            }
            _ => {} // 5, 7: no usados
        }

        if let Some(ev) = materialized {
            steps.push(MusStep {
                delay: pending_delay,
                event: ev,
            });
            pending_delay = 0;
            if ev == MusEvent::End {
                break;
            }
        }

        if last {
            // Delay var-length en ticks de 140 Hz.
            let mut value: u32 = 0;
            while pos < score.len() {
                let b = score[pos];
                pos += 1;
                value = (value << 7) | (b & 0x7f) as u32;
                if b & 0x80 == 0 {
                    break;
                }
            }
            pending_delay = pending_delay.saturating_add(value);
        }
    }

    Some(MusSong { steps })
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

    /// Decodifica un lump de sonido `DS*` en formato DMX a [`Sound`].
    ///
    /// Layout DMX: `u16 formato (==3) | u16 sample_rate | u32 count |
    /// count × u8` (PCM unsigned, 128 = silencio). Las primeras 16 y
    /// últimas 16 muestras son padding (copias del primer/último valor
    /// real) que recortamos como hace Chocolate Doom. Devuelve `None`
    /// si el lump no existe, no es formato 3, o está truncado.
    pub fn sound(&self, name: &str) -> Option<Sound> {
        let l = self.lump(name)?;
        if l.len() < 8 {
            return None;
        }
        let fmt = u16::from_le_bytes([l[0], l[1]]);
        if fmt != 3 {
            return None;
        }
        let rate = u16::from_le_bytes([l[2], l[3]]).max(1);
        let count = u32::from_le_bytes([l[4], l[5], l[6], l[7]]) as usize;
        if count == 0 {
            return None;
        }
        // El count declarado a veces excede el payload real (lumps
        // corruptos / truncados); clampeamos a lo que hay en disco.
        let avail = l.len() - 8;
        let payload = &l[8..8 + count.min(avail)];
        // Recorte de padding: 16 lead + 16 trail si hay suficiente.
        let body = if payload.len() >= 32 {
            &payload[16..payload.len() - 16]
        } else {
            payload
        };
        let samples: Vec<f32> = body.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect();
        if samples.is_empty() {
            return None;
        }
        Some(Sound {
            sample_rate: rate,
            samples,
        })
    }

    /// Lump de música crudo (e.g. `"D_E1M1"`). Devuelve los bytes tal
    /// cual — el caller decide si es MUS (magic `MUS\x1a`) o MIDI.
    pub fn music(&self, name: &str) -> Option<&[u8]> {
        self.lump(name)
    }

    /// Parsea un lump MUS a [`MusSong`]. Devuelve `None` si el magic no
    /// es `MUS\x1a` o el header está truncado. Ver [`parse_mus`].
    pub fn music_song(&self, name: &str) -> Option<MusSong> {
        parse_mus(self.lump(name)?)
    }

    /// Busca un sprite lump por (name, frame_letter, angle).
    ///
    /// La convención de naming Doom es:
    /// - `<NAME><F>0` = omnidireccional (un único frame para todos los
    ///   ángulos; usado por keys, ammo, decoración).
    /// - `<NAME><F><A>` = direccional, `A ∈ '1'..='8'` (1=front,
    ///   3=right, 5=back, 7=left; 2/4/6/8 son cuartos).
    /// - `<NAME><F><A><F2><A2>` = un lump que cubre dos ángulos. Cuando
    ///   se requiere `A2`, se renderiza horizontalmente espejado.
    ///
    /// Orden de fallback:
    /// 1. `<NAME><F><angle>` directo.
    /// 2. `<NAME><F>0` omnidireccional.
    /// 3. Cualquier lump que empiece con `<NAME><F>` y tenga segundo
    ///    suffix igualando `<angle>` → flag `mirror = true`.
    ///
    /// Devuelve `(lump_name, mirror)` o `None` si no se encuentra
    /// nada parseable.
    pub fn sprite_lump(&self, name: &str, frame_letter: char, angle: u8) -> Option<(String, bool)> {
        if angle < 1 || angle > 8 {
            return None;
        }
        let base = format!("{}{}", name.to_ascii_uppercase(), frame_letter.to_ascii_uppercase());
        let digit = (b'0' + angle) as char;
        // 1. Directo.
        let direct = format!("{base}{digit}");
        if self.lump(&direct).is_some() {
            return Some((direct, false));
        }
        // 2. Omnidireccional.
        let omni = format!("{base}0");
        if self.lump(&omni).is_some() {
            return Some((omni, false));
        }
        // 3. Espejado: lump <NAME><F><X><F><angle> donde X es cualquier
        //    dígito 1..8. Escaneo lineal del directorio entre
        //    S_START..S_END — barato porque ~500 lumps de sprites por WAD.
        let s_start = self.lump_index("S_START");
        let s_end = self.lump_index("S_END");
        let (start, end) = match (s_start, s_end) {
            (Some(s), Some(e)) if s < e => (s + 1, e),
            _ => (0, self.entries.len()),
        };
        let f_char = frame_letter.to_ascii_uppercase();
        for i in start..end {
            let Some(n) = self.lump_name_at(i) else { continue };
            // Necesitamos exactamente 8 chars: base(5) + dir1(1) + f(1) + dir2(1).
            // Algunos sprites como `TROOA2A8` son 8 exactos.
            if n.len() != 8 {
                continue;
            }
            let bytes = n.as_bytes();
            // bytes[0..5] == base (4 chars name + 1 char frame)
            if !n.starts_with(&base) {
                continue;
            }
            // bytes[6] debería ser el frame letter otra vez, bytes[7] el ángulo deseado.
            if bytes[6] != f_char as u8 || bytes[7] != digit as u8 {
                continue;
            }
            return Some((n.to_string(), true));
        }
        None
    }

    /// Decodifica `PNAMES` — lista plana de nombres de patches que
    /// `TEXTURE1` referencia por índice. Formato:
    ///
    /// ```text
    /// num_patches  (i32 LE)
    /// num_patches × 8 bytes  (nombre del patch, null-padded ASCII)
    /// ```
    ///
    /// Devuelve los nombres normalizados a uppercase.
    pub fn pnames(&self) -> Option<Vec<String>> {
        let lump = self.lump("PNAMES")?;
        if lump.len() < 4 {
            return None;
        }
        let n = u32_le(&lump[0..4]) as usize;
        let needed = 4 + n * 8;
        if lump.len() < needed {
            return None;
        }
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let off = 4 + i * 8;
            let raw = &lump[off..off + 8];
            out.push(parse_lump_name(raw).to_ascii_uppercase());
        }
        Some(out)
    }

    /// Compone una textura de pared a partir de `TEXTURE1` + `PNAMES`.
    ///
    /// `TEXTURE1` formato:
    ///
    /// ```text
    /// num_textures        (i32 LE)
    /// offsets[num_textures] (i32 LE) — desde inicio del lump al maptexture
    /// por cada texture:
    ///   name      (8 bytes ASCII null-padded)
    ///   masked    (i32 LE, unused)
    ///   width     (i16 LE)
    ///   height    (i16 LE)
    ///   columndir (i32 LE, unused)
    ///   patchcount (i16 LE)
    ///   patchcount × 10 bytes:
    ///     originx (i16) originy (i16) patch_idx (i16) stepdir (i16) colormap (i16)
    /// ```
    ///
    /// La textura compuesta es `width × height` RGBA8, con todos los
    /// patches blittered back-to-front en sus offsets. Pixels fuera de
    /// cualquier patch quedan transparentes (alpha 0) — el caller
    /// puede asumir opaco para texturas de pared one-sided.
    ///
    /// Devuelve `None` si la textura no existe, el lump está mal
    /// formado, o algún patch referenciado falta.
    pub fn texture(
        &self,
        name: &str,
        palette: &[(u8, u8, u8); PALETTE_ENTRIES],
    ) -> Option<Texture> {
        let lump_t1 = self.lump("TEXTURE1").or_else(|| self.lump("TEXTURE2"))?;
        let pnames = self.pnames()?;
        let want = name.to_ascii_uppercase();
        if lump_t1.len() < 4 {
            return None;
        }
        let n = u32_le(&lump_t1[0..4]) as usize;
        // Tabla de offsets a maptexture_t.
        if lump_t1.len() < 4 + n * 4 {
            return None;
        }
        for i in 0..n {
            let off = u32_le(&lump_t1[4 + i * 4..4 + i * 4 + 4]) as usize;
            if off + 22 > lump_t1.len() {
                continue;
            }
            let raw_name = &lump_t1[off..off + 8];
            let tname = parse_lump_name(raw_name).to_ascii_uppercase();
            if tname != want {
                continue;
            }
            let width = i16::from_le_bytes([lump_t1[off + 12], lump_t1[off + 13]]) as i32;
            let height = i16::from_le_bytes([lump_t1[off + 14], lump_t1[off + 15]]) as i32;
            let patchcount =
                i16::from_le_bytes([lump_t1[off + 20], lump_t1[off + 21]]) as usize;
            if width <= 0 || height <= 0 || width > 4096 || height > 4096 {
                return None;
            }
            let w = width as usize;
            let h = height as usize;
            let mut rgba = vec![0u8; w * h * 4]; // alpha 0 por defecto
            let patches_off = off + 22;
            if patches_off + patchcount * 10 > lump_t1.len() {
                return None;
            }
            for pi in 0..patchcount {
                let po = patches_off + pi * 10;
                let originx = i16::from_le_bytes([lump_t1[po], lump_t1[po + 1]]) as i32;
                let originy = i16::from_le_bytes([lump_t1[po + 2], lump_t1[po + 3]]) as i32;
                let patch_idx =
                    i16::from_le_bytes([lump_t1[po + 4], lump_t1[po + 5]]) as usize;
                let Some(pname) = pnames.get(patch_idx) else {
                    continue;
                };
                let Some(patch) = self.patch_rgba(pname, palette) else {
                    continue;
                };
                blit_patch(&mut rgba, w, h, &patch, originx, originy);
            }
            // Forzar alpha 255 en cualquier pixel cubierto por al menos
            // un patch (ya quedó así arriba). Pixels nunca cubiertos
            // siguen alpha 0 — para una pared sólida típica esto no
            // pasa, pero para masked textures (rejas) sí importa.
            return Some(Texture {
                width: w as u16,
                height: h as u16,
                rgba,
            });
        }
        None
    }

    /// Decodifica un lump en formato "patch" (sprites + wall patches) a
    /// RGBA8 con transparencia. El formato:
    ///
    /// ```text
    /// header (8 bytes):
    ///   width        (i16 LE)
    ///   height       (i16 LE)
    ///   leftoffset   (i16 LE)  — distancia del centro al lado izq
    ///   topoffset    (i16 LE)  — distancia del baseline al top
    /// columnofs[width] (i32 LE) — offset desde inicio del lump a cada col
    /// columns: para cada col, secuencia de "posts":
    ///   topdelta (u8)            — 0xFF termina la columna
    ///   length   (u8)
    ///   pad      (u8)            — unused
    ///   length × u8 (palette idx)
    ///   pad      (u8)            — unused
    /// ```
    ///
    /// Pixels no cubiertos por ningún post quedan transparentes
    /// (alpha = 0). El renderer 3D pinta el `Image` resultante como
    /// billboard con scale aplicado.
    pub fn patch_rgba(
        &self,
        name: &str,
        palette: &[(u8, u8, u8); PALETTE_ENTRIES],
    ) -> Option<Patch> {
        let lump = self.lump(name)?;
        decode_patch(lump, palette)
    }
}

/// Sprite o wall-patch decodificado a RGBA8.
#[derive(Clone, Debug)]
pub struct Patch {
    pub width: u16,
    pub height: u16,
    /// Offsets de centrado — el renderer billboarder coloca el píxel
    /// (leftoffset, topoffset) del patch en el "anchor" (típicamente
    /// los pies del mobj). Pueden ser negativos en Doom; los exponemos
    /// como i16 sin saturar.
    pub leftoffset: i16,
    pub topoffset: i16,
    /// `width × height × 4` bytes (RGBA8). Stride = `width × 4`.
    pub rgba: Vec<u8>,
}

/// Textura de pared compuesta a partir de patches via TEXTURE1/PNAMES.
#[derive(Clone, Debug)]
pub struct Texture {
    pub width: u16,
    pub height: u16,
    /// `width × height × 4` bytes RGBA8.
    pub rgba: Vec<u8>,
}

/// Blitta un `Patch` ya decodificado al buffer de la textura en
/// `(originx, originy)`. Pixels transparentes del patch (alpha 0)
/// no escriben — eso es lo que permite que TEXTURE1 componga
/// múltiples patches superpuestos con máscaras.
fn blit_patch(
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
    patch: &Patch,
    originx: i32,
    originy: i32,
) {
    let pw = patch.width as i32;
    let ph = patch.height as i32;
    for py in 0..ph {
        let dy = originy + py;
        if dy < 0 || dy >= dst_h as i32 {
            continue;
        }
        for px in 0..pw {
            let dx = originx + px;
            if dx < 0 || dx >= dst_w as i32 {
                continue;
            }
            let p_off = ((py * pw + px) * 4) as usize;
            if patch.rgba[p_off + 3] == 0 {
                continue;
            }
            let d_off = ((dy as usize) * dst_w + (dx as usize)) * 4;
            dst[d_off] = patch.rgba[p_off];
            dst[d_off + 1] = patch.rgba[p_off + 1];
            dst[d_off + 2] = patch.rgba[p_off + 2];
            dst[d_off + 3] = 0xFF;
        }
    }
}

fn read_i16_le(b: &[u8], off: usize) -> Option<i16> {
    let s = b.get(off..off + 2)?;
    Some(i16::from_le_bytes([s[0], s[1]]))
}

fn read_i32_le(b: &[u8], off: usize) -> Option<i32> {
    let s = b.get(off..off + 4)?;
    Some(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn decode_patch(
    lump: &[u8],
    palette: &[(u8, u8, u8); PALETTE_ENTRIES],
) -> Option<Patch> {
    if lump.len() < 8 {
        return None;
    }
    let width = read_i16_le(lump, 0)?;
    let height = read_i16_le(lump, 2)?;
    let leftoffset = read_i16_le(lump, 4)?;
    let topoffset = read_i16_le(lump, 6)?;
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 {
        // Heurística anti-bogus: ningún patch razonable supera 4096.
        return None;
    }
    let w = width as usize;
    let h = height as usize;
    let header_end = 8 + w * 4;
    if lump.len() < header_end {
        return None;
    }
    let mut rgba = vec![0u8; w * h * 4]; // todo alpha 0 por default
    for col in 0..w {
        let off = read_i32_le(lump, 8 + col * 4)? as usize;
        if off >= lump.len() {
            // Columna apunta fuera del lump — skip.
            continue;
        }
        let mut p = off;
        loop {
            let topdelta = *lump.get(p)?;
            if topdelta == 0xFF {
                break;
            }
            let length = *lump.get(p + 1)? as usize;
            // p+2 = unused pad; data en p+3..p+3+length; p+3+length = pad
            let data_start = p + 3;
            let data_end = data_start + length;
            if data_end > lump.len() {
                break;
            }
            for i in 0..length {
                let y = topdelta as usize + i;
                if y >= h {
                    break;
                }
                let idx = lump[data_start + i] as usize;
                let (r, g, b) = palette[idx];
                let o = (y * w + col) * 4;
                rgba[o] = r;
                rgba[o + 1] = g;
                rgba[o + 2] = b;
                rgba[o + 3] = 0xFF;
            }
            p = data_end + 1; // +1 por el pad final del post
            if p >= lump.len() {
                break;
            }
        }
    }
    Some(Patch {
        width: width as u16,
        height: height as u16,
        leftoffset,
        topoffset,
        rgba,
    })
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

    /// Construye un patch sintético 4×4 con dos posts:
    /// - Columna 0: post topdelta=0, length=2, palette idx 100, 200.
    /// - Columna 1: post topdelta=2, length=2, palette idx 50, 75.
    /// - Columnas 2, 3: vacías (sólo el 0xFF terminator).
    fn build_synth_patch() -> Vec<u8> {
        let mut out = Vec::new();
        // header
        out.extend_from_slice(&4i16.to_le_bytes()); // width
        out.extend_from_slice(&4i16.to_le_bytes()); // height
        out.extend_from_slice(&2i16.to_le_bytes()); // leftoffset
        out.extend_from_slice(&3i16.to_le_bytes()); // topoffset
        // columnofs placeholders (parchear después).
        let cof_start = out.len();
        for _ in 0..4 {
            out.extend_from_slice(&0i32.to_le_bytes());
        }
        let mut col_offs: [u32; 4] = [0; 4];
        // Col 0: post topdelta=0, len=2.
        col_offs[0] = out.len() as u32;
        out.extend_from_slice(&[0u8, 2, 0, 100, 200, 0, 0xFF]);
        // Col 1: post topdelta=2, len=2.
        col_offs[1] = out.len() as u32;
        out.extend_from_slice(&[2u8, 2, 0, 50, 75, 0, 0xFF]);
        // Col 2 + 3: vacías.
        col_offs[2] = out.len() as u32;
        out.extend_from_slice(&[0xFFu8]);
        col_offs[3] = out.len() as u32;
        out.extend_from_slice(&[0xFFu8]);
        // Parchear columnofs.
        for (i, &o) in col_offs.iter().enumerate() {
            let pos = cof_start + i * 4;
            out[pos..pos + 4].copy_from_slice(&(o as i32).to_le_bytes());
        }
        out
    }

    #[test]
    fn patch_decode_synthetic() {
        // Palette: i → (i, i, i) grayscale.
        let mut pal = [(0u8, 0u8, 0u8); PALETTE_ENTRIES];
        for i in 0..PALETTE_ENTRIES {
            let v = i as u8;
            pal[i] = (v, v, v);
        }
        let lump = build_synth_patch();
        let p = decode_patch(&lump, &pal).expect("decode");
        assert_eq!(p.width, 4);
        assert_eq!(p.height, 4);
        assert_eq!(p.leftoffset, 2);
        assert_eq!(p.topoffset, 3);
        assert_eq!(p.rgba.len(), 4 * 4 * 4);
        // Pixel (col=0, y=0) → idx 100 → opaco gris 100.
        let px = |x: usize, y: usize| {
            let o = (y * 4 + x) * 4;
            (p.rgba[o], p.rgba[o + 1], p.rgba[o + 2], p.rgba[o + 3])
        };
        assert_eq!(px(0, 0), (100, 100, 100, 0xFF));
        assert_eq!(px(0, 1), (200, 200, 200, 0xFF));
        // Pixel (col=0, y=2): no cubierto → transparente.
        assert_eq!(px(0, 2), (0, 0, 0, 0));
        // Pixel (col=1, y=2): topdelta=2 → 50 gris opaco.
        assert_eq!(px(1, 2), (50, 50, 50, 0xFF));
        assert_eq!(px(1, 3), (75, 75, 75, 0xFF));
        // Col 1, y < 2: transparente (no post hasta topdelta=2).
        assert_eq!(px(1, 0), (0, 0, 0, 0));
        // Col 2, 3: todas transparentes.
        assert_eq!(px(2, 0), (0, 0, 0, 0));
        assert_eq!(px(3, 3), (0, 0, 0, 0));
    }

    #[test]
    fn patch_decode_rejects_bogus_dimensions() {
        let mut bytes = build_synth_patch();
        // width=0 → rechazo.
        bytes[0] = 0;
        bytes[1] = 0;
        let pal = [(0u8, 0u8, 0u8); PALETTE_ENTRIES];
        assert!(decode_patch(&bytes, &pal).is_none());
    }

    #[test]
    fn patch_decode_handles_truncated_header() {
        let pal = [(0u8, 0u8, 0u8); PALETTE_ENTRIES];
        assert!(decode_patch(&[0u8; 4], &pal).is_none());
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

    /// WAD mínimo con un único lump de sonido DMX (`name`) cuyas
    /// muestras son las dadas (sin padding — el builder agrega los 32
    /// bytes de padding 0x80 alrededor para emular el formato real).
    fn build_sound_wad(name: &str, body: &[u8], rate: u16) -> Vec<u8> {
        // payload = 16 pad + body + 16 pad. count = payload.len().
        let mut payload = vec![0x80u8; 16];
        payload.extend_from_slice(body);
        payload.extend(std::iter::repeat(0x80u8).take(16));
        let count = payload.len() as u32;
        let mut lump = Vec::new();
        lump.extend_from_slice(&3u16.to_le_bytes()); // formato 3
        lump.extend_from_slice(&rate.to_le_bytes());
        lump.extend_from_slice(&count.to_le_bytes());
        lump.extend_from_slice(&payload);

        let mut out = Vec::new();
        out.extend_from_slice(b"IWAD");
        out.extend_from_slice(&1u32.to_le_bytes());
        let dir_off_ph = out.len();
        out.extend_from_slice(&0u32.to_le_bytes());
        let p1 = out.len();
        out.extend_from_slice(&lump);
        let dir_off = out.len() as u32;
        out.extend_from_slice(&(p1 as u32).to_le_bytes());
        out.extend_from_slice(&(lump.len() as u32).to_le_bytes());
        let mut nm = [0u8; 8];
        for (i, b) in name.bytes().take(8).enumerate() {
            nm[i] = b;
        }
        out.extend_from_slice(&nm);
        out[dir_off_ph..dir_off_ph + 4].copy_from_slice(&dir_off.to_le_bytes());
        out
    }

    #[test]
    fn sound_decode_trims_padding_and_normalizes() {
        // body: 128 (silencio) → 0.0, 255 → ~+1, 0 → -1, 192 → +0.5.
        let body = [128u8, 255, 0, 192];
        let bytes = build_sound_wad("DSTEST", &body, 11025);
        let wad = Wad::parse(bytes).unwrap();
        let snd = wad.sound("DSTEST").expect("decode");
        assert_eq!(snd.sample_rate, 11025);
        // El padding (16+16) se recortó → quedan las 4 muestras del body.
        assert_eq!(snd.samples.len(), 4);
        assert!((snd.samples[0] - 0.0).abs() < 1e-6);
        assert!((snd.samples[1] - (127.0 / 128.0)).abs() < 1e-6);
        assert!((snd.samples[2] - (-1.0)).abs() < 1e-6);
        assert!((snd.samples[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sound_rejects_wrong_format() {
        // formato 0 (PC speaker) → None.
        let mut bytes = build_sound_wad("DSTEST", &[10, 20, 30, 40], 11025);
        // El header del lump arranca tras header WAD (12) — parchear el
        // u16 de formato del lump a 0. p1 = 12.
        bytes[12] = 0;
        bytes[13] = 0;
        let wad = Wad::parse(bytes).unwrap();
        assert!(wad.sound("DSTEST").is_none());
    }

    #[test]
    fn sound_missing_lump_is_none() {
        let bytes = build_sound_wad("DSTEST", &[1, 2, 3, 4], 11025);
        let wad = Wad::parse(bytes).unwrap();
        assert!(wad.sound("DSNOPE").is_none());
    }

    /// Construye un lump MUS mínimo: header de 16 bytes (sin
    /// instrumentos) + el `score` dado a continuación.
    fn build_mus(score: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"MUS\x1a");
        out.extend_from_slice(&(score.len() as u16).to_le_bytes()); // scoreLen
        out.extend_from_slice(&16u16.to_le_bytes()); // scoreStart = 16
        out.extend_from_slice(&1u16.to_le_bytes()); // channels
        out.extend_from_slice(&0u16.to_le_bytes()); // sec channels
        out.extend_from_slice(&0u16.to_le_bytes()); // instr count = 0
        out.extend_from_slice(&0u16.to_le_bytes()); // dummy
        debug_assert_eq!(out.len(), 16);
        out.extend_from_slice(score);
        out
    }

    #[test]
    fn parse_mus_play_delay_release_end() {
        // Evento play (tipo 1) canal 0, nota 60, con volumen (bit7) 100,
        // y bit "last" → delay 35 ticks. Luego release (tipo 0) nota 60.
        // Luego score-end (tipo 6).
        let score = [
            // play, ch0, last-bit: 0b1_001_0000 = 0x90
            0x90, 0x80 | 60, 100, // nota 60 con vel 100
            35,   // delay var-length = 35
            // release, ch0, no last: 0b0_000_0000 = 0x00
            0x00, 60, // nota 60
            // score end, ch0: 0b0_110_0000 = 0x60
            0x60,
        ];
        let song = parse_mus(&build_mus(&score)).expect("parse");
        assert_eq!(song.steps.len(), 3);
        assert_eq!(
            song.steps[0],
            MusStep {
                delay: 0,
                event: MusEvent::NoteOn { channel: 0, note: 60, vel: 100 }
            }
        );
        // El delay 35 quedó como delay_before del siguiente evento.
        assert_eq!(
            song.steps[1],
            MusStep {
                delay: 35,
                event: MusEvent::NoteOff { channel: 0, note: 60 }
            }
        );
        assert_eq!(song.steps[2].event, MusEvent::End);
    }

    #[test]
    fn parse_mus_varlen_delay_multibyte() {
        // play con last → delay = 0x81,0x00 = (1<<7)|0 = 128 ticks.
        let score = [
            0x90, 0x80 | 64, 90, 0x81, 0x00, // delay 128
            0x60, // end
        ];
        let song = parse_mus(&build_mus(&score)).unwrap();
        assert_eq!(song.steps[0].event, MusEvent::NoteOn { channel: 0, note: 64, vel: 90 });
        assert_eq!(song.steps[1].delay, 128);
        assert_eq!(song.steps[1].event, MusEvent::End);
    }

    #[test]
    fn parse_mus_controller_volume_and_skipped_events() {
        // controller (tipo 4) ctrl=3 (volumen) val=80 → Volume.
        // pitch wheel (tipo 2) con last → su delay se preserva.
        let score = [
            // controller ch0: 0b0_100_0000 = 0x40
            0x40, 3, 80, // volumen 80
            // pitch wheel ch0 last: 0b1_010_0000 = 0xA0, dato 0x40, delay 10
            0xA0, 0x40, 10,
            // play sin volumen (reusa last_vel=100 default): note 50
            0x10, 50, // 0b0_001_0000 = 0x10
            0x60, // end
        ];
        let song = parse_mus(&build_mus(&score)).unwrap();
        assert_eq!(song.steps[0].event, MusEvent::Volume { channel: 0, vol: 80 });
        // El pitch wheel no se materializa, pero su delay 10 va al play.
        assert_eq!(
            song.steps[1],
            MusStep {
                delay: 10,
                event: MusEvent::NoteOn { channel: 0, note: 50, vel: 100 }
            }
        );
    }

    #[test]
    fn parse_mus_rejects_non_mus() {
        assert!(parse_mus(b"MThd\0\0\0\0").is_none());
        assert!(parse_mus(&[0u8; 4]).is_none());
    }
}
