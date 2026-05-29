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
    /// Operaciones sobre atestaciones.
    Atestacion {
        #[command(subcommand)]
        op: AtestacionOp,
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
    /// Operaciones host-side específicas de wawa: forjar pubkey
    /// para el AGORA_AUTH_RING + forjar propuestas de manifiesto.
    Wawa {
        #[command(subcommand)]
        op: WawaOp,
    },
}

#[derive(Subcommand)]
enum WawaOp {
    /// Forja un par Ed25519 nuevo, guarda la seed cifrada en el
    /// keystore y escribe la pubkey (32 B raw + 64 chars hex) a stdout.
    /// Útil para alimentar el AGORA_AUTH_RING de wawa-kernel en la
    /// ceremonia de la Fase 48: la pubkey va al binario, la seed queda
    /// offline en el HSM/USB del operador.
    ForjarClave {
        #[arg(long, default_value = "wawa-soberano")]
        name: String,
    },
    /// Toma un hash de manifiesto + una identidad propia y produce un
    /// `format::ManifiestoFirmado` postcard de 128 bytes, listo para
    /// embeber en `apps/mudanza/src/propuesta_demo.bin` o emitir por
    /// `MensajeAkasha::AnunciarCanal`.
    ForjarPropuesta {
        /// Identidad firmante (debe estar en el keystore local).
        #[arg(long)]
        como: String,
        /// Hash hex (64 chars) del manifiesto a anclar.
        #[arg(long)]
        hash: String,
        /// Archivo de salida con los 128 bytes raw.
        #[arg(long)]
        salida: PathBuf,
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
        /// Lee la seed de stdin en vez de generarla con CSPRNG. Acepta
        /// 64 chars hex (más espacios/saltos de línea) o exactamente
        /// 32 bytes raw. Útil para restaurar identidades desde un
        /// backup offline.
        #[arg(long)]
        seed_stdin: bool,
    },
    /// Lista todas las identidades del grafo (★ = seed propia).
    Listar,
    /// Imprime la cara pública de una identidad (pubkey hex).
    Exportar {
        id: String,
    },
    /// Cambia el `display_name` de una identidad ya registrada (el id
    /// no cambia — sólo la etiqueta presentacional).
    Rename {
        /// Id o prefijo hex de la identidad a renombrar.
        id: String,
        /// Nombre nuevo. No es único; no se valida contra el grafo.
        #[arg(long)]
        nombre: String,
    },
    /// Borra una identidad del grafo local y purga sus atestaciones
    /// asociadas (como attester o como subject). Sólo aplica a seeds
    /// propias del keystore — para borrar identidades ajenas hay que
    /// pasar `--force` (se mantiene la atestación huérfana fuera).
    Remove {
        /// Id o prefijo hex de la identidad a borrar.
        id: String,
        /// Permite borrar identidades sin seed local. Por defecto sólo
        /// se aceptan las propias para evitar errores destructivos.
        #[arg(long)]
        force: bool,
        /// Borra también la seed cifrada del keystore. Sin esto, la
        /// identidad se puede re-registrar con `agora-cli identidad
        /// nueva --seed-stdin` después.
        #[arg(long)]
        purgar_keystore: bool,
    },
}

#[derive(Subcommand)]
enum AtestacionOp {
    /// Lista atestaciones del grafo local. Sin filtros, muestra todas
    /// (orden de inserción). Los filtros son AND.
    Listar {
        /// Sólo atestaciones cuyo `claim.subject` matchea ese id/prefijo.
        #[arg(long)]
        subject: Option<String>,
        /// Sólo atestaciones cuyo `attester` matchea ese id/prefijo.
        #[arg(long)]
        attester: Option<String>,
        /// Sólo claims con ese predicado exacto (case-sensitive).
        #[arg(long)]
        predicate: Option<String>,
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

/// Lee una seed de 32 bytes desde stdin. Acepta dos formatos:
/// - 64 chars hex (con whitespace/newlines tolerados — `s.trim()` +
///   strip de espacios internos).
/// - exactamente 32 bytes binarios raw.
///
/// Elige por largo del input: si después de strip ascii whitespace
/// queda exactamente 64, intenta parsear como hex; si los bytes raw
/// suman 32, los usa tal cual; otra cosa es error.
fn leer_seed_de_stdin() -> CliResult<[u8; 32]> {
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
            IdentidadOp::Nueva { name, kind, seed_stdin } => {
                identidad_nueva(name, kind.into(), seed_stdin)
            }
            IdentidadOp::Listar => identidad_listar(),
            IdentidadOp::Exportar { id } => identidad_exportar(&id),
            IdentidadOp::Rename { id, nombre } => identidad_rename(&id, &nombre),
            IdentidadOp::Remove { id, force, purgar_keystore } => {
                identidad_remove(&id, force, purgar_keystore)
            }
        },
        Cmd::Atestacion { op } => match op {
            AtestacionOp::Listar { subject, attester, predicate } => {
                atestacion_listar(subject.as_deref(), attester.as_deref(), predicate.as_deref())
            }
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
        Cmd::Wawa { op } => match op {
            WawaOp::ForjarClave { name } => wawa_forjar_clave(&name),
            WawaOp::ForjarPropuesta { como, hash, salida } => {
                wawa_forjar_propuesta(&como, &hash, &salida)
            }
        },
    }
}

fn identidad_nueva(name: String, kind: IdentityKind, seed_stdin: bool) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let seed = if seed_stdin {
        leer_seed_de_stdin()?
    } else {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        seed
    };
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

fn identidad_rename(id: &str, nombre: &str) -> CliResult<()> {
    if nombre.is_empty() {
        return Err(Error::Canal("nombre vacío — pasá --nombre con un valor"));
    }
    let mut s = Sesion::abrir()?;
    let id = s.resolver_id(id)?;
    let prev = s
        .graph
        .identity(id)
        .ok_or(Error::IdentidadDesconocida(id))?
        .display_name
        .clone();
    if !s.graph.set_display_name(id, nombre.to_string()) {
        // No debería pasar — `identity()` ya devolvió Some — pero
        // dejamos el error explícito por si el contrato del graph cambia.
        return Err(Error::IdentidadDesconocida(id));
    }
    s.guardar()?;
    println!(
        "identidad {} renombrada: \"{}\" → \"{}\"",
        hex_de(id.as_bytes()),
        prev,
        nombre
    );
    Ok(())
}

fn identidad_remove(id: &str, force: bool, purgar_keystore: bool) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let id = s.resolver_id(id)?;
    if s.graph.identity(id).is_none() {
        return Err(Error::IdentidadDesconocida(id));
    }
    if !s.es_mia(id) && !force {
        return Err(Error::Canal(
            "identidad ajena (sin seed local) — pasá --force si querés \
             borrarla igual del grafo local",
        ));
    }
    let stats = s.graph.remove_identity(id);
    if purgar_keystore && s.keystore.exists(id) {
        s.keystore.remove(id).map_err(Error::Keystore)?;
    }
    s.guardar()?;
    println!(
        "identidad {} borrada del grafo · {} atestación{} relacionada{} purgada{}{}",
        hex_de(id.as_bytes()),
        stats.attestations,
        if stats.attestations == 1 { "" } else { "es" },
        if stats.attestations == 1 { "" } else { "s" },
        if stats.attestations == 1 { "" } else { "s" },
        if purgar_keystore {
            " · seed borrada del keystore"
        } else if s.es_mia(id) {
            " · seed PRESERVADA en el keystore (re-registrable con --seed-stdin)"
        } else {
            ""
        }
    );
    Ok(())
}

fn atestacion_listar(
    subject: Option<&str>,
    attester: Option<&str>,
    predicate: Option<&str>,
) -> CliResult<()> {
    let s = Sesion::abrir()?;
    // Los filtros de id se resuelven contra el grafo: aceptamos
    // prefijos por consistencia con el resto de la CLI.
    let subject_id = subject.map(|x| s.resolver_id(x)).transpose()?;
    let attester_id = attester.map(|x| s.resolver_id(x)).transpose()?;

    let mut total = 0usize;
    for att in s.graph.attestations() {
        if let Some(id) = subject_id {
            if att.claim.subject != id {
                continue;
            }
        }
        if let Some(id) = attester_id {
            if att.attester != id {
                continue;
            }
        }
        if let Some(p) = predicate {
            if att.claim.predicate != p {
                continue;
            }
        }
        total += 1;
        let hash = hex_de(&att.stable_hash());
        let hash_short: String = hash.chars().take(12).collect();
        let attester_short: String =
            hex_de(att.attester.as_bytes()).chars().take(12).collect();
        let subject_short: String = hex_de(att.claim.subject.as_bytes())
            .chars()
            .take(12)
            .collect();
        let mark = if s.es_mia(att.attester) { "★" } else { " " };
        println!(
            "{mark} {hash_short}  {attester_short}→{subject_short}  {pred}={valor}  ts={ts}",
            mark = mark,
            hash_short = hash_short,
            attester_short = attester_short,
            subject_short = subject_short,
            pred = att.claim.predicate,
            valor = att.claim.value,
            ts = att.claim.issued_at,
        );
    }
    if total == 0 {
        println!("(0 atestaciones bajo los filtros aplicados)");
    } else {
        println!("— {total} atestación{plural}", plural = if total == 1 { "" } else { "es" });
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
    // Append-only: en grafos grandes no re-serializamos todo. El
    // siguiente load consolidará snapshot + log; compactar es manual.
    agora_store::append_attestation(&s.store_path, &att).map_err(Error::Store)?;
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

// =============================================================================
//  Wawa host-side
// =============================================================================

fn wawa_forjar_clave(name: &str) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let kp = Keypair::from_seed(seed);
    let id = kp.identity_id();
    s.keystore.save(id, &seed, &s.passphrase).map_err(Error::Keystore)?;
    s.graph.register(kp.identity(IdentityKind::Person, name));
    s.guardar()?;

    println!("clave forjada para AGORA_AUTH_RING:");
    println!("  id     {}", hex_de(id.as_bytes()));
    println!("  pubkey {}", hex_de(&kp.public_key()));
    println!();
    println!("Para empotrar en wawa-kernel/src/claves.rs:");
    println!("  pub const AGORA_AUTH_RING: [[u8; 32]; N] = [");
    println!("      // slot X :: {name}");
    print!("      [");
    for (i, b) in kp.public_key().iter().enumerate() {
        if i % 8 == 0 {
            println!();
            print!("          ");
        }
        print!("0x{b:02x}, ");
    }
    println!();
    println!("      ],");
    println!("      // ... otros slots");
    println!("  ];");
    println!();
    println!("La seed correspondiente vive cifrada en el keystore local.");
    println!("Hacer backup con: agora-cli identidad exportar {id}");
    Ok(())
}

fn wawa_forjar_propuesta(como: &str, hash_hex: &str, salida: &Path) -> CliResult<()> {
    let s = Sesion::abrir()?;
    let como_id = s.resolver_id(como)?;
    let kp = s.cargar_keypair(como_id)?;
    let manifiesto_hash = parse_hash(hash_hex)?;
    let mf = agora_channel::firmar_manifiesto(&kp, &manifiesto_hash);
    let bytes = mf.serializar().map_err(Error::Canal)?;
    if bytes.len() != 128 {
        return Err(Error::Canal("ManifiestoFirmado postcard ≠ 128 bytes (contrato roto)"));
    }
    fs::write(salida, &bytes)?;
    println!("propuesta forjada: {} bytes → {}", bytes.len(), salida.display());
    println!("  manifiesto_hash : {}", hex_de(&manifiesto_hash));
    println!("  autor (pubkey)  : {}", hex_de(&mf.autor));
    println!("  firma           : {}...{} (64 B)", hex_de(&mf.firma[..4]), hex_de(&mf.firma[60..]));
    println!();
    println!("Para que wawa-kernel lo acepte, la pubkey del autor debe");
    println!("estar en AGORA_AUTH_RING de claves.rs. Si no está, mudanza");
    println!("la verifica en userspace OK y el kernel responde con");
    println!("CapacidadInsuficiente.");
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
