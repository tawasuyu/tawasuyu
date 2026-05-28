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
    /// Operaciones sobre canales de release (format::Canal).
    Canal {
        #[command(subcommand)]
        op: CanalOp,
    },
}

#[derive(Subcommand)]
enum CanalOp {
    /// Crea un canal nuevo (sin raíces) con `autor` como firmante.
    /// La pubkey de `autor` debe estar en el keystore local — sólo
    /// quien tiene la seed puede extender el canal después.
    Nuevo {
        #[arg(long)]
        nombre: String,
        #[arg(long)]
        autor: String,
        /// Archivo postcard a escribir.
        #[arg(long)]
        salida: PathBuf,
    },
    /// Agrega una RaizFirmada al final del historial del canal y
    /// re-escribe el archivo. El timestamp es ahora UNIX.
    Extender {
        #[arg(long)]
        archivo: PathBuf,
        /// Hash hex (64 chars) del manifiesto que la raíz inaugura.
        #[arg(long)]
        raiz: String,
    },
    /// Verifica firma + monotonicidad de timestamps del canal completo.
    Verificar {
        archivo: PathBuf,
    },
    /// Imprime el canal en formato legible.
    Mostrar {
        archivo: PathBuf,
    },
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

    /// Resuelve un id desde un input de usuario que puede ser
    /// (a) hex completo de 64 chars o (b) un prefijo hex no ambiguo
    /// contra el conjunto de identidades del grafo. Devuelve error
    /// si el prefijo matchea cero o más de una identidad.
    fn resolver_id(&self, input: &str) -> CliResult<IdentityId> {
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
}

type CliResult<T> = std::result::Result<T, Error>;

// =============================================================================
//  Helpers
// =============================================================================

fn parse_id(s: &str) -> CliResult<IdentityId> {
    let bytes = parse_hex_32(s).map_err(|_| Error::HexInvalido(s.to_string()))?;
    Ok(IdentityId::from_bytes(bytes))
}

/// Parsea 64 chars hex a un `[u8; 32]`. Usado para ids, hashes y pubkeys.
fn parse_hex_32(s: &str) -> Result<[u8; 32], ()> {
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

fn parse_hash(s: &str) -> CliResult<[u8; 32]> {
    parse_hex_32(s).map_err(|_| Error::HashInvalido(s.to_string()))
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
            IdentidadOp::Exportar { id } => identidad_exportar(&id),
        },
        Cmd::Atestar { como, sobre, pred, valor } => atestar(&como, &sobre, &pred, &valor),
        Cmd::Verificar { archivo } => verificar(&archivo),
        Cmd::Exportar { archivo } => exportar(&archivo),
        Cmd::Importar { archivo } => importar(&archivo),
        Cmd::Grafo => grafo_resumen(),
        Cmd::Canal { op } => match op {
            CanalOp::Nuevo { nombre, autor, salida } => canal_nuevo(&nombre, &autor, &salida),
            CanalOp::Extender { archivo, raiz } => canal_extender(&archivo, &raiz),
            CanalOp::Verificar { archivo } => canal_verificar(&archivo),
            CanalOp::Mostrar { archivo } => canal_mostrar(&archivo),
        },
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

fn identidad_exportar(id: &str) -> CliResult<()> {
    let s = Sesion::abrir()?;
    let id = s.resolver_id(id)?;
    let ident = s.graph.identity(id).ok_or(Error::IdentidadDesconocida(id))?;
    println!("id     {}", hex_de(id.as_bytes()));
    println!("kind   {}", kind_label(ident.kind));
    println!("name   {}", ident.display_name);
    println!("pubkey {}", hex_de(&ident.public_key));
    Ok(())
}

fn atestar(como: &str, sobre: &str, pred: &str, valor: &str) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let como = s.resolver_id(como)?;
    let sobre = s.resolver_id(sobre)?;
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

// =============================================================================
//  Canales
// =============================================================================

fn ahora_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn canal_nuevo(nombre: &str, autor: &str, salida: &Path) -> CliResult<()> {
    use format::{Canal, NOMBRE_CANAL_LIMITE, VERSION_CANAL};
    let s = Sesion::abrir()?;
    let autor_id = s.resolver_id(autor)?;
    let ident = s
        .graph
        .identity(autor_id)
        .ok_or(Error::IdentidadDesconocida(autor_id))?;
    if !s.es_mia(autor_id) {
        return Err(Error::IdentidadNoPropia(autor_id));
    }
    if nombre.is_empty() || nombre.len() > NOMBRE_CANAL_LIMITE {
        return Err(Error::Canal("nombre vacío o más largo que NOMBRE_CANAL_LIMITE"));
    }
    let canal = Canal {
        version: VERSION_CANAL,
        nombre: nombre.to_string(),
        autor: ident.public_key,
        raices: Vec::new(),
    };
    let bytes = canal.serializar().map_err(Error::Canal)?;
    fs::write(salida, &bytes)?;
    println!(
        "canal nuevo creado: nombre=\"{}\" autor={} → {} ({} bytes)",
        nombre,
        hex_de(autor_id.as_bytes()),
        salida.display(),
        bytes.len()
    );
    Ok(())
}

fn canal_extender(archivo: &Path, raiz_hex: &str) -> CliResult<()> {
    use format::Canal;
    let s = Sesion::abrir()?;
    let raiz_hash = parse_hash(raiz_hex)?;
    let bytes = fs::read(archivo)?;
    let mut canal = Canal::deserializar(&bytes).map_err(Error::Canal)?;

    let autor_id = agora_core::IdentityId::from_public_key(&canal.autor);
    if !s.es_mia(autor_id) {
        return Err(Error::IdentidadNoPropia(autor_id));
    }
    let kp = s.cargar_keypair(autor_id)?;

    let ts = ahora_unix();
    // Forzamos timestamp estrictamente posterior al último — verificar_canal
    // lo exigirá al releer.
    let ts = match canal.raices.last() {
        Some(prev) if ts <= prev.timestamp => prev.timestamp + 1,
        _ => ts,
    };
    let nueva = agora_channel::firmar_raiz(&kp, &canal.nombre, &raiz_hash, ts);
    canal.raices.push(nueva.clone());

    let bytes = canal.serializar().map_err(Error::Canal)?;
    fs::write(archivo, &bytes)?;
    println!(
        "canal \"{}\" extendido: raíz={} ts={} → ahora {} raíces ({} bytes)",
        canal.nombre,
        hex_de(&raiz_hash),
        ts,
        canal.raices.len(),
        bytes.len()
    );
    Ok(())
}

fn canal_verificar(archivo: &Path) -> CliResult<()> {
    use format::Canal;
    let bytes = fs::read(archivo)?;
    let canal = Canal::deserializar(&bytes).map_err(Error::Canal)?;
    agora_channel::verificar_canal(&canal).map_err(Error::AgoraChannel)?;
    println!(
        "canal \"{}\" válido: {} raíces firmadas por {} (timestamps estrictamente monotónicos)",
        canal.nombre,
        canal.raices.len(),
        hex_de(&canal.autor)
    );
    Ok(())
}

fn canal_mostrar(archivo: &Path) -> CliResult<()> {
    use format::Canal;
    let s = Sesion::abrir()?;
    let bytes = fs::read(archivo)?;
    let canal = Canal::deserializar(&bytes).map_err(Error::Canal)?;
    let autor_id = agora_core::IdentityId::from_public_key(&canal.autor);
    let autor_name = s
        .graph
        .identity(autor_id)
        .map(|i| i.display_name.as_str())
        .unwrap_or("(desconocido en el grafo local)");
    println!("canal: {}", canal.nombre);
    println!("autor: {} ({})", hex_de(&canal.autor), autor_name);
    println!("version: {}", canal.version);
    println!("raíces: {}", canal.raices.len());
    for (i, raiz) in canal.raices.iter().enumerate() {
        let valida = agora_channel::verificar_raiz(&canal.autor, &canal.nombre, raiz).is_ok();
        let mark = if valida { "✔" } else { "✘" };
        println!(
            "  #{i:<3} {mark}  ts={ts}  raíz={raiz}",
            i = i,
            ts = raiz.timestamp,
            raiz = hex_de(&raiz.raiz_manifiesto)
        );
    }
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
