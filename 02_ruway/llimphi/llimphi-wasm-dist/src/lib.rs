//! llimphi-wasm-dist — distribución por hash de apps WASM Tier 3.
//!
//! La cadena, espejo host de lo que el kernel de wawa hace con sus apps:
//!
//! 1. **Resolver** el bytecode por su hash BLAKE3 desde un [`BlobSource`]
//!    (hoy [`DiskStore`], un CAS local; mañana un backend P2P sobre
//!    `BrahmanNet`).
//! 2. **Verificar integridad**: el wasm recuperado debe rehashear al hash
//!    pedido — content-addressing, detección de tampering.
//! 3. **Verificar la concesión** Ed25519 ([`format::ConcesionCapacidad`]):
//!    el autor debe habitar un [`TrustRing`] cargable y la firma cubrir
//!    `mensaje_capacidad(bytecode, permisos)`. Una concesión para el bytecode
//!    X jamás vale para Y.
//! 4. **Permisos efectivos** = `declarados & concedidos` — un manifiesto no
//!    puede escalar un binario más allá de su concesión.
//! 5. **Correr** la [`VerifiedApp`] en `llimphi-wasm-runner` con esos permisos,
//!    que gatean qué host imports se enlazan (frontera física).
//!
//! El hash del bytecode es el del **objeto que lo envuelve**
//! (`Objeto{datos:wasm,hijos:[]}`), idéntico a la ceremonia `agora-cli wawa
//! concesion` y al kernel: una concesión firmada allá vale acá sin cambios.

use std::fs;
use std::path::{Path, PathBuf};

use agora_core::verify_signature;
use format::{ConcesionCapacidad, Objeto, Permisos};

pub use format::{Hash, Permisos as PermisosBitfield};
pub use llimphi_wasm_runner::WasmGuest;

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
            DistError::Carga(e) => write!(f, "cargar guest: {e}"),
        }
    }
}

impl std::error::Error for DistError {}

// =====================================================================
// Hash canónico del bytecode
// =====================================================================

/// Hash BLAKE3 del **objeto** que envuelve al wasm — la identidad direccionada
/// por contenido que usan la concesión y el kernel. NO es `blake3(wasm)` crudo.
pub fn bytecode_hash(wasm: &[u8]) -> Hash {
    let obj = Objeto {
        datos: wasm.to_vec(),
        hijos: Vec::new(),
    };
    let payload = obj.serializar().expect("serializar objeto-bytecode");
    format::hash(&payload)
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

/// App resuelta y verificada, lista para correr.
#[derive(Debug, Clone)]
pub struct VerifiedApp {
    pub wasm: Vec<u8>,
    pub permisos: Permisos,
    pub bytecode: Hash,
}

impl VerifiedApp {
    /// Carga la app en el runner Tier 3 con sus permisos efectivos.
    pub fn load(&self) -> Result<WasmGuest, DistError> {
        WasmGuest::load(&self.wasm, self.permisos).map_err(DistError::Carga)
    }
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

/// Extrae el hash de un `Launch::Wasm { bytecode_hex }`.
pub fn hash_from_launch(launch: &app_bus::Launch) -> Result<Hash, DistError> {
    match launch {
        app_bus::Launch::Wasm { bytecode_hex } => hash_from_hex(bytecode_hex),
        _ => Err(DistError::LaunchNoWasm),
    }
}

/// Resuelve un `Launch::Wasm` a una `VerifiedApp`. El `Launch` sólo lleva el
/// hash (sin concesión), así que la app resuelve como UI pura (permisos 0); las
/// concesiones, cuando existan, viajan aparte y se arman vía [`resolve`] con un
/// [`AppRef`] completo.
pub fn resolve_launch(
    source: &impl BlobSource,
    trust: &TrustRing,
    launch: &app_bus::Launch,
) -> Result<VerifiedApp, DistError> {
    let bytecode = hash_from_launch(launch)?;
    resolve(source, trust, &AppRef::pure(bytecode))
}
