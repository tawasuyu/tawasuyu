//! Gate de atestación al arranque (A2 de `PLAN-ATESTACION-Y-HAMMER.md`).
//!
//! Tras montar el bus y **antes de incarnar el target gráfico**, `arje-zero`
//! computa el BLAKE3 de cada binario crítico del seed y lo contrasta contra el
//! manifiesto firmado (`Card::attest`). El resultado se registra en el audit
//! log (cadena anclada al CAS) y, según `Card::attest_policy`, puede abortar el
//! arranque.
//!
//! El núcleo criptográfico vive en `arje-attest` (reusa agora, cero
//! criptografía nueva); este módulo es sólo el **cableado de boot**: enumerar
//! los binarios, leerlos del disco vivo, aplicar la política y armar las
//! entradas de auditoría.
//!
//! **Compat:** si el seed no trae manifiesto (`attest` vacío) el gate es un
//! no-op — los seeds previos arrancan idénticos. La política por defecto es
//! [`AttestPolicy::Warn`] (sólo registra): estrenar la atestación nunca debe
//! poder dejar una máquina sin arranque; el operador la endurece a `Halt`
//! cuando el manifiesto está completo y ancla una rootkey soberana.

use arje_attest::{atestar_bytes, hash_de, rootkey_desde_hex, AgoraId, Veredicto};
use arje_card::{AttestPolicy, EntityCard, Payload};
use tracing::{info, warn};

/// Path por defecto del ancla soberana en disco, si no se compiló una rootkey
/// en el binario ni se pasó `ARJE_ATTEST_ROOTKEY_FILE`.
const ANCLA_FILE_DEFAULT: &str = "/etc/arje/rootkey.pub";

/// Resuelve la **rootkey soberana anclada fuera de la Card**, en orden de
/// confianza decreciente:
///
/// 1. **compilada** en el propio binario de `arje-zero`
///    (`ARJE_ATTEST_ROOTKEY=<hex64>` al build) — la más fuerte, porque viaja
///    *dentro* del binario que el gate también atesta: reescribir la Card no la
///    toca;
/// 2. **archivo** en path confiable (`ARJE_ATTEST_ROOTKEY_FILE`, default
///    [`ANCLA_FILE_DEFAULT`]) — 32 bytes crudos o 64 chars hex.
///
/// `None` si no hay ancla externa: el gate cae a la rootkey **auto-declarada**
/// del seed (`Card::attest_rootkey`), que es el modelo débil — un seed reescrito
/// por completo podría también reemplazar su rootkey. El ancla externa es lo que
/// cierra ese hueco (la "resta soberano" de A2).
fn ancla_externa() -> Option<AgoraId> {
    ancla_externa_con_fuente().map(|(k, _)| k)
}

/// Como [`ancla_externa`] pero también devuelve una descripción legible de
/// **de dónde** salió el ancla (para el reporte del dry-run `--attest-check`).
fn ancla_externa_con_fuente() -> Option<(AgoraId, String)> {
    // (1) compilada en el binario.
    if let Some(hex) = option_env!("ARJE_ATTEST_ROOTKEY") {
        match rootkey_desde_hex(hex) {
            Some(k) => return Some((k, "compilada en el binario (ARJE_ATTEST_ROOTKEY)".into())),
            None => warn!(
                "ARJE_ATTEST_ROOTKEY compilada no es hex de 32 bytes — se ignora"
            ),
        }
    }
    // (2) archivo en disco.
    let path = std::env::var("ARJE_ATTEST_ROOTKEY_FILE")
        .unwrap_or_else(|_| ANCLA_FILE_DEFAULT.into());
    match std::fs::read(&path) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(&bytes);
            Some((k, format!("archivo {path} (32 bytes raw)")))
        }
        Ok(bytes) => match rootkey_desde_hex(&String::from_utf8_lossy(&bytes)) {
            Some(k) => Some((k, format!("archivo {path} (hex)"))),
            None => {
                warn!(%path, "ancla de rootkey en disco no es 32 bytes ni hex válido — se ignora");
                None
            }
        },
        // Ausente es el caso normal mientras el operador no ancla nada.
        Err(_) => None,
    }
}

/// Veredicto de un binario crítico, listo para auditar.
pub struct AttestVerdict {
    pub binary: String,
    pub verdict: Veredicto,
    pub got_hash: [u8; 32],
}

/// Recolecta los paths de los binarios críticos del seed: el `exec` de cada
/// genesis con payload `Native`/`Legacy` (recursivo), más el propio binario de
/// `arje-zero` (PID 1). Deduplica preservando orden.
fn critical_paths(seed: &EntityCard) -> Vec<String> {
    fn walk(card: &EntityCard, out: &mut Vec<String>) {
        match &card.payload {
            Payload::Native { exec, .. } | Payload::Legacy { exec, .. } => out.push(exec.clone()),
            _ => {}
        }
        for hija in &card.genesis {
            walk(hija, out);
        }
    }
    let mut paths = Vec::new();
    for hija in &seed.genesis {
        walk(hija, &mut paths);
    }
    // arje-zero mismo: la raíz de confianza ejecutable también se atesta.
    if let Ok(exe) = std::env::current_exe() {
        paths.push(exe.to_string_lossy().into_owned());
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Corre la atestación sobre los binarios críticos del seed.
///
/// Devuelve los veredictos (para que el caller los vuelque al audit log una vez
/// que el brain exista). Si la política es [`AttestPolicy::Halt`] y algún
/// binario crítico no atesta, devuelve `Err` para abortar el arranque **antes**
/// de incarnar el target.
pub fn run(seed: &EntityCard) -> anyhow::Result<Vec<AttestVerdict>> {
    if seed.attest.is_empty() {
        // Sin manifiesto de atestación: no-op (compat con seeds previos).
        return Ok(Vec::new());
    }
    run_with_paths(seed, critical_paths(seed))
}

/// Núcleo del gate sobre un conjunto explícito de paths (separado de [`run`]
/// para poder testear la política sin depender de `current_exe`). Resuelve el
/// ancla soberana externa y delega en [`run_inner`].
fn run_with_paths(seed: &EntityCard, paths: Vec<String>) -> anyhow::Result<Vec<AttestVerdict>> {
    run_inner(seed, paths, ancla_externa())
}

/// Núcleo del gate con el ancla externa ya resuelta (parámetro explícito para
/// testear el override sin tocar env/disco).
///
/// **La rootkey efectiva = `ancla.or(seed.attest_rootkey)`.** Si hay ancla
/// externa, *manda ella*: las concesiones del seed deben estar firmadas por la
/// rootkey soberana, no por la que el seed declare. Así un seed reescrito por
/// completo que también cambió su `attest_rootkey` cae en `AutorNoConfiable`
/// (la firma no es del ancla) — el ataque que A2 dejaba abierto.
fn run_inner(
    seed: &EntityCard,
    paths: Vec<String>,
    ancla: Option<AgoraId>,
) -> anyhow::Result<Vec<AttestVerdict>> {
    let verdicts = gather_verdicts(seed, paths, ancla);
    let hubo_fallo = verdicts.iter().any(|v| !v.verdict.es_ok());

    if hubo_fallo && seed.attest_policy == AttestPolicy::Halt {
        anyhow::bail!(
            "atestación al arranque falló y la política es Halt — abortando antes \
             de incarnar el target"
        );
    }
    if hubo_fallo && seed.attest_policy == AttestPolicy::Degraded {
        warn!(
            "atestación: arranque DEGRADADO — hubo binarios sin atestar; el target \
             se levanta igual (marcar la unidad comprometida en el brain queda como \
             follow-up A3)"
        );
    }
    Ok(verdicts)
}

/// Dry-run del gate (`--attest-check`): corre la **misma** verificación que el
/// boot pero **nunca aborta** — devuelve todos los veredictos para que el
/// operador inspeccione off-boot antes de endurecer la política a `Halt`.
/// Vacío si el seed no trae manifiesto.
pub fn check(seed: &EntityCard) -> Vec<AttestVerdict> {
    if seed.attest.is_empty() {
        return Vec::new();
    }
    gather_verdicts(seed, critical_paths(seed), ancla_externa())
}

/// Descripción legible del ancla soberana externa resuelta (o `None` si no hay
/// — el gate cae a la rootkey auto-declarada del seed). Para el reporte.
pub fn ancla_fuente() -> Option<String> {
    ancla_externa_con_fuente().map(|(_, src)| src)
}

/// Núcleo compartido: lee cada binario crítico del disco vivo y lo atesta
/// contra el manifiesto del seed con la rootkey efectiva. No aplica política
/// (eso lo hace [`run_inner`]); sólo junta veredictos.
fn gather_verdicts(
    seed: &EntityCard,
    paths: Vec<String>,
    ancla: Option<AgoraId>,
) -> Vec<AttestVerdict> {
    let trust = ancla.or(seed.attest_rootkey);
    match (ancla, seed.attest_rootkey) {
        (Some(a), Some(s)) if a != s => warn!(
            "atestación: el ancla soberana externa ≠ attest_rootkey del seed — \
             el seed declara otra rootkey; se EXIGE la soberana (un seed reescrito \
             no puede re-firmar). Las concesiones del seed van a fallar si no las \
             firmó el ancla."
        ),
        (Some(_), _) => info!("atestación: anclada a rootkey soberana externa"),
        (None, None) => warn!(
            "atestación: sin ancla soberana externa y el seed no declara rootkey \
             (attest_rootkey=None) — se valida firma+hash pero NO la procedencia. \
             Compilá ARJE_ATTEST_ROOTKEY o poné /etc/arje/rootkey.pub para endurecer."
        ),
        (None, Some(_)) => warn!(
            "atestación: sin ancla soberana externa — se confía en la rootkey \
             auto-declarada del seed, que un seed reescrito podría reemplazar. \
             Ancla una rootkey soberana (ARJE_ATTEST_ROOTKEY / rootkey.pub) para cerrar el hueco."
        ),
    }

    info!(
        binarios = paths.len(),
        concesiones = seed.attest.len(),
        politica = ?seed.attest_policy,
        "atestación al arranque: verificando binarios críticos"
    );

    let mut verdicts = Vec::with_capacity(paths.len());

    for path in paths {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let verdict = atestar_bytes(&seed.attest, &bytes, trust);
                let got_hash = hash_de(&bytes);
                if verdict.es_ok() {
                    info!(%path, "atestación ✓");
                } else {
                    warn!(%path, motivo = verdict.motivo(), "atestación ✗");
                }
                verdicts.push(AttestVerdict { binary: path, verdict, got_hash });
            }
            Err(e) => {
                // No poder leer un binario crítico cuenta como fallo: no
                // podemos afirmar que el binario que correrá es el atestado.
                warn!(%path, error = %e, "atestación: no pude leer el binario crítico");
                verdicts.push(AttestVerdict {
                    binary: path,
                    verdict: Veredicto::NoAtestada,
                    got_hash: [0u8; 32],
                });
            }
        }
    }

    verdicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use arje_attest::firmar_binarios;
    use std::path::PathBuf;

    #[test]
    fn seed_sin_manifiesto_es_noop() {
        let seed = EntityCard::new("seed-sin-attest");
        let v = run(&seed).expect("no-op no falla");
        assert!(v.is_empty());
    }

    /// Escribe un binario falso en un tmp único y devuelve (dir, path, bytes).
    fn fake_bin(nombre: &str, contenido: &[u8]) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "arje-attest-gate-{}-{}",
            std::process::id(),
            nombre
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bin");
        std::fs::write(&path, contenido).unwrap();
        (dir, path.to_string_lossy().into_owned())
    }

    fn seed_con_attest(seed_key: [u8; 32], bytes: &[u8], policy: AttestPolicy) -> EntityCard {
        let (rootkey, concesiones) = firmar_binarios(seed_key, &[(bytes.to_vec(), 0u32)]);
        let mut seed = EntityCard::new("seed");
        seed.attest = concesiones;
        seed.attest_rootkey = Some(rootkey);
        seed.attest_policy = policy;
        seed
    }

    #[test]
    fn gate_atesta_binario_intacto_y_detecta_tampering() {
        let bytes = b"binario critico intacto".to_vec();
        let (dir, path) = fake_bin("warn", &bytes);
        let seed = seed_con_attest([3u8; 32], &bytes, AttestPolicy::Warn);

        // Intacto → Ok. (run_inner con ancla None = modelo seed-declarado,
        // hermético: no consulta env/disco.)
        let v = run_inner(&seed, vec![path.clone()], None).expect("warn no aborta");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].verdict, Veredicto::Ok);

        // Alterado en disco → su hash no está en el manifiesto → NoAtestada.
        std::fs::write(&path, b"binario + backdoor").unwrap();
        let v2 = run_inner(&seed, vec![path.clone()], None).expect("warn nunca aborta");
        assert_eq!(v2[0].verdict, Veredicto::NoAtestada);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gather_verdicts_junta_sin_abortar_aun_con_halt() {
        // El núcleo del dry-run (`check` → `gather_verdicts`) debe devolver
        // veredictos SIEMPRE, incluso bajo política Halt con un binario alterado
        // — no aborta como el boot. (Probamos el núcleo con paths explícitos y
        // ancla None: hermético y rápido, sin hashear `current_exe`.)
        let bytes = b"binario para dry-run".to_vec();
        let (dir, path) = fake_bin("check", &bytes);
        let seed = seed_con_attest([7u8; 32], &bytes, AttestPolicy::Halt);

        // Intacto → ✓.
        let v = gather_verdicts(&seed, vec![path.clone()], None);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].verdict, Veredicto::Ok);

        // Alterado → ✗, pero NO aborta (devuelve el veredicto malo).
        std::fs::write(&path, b"alterado").unwrap();
        let v2 = gather_verdicts(&seed, vec![path.clone()], None);
        assert!(
            !v2[0].verdict.es_ok(),
            "el dry-run debe reportar el binario comprometido, no abortar",
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn halt_aborta_si_un_binario_no_atesta() {
        let bytes = b"original".to_vec();
        let (dir, path) = fake_bin("halt", &bytes);
        let seed = seed_con_attest([4u8; 32], &bytes, AttestPolicy::Halt);

        // Intacto + Halt → no aborta.
        assert!(run_inner(&seed, vec![path.clone()], None).is_ok());

        // Alterado + Halt → aborta el arranque (Err).
        std::fs::write(&path, b"tampered").unwrap();
        assert!(run_inner(&seed, vec![path.clone()], None).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El ancla soberana externa cierra el ataque de "seed reescrito por
    /// completo": un atacante re-empaqueta el seed firmando los binarios con SU
    /// propia llave y pone su propia pubkey en `attest_rootkey`. Sin ancla, eso
    /// pasa (el seed es internamente consistente). Con ancla = la rootkey
    /// legítima, las concesiones del atacante caen en AutorNoConfiable → Halt
    /// aborta.
    #[test]
    fn ancla_externa_vence_a_un_seed_reescrito() {
        use arje_attest::firmar_binarios;

        let bytes = b"binario critico real".to_vec();
        let (dir, path) = fake_bin("ancla", &bytes);

        // Llave legítima del operador (la que va anclada en arje-zero / disco).
        let (rootkey_legitima, _) = firmar_binarios([1u8; 32], &[(bytes.clone(), 0u32)]);

        // El atacante reescribe el seed entero con SU llave [66;32] y declara su
        // propia pubkey como attest_rootkey — un seed auto-consistente.
        let seed_atacante = seed_con_attest([66u8; 32], &bytes, AttestPolicy::Halt);

        // (a) SIN ancla externa: el seed reescrito pasa (su rootkey auto-declarada
        //     valida sus propias firmas). Este es exactamente el hueco de A2.
        assert!(
            run_inner(&seed_atacante, vec![path.clone()], None).is_ok(),
            "sin ancla, un seed reescrito se auto-valida"
        );

        // (b) CON ancla = la rootkey legítima: las concesiones del atacante no
        //     fueron firmadas por ella → AutorNoConfiable → Halt aborta.
        let r = run_inner(&seed_atacante, vec![path.clone()], Some(rootkey_legitima));
        assert!(r.is_err(), "el ancla soberana rechaza el seed reescrito");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// rootkey_desde_hex acepta el formato que el operador anclaría a mano.
    #[test]
    fn ancla_parsea_hex_del_operador() {
        let k = rootkey_desde_hex(&"ab".repeat(32)).expect("64 hex chars");
        assert_eq!(k, [0xabu8; 32]);
    }
}
