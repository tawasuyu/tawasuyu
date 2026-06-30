//! Adapter [`Source`] sobre un **dispositivo de bloques crudo** — la mirada
//! SOBERANA de "manejar particiones". No hay `udisks2`, no hay `/run/media`, no
//! hay montaje del kernel: se LEE el dispositivo por bytes con `foreign-fs`
//! (FAT/ext sobre GPT/MBR) y se navega su sistema de archivos sin montarlo.
//!
//! Es el gemelo navegable del absorbedor de `foreign-fs`: donde aquél TRAGA el
//! medio al grafo content-addressed de wawa, éste lo NAVEGA en sitio, perezoso,
//! leyendo sólo los sectores que cada listado/lectura necesita —de modo que un
//! disco de 2 TB se explora sin cargarlo a RAM—.
//!
//! Jerarquía de navegación:
//!
//! ```text
//! @dispositivos                      (raíz sintética)
//!  └─ /dev/sdb        — Kingston…     (un dispositivo)
//!      ├─ particion1 · FAT            (una partición con FS reconocido)
//!      │   └─ … árbol del FS …
//!      └─ particion2 · ext
//!          └─ … árbol del FS …
//! ```
//!
//! Read-only por diseño (no implementa `SourceMut`): la frontera honesta de
//! `Source` deja deshabilitadas crear/borrar/renombrar. Escribir en un FS ajeno
//! sin driver es otra liga; esta fase sólo lee.

use std::io::{self};
use std::path::{Path, PathBuf};

use foreign_fs::particion::{
    detectar_fs_fuente, tabla_particiones_fuente, SistemaArchivos,
};
use foreign_fs::{ext4::LectorExt4, fat::LectorFat};
use foreign_fs::{Clase, FsError, FuenteArchivo, LectorFs, SubFuente};

use crate::{Node, NodeId, NodeKind, Source};

/// Id sintético de la raíz: contiene la lista de dispositivos.
const RAIZ: &str = "@dispositivos";

/// `true` si `id` pertenece a una [`DispositivosSource`] (su raíz, o un nodo
/// device/partición/archivo). Deja al front rutar una extracción cross-source
/// (leer del device, escribir en POSIX) sin conocer el tipo concreto de fuente.
pub fn es_id_de_dispositivo(id: &str) -> bool {
    id == RAIZ || id.starts_with("dev\u{1f}") || id.starts_with("fs\u{1f}")
}
/// Separador de campos del [`NodeId`]: el byte `US` (unit separator). No aparece
/// nunca en una ruta de dispositivo ni en un nombre de archivo, así que parte el
/// id sin ambigüedad.
const SEP: char = '\u{1f}';

/// Un medio de bloques enumerado: su ruta (`/dev/sdX` o una imagen de archivo)
/// y la metadata que `/sys/block` ofrece para pintarlo.
#[derive(Clone, Debug)]
pub struct DispositivoInfo {
    /// Ruta abrible: `/dev/sdb`, `/dev/nvme0n1`… o un `.img` para pruebas.
    pub ruta: PathBuf,
    /// Nombre corto del kernel (`sdb`) — para la fila.
    pub nombre: String,
    /// Tamaño en bytes, si se conoce.
    pub tam: Option<u64>,
    /// `true` si el kernel lo marca removible (USB, SD…).
    pub removible: bool,
    /// Modelo legible (`Kingston DataTraveler`), si `/sys` lo expone.
    pub modelo: Option<String>,
}

/// Fuente navegable de los dispositivos de bloques del sistema.
pub struct DispositivosSource {
    dispositivos: Vec<DispositivoInfo>,
}

impl DispositivosSource {
    /// Enumera los dispositivos de bloques reales del sistema vía `/sys/block`.
    /// Omite los virtuales (`loop`, `ram`, `zram`, `dm-`). En un sistema sin
    /// `/sys` (no-Linux) devuelve una lista vacía —la raíz queda sin hijos, sin
    /// reventar—.
    pub fn nueva() -> Self {
        Self { dispositivos: enumerar_sys_block() }
    }

    /// Construye la fuente con una lista explícita de dispositivos. Es la puerta
    /// para pruebas (apuntar a una imagen de archivo como si fuera un device) y
    /// para un front que ya tenga su propia enumeración.
    pub fn con_dispositivos(dispositivos: Vec<DispositivoInfo>) -> Self {
        Self { dispositivos }
    }

    /// Reconstruye una fuente capaz de LEER por `id` —un [`NodeId`] que esta
    /// fuente entregó—. Los ids `dev`/`fs` codifican la ruta del device, así que
    /// `children`/`read` no consultan la lista de dispositivos: basta sembrarla
    /// con el device del id. Es lo que deja a un worker (la cola de ops) extraer
    /// de un dispositivo sin compartir la fuente viva entre hilos.
    pub fn reconstruir_para(id: &NodeId) -> io::Result<Self> {
        let ruta = match decode(id)? {
            Loc::Dispositivo { ruta } => ruta,
            Loc::Ruta { ruta_dev, .. } => ruta_dev,
            Loc::Raiz => return Err(id_malo()),
        };
        let nombre = ruta
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Self::con_dispositivos(vec![DispositivoInfo {
            ruta,
            nombre,
            tam: None,
            removible: false,
            modelo: None,
        }]))
    }

    fn hijos_raiz(&self) -> Vec<Node> {
        self.dispositivos
            .iter()
            .map(|d| {
                let mut nombre = d.nombre.clone();
                if let Some(m) = &d.modelo {
                    nombre = format!("{nombre} — {m}");
                }
                if d.removible {
                    nombre.push_str(" ⏏");
                }
                let mut n = Node::new(encode_dev(&d.ruta), nombre, true)
                    .with_kind(NodeKind::Dir);
                if let Some(t) = d.tam {
                    n = n.with_size(t);
                }
                n
            })
            .collect()
    }

    fn hijos_dispositivo(&self, ruta_dev: &Path) -> io::Result<Vec<Node>> {
        let fuente = FuenteArchivo::abrir(ruta_dev)?;
        let parts = tabla_particiones_fuente(&fuente).map_err(fserr)?;
        let mut out = Vec::new();
        for p in parts {
            // Olfatea el FS de la partición SIN mover la fuente (la presta).
            let sub = SubFuente::nueva(&fuente, p.inicio, p.tam);
            let (etiqueta, navegable) = match detectar_fs_fuente(&sub) {
                SistemaArchivos::Fat => ("FAT", true),
                SistemaArchivos::Ext => ("ext", true),
                SistemaArchivos::Desconocido => ("sin FS reconocido", false),
            };
            let nombre = format!("particion{} · {}", p.indice, etiqueta);
            let mut n = Node::new(
                encode_ruta(ruta_dev, p.indice, "/"),
                nombre,
                navegable,
            )
            .with_kind(if navegable { NodeKind::Dir } else { NodeKind::File });
            n = n.with_size(p.tam);
            out.push(n);
        }
        Ok(out)
    }
}

impl Source for DispositivosSource {
    fn label(&self) -> String {
        "Dispositivos".into()
    }

    fn root(&self) -> Node {
        Node::new(RAIZ, "Dispositivos", true).with_kind(NodeKind::Synthetic)
    }

    fn children(&self, id: &NodeId) -> io::Result<Vec<Node>> {
        match decode(id)? {
            Loc::Raiz => Ok(self.hijos_raiz()),
            Loc::Dispositivo { ruta } => self.hijos_dispositivo(&ruta),
            Loc::Ruta { ruta_dev, indice, interna } => {
                con_particion(
                    &ruta_dev,
                    indice,
                    |l| listar_generico(l, &interna, &ruta_dev, indice),
                    |l| listar_generico(l, &interna, &ruta_dev, indice),
                )
            }
        }
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        match decode(id)? {
            Loc::Raiz | Loc::Dispositivo { .. } => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "un contenedor de dispositivos no tiene contenido leíble",
            )),
            Loc::Ruta { ruta_dev, indice, interna } => con_particion(
                &ruta_dev,
                indice,
                |l| leer_generico(l, &interna),
                |l| leer_generico(l, &interna),
            ),
        }
    }
}

// ── Despacho de partición + recorrido del FS ────────────────────────────────

/// Abre la partición `indice` de `ruta_dev`, detecta su FS y corre el cierre
/// correspondiente sobre el lector ya construido. Los dos cierres existen
/// porque FAT y ext son tipos de lector distintos; en la práctica ambos llaman
/// a la misma función genérica, así que no hay lógica duplicada.
fn con_particion<R>(
    ruta_dev: &Path,
    indice: usize,
    en_fat: impl FnOnce(&LectorFat<SubFuente<FuenteArchivo>>) -> io::Result<R>,
    en_ext: impl FnOnce(&LectorExt4<SubFuente<FuenteArchivo>>) -> io::Result<R>,
) -> io::Result<R> {
    let fuente = FuenteArchivo::abrir(ruta_dev)?;
    let parts = tabla_particiones_fuente(&fuente).map_err(fserr)?;
    let p = parts.into_iter().find(|p| p.indice == indice).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, format!("partición {indice} inexistente"))
    })?;
    let sub = SubFuente::nueva(fuente, p.inicio, p.tam);
    match detectar_fs_fuente(&sub) {
        SistemaArchivos::Fat => en_fat(&LectorFat::nuevo(sub).map_err(fserr)?),
        SistemaArchivos::Ext => en_ext(&LectorExt4::nuevo(sub).map_err(fserr)?),
        SistemaArchivos::Desconocido => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "el FS de la partición no es FAT ni ext",
        )),
    }
}

/// Resuelve una ruta interna (`/a/b/c`) a la manija de su nodo, caminando desde
/// la raíz del FS. Devuelve `(manija, es_directorio)`; `None` si algún
/// componente no existe o se intenta descender en un no-directorio.
fn resolver<L: LectorFs>(fs: &L, interna: &str) -> Option<(L::Manija, bool)> {
    let mut manija = fs.raiz();
    let mut es_dir = true;
    for comp in interna.split('/').filter(|c| !c.is_empty()) {
        if !es_dir {
            return None;
        }
        let entradas = fs.listar(&manija).ok()?;
        let ent = entradas.into_iter().find(|e| e.nombre == comp)?;
        manija = ent.manija;
        es_dir = matches!(ent.clase, Clase::Directorio);
    }
    Some((manija, es_dir))
}

fn listar_generico<L: LectorFs>(
    fs: &L,
    interna: &str,
    ruta_dev: &Path,
    indice: usize,
) -> io::Result<Vec<Node>> {
    let (manija, es_dir) = resolver(fs, interna)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("ruta inexistente: {interna}")))?;
    if !es_dir {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no es un directorio"));
    }
    let entradas = fs.listar(&manija).map_err(fserr)?;
    let mut out = Vec::with_capacity(entradas.len());
    for e in entradas {
        let hija = join_interna(interna, &e.nombre);
        let (cont, kind) = match e.clase {
            Clase::Directorio => (true, NodeKind::Dir),
            Clase::Symlink => (false, NodeKind::Symlink),
            Clase::Archivo { .. } => (false, NodeKind::File),
        };
        let mut n = Node::new(encode_ruta(ruta_dev, indice, &hija), e.nombre, cont)
            .with_kind(kind);
        if matches!(e.clase, Clase::Archivo { .. }) {
            if let Ok(sz) = fs.tamano_archivo(&e.manija) {
                n = n.with_size(sz);
            }
        }
        out.push(n);
    }
    // Orden presentable: contenedores primero, luego por nombre.
    out.sort_by(|a, b| b.is_container.cmp(&a.is_container).then(a.name.cmp(&b.name)));
    Ok(out)
}

fn leer_generico<L: LectorFs>(fs: &L, interna: &str) -> io::Result<Vec<u8>> {
    let (manija, es_dir) = resolver(fs, interna)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("ruta inexistente: {interna}")))?;
    if es_dir {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "un directorio no se lee como hoja"));
    }
    fs.leer_archivo(&manija).map_err(fserr)
}

// ── Codificación del NodeId ─────────────────────────────────────────────────

enum Loc {
    Raiz,
    Dispositivo { ruta: PathBuf },
    Ruta { ruta_dev: PathBuf, indice: usize, interna: String },
}

fn encode_dev(ruta: &Path) -> NodeId {
    format!("dev{SEP}{}", ruta.display())
}

fn encode_ruta(ruta_dev: &Path, indice: usize, interna: &str) -> NodeId {
    format!("fs{SEP}{}{SEP}{}{SEP}{}", ruta_dev.display(), indice, interna)
}

fn decode(id: &NodeId) -> io::Result<Loc> {
    if id == RAIZ {
        return Ok(Loc::Raiz);
    }
    let mut it = id.splitn(4, SEP);
    match it.next() {
        Some("dev") => {
            let ruta = it.next().ok_or_else(id_malo)?;
            Ok(Loc::Dispositivo { ruta: PathBuf::from(ruta) })
        }
        Some("fs") => {
            let ruta_dev = it.next().ok_or_else(id_malo)?;
            let indice = it
                .next()
                .ok_or_else(id_malo)?
                .parse()
                .map_err(|_| id_malo())?;
            // El 4º campo es la ruta interna completa (nunca contiene SEP).
            let interna = it.next().unwrap_or("/").to_string();
            Ok(Loc::Ruta { ruta_dev: PathBuf::from(ruta_dev), indice, interna })
        }
        _ => Err(id_malo()),
    }
}

fn id_malo() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "id de dispositivo inválido")
}

fn fserr(e: FsError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("foreign-fs: {e:?}"))
}

fn join_interna(base: &str, nombre: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{nombre}")
    } else {
        format!("{base}/{nombre}")
    }
}

// ── Enumeración de /sys/block (Linux) ───────────────────────────────────────

fn enumerar_sys_block() -> Vec<DispositivoInfo> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/block") else {
        return out;
    };
    for ent in rd.flatten() {
        let nombre = ent.file_name().to_string_lossy().into_owned();
        if nombre.starts_with("loop")
            || nombre.starts_with("ram")
            || nombre.starts_with("zram")
            || nombre.starts_with("dm-")
        {
            continue;
        }
        let base = ent.path();
        // `/sys/block/<dev>/size` está en sectores de 512 B (convención del
        // kernel, independiente del sector físico real).
        let tam = leer_u64(&base.join("size")).map(|s| s * 512);
        let removible = leer_u64(&base.join("removable")).unwrap_or(0) == 1;
        let modelo = std::fs::read_to_string(base.join("device/model"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        out.push(DispositivoInfo {
            ruta: Path::new("/dev").join(&nombre),
            nombre,
            tam,
            removible,
            modelo,
        });
    }
    out.sort_by(|a, b| a.nombre.cmp(&b.nombre));
    out
}

fn leer_u64(p: &Path) -> Option<u64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}
