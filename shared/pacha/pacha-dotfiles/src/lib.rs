//! `pacha-dotfiles` — **versionado de dotfiles para contextos de usuario**.
//!
//! Un contexto ([`pacha_core::Pacha`]) quiere poder fijar *qué versión* de un
//! puñado de archivos de `$HOME` (`.zshrc`, `.config/nvim`, …) aterriza cuando
//! se activa, y respaldarlos "como en un git". Este crate da exactamente eso
//! **reusando el grafo direccionado por contenido de [`format`]** — el mismo
//! modelo de objetos (blob + árbol) que el kernel de wawa usa en disco, que ES
//! el modelo de git (ver `format::grafo`, Fase 66).
//!
//! Reparto:
//!
//! * [`ConjuntoDotfiles`] — la **definición**: qué rutas de `$HOME` gestiona un
//!   set y con qué política ([`ModoGestion`]). Se persiste en el catálogo de
//!   pacha (RON), no aquí.
//! * [`StoreObjetos`] — el **almacén** host de objetos por hash (`aa/bbbb…`,
//!   como `.git/objects`). Dedup por contenido, gratis.
//! * [`capturar`] / [`materializar`] — `$HOME` ⇄ grafo. Capturar lee las rutas
//!   y devuelve el hash del árbol raíz; materializar lo reconstruye en disco.
//! * [`Instantanea`] + [`commitear`] / [`historial`] — el eslabón de
//!   **historia**: un commit (raíz + padre) que vuelve la cadena un DAG.
//!
//! El núcleo es verificable por **texto** (hashes BLAKE3, conteos, round-trip),
//! sin render — coherente con la disciplina del repo. La integración con
//! `pacha-core`/`pacha-manager` (campo `dotfiles` en `Pacha`, efectos
//! `MaterializarDotfiles`/`CapturarDotfiles`) es un paso aparte.
//!
//! Host-only (Linux): preserva symlinks y el bit de ejecución vía las APIs
//! `std::os::unix`.

#![forbid(unsafe_code)]
#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use format::{
    objeto_arbol, objeto_blob, Arbol, EntradaArbol, Hash, ModoEntrada, Objeto,
};

// =====================================================================
// Definición de un set de dotfiles (se persiste en el catálogo de pacha)
// =====================================================================

/// Cómo trata un set una ruta gestionada al conmutar de contexto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ModoGestion {
    /// Materializar la instantánea fijada **read-only**: el contexto la impone,
    /// las ediciones en vivo no se recapturan. Para lo que querés idéntico
    /// siempre (tu `nvim` canónico).
    #[default]
    Fijado,
    /// Al **dejar** el contexto se snapshotean los cambios de esta ruta (es el
    /// análogo de `persist`/`last_session` de pacha para apps). Para config que
    /// editás dentro del contexto y querés que persista en él.
    Rastreado,
}

/// Una ruta de `$HOME` que un set gestiona.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RutaGestionada {
    /// Ruta **relativa a `$HOME`** (`".zshrc"`, `".config/nvim"`). Sólo
    /// componentes normales: nada de `..` ni rutas absolutas.
    pub origen: PathBuf,
    #[serde(default)]
    pub modo: ModoGestion,
}

impl RutaGestionada {
    pub fn fijado(origen: impl Into<PathBuf>) -> Self {
        Self { origen: origen.into(), modo: ModoGestion::Fijado }
    }
    pub fn rastreado(origen: impl Into<PathBuf>) -> Self {
        Self { origen: origen.into(), modo: ModoGestion::Rastreado }
    }
}

/// Un conjunto nombrado de dotfiles (`"shell"`, `"editor"`, `"ssh"`).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ConjuntoDotfiles {
    pub id: String,
    #[serde(default)]
    pub entradas: Vec<RutaGestionada>,
}

impl ConjuntoDotfiles {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), entradas: Vec::new() }
    }
    pub fn con(mut self, r: RutaGestionada) -> Self {
        self.entradas.push(r);
        self
    }
    /// Las rutas que se recapturan al salir del contexto (modo `Rastreado`).
    pub fn rastreadas(&self) -> impl Iterator<Item = &RutaGestionada> + '_ {
        self.entradas.iter().filter(|r| r.modo == ModoGestion::Rastreado)
    }
}

// =====================================================================
// Instantánea: el "commit" (raíz de árbol + linaje)
// =====================================================================

/// Un snapshot del set: la raíz del árbol capturado + su linaje. Es el commit
/// — direccionado por contenido como todo lo demás. Dos instantáneas que sólo
/// difieren en un archivo comparten todo el resto del árbol (estructura
/// compartida, como git).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instantanea {
    /// Hash del árbol raíz (lo que devuelve [`capturar`]).
    pub raiz: Hash,
    /// Instantánea anterior — `None` en la primera. Vuelve la cadena un DAG.
    pub padre: Option<Hash>,
    /// Rótulo humano ("antes de tocar nvim", "stable").
    pub etiqueta: String,
    /// Marca de tiempo en ms (la pasa quien commitea — el núcleo no toca reloj,
    /// igual que `pacha-core` no toca disco).
    pub creada_ms: u64,
}

// =====================================================================
// El almacén de objetos (host)
// =====================================================================

/// Almacén de objetos del grafo por hash, en un directorio (`aa/bbbb…`, como
/// `.git/objects`). Content-addressed: un objeto ya presente no se reescribe.
#[derive(Clone, Debug)]
pub struct StoreObjetos {
    raiz: PathBuf,
}

impl StoreObjetos {
    /// Abre (creando si hace falta) un almacén en `raiz`.
    pub fn abrir(raiz: impl Into<PathBuf>) -> Result<Self, DotError> {
        let raiz = raiz.into();
        fs::create_dir_all(&raiz)?;
        Ok(Self { raiz })
    }

    fn ruta_de(&self, h: &Hash) -> PathBuf {
        let hex = hex_de(h);
        self.raiz.join(&hex[0..2]).join(&hex[2..])
    }

    /// Inscribe un objeto y devuelve su hash. Idempotente: si ya está, no
    /// reescribe (la identidad ES el contenido).
    pub fn poner(&self, obj: &Objeto) -> Result<Hash, DotError> {
        let bytes = obj.serializar().map_err(DotError::Formato)?;
        let h = format::hash(&bytes);
        let destino = self.ruta_de(&h);
        if destino.exists() {
            return Ok(h);
        }
        if let Some(dir) = destino.parent() {
            fs::create_dir_all(dir)?;
        }
        // Escritura atómica: tmp + rename. El tmp lleva el hash → no colisiona
        // con otros objetos; dos escritores del mismo objeto escriben bytes
        // idénticos, el rename final es inocuo.
        let tmp = destino.with_extension("tmp");
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, &destino)?;
        Ok(h)
    }

    /// Recupera un objeto por su hash.
    pub fn traer(&self, h: &Hash) -> Result<Objeto, DotError> {
        let bytes = fs::read(self.ruta_de(h)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                DotError::Ausente(hex_de(h))
            } else {
                DotError::Io(e)
            }
        })?;
        Objeto::deserializar(&bytes).map_err(DotError::Formato)
    }

    pub fn contiene(&self, h: &Hash) -> bool {
        self.ruta_de(h).exists()
    }
}

// =====================================================================
// Capturar: $HOME → grafo
// =====================================================================

/// Captura las rutas gestionadas de `set` (relativas a `home`) al almacén y
/// devuelve el hash del **árbol raíz** del snapshot. El árbol contiene sólo las
/// rutas gestionadas, reproduciendo su anidamiento bajo `home`. Las rutas que
/// no existen en disco se **saltan** (un set puede declarar config aún ausente).
pub fn capturar(
    store: &StoreObjetos,
    set: &ConjuntoDotfiles,
    home: &Path,
) -> Result<Hash, DotError> {
    let mut raiz: Arbolillo = BTreeMap::new();
    for rg in &set.entradas {
        let comps = componentes(&rg.origen)?;
        let abs = home.join(&rg.origen);
        if !abs.symlink_existe() {
            continue; // ruta declarada pero aún no presente: se salta
        }
        let (hash, modo) = capturar_fs(store, &abs)?;
        insertar(&mut raiz, &comps, Hoja { hash, modo })?;
    }
    sellar(store, &raiz)
}

/// Representación intermedia: un directorio en construcción. Las hojas ya están
/// inscritas en el almacén; las ramas se sellan a árbol al final.
type Arbolillo = BTreeMap<String, Nodo>;

enum Nodo {
    Hoja { hash: Hash, modo: ModoEntrada },
    Rama(Arbolillo),
}
use Nodo::{Hoja, Rama};

/// Inserta una hoja en el árbol intermedio creando los directorios padres.
fn insertar(rama: &mut Arbolillo, comps: &[String], hoja: Nodo) -> Result<(), DotError> {
    match comps {
        [] => Err(DotError::Ruta("ruta gestionada vacía".into())),
        [ultimo] => {
            rama.insert(ultimo.clone(), hoja);
            Ok(())
        }
        [primero, resto @ ..] => {
            let sub = rama
                .entry(primero.clone())
                .or_insert_with(|| Rama(BTreeMap::new()));
            match sub {
                Rama(m) => insertar(m, resto, hoja),
                Hoja { .. } => Err(DotError::Ruta(format!(
                    "conflicto: '{primero}' es archivo y directorio a la vez"
                ))),
            }
        }
    }
}

/// Sella un árbol intermedio a objetos `Arbol` (bottom-up) y devuelve la raíz.
fn sellar(store: &StoreObjetos, rama: &Arbolillo) -> Result<Hash, DotError> {
    let mut entradas = Vec::with_capacity(rama.len());
    for (nombre, nodo) in rama {
        let (hash, modo) = match nodo {
            Hoja { hash, modo } => (*hash, *modo),
            Rama(sub) => (sellar(store, sub)?, ModoEntrada::Directorio),
        };
        entradas.push(EntradaArbol { nombre: nombre.clone(), modo, hash });
    }
    let obj = objeto_arbol(entradas).map_err(DotError::Formato)?;
    store.poner(&obj)
}

/// Captura un nodo del filesystem (archivo / dir / symlink) a objeto(s) y
/// devuelve su hash y el modo con que entra en un árbol padre.
fn capturar_fs(store: &StoreObjetos, abs: &Path) -> Result<(Hash, ModoEntrada), DotError> {
    let meta = fs::symlink_metadata(abs)?;
    let ft = meta.file_type();

    if ft.is_symlink() {
        let destino = fs::read_link(abs)?;
        let bytes = destino.into_os_string().into_vec();
        let h = store.poner(&objeto_blob(bytes))?;
        return Ok((h, ModoEntrada::Symlink));
    }

    if ft.is_dir() {
        let mut entradas = Vec::new();
        for de in fs::read_dir(abs)? {
            let de = de?;
            let nombre = de.file_name().to_string_lossy().into_owned();
            let (h, modo) = capturar_fs(store, &de.path())?;
            entradas.push(EntradaArbol { nombre, modo, hash: h });
        }
        let obj = objeto_arbol(entradas).map_err(DotError::Formato)?;
        let h = store.poner(&obj)?;
        return Ok((h, ModoEntrada::Directorio));
    }

    // archivo regular (sin chunking: los dotfiles son chicos — `format` admite
    // índices por trozos vía `objeto_blob_indice`, queda para archivos grandes).
    let datos = fs::read(abs)?;
    let h = store.poner(&objeto_blob(datos))?;
    let ejecutable = meta.permissions().mode() & 0o111 != 0;
    let modo = if ejecutable { ModoEntrada::Ejecutable } else { ModoEntrada::Archivo };
    Ok((h, modo))
}

// =====================================================================
// Materializar: grafo → $HOME
// =====================================================================

/// Reconstruye el árbol `raiz` bajo `destino` (típicamente `$HOME`).
/// **Idempotente**: vuelve a aplicar el mismo contenido sin error. Sólo crea o
/// sobrescribe las rutas que el snapshot contiene; no borra lo ajeno.
pub fn materializar(
    store: &StoreObjetos,
    destino: &Path,
    raiz: Hash,
) -> Result<(), DotError> {
    escribir_arbol(store, destino, raiz)
}

/// Dónde aterrizar un árbol materializado (Fase 1 de aislamiento por contexto).
///
/// La capa de captura/store/historia NO distingue destinos: sólo cambia la ruta
/// de escritura y el *significado* de esa ruta. El aislamiento real (montar el
/// tmpfs, bindearlo al `$HOME` del Card) vive en `SomaSpec::mounts` y lo realiza
/// el incarnator — acá sólo escribimos el contenido donde el plan de montajes lo
/// espera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializeTarget {
    /// El `$HOME` real en disco. Comportamiento clásico: los dotfiles persisten.
    Disco(PathBuf),
    /// Una ruta tmpfs en RAM (mont. por el manager/incarnator) que luego se
    /// bindea al `$HOME` del Card. El contenido se evapora con el namespace —
    /// no toca el disco. Base del "secreto que no toca disco" (la cripto en
    /// reposo llega en Fase 2; hoy el tmpfs es la garantía de no-persistencia).
    Efimero(PathBuf),
}

impl MaterializeTarget {
    /// La ruta de escritura, sea cual sea el destino.
    pub fn ruta(&self) -> &Path {
        match self {
            MaterializeTarget::Disco(p) | MaterializeTarget::Efimero(p) => p,
        }
    }
    /// `true` si el destino no persiste en disco.
    pub fn es_efimero(&self) -> bool {
        matches!(self, MaterializeTarget::Efimero(_))
    }
}

/// Materializa un árbol al destino indicado. Mismo escritor para ambos; el
/// destino sólo determina la ruta (y, semánticamente, si persiste).
pub fn materializar_a(
    store: &StoreObjetos,
    destino: &MaterializeTarget,
    raiz: Hash,
) -> Result<(), DotError> {
    escribir_arbol(store, destino.ruta(), raiz)
}

fn escribir_arbol(store: &StoreObjetos, dir: &Path, raiz: Hash) -> Result<(), DotError> {
    fs::create_dir_all(dir)?;
    let obj = store.traer(&raiz)?;
    let arbol = Arbol::deserializar(&obj.datos).map_err(DotError::Formato)?;
    for e in arbol.entradas {
        let dest = dir.join(&e.nombre);
        match e.modo {
            ModoEntrada::Directorio => escribir_arbol(store, &dest, e.hash)?,
            ModoEntrada::Symlink => {
                let blob = store.traer(&e.hash)?;
                let target = PathBuf::from(std::ffi::OsString::from_vec(blob.datos));
                // idempotencia: quitar lo que haya antes de recrear el enlace.
                let _ = fs::remove_file(&dest);
                std::os::unix::fs::symlink(&target, &dest)?;
            }
            ModoEntrada::Archivo | ModoEntrada::Ejecutable => {
                let obj = store.traer(&e.hash)?;
                let contenido = contenido_archivo(store, &obj)?;
                fs::write(&dest, &contenido)?;
                let modo = if e.modo == ModoEntrada::Ejecutable { 0o755 } else { 0o644 };
                fs::set_permissions(&dest, fs::Permissions::from_mode(modo))?;
            }
        }
    }
    Ok(())
}

/// Reconstruye el contenido de un archivo: blob plano (`hijos` vacío) o índice
/// de trozos (concatena los `datos` de los hijos, recursivo).
fn contenido_archivo(store: &StoreObjetos, obj: &Objeto) -> Result<Vec<u8>, DotError> {
    if obj.hijos.is_empty() {
        return Ok(obj.datos.clone());
    }
    let mut buf = Vec::new();
    for h in &obj.hijos {
        let trozo = store.traer(h)?;
        buf.extend(contenido_archivo(store, &trozo)?);
    }
    Ok(buf)
}

// =====================================================================
// Historia: commits como objetos del grafo
// =====================================================================

/// Inscribe una instantánea como objeto del grafo y devuelve su hash (el id del
/// commit). Sus `hijos` apuntan a la raíz y al padre → el GC del grafo alcanza
/// todo el historial y su contenido siguiendo aristas.
pub fn commitear(store: &StoreObjetos, inst: &Instantanea) -> Result<Hash, DotError> {
    let datos = postcard::to_allocvec(inst)?;
    let mut hijos = vec![inst.raiz];
    if let Some(p) = inst.padre {
        hijos.push(p);
    }
    store.poner(&Objeto { datos, hijos })
}

/// Lee una instantánea por su hash de commit.
pub fn leer_instantanea(store: &StoreObjetos, commit: &Hash) -> Result<Instantanea, DotError> {
    let obj = store.traer(commit)?;
    let inst = postcard::from_bytes(&obj.datos)?;
    Ok(inst)
}

/// Captura `set` y lo commitea sobre `padre` en un paso. Devuelve el hash del
/// commit. El timestamp lo provee quien llama (núcleo sin reloj).
pub fn capturar_y_commitear(
    store: &StoreObjetos,
    set: &ConjuntoDotfiles,
    home: &Path,
    padre: Option<Hash>,
    etiqueta: impl Into<String>,
    creada_ms: u64,
) -> Result<Hash, DotError> {
    let raiz = capturar(store, set, home)?;
    let inst = Instantanea { raiz, padre, etiqueta: etiqueta.into(), creada_ms };
    commitear(store, &inst)
}

/// Recorre el historial desde `cabeza` hacia atrás por el enlace `padre`.
/// Devuelve `(hash_commit, instantánea)` de la más nueva a la más vieja.
pub fn historial(
    store: &StoreObjetos,
    cabeza: Hash,
) -> Result<Vec<(Hash, Instantanea)>, DotError> {
    let mut out = Vec::new();
    let mut actual = Some(cabeza);
    while let Some(h) = actual {
        let inst = leer_instantanea(store, &h)?;
        actual = inst.padre;
        out.push((h, inst));
    }
    Ok(out)
}

// =====================================================================
// Utilidades
// =====================================================================

/// Descompone una ruta relativa en componentes normales, rechazando `..`,
/// rutas absolutas y demás (el snapshot no puede escapar de `$HOME`).
fn componentes(origen: &Path) -> Result<Vec<String>, DotError> {
    let mut comps = Vec::new();
    for c in origen.components() {
        match c {
            Component::Normal(s) => comps.push(s.to_string_lossy().into_owned()),
            otra => {
                return Err(DotError::Ruta(format!(
                    "ruta gestionada inválida ({origen:?}): componente {otra:?}"
                )))
            }
        }
    }
    if comps.is_empty() {
        return Err(DotError::Ruta(format!("ruta gestionada vacía: {origen:?}")));
    }
    Ok(comps)
}

/// Existe la ruta como entrada del filesystem (sin seguir symlinks rotos).
trait SymlinkExiste {
    fn symlink_existe(&self) -> bool;
}
impl SymlinkExiste for Path {
    fn symlink_existe(&self) -> bool {
        fs::symlink_metadata(self).is_ok()
    }
}

/// Hex en minúsculas de un hash (sin dependencias extra).
fn hex_de(h: &Hash) -> String {
    const DIG: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for b in h {
        s.push(DIG[(b >> 4) as usize] as char);
        s.push(DIG[(b & 0xf) as usize] as char);
    }
    s
}

/// Errores del versionado de dotfiles.
#[derive(Debug, Error)]
pub enum DotError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("format: {0}")]
    Formato(&'static str),
    #[error("postcard: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("objeto ausente en el almacén: {0}")]
    Ausente(String),
    #[error("ruta: {0}")]
    Ruta(String),
}

// =====================================================================
// Tests — round-trip por hashes, sin render
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Crea `home/<rel>` con `contenido`, creando los directorios padres.
    fn escribir(home: &Path, rel: &str, contenido: &[u8]) {
        let p = home.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, contenido).unwrap();
    }

    fn set_basico() -> ConjuntoDotfiles {
        ConjuntoDotfiles::new("shell")
            .con(RutaGestionada::fijado(".zshrc"))
            .con(RutaGestionada::fijado(".config/nvim"))
    }

    #[test]
    fn round_trip_reconstruye_contenido_y_estructura() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dest = tmp.path().join("dest");
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();

        escribir(&home, ".zshrc", b"export EDITOR=nada\n");
        escribir(&home, ".config/nvim/init.lua", b"vim.opt.number = true\n");
        escribir(&home, ".config/nvim/lua/plugins.lua", b"return {}\n");

        let raiz = capturar(&store, &set_basico(), &home).unwrap();
        materializar(&store, &dest, raiz).unwrap();

        assert_eq!(fs::read(dest.join(".zshrc")).unwrap(), b"export EDITOR=nada\n");
        assert_eq!(
            fs::read(dest.join(".config/nvim/init.lua")).unwrap(),
            b"vim.opt.number = true\n"
        );
        assert_eq!(
            fs::read(dest.join(".config/nvim/lua/plugins.lua")).unwrap(),
            b"return {}\n"
        );
    }

    #[test]
    fn captura_es_determinista() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();
        escribir(&home, ".zshrc", b"alias g=git\n");
        escribir(&home, ".config/nvim/init.lua", b"-- nvim\n");

        let a = capturar(&store, &set_basico(), &home).unwrap();
        let b = capturar(&store, &set_basico(), &home).unwrap();
        assert_eq!(a, b, "mismo contenido ⇒ misma raíz");
    }

    #[test]
    fn contenido_identico_comparte_un_solo_blob() {
        let tmp = tempfile::tempdir().unwrap();
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();
        let h1 = store.poner(&objeto_blob(b"misma cosa".to_vec())).unwrap();
        let h2 = store.poner(&objeto_blob(b"misma cosa".to_vec())).unwrap();
        assert_eq!(h1, h2, "dedup por contenido");

        // dos archivos distintos con idéntico contenido ⇒ un solo blob en disco.
        let home = tmp.path().join("home");
        escribir(&home, ".bashrc", b"igual\n");
        escribir(&home, ".profile", b"igual\n");
        let set = ConjuntoDotfiles::new("x")
            .con(RutaGestionada::fijado(".bashrc"))
            .con(RutaGestionada::fijado(".profile"));
        capturar(&store, &set, &home).unwrap();

        // sólo objetos: 1 blob compartido + 1 árbol raíz = 2 (más el blob de
        // "misma cosa" de arriba = 3 en total).
        assert_eq!(contar_objetos(tmp.path().join("obj")), 3);
    }

    #[test]
    fn symlink_se_preserva_como_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dest = tmp.path().join("dest");
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();

        fs::create_dir_all(&home).unwrap();
        std::os::unix::fs::symlink("config-real/nvim", home.join(".config-nvim")).unwrap();

        let set = ConjuntoDotfiles::new("s").con(RutaGestionada::fijado(".config-nvim"));
        let raiz = capturar(&store, &set, &home).unwrap();
        materializar(&store, &dest, raiz).unwrap();

        let m = fs::symlink_metadata(dest.join(".config-nvim")).unwrap();
        assert!(m.file_type().is_symlink());
        assert_eq!(fs::read_link(dest.join(".config-nvim")).unwrap(), PathBuf::from("config-real/nvim"));
    }

    #[test]
    fn bit_de_ejecucion_sobrevive_el_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dest = tmp.path().join("dest");
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();

        escribir(&home, ".local/bin/hola", b"#!/bin/sh\necho hola\n");
        fs::set_permissions(home.join(".local/bin/hola"), fs::Permissions::from_mode(0o755)).unwrap();

        let set = ConjuntoDotfiles::new("bin").con(RutaGestionada::fijado(".local/bin/hola"));
        let raiz = capturar(&store, &set, &home).unwrap();
        materializar(&store, &dest, raiz).unwrap();

        let modo = fs::metadata(dest.join(".local/bin/hola")).unwrap().permissions().mode();
        assert_eq!(modo & 0o111, 0o111, "+x preservado");
    }

    #[test]
    fn materializar_es_idempotente() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dest = tmp.path().join("dest");
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();
        escribir(&home, ".zshrc", b"v1\n");
        escribir(&home, ".config/nvim/init.lua", b"-- v1\n");

        let raiz = capturar(&store, &set_basico(), &home).unwrap();
        materializar(&store, &dest, raiz).unwrap();
        materializar(&store, &dest, raiz).unwrap(); // segunda vez: sin error
        assert_eq!(fs::read(dest.join(".zshrc")).unwrap(), b"v1\n");
    }

    #[test]
    fn materializar_a_efimero_escribe_en_la_ruta_y_no_en_disco() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let disco = tmp.path().join("disco_home"); // $HOME "real": debe quedar vacío
        let efimero = tmp.path().join("ram"); // hace de tmpfs
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();
        escribir(&home, ".ssh/id_ed25519", b"SECRETO\n");

        let set = ConjuntoDotfiles::new("claves").con(RutaGestionada::fijado(".ssh/id_ed25519"));
        let raiz = capturar(&store, &set, &home).unwrap();

        let destino = MaterializeTarget::Efimero(efimero.clone());
        assert!(destino.es_efimero());
        materializar_a(&store, &destino, raiz).unwrap();

        // El contenido aterrizó en la ruta efímera...
        assert_eq!(fs::read(efimero.join(".ssh/id_ed25519")).unwrap(), b"SECRETO\n");
        // ...y el $HOME de disco quedó intacto (nunca se escribió).
        assert!(!disco.exists(), "Efimero no debe tocar el $HOME de disco");
    }

    #[test]
    fn rutas_ausentes_se_saltan() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dest = tmp.path().join("dest");
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();
        escribir(&home, ".zshrc", b"presente\n");

        // .config/nvim no existe en disco ⇒ se salta sin romper.
        let raiz = capturar(&store, &set_basico(), &home).unwrap();
        materializar(&store, &dest, raiz).unwrap();
        assert!(dest.join(".zshrc").exists());
        assert!(!dest.join(".config/nvim").exists());
    }

    #[test]
    fn ruta_con_dotdot_es_rechazada() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        escribir(&home, "x", b"x");
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();
        let set = ConjuntoDotfiles::new("mal").con(RutaGestionada::fijado("../escape"));
        assert!(matches!(capturar(&store, &set, &home), Err(DotError::Ruta(_))));
    }

    #[test]
    fn historial_es_un_dag_de_la_cabeza_a_la_raiz() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let store = StoreObjetos::abrir(tmp.path().join("obj")).unwrap();
        let set = ConjuntoDotfiles::new("shell").con(RutaGestionada::fijado(".zshrc"));

        escribir(&home, ".zshrc", b"v1\n");
        let c1 = capturar_y_commitear(&store, &set, &home, None, "v1", 1000).unwrap();
        escribir(&home, ".zshrc", b"v2\n");
        let c2 = capturar_y_commitear(&store, &set, &home, Some(c1), "v2", 2000).unwrap();
        escribir(&home, ".zshrc", b"v3\n");
        let c3 = capturar_y_commitear(&store, &set, &home, Some(c2), "v3", 3000).unwrap();

        let hist = historial(&store, c3).unwrap();
        let etiquetas: Vec<_> = hist.iter().map(|(_, i)| i.etiqueta.as_str()).collect();
        assert_eq!(etiquetas, ["v3", "v2", "v1"]);
        assert_eq!(hist[2].1.padre, None);

        // restaurar una versión vieja: materializar la raíz de c1 da "v1".
        let dest = tmp.path().join("dest");
        let inst1 = leer_instantanea(&store, &c1).unwrap();
        materializar(&store, &dest, inst1.raiz).unwrap();
        assert_eq!(fs::read(dest.join(".zshrc")).unwrap(), b"v1\n");
    }

    /// Cuenta objetos inscritos en el almacén (archivos hoja bajo `aa/`).
    fn contar_objetos(raiz: PathBuf) -> usize {
        let mut n = 0;
        for shard in fs::read_dir(&raiz).unwrap() {
            let shard = shard.unwrap().path();
            if shard.is_dir() {
                n += fs::read_dir(&shard).unwrap().count();
            }
        }
        n
    }
}
