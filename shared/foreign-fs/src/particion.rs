// =============================================================================
//  foreign-fs :: particion — tabla de particiones GPT/MBR + autodetección de FS
// -----------------------------------------------------------------------------
//  Un USB o un disco real NO es un sistema de archivos suelto: es un medio
//  PARTICIONADO (GPT moderno o MBR clásico) con uno o más FS dentro. Esta capa
//  enumera las particiones de un `&[u8]` crudo, autodetecta el FS de cada una
//  (FAT vs ext) y la absorbe al grafo con el lector adecuado. Es el paso que
//  hace que `foreign-fs` funcione sobre un dispositivo de verdad —el caso de
//  "absorber el USB" / "instalar desde USB" de la visión— y no sólo sobre una
//  imagen de una sola partición.
//
//  Cada partición se sub-slicea (`&datos[inicio..inicio+tam]`) y se pasa al
//  lector tal cual: el BPB de FAT vive en el offset 0 de su partición y el
//  superbloque ext en el 1024 de la suya, así que el sub-slice basta —cero
//  copia, cero ajuste de offsets en los lectores—.
//
//  Asume sectores lógicos de 512 B (la convención universal de GPT/MBR; los
//  discos 4Kn nativos quedan fuera del MVP).
// =============================================================================

use alloc::format;
use alloc::vec::Vec;

use crate::ext4::LectorExt4;
use crate::fat::LectorFat;
use crate::{absorber, Emisor, FsError};

/// Tamaño de sector lógico asumido para interpretar LBAs de GPT/MBR.
const SECTOR: u64 = 512;
/// Firma de la cabecera GPT, en el LBA 1 (offset 512).
const GPT_SIG: &[u8; 8] = b"EFI PART";

#[inline]
fn u16le(d: &[u8], o: usize) -> Result<u16, FsError> {
    d.get(o..o + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or(FsError::MedioInvalido("u16 fuera del medio"))
}

#[inline]
fn u32le(d: &[u8], o: usize) -> Result<u32, FsError> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(FsError::MedioInvalido("u32 fuera del medio"))
}

#[inline]
fn u64le(d: &[u8], o: usize) -> Result<u64, FsError> {
    d.get(o..o + 8)
        .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
        .ok_or(FsError::MedioInvalido("u64 fuera del medio"))
}

/// El esquema que produjo una partición.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Esquema {
    Gpt,
    Mbr,
    /// El medio entero es un FS suelto (sin tabla de particiones).
    SinTabla,
}

/// Una partición localizada: dónde empieza, cuánto mide y su slot 1-based en la
/// tabla (la base del nombre `particionN`).
#[derive(Debug, Clone)]
pub struct Particion {
    pub inicio: u64,
    pub tam: u64,
    pub esquema: Esquema,
    pub indice: usize,
}

impl Particion {
    /// El sub-slice de bytes de esta partición dentro del medio.
    fn slice<'a>(&self, datos: &'a [u8]) -> Result<&'a [u8], FsError> {
        let ini = self.inicio as usize;
        let fin = (self.inicio + self.tam) as usize;
        datos
            .get(ini..fin.min(datos.len()))
            .filter(|s| !s.is_empty())
            .ok_or(FsError::MedioInvalido("partición fuera del medio"))
    }
}

/// El FS detectado en un slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SistemaArchivos {
    Fat,
    Ext,
    Desconocido,
}

/// Olfatea qué FS hay al principio de un slice. Mira primero la magia ext
/// (0xEF53 a 1024+0x38), muy específica; si no, intenta construir el lector FAT
/// (un BPB válido es señal suficiente). El resto es `Desconocido` (swap, NTFS,
/// btrfs, vacío…).
pub fn detectar_fs(slice: &[u8]) -> SistemaArchivos {
    if let Ok(magia) = u16le(slice, 1024 + 0x38) {
        if magia == 0xEF53 {
            return SistemaArchivos::Ext;
        }
    }
    if LectorFat::nuevo(slice).is_ok() {
        return SistemaArchivos::Fat;
    }
    SistemaArchivos::Desconocido
}

/// Enumera las particiones del medio. Preferencia: GPT (si está la firma
/// `EFI PART`); si no, FS suelto (medio entero como una partición); si no, MBR.
/// El orden importa: una imagen FAT/ext suelta tiene `0x55AA` en el offset 510
/// que un parser MBR ingenuo confundiría con una tabla — por eso el FS suelto
/// se chequea ANTES que MBR.
pub fn tabla_particiones(datos: &[u8]) -> Result<Vec<Particion>, FsError> {
    if datos.len() >= 520 && &datos[512..520] == GPT_SIG {
        return parsear_gpt(datos);
    }
    if detectar_fs(datos) != SistemaArchivos::Desconocido {
        return Ok(alloc::vec![Particion {
            inicio: 0,
            tam: datos.len() as u64,
            esquema: Esquema::SinTabla,
            indice: 1,
        }]);
    }
    parsear_mbr(datos)
}

fn parsear_gpt(datos: &[u8]) -> Result<Vec<Particion>, FsError> {
    let entries_lba = u64le(datos, 512 + 72)?;
    let num = u32le(datos, 512 + 80)? as usize;
    let esz = u32le(datos, 512 + 84)? as usize;
    if esz < 128 {
        return Err(FsError::MedioInvalido("GPT: tamaño de entrada irreal"));
    }
    let base = (entries_lba * SECTOR) as usize;
    let mut parts = Vec::new();
    for i in 0..num {
        let off = match base.checked_add(i * esz) {
            Some(o) => o,
            None => break,
        };
        if off + esz > datos.len() {
            break;
        }
        // Tipo todo-ceros ⇒ entrada sin usar.
        if datos[off..off + 16].iter().all(|&b| b == 0) {
            continue;
        }
        let first = u64le(datos, off + 32)?;
        let last = u64le(datos, off + 40)?;
        if last < first {
            continue;
        }
        parts.push(Particion {
            inicio: first * SECTOR,
            tam: (last - first + 1) * SECTOR,
            esquema: Esquema::Gpt,
            indice: i + 1,
        });
    }
    Ok(parts)
}

fn parsear_mbr(datos: &[u8]) -> Result<Vec<Particion>, FsError> {
    if datos.len() < 512 {
        return Err(FsError::MedioInvalido("medio más corto que un sector MBR"));
    }
    let mut parts = Vec::new();
    for i in 0..4 {
        let off = 446 + i * 16;
        let tipo = datos[off + 4];
        if tipo == 0x00 || tipo == 0xEE {
            // 0x00 = vacía; 0xEE = MBR protectivo de un GPT (ya se intentó GPT).
            continue;
        }
        let first = u32le(datos, off + 8)? as u64;
        let nsec = u32le(datos, off + 12)? as u64;
        if first == 0 || nsec == 0 {
            continue;
        }
        let inicio = first * SECTOR;
        if inicio >= datos.len() as u64 {
            continue;
        }
        parts.push(Particion {
            inicio,
            tam: nsec * SECTOR,
            esquema: Esquema::Mbr,
            indice: i + 1,
        });
    }
    Ok(parts)
}

/// Absorbe UNA partición al grafo, despachando al lector según el FS detectado.
/// Devuelve el hash del árbol raíz de esa partición.
pub fn absorber_particion<E: Emisor>(
    datos: &[u8],
    p: &Particion,
    emisor: &mut E,
) -> Result<format::Hash, FsError> {
    let slice = p.slice(datos)?;
    match detectar_fs(slice) {
        SistemaArchivos::Fat => {
            let lector = LectorFat::nuevo(slice)?;
            absorber(&lector, emisor)
        }
        SistemaArchivos::Ext => {
            let lector = LectorExt4::nuevo(slice)?;
            absorber(&lector, emisor)
        }
        SistemaArchivos::Desconocido => {
            Err(FsError::MedioInvalido("FS de la partición no reconocido"))
        }
    }
}

/// Absorbe un DISPOSITIVO entero: un árbol top cuyas entradas son las
/// particiones con FS reconocido (cada una un subdirectorio `particionN` →
/// raíz de su FS). Las particiones de FS desconocido (swap, etc.) se omiten.
/// Devuelve el hash único que representa todo el disco. Determinista: el orden
/// de la tabla fija los nombres.
pub fn absorber_dispositivo<E: Emisor>(
    datos: &[u8],
    emisor: &mut E,
) -> Result<format::Hash, FsError> {
    let parts = tabla_particiones(datos)?;
    let mut entradas: Vec<format::EntradaArbol> = Vec::new();
    for p in &parts {
        let slice = match p.slice(datos) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if detectar_fs(slice) == SistemaArchivos::Desconocido {
            continue; // swap / FS no soportado: se salta
        }
        let hash = absorber_particion(datos, p, emisor)?;
        entradas.push(format::EntradaArbol {
            nombre: format!("particion{}", p.indice),
            modo: format::ModoEntrada::Directorio,
            hash,
        });
    }
    let objeto = format::objeto_arbol(entradas).map_err(FsError::Format)?;
    emisor.emitir(&objeto)
}
