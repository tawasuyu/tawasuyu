//! `agora-store` — persistencia del [`TrustGraph`] con re-verificación.
//!
//! El TrustGraph deriva `Serialize/Deserialize`, así que técnicamente se
//! podría leer del disco con un `serde_json::from_str` directo. **No lo
//! hacemos**: ese camino confía en el archivo de disco. Si alguien lo
//! edita a mano puede inyectar atestaciones falsas con firmas inválidas
//! y el grafo en memoria las daría por buenas — viola el contrato del
//! crate (*"el grafo sólo guarda evidencia comprobable"*).
//!
//! Por eso [`load`] lee a una estructura espejo privada y reconstruye el
//! grafo invocando `add_attestation` por cada entrada — re-verifica las
//! firmas. Una firma rota en el archivo es un error de carga, no un
//! silencio.
//!
//! Lo que **no** se persiste: los [`Keypair`](agora_core::Keypair). El
//! crate de identidad lo declara explícito (*"la clave privada nunca se
//! serializa ni viaja por la red"*). Si en algún momento hace falta
//! perpetuar una identidad propia entre arranques, la API tiene que ser
//! deliberada y aparte de este store — quizá un seed cifrado con
//! passphrase, no un derive de Serialize callado.

#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use agora_core::{AgoraError, Attestation, Identity, KeyRotation, Revocation};
use agora_graph::TrustGraph;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Versión actual del esquema en disco.
pub const SCHEMA: u32 = 1;

/// Magic prefix del formato postcard binario. Distingue al snapshot
/// nuevo del JSON legacy sin tocar el layout JSON existente — `load`
/// detecta uno u otro por los primeros bytes del archivo. Cuatro bytes
/// ASCII para que `file <ruta>` los muestre legibles.
pub const POSTCARD_MAGIC: &[u8; 4] = b"AGRP";

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("postcard: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("versión de esquema desconocida: {found} (esta build soporta {SCHEMA})")]
    SchemaDesconocida { found: u32 },
    #[error("atestación con firma inválida en el archivo: {0}")]
    AtestacionInvalida(AgoraError),
    #[error("rotación de clave con firma inválida en el archivo: {0}")]
    RotacionInvalida(AgoraError),
    #[error("revocación con firma inválida en el archivo: {0}")]
    RevocacionInvalida(AgoraError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Snapshot serializable del grafo. Usa `Vec` (no `HashMap`) para que
/// el formato sea JSON-friendly — `IdentityId` no es string y los map
/// keys de JSON sí lo son.
#[derive(Serialize, Deserialize)]
struct GraphSnapshot {
    identities: Vec<Identity>,
    attestations: Vec<Attestation>,
    /// Tombstones del ciclo de vida de claves (SDD #4). Son eventos de CAMINO
    /// FRÍO (raros), así que viven en el snapshot y no en el append-log de
    /// atestaciones (camino caliente). `serde(default)` lee snapshots viejos
    /// —sin estos campos— sin romper, conservando `SCHEMA = 1`.
    #[serde(default)]
    rotations: Vec<KeyRotation>,
    #[serde(default)]
    revocations: Vec<Revocation>,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    schema: u32,
    graph: GraphSnapshot,
}

fn snapshot_of(g: &TrustGraph) -> GraphSnapshot {
    GraphSnapshot {
        identities: g.identities().cloned().collect(),
        attestations: g.attestations().to_vec(),
        rotations: g.rotations().to_vec(),
        revocations: g.revocations().to_vec(),
    }
}

/// Guarda el grafo de forma atómica (tmp → fsync → rename) en formato
/// JSON legible — el default histórico, retrocompat.
pub fn save(ruta: &Path, graph: &TrustGraph) -> Result<()> {
    let env = Envelope { schema: SCHEMA, graph: snapshot_of(graph) };

    let tmp = tmp_path(ruta);
    {
        let f = File::create(&tmp)?;
        let mut w = BufWriter::new(f);
        serde_json::to_writer_pretty(&mut w, &env)?;
        w.flush()?;
        w.into_inner()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .sync_all()?;
    }
    fs::rename(&tmp, ruta)?;
    Ok(())
}

/// Variante binaria: el mismo `Envelope` serializado con postcard,
/// precedido del prefijo `POSTCARD_MAGIC` para que [`load`] lo distinga
/// del JSON legacy. ~3–5× más chico que el JSON en grafos densos; útil
/// como apuesta default cuando un repo de ágora va por miles de
/// atestaciones.
///
/// El archivo no es legible con `cat` — para inspección manual usar
/// [`save`] o pasar el binario por una herramienta postcard.
pub fn save_postcard(ruta: &Path, graph: &TrustGraph) -> Result<()> {
    let env = Envelope { schema: SCHEMA, graph: snapshot_of(graph) };
    let body = postcard::to_allocvec(&env)?;

    let tmp = tmp_path(ruta);
    {
        let f = File::create(&tmp)?;
        let mut w = BufWriter::new(f);
        w.write_all(POSTCARD_MAGIC)?;
        w.write_all(&body)?;
        w.flush()?;
        w.into_inner()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .sync_all()?;
    }
    fs::rename(&tmp, ruta)?;
    Ok(())
}

/// Carga el grafo desde disco y reconstruye un [`TrustGraph`] nuevo
/// re-verificando cada atestación.
///
/// Combina dos fuentes:
/// 1. El **snapshot JSON** en `ruta` (lo que produce [`save`]).
/// 2. El **append-log** en `<ruta>.log` (lo que produce
///    [`append_attestation`]) — replay completo.
///
/// Si el snapshot falla por una firma rota, devuelve
/// [`Error::AtestacionInvalida`] sin entregar grafo parcial. Si el
/// log contiene registros con firma rota o truncados, los ignora en
/// silencio (un log es lo que es: append-only, puede tener basura al
/// final por crashes). El conteo de aceptadas/rechazadas del log no se
/// expone en el resultado — pasar por [`replay_log`] si hace falta.
pub fn load(ruta: &Path) -> Result<TrustGraph> {
    let mut g = if ruta.exists() {
        let env = read_envelope(ruta)?;
        if env.schema != SCHEMA {
            return Err(Error::SchemaDesconocida { found: env.schema });
        }
        let mut g = TrustGraph::new();
        for identity in env.graph.identities {
            g.register(identity);
        }
        for att in env.graph.attestations {
            g.add_attestation(att).map_err(Error::AtestacionInvalida)?;
        }
        // Tombstones de ciclo de vida: re-verificar como las atestaciones —una
        // firma rota en el archivo es error de carga, no un silencio—. La
        // rotación re-chequea su doble firma; la revocación, la integridad de
        // sus firmas (el umbral M-of-N contra el set autorizador lo aplica el
        // consumidor, no el store: ver `TrustGraph::ingest_revocation`).
        for rot in env.graph.rotations {
            g.add_rotation(rot).map_err(Error::RotacionInvalida)?;
        }
        for rev in env.graph.revocations {
            g.ingest_revocation(rev).map_err(Error::RevocacionInvalida)?;
        }
        g
    } else {
        TrustGraph::new()
    };

    let log = log_path(ruta);
    if log.exists() {
        let _ = replay_log(&mut g, &log)?;
    }
    Ok(g)
}

// =============================================================================
//  Append-log: agregar UNA atestación sin reescribir el snapshot
// =============================================================================

/// Agrega una atestación al append-log de `ruta` (`<ruta>.log`). Es la
/// operación idiomática para sumar una atestación sin pagar el costo
/// de re-serializar el grafo entero — muy importante en grafos grandes.
/// Cada registro es `[u32 LE largo][postcard de Attestation]`. El log
/// crece append-only; usar [`compact`] para fusionar al snapshot y
/// truncarlo.
///
/// El archivo se abre con `O_APPEND` y se hace `sync_all` tras cada
/// registro — si el proceso crashea a mitad del write, el lector
/// detecta la cola truncada y la descarta (ver [`replay_log`]).
pub fn append_attestation(ruta: &Path, att: &Attestation) -> Result<()> {
    let log = log_path(ruta);
    let bytes = postcard::to_allocvec(att)?;
    let len = bytes.len() as u32;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)?;
    f.write_all(&len.to_le_bytes())?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    Ok(())
}

/// Reproduce el append-log sobre `g`: aplica cada atestación con
/// [`TrustGraph::add_attestation`] (que re-verifica firma) y devuelve
/// `(aceptadas, rechazadas)`. Registros truncados o postcard inválido
/// en el TAIL se ignoran silenciosamente — son la consecuencia normal
/// de un crash a mitad de append.
pub fn replay_log(g: &mut TrustGraph, log: &Path) -> Result<(usize, usize)> {
    let mut f = File::open(log)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let mut aceptadas = 0;
    let mut rechazadas = 0;
    let mut cursor = 0;
    while cursor + 4 <= buf.len() {
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&buf[cursor..cursor + 4]);
        let len = u32::from_le_bytes(len_bytes) as usize;
        cursor += 4;
        if cursor + len > buf.len() {
            // Registro truncado — crash a mitad del append. Ignorar tail.
            break;
        }
        match postcard::from_bytes::<Attestation>(&buf[cursor..cursor + len]) {
            Ok(att) => match g.add_attestation(att) {
                Ok(()) => aceptadas += 1,
                Err(_) => rechazadas += 1,
            },
            Err(_) => {
                // Postcard inválido — corrupción o cambio de formato.
                // Asumimos que el resto del log es basura y cortamos.
                break;
            }
        }
        cursor += len;
    }
    Ok((aceptadas, rechazadas))
}

/// Fusiona snapshot + log en un snapshot fresco, y trunca el log.
///
/// **No es atómico** entre los dos pasos: si crashea entre el snapshot
/// y el unlink del log, el log puede contener registros que YA están en
/// el snapshot — al siguiente `load`, el replay los volverá a aplicar y
/// `add_attestation` los descartará como duplicados (idempotente).
/// Resultado neto: indistinguible del éxito completo.
pub fn compact(ruta: &Path, graph: &TrustGraph) -> Result<()> {
    save(ruta, graph)?;
    let log = log_path(ruta);
    if log.exists() {
        fs::remove_file(&log)?;
    }
    Ok(())
}

/// Lee `ruta` y deserializa el `Envelope` detectando el formato. El
/// magic `POSTCARD_MAGIC` al inicio activa el branch postcard; cualquier
/// otra cosa cae al branch JSON legacy.
fn read_envelope(ruta: &Path) -> Result<Envelope> {
    let mut f = File::open(ruta)?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes)?;
    if bytes.len() >= POSTCARD_MAGIC.len() && &bytes[..POSTCARD_MAGIC.len()] == POSTCARD_MAGIC {
        Ok(postcard::from_bytes(&bytes[POSTCARD_MAGIC.len()..])?)
    } else {
        Ok(serde_json::from_slice(&bytes)?)
    }
}

fn log_path(ruta: &Path) -> PathBuf {
    let mut s = ruta.as_os_str().to_owned();
    s.push(".log");
    PathBuf::from(s)
}

fn tmp_path(ruta: &Path) -> PathBuf {
    let mut s = ruta.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::{Attestation, Claim, IdentityId, IdentityKind, Keypair};

    fn graph_ejemplo() -> (TrustGraph, IdentityId) {
        let yumaira = Keypair::from_seed([20; 32]);
        let venezuela = Keypair::from_seed([10; 32]);
        let comunidad = Keypair::from_seed([30; 32]);

        let mut g = TrustGraph::new();
        g.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
        g.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
        g.register(comunidad.identity(IdentityKind::Community, "Vecinos del Valle"));

        let yid = yumaira.identity_id();
        g.add_attestation(Attestation::create(
            &venezuela,
            Claim::new(yid, "nacionalidad", "venezolana", 1_700_000_000),
        ))
        .unwrap();
        g.add_attestation(Attestation::create(
            &comunidad,
            Claim::new(yid, "vive_en", "El Valle", 1_700_000_100),
        ))
        .unwrap();

        (g, yid)
    }

    #[test]
    fn save_load_roundtrip_preserva_conteos() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("agora.json");
        let (original, _) = graph_ejemplo();
        save(&ruta, &original).unwrap();
        let cargado = load(&ruta).unwrap();
        assert_eq!(cargado.identity_count(), original.identity_count());
        assert_eq!(cargado.attestation_count(), original.attestation_count());
    }

    #[test]
    fn load_re_verifica_evidencia() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("agora.json");
        let (original, yid) = graph_ejemplo();
        save(&ruta, &original).unwrap();
        let cargado = load(&ruta).unwrap();

        let cor_orig = original.corroboration(yid, "nacionalidad", "venezolana");
        let cor_load = cargado.corroboration(yid, "nacionalidad", "venezolana");
        assert_eq!(cor_orig.total(), cor_load.total());
        assert_eq!(cor_orig.attesters.len(), 1);
    }

    #[test]
    fn tampered_attestation_falla_load() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("malicioso.json");
        let (g, yid) = graph_ejemplo();

        // Forjar att con firma rota: crear bien, luego editar el value.
        let venezuela = Keypair::from_seed([10; 32]);
        let mut att = Attestation::create(
            &venezuela,
            Claim::new(yid, "nacionalidad", "venezolana", 1_700_000_000),
        );
        att.claim.value = "antártica".into();

        let mut snapshot = snapshot_of(&g);
        snapshot.attestations.push(att);
        let env = Envelope { schema: SCHEMA, graph: snapshot };
        std::fs::write(&ruta, serde_json::to_string(&env).unwrap()).unwrap();

        let err = load(&ruta).unwrap_err();
        assert!(
            matches!(err, Error::AtestacionInvalida(_)),
            "esperaba AtestacionInvalida, fue {err:?}"
        );
    }

    #[test]
    fn schema_desconocida_falla() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("futuro.json");
        std::fs::write(
            &ruta,
            r#"{"schema": 999, "graph": {"identities": [], "attestations": []}}"#,
        )
        .unwrap();
        assert!(matches!(load(&ruta), Err(Error::SchemaDesconocida { found: 999 })));
    }

    #[test]
    fn append_y_load_replayan_el_log() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("agora.json");
        let (original, yid) = graph_ejemplo();
        save(&ruta, &original).unwrap();

        // Atestación nueva que NO está en el snapshot, solo en el log.
        let vecina = Keypair::from_seed([40; 32]);
        let extra = Attestation::create(
            &vecina,
            Claim::new(yid, "oficio", "partera", 1_700_000_200),
        );
        append_attestation(&ruta, &extra).unwrap();

        let cargado = load(&ruta).unwrap();
        assert_eq!(cargado.attestation_count(), original.attestation_count() + 1);
        let c = cargado.corroboration(yid, "oficio", "partera");
        assert_eq!(c.total(), 1);
    }

    #[test]
    fn compact_funde_el_log_al_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("agora.json");
        let log = dir.path().join("agora.json.log");
        let (original, yid) = graph_ejemplo();
        save(&ruta, &original).unwrap();

        let vecina = Keypair::from_seed([40; 32]);
        let extra = Attestation::create(
            &vecina,
            Claim::new(yid, "oficio", "partera", 1_700_000_200),
        );
        append_attestation(&ruta, &extra).unwrap();
        assert!(log.exists());

        // Cargar incluye snapshot + log.
        let mut g = load(&ruta).unwrap();
        compact(&ruta, &g).unwrap();
        assert!(!log.exists(), "compact debe borrar el log");

        // Después de compact, el snapshot ya contiene la atestación.
        // Aplicar duplicada (simular replay tras crash) no rompe.
        append_attestation(&ruta, &extra).unwrap();
        let cargado = load(&ruta).unwrap();
        assert_eq!(cargado.attestation_count(), original.attestation_count() + 1);
        g.add_attestation(extra).ok(); // ya estaba — silencioso
    }

    #[test]
    fn log_truncado_se_ignora_silencioso() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("agora.json");
        let log = dir.path().join("agora.json.log");
        let (original, yid) = graph_ejemplo();
        save(&ruta, &original).unwrap();

        let vecina = Keypair::from_seed([40; 32]);
        let extra = Attestation::create(
            &vecina,
            Claim::new(yid, "oficio", "partera", 1_700_000_200),
        );
        append_attestation(&ruta, &extra).unwrap();

        // Truncar el log en mitad del último registro.
        let bytes = std::fs::read(&log).unwrap();
        let truncado = &bytes[..bytes.len() - 10];
        std::fs::write(&log, truncado).unwrap();

        // load no debe romper; la atestación del registro truncado se pierde.
        let cargado = load(&ruta).unwrap();
        assert_eq!(cargado.attestation_count(), original.attestation_count());
    }

    #[test]
    fn append_sin_snapshot_y_load_funciona() {
        // Caso "primer arranque": no hay snapshot todavía pero algo ya
        // hizo append. load debe arrancar de TrustGraph::new y replayar.
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("agora.json");
        let (_, yid) = graph_ejemplo();

        // Necesitamos que las identidades estén en el grafo del replay,
        // si no add_attestation no las registra automáticamente.
        // Por simplicidad acá, sólo verificamos que load no rompa y
        // devuelva un grafo vacío con 0 atestaciones (el log existe pero
        // sus atestaciones son sobre yids que el grafo nuevo no conoce
        // — add_attestation las acepta igual, las identidades se
        // registran aparte).
        let vecina = Keypair::from_seed([40; 32]);
        let att = Attestation::create(
            &vecina,
            Claim::new(yid, "oficio", "partera", 1_700_000_200),
        );
        append_attestation(&ruta, &att).unwrap();

        let cargado = load(&ruta).unwrap();
        assert_eq!(cargado.attestation_count(), 1);
        // No registramos identidades en el grafo durante el replay
        // (el log sólo lleva atestaciones, no identidades).
        assert_eq!(cargado.identity_count(), 0);
    }

    #[test]
    fn save_postcard_y_load_roundtrip() {
        // save_postcard escribe binario; load autodetecta y devuelve un
        // grafo idéntico. JSON y postcard son intercambiables como
        // backend de persistencia.
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("agora.bin");
        let (original, _) = graph_ejemplo();
        save_postcard(&ruta, &original).unwrap();

        // El primer bytes del archivo deben ser el magic.
        let bytes = std::fs::read(&ruta).unwrap();
        assert_eq!(&bytes[..4], POSTCARD_MAGIC, "magic AGRP al inicio");

        let cargado = load(&ruta).unwrap();
        assert_eq!(cargado.identity_count(), original.identity_count());
        assert_eq!(cargado.attestation_count(), original.attestation_count());
    }

    #[test]
    fn load_autodetecta_json_y_postcard() {
        let dir = tempfile::tempdir().unwrap();
        let (g, _) = graph_ejemplo();

        let json_path = dir.path().join("legacy.json");
        save(&json_path, &g).unwrap();
        let from_json = load(&json_path).unwrap();

        let bin_path = dir.path().join("nuevo.bin");
        save_postcard(&bin_path, &g).unwrap();
        let from_bin = load(&bin_path).unwrap();

        assert_eq!(from_json.identity_count(), from_bin.identity_count());
        assert_eq!(
            from_json.attestation_count(),
            from_bin.attestation_count()
        );
    }

    #[test]
    fn postcard_es_significativamente_mas_chico_que_json() {
        // No nos casamos con un ratio exacto — depende de la
        // densidad del grafo y del pretty-print del JSON — pero sí
        // exigimos un improvement visible (>1.5×) en el caso de
        // prueba, que es lo que vendería el cambio de formato.
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("a.json");
        let bin_path = dir.path().join("a.bin");
        let (g, _) = graph_ejemplo();
        save(&json_path, &g).unwrap();
        save_postcard(&bin_path, &g).unwrap();
        let json_size = std::fs::metadata(&json_path).unwrap().len();
        let bin_size = std::fs::metadata(&bin_path).unwrap().len();
        assert!(
            (bin_size as f64) * 1.5 <= json_size as f64,
            "esperaba postcard < json/1.5; json={json_size} bin={bin_size}"
        );
    }

    #[test]
    fn save_no_deja_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("a.json");
        let (g, _) = graph_ejemplo();
        save(&ruta, &g).unwrap();
        assert!(ruta.exists());
        assert!(!tmp_path(&ruta).exists());
    }

    // =========================================================================
    //  Ciclo de vida — persistencia de rotaciones/revocaciones (SDD #4 fase 3)
    // =========================================================================

    /// Grafo con una rotación (v1→v2) y una revocación M-of-2 sobre `target`.
    fn graph_con_tombstones() -> TrustGraph {
        use agora_core::{KeyRotation, RevReason, Revocation};
        let v1 = Keypair::from_seed([1; 32]);
        let v2 = Keypair::from_seed([2; 32]);
        let g1 = Keypair::from_seed([71; 32]);
        let g2 = Keypair::from_seed([72; 32]);
        let allowed = [g1.identity_id(), g2.identity_id()];
        let target = Keypair::from_seed([99; 32]).public_key();

        let mut g = TrustGraph::new();
        g.add_rotation(KeyRotation::create(&v1, &v2, 100)).unwrap();
        let rev = Revocation::create(target, RevReason::Compromised, 200, None, &[&g1, &g2]);
        g.add_revocation(rev, 2, &allowed).unwrap();
        g
    }

    #[test]
    fn tombstones_roundtrip_json_y_postcard() {
        let dir = tempfile::tempdir().unwrap();
        let g = graph_con_tombstones();

        for (nombre, guardar) in [
            ("a.json", save as fn(&Path, &TrustGraph) -> Result<()>),
            ("a.bin", save_postcard as fn(&Path, &TrustGraph) -> Result<()>),
        ] {
            let ruta = dir.path().join(nombre);
            guardar(&ruta, &g).unwrap();
            let cargado = load(&ruta).unwrap();
            assert_eq!(cargado.rotations().len(), 1, "{nombre}: rotación perdida");
            assert_eq!(cargado.revocations().len(), 1, "{nombre}: revocación perdida");
            // La revocación sigue suprimiendo: el target queda revocado a now.
            let target = agora_core::Keypair::from_seed([99; 32]).identity_id();
            assert!(cargado.is_revoked_at(target, 300), "{nombre}: revocación no rige tras recargar");
            // La cadena de rotación se reconstruyó: v1 resuelve a v2.
            let v1 = agora_core::Keypair::from_seed([1; 32]).identity_id();
            let v2 = agora_core::Keypair::from_seed([2; 32]).identity_id();
            assert_eq!(cargado.current_key_at(v1, 50), Some(v2));
        }
    }

    #[test]
    fn snapshot_viejo_sin_tombstones_carga_igual() {
        // Un Envelope JSON sin los campos rotations/revocations (schema 1
        // anterior a la fase 4) debe cargar sin romper — serde(default).
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("legacy.json");
        std::fs::write(
            &ruta,
            r#"{"schema":1,"graph":{"identities":[],"attestations":[]}}"#,
        )
        .unwrap();
        let g = load(&ruta).unwrap();
        assert_eq!(g.rotations().len(), 0);
        assert_eq!(g.revocations().len(), 0);
    }

    #[test]
    fn revocacion_con_firma_forjada_falla_load() {
        use agora_core::{RevReason, Revocation};
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("malicioso.json");
        let g1 = Keypair::from_seed([71; 32]);
        let target = Keypair::from_seed([99; 32]).public_key();
        // Revocación con una firma real, luego corrompida en el archivo.
        let mut rev = Revocation::create(target, RevReason::Compromised, 1, None, &[&g1]);
        rev.authorizers.signers[0].signature[0] ^= 0xFF;

        let snapshot = GraphSnapshot {
            identities: vec![],
            attestations: vec![],
            rotations: vec![],
            revocations: vec![rev],
        };
        let env = Envelope { schema: SCHEMA, graph: snapshot };
        std::fs::write(&ruta, serde_json::to_string(&env).unwrap()).unwrap();

        let err = load(&ruta).unwrap_err();
        assert!(
            matches!(err, Error::RevocacionInvalida(_)),
            "esperaba RevocacionInvalida, fue {err:?}"
        );
    }

    #[test]
    fn rotacion_con_firma_forjada_falla_load() {
        use agora_core::KeyRotation;
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("malicioso.json");
        let v1 = Keypair::from_seed([1; 32]);
        let v2 = Keypair::from_seed([2; 32]);
        let mut rot = KeyRotation::create(&v1, &v2, 100);
        rot.sig_new[0] ^= 0xFF; // rompe la firma de la nueva

        let snapshot = GraphSnapshot {
            identities: vec![],
            attestations: vec![],
            rotations: vec![rot],
            revocations: vec![],
        };
        let env = Envelope { schema: SCHEMA, graph: snapshot };
        std::fs::write(&ruta, serde_json::to_string(&env).unwrap()).unwrap();

        let err = load(&ruta).unwrap_err();
        assert!(
            matches!(err, Error::RotacionInvalida(_)),
            "esperaba RotacionInvalida, fue {err:?}"
        );
    }
}
