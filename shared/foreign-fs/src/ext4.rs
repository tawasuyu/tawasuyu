// =============================================================================
//  foreign-fs :: ext4 — lector ext2/3/4 de sólo-lectura sobre bytes crudos
// -----------------------------------------------------------------------------
//  El FS nativo de Linux: la vía para que wawa absorba los datos VIEJOS del
//  usuario (su partición ext4) DESDE DENTRO, sin montar ni driver de FS del
//  kernel. Opera sobre un `&[u8]` que ES la imagen del volumen.
//
//  Cubre lo que un ext4 real trae (verificado contra `mke2fs -d`): superbloque,
//  descriptores de grupo de 32 ó 64 bytes (feature 64BIT), inodos de tamaño
//  variable, archivos por ÁRBOL DE EXTENTS (ext4) y por BLOQUES INDIRECTOS
//  (directo/simple/doble/triple — ext2/3), directorios lineales (incl. el
//  relleno htree/metadata_csum que se salta por `inode==0`), enlaces simbólicos
//  rápidos (inline en el inodo) y lentos (en bloques de datos), y el BIT DE
//  EJECUCIÓN leído de `i_mode` (ext4 sí lo preserva, a diferencia de FAT).
//
//  NO verifica checksums (metadata_csum): leer no los necesita. NO escribe.
// =============================================================================

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{Clase, EntradaDir, FsError, LectorFs};

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

/// Un extent ya decodificado: rango lógico de bloques → físico.
struct Extent {
    logico: u32,
    fisico: u64,
    len: u32,
    no_inicializado: bool,
}

/// Lector de un volumen ext2/3/4 montado sobre un slice de bytes.
pub struct LectorExt4<'a> {
    datos: &'a [u8],
    bs: usize,             // tamaño de bloque
    inode_size: usize,
    inodes_per_group: u32,
    desc_size: usize,      // 32 (clásico) ó 64 (feature 64BIT)
    gdt_start_block: u64,  // primer bloque de la tabla de descriptores de grupo
    is_64bit: bool,
}

impl<'a> LectorExt4<'a> {
    /// Parsea el superbloque y deja el lector listo. Rechaza un medio sin la
    /// magia ext o con geometría imposible.
    pub fn nuevo(datos: &'a [u8]) -> Result<Self, FsError> {
        if datos.len() < SB_OFFSET + 1024 {
            return Err(FsError::MedioInvalido("medio más corto que el superbloque"));
        }
        if rd_u16(datos, SB_OFFSET + 0x38)? != MAGIC {
            return Err(FsError::MedioInvalido("magia ext ausente (no es ext2/3/4)"));
        }
        let log_bs = rd_u32(datos, SB_OFFSET + 0x18)?;
        if log_bs > 6 {
            return Err(FsError::MedioInvalido("tamaño de bloque irreal"));
        }
        let bs = 1024usize << log_bs;
        let inode_size = {
            let s = rd_u16(datos, SB_OFFSET + 0x58)? as usize;
            if s == 0 {
                128
            } else {
                s
            }
        };
        let inodes_per_group = rd_u32(datos, SB_OFFSET + 0x28)?;
        if inodes_per_group == 0 {
            return Err(FsError::MedioInvalido("inodos por grupo en cero"));
        }
        let first_data_block = rd_u32(datos, SB_OFFSET + 0x14)?;
        let feat_incompat = rd_u32(datos, SB_OFFSET + 0x60)?;
        let is_64bit = feat_incompat & INCOMPAT_64BIT != 0;
        let desc_size = if is_64bit {
            let s = rd_u16(datos, SB_OFFSET + 0xFE)? as usize;
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
            datos,
            bs,
            inode_size,
            inodes_per_group,
            desc_size,
            gdt_start_block,
            is_64bit,
        })
    }

    /// El slice de un bloque físico, con chequeo de límites.
    fn bloque(&self, n: u64) -> Result<&[u8], FsError> {
        let off = (n as usize)
            .checked_mul(self.bs)
            .ok_or(FsError::Corrupto("offset de bloque desbordó"))?;
        self.datos
            .get(off..off + self.bs)
            .ok_or(FsError::Corrupto("bloque fuera del medio"))
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
        let itable_lo = rd_u32(self.datos, desc_off + 0x08)? as u64;
        let itable = if self.is_64bit && self.desc_size >= 64 {
            ((rd_u32(self.datos, desc_off + 0x28)? as u64) << 32) | itable_lo
        } else {
            itable_lo
        };

        let ioff = (itable as usize)
            .checked_mul(self.bs)
            .and_then(|b| b.checked_add(idx * self.inode_size))
            .ok_or(FsError::Corrupto("offset de inodo desbordó"))?;
        if ioff + self.inode_size > self.datos.len() {
            return Err(FsError::Corrupto("inodo fuera del medio"));
        }

        let mode = rd_u16(self.datos, ioff + 0x00)?;
        let size_lo = rd_u32(self.datos, ioff + 0x04)? as u64;
        let blocks_512 = rd_u32(self.datos, ioff + 0x1C)? as u64;
        let flags = rd_u32(self.datos, ioff + 0x20)?;
        // El tamaño alto (`i_size_high`) sólo aplica a archivos regulares.
        let size = if mode & S_IFMT == S_IFREG {
            (rd_u32(self.datos, ioff + 0x6C)? as u64) << 32 | size_lo
        } else {
            size_lo
        };
        let mut i_block = [0u8; 60];
        i_block.copy_from_slice(&self.datos[ioff + 0x28..ioff + 0x28 + 60]);

        Ok(Inodo {
            mode,
            size,
            flags,
            blocks_512,
            i_block,
        })
    }

    /// Recorre un nodo del árbol de extents (la raíz vive en `i_block`; los
    /// nodos internos, en bloques). Acumula los extents hoja EN ORDEN.
    fn recorrer_extents(&self, region: &[u8], salida: &mut Vec<Extent>) -> Result<(), FsError> {
        if rd_u16(region, 0)? != EXT_MAGIC {
            return Err(FsError::Corrupto("nodo de extents sin magia"));
        }
        let entradas = rd_u16(region, 2)? as usize;
        let prof = rd_u16(region, 6)?;
        for i in 0..entradas {
            let base = 12 + i * 12;
            if base + 12 > region.len() {
                return Err(FsError::Corrupto("entrada de extent truncada"));
            }
            if prof == 0 {
                let logico = rd_u32(region, base)?;
                let len_raw = rd_u16(region, base + 4)?;
                let inicio_hi = rd_u16(region, base + 6)? as u64;
                let inicio_lo = rd_u32(region, base + 8)? as u64;
                // len > 32768 marca un extent NO inicializado (su contenido es
                // cero); guardamos el rango para dejar ceros, no para copiar.
                let (len, no_inicializado) = if len_raw > 32768 {
                    (len_raw - 32768, true)
                } else {
                    (len_raw, false)
                };
                salida.push(Extent {
                    logico,
                    fisico: (inicio_hi << 32) | inicio_lo,
                    len: len as u32,
                    no_inicializado,
                });
            } else {
                let hoja_lo = rd_u32(region, base + 4)? as u64;
                let hoja_hi = rd_u16(region, base + 8)? as u64;
                let hoja = (hoja_hi << 32) | hoja_lo;
                let blq = self.bloque(hoja)?;
                self.recorrer_extents(blq, salida)?;
            }
        }
        Ok(())
    }

    /// Lista de bloques físicos para un archivo por bloques INDIRECTOS (ext2/3),
    /// en orden lógico, hasta `nbloques`. `0` = agujero (queda en ceros).
    fn lista_indirecta(&self, inodo: &Inodo, nbloques: usize) -> Result<Vec<u64>, FsError> {
        let mut out: Vec<u64> = Vec::new();
        let ptr = |i: usize| -> u64 {
            u32::from_le_bytes([
                inodo.i_block[i * 4],
                inodo.i_block[i * 4 + 1],
                inodo.i_block[i * 4 + 2],
                inodo.i_block[i * 4 + 3],
            ]) as u64
        };
        for i in 0..12 {
            if out.len() >= nbloques {
                break;
            }
            out.push(ptr(i));
        }
        for (slot, nivel) in [(12usize, 1u32), (13, 2), (14, 3)] {
            if out.len() >= nbloques {
                break;
            }
            self.expandir_indirecto(ptr(slot), nivel, &mut out, nbloques)?;
        }
        out.truncate(nbloques);
        Ok(out)
    }

    /// Expande un bloque indirecto de `nivel` (1=simple, 2=doble, 3=triple) en
    /// punteros de bloque de datos, en orden.
    fn expandir_indirecto(
        &self,
        blq: u64,
        nivel: u32,
        out: &mut Vec<u64>,
        necesarios: usize,
    ) -> Result<(), FsError> {
        if out.len() >= necesarios {
            return Ok(());
        }
        if blq == 0 {
            // Agujero a nivel de puntero indirecto: el rango entero es cero. Para
            // archivos no dispersos (los nuestros) no ocurre; lo dejamos corto.
            return Ok(());
        }
        let b = self.bloque(blq)?;
        let por_bloque = self.bs / 4;
        for i in 0..por_bloque {
            if out.len() >= necesarios {
                break;
            }
            let p = rd_u32(b, i * 4)? as u64;
            if nivel == 1 {
                out.push(p);
            } else {
                self.expandir_indirecto(p, nivel - 1, out, necesarios)?;
            }
        }
        Ok(())
    }

    /// Materializa el contenido completo de un inodo de archivo o directorio
    /// (exactamente `size` bytes), siguiendo extents o bloques indirectos.
    fn leer_contenido(&self, inodo: &Inodo) -> Result<Vec<u8>, FsError> {
        let n = inodo.size as usize;
        if n == 0 {
            return Ok(Vec::new());
        }
        let nbloques = (n + self.bs - 1) / self.bs;
        let mut salida = vec![0u8; n];

        if inodo.usa_extents() {
            let mut extents = Vec::new();
            self.recorrer_extents(&inodo.i_block, &mut extents)?;
            for ext in extents {
                if ext.no_inicializado {
                    continue; // queda en ceros
                }
                for j in 0..ext.len as u64 {
                    let lb = ext.logico as u64 + j;
                    let off = (lb as usize) * self.bs;
                    if off >= n {
                        break;
                    }
                    let src = self.bloque(ext.fisico + j)?;
                    let toma = core::cmp::min(self.bs, n - off);
                    salida[off..off + toma].copy_from_slice(&src[..toma]);
                }
            }
        } else {
            let bloques = self.lista_indirecta(inodo, nbloques)?;
            for (logico, &fisico) in bloques.iter().enumerate() {
                if fisico == 0 {
                    continue; // agujero
                }
                let off = logico * self.bs;
                if off >= n {
                    break;
                }
                let src = self.bloque(fisico)?;
                let toma = core::cmp::min(self.bs, n - off);
                salida[off..off + toma].copy_from_slice(&src[..toma]);
            }
        }
        Ok(salida)
    }
}

impl<'a> LectorFs for LectorExt4<'a> {
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

    fn leer_archivo(&self, archivo: &u32) -> Result<Vec<u8>, FsError> {
        let inodo = self.leer_inodo(*archivo)?;
        self.leer_contenido(&inodo)
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
