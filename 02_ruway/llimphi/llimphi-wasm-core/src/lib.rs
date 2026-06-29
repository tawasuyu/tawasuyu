//! llimphi-wasm-core — distribución por hash de apps WASM Tier 3 (sin GPU).
//!
//! El núcleo puro de la distribución: resolución por hash, verificación de
//! integridad y de concesión Ed25519, permisos efectivos. **No depende del
//! runner ni de Llimphi** — así un servicio headless (relay/servidor de blobs)
//! lo usa sin arrastrar el stack gráfico. El puente para *correr* la
//! [`VerifiedApp`] vive en `llimphi-wasm-dist` (que suma el runner).
//!
//! La cadena, espejo host de lo que el kernel de wawa hace con sus apps:
//!
//! 1. **Resolver** el bytecode por su hash BLAKE3 desde un [`BlobSource`]
//!    (hoy [`DiskStore`], un CAS local; o un backend P2P como `llimphi-wasm-net`).
//! 2. **Verificar integridad**: el wasm recuperado debe rehashear al hash
//!    pedido — content-addressing, detección de tampering.
//! 3. **Verificar la concesión** Ed25519 ([`format::ConcesionCapacidad`]):
//!    el autor debe habitar un [`TrustRing`] cargable y la firma cubrir
//!    `mensaje_capacidad(bytecode, permisos)`. Una concesión para el bytecode
//!    X jamás vale para Y.
//! 4. **Permisos efectivos** = `declarados & concedidos` — un manifiesto no
//!    puede escalar un binario más allá de su concesión.
//!
//! El hash del bytecode es el del **objeto que lo envuelve**
//! (`Objeto{datos:wasm,hijos:[]}`), idéntico a la ceremonia `agora-cli wawa
//! concesion` y al kernel: una concesión firmada allá vale acá sin cambios.

use std::fs;
use std::path::{Path, PathBuf};

use std::collections::HashMap;

use agora_core::verify_signature;
use format::{ConcesionCapacidad, Objeto, Permisos};
use serde::{Deserialize, Serialize};

pub use format::{Hash, Permisos as PermisosBitfield};

/// Errores de la cadena resolver→verificar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DistError {
    /// El `BlobSource` no tiene bytes para ese hash.
    NoEncontrado,
    /// El wasm recuperado no rehashea al hash pedido.
    IntegridadFallo,
    /// El autor de la concesión no está en el anillo de confianza.
    AutorNoConfiable,
    /// La firma Ed25519 de la concesión no valida.
    FirmaInvalida,
    /// La concesión es para otro bytecode (su `bytecode` no coincide).
    ConcesionParaOtroBytecode,
    /// El hex del hash es inválido.
    HexInvalido,
    /// El hash no mide 32 bytes.
    HashLongitud,
    /// El `Launch` no es una variante `Wasm`.
    LaunchNoWasm,
    /// El manifiesto referencia una concesión que el source no tiene.
    ConcesionNoEncontrada,
    /// El blob de la concesión no deserializa a una ConcesionCapacidad.
    ConcesionCorrupta,
    /// No se pudo cargar el guest en el runner.
    Carga(String),
}

impl std::fmt::Display for DistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DistError::NoEncontrado => write!(f, "bytecode no encontrado en el source"),
            DistError::IntegridadFallo => write!(f, "el wasm no coincide con su hash (tampering)"),
            DistError::AutorNoConfiable => write!(f, "autor de la concesión fuera del anillo"),
            DistError::FirmaInvalida => write!(f, "firma Ed25519 de la concesión inválida"),
            DistError::ConcesionParaOtroBytecode => write!(f, "la concesión es para otro bytecode"),
            DistError::HexInvalido => write!(f, "hash hex inválido"),
            DistError::HashLongitud => write!(f, "el hash no mide 32 bytes"),
            DistError::LaunchNoWasm => write!(f, "el Launch no es Wasm"),
            DistError::ConcesionNoEncontrada => write!(f, "concesión no encontrada en el source"),
            DistError::ConcesionCorrupta => write!(f, "el blob de la concesión es inválido"),
            DistError::Carga(e) => write!(f, "cargar guest: {e}"),
        }
    }
}

impl std::error::Error for DistError {}

// =====================================================================
// Hash canónico del bytecode
// =====================================================================

/// Hash BLAKE3 del **objeto** que envuelve `inner` (`Objeto{datos:inner,hijos:[]}`)
/// — la identidad direccionada por contenido que usan el kernel y agora-cli. NO
/// es `blake3(inner)` crudo. El bytecode y la concesión se direccionan así.
pub fn object_hash(inner: &[u8]) -> Hash {
    let obj = Objeto {
        datos: inner.to_vec(),
        hijos: Vec::new(),
    };
    let payload = obj.serializar().expect("serializar objeto");
    format::hash(&payload)
}

/// Hash canónico de un bytecode wasm (= [`object_hash`] del wasm).
pub fn bytecode_hash(wasm: &[u8]) -> Hash {
    object_hash(wasm)
}

/// Hash del objeto-concesión: su dirección en el CAS, idéntica a la que emite
/// `agora-cli wawa concesion` y a la que `EntradaApp.concesion` referencia.
pub fn grant_hash(grant: &ConcesionCapacidad) -> Hash {
    object_hash(&grant.serializar().expect("serializar concesión"))
}

/// Hash → hex de 64 caracteres.
pub fn hash_to_hex(h: &Hash) -> String {
    hex::encode(h)
}

/// hex → Hash (32 bytes). Tolera espacios alrededor.
pub fn hash_from_hex(s: &str) -> Result<Hash, DistError> {
    let v = hex::decode(s.trim()).map_err(|_| DistError::HexInvalido)?;
    v.try_into().map_err(|_| DistError::HashLongitud)
}

// =====================================================================
// Fuente de bytecode — la costura de transporte
// =====================================================================

/// De dónde salen los bytes de un bytecode dado su hash. `DiskStore` es la
/// impl local; un backend P2P (BrahmanNet: DHT find_providers + stream del
/// blob) sería otra impl, y `LayeredSource` los compone (caché local → red).
pub trait BlobSource {
    /// Recupera el wasm cuyo objeto-bytecode hashea a `hash`, si lo tiene.
    fn fetch(&self, hash: &Hash) -> Option<Vec<u8>>;
}

/// CAS local en disco: un archivo por bytecode, nombrado por el hex de su hash.
pub struct DiskStore {
    dir: PathBuf,
}

impl DiskStore {
    /// Abre (creando si hace falta) el directorio del store.
    pub fn open(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// Inscribe un wasm y devuelve su hash canónico (la "dirección").
    pub fn put(&self, wasm: &[u8]) -> std::io::Result<Hash> {
        let h = bytecode_hash(wasm);
        fs::write(self.path(&h), wasm)?;
        Ok(h)
    }

    /// Inscribe una concesión como blob y devuelve su [`grant_hash`]. Así el
    /// grant viaja por el mismo CAS/P2P que el bytecode — un blob más.
    pub fn put_grant(&self, grant: &ConcesionCapacidad) -> std::io::Result<Hash> {
        let bytes = grant
            .serializar()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let h = object_hash(&bytes);
        fs::write(self.path(&h), &bytes)?;
        Ok(h)
    }

    /// Recupera los bytes crudos por hash (sin verificar — eso lo hace `resolve`).
    pub fn get(&self, hash: &Hash) -> Option<Vec<u8>> {
        fs::read(self.path(hash)).ok()
    }

    fn path(&self, h: &Hash) -> PathBuf {
        self.dir.join(hash_to_hex(h))
    }
}

impl BlobSource for DiskStore {
    fn fetch(&self, hash: &Hash) -> Option<Vec<u8>> {
        self.get(hash)
    }
}

/// Compone fuentes en orden: la primera que tenga el blob gana. Patrón
/// caché-local → red: `LayeredSource::new(vec![Box::new(disk), Box::new(p2p)])`.
pub struct LayeredSource {
    layers: Vec<Box<dyn BlobSource>>,
}

impl LayeredSource {
    pub fn new(layers: Vec<Box<dyn BlobSource>>) -> Self {
        Self { layers }
    }
}

impl BlobSource for LayeredSource {
    fn fetch(&self, hash: &Hash) -> Option<Vec<u8>> {
        self.layers.iter().find_map(|l| l.fetch(hash))
    }
}

/// CAS en memoria — útil para tests y para juntar blobs ya traídos de la red
/// (bytecode + concesión) antes de pasarlos por [`resolve_manifest`].
#[derive(Debug, Default, Clone)]
pub struct MapSource {
    blobs: HashMap<Hash, Vec<u8>>,
}

impl MapSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserta un blob bajo un hash arbitrario (p.ej. bytes ya traídos por P2P).
    pub fn insert(&mut self, hash: Hash, bytes: Vec<u8>) {
        self.blobs.insert(hash, bytes);
    }

    /// Inserta un wasm bajo su [`bytecode_hash`] y lo devuelve.
    pub fn put(&mut self, wasm: &[u8]) -> Hash {
        let h = bytecode_hash(wasm);
        self.blobs.insert(h, wasm.to_vec());
        h
    }

    /// Inserta una concesión bajo su [`grant_hash`] y lo devuelve.
    pub fn put_grant(&mut self, grant: &ConcesionCapacidad) -> Hash {
        let bytes = grant.serializar().expect("serializar concesión");
        let h = object_hash(&bytes);
        self.blobs.insert(h, bytes);
        h
    }
}

impl BlobSource for MapSource {
    fn fetch(&self, hash: &Hash) -> Option<Vec<u8>> {
        self.blobs.get(hash).cloned()
    }
}

// =====================================================================
// Anillo de confianza host-side
// =====================================================================

/// Conjunto de claves públicas Ed25519 que pueden conceder capacidades. Es el
/// equivalente cargable del `AGORA_AUTH_RING` del kernel (que vive en `.rodata`
/// y sólo cambia con reflash); acá se carga de archivo.
#[derive(Debug, Clone, Default)]
pub struct TrustRing {
    keys: Vec<[u8; 32]>,
}

impl TrustRing {
    pub fn new(keys: Vec<[u8; 32]>) -> Self {
        Self { keys }
    }

    pub fn empty() -> Self {
        Self { keys: Vec::new() }
    }

    pub fn contains(&self, k: &[u8; 32]) -> bool {
        self.keys.iter().any(|x| x == k)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Parsea un anillo: una pubkey en hex (64 chars) por línea; `#` comenta;
    /// líneas en blanco se ignoran. Las entradas mal formadas se descartan.
    pub fn from_hex_lines(s: &str) -> Self {
        let keys = s
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim())
            .filter(|l| !l.is_empty())
            .filter_map(|l| hash_from_hex(l).ok())
            .collect();
        Self { keys }
    }

    /// Carga el anillo desde un archivo (formato de [`Self::from_hex_lines`]).
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self::from_hex_lines(&fs::read_to_string(path)?))
    }
}

// =====================================================================
// Verificación
// =====================================================================

/// `true` si `wasm` rehashea exactamente a `expected`.
pub fn verify_integrity(wasm: &[u8], expected: &Hash) -> bool {
    &bytecode_hash(wasm) == expected
}

/// Verifica una concesión: autor en el anillo + firma Ed25519 válida sobre
/// `mensaje_capacidad(bytecode, permisos)`. Devuelve los permisos concedidos.
///
/// Inlinea el equivalente host de `claves::verificar_concesion_capacidad` del
/// kernel (allá usa `ed25519-compact` zero-alloc; acá `ed25519-dalek` vía
/// `agora_core::verify_signature`).
pub fn verify_grant(grant: &ConcesionCapacidad, trust: &TrustRing) -> Result<Permisos, DistError> {
    if !trust.contains(&grant.autor) {
        return Err(DistError::AutorNoConfiable);
    }
    let mensaje = format::mensaje_capacidad(&grant.bytecode, grant.permisos);
    verify_signature(&grant.autor, &mensaje, &grant.firma).map_err(|_| DistError::FirmaInvalida)?;
    Ok(grant.permisos)
}

// =====================================================================
// Referencia, resolución y carga
// =====================================================================

/// Referencia a una app distribuida: qué bytecode, qué permisos declara su
/// manifiesto, y (opcional) la concesión que los respalda.
#[derive(Debug, Clone)]
pub struct AppRef {
    pub bytecode: Hash,
    /// Permisos que el manifiesto declara querer. Sin concesión, no valen nada.
    pub declarados: Permisos,
    /// Concesión firmada que autoriza permisos para `bytecode`. `None` ⇒ app
    /// sin capacidades (fail-closed): permisos efectivos = 0.
    pub concesion: Option<ConcesionCapacidad>,
}

impl AppRef {
    /// App de sólo-UI, sin permisos ni concesión.
    pub fn pure(bytecode: Hash) -> Self {
        Self {
            bytecode,
            declarados: 0,
            concesion: None,
        }
    }
}

/// App resuelta y verificada, lista para correr. Datos puros: el puente para
/// cargarla en el runner Tier 3 (`VerifiedApp::load`) lo agrega
/// `llimphi-wasm-dist`, que es quien depende del runner/GPU.
#[derive(Debug, Clone)]
pub struct VerifiedApp {
    pub wasm: Vec<u8>,
    pub permisos: Permisos,
    pub bytecode: Hash,
}

/// Manifiesto distribuible de una app: referencia **por hash** a su bytecode y,
/// opcional, a la concesión que autoriza sus permisos. Es un objeto
/// content-addressed más — un peer lo publica, otro lo resuelve trayendo ambos
/// blobs por la malla. Espejo host de `format::EntradaApp{bytecode, permisos,
/// concesion}`, sin los campos de runtime del kernel (techo de memoria, fuel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppManifest {
    pub bytecode: Hash,
    /// Permisos que el manifiesto declara. Sin concesión que los respalde, 0.
    pub declarados: Permisos,
    /// Hash del objeto-concesión en el CAS. `None` ⇒ app de sólo-UI.
    pub concesion: Option<Hash>,
}

impl AppManifest {
    /// App de sólo-UI: bytecode, sin permisos ni concesión.
    pub fn pure(bytecode: Hash) -> Self {
        Self {
            bytecode,
            declarados: 0,
            concesion: None,
        }
    }

    pub fn serializar(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("serializar AppManifest")
    }

    pub fn deserializar(bytes: &[u8]) -> Result<Self, DistError> {
        postcard::from_bytes(bytes).map_err(|_| DistError::ConcesionCorrupta)
    }
}

/// Resuelve un [`AppManifest`] end-to-end: trae el bytecode **y** la concesión
/// por hash del `source` (CAS local o P2P), verifica integridad de ambos, valida
/// la concesión contra el anillo y devuelve la app con sus permisos efectivos.
///
/// Esto es el "descubrimiento de concesiones": el grant no viaja inline, se
/// fetchea por su hash igual que el bytecode. Una app con capacidades llega así
/// por la red y corre con permisos reales.
pub fn resolve_manifest(
    source: &impl BlobSource,
    trust: &TrustRing,
    manifest: &AppManifest,
) -> Result<VerifiedApp, DistError> {
    let wasm = source.fetch(&manifest.bytecode).ok_or(DistError::NoEncontrado)?;
    if !verify_integrity(&wasm, &manifest.bytecode) {
        return Err(DistError::IntegridadFallo);
    }
    let permisos = match &manifest.concesion {
        Some(grant_hash) => {
            let blob = source
                .fetch(grant_hash)
                .ok_or(DistError::ConcesionNoEncontrada)?;
            // Integridad del objeto-concesión: su contenido debe direccionarse al
            // hash pedido (la red no es de fiar).
            if &object_hash(&blob) != grant_hash {
                return Err(DistError::IntegridadFallo);
            }
            let grant =
                ConcesionCapacidad::deserializar(&blob).map_err(|_| DistError::ConcesionCorrupta)?;
            if grant.bytecode != manifest.bytecode {
                return Err(DistError::ConcesionParaOtroBytecode);
            }
            let concedidos = verify_grant(&grant, trust)?;
            format::permisos_efectivos(manifest.declarados, concedidos)
        }
        None => 0,
    };
    Ok(VerifiedApp {
        wasm,
        permisos,
        bytecode: manifest.bytecode,
    })
}

/// Resuelve y verifica una `AppRef` contra un source y un anillo de confianza.
pub fn resolve(
    source: &impl BlobSource,
    trust: &TrustRing,
    app: &AppRef,
) -> Result<VerifiedApp, DistError> {
    let wasm = source.fetch(&app.bytecode).ok_or(DistError::NoEncontrado)?;
    if !verify_integrity(&wasm, &app.bytecode) {
        return Err(DistError::IntegridadFallo);
    }
    let permisos = match &app.concesion {
        Some(c) => {
            if c.bytecode != app.bytecode {
                return Err(DistError::ConcesionParaOtroBytecode);
            }
            let concedidos = verify_grant(c, trust)?;
            format::permisos_efectivos(app.declarados, concedidos)
        }
        // Sin concesión: cero capacidades. Una app de UI pura corre igual.
        None => 0,
    };
    Ok(VerifiedApp {
        wasm,
        permisos,
        bytecode: app.bytecode,
    })
}

// =====================================================================
// Cableado de app_bus::Launch::Wasm
// =====================================================================

/// Extrae el hash del bytecode de un `Launch::Wasm`.
pub fn hash_from_launch(launch: &app_bus::Launch) -> Result<Hash, DistError> {
    match launch {
        app_bus::Launch::Wasm { bytecode_hex, .. } => hash_from_hex(bytecode_hex),
        _ => Err(DistError::LaunchNoWasm),
    }
}

/// Traduce un `Launch::Wasm` a un [`AppManifest`]. Si el launch trae `grant_hex`,
/// el manifiesto referencia esa concesión y declara `Permisos::MAX` (honrar el
/// grant completo: efectivos = MAX & concedidos = concedidos). Sin grant, app de
/// sólo-UI. Es la traducción "UI → distribución": app-bus transporta los hex,
/// `dist` los vuelve un manifiesto resoluble.
pub fn manifest_from_launch(launch: &app_bus::Launch) -> Result<AppManifest, DistError> {
    match launch {
        app_bus::Launch::Wasm {
            bytecode_hex,
            grant_hex,
        } => {
            let bytecode = hash_from_hex(bytecode_hex)?;
            let concesion = match grant_hex {
                Some(h) => Some(hash_from_hex(h)?),
                None => None,
            };
            let declarados = if concesion.is_some() { Permisos::MAX } else { 0 };
            Ok(AppManifest {
                bytecode,
                declarados,
                concesion,
            })
        }
        _ => Err(DistError::LaunchNoWasm),
    }
}

/// Resuelve un `Launch::Wasm` a una `VerifiedApp`, end-to-end: trae el bytecode
/// y —si el launch declara `grant_hex`— la concesión por hash del `source`, los
/// verifica y corre la app con sus permisos efectivos. Cierra el lanzamiento de
/// apps WASM con capacidades desde la UI.
pub fn resolve_launch(
    source: &impl BlobSource,
    trust: &TrustRing,
    launch: &app_bus::Launch,
) -> Result<VerifiedApp, DistError> {
    let manifest = manifest_from_launch(launch)?;
    resolve_manifest(source, trust, &manifest)
}

// =====================================================================
// Catálogo — el índice buscable de apps ("qué apps existen")
// =====================================================================

/// Una entrada del catálogo: metadatos buscables + referencias **por hash** al
/// bytecode y, opcional, a su concesión. Es la unidad que falta sobre el
/// transporte por hash: el `BlobSource` sabe traer un blob *si conocés su hash*,
/// pero no *qué apps hay*; esto lo nombra y lo describe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// Identificador estable (para `get`/lanzar por id).
    pub id: String,
    /// Nombre legible.
    pub name: String,
    /// Descripción corta (entra en la búsqueda).
    pub description: String,
    /// Categoría opcional (para filtrar/agrupar).
    pub category: Option<String>,
    /// Hash del bytecode en el CAS.
    pub bytecode: Hash,
    /// Permisos que la app declara querer (sin concesión que los respalde, 0).
    pub declarados: Permisos,
    /// Hash del objeto-concesión, si la app pide capacidades.
    pub concesion: Option<Hash>,
}

impl CatalogEntry {
    /// El [`AppManifest`] resoluble de esta entrada — el puente catálogo→correr.
    pub fn manifest(&self) -> AppManifest {
        AppManifest {
            bytecode: self.bytecode,
            declarados: self.declarados,
            concesion: self.concesion,
        }
    }

    /// ¿La consulta (sin distinguir mayúsculas) aparece en id/nombre/
    /// descripción/categoría? Una consulta vacía siempre coincide.
    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        self.id.to_lowercase().contains(&q)
            || self.name.to_lowercase().contains(&q)
            || self.description.to_lowercase().contains(&q)
            || self
                .category
                .as_deref()
                .map(|c| c.to_lowercase().contains(&q))
                .unwrap_or(false)
    }
}

/// Catálogo de apps: el índice buscable. Es **él mismo un objeto
/// content-addressed** (postcard) — por eso viaja por la misma malla que los
/// bytecodes (`llimphi-wasm-net::fetch_blob` por su [`Catalog::hash`]) sin
/// protocolo nuevo: quien lo publica comparte su hash, quien lo recibe lo busca.
///
/// **No necesita ser de confianza para ser seguro**: cada app que nombra se
/// resuelve por su propio hash y se re-verifica de forma independiente
/// (integridad + concesión Ed25519 contra el [`TrustRing`]). Un catálogo
/// adversario, a lo sumo, ofrece apps cuyas concesiones igual deben estar
/// firmadas por una clave de tu anillo — no puede colar código alterado ni
/// permisos sin respaldo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub entries: Vec<CatalogEntry>,
}

impl Catalog {
    pub fn new(entries: Vec<CatalogEntry>) -> Self {
        Self { entries }
    }

    pub fn serializar(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("serializar Catalog")
    }

    pub fn deserializar(bytes: &[u8]) -> Result<Self, DistError> {
        postcard::from_bytes(bytes).map_err(|_| DistError::ConcesionCorrupta)
    }

    /// La dirección de contenido del catálogo (el mismo esquema que bytecode y
    /// concesión: `hash(Objeto{datos,hijos:[]})`), para publicarlo/traerlo.
    pub fn hash(&self) -> Hash {
        object_hash(&self.serializar())
    }

    /// Entradas que coinciden con `query` (substring en id/nombre/desc/cat).
    /// Una consulta vacía devuelve todo el catálogo.
    pub fn search(&self, query: &str) -> Vec<&CatalogEntry> {
        self.entries.iter().filter(|e| e.matches(query)).collect()
    }

    /// La entrada de id exacto, si existe.
    pub fn get(&self, id: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Agrega o reemplaza (por id) una entrada. Mantiene el id único, así
    /// re-publicar una app la actualiza en vez de duplicarla.
    pub fn upsert(&mut self, entry: CatalogEntry) {
        match self.entries.iter_mut().find(|e| e.id == entry.id) {
            Some(slot) => *slot = entry,
            None => self.entries.push(entry),
        }
    }
}

/// Resuelve y verifica una app del catálogo por su `id`, end-to-end: ubica la
/// entrada, trae bytecode (+ concesión) por hash del `source`, los verifica y
/// devuelve la app con sus permisos efectivos. El puente catálogo→correr.
pub fn resolve_from_catalog(
    source: &impl BlobSource,
    trust: &TrustRing,
    catalog: &Catalog,
    id: &str,
) -> Result<VerifiedApp, DistError> {
    let entry = catalog.get(id).ok_or(DistError::NoEncontrado)?;
    resolve_manifest(source, trust, &entry.manifest())
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    fn entry(id: &str, name: &str, desc: &str, cat: Option<&str>) -> CatalogEntry {
        CatalogEntry {
            id: id.into(),
            name: name.into(),
            description: desc.into(),
            category: cat.map(Into::into),
            bytecode: bytecode_hash(id.as_bytes()),
            declarados: 0,
            concesion: None,
        }
    }

    fn cat() -> Catalog {
        Catalog::new(vec![
            entry("counter", "Contador", "suma y resta un número", Some("demo")),
            entry("form", "Formulario", "campos, slider y radio", Some("demo")),
            entry("paint", "Pintura", "lienzo de dibujo", Some("arte")),
        ])
    }

    #[test]
    fn search_substring_case_insensitive() {
        let c = cat();
        // por nombre/desc, sin distinguir mayúsculas
        let ids: Vec<&str> = c.search("RADIO").iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["form"]);
        // por categoría: dos demos
        assert_eq!(c.search("demo").len(), 2);
        // consulta vacía = todo
        assert_eq!(c.search("").len(), 3);
        // sin coincidencias
        assert!(c.search("zzz").is_empty());
    }

    #[test]
    fn get_y_manifest() {
        let c = cat();
        let e = c.get("paint").expect("existe");
        assert_eq!(e.name, "Pintura");
        let m = e.manifest();
        assert_eq!(m.bytecode, bytecode_hash(b"paint"));
        assert!(m.concesion.is_none());
        assert!(c.get("ausente").is_none());
    }

    #[test]
    fn upsert_actualiza_sin_duplicar() {
        let mut c = cat();
        assert_eq!(c.entries.len(), 3);
        c.upsert(entry("form", "Formulario v2", "ahora con multiline", Some("demo")));
        assert_eq!(c.entries.len(), 3, "mismo id ⇒ reemplaza, no agrega");
        assert_eq!(c.get("form").unwrap().name, "Formulario v2");
        c.upsert(entry("nuevo", "Nuevo", "otra app", None));
        assert_eq!(c.entries.len(), 4);
    }

    #[test]
    fn serde_roundtrip_y_hash_estable() {
        let c = cat();
        let bytes = c.serializar();
        let back = Catalog::deserializar(&bytes).expect("re-deserializa");
        assert_eq!(c, back);
        // El hash es la dirección de contenido: estable y reproducible.
        assert_eq!(c.hash(), back.hash());
        assert_eq!(c.hash(), object_hash(&c.serializar()));
        // Cambiar una entrada cambia el hash.
        let mut c2 = c.clone();
        c2.upsert(entry("counter", "Contador", "cambió la desc", Some("demo")));
        assert_ne!(c.hash(), c2.hash());
    }
}
