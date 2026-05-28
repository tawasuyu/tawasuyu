use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("keypair file: {0}")]
    KeypairFile(#[from] minga_store::KeypairFileError),

    #[error("store: {0}")]
    Store(#[from] minga_store::StoreError),

    #[error("attestation: {0}")]
    Attestation(#[from] minga_core::AttestationError),

    #[error("parse: {0}")]
    Parse(#[from] minga_core::parse::ParseError),

    #[error("network: {0}")]
    Network(#[from] minga_p2p::NodeError),

    #[error("peer open: {0}")]
    PeerOpen(#[from] minga_p2p::PeerOpenError),

    #[error("peer sync: {0}")]
    PeerSync(#[from] minga_p2p::PeerSyncError),

    #[error("multiaddr inválido: {0}")]
    Multiaddr(String),

    #[error("el directorio del repo ya existe: {0}")]
    AlreadyExists(PathBuf),

    #[error("el multiaddr no incluye `/p2p/<peer_id>`")]
    NoPeerIdInMultiaddr,

    #[error("timeout esperando conexión")]
    SyncTimeout,

    #[error("notify (file watcher): {0}")]
    Notify(#[from] notify::Error),

    #[error(
        "lenguaje no soportado para {path}: extensión '{extension}' no mapea \
         a ningún dialecto conocido (rs, py, pyi, ts, js, mjs, cjs, go)"
    )]
    UnsupportedLanguage { path: PathBuf, extension: String },

    #[error("hash hex inválido: '{0}' (esperado 64 caracteres hex)")]
    InvalidHash(String),

    #[error("hash no encontrado en el repo: {0}")]
    HashNotFound(minga_core::ContentHash),

    #[error("ningún peer del DHT anuncia ser proveedor de {0}")]
    NoProvidersForHash(minga_core::ContentHash),

    #[error(
        "el path {0} no tiene historial de ingesta — corré `minga ingest` o \
         `minga watch` primero"
    )]
    PathNotIngested(PathBuf),

    #[error("bundle malformado: postcard no pudo decodificarlo")]
    InvalidBundle,

    #[error(
        "versión de bundle {0} no soportada por este binario (esperado 1) — \
         actualizá minga o re-exportá desde la versión que generó el archivo"
    )]
    UnsupportedBundleVersion(u32),

    #[error(
        "el dialecto del bundle (byte {0}) no es reconocido por este binario \
         — el archivo lo generó una versión más nueva con soporte para un \
         lenguaje que esta no entiende"
    )]
    UnknownDialect(u8),

    #[error(
        "la raíz del bundle ({struct_hash}) no produce el α-hash declarado \
         ({claimed_alpha}) bajo el dialecto declarado — bundle corrupto o \
         falsificado"
    )]
    BundleAlphaMismatch {
        struct_hash: minga_core::ContentHash,
        claimed_alpha: minga_core::ContentHash,
    },

    #[error(
        "la raíz {0} no tiene dialect registrado en `roots` — exportá una \
         raíz que haya sido ingerida (no sincronizada bajo el viejo wire \
         pre-RootDeclaration)"
    )]
    BundleMissingDialect(minga_core::ContentHash),

    #[error(
        "el archivo es un multi-bundle — usá `minga bundle import-all` para \
         importar todas las raíces de una vez"
    )]
    ExpectedSingleBundle,

    #[error(
        "el archivo es un bundle single — usá `minga bundle import` para \
         importar una raíz individual"
    )]
    ExpectedMultiBundle,
}
