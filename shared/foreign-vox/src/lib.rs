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

/// Paleta por defecto **oficial de MagicaVoxel** (sólo se usa si el `.vox` no trae
/// chunk `RGBA`). Es la tabla canónica que publica el formato (ephtracy/voxel-model):
/// los exportes reales casi siempre incluyen `RGBA`, pero los modelos viejos /
/// generados a mano que omiten la paleta ahora abren con los colores correctos en
/// vez de una rampa inventada. `[0]` = vacío. Ver [`DEFAULT_PALETTE_ABGR`].
fn default_palette() -> [[u8; 4]; 256] {
    let mut p = [[0u8; 4]; 256];
    for (c, &v) in DEFAULT_PALETTE_ABGR.iter().enumerate() {
        // La tabla viene en 0xAABBGGRR (igual que el spec); la desempacamos a RGBA.
        p[c] = [
            (v & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            ((v >> 16) & 0xff) as u8,
            ((v >> 24) & 0xff) as u8,
        ];
    }
    p
}

/// Paleta default canónica de MagicaVoxel, en `0xAABBGGRR` por entrada (formato del
/// spec). `[0]` = transparente. Es una rampa de matices ×3 niveles + gradientes de
/// grises/RGB al final — idéntica a la que muestra MagicaVoxel al abrir un modelo
/// sin paleta propia.
#[rustfmt::skip]
const DEFAULT_PALETTE_ABGR: [u32; 256] = [
    0x00000000, 0xffffffff, 0xffccffff, 0xff99ffff, 0xff66ffff, 0xff33ffff, 0xff00ffff, 0xffffccff,
    0xffccccff, 0xff99ccff, 0xff66ccff, 0xff33ccff, 0xff00ccff, 0xffff99ff, 0xffcc99ff, 0xff9999ff,
    0xff6699ff, 0xff3399ff, 0xff0099ff, 0xffff66ff, 0xffcc66ff, 0xff9966ff, 0xff6666ff, 0xff3366ff,
    0xff0066ff, 0xffff33ff, 0xffcc33ff, 0xff9933ff, 0xff6633ff, 0xff3333ff, 0xff0033ff, 0xffff00ff,
    0xffcc00ff, 0xff9900ff, 0xff6600ff, 0xff3300ff, 0xff0000ff, 0xffffffcc, 0xffccffcc, 0xff99ffcc,
    0xff66ffcc, 0xff33ffcc, 0xff00ffcc, 0xffffcccc, 0xffcccccc, 0xff99cccc, 0xff66cccc, 0xff33cccc,
    0xff00cccc, 0xffff99cc, 0xffcc99cc, 0xff9999cc, 0xff6699cc, 0xff3399cc, 0xff0099cc, 0xffff66cc,
    0xffcc66cc, 0xff9966cc, 0xff6666cc, 0xff3366cc, 0xff0066cc, 0xffff33cc, 0xffcc33cc, 0xff9933cc,
    0xff6633cc, 0xff3333cc, 0xff0033cc, 0xffff00cc, 0xffcc00cc, 0xff9900cc, 0xff6600cc, 0xff3300cc,
    0xff0000cc, 0xffffff99, 0xffccff99, 0xff99ff99, 0xff66ff99, 0xff33ff99, 0xff00ff99, 0xffffcc99,
    0xffcccc99, 0xff99cc99, 0xff66cc99, 0xff33cc99, 0xff00cc99, 0xffff9999, 0xffcc9999, 0xff999999,
    0xff669999, 0xff339999, 0xff009999, 0xffff6699, 0xffcc6699, 0xff996699, 0xff666699, 0xff336699,
    0xff006699, 0xffff3399, 0xffcc3399, 0xff993399, 0xff663399, 0xff333399, 0xff003399, 0xffff0099,
    0xffcc0099, 0xff990099, 0xff660099, 0xff330099, 0xff000099, 0xffffff66, 0xffccff66, 0xff99ff66,
    0xff66ff66, 0xff33ff66, 0xff00ff66, 0xffffcc66, 0xffcccc66, 0xff99cc66, 0xff66cc66, 0xff33cc66,
    0xff00cc66, 0xffff9966, 0xffcc9966, 0xff999966, 0xff669966, 0xff339966, 0xff009966, 0xffff6666,
    0xffcc6666, 0xff996666, 0xff666666, 0xff336666, 0xff006666, 0xffff3366, 0xffcc3366, 0xff993366,
    0xff663366, 0xff333366, 0xff003366, 0xffff0066, 0xffcc0066, 0xff990066, 0xff660066, 0xff330066,
    0xff000066, 0xffffff33, 0xffccff33, 0xff99ff33, 0xff66ff33, 0xff33ff33, 0xff00ff33, 0xffffcc33,
    0xffcccc33, 0xff99cc33, 0xff66cc33, 0xff33cc33, 0xff00cc33, 0xffff9933, 0xffcc9933, 0xff999933,
    0xff669933, 0xff339933, 0xff009933, 0xffff6633, 0xffcc6633, 0xff996633, 0xff666633, 0xff336633,
    0xff006633, 0xffff3333, 0xffcc3333, 0xff993333, 0xff663333, 0xff333333, 0xff003333, 0xffff0033,
    0xffcc0033, 0xff990033, 0xff660033, 0xff330033, 0xff000033, 0xffffff00, 0xffccff00, 0xff99ff00,
    0xff66ff00, 0xff33ff00, 0xff00ff00, 0xffffcc00, 0xffcccc00, 0xff99cc00, 0xff66cc00, 0xff33cc00,
    0xff00cc00, 0xffff9900, 0xffcc9900, 0xff999900, 0xff669900, 0xff339900, 0xff009900, 0xffff6600,
    0xffcc6600, 0xff996600, 0xff666600, 0xff336600, 0xff006600, 0xffff3300, 0xffcc3300, 0xff993300,
    0xff663300, 0xff333300, 0xff003300, 0xffff0000, 0xffcc0000, 0xff990000, 0xff660000, 0xff330000,
    0xff0000ee, 0xff0000dd, 0xff0000bb, 0xff0000aa, 0xff000088, 0xff000077, 0xff000055, 0xff000044,
    0xff000022, 0xff000011, 0xff00ee00, 0xff00dd00, 0xff00bb00, 0xff00aa00, 0xff008800, 0xff007700,
    0xff005500, 0xff004400, 0xff002200, 0xff001100, 0xffee0000, 0xffdd0000, 0xffbb0000, 0xffaa0000,
    0xff880000, 0xff770000, 0xff550000, 0xff440000, 0xff220000, 0xff110000, 0xffeeeeee, 0xffdddddd,
    0xffbbbbbb, 0xffaaaaaa, 0xff888888, 0xff777777, 0xff555555, 0xff444444, 0xff222222, 0xff111111,
];

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

    #[test]
    fn paleta_default_es_la_oficial() {
        let p = default_palette();
        assert_eq!(p[0], [0, 0, 0, 0]); // índice 0 = vacío/transparente
        assert_eq!(p[1], [255, 255, 255, 255]); // 0xffffffff = blanco
        assert_eq!(p[6], [255, 255, 0, 255]); // 0xff00ffff (AABBGGRR) = amarillo
        assert_eq!(p[255], [17, 17, 17, 255]); // 0xff111111 = gris oscuro
        // Toda entrada usable (1..256) es opaca y no vacía.
        for c in 1..256 {
            assert_eq!(p[c][3], 255, "alpha en índice {c}");
        }
    }
}
