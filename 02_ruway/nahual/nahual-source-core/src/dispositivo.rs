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

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use foreign_fs::particion::{
    detectar_fs_fuente, tabla_particiones_fuente, Particion, SistemaArchivos,
};
use foreign_fs::{ext4::LectorExt4, fat::LectorFat};
use foreign_fs::{Clase, FsError, FuenteArchivo, LectorFs, SubFuente};

/// Tope de bytes que [`DispositivosSource::read_preview`] lee de una hoja para
/// previsualizar: suficiente para discernir el tipo + cabeza de texto/hex, sin
/// volcar a RAM un video/ISO entero. Archivos grandes se EXTRAEN, no se preven.
const TOPE_PREVIEW: u64 = 16 * 1024 * 1024;

use crate::{Node, NodeId, NodeKind, Source};

/// Id sintético de la raíz: contiene la lista de dispositivos.
const RAIZ: &str = "@dispositivos";

/// `true` si `id` pertenece a una [`DispositivosSource`] (su raíz, o un nodo
/// device/partición/archivo). Deja al front rutar una extracción cross-source
/// (leer del device, escribir en POSIX) sin conocer el tipo concreto de fuente.
pub fn es_id_de_dispositivo(id: &str) -> bool {
    id == RAIZ || id.starts_with("dev\u{1f}") || id.starts_with("fs\u{1f}")
}

/// Qué se puede ABSORBER al grafo wawa desde un nodo de la fuente: un
/// dispositivo entero (árbol `particionN/`) o una partición concreta (su FS
/// directo). Un archivo/dir interno o la raíz `@dispositivos` no son objetivos.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjetivoAbsorcion {
    /// El dispositivo entero: `ruta` al device.
    Dispositivo(PathBuf),
    /// La partición `indice` (1-based) del device en `ruta`.
    Particion(PathBuf, usize),
}

/// Decide el [`ObjetivoAbsorcion`] de un `id`, o `None` si no es absorbible (un
/// archivo/dir dentro de un FS, o la raíz sintética).
pub fn objetivo_absorcion(id: &NodeId) -> Option<ObjetivoAbsorcion> {
    match decode(id).ok()? {
        Loc::Dispositivo { ruta } => Some(ObjetivoAbsorcion::Dispositivo(ruta)),
        Loc::Ruta { ruta_dev, indice, interna } if interna == "/" => {
            Some(ObjetivoAbsorcion::Particion(ruta_dev, indice))
        }
        _ => None,
    }
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

/// Cache por instancia: la fuente abierta y la tabla de particiones de cada
/// device, para no reabrir/re-parsear en cada `children`/`read`. Clave al
/// extraer un árbol: el device se abre UNA vez, no una por archivo. Seguro
/// porque la navegación es read-only —el device no cambia bajo nuestros pies—.
#[derive(Default)]
struct Cache {
    fuentes: HashMap<PathBuf, Arc<FuenteArchivo>>,
    tablas: HashMap<PathBuf, Arc<Vec<Particion>>>,
}

/// Fuente navegable de los dispositivos de bloques del sistema.
pub struct DispositivosSource {
    dispositivos: Vec<DispositivoInfo>,
    cache: Mutex<Cache>,
}

impl DispositivosSource {
    /// Enumera los dispositivos de bloques reales del sistema vía `/sys/block`.
    /// Omite los virtuales (`loop`, `ram`, `zram`, `dm-`). En un sistema sin
    /// `/sys` (no-Linux) devuelve una lista vacía —la raíz queda sin hijos, sin
    /// reventar—.
    pub fn nueva() -> Self {
        Self::con_dispositivos(enumerar_sys_block())
    }

    /// Construye la fuente con una lista explícita de dispositivos. Es la puerta
    /// para pruebas (apuntar a una imagen de archivo como si fuera un device) y
    /// para un front que ya tenga su propia enumeración.
    pub fn con_dispositivos(dispositivos: Vec<DispositivoInfo>) -> Self {
        Self { dispositivos, cache: Mutex::new(Cache::default()) }
    }

    /// La fuente abierta de `ruta_dev`, cacheada (se abre una sola vez).
    fn fuente_de(&self, ruta_dev: &Path) -> io::Result<Arc<FuenteArchivo>> {
        let mut c = self.cache.lock().map_err(|_| io::Error::other("cache envenenado"))?;
        if let Some(f) = c.fuentes.get(ruta_dev) {
            return Ok(f.clone());
        }
        let f = Arc::new(FuenteArchivo::abrir(ruta_dev)?);
        c.fuentes.insert(ruta_dev.to_path_buf(), f.clone());
        Ok(f)
    }

    /// La tabla de particiones de `ruta_dev`, cacheada.
    fn tabla_de(&self, ruta_dev: &Path) -> io::Result<Arc<Vec<Particion>>> {
        {
            let c = self.cache.lock().map_err(|_| io::Error::other("cache envenenado"))?;
            if let Some(t) = c.tablas.get(ruta_dev) {
                return Ok(t.clone());
            }
        }
        let f = self.fuente_de(ruta_dev)?;
        let t = Arc::new(tabla_particiones_fuente(&f).map_err(fserr)?);
        self.cache
            .lock()
            .map_err(|_| io::Error::other("cache envenenado"))?
            .tablas
            .insert(ruta_dev.to_path_buf(), t.clone());
        Ok(t)
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
        let fuente = self.fuente_de(ruta_dev)?;
        let parts = self.tabla_de(ruta_dev)?;
        let mut out = Vec::new();
        for p in parts.iter() {
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

impl DispositivosSource {
    /// Vuelca la hoja `id` a `w` por STREAMING: lee el archivo en ventanas
    /// (foreign-fs `leer_archivo_en`) sin materializarlo entero en RAM. Devuelve
    /// los bytes escritos. Es lo que deja EXTRAER un archivo grande (un video,
    /// una ISO) de un dispositivo sin OOM —a diferencia de [`Source::read`], que
    /// devuelve un `Vec` completo—.
    pub fn leer_a_escritor<W: Write>(&self, id: &NodeId, w: &mut W) -> io::Result<u64> {
        let (ruta_dev, indice, interna) = match decode(id)? {
            Loc::Ruta { ruta_dev, indice, interna } => (ruta_dev, indice, interna),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "sólo una hoja de un dispositivo se vuelca por streaming",
                ))
            }
        };
        // Inline (no `con_particion`): dos cierres capturando `&mut w` no
        // compilan aunque sólo corra uno; un solo match lo evita.
        let sub = self.ventana_de(&ruta_dev, indice)?;
        match detectar_fs_fuente(&sub) {
            SistemaArchivos::Fat => volcar_generico(&LectorFat::nuevo(sub).map_err(fserr)?, &interna, w),
            SistemaArchivos::Ext => volcar_generico(&LectorExt4::nuevo(sub).map_err(fserr)?, &interna, w),
            SistemaArchivos::Desconocido => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "el FS de la partición no es FAT ni ext",
            )),
        }
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
            Loc::Ruta { ruta_dev, indice, interna } => self.con_particion(
                &ruta_dev,
                indice,
                |l| listar_generico(l, &interna, &ruta_dev, indice),
                |l| listar_generico(l, &interna, &ruta_dev, indice),
            ),
        }
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        match decode(id)? {
            Loc::Raiz | Loc::Dispositivo { .. } => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "un contenedor de dispositivos no tiene contenido leíble",
            )),
            Loc::Ruta { ruta_dev, indice, interna } => self.con_particion(
                &ruta_dev,
                indice,
                |l| leer_generico(l, &interna),
                |l| leer_generico(l, &interna),
            ),
        }
    }

    /// Para PREVIEW: a diferencia de [`read`](Self::read) (archivo entero), lee
    /// a lo sumo [`TOPE_PREVIEW`] bytes por streaming. Evita el OOM al hacer
    /// clic sobre un video/ISO en un device — el preview sólo necesita la cabeza
    /// (discernir el tipo, texto/hex). Para abrirlo de verdad se EXTRAE.
    fn read_preview(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        match decode(id)? {
            Loc::Ruta { ruta_dev, indice, interna } => self.con_particion(
                &ruta_dev,
                indice,
                |l| leer_acotado(l, &interna, TOPE_PREVIEW),
                |l| leer_acotado(l, &interna, TOPE_PREVIEW),
            ),
            _ => self.read(id),
        }
    }
}

// ── Despacho de partición + recorrido del FS ────────────────────────────────

/// La ventana (`SubFuente`) de una partición sobre la fuente CACHEADA. Compartir
/// el `Arc` evita reabrir el device por llamada.
type VentanaParticion = SubFuente<Arc<FuenteArchivo>>;

impl DispositivosSource {
    /// Localiza la partición `indice` de `ruta_dev` (vía cache), construye su
    /// lector y corre el cierre correspondiente. Los dos cierres existen porque
    /// FAT y ext son tipos de lector distintos; en la práctica ambos llaman a la
    /// misma función genérica, así que no hay lógica duplicada.
    fn con_particion<R>(
        &self,
        ruta_dev: &Path,
        indice: usize,
        en_fat: impl FnOnce(&LectorFat<VentanaParticion>) -> io::Result<R>,
        en_ext: impl FnOnce(&LectorExt4<VentanaParticion>) -> io::Result<R>,
    ) -> io::Result<R> {
        let sub = self.ventana_de(ruta_dev, indice)?;
        match detectar_fs_fuente(&sub) {
            SistemaArchivos::Fat => en_fat(&LectorFat::nuevo(sub).map_err(fserr)?),
            SistemaArchivos::Ext => en_ext(&LectorExt4::nuevo(sub).map_err(fserr)?),
            SistemaArchivos::Desconocido => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "el FS de la partición no es FAT ni ext",
            )),
        }
    }

    /// La `SubFuente` de la partición `indice` sobre la fuente cacheada.
    fn ventana_de(&self, ruta_dev: &Path, indice: usize) -> io::Result<VentanaParticion> {
        let fuente = self.fuente_de(ruta_dev)?;
        let parts = self.tabla_de(ruta_dev)?;
        let p = parts.iter().find(|p| p.indice == indice).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("partición {indice} inexistente"))
        })?;
        Ok(SubFuente::nueva(fuente, p.inicio, p.tam))
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

/// Tamaño de ventana de streaming (256 KiB, = `foreign_fs::TAMANO_TROZO`): el
/// pico de RAM al extraer es una ventana, no el archivo entero.
const VENTANA: usize = 256 * 1024;

/// Resuelve `interna` a una hoja y la streamea a `w` por ventanas de [`VENTANA`].
fn volcar_generico<L: LectorFs, W: Write>(fs: &L, interna: &str, w: &mut W) -> io::Result<u64> {
    let (manija, es_dir) = resolver(fs, interna)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("ruta inexistente: {interna}")))?;
    if es_dir {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "un directorio no se vuelca como hoja"));
    }
    let mut buf = vec![0u8; VENTANA];
    let mut offset = 0u64;
    let mut total = 0u64;
    loop {
        let n = fs.leer_archivo_en(&manija, offset, &mut buf).map_err(fserr)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        offset += n as u64;
        total += n as u64;
    }
    Ok(total)
}

/// Lee a lo sumo `max` bytes de la hoja `interna` (por ventanas) a un `Vec`. Es
/// el preview acotado: la cabeza basta para discernir/ver texto sin OOM.
fn leer_acotado<L: LectorFs>(fs: &L, interna: &str, max: u64) -> io::Result<Vec<u8>> {
    let (manija, es_dir) = resolver(fs, interna)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("ruta inexistente: {interna}")))?;
    if es_dir {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "un directorio no se lee como hoja"));
    }
    let mut out = Vec::new();
    let mut buf = vec![0u8; VENTANA];
    let mut offset = 0u64;
    while offset < max {
        let pedir = core::cmp::min(VENTANA as u64, max - offset) as usize;
        let n = fs.leer_archivo_en(&manija, offset, &mut buf[..pedir]).map_err(fserr)?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        offset += n as u64;
    }
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
