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
