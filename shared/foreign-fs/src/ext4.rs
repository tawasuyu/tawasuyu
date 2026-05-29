// =============================================================================
//  foreign-fs :: ext4 — lector ext2/3/4 de sólo-lectura sobre una `Fuente`
// -----------------------------------------------------------------------------
//  El FS nativo de Linux: la vía para que wawa absorba los datos VIEJOS del
//  usuario (su partición ext4) DESDE DENTRO, sin montar ni driver de FS del
//  kernel. Opera sobre una `Fuente` de bloques —lee sólo lo que necesita, sin
//  tener todo el volumen en RAM—.
//
//  Cubre lo que un ext4 real trae (verificado contra `mke2fs -d`): superbloque,
//  descriptores de grupo de 32 ó 64 bytes (feature 64BIT), inodos de tamaño
//  variable, archivos por ÁRBOL DE EXTENTS (ext4) y por BLOQUES INDIRECTOS
//  (directo/simple/doble/triple — ext2/3), directorios lineales (incl. el
//  relleno htree/metadata_csum que se salta por `inode==0`), enlaces simbólicos
//  rápidos (inline en el inodo) y lentos (en bloques de datos), y el BIT DE
//  EJECUCIÓN leído de `i_mode`.
//
//  La resolución lógico→físico de un bloque (`bloque_logico`) navega el árbol de
//  extents o la cadena indirecta a demanda, con memoria O(1) por bloque (un
//  bloque-nodo/puntero por nivel), de modo que `leer_archivo_en` haga streaming
//  sin materializar la cadena completa. NO verifica checksums. NO escribe.
// =============================================================================

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{Clase, EntradaDir, Fuente, FsError, LectorFs};

/// El superbloque vive 1024 bytes dentro del volumen (tras el sector de boot).
const SB_OFFSET: usize = 1024;
/// Magia del superbloque ext (`s_magic`).
const MAGIC: u16 = 0xEF53;
/// Feature INCOMPAT_64BIT: descriptores de grupo de 64 B y punteros de 64 bits.
const INCOMPAT_64BIT: u32 = 0x0080;
/// Bandera del inodo `EXT4_EXTENTS_FL`: el archivo usa árbol de extents.
const EXTENTS_FL: u32 = 0x0008_0000;
/// Magia de una cabecera de nodo de extents (`eh_magic`).
const EXT_MAGIC: u16 = 0xF30A;
/// Máscara del tipo en `i_mode` y los tres tipos que nos importan.
const S_IFMT: u16 = 0xF000;
const S_IFREG: u16 = 0x8000;
const S_IFDIR: u16 = 0x4000;
const S_IFLNK: u16 = 0xA000;

#[inline]
fn rd_u16(d: &[u8], o: usize) -> Result<u16, FsError> {
    d.get(o..o + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or(FsError::Corrupto("u16 fuera del medio"))
}

#[inline]
fn rd_u32(d: &[u8], o: usize) -> Result<u32, FsError> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(FsError::Corrupto("u32 fuera del medio"))
}

/// Lo que extraemos de un inodo para recorrer/leer su contenido.
struct Inodo {
    mode: u16,
    size: u64,
    flags: u32,
    blocks_512: u64,  // `i_blocks`: sectores de 512 B asignados (0 ⇒ symlink rápido)
    i_block: [u8; 60], // 12 directos + 3 indirectos, o raíz del árbol de extents, o destino inline
}

impl Inodo {
    fn tipo(&self) -> u16 {
        self.mode & S_IFMT
    }
    fn usa_extents(&self) -> bool {
        self.flags & EXTENTS_FL != 0
    }
}

/// Lector de un volumen ext2/3/4 sobre una `Fuente`.
pub struct LectorExt4<F: Fuente> {
    fuente: F,
    bs: usize,             // tamaño de bloque
    inode_size: usize,
    inodes_per_group: u32,
    desc_size: usize,      // 32 (clásico) ó 64 (feature 64BIT)
    gdt_start_block: u64,  // primer bloque de la tabla de descriptores de grupo
    is_64bit: bool,
}

impl<F: Fuente> LectorExt4<F> {
    /// Parsea el superbloque y deja el lector listo. Rechaza un medio sin la
    /// magia ext o con geometría imposible.
    pub fn nuevo(fuente: F) -> Result<Self, FsError> {
        if fuente.tamano() < (SB_OFFSET + 1024) as u64 {
            return Err(FsError::MedioInvalido("medio más corto que el superbloque"));
        }
        let mut sb = [0u8; 1024];
        fuente.leer_en(SB_OFFSET as u64, &mut sb)?;

        if rd_u16(&sb, 0x38)? != MAGIC {
            return Err(FsError::MedioInvalido("magia ext ausente (no es ext2/3/4)"));
        }
        let log_bs = rd_u32(&sb, 0x18)?;
        if log_bs > 6 {
            return Err(FsError::MedioInvalido("tamaño de bloque irreal"));
        }
        let bs = 1024usize << log_bs;
        let inode_size = {
            let s = rd_u16(&sb, 0x58)? as usize;
            if s == 0 {
                128
            } else {
                s
            }
        };
        let inodes_per_group = rd_u32(&sb, 0x28)?;
        if inodes_per_group == 0 {
            return Err(FsError::MedioInvalido("inodos por grupo en cero"));
        }
        let first_data_block = rd_u32(&sb, 0x14)?;
        let feat_incompat = rd_u32(&sb, 0x60)?;
        let is_64bit = feat_incompat & INCOMPAT_64BIT != 0;
        let desc_size = if is_64bit {
            let s = rd_u16(&sb, 0xFE)? as usize;
            if s < 32 {
                32
            } else {
                s
            }
        } else {
            32
        };
        let gdt_start_block = first_data_block as u64 + 1;

        Ok(LectorExt4 {
            fuente,
            bs,
            inode_size,
            inodes_per_group,
            desc_size,
            gdt_start_block,
            is_64bit,
        })
    }

    /// Lee un bloque físico completo en un `Vec`, con chequeo de límites.
    fn bloque(&self, n: u64) -> Result<Vec<u8>, FsError> {
        let off = n
            .checked_mul(self.bs as u64)
            .ok_or(FsError::Corrupto("offset de bloque desbordó"))?;
        let mut b = vec![0u8; self.bs];
        self.fuente.leer_en(off, &mut b)?;
        Ok(b)
    }

    /// Lee el inodo número `ino` (1-based; la raíz es 2).
    fn leer_inodo(&self, ino: u32) -> Result<Inodo, FsError> {
        if ino == 0 {
            return Err(FsError::Corrupto("inodo 0 no existe"));
        }
        let g = (ino - 1) / self.inodes_per_group;
        let idx = ((ino - 1) % self.inodes_per_group) as usize;

        // Descriptor del grupo `g`: localiza la tabla de inodos.
        let desc_off = (self.gdt_start_block as usize) * self.bs + (g as usize) * self.desc_size;
        let mut desc = vec![0u8; self.desc_size];
        self.fuente.leer_en(desc_off as u64, &mut desc)?;
        let itable_lo = rd_u32(&desc, 0x08)? as u64;
        let itable = if self.is_64bit && self.desc_size >= 64 {
            ((rd_u32(&desc, 0x28)? as u64) << 32) | itable_lo
        } else {
            itable_lo
        };

        let ioff = (itable as usize)
            .checked_mul(self.bs)
            .and_then(|b| b.checked_add(idx * self.inode_size))
            .ok_or(FsError::Corrupto("offset de inodo desbordó"))?;
        let mut buf = vec![0u8; self.inode_size];
        self.fuente.leer_en(ioff as u64, &mut buf)?;

        let mode = rd_u16(&buf, 0x00)?;
        let size_lo = rd_u32(&buf, 0x04)? as u64;
        let blocks_512 = rd_u32(&buf, 0x1C)? as u64;
        let flags = rd_u32(&buf, 0x20)?;
        // El tamaño alto (`i_size_high`) sólo aplica a archivos regulares.
        let size = if mode & S_IFMT == S_IFREG {
            (rd_u32(&buf, 0x6C)? as u64) << 32 | size_lo
        } else {
            size_lo
        };
        let mut i_block = [0u8; 60];
        i_block.copy_from_slice(&buf[0x28..0x28 + 60]);

        Ok(Inodo {
            mode,
            size,
            flags,
            blocks_512,
            i_block,
        })
    }

    /// Resuelve el bloque FÍSICO del bloque LÓGICO `lblock` de un inodo. `None`
    /// = agujero / extent no inicializado (su contenido es cero). Navega a
    /// demanda con memoria O(1) por bloque.
    fn bloque_logico(&self, inodo: &Inodo, lblock: u64) -> Result<Option<u64>, FsError> {
        if inodo.usa_extents() {
            self.fisico_por_extents(inodo, lblock)
        } else {
            self.fisico_por_indirectos(inodo, lblock)
        }
    }

    /// Navega el árbol de extents (raíz en `i_block`, nodos internos en bloques)
    /// hasta el extent que cubre `lblock`.
    fn fisico_por_extents(&self, inodo: &Inodo, lblock: u64) -> Result<Option<u64>, FsError> {
        // `nodo` arranca como la raíz de 60 B en el inodo; en cada nivel índice
        // se reemplaza por el bloque hijo (un solo bloque residente a la vez).
        let mut nodo: Vec<u8> = inodo.i_block.to_vec();
        loop {
            if rd_u16(&nodo, 0)? != EXT_MAGIC {
                return Err(FsError::Corrupto("nodo de extents sin magia"));
            }
            let entradas = rd_u16(&nodo, 2)? as usize;
            let prof = rd_u16(&nodo, 6)?;
            if prof == 0 {
                for i in 0..entradas {
                    let base = 12 + i * 12;
                    let ee_block = rd_u32(&nodo, base)? as u64;
                    let len_raw = rd_u16(&nodo, base + 4)?;
                    let (len, uninit) = if len_raw > 32768 {
                        ((len_raw - 32768) as u64, true)
                    } else {
                        (len_raw as u64, false)
                    };
                    if lblock >= ee_block && lblock < ee_block + len {
                        if uninit {
                            return Ok(None);
                        }
                        let hi = rd_u16(&nodo, base + 6)? as u64;
                        let lo = rd_u32(&nodo, base + 8)? as u64;
                        let fisico = (hi << 32) | lo;
                        return Ok(Some(fisico + (lblock - ee_block)));
                    }
                }
                return Ok(None); // ningún extent cubre lblock → agujero
            }
            // Nodo índice: elegir el último hijo con `ei_block <= lblock`.
            let mut elegido: Option<u64> = None;
            for i in 0..entradas {
                let base = 12 + i * 12;
                let ei_block = rd_u32(&nodo, base)? as u64;
                if ei_block <= lblock {
                    let lo = rd_u32(&nodo, base + 4)? as u64;
                    let hi = rd_u16(&nodo, base + 8)? as u64;
                    elegido = Some((hi << 32) | lo);
                } else {
                    break;
                }
            }
            match elegido {
                Some(hijo) => nodo = self.bloque(hijo)?,
                None => return Ok(None),
            }
        }
    }

    /// Resuelve `lblock` por el esquema clásico de bloques indirectos
    /// (12 directos + simple + doble + triple).
    fn fisico_por_indirectos(&self, inodo: &Inodo, lblock: u64) -> Result<Option<u64>, FsError> {
        let per = (self.bs / 4) as u64;
        let ptr_directo = |i: usize| -> u64 {
            u32::from_le_bytes([
                inodo.i_block[i * 4],
                inodo.i_block[i * 4 + 1],
                inodo.i_block[i * 4 + 2],
                inodo.i_block[i * 4 + 3],
            ]) as u64
        };
        if lblock < 12 {
            return Ok(no_cero(ptr_directo(lblock as usize)));
        }
        let mut l = lblock - 12;
        if l < per {
            return self.ptr_indirecto(ptr_directo(12), l);
        }
        l -= per;
        if l < per * per {
            let mid = self.seguir(ptr_directo(13), l / per)?;
            return self.ptr_indirecto(mid, l % per);
        }
        l -= per * per;
        let i1 = l / (per * per);
        let resto = l % (per * per);
        let l1 = self.seguir(ptr_directo(14), i1)?;
        let l2 = self.seguir(l1, resto / per)?;
        self.ptr_indirecto(l2, resto % per)
    }

    /// Lee el puntero `idx` de un bloque de punteros (`0` si el bloque es un
    /// agujero), devolviéndolo crudo (sin interpretar `0` como agujero — eso lo
    /// decide el caller).
    fn seguir(&self, blq: u64, idx: u64) -> Result<u64, FsError> {
        if blq == 0 {
            return Ok(0);
        }
        let b = self.bloque(blq)?;
        Ok(rd_u32(&b, (idx as usize) * 4)? as u64)
    }

    /// Como `seguir`, pero interpreta `0` (en el bloque de punteros o como
    /// resultado) como agujero → `None`.
    fn ptr_indirecto(&self, blq: u64, idx: u64) -> Result<Option<u64>, FsError> {
        if blq == 0 {
            return Ok(None);
        }
        Ok(no_cero(self.seguir(blq, idx)?))
    }

    /// Materializa el contenido COMPLETO de un inodo (exactamente `size` bytes),
    /// bloque a bloque. Lo usan los directorios (que se leen enteros) y el
    /// destino de un symlink lento; el contenido de ARCHIVO va por
    /// `leer_archivo_en` (streaming).
    fn leer_contenido(&self, inodo: &Inodo) -> Result<Vec<u8>, FsError> {
        let n = inodo.size as usize;
        if n == 0 {
            return Ok(Vec::new());
        }
        let bs = self.bs;
        let nbloques = (n + bs - 1) / bs;
        let mut salida = vec![0u8; n];
        for lb in 0..nbloques as u64 {
            let off = lb as usize * bs;
            let toma = core::cmp::min(bs, n - off);
            if let Some(fb) = self.bloque_logico(inodo, lb)? {
                self.fuente
                    .leer_en(fb * bs as u64, &mut salida[off..off + toma])?;
            } // agujero → queda en ceros
        }
        Ok(salida)
    }
}

/// `Some(p)` salvo que `p == 0` (agujero) → `None`.
#[inline]
fn no_cero(p: u64) -> Option<u64> {
    if p == 0 {
        None
    } else {
        Some(p)
    }
}

impl<F: Fuente> LectorFs for LectorExt4<F> {
    /// La manija es el número de inodo.
    type Manija = u32;

    fn raiz(&self) -> u32 {
        2 // el inodo raíz de ext es siempre el 2
    }

    fn listar(&self, dir: &u32) -> Result<Vec<EntradaDir<u32>>, FsError> {
        let inodo = self.leer_inodo(*dir)?;
        if inodo.tipo() != S_IFDIR {
            return Err(FsError::Corrupto("listar sobre un no-directorio"));
        }
        let bytes = self.leer_contenido(&inodo)?;
        let mut entradas = Vec::new();

        let mut i = 0usize;
        while i + 8 <= bytes.len() {
            let inode = rd_u32(&bytes, i)?;
            let rec_len = rd_u16(&bytes, i + 4)? as usize;
            if rec_len < 8 {
                break; // registro inválido / fin defensivo
            }
            let name_len = bytes[i + 6] as usize; // feature filetype: name_len es u8

            if inode != 0 && name_len > 0 && i + 8 + name_len <= bytes.len() {
                let nombre_bytes = &bytes[i + 8..i + 8 + name_len];
                if nombre_bytes != b"." && nombre_bytes != b".." {
                    let cinodo = self.leer_inodo(inode)?;
                    let clase = match cinodo.tipo() {
                        S_IFREG => Some(Clase::Archivo {
                            ejecutable: cinodo.mode & 0o111 != 0,
                        }),
                        S_IFDIR => Some(Clase::Directorio),
                        S_IFLNK => Some(Clase::Symlink),
                        _ => None, // devices/fifos/sockets: no son código, se ignoran
                    };
                    if let Some(clase) = clase {
                        entradas.push(EntradaDir {
                            nombre: String::from_utf8_lossy(nombre_bytes).into_owned(),
                            clase,
                            manija: inode,
                        });
                    }
                }
            }
            i += rec_len;
        }
        Ok(entradas)
    }

    fn tamano_archivo(&self, archivo: &u32) -> Result<u64, FsError> {
        Ok(self.leer_inodo(*archivo)?.size)
    }

    fn leer_archivo_en(&self, archivo: &u32, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let inodo = self.leer_inodo(*archivo)?;
        let tam = inodo.size;
        if offset >= tam || buf.is_empty() {
            return Ok(0);
        }
        let bs = self.bs as u64;
        let max = core::cmp::min(buf.len() as u64, tam - offset) as usize;
        let mut leido = 0usize;
        while leido < max {
            let pos = offset + leido as u64;
            let lblock = pos / bs;
            let dentro = (pos % bs) as usize;
            let n = core::cmp::min(self.bs - dentro, max - leido);
            match self.bloque_logico(&inodo, lblock)? {
                Some(fb) => {
                    self.fuente
                        .leer_en(fb * bs + dentro as u64, &mut buf[leido..leido + n])?;
                }
                None => {
                    // Agujero: ceros.
                    for byte in &mut buf[leido..leido + n] {
                        *byte = 0;
                    }
                }
            }
            leido += n;
        }
        Ok(leido)
    }

    fn destino_symlink(&self, enlace: &u32) -> Result<String, FsError> {
        let inodo = self.leer_inodo(*enlace)?;
        // Symlink RÁPIDO: sin bloques asignados, el destino vive inline en los
        // 60 bytes de `i_block`. Symlink LENTO: el destino está en bloques de
        // datos, como un archivo.
        let bytes = if inodo.blocks_512 == 0 {
            let n = core::cmp::min(inodo.size as usize, 60);
            inodo.i_block[..n].to_vec()
        } else {
            self.leer_contenido(&inodo)?
        };
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}
