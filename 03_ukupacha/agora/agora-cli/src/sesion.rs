//! Sesión de trabajo: paths, keystore, grafo, errores y helpers.
//!
//! `Sesion::abrir()` carga el keystore y el grafo del directorio de datos
//! del usuario y expone métodos comunes (resolver ids, cargar keypairs…).
//! El tipo de error `Error` y los helpers de bajo nivel (hex, seeds, etc.)
//! también viven aquí para no dispersarlos.

use std::path::PathBuf;

use agora_core::{IdentityId, Keypair};
use agora_graph::TrustGraph;
use agora_keystore::Keystore;

// =============================================================================
//  Error y alias de resultado
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("no pude resolver el directorio de datos del usuario")]
    DirNoResuelto,
    #[error("keystore: {0}")]
    Keystore(agora_keystore::Error),
    #[error("store: {0}")]
    Store(agora_store::Error),
    #[error("postcard: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("agora: {0}")]
    Agora(#[from] agora_core::AgoraError),
    #[error("id hex inválido: esperaba 64 chars hex (recibí {0})")]
    HexInvalido(String),
    #[error("la identidad {0} no tiene seed en el keystore local")]
    IdentidadNoPropia(IdentityId),
    #[error("la identidad {0} no está registrada en el grafo local")]
    IdentidadDesconocida(IdentityId),
    #[error("ningún id del grafo empieza con el prefijo \"{0}\"")]
    PrefijoSinMatch(String),
    #[error(
        "prefijo \"{prefijo}\" matchea {total} identidades distintas (mostrando hasta 5): {candidatos:?}"
    )]
    PrefijoAmbiguo {
        prefijo: String,
        candidatos: Vec<String>,
        total: usize,
    },
    #[error("hash hex inválido: esperaba 64 chars hex (recibí {0})")]
    HashInvalido(String),
    #[error("canal: {0}")]
    Canal(&'static str),
    #[error("agora-channel: {0}")]
    AgoraChannel(agora_channel::CanalError),
    #[error("release: {0}")]
    Release(String),
    #[error("permisos inválidos: {0}")]
    Permiso(String),
    #[error("spec JSON: {0}")]
    Spec(String),
    #[error("foreign-fs: {0:?}")]
    ForeignFs(foreign_fs::FsError),
    #[error("AoE (raw socket): {0}")]
    Aoe(String),
    #[error("multifirma: {0}")]
    MultiSig(agora_core::MultiSigError),
}

pub type CliResult<T> = std::result::Result<T, Error>;

// =============================================================================
//  Sesión
// =============================================================================

pub struct Sesion {
    pub keystore: Keystore,
    pub graph: TrustGraph,
    pub store_path: PathBuf,
    pub passphrase: String,
}

impl Sesion {
    pub fn abrir() -> CliResult<Self> {
        let data_dir = directories::ProjectDirs::from("net", "tawasuyu", "agora")
            .ok_or(Error::DirNoResuelto)?
            .data_dir()
            .to_path_buf();
        std::fs::create_dir_all(&data_dir).map_err(Error::Io)?;
        let store_path = data_dir.join("graph.json");

        let passphrase = std::env::var("AGORA_PASSPHRASE").unwrap_or_else(|_| {
            eprintln!(
                "agora-cli: usando passphrase de desarrollo \"agora-dev\". \
                 Setear AGORA_PASSPHRASE para producción."
            );
            "agora-dev".to_string()
        });

        let keystore = Keystore::open_default().map_err(Error::Keystore)?;
        let graph = if store_path.exists() {
            agora_store::load(&store_path).map_err(Error::Store)?
        } else {
            TrustGraph::new()
        };

        Ok(Self { keystore, graph, store_path, passphrase })
    }

    pub fn guardar(&self) -> CliResult<()> {
        agora_store::save(&self.store_path, &self.graph).map_err(Error::Store)
    }

    pub fn cargar_keypair(&self, id: IdentityId) -> CliResult<Keypair> {
        if !self.keystore.exists(id) {
            return Err(Error::IdentidadNoPropia(id));
        }
        let seed = self.keystore.load(id, &self.passphrase).map_err(Error::Keystore)?;
        Ok(Keypair::from_seed(seed))
    }

    /// `true` si esta identidad tiene seed en el keystore local.
    pub fn es_mia(&self, id: IdentityId) -> bool {
        self.keystore.exists(id)
    }

    /// Resuelve un id desde un input de usuario que puede ser
    /// (a) hex completo de 64 chars o (b) un prefijo hex no ambiguo
    /// contra el conjunto de identidades del grafo. Devuelve error
    /// si el prefijo matchea cero o más de una identidad.
    pub fn resolver_id(&self, input: &str) -> CliResult<IdentityId> {
        let input = input.trim().to_ascii_lowercase();
        if input.len() == 64 {
            return parse_id(&input);
        }
        if input.is_empty() || input.len() > 64 || !input.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::HexInvalido(input));
        }
        let mut matches: Vec<IdentityId> = Vec::new();
        for ident in self.graph.identities() {
            let hex = hex_de(ident.id().as_bytes());
            if hex.starts_with(&input) {
                matches.push(ident.id());
            }
        }
        match matches.len() {
            1 => Ok(matches[0]),
            0 => Err(Error::PrefijoSinMatch(input)),
            n => Err(Error::PrefijoAmbiguo {
                prefijo: input,
                candidatos: matches.iter().take(5).map(|id| hex_de(id.as_bytes())).collect(),
                total: n,
            }),
        }
    }
}

// =============================================================================
//  Helpers de bajo nivel
// =============================================================================

pub fn parse_id(s: &str) -> CliResult<IdentityId> {
    let bytes = parse_hex_32(s).map_err(|_| Error::HexInvalido(s.to_string()))?;
    Ok(IdentityId::from_bytes(bytes))
}

/// Parsea 64 chars hex a un `[u8; 32]`. Usado para ids, hashes y pubkeys.
pub fn parse_hex_32(s: &str) -> Result<[u8; 32], ()> {
    let s = s.trim();
    if s.len() != 64 {
        return Err(());
    }
    let mut bytes = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let ch = std::str::from_utf8(chunk).map_err(|_| ())?;
        bytes[i] = u8::from_str_radix(ch, 16).map_err(|_| ())?;
    }
    Ok(bytes)
}

pub fn parse_hash(s: &str) -> CliResult<[u8; 32]> {
    parse_hex_32(s).map_err(|_| Error::HashInvalido(s.to_string()))
}

/// Lee una seed de 32 bytes desde stdin. Acepta dos formatos:
/// - 64 chars hex (con whitespace/newlines tolerados — `s.trim()` +
///   strip de espacios internos).
/// - exactamente 32 bytes binarios raw.
///
/// Elige por largo del input: si después de strip ascii whitespace
/// queda exactamente 64, intenta parsear como hex; si los bytes raw
/// suman 32, los usa tal cual; otra cosa es error.
pub fn leer_seed_de_stdin() -> CliResult<[u8; 32]> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(64);
    std::io::stdin().read_to_end(&mut buf)?;
    // Strip de whitespace para el caso hex.
    let sin_ws: Vec<u8> = buf.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    if sin_ws.len() == 64 {
        let s = std::str::from_utf8(&sin_ws).map_err(|_| Error::HexInvalido("(stdin)".into()))?;
        return parse_hex_32(s).map_err(|_| Error::HexInvalido(s.to_string()));
    }
    if buf.len() == 32 {
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&buf);
        return Ok(seed);
    }
    Err(Error::HexInvalido(format!(
        "stdin: se esperaba 64 chars hex (recibí {} sin whitespace) o 32 bytes raw (recibí {})",
        sin_ws.len(),
        buf.len()
    )))
}

pub fn hex_de(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() * 2);
    for x in b {
        out.push_str(&format!("{x:02x}"));
    }
    out
}

pub fn kind_label(k: agora_core::IdentityKind) -> &'static str {
    match k {
        agora_core::IdentityKind::Person => "person",
        agora_core::IdentityKind::Community => "community",
        agora_core::IdentityKind::Alliance => "alliance",
        agora_core::IdentityKind::Institution => "institution",
    }
}

/// Segundos UNIX actuales, devuelve 0 si el reloj es pre-epoch.
pub fn ahora_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
