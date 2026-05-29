// =============================================================================
//  foreign-fs :: fat — lector FAT12/16/32 de sólo-lectura sobre una `Fuente`
// -----------------------------------------------------------------------------
//  Opera sobre una `Fuente` de bloques —el dispositivo tal como wawa lo ve, SIN
//  montar ni driver de FS del kernel—. No requiere tener todo el volumen en RAM:
//  lee sólo lo que necesita (BPB, entradas de FAT, bloques de directorio, y la
//  ventana de archivo pedida por `leer_archivo_en`). El host satisface la
//  `Fuente` con un `&[u8]`; in-cage, con un syscall de lectura.
//
//  Cubre lo que un USB/partición EFI real trae: BPB clásico, tabla FAT, raíz
//  fija (FAT12/16) o raíz en cadena de clusters (FAT32), entradas 8.3 con sus
//  flags de minúsculas, y nombres largos VFAT (LFN, UCS-2). FAT no tiene bit de
//  ejecución ni enlaces simbólicos: todo archivo se absorbe como
//  `Clase::Archivo { ejecutable: false }`.
// =============================================================================

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{Clase, EntradaDir, Fuente, FsError, LectorFs};

/// El sabor de FAT, decidido por el número de clusters de datos (algoritmo
/// canónico de Microsoft): la ANCHURA de cada entrada de la tabla FAT y el
/// marcador de fin-de-cadena dependen de él.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoFat {
    Fat12,
    Fat16,
    Fat32,
}

/// Dónde empieza un nodo del volumen. La raíz de FAT12/16 es una región fija
/// fuera del área de datos (no es una cadena de clusters), de ahí la variante
/// dedicada. Un archivo/directorio vacío tiene cluster inicial 0 → `Vacio`.
#[derive(Debug, Clone, Copy)]
enum Inicio {
    RaizFija,
    Cluster(u32),
    Vacio,
}

/// Manija opaca de un nodo FAT: dónde empieza y, para archivos, su tamaño
/// exacto (la cadena de clusters se redondea al cluster; el tamaño recorta).
#[derive(Debug, Clone)]
pub struct ManijaFat {
    inicio: Inicio,
    tam: u32,
}

/// Lector de un volumen FAT sobre una `Fuente`.
pub struct LectorFat<F: Fuente> {
    fuente: F,
    tipo: TipoFat,
    bps: usize,             // bytes por sector
    spc: usize,             // sectores por cluster
    rsvd: usize,            // sectores reservados (antes de la 1ª FAT)
    num_fats: usize,        // número de copias de la FAT
    fat_sz: usize,          // sectores por FAT
    root_dir_sectors: usize,
    first_data_sector: usize,
    count_of_clusters: u32, // nº de clusters de datos (define el tipo)
    root_clus: u32,         // cluster raíz (FAT32)
}

#[inline]
fn u16le(d: &[u8], off: usize) -> Result<u16, FsError> {
    d.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or(FsError::MedioInvalido("BPB: lectura u16 fuera de rango"))
}

#[inline]
fn u32le(d: &[u8], off: usize) -> Result<u32, FsError> {
    d.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(FsError::MedioInvalido("BPB: lectura u32 fuera de rango"))
}

impl<F: Fuente> LectorFat<F> {
    /// Parsea el BPB y deja el lector listo. Rechaza un medio que no parezca
    /// FAT (sector/cluster no potencia de dos, FAT vacía, etc.) en vez de
    /// malinterpretar basura.
    pub fn nuevo(fuente: F) -> Result<Self, FsError> {
        if fuente.tamano() < 512 {
            return Err(FsError::MedioInvalido("medio más corto que un sector"));
        }
        let mut bpb = [0u8; 512];
        fuente.leer_en(0, &mut bpb)?;

        let bps = u16le(&bpb, 11)? as usize;
        let spc = bpb[13] as usize;
        let rsvd = u16le(&bpb, 14)? as usize;
        let num_fats = bpb[16] as usize;
        let root_ent_cnt = u16le(&bpb, 17)? as usize;
        let tot_sec16 = u16le(&bpb, 19)? as usize;
        let fat_sz16 = u16le(&bpb, 22)? as usize;
        let tot_sec32 = u32le(&bpb, 32)? as usize;
        let fat_sz32 = u32le(&bpb, 36)? as usize;

        // Validaciones mínimas: bps potencia de dos en [512, 4096], spc
        // potencia de dos ≥1, al menos una FAT.
        if !(512..=4096).contains(&bps) || !bps.is_power_of_two() {
            return Err(FsError::MedioInvalido("bytes/sector inválido"));
        }
        if spc == 0 || !spc.is_power_of_two() {
            return Err(FsError::MedioInvalido("sectores/cluster inválido"));
        }
        if num_fats == 0 {
            return Err(FsError::MedioInvalido("cero FATs"));
        }

        let fat_sz = if fat_sz16 != 0 { fat_sz16 } else { fat_sz32 };
        let tot_sec = if tot_sec16 != 0 { tot_sec16 } else { tot_sec32 };
        if fat_sz == 0 || tot_sec == 0 {
            return Err(FsError::MedioInvalido("FAT o total de sectores en cero"));
        }

        let root_dir_sectors = (root_ent_cnt * 32 + (bps - 1)) / bps;
        let first_data_sector = rsvd + num_fats * fat_sz + root_dir_sectors;
        if first_data_sector >= tot_sec {
            return Err(FsError::MedioInvalido("área de datos vacía"));
        }
        let data_sectors = tot_sec - first_data_sector;
        let count_of_clusters = (data_sectors / spc) as u32;

        let tipo = if count_of_clusters < 4085 {
            TipoFat::Fat12
        } else if count_of_clusters < 65525 {
            TipoFat::Fat16
        } else {
            TipoFat::Fat32
        };

        let root_clus = if tipo == TipoFat::Fat32 {
            u32le(&bpb, 44)?
        } else {
            0
        };

        // El medio debe contener al menos hasta el primer sector de datos.
        if (first_data_sector * bps) as u64 > fuente.tamano() {
            return Err(FsError::MedioInvalido("medio truncado: falta área de datos"));
        }

        Ok(LectorFat {
            fuente,
            tipo,
            bps,
            spc,
            rsvd,
            num_fats,
            fat_sz,
            root_dir_sectors,
            first_data_sector,
            count_of_clusters,
            root_clus,
        })
    }

    pub fn tipo(&self) -> TipoFat {
        self.tipo
    }

    /// El offset en bytes del primer sector de un cluster de datos.
    fn offset_cluster(&self, n: u32) -> usize {
        let sector = self.first_data_sector + (n as usize - 2) * self.spc;
        sector * self.bps
    }

    /// `true` si `n` es un cluster de datos direccionable.
    fn cluster_valido(&self, n: u32) -> bool {
        n >= 2 && n <= self.count_of_clusters + 1
    }

    /// Sigue un eslabón de la cadena FAT. `None` = fin de cadena (EOC) o
    /// cluster malo; `Some(siguiente)` continúa.
    fn siguiente_cluster(&self, n: u32) -> Result<Option<u32>, FsError> {
        let base = self.rsvd * self.bps;
        match self.tipo {
            TipoFat::Fat12 => {
                let off = base + (n as usize) + (n as usize) / 2;
                let mut b = [0u8; 2];
                self.fuente.leer_en(off as u64, &mut b)?;
                let raw = u16::from_le_bytes(b);
                let val = if n & 1 == 0 { raw & 0x0FFF } else { raw >> 4 };
                if val >= 0xFF8 || val == 0xFF7 {
                    Ok(None)
                } else {
                    Ok(Some(val as u32))
                }
            }
            TipoFat::Fat16 => {
                let off = base + (n as usize) * 2;
                let mut b = [0u8; 2];
                self.fuente.leer_en(off as u64, &mut b)?;
                let val = u16::from_le_bytes(b);
                if val >= 0xFFF8 || val == 0xFFF7 {
                    Ok(None)
                } else {
                    Ok(Some(val as u32))
                }
            }
            TipoFat::Fat32 => {
                let off = base + (n as usize) * 4;
                let mut b = [0u8; 4];
                self.fuente.leer_en(off as u64, &mut b)?;
                let val = u32::from_le_bytes(b) & 0x0FFF_FFFF;
                if val >= 0x0FFF_FFF8 || val == 0x0FFF_FFF7 {
                    Ok(None)
                } else {
                    Ok(Some(val))
                }
            }
        }
    }

    /// Lee los bytes de una cadena de clusters en un `Vec`. Si `limite` está
    /// presente, detiene la concatenación al alcanzarlo. Aborta ante un cluster
    /// fuera de rango o un ciclo (tope = total de clusters + 2): un FS corrupto
    /// no debe colgar el lector. Se usa para directorios (que se leen enteros);
    /// el contenido de archivo va por `leer_archivo_en` (streaming, O(1) RAM).
    fn leer_cadena(&self, inicio: u32, limite: Option<usize>) -> Result<Vec<u8>, FsError> {
        let tam_cluster = self.spc * self.bps;
        let tope = self.count_of_clusters as usize + 2;
        let mut salida = Vec::new();
        let mut actual = inicio;
        let mut contados = 0usize;
        loop {
            if !self.cluster_valido(actual) {
                return Err(FsError::Corrupto("cluster fuera de rango en la cadena"));
            }
            let antes = salida.len();
            salida.resize(antes + tam_cluster, 0);
            self.fuente
                .leer_en(self.offset_cluster(actual) as u64, &mut salida[antes..])?;
            contados += 1;
            if contados > tope {
                return Err(FsError::Corrupto("ciclo en la cadena de clusters"));
            }
            if let Some(lim) = limite {
                if salida.len() >= lim {
                    salida.truncate(lim);
                    break;
                }
            }
            match self.siguiente_cluster(actual)? {
                Some(sig) => actual = sig,
                None => break,
            }
        }
        if let Some(lim) = limite {
            salida.truncate(lim.min(salida.len()));
        }
        Ok(salida)
    }

    /// Bytes crudos de un directorio: la región de raíz fija (FAT12/16) o la
    /// cadena de clusters del directorio (FAT32 y todo subdirectorio).
    fn bytes_directorio(&self, inicio: &Inicio) -> Result<Vec<u8>, FsError> {
        match inicio {
            Inicio::Vacio => Ok(Vec::new()),
            Inicio::Cluster(c) => self.leer_cadena(*c, None),
            Inicio::RaizFija => {
                let inicio_sector = self.rsvd + self.num_fats * self.fat_sz;
                let off = inicio_sector * self.bps;
                let len = self.root_dir_sectors * self.bps;
                let mut buf = vec![0u8; len];
                self.fuente.leer_en(off as u64, &mut buf)?;
                Ok(buf)
            }
        }
    }
}

/// Atributo ATTR_LONG_NAME: la entrada es un fragmento de nombre largo (LFN).
const ATTR_LONG_NAME: u8 = 0x0F;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_VOLUME_ID: u8 = 0x08;

/// Extrae los 13 caracteres UCS-2 de una entrada LFN (posiciones 1, 14 y 28).
fn lfn_fragmento(entrada: &[u8]) -> [u16; 13] {
    let mut chars = [0u16; 13];
    let leer = |off: usize| u16::from_le_bytes([entrada[off], entrada[off + 1]]);
    for i in 0..5 {
        chars[i] = leer(1 + i * 2);
    }
    for i in 0..6 {
        chars[5 + i] = leer(14 + i * 2);
    }
    for i in 0..2 {
        chars[11 + i] = leer(28 + i * 2);
    }
    chars
}

/// Reconstruye el nombre 8.3 corto de una entrada, honrando los flags de
/// minúsculas de VFAT (offset 12, bit3 = base, bit4 = extensión). FAT guarda
/// el 8.3 en mayúsculas; estos flags recuperan un `archivo.txt` sin LFN.
fn nombre_corto(entrada: &[u8]) -> String {
    let flags = entrada[12];
    let base_min = flags & 0x08 != 0;
    let ext_min = flags & 0x10 != 0;

    let mut s = String::new();
    let aplicar = |dst: &mut String, b: u8, minus: bool| {
        let c = b as char;
        if minus {
            for m in c.to_lowercase() {
                dst.push(m);
            }
        } else {
            dst.push(c);
        }
    };

    let base = &entrada[0..8];
    let fin_base = base.iter().rposition(|&b| b != b' ').map(|i| i + 1).unwrap_or(0);
    for &b in &base[..fin_base] {
        aplicar(&mut s, b, base_min);
    }
    let ext = &entrada[8..11];
    let fin_ext = ext.iter().rposition(|&b| b != b' ').map(|i| i + 1).unwrap_or(0);
    if fin_ext > 0 {
        s.push('.');
        for &b in &ext[..fin_ext] {
            aplicar(&mut s, b, ext_min);
        }
    }
    s
}

/// Decodifica una secuencia UCS-2 (LFN ensamblado) a `String`, parando en el
/// terminador 0x0000 e ignorando el relleno 0xFFFF.
fn decodificar_lfn(unidades: &[u16]) -> String {
    let mut utiles: Vec<u16> = Vec::new();
    for &u in unidades {
        if u == 0x0000 {
            break;
        }
        if u == 0xFFFF {
            continue;
        }
        utiles.push(u);
    }
    core::char::decode_utf16(utiles.into_iter())
        .map(|r| r.unwrap_or('\u{FFFD}'))
        .collect()
}

impl<F: Fuente> LectorFs for LectorFat<F> {
    type Manija = ManijaFat;

    fn raiz(&self) -> ManijaFat {
        let inicio = match self.tipo {
            TipoFat::Fat32 => Inicio::Cluster(self.root_clus),
            _ => Inicio::RaizFija,
        };
        ManijaFat { inicio, tam: 0 }
    }

    fn listar(&self, dir: &ManijaFat) -> Result<Vec<EntradaDir<ManijaFat>>, FsError> {
        let bytes = self.bytes_directorio(&dir.inicio)?;
        let mut entradas = Vec::new();
        // Acumulador de fragmentos LFN: (orden, 13 chars). Se vacía al cerrar
        // una entrada corta o ante una entrada borrada.
        let mut lfn: Vec<(u8, [u16; 13])> = Vec::new();

        let mut i = 0;
        while i + 32 <= bytes.len() {
            let e = &bytes[i..i + 32];
            i += 32;
            let primero = e[0];
            if primero == 0x00 {
                break; // fin del directorio
            }
            if primero == 0xE5 {
                lfn.clear(); // entrada borrada: descarta LFN pendiente
                continue;
            }
            let attr = e[11];
            if attr == ATTR_LONG_NAME {
                let orden = e[0] & 0x1F;
                lfn.push((orden, lfn_fragmento(e)));
                continue;
            }
            if attr & ATTR_VOLUME_ID != 0 {
                lfn.clear(); // etiqueta de volumen: no es archivo
                continue;
            }
            // `.` y `..` no se listan.
            if e[0] == b'.' {
                lfn.clear();
                continue;
            }

            // Nombre: LFN ensamblado si lo hubo, si no el 8.3.
            let nombre = if lfn.is_empty() {
                nombre_corto(e)
            } else {
                lfn.sort_by_key(|(orden, _)| *orden);
                let mut unidades: Vec<u16> = Vec::new();
                for (_, frag) in &lfn {
                    unidades.extend_from_slice(frag);
                }
                decodificar_lfn(&unidades)
            };
            lfn.clear();

            let clus_hi = u16::from_le_bytes([e[20], e[21]]) as u32;
            let clus_lo = u16::from_le_bytes([e[26], e[27]]) as u32;
            let primer_cluster = (clus_hi << 16) | clus_lo;
            let tam = u32::from_le_bytes([e[28], e[29], e[30], e[31]]);
            let es_dir = attr & ATTR_DIRECTORY != 0;

            let inicio = if primer_cluster == 0 {
                Inicio::Vacio
            } else {
                Inicio::Cluster(primer_cluster)
            };

            let clase = if es_dir {
                Clase::Directorio
            } else {
                Clase::Archivo { ejecutable: false }
            };

            entradas.push(EntradaDir {
                nombre,
                clase,
                manija: ManijaFat { inicio, tam },
            });
        }
        Ok(entradas)
    }

    fn tamano_archivo(&self, archivo: &ManijaFat) -> Result<u64, FsError> {
        Ok(archivo.tam as u64)
    }

    fn leer_archivo_en(
        &self,
        archivo: &ManijaFat,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        let tam = archivo.tam as u64;
        if offset >= tam || buf.is_empty() {
            return Ok(0);
        }
        let cluster_inicial = match archivo.inicio {
            Inicio::Cluster(c) => c,
            // Vacío o raíz fija: no es un archivo con datos.
            _ => return Ok(0),
        };
        let cluster_size = (self.spc * self.bps) as u64;
        let max = core::cmp::min(buf.len() as u64, tam - offset) as usize;

        // Avanza por la cadena (sin materializarla: O(1) RAM) hasta el cluster
        // que contiene `offset`, luego copia la ventana cruzando clusters.
        let mut cluster = cluster_inicial;
        let mut base = 0u64; // offset lógico del inicio de `cluster`
        while base + cluster_size <= offset {
            match self.siguiente_cluster(cluster)? {
                Some(s) => {
                    cluster = s;
                    base += cluster_size;
                }
                None => return Ok(0),
            }
        }

        let mut leido = 0usize;
        loop {
            if !self.cluster_valido(cluster) {
                return Err(FsError::Corrupto("cluster fuera de rango al leer"));
            }
            let dentro = (offset + leido as u64 - base) as usize;
            let disponible = cluster_size as usize - dentro;
            let n = core::cmp::min(disponible, max - leido);
            let off_bytes = self.offset_cluster(cluster) as u64 + dentro as u64;
            self.fuente.leer_en(off_bytes, &mut buf[leido..leido + n])?;
            leido += n;
            if leido >= max {
                break;
            }
            match self.siguiente_cluster(cluster)? {
                Some(s) => {
                    cluster = s;
                    base += cluster_size;
                }
                None => break,
            }
        }
        Ok(leido)
    }
}
