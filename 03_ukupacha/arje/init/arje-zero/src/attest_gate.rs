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

use arje_attest::{atestar_bytes, hash_de, Veredicto};
use arje_card::{AttestPolicy, EntityCard, Payload};
use tracing::{info, warn};

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
/// para poder testear la política sin depender de `current_exe`).
fn run_with_paths(seed: &EntityCard, paths: Vec<String>) -> anyhow::Result<Vec<AttestVerdict>> {
    let trust = seed.attest_rootkey;
    if trust.is_none() {
        warn!(
            "atestación: el seed no ancla una rootkey (attest_rootkey=None) — \
             se valida firma+hash pero NO la procedencia; un seed reescrito por \
             completo podría re-firmar. Ancla una rootkey soberana para endurecer."
        );
    }

    info!(
        binarios = paths.len(),
        concesiones = seed.attest.len(),
        politica = ?seed.attest_policy,
        "atestación al arranque: verificando binarios críticos"
    );

    let mut verdicts = Vec::with_capacity(paths.len());
    let mut hubo_fallo = false;

    for path in paths {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let verdict = atestar_bytes(&seed.attest, &bytes, trust);
                let got_hash = hash_de(&bytes);
                if verdict.es_ok() {
                    info!(%path, "atestación ✓");
                } else {
                    hubo_fallo = true;
                    warn!(%path, motivo = verdict.motivo(), "atestación ✗");
                }
                verdicts.push(AttestVerdict { binary: path, verdict, got_hash });
            }
            Err(e) => {
                // No poder leer un binario crítico cuenta como fallo: no
                // podemos afirmar que el binario que correrá es el atestado.
                hubo_fallo = true;
                warn!(%path, error = %e, "atestación: no pude leer el binario crítico");
                verdicts.push(AttestVerdict {
                    binary: path,
                    verdict: Veredicto::NoAtestada,
                    got_hash: [0u8; 32],
                });
            }
        }
    }

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

        // Intacto → Ok.
        let v = run_with_paths(&seed, vec![path.clone()]).expect("warn no aborta");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].verdict, Veredicto::Ok);

        // Alterado en disco → su hash no está en el manifiesto → NoAtestada.
        std::fs::write(&path, b"binario + backdoor").unwrap();
        let v2 = run_with_paths(&seed, vec![path.clone()]).expect("warn nunca aborta");
        assert_eq!(v2[0].verdict, Veredicto::NoAtestada);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn halt_aborta_si_un_binario_no_atesta() {
        let bytes = b"original".to_vec();
        let (dir, path) = fake_bin("halt", &bytes);
        let seed = seed_con_attest([4u8; 32], &bytes, AttestPolicy::Halt);

        // Intacto + Halt → no aborta.
        assert!(run_with_paths(&seed, vec![path.clone()]).is_ok());

        // Alterado + Halt → aborta el arranque (Err).
        std::fs::write(&path, b"tampered").unwrap();
        assert!(run_with_paths(&seed, vec![path.clone()]).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
