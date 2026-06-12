//! Handlers para atestaciones, verificación de firmas, exportar/importar grafo
//! y resumen del grafo.
//!
//! Cubre los subcomandos: `atestar`, `verificar`, `exportar`, `importar`,
//! `grafo`, y `atestacion listar`.

use std::fs;
use std::path::Path;

use agora_core::{Attestation, Claim, Identity};

use crate::sesion::{ahora_unix, hex_de, CliResult, Error, Sesion};

// =============================================================================
//  Bundle sneakernet: serializa el grafo completo a postcard
// =============================================================================

/// Empaqueta el grafo (identidades + atestaciones verificadas) en postcard.
/// Comparte forma con el snapshot de agora-store pero sin envelope JSON —
/// optimizado para transporte por bytes (sneakernet, pipe, etc.).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct GraphBundle {
    pub identities: Vec<Identity>,
    pub attestations: Vec<Attestation>,
}

// =============================================================================
//  atestacion listar
// =============================================================================

pub fn atestacion_listar(
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
        let attester_short: String = hex_de(att.attester.as_bytes()).chars().take(12).collect();
        let subject_short: String =
            hex_de(att.claim.subject.as_bytes()).chars().take(12).collect();
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

// =============================================================================
//  atestar
// =============================================================================

pub fn atestar(como: &str, sobre: &str, pred: &str, valor: &str) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let como = s.resolver_id(como)?;
    let sobre = s.resolver_id(sobre)?;
    if s.graph.identity(sobre).is_none() {
        return Err(Error::IdentidadDesconocida(sobre));
    }
    let kp = s.cargar_keypair(como)?;
    let now = ahora_unix();
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

// =============================================================================
//  verificar (firma suelta)
// =============================================================================

pub fn verificar(archivo: &Path) -> CliResult<()> {
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

// =============================================================================
//  exportar / importar grafo (sneakernet)
// =============================================================================

pub fn exportar(archivo: &Path) -> CliResult<()> {
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

pub fn importar(archivo: &Path) -> CliResult<()> {
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
//  resumen del grafo
// =============================================================================

pub fn grafo_resumen() -> CliResult<()> {
    let s = Sesion::abrir()?;
    let total_id = s.graph.identity_count();
    let total_att = s.graph.attestation_count();
    let mias = s.graph.identities().filter(|i| s.es_mia(i.id())).count();
    println!(
        "{total_id} identidades ({mias} mías) · {total_att} atestaciones verificadas"
    );
    println!("  store : {}", s.store_path.display());
    println!("  keys  : {}", s.keystore.path().display());
    Ok(())
}
