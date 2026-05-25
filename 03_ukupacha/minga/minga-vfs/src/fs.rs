//! Adaptador a `fuser`: el único módulo del crate acoplado a FUSE.
//!
//! Traduce el contrato [`NodeSource`] a la `Filesystem` trait. El
//! filesystem es de sólo lectura y de estructura fija (ver el layout en
//! la documentación del crate). Los inodos estáticos (raíz, `README`,
//! `roots/`, `cas/`) tienen números reservados; los archivos por hash
//! reciben un inodo dinámico la primera vez que se nombran, estable a
//! partir de ahí.
//!
//! `fuser` 0.15 despacha las peticiones de forma secuencial en un único
//! hilo de sesión, así que los métodos toman `&mut self` y mutamos los
//! mapas internos sin necesidad de locks.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::time::{Duration, SystemTime};

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyStatfs,
    Request,
};
use minga_core::ContentHash;

use crate::render::{render_sexp, render_source};
use crate::source::{reconstruct, NodeSource};

/// TTL de las respuestas cacheadas por el kernel. El contenido es
/// inmutable (direccionado por contenido), pero el *conjunto* de raíces
/// crece con cada ingest; 1 s es el compromiso habitual.
const TTL: Duration = Duration::from_secs(1);

// Inodos estáticos. Los dinámicos arrancan en INO_DYNAMIC_BASE.
const INO_ROOT: u64 = 1;
const INO_README: u64 = 2;
const INO_ROOTS_DIR: u64 = 3;
const INO_CAS_DIR: u64 = 4;
const INO_DYNAMIC_BASE: u64 = 16;

/// Contenido del archivo `/README` del propio montaje.
const README: &str = "\
Minga VFS — proyección de sólo lectura de un repositorio Minga.

Layout:
  roots/<hash>   Código fuente reconstruido (format normalizado) de
                 cada archivo ingerido. `ls roots/` los lista todos.
  cas/<hash>     S-expression del subárbol con ese hash. Este
                 directorio NO se lista (son demasiados nodos), pero
                 `cat cas/<hash>` resuelve cualquier hash conocido.

El hash es un BLAKE3 de 64 hex en minúsculas sobre la ESTRUCTURA
semántica del código: whitespace y comentarios no cuentan. Por eso
`roots/<hash>` es una reconstrucción normalizada, no el archivo
original byte-a-byte.

Filesystem de sólo lectura. Desmontar: fusermount -u <punto>.
";

/// Cuál de los dos directorios de hashes; determina el renderizado.
#[derive(Clone, Copy)]
enum Dir {
    /// `roots/` — código fuente reconstruido.
    Roots,
    /// `cas/` — S-expression del árbol.
    Cas,
}

/// Implementación de `fuser::Filesystem` sobre un [`NodeSource`].
pub struct MingaFs<S: NodeSource> {
    source: S,
    /// Siguiente inodo dinámico libre.
    next_ino: u64,
    /// `(inodo_padre, nombre)` → inodo dinámico, para que un mismo hash
    /// conserve su inodo entre llamadas.
    name_to_ino: HashMap<(u64, String), u64>,
    /// Inodo dinámico → contenido ya renderizado. Cachea el resultado
    /// del primer `lookup`/`read` de cada archivo.
    content: HashMap<u64, Vec<u8>>,
    /// Marca de tiempo uniforme para todos los atributos.
    epoch: SystemTime,
    uid: u32,
    gid: u32,
}

impl<S: NodeSource> MingaFs<S> {
    /// Construye el filesystem sobre `source`. Los archivos virtuales
    /// quedan a nombre del usuario y grupo del proceso, para que pueda
    /// leerlos sin `allow_other`.
    pub fn new(source: S) -> Self {
        Self {
            source,
            next_ino: INO_DYNAMIC_BASE,
            name_to_ino: HashMap::new(),
            content: HashMap::new(),
            epoch: SystemTime::now(),
            // SAFETY: getuid/getgid son siempre seguras, sin efectos.
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        }
    }

    /// Inodo dinámico para `(parent, name)`, asignándolo si es la
    /// primera vez que se ve.
    fn intern_ino(&mut self, parent: u64, name: &str) -> u64 {
        if let Some(&ino) = self.name_to_ino.get(&(parent, name.to_string())) {
            return ino;
        }
        let ino = self.next_ino;
        self.next_ino += 1;
        self.name_to_ino.insert((parent, name.to_string()), ino);
        ino
    }

    /// Resuelve un nombre bajo `roots/` o `cas/`: parsea el hash,
    /// reconstruye el nodo, lo renderiza según `dir`, cachea el
    /// contenido y devuelve `(inodo, tamaño)`. `None` si el nombre no
    /// es un hash válido o el nodo no está en el store.
    fn resolve(&mut self, dir: Dir, parent: u64, name: &str) -> Option<(u64, usize)> {
        let hash = parse_hash(name)?;
        let node = reconstruct(&self.source, &hash)?;
        let rendered = match dir {
            Dir::Roots => render_source(&node),
            Dir::Cas => render_sexp(&node),
        };
        let bytes = rendered.into_bytes();
        let size = bytes.len();
        let ino = self.intern_ino(parent, name);
        self.content.insert(ino, bytes);
        Some((ino, size))
    }

    fn dir_attr(&self, ino: u64) -> FileAttr {
        FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: self.epoch,
            mtime: self.epoch,
            ctime: self.epoch,
            crtime: self.epoch,
            kind: FileType::Directory,
            perm: 0o555,
            nlink: 2,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 512,
            flags: 0,
        }
    }

    fn file_attr(&self, ino: u64, size: usize) -> FileAttr {
        let size = size as u64;
        FileAttr {
            ino,
            size,
            blocks: size.div_ceil(512),
            atime: self.epoch,
            mtime: self.epoch,
            ctime: self.epoch,
            crtime: self.epoch,
            kind: FileType::RegularFile,
            perm: 0o444,
            nlink: 1,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 512,
            flags: 0,
        }
    }
}

impl<S: NodeSource> Filesystem for MingaFs<S> {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let Some(name) = name.to_str() else {
            reply.error(libc::ENOENT);
            return;
        };
        match parent {
            INO_ROOT => match name {
                "README" => reply.entry(&TTL, &self.file_attr(INO_README, README.len()), 0),
                "roots" => reply.entry(&TTL, &self.dir_attr(INO_ROOTS_DIR), 0),
                "cas" => reply.entry(&TTL, &self.dir_attr(INO_CAS_DIR), 0),
                _ => reply.error(libc::ENOENT),
            },
            INO_ROOTS_DIR | INO_CAS_DIR => {
                let dir = if parent == INO_ROOTS_DIR {
                    Dir::Roots
                } else {
                    Dir::Cas
                };
                match self.resolve(dir, parent, name) {
                    Some((ino, size)) => reply.entry(&TTL, &self.file_attr(ino, size), 0),
                    None => reply.error(libc::ENOENT),
                }
            }
            // Los archivos no tienen hijos.
            _ => reply.error(libc::ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        match ino {
            INO_ROOT | INO_ROOTS_DIR | INO_CAS_DIR => reply.attr(&TTL, &self.dir_attr(ino)),
            INO_README => reply.attr(&TTL, &self.file_attr(INO_README, README.len())),
            _ => match self.content.get(&ino) {
                Some(bytes) => {
                    let size = bytes.len();
                    reply.attr(&TTL, &self.file_attr(ino, size));
                }
                None => reply.error(libc::ENOENT),
            },
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let data: &[u8] = if ino == INO_README {
            README.as_bytes()
        } else {
            match self.content.get(&ino) {
                Some(bytes) => bytes.as_slice(),
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };
        let start = (offset.max(0) as usize).min(data.len());
        let end = start.saturating_add(size as usize).min(data.len());
        reply.data(&data[start..end]);
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        // Lista completa, incluidos `.` y `..`; el `offset` indica
        // desde qué entrada reanudar.
        let entries: Vec<(u64, FileType, String)> = match ino {
            INO_ROOT => vec![
                (INO_ROOT, FileType::Directory, ".".into()),
                (INO_ROOT, FileType::Directory, "..".into()),
                (INO_README, FileType::RegularFile, "README".into()),
                (INO_ROOTS_DIR, FileType::Directory, "roots".into()),
                (INO_CAS_DIR, FileType::Directory, "cas".into()),
            ],
            INO_ROOTS_DIR => {
                let mut v = vec![
                    (INO_ROOTS_DIR, FileType::Directory, ".".into()),
                    (INO_ROOT, FileType::Directory, "..".into()),
                ];
                for hash in self.source.roots() {
                    let name = hash.to_string();
                    let child = self.intern_ino(INO_ROOTS_DIR, &name);
                    v.push((child, FileType::RegularFile, name));
                }
                v
            }
            // `cas/` no se enumera: resuelve sólo por `lookup` directo.
            INO_CAS_DIR => vec![
                (INO_CAS_DIR, FileType::Directory, ".".into()),
                (INO_ROOT, FileType::Directory, "..".into()),
            ],
            _ => {
                reply.error(libc::ENOTDIR);
                return;
            }
        };

        for (i, (e_ino, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            // El offset del siguiente registro es `i + 1`.
            if reply.add(e_ino, (i + 1) as i64, kind, name) {
                break; // buffer del kernel lleno
            }
        }
        reply.ok();
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        // blocks, bfree, bavail, files, ffree, bsize, namelen, frsize.
        reply.statfs(0, 0, 0, 0, 0, 512, 255, 512);
    }
}

/// Parsea un nombre de archivo como un `ContentHash`: exactamente 64
/// dígitos hex en minúsculas (el format que produce `Display`).
fn parse_hash(name: &str) -> Option<ContentHash> {
    if name.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    let raw = name.as_bytes();
    for (i, slot) in bytes.iter_mut().enumerate() {
        let hi = hex_val(raw[2 * i])?;
        let lo = hex_val(raw[2 * i + 1])?;
        *slot = (hi << 4) | lo;
    }
    Some(ContentHash(bytes))
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hash_accepts_64_lowercase_hex() {
        let h = parse_hash(&"ab".repeat(32)).expect("64 hex válidos");
        assert_eq!(h.0, [0xab; 32]);
    }

    #[test]
    fn parse_hash_rejects_bad_length_and_chars() {
        assert!(parse_hash("abc").is_none());
        assert!(parse_hash(&"AB".repeat(32)).is_none(), "mayúsculas no");
        assert!(parse_hash(&"zz".repeat(32)).is_none(), "no-hex no");
    }

    #[test]
    fn parse_hash_roundtrips_display() {
        let original = ContentHash([0x3f; 32]);
        let back = parse_hash(&original.to_string()).expect("roundtrip");
        assert_eq!(original, back);
    }
}
