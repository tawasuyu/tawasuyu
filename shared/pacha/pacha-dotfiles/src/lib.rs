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
// Cifrado en reposo (Fase 2 — "secreto por defecto")
// =====================================================================

/// Sella/abre los bytes de un objeto antes de tocar disco. AEAD
/// `XChaCha20Poly1305` con nonce aleatorio de 192 bits por objeto (margen amplio
/// frente a colisión de nonce bajo una sola clave de store de larga vida). El
/// sobre en disco es `nonce(24) || ciphertext+tag`.
///
/// **Identidad vs. opacidad (decisión abierta #3, resuelta).** El hash que
/// identifica al objeto (y su ruta `aa/bbbb`) sigue siendo el de los bytes **en
/// claro** — así el grafo (referencias hijo→hash) y el dedup por contenido NO
/// cambian, y un store en claro se puede migrar a cifrado sin recomputar hashes.
/// El contenido **y la estructura** (nombres de archivo en los `Arbol`) viajan
/// cifrados: el objeto entero se sella. Lo único que filtra el disco es el hash
/// del claro (la ruta) — habilita un ataque de *confirmación* (probar si un
/// contenido candidato está presente), no de lectura. La opacidad total (hash
/// del sobre) rompería el dedup determinista y queda fuera de alcance.
#[derive(Clone)]
pub struct Cifrador {
    clave: chacha20poly1305::Key,
}

impl std::fmt::Debug for Cifrador {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No filtrar la clave en logs/aserciones.
        f.debug_struct("Cifrador").field("clave", &"<oculta>").finish()
    }
}

impl Cifrador {
    /// Deriva la clave del store de una `seed` de 32 bytes — la **identidad
    /// Ed25519 del usuario** (la que `agora-keystore` desbloquea; el *cómo* se
    /// desbloquea es Fase 3) — con HKDF-SHA256 y separación de dominio. Estilo
    /// `age`: la seed nunca se usa directo como clave de cifrado.
    pub fn derivar_de_seed(seed: &[u8; 32]) -> Self {
        let hk = hkdf::Hkdf::<sha2::Sha256>::new(None, seed);
        let mut clave = [0u8; 32];
        hk.expand(b"pacha-dotfiles-store-v1", &mut clave)
            .expect("32 <= 255*HashLen");
        let c = Self { clave: clave.into() };
        clave.fill(0); // best-effort: no dejar la clave derivada en el stack
        c
    }

    /// Construye un cifrador con una clave simétrica ya derivada (tests / claves
    /// provistas por otra capa).
    pub fn con_clave(clave: [u8; 32]) -> Self {
        Self { clave: clave.into() }
    }

    fn sellar(&self, claro: &[u8]) -> Result<Vec<u8>, DotError> {
        use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
        use chacha20poly1305::XChaCha20Poly1305;
        let cipher = XChaCha20Poly1305::new(&self.clave);
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ct = cipher.encrypt(&nonce, claro).map_err(|_| DotError::Cripto("sellar"))?;
        let mut sobre = Vec::with_capacity(nonce.len() + ct.len());
        sobre.extend_from_slice(&nonce);
        sobre.extend_from_slice(&ct);
        Ok(sobre)
    }

    fn abrir(&self, sobre: &[u8]) -> Result<Vec<u8>, DotError> {
        use chacha20poly1305::aead::{Aead, KeyInit};
        use chacha20poly1305::{XChaCha20Poly1305, XNonce};
        if sobre.len() < 24 {
            return Err(DotError::Cripto("sobre truncado"));
        }
        let (nonce, ct) = sobre.split_at(24);
        let cipher = XChaCha20Poly1305::new(&self.clave);
        cipher
            .decrypt(XNonce::from_slice(nonce), ct)
            .map_err(|_| DotError::Cripto("autenticación falló (clave o sobre)"))
    }
}

// =====================================================================
// El almacén de objetos (host)
// =====================================================================

/// Almacén de objetos del grafo por hash, en un directorio (`aa/bbbb…`, como
/// `.git/objects`). Content-addressed: un objeto ya presente no se reescribe.
/// Con un [`Cifrador`] opcional, los bytes en disco son un sobre AEAD; sin él,
/// van en claro (compat con stores existentes).
#[derive(Clone, Debug)]
pub struct StoreObjetos {
    raiz: PathBuf,
    cifrador: Option<Cifrador>,
}

impl StoreObjetos {
    /// Abre (creando si hace falta) un almacén **en claro** en `raiz`.
    pub fn abrir(raiz: impl Into<PathBuf>) -> Result<Self, DotError> {
        let raiz = raiz.into();
        fs::create_dir_all(&raiz)?;
        Ok(Self { raiz, cifrador: None })
    }

    /// Abre un almacén **cifrado en reposo** con el `cifrador` dado. La identidad
    /// (hash/ruta) sigue siendo la del claro; sólo cambian los bytes en disco.
    pub fn abrir_cifrado(raiz: impl Into<PathBuf>, cifrador: Cifrador) -> Result<Self, DotError> {
        let raiz = raiz.into();
        fs::create_dir_all(&raiz)?;
        Ok(Self { raiz, cifrador: Some(cifrador) })
    }

    /// `true` si el store sella los objetos en reposo.
    pub fn es_cifrado(&self) -> bool {
        self.cifrador.is_some()
    }

    fn ruta_de(&self, h: &Hash) -> PathBuf {
        let hex = hex_de(h);
        self.raiz.join(&hex[0..2]).join(&hex[2..])
    }

    /// Inscribe un objeto y devuelve su hash. Idempotente: si ya está, no
    /// reescribe (la identidad ES el contenido **en claro**). El sobre (si hay
    /// cifrador) se pone ACÁ: `capturar`/`materializar` nunca ven cripto.
    pub fn poner(&self, obj: &Objeto) -> Result<Hash, DotError> {
        let bytes = obj.serializar().map_err(DotError::Formato)?;
        // El hash identifica el CLARO → grafo y dedup intactos, cifre o no.
        let h = format::hash(&bytes);
        let destino = self.ruta_de(&h);
        if destino.exists() {
            return Ok(h);
        }
        if let Some(dir) = destino.parent() {
            fs::create_dir_all(dir)?;
        }
        let en_disco = match &self.cifrador {
            Some(c) => c.sellar(&bytes)?,
            None => bytes,
        };
        // Escritura atómica: tmp + rename. El tmp lleva el hash → no colisiona
        // con otros objetos; dos escritores del mismo objeto escriben sobres
        // distintos (nonce aleatorio), pero ambos abren al mismo claro y el
        // rename final es inocuo.
        let tmp = destino.with_extension("tmp");
        fs::write(&tmp, &en_disco)?;
        fs::rename(&tmp, &destino)?;
        Ok(h)
    }

    /// Recupera un objeto por su hash. Si el store es cifrado, abre el sobre en
    /// RAM antes de deserializar — base del descifrado del destino `Efimero`.
    pub fn traer(&self, h: &Hash) -> Result<Objeto, DotError> {
        let en_disco = fs::read(self.ruta_de(h)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                DotError::Ausente(hex_de(h))
            } else {
                DotError::Io(e)
            }
        })?;
        let bytes = match &self.cifrador {
            Some(c) => c.abrir(&en_disco)?,
            None => en_disco,
        };
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
    #[error("cripto: {0}")]
    Cripto(&'static str),
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

    /// Lee todos los bytes de todos los objetos del store (para grep de claro).
    fn bytes_de_todos_los_objetos(raiz: &Path) -> Vec<u8> {
        let mut acc = Vec::new();
        for shard in fs::read_dir(raiz).unwrap() {
            let shard = shard.unwrap().path();
            if shard.is_dir() {
                for obj in fs::read_dir(&shard).unwrap() {
                    acc.extend(fs::read(obj.unwrap().path()).unwrap());
                }
            }
        }
        acc
    }

    #[test]
    fn cifrado_round_trip_opaco_en_disco_pero_reproduce_el_original() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dest = tmp.path().join("dest");
        let store_dir = tmp.path().join("obj");

        // Un secreto reconocible + un NOMBRE de archivo reconocible (probar que
        // la ESTRUCTURA también va cifrada, no sólo el contenido).
        escribir(&home, ".ssh/clave_secreta", b"CONTENIDO-ULTRA-SECRETO-12345\n");

        let cif = Cifrador::con_clave([7u8; 32]);
        let store = StoreObjetos::abrir_cifrado(&store_dir, cif.clone()).unwrap();
        assert!(store.es_cifrado());

        let set = ConjuntoDotfiles::new("ssh").con(RutaGestionada::fijado(".ssh/clave_secreta"));
        let raiz = capturar(&store, &set, &home).unwrap();

        // 1) Opacidad: ni el contenido ni el nombre del archivo aparecen en claro
        //    en NINGÚN byte de los objetos en disco.
        let crudo = bytes_de_todos_los_objetos(&store_dir);
        assert!(!crudo.is_empty(), "el store debería tener objetos");
        assert!(
            !contiene_sub(&crudo, b"CONTENIDO-ULTRA-SECRETO-12345"),
            "el contenido en claro NO debe aparecer en disco"
        );
        assert!(
            !contiene_sub(&crudo, b"clave_secreta"),
            "el nombre de archivo (estructura) NO debe aparecer en disco"
        );

        // 2) Round-trip: materializar con el mismo cifrador reproduce el original.
        materializar(&store, &dest, raiz).unwrap();
        assert_eq!(
            fs::read(dest.join(".ssh/clave_secreta")).unwrap(),
            b"CONTENIDO-ULTRA-SECRETO-12345\n"
        );

        // 3) Reabrir el MISMO store con la MISMA clave también abre.
        let store2 = StoreObjetos::abrir_cifrado(&store_dir, cif).unwrap();
        let dest2 = tmp.path().join("dest2");
        materializar(&store2, &dest2, raiz).unwrap();
        assert_eq!(
            fs::read(dest2.join(".ssh/clave_secreta")).unwrap(),
            b"CONTENIDO-ULTRA-SECRETO-12345\n"
        );
    }

    #[test]
    fn clave_equivocada_no_abre_los_objetos() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let store_dir = tmp.path().join("obj");
        escribir(&home, ".zshrc", b"export X=1\n");

        let store = StoreObjetos::abrir_cifrado(&store_dir, Cifrador::con_clave([1u8; 32])).unwrap();
        let set = ConjuntoDotfiles::new("shell").con(RutaGestionada::fijado(".zshrc"));
        let raiz = capturar(&store, &set, &home).unwrap();

        // Otro store, MISMA raíz (el hash es del claro), clave distinta ⇒ AEAD falla.
        let store_mal = StoreObjetos::abrir_cifrado(&store_dir, Cifrador::con_clave([2u8; 32])).unwrap();
        let dest = tmp.path().join("dest");
        let err = materializar(&store_mal, &dest, raiz).unwrap_err();
        assert!(matches!(err, DotError::Cripto(_)), "esperaba error de cripto, fue {err:?}");
    }

    #[test]
    fn derivar_de_seed_es_determinista_y_separa_dominio() {
        let seed = [9u8; 32];
        let a = Cifrador::derivar_de_seed(&seed);
        let b = Cifrador::derivar_de_seed(&seed);
        // Misma seed ⇒ misma clave ⇒ un store sellado por `a` lo abre `b`.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let store_dir = tmp.path().join("obj");
        escribir(&home, ".gitconfig", b"[user]\n  name = x\n");
        let set = ConjuntoDotfiles::new("git").con(RutaGestionada::fijado(".gitconfig"));
        let raiz = capturar(&StoreObjetos::abrir_cifrado(&store_dir, a).unwrap(), &set, &home).unwrap();
        let dest = tmp.path().join("dest");
        materializar(&StoreObjetos::abrir_cifrado(&store_dir, b).unwrap(), &dest, raiz).unwrap();
        assert_eq!(fs::read(dest.join(".gitconfig")).unwrap(), b"[user]\n  name = x\n");
        // Una seed distinta NO debe derivar la misma clave (no se puede abrir).
        let otra = Cifrador::derivar_de_seed(&[8u8; 32]);
        let dest2 = tmp.path().join("dest2");
        assert!(materializar(&StoreObjetos::abrir_cifrado(&store_dir, otra).unwrap(), &dest2, raiz).is_err());
    }

    /// `true` si `hay` contiene la subsecuencia `aguja`.
    fn contiene_sub(hay: &[u8], aguja: &[u8]) -> bool {
        hay.windows(aguja.len()).any(|w| w == aguja)
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
