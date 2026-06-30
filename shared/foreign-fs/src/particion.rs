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
use crate::{absorber, Emisor, Fuente, FsError, SubFuente};

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

// ── Variantes sobre `Fuente` ────────────────────────────────────────────────
//  Las funciones de arriba toman el medio entero como `&[u8]` —cómodo para una
//  imagen residente—. Para un DISPOSITIVO real (que no entra en RAM) se leen
//  sólo los sectores necesarios a través de una `Fuente`. Misma lógica, mismos
//  offsets; la única diferencia es de dónde salen los bytes.

/// Lee `n` bytes desde `off` de una `Fuente` a un `Vec` fresco.
fn leer_vec<F: Fuente>(f: &F, off: u64, n: usize) -> Result<Vec<u8>, FsError> {
    let mut buf = alloc::vec![0u8; n];
    f.leer_en(off, &mut buf)?;
    Ok(buf)
}

/// Olfatea el FS al principio de una `Fuente` (espejo de [`detectar_fs`]): magia
/// ext (0xEF53 en 1024+0x38), si no un BPB FAT válido, si no `Desconocido`.
pub fn detectar_fs_fuente<F: Fuente>(f: &F) -> SistemaArchivos {
    if f.tamano() >= 1024 + 0x38 + 2 {
        let mut m = [0u8; 2];
        if f.leer_en(1024 + 0x38, &mut m).is_ok() && u16::from_le_bytes(m) == 0xEF53 {
            return SistemaArchivos::Ext;
        }
    }
    if LectorFat::nuevo(f).is_ok() {
        return SistemaArchivos::Fat;
    }
    SistemaArchivos::Desconocido
}

/// Tope de bytes que se leen para la tabla de entradas GPT (128 entradas ×
/// 128 B = 16 KiB es lo normal; el tope evita una asignación absurda si la
/// cabecera viene corrupta).
const MAX_TABLA_GPT: usize = 1024 * 1024;

/// Enumera las particiones de una `Fuente` (espejo de [`tabla_particiones`]),
/// leyendo sólo los sectores de cabecera. Preferencia idéntica: GPT, si no FS
/// suelto, si no MBR.
pub fn tabla_particiones_fuente<F: Fuente>(f: &F) -> Result<Vec<Particion>, FsError> {
    let total = f.tamano();
    // ¿Firma GPT en el LBA 1 (offset 512)?
    if total >= 520 {
        let mut sig = [0u8; 8];
        if f.leer_en(512, &mut sig).is_ok() && &sig == GPT_SIG {
            return parsear_gpt_fuente(f);
        }
    }
    if detectar_fs_fuente(f) != SistemaArchivos::Desconocido {
        return Ok(alloc::vec![Particion {
            inicio: 0,
            tam: total,
            esquema: Esquema::SinTabla,
            indice: 1,
        }]);
    }
    // MBR: basta el primer sector. NO se reusa `parsear_mbr` porque su guarda
    // `inicio >= datos.len()` está pensada para una imagen residente entera;
    // aquí el medio es el dispositivo completo, así que se acota contra `total`.
    if total < 512 {
        return Err(FsError::MedioInvalido("medio más corto que un sector MBR"));
    }
    let s = leer_vec(f, 0, 512)?;
    let mut parts = Vec::new();
    for i in 0..4 {
        let off = 446 + i * 16;
        let tipo = s[off + 4];
        if tipo == 0x00 || tipo == 0xEE {
            continue; // vacía / MBR protectivo de GPT (ya se intentó GPT)
        }
        let first = u32le(&s, off + 8)? as u64;
        let nsec = u32le(&s, off + 12)? as u64;
        if first == 0 || nsec == 0 {
            continue;
        }
        let inicio = first * SECTOR;
        if inicio >= total {
            continue; // la partición empieza fuera del dispositivo
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

fn parsear_gpt_fuente<F: Fuente>(f: &F) -> Result<Vec<Particion>, FsError> {
    // Cabecera GPT en el LBA 1: campos en 512+{72,80,84}.
    let cab = leer_vec(f, 512, 96)?;
    let entries_lba = u64le(&cab, 72)?;
    let num = u32le(&cab, 80)? as usize;
    let esz = u32le(&cab, 84)? as usize;
    if esz < 128 {
        return Err(FsError::MedioInvalido("GPT: tamaño de entrada irreal"));
    }
    let bytes = num
        .checked_mul(esz)
        .filter(|&b| b <= MAX_TABLA_GPT)
        .ok_or(FsError::MedioInvalido("GPT: tabla de entradas irreal"))?;
    let base = entries_lba * SECTOR;
    // Una sola lectura de toda la tabla de entradas; luego se parsea en RAM
    // con la misma rutina que la variante `&[u8]` (sub-slice virtual con base 0).
    let tabla = leer_vec(f, base, bytes)?;
    let mut parts = Vec::new();
    for i in 0..num {
        let off = i * esz;
        if off + esz > tabla.len() {
            break;
        }
        if tabla[off..off + 16].iter().all(|&b| b == 0) {
            continue;
        }
        let first = u64le(&tabla, off + 32)?;
        let last = u64le(&tabla, off + 40)?;
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

/// Absorbe UNA partición de una [`Fuente`] (gemelo de [`absorber_particion`],
/// pero perezoso: la presta como [`SubFuente`] y el lector lee sólo lo que
/// necesita). Es el camino para tragar una partición de un dispositivo real que
/// no entra en RAM.
pub fn absorber_particion_fuente<F: Fuente, E: Emisor>(
    f: &F,
    p: &Particion,
    emisor: &mut E,
) -> Result<format::Hash, FsError> {
    let sub = SubFuente::nueva(f, p.inicio, p.tam);
    match detectar_fs_fuente(&sub) {
        SistemaArchivos::Fat => absorber(&LectorFat::nuevo(sub)?, emisor),
        SistemaArchivos::Ext => absorber(&LectorExt4::nuevo(sub)?, emisor),
        SistemaArchivos::Desconocido => {
            Err(FsError::MedioInvalido("FS de la partición no reconocido"))
        }
    }
}

/// Absorbe un DISPOSITIVO entero de una [`Fuente`] (gemelo de
/// [`absorber_dispositivo`], perezoso). Cada partición con FS reconocido es un
/// subárbol `particionN`; las desconocidas (swap, NTFS…) se omiten. Determinista:
/// el orden de la tabla fija los nombres, idéntico a la variante `&[u8]`.
pub fn absorber_dispositivo_fuente<F: Fuente, E: Emisor>(
    f: &F,
    emisor: &mut E,
) -> Result<format::Hash, FsError> {
    let parts = tabla_particiones_fuente(f)?;
    let mut entradas: Vec<format::EntradaArbol> = Vec::new();
    for p in &parts {
        let sub = SubFuente::nueva(f, p.inicio, p.tam);
        if detectar_fs_fuente(&sub) == SistemaArchivos::Desconocido {
            continue;
        }
        let hash = absorber_particion_fuente(f, p, emisor)?;
        entradas.push(format::EntradaArbol {
            nombre: format!("particion{}", p.indice),
            modo: format::ModoEntrada::Directorio,
            hash,
        });
    }
    let objeto = format::objeto_arbol(entradas).map_err(FsError::Format)?;
    emisor.emitir(&objeto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SubFuente;

    fn put_u32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn put_u64(b: &mut [u8], o: usize, v: u64) {
        b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }

    /// MBR con una partición tipo 0x83 en LBA 2048, 100 sectores.
    fn mbr_sintetico() -> Vec<u8> {
        let mut b = alloc::vec![0u8; 512];
        let off = 446;
        b[off + 4] = 0x83; // tipo Linux
        put_u32(&mut b, off + 8, 2048); // primer LBA
        put_u32(&mut b, off + 12, 100); // nº sectores
        b[510] = 0x55;
        b[511] = 0xAA;
        b
    }

    /// GPT: firma + cabecera (entries_lba=2, num=4, esz=128) + una entrada
    /// usada (LBA 34..=133).
    fn gpt_sintetico() -> Vec<u8> {
        let mut b = alloc::vec![0u8; 1536];
        b[512..520].copy_from_slice(GPT_SIG);
        put_u64(&mut b, 512 + 72, 2); // entries_lba
        put_u32(&mut b, 512 + 80, 4); // num entradas
        put_u32(&mut b, 512 + 84, 128); // tamaño entrada
        let e = 1024; // 2 * 512
        b[e] = 1; // tipo GUID no-cero ⇒ entrada usada
        put_u64(&mut b, e + 32, 34); // first LBA
        put_u64(&mut b, e + 40, 133); // last LBA
        b
    }

    /// Medio falso «grande»: materializa sólo el sector 0; el resto son ceros y
    /// el tamaño es arbitrariamente grande. Modela el dispositivo real cuya
    /// tabla MBR coloca particiones muy lejos del inicio sin cargarlo a RAM.
    struct MedioFalso {
        sector0: Vec<u8>,
        total: u64,
    }
    impl Fuente for MedioFalso {
        fn tamano(&self) -> u64 {
            self.total
        }
        fn leer_en(&self, offset: u64, buf: &mut [u8]) -> Result<(), FsError> {
            let fin = offset + buf.len() as u64;
            if fin > self.total {
                return Err(FsError::Corrupto("fuera del medio falso"));
            }
            for (i, b) in buf.iter_mut().enumerate() {
                let o = offset as usize + i;
                *b = self.sector0.get(o).copied().unwrap_or(0);
            }
            Ok(())
        }
    }

    #[test]
    fn mbr_sobre_dispositivo_grande() {
        // La partición empieza en LBA 2048 (1 MiB), muy más allá del sector 0,
        // y el medio mide 200 MiB. La variante `Fuente` la enumera leyendo sólo
        // el primer sector — lo que la variante `&[u8]` no podría sin el medio
        // entero residente.
        let medio = MedioFalso {
            sector0: mbr_sintetico(),
            total: 200 * 1024 * 1024,
        };
        let f = tabla_particiones_fuente(&medio).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].inicio, 2048 * 512);
        assert_eq!(f[0].tam, 100 * 512);
        assert_eq!(f[0].esquema, Esquema::Mbr);
    }

    #[test]
    fn fuente_y_slice_coinciden_en_gpt() {
        let b = gpt_sintetico();
        let slice = b.as_slice();
        let a = tabla_particiones(slice).unwrap();
        let f = tabla_particiones_fuente(&slice).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].inicio, 34 * 512);
        assert_eq!(f[0].tam, (133 - 34 + 1) * 512);
        assert_eq!(f[0].esquema, Esquema::Gpt);
        assert_eq!(f[0].inicio, a[0].inicio);
        assert_eq!(f[0].tam, a[0].tam);
    }

    #[test]
    fn ext_suelto_es_sin_tabla() {
        // Buffer con la magia ext (0xEF53 en 1024+0x38) y nada más.
        let mut b = alloc::vec![0u8; 2048];
        b[1024 + 0x38..1024 + 0x38 + 2].copy_from_slice(&0xEF53u16.to_le_bytes());
        let slice = b.as_slice();
        assert_eq!(detectar_fs_fuente(&slice), SistemaArchivos::Ext);
        let f = tabla_particiones_fuente(&slice).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].esquema, Esquema::SinTabla);
        assert_eq!(f[0].inicio, 0);
        assert_eq!(f[0].tam, b.len() as u64);
    }

    #[test]
    fn subfuente_es_ventana() {
        let datos: Vec<u8> = (0..=255u8).collect();
        let slice = datos.as_slice();
        let sub = SubFuente::nueva(slice, 100, 50);
        assert_eq!(sub.tamano(), 50);
        let mut buf = [0u8; 10];
        sub.leer_en(5, &mut buf).unwrap();
        // offset 5 de la ventana = byte 105 del medio.
        assert_eq!(buf, [105, 106, 107, 108, 109, 110, 111, 112, 113, 114]);
        // Leer más allá del fin de la ventana falla (aunque el medio tenga más).
        let mut over = [0u8; 10];
        assert!(sub.leer_en(45, &mut over).is_err());
    }

    #[test]
    fn referencia_es_fuente() {
        // El blanket `impl Fuente for &F` deja olfatear sin mover el medio.
        let b = gpt_sintetico();
        let slice = b.as_slice();
        let r = &slice;
        assert_eq!(tabla_particiones_fuente(r).unwrap().len(), 1);
    }
}
