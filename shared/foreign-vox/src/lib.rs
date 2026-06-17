//! # foreign-vox — puente al formato **MagicaVoxel `.vox`**
//!
//! Lee y escribe modelos voxel en el formato de MagicaVoxel (estructura de
//! *chunks* tipo-RIFF: `MAIN` → `SIZE`/`XYZI`/`RGBA`) y los expone como un
//! [`VoxModel`] **neutral**: dimensiones + lista de voxels `(x,y,z,índice)` +
//! una paleta de 256 colores RGBA indexada por el índice del voxel.
//!
//! Sin dependencias (sólo `std`): el formato son bytes planos *little-endian*.
//! **No** conoce el motor — el conversor a `VoxelGrid` vive en `llimphi-voxel`
//! (CLAUDE.md regla #4: lo ajeno entra por el puente, el núcleo trabaja nativo).
//!
//! Convención de color: el índice `i` de un voxel (1..255; `0` = vacío) indexa
//! [`VoxModel::palette`]`[i]`. Al leer el chunk `RGBA` (256 entradas crudas) se
//! aplica el corrimiento documentado por MagicaVoxel (`palette[i] = crudo[i-1]`).
//! Si el archivo no trae `RGBA`, se usa una paleta por defecto (rampa HSV) — los
//! exportes reales de MagicaVoxel siempre incluyen `RGBA`, así que es un fallback.

use std::fmt;

/// Un voxel: posición en la grilla del modelo (`0..size`) + índice de color
/// (`1..255`) en la [`VoxModel::palette`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Voxel {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub i: u8,
}

/// Un modelo voxel: dimensiones, voxels y paleta (indexada por `voxel.i`).
#[derive(Debug, Clone)]
pub struct VoxModel {
    /// Dimensiones `[x, y, z]` (en el espacio del `.vox`, donde `z` es arriba).
    pub size: [u32; 3],
    /// Voxels ocupados.
    pub voxels: Vec<Voxel>,
    /// 256 colores RGBA; `palette[voxel.i]` es el color del voxel (`[0]` vacío).
    pub palette: [[u8; 4]; 256],
}

impl VoxModel {
    /// Modelo vacío de las dimensiones dadas con la paleta por defecto.
    pub fn new(size: [u32; 3]) -> Self {
        Self { size, voxels: Vec::new(), palette: default_palette() }
    }

    /// Color RGBA de un voxel (vía su índice en la paleta).
    pub fn color(&self, v: &Voxel) -> [u8; 4] {
        self.palette[v.i as usize]
    }
}

/// Error de parseo de un `.vox`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoxError {
    /// Faltan los 4 bytes mágicos `"VOX "` al inicio.
    BadMagic,
    /// El buffer se corta antes de lo que un chunk declara.
    Truncated,
    /// No hay ningún par `SIZE`+`XYZI` (ningún modelo).
    NoModel,
}

impl fmt::Display for VoxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoxError::BadMagic => write!(f, "no es un .vox (faltan los bytes 'VOX ')"),
            VoxError::Truncated => write!(f, ".vox truncado (un chunk declara más bytes de los que hay)"),
            VoxError::NoModel => write!(f, ".vox sin modelos (ningún par SIZE+XYZI)"),
        }
    }
}

impl std::error::Error for VoxError {}

/// Lee un `.vox` y devuelve todos sus modelos (uno por par `SIZE`+`XYZI`). La
/// paleta `RGBA`, si existe, se aplica a todos.
pub fn parse(bytes: &[u8]) -> Result<Vec<VoxModel>, VoxError> {
    if bytes.len() < 8 || &bytes[0..4] != b"VOX " {
        return Err(VoxError::BadMagic);
    }
    // Tras el header (8 bytes) viene el chunk MAIN; sus hijos son el resto. No
    // hace falta interpretar MAIN: barremos los chunks linealmente desde su
    // contenido en adelante (SIZE/XYZI/RGBA no tienen hijos).
    let mut pos = 8usize;
    // Saltar el header del chunk MAIN (id + nContent + nChildren = 12 bytes) y su
    // contenido (que es 0 en la práctica).
    let main_n = read_u32(bytes, pos + 4)? as usize;
    pos += 12 + main_n;

    let mut sizes: Vec<[u32; 3]> = Vec::new();
    let mut groups: Vec<Vec<Voxel>> = Vec::new();
    let mut palette: Option<[[u8; 4]; 256]> = None;

    while pos + 12 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let n = read_u32(bytes, pos + 4)? as usize;
        let m = read_u32(bytes, pos + 8)? as usize;
        let content_start = pos + 12;
        let content_end = content_start.checked_add(n).ok_or(VoxError::Truncated)?;
        if content_end > bytes.len() {
            return Err(VoxError::Truncated);
        }
        let content = &bytes[content_start..content_end];

        match id {
            b"SIZE" => {
                if content.len() < 12 {
                    return Err(VoxError::Truncated);
                }
                sizes.push([
                    read_u32(content, 0)?,
                    read_u32(content, 4)?,
                    read_u32(content, 8)?,
                ]);
            }
            b"XYZI" => {
                let count = read_u32(content, 0)? as usize;
                let mut vs = Vec::with_capacity(count);
                let need = 4 + count * 4;
                if content.len() < need {
                    return Err(VoxError::Truncated);
                }
                for k in 0..count {
                    let o = 4 + k * 4;
                    vs.push(Voxel { x: content[o], y: content[o + 1], z: content[o + 2], i: content[o + 3] });
                }
                groups.push(vs);
            }
            b"RGBA" => {
                if content.len() < 256 * 4 {
                    return Err(VoxError::Truncated);
                }
                let mut pal = [[0u8; 4]; 256];
                // Corrimiento MagicaVoxel: el índice de voxel `c` (1..255) → la
                // `c-1`-ésima entrada cruda. `palette[0]` queda vacío.
                for c in 1..256usize {
                    let o = (c - 1) * 4;
                    pal[c] = [content[o], content[o + 1], content[o + 2], content[o + 3]];
                }
                palette = Some(pal);
            }
            _ => {} // PACK, nTRN, nGRP, MATL, … no afectan la geometría base.
        }
        pos = content_end + m;
    }

    if sizes.is_empty() || groups.is_empty() {
        return Err(VoxError::NoModel);
    }
    let pal = palette.unwrap_or_else(default_palette);
    let n_models = sizes.len().min(groups.len());
    Ok((0..n_models)
        .map(|k| VoxModel { size: sizes[k], voxels: groups[k].clone(), palette: pal })
        .collect())
}

/// Serializa un modelo a bytes `.vox` (header + `MAIN` con `SIZE`+`XYZI`+`RGBA`).
/// Útil para **exportar** una escena voxel y editarla en MagicaVoxel, y para las
/// pruebas de ida y vuelta.
pub fn write(model: &VoxModel) -> Vec<u8> {
    // SIZE.
    let mut size = Vec::with_capacity(12);
    for d in model.size {
        size.extend_from_slice(&d.to_le_bytes());
    }
    // XYZI.
    let mut xyzi = Vec::with_capacity(4 + model.voxels.len() * 4);
    xyzi.extend_from_slice(&(model.voxels.len() as u32).to_le_bytes());
    for v in &model.voxels {
        xyzi.extend_from_slice(&[v.x, v.y, v.z, v.i]);
    }
    // RGBA: crudo[j] = palette[j+1] (inverso del corrimiento de lectura).
    let mut rgba = Vec::with_capacity(256 * 4);
    for j in 0..256usize {
        let c = j + 1;
        rgba.extend_from_slice(&model.palette.get(c).copied().unwrap_or([0, 0, 0, 0]));
    }

    let mut children = Vec::new();
    children.extend_from_slice(&chunk(b"SIZE", &size));
    children.extend_from_slice(&chunk(b"XYZI", &xyzi));
    children.extend_from_slice(&chunk(b"RGBA", &rgba));

    let mut out = Vec::new();
    out.extend_from_slice(b"VOX ");
    out.extend_from_slice(&150u32.to_le_bytes()); // versión
    // MAIN: contenido vacío, hijos = los chunks de arriba.
    out.extend_from_slice(b"MAIN");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(children.len() as u32).to_le_bytes());
    out.extend_from_slice(&children);
    out
}

/// Codifica un chunk **sin hijos**: `id` + `len(content)` + `0` + `content`.
fn chunk(id: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let mut c = Vec::with_capacity(12 + content.len());
    c.extend_from_slice(id);
    c.extend_from_slice(&(content.len() as u32).to_le_bytes());
    c.extend_from_slice(&0u32.to_le_bytes());
    c.extend_from_slice(content);
    c
}

/// Lee un `u32` little-endian en `off`, o [`VoxError::Truncated`] si no entra.
fn read_u32(b: &[u8], off: usize) -> Result<u32, VoxError> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(VoxError::Truncated)
}

/// Paleta por defecto (sólo si el `.vox` no trae `RGBA`): rampa HSV determinista
/// — no es la paleta oficial de MagicaVoxel, pero da colores distinguibles.
/// `[0]` = vacío.
fn default_palette() -> [[u8; 4]; 256] {
    let mut p = [[0u8; 4]; 256];
    for c in 1..256usize {
        let h = (c as f32 * 137.5) % 360.0; // ángulo áureo → buena dispersión
        let s = 0.55 + 0.35 * (((c * 7) % 5) as f32 / 4.0);
        let v = 0.65 + 0.30 * (((c * 13) % 4) as f32 / 3.0);
        let [r, g, b] = hsv_to_rgb(h, s, v);
        p[c] = [r, g, b, 255];
    }
    p
}

/// HSV (`h`∈[0,360), `s,v`∈[0,1]) → RGB `u8`.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modelo_demo() -> VoxModel {
        let mut m = VoxModel::new([4, 5, 6]);
        m.palette[1] = [200, 30, 30, 255];
        m.palette[2] = [30, 180, 60, 255];
        m.voxels = vec![
            Voxel { x: 0, y: 0, z: 0, i: 1 },
            Voxel { x: 3, y: 4, z: 5, i: 2 },
            Voxel { x: 1, y: 2, z: 3, i: 1 },
        ];
        m
    }

    #[test]
    fn ida_y_vuelta() {
        let m = modelo_demo();
        let bytes = write(&m);
        let back = parse(&bytes).expect("parse");
        assert_eq!(back.len(), 1);
        let r = &back[0];
        assert_eq!(r.size, [4, 5, 6]);
        assert_eq!(r.voxels, m.voxels);
        // Los colores de los índices usados sobreviven el corrimiento RGBA.
        assert_eq!(r.palette[1], [200, 30, 30, 255]);
        assert_eq!(r.palette[2], [30, 180, 60, 255]);
    }

    #[test]
    fn rechaza_no_vox() {
        assert_eq!(parse(b"NOPE....").unwrap_err(), VoxError::BadMagic);
    }

    #[test]
    fn parsea_bytes_a_mano_sin_rgba() {
        // .vox mínimo escrito a mano: header + MAIN(vacío) + SIZE(1,1,1) +
        // XYZI(1 voxel en 0,0,0 índice 1). Sin RGBA → paleta por defecto.
        let mut b = Vec::new();
        b.extend_from_slice(b"VOX ");
        b.extend_from_slice(&150u32.to_le_bytes());
        // MAIN: n=0, m = bytes de los dos chunks hijos (24+16=... lo calculamos).
        let size_chunk = {
            let mut c = Vec::new();
            c.extend_from_slice(b"SIZE");
            c.extend_from_slice(&12u32.to_le_bytes());
            c.extend_from_slice(&0u32.to_le_bytes());
            c.extend_from_slice(&1u32.to_le_bytes());
            c.extend_from_slice(&1u32.to_le_bytes());
            c.extend_from_slice(&1u32.to_le_bytes());
            c
        };
        let xyzi_chunk = {
            let mut c = Vec::new();
            c.extend_from_slice(b"XYZI");
            c.extend_from_slice(&8u32.to_le_bytes()); // 4 (count) + 4 (un voxel)
            c.extend_from_slice(&0u32.to_le_bytes());
            c.extend_from_slice(&1u32.to_le_bytes()); // count
            c.extend_from_slice(&[0, 0, 0, 1]); // voxel
            c
        };
        let children_len = size_chunk.len() + xyzi_chunk.len();
        b.extend_from_slice(b"MAIN");
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&(children_len as u32).to_le_bytes());
        b.extend_from_slice(&size_chunk);
        b.extend_from_slice(&xyzi_chunk);

        let models = parse(&b).expect("parse");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].size, [1, 1, 1]);
        assert_eq!(models[0].voxels, vec![Voxel { x: 0, y: 0, z: 0, i: 1 }]);
        assert_ne!(models[0].palette[1], [0, 0, 0, 0]); // default no vacío
    }
}
