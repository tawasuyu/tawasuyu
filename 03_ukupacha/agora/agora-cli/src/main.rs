//! `agora-cli` — operación shell del ágora.
//!
//! Comparte keystore y grafo con [`agora-app`]. Es la cara CLI del
//! mismo dominio: lo que se crea acá aparece en la UI y viceversa.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agora_core::{Attestation, Claim, Identity, IdentityId, IdentityKind, Keypair};
use agora_graph::TrustGraph;
use agora_keystore::Keystore;
use clap::{Parser, Subcommand, ValueEnum};
use rand::RngCore;

// =============================================================================
//  CLI shape
// =============================================================================

#[derive(Parser)]
#[command(name = "agora-cli", about = "Shell para el ágora — identidad, atestaciones, grafo.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Operaciones sobre identidades.
    Identidad {
        #[command(subcommand)]
        op: IdentidadOp,
    },
    /// Firma una atestación con una identidad propia y la agrega al grafo.
    Atestar {
        /// Identidad firmante (debe estar en el keystore).
        #[arg(long)]
        como: String,
        /// Sujeto del claim.
        #[arg(long)]
        sobre: String,
        /// Predicado del claim.
        #[arg(long)]
        pred: String,
        /// Valor del claim.
        #[arg(long)]
        valor: String,
    },
    /// Verifica la firma de una atestación leída desde archivo postcard.
    Verificar {
        archivo: PathBuf,
    },
    /// Exporta el grafo entero a un archivo postcard para sneakernet.
    Exportar {
        archivo: PathBuf,
    },
    /// Importa un grafo postcard y mergea sus atestaciones al local.
    Importar {
        archivo: PathBuf,
    },
    /// Resumen del grafo: cuántas identidades, atestaciones, mías.
    Grafo,
}

#[derive(Subcommand)]
enum IdentidadOp {
    /// Genera una identidad nueva, la cifra en el keystore y la registra
    /// en el grafo.
    Nueva {
        /// Nombre legible (no es único ni autoritativo).
        #[arg(long, default_value = "yo")]
        name: String,
        /// Tipo: person, community, alliance, institution.
        #[arg(long, value_enum, default_value_t = KindArg::Person)]
        kind: KindArg,
    },
    /// Lista todas las identidades del grafo (★ = seed propia).
    Listar,
    /// Imprime la cara pública de una identidad (pubkey hex).
    Exportar {
        id: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    Person,
    Community,
    Alliance,
    Institution,
}

impl From<KindArg> for IdentityKind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Person => IdentityKind::Person,
            KindArg::Community => IdentityKind::Community,
            KindArg::Alliance => IdentityKind::Alliance,
            KindArg::Institution => IdentityKind::Institution,
        }
    }
}

// =============================================================================
//  Sesión: paths, keystore, grafo
// =============================================================================

struct Sesion {
    keystore: Keystore,
    graph: TrustGraph,
    store_path: PathBuf,
    passphrase: String,
}

impl Sesion {
    fn abrir() -> CliResult<Self> {
        let data_dir = directories::ProjectDirs::from("net", "gioser", "agora")
            .ok_or(Error::DirNoResuelto)?
            .data_dir()
            .to_path_buf();
        fs::create_dir_all(&data_dir).map_err(Error::Io)?;
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

        Ok(Self {
            keystore,
            graph,
            store_path,
            passphrase,
        })
    }

    fn guardar(&self) -> CliResult<()> {
        agora_store::save(&self.store_path, &self.graph).map_err(Error::Store)
    }

    fn cargar_keypair(&self, id: IdentityId) -> CliResult<Keypair> {
        if !self.keystore.exists(id) {
            return Err(Error::IdentidadNoPropia(id));
        }
        let seed = self.keystore.load(id, &self.passphrase).map_err(Error::Keystore)?;
        Ok(Keypair::from_seed(seed))
    }

    /// `true` si esta identidad tiene seed en el keystore local.
    fn es_mia(&self, id: IdentityId) -> bool {
        self.keystore.exists(id)
    }
}

// =============================================================================
//  Errores
// =============================================================================

#[derive(Debug, thiserror::Error)]
enum Error {
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
}

type CliResult<T> = std::result::Result<T, Error>;

// =============================================================================
//  Helpers
// =============================================================================

fn parse_id(s: &str) -> CliResult<IdentityId> {
    let s = s.trim();
    if s.len() != 64 {
        return Err(Error::HexInvalido(s.to_string()));
    }
    let mut bytes = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let ch = std::str::from_utf8(chunk).map_err(|_| Error::HexInvalido(s.into()))?;
        bytes[i] = u8::from_str_radix(ch, 16).map_err(|_| Error::HexInvalido(s.into()))?;
    }
    Ok(IdentityId::from_bytes(bytes))
}

fn hex_de(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() * 2);
    for x in b {
        out.push_str(&format!("{x:02x}"));
    }
    out
}

fn kind_label(k: IdentityKind) -> &'static str {
    match k {
        IdentityKind::Person => "person",
        IdentityKind::Community => "community",
        IdentityKind::Alliance => "alliance",
        IdentityKind::Institution => "institution",
    }
}

// =============================================================================
//  Handlers
// =============================================================================

fn run(cmd: Cmd) -> CliResult<()> {
    match cmd {
        Cmd::Identidad { op } => match op {
            IdentidadOp::Nueva { name, kind } => identidad_nueva(name, kind.into()),
            IdentidadOp::Listar => identidad_listar(),
            IdentidadOp::Exportar { id } => identidad_exportar(parse_id(&id)?),
        },
        Cmd::Atestar { como, sobre, pred, valor } => {
            atestar(parse_id(&como)?, parse_id(&sobre)?, &pred, &valor)
        }
        Cmd::Verificar { archivo } => verificar(&archivo),
        Cmd::Exportar { archivo } => exportar(&archivo),
        Cmd::Importar { archivo } => importar(&archivo),
        Cmd::Grafo => grafo_resumen(),
    }
}

fn identidad_nueva(name: String, kind: IdentityKind) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let kp = Keypair::from_seed(seed);
    let id = kp.identity_id();
    s.keystore
        .save(id, &seed, &s.passphrase)
        .map_err(Error::Keystore)?;
    s.graph.register(kp.identity(kind, &name));
    s.guardar()?;
    println!("nueva identidad creada");
    println!("  id     {id_full}", id_full = hex_de(id.as_bytes()));
    println!("  kind   {}", kind_label(kind));
    println!("  name   {name}");
    println!("  pubkey {}", hex_de(&kp.public_key()));
    Ok(())
}

fn identidad_listar() -> CliResult<()> {
    let s = Sesion::abrir()?;
    let mut idents: Vec<&Identity> = s.graph.identities().collect();
    idents.sort_by(|a, b| a.id().as_bytes().cmp(b.id().as_bytes()));
    if idents.is_empty() {
        println!("(grafo vacío — corré `agora-cli identidad nueva`)");
        return Ok(());
    }
    println!("{:>2}  {:<64}  {:<11}  {}", "", "id (hex)", "kind", "name");
    for ident in idents {
        let mark = if s.es_mia(ident.id()) { "★" } else { " " };
        println!(
            "{mark:>2}  {id}  {kind:<11}  {name}",
            id = hex_de(ident.id().as_bytes()),
            kind = kind_label(ident.kind),
            name = ident.display_name,
        );
    }
    Ok(())
}

fn identidad_exportar(id: IdentityId) -> CliResult<()> {
    let s = Sesion::abrir()?;
    let ident = s.graph.identity(id).ok_or(Error::IdentidadDesconocida(id))?;
    println!("id     {}", hex_de(id.as_bytes()));
    println!("kind   {}", kind_label(ident.kind));
    println!("name   {}", ident.display_name);
    println!("pubkey {}", hex_de(&ident.public_key));
    Ok(())
}

fn atestar(como: IdentityId, sobre: IdentityId, pred: &str, valor: &str) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    if s.graph.identity(sobre).is_none() {
        return Err(Error::IdentidadDesconocida(sobre));
    }
    let kp = s.cargar_keypair(como)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let claim = Claim::new(sobre, pred, valor, now);
    let att = Attestation::create(&kp, claim);
    s.graph.add_attestation(att.clone())?;
    s.guardar()?;
    println!("atestación firmada y agregada al grafo");
    println!("  hash   {}", hex_de(&att.stable_hash()));
    println!("  por    {}", hex_de(att.attester.as_bytes()));
    println!("  sobre  {}", hex_de(sobre.as_bytes()));
    println!("  claim  {pred} = {valor}");
    Ok(())
}

fn verificar(archivo: &Path) -> CliResult<()> {
    let bytes = fs::read(archivo)?;
    let att: Attestation = postcard::from_bytes(&bytes)?;
    att.verify()?;
    println!("firma válida");
    println!("  hash   {}", hex_de(&att.stable_hash()));
    println!("  por    {}", hex_de(att.attester.as_bytes()));
    println!("  sobre  {}", hex_de(att.claim.subject.as_bytes()));
    println!("  claim  {} = {}", att.claim.predicate, att.claim.value);
    Ok(())
}

/// Empaqueta el grafo (identidades + atestaciones verificadas) en postcard.
/// Comparte forma con el snapshot de agora-store pero sin envelope JSON —
/// optimizado para transporte por bytes (sneakernet, pipe, etc.).
#[derive(serde::Serialize, serde::Deserialize)]
struct GraphBundle {
    identities: Vec<Identity>,
    attestations: Vec<Attestation>,
}

fn exportar(archivo: &Path) -> CliResult<()> {
    let s = Sesion::abrir()?;
    let bundle = GraphBundle {
        identities: s.graph.identities().cloned().collect(),
        attestations: s.graph.attestations().to_vec(),
    };
    let bytes = postcard::to_allocvec(&bundle)?;
    let n_id = bundle.identities.len();
    let n_att = bundle.attestations.len();
    fs::write(archivo, &bytes)?;
    println!(
        "exportadas {n_id} identidades, {n_att} atestaciones ({} bytes) a {}",
        bytes.len(),
        archivo.display()
    );
    Ok(())
}

fn importar(archivo: &Path) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let bytes = fs::read(archivo)?;
    let bundle: GraphBundle = postcard::from_bytes(&bytes)?;
    let mut ids = 0;
    for ident in bundle.identities {
        s.graph.register(ident);
        ids += 1;
    }
    let mut ok = 0;
    let mut rechazadas = 0;
    for att in bundle.attestations {
        match s.graph.add_attestation(att) {
            Ok(()) => ok += 1,
            Err(_) => rechazadas += 1,
        }
    }
    s.guardar()?;
    println!("importadas {ids} identidades, {ok} atestaciones aceptadas, {rechazadas} rechazadas");
    Ok(())
}

fn grafo_resumen() -> CliResult<()> {
    let s = Sesion::abrir()?;
    let total_id = s.graph.identity_count();
    let total_att = s.graph.attestation_count();
    let mias = s
        .graph
        .identities()
        .filter(|i| s.es_mia(i.id()))
        .count();
    println!(
        "{total_id} identidades ({mias} mías) · {total_att} atestaciones verificadas"
    );
    println!("  store : {}", s.store_path.display());
    println!("  keys  : {}", s.keystore.path().display());
    Ok(())
}

// =============================================================================
//  Entrypoint
// =============================================================================

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agora-cli: {e}");
            ExitCode::FAILURE
        }
    }
}
