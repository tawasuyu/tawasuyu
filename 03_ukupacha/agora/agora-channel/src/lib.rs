//! `agora-channel` — la verificación criptográfica de los canales de
//! release que `format` declara pero deja sin implementar.
//!
//! Cuatro funciones bastan:
//!
//! - [`firmar_raiz`] firma una raíz para un canal con un [`Keypair`].
//! - [`verificar_raiz`] re-verifica una `RaizFirmada` contra la pubkey
//!   del autor del canal.
//! - [`verificar_canal`] recorre todo el historial de un canal y
//!   exige que las firmas sean válidas Y que los timestamps crezcan
//!   estrictamente.
//! - [`firmar_para_anuncio`] produce el par `(AgoraId, Firma)` que va
//!   en `MensajeAkasha::AnunciarCanal`. El caller (que sí depende del
//!   crate `akasha`) ensambla el frame.
//!
//! No depende de `akasha` ni de ningún transporte. Crypto pura sobre
//! tipos que `format` ya declara.

#![forbid(unsafe_code)]

use agora_core::{verify_signature, AgoraError, IdentityId, Keypair};
use format::{AgoraId, Canal, Firma, Hash, RaizFirmada};
use thiserror::Error;

/// Falla al firmar o verificar dentro del contrato del canal.
#[derive(Debug, Error)]
pub enum CanalError {
    /// La firma de una raíz no valida contra `autor` del canal.
    #[error("firma inválida en raíz timestamp={timestamp}")]
    FirmaInvalida { timestamp: u64 },

    /// El historial de raíces no es estrictamente monotónico en
    /// timestamp: dos raíces con el mismo segundo, o una con timestamp
    /// menor o igual a la anterior. Replay barato — lo rechazamos.
    #[error(
        "timestamps no monotónicos en raíz #{indice} ({timestamp} <= {previo})"
    )]
    TimestampNoMonotonico {
        indice: usize,
        timestamp: u64,
        previo: u64,
    },

    /// La clave pública del autor del canal no es válida Ed25519 (32
    /// bytes que no forman un punto válido en la curva). Muy raro —
    /// sólo sucede con basura inyectada deliberadamente.
    #[error("clave pública del autor del canal inválida")]
    AutorInvalido,
}

impl From<AgoraError> for CanalError {
    fn from(_: AgoraError) -> Self {
        // El detalle exacto del AgoraError no aporta al consumidor del
        // canal — sólo importa que algo del paquete criptográfico
        // falló. Para distinguir BadPublicKey vs BadSignature
        // mapearíamos por variant; acá nos quedamos con FirmaInvalida
        // como cubeta de error genérica al verificar una sola raíz.
        Self::FirmaInvalida { timestamp: 0 }
    }
}

// =============================================================================
//  Firmar
// =============================================================================

/// Firma una raíz de manifiesto para un canal: produce la entrada que
/// `Canal::raices` va a aceptar. La firma cubre el mensaje canónico de
/// `format::mensaje_a_firmar(nombre_canal, timestamp, raiz)` — incluye
/// el nombre del canal para que una firma válida en `dev` no se replique
/// en `estable`.
pub fn firmar_raiz(
    kp: &Keypair,
    canal_nombre: &str,
    raiz: &Hash,
    timestamp: u64,
) -> RaizFirmada {
    let mensaje = format::mensaje_a_firmar(canal_nombre, timestamp, raiz);
    let firma = kp.sign(&mensaje);
    RaizFirmada {
        timestamp,
        raiz_manifiesto: *raiz,
        firma,
    }
}

/// Variante para el anuncio Akasha: devuelve `(autor, firma)` directos
/// para que el caller (que sí depende de `akasha`) los meta en
/// `MensajeAkasha::AnunciarCanal { canal, raiz, autor, timestamp, firma }`.
/// El mensaje firmado es idéntico al de [`firmar_raiz`] — un anuncio y
/// un historial usan la misma firma.
pub fn firmar_para_anuncio(
    kp: &Keypair,
    canal_nombre: &str,
    raiz: &Hash,
    timestamp: u64,
) -> (AgoraId, Firma) {
    let mensaje = format::mensaje_a_firmar(canal_nombre, timestamp, raiz);
    (kp.public_key(), kp.sign(&mensaje))
}

// =============================================================================
//  Verificar
// =============================================================================

/// Verifica una `RaizFirmada` aislada contra la pubkey del autor del
/// canal. Devuelve `Err(FirmaInvalida{...})` si la firma no covers el
/// mensaje canónico bajo `autor`.
pub fn verificar_raiz(
    autor: &AgoraId,
    canal_nombre: &str,
    raiz: &RaizFirmada,
) -> Result<(), CanalError> {
    let mensaje = format::mensaje_a_firmar(canal_nombre, raiz.timestamp, &raiz.raiz_manifiesto);
    verify_signature(autor, &mensaje, &raiz.firma)
        .map_err(|_| CanalError::FirmaInvalida { timestamp: raiz.timestamp })
}

/// Verifica el historial entero de un canal. Dos invariantes:
/// 1. Cada `RaizFirmada` valida bajo `canal.autor` para el mensaje
///    canónico que su `(canal.nombre, timestamp, raiz_manifiesto)`
///    produce.
/// 2. Los `timestamp`s crecen ESTRICTAMENTE: dos entradas con el mismo
///    segundo se rechazan, igual que una con timestamp menor que la
///    anterior. `format` deja explícito que el historial está ordenado
///    por timestamp ascendente; acá enforce eso al verificar.
pub fn verificar_canal(canal: &Canal) -> Result<(), CanalError> {
    let mut previo: Option<u64> = None;
    for (indice, raiz) in canal.raices.iter().enumerate() {
        if let Some(prev) = previo {
            if raiz.timestamp <= prev {
                return Err(CanalError::TimestampNoMonotonico {
                    indice,
                    timestamp: raiz.timestamp,
                    previo: prev,
                });
            }
        }
        verificar_raiz(&canal.autor, &canal.nombre, raiz)?;
        previo = Some(raiz.timestamp);
    }
    Ok(())
}

// =============================================================================
//  Conveniencias
// =============================================================================

/// IdentityId que correspondería al autor de un canal — útil para
/// localizar la identidad en un `TrustGraph` (agora-graph).
pub fn autor_como_identity_id(canal: &Canal) -> IdentityId {
    IdentityId::from_public_key(&canal.autor)
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use format::VERSION_CANAL;

    fn hash_de(n: u8) -> Hash {
        [n; 32]
    }

    #[test]
    fn firmar_y_verificar_una_raiz() {
        let kp = Keypair::from_seed([7; 32]);
        let raiz = firmar_raiz(&kp, "estable", &hash_de(1), 1_700_000_000);
        let autor = kp.public_key();
        assert!(verificar_raiz(&autor, "estable", &raiz).is_ok());
    }

    #[test]
    fn firma_de_canal_dev_no_replica_en_estable() {
        // Garantía clave del diseño: el nombre del canal entra en el
        // mensaje firmado, así que firmar en "dev" no le sirve a nadie
        // para validar en "estable".
        let kp = Keypair::from_seed([7; 32]);
        let raiz = firmar_raiz(&kp, "dev", &hash_de(1), 1_700_000_000);
        let autor = kp.public_key();
        assert!(verificar_raiz(&autor, "dev", &raiz).is_ok());
        assert!(matches!(
            verificar_raiz(&autor, "estable", &raiz),
            Err(CanalError::FirmaInvalida { .. })
        ));
    }

    #[test]
    fn firma_con_autor_ajeno_falla() {
        let real = Keypair::from_seed([7; 32]);
        let impostor = Keypair::from_seed([99; 32]);
        let raiz = firmar_raiz(&real, "estable", &hash_de(1), 1_700_000_000);
        assert!(matches!(
            verificar_raiz(&impostor.public_key(), "estable", &raiz),
            Err(CanalError::FirmaInvalida { .. })
        ));
    }

    #[test]
    fn raiz_manipulada_se_detecta() {
        let kp = Keypair::from_seed([7; 32]);
        let mut raiz = firmar_raiz(&kp, "estable", &hash_de(1), 1_700_000_000);
        // Alterar el hash de la raíz invalida la firma — exactamente lo
        // que un atacante que reemplaza el manifiesto querría hacer.
        raiz.raiz_manifiesto[0] ^= 0x01;
        let autor = kp.public_key();
        assert!(matches!(
            verificar_raiz(&autor, "estable", &raiz),
            Err(CanalError::FirmaInvalida { .. })
        ));
    }

    #[test]
    fn timestamp_manipulado_se_detecta() {
        let kp = Keypair::from_seed([7; 32]);
        let mut raiz = firmar_raiz(&kp, "estable", &hash_de(1), 1_700_000_000);
        // Avanzar el timestamp sin re-firmar rompe el mensaje canónico.
        raiz.timestamp += 60;
        let autor = kp.public_key();
        assert!(matches!(
            verificar_raiz(&autor, "estable", &raiz),
            Err(CanalError::FirmaInvalida { .. })
        ));
    }

    #[test]
    fn canal_con_historial_valido_pasa() {
        let kp = Keypair::from_seed([7; 32]);
        let canal = Canal {
            version: VERSION_CANAL,
            nombre: "estable".into(),
            autor: kp.public_key(),
            raices: vec![
                firmar_raiz(&kp, "estable", &hash_de(1), 1_700_000_000),
                firmar_raiz(&kp, "estable", &hash_de(2), 1_700_000_100),
                firmar_raiz(&kp, "estable", &hash_de(3), 1_700_000_200),
            ],
        };
        assert!(verificar_canal(&canal).is_ok());
    }

    #[test]
    fn canal_con_timestamps_no_monotonicos_se_rechaza() {
        let kp = Keypair::from_seed([7; 32]);
        let canal = Canal {
            version: VERSION_CANAL,
            nombre: "estable".into(),
            autor: kp.public_key(),
            raices: vec![
                firmar_raiz(&kp, "estable", &hash_de(1), 1_700_000_100),
                // Esta segunda tiene un timestamp ANTERIOR — replay.
                firmar_raiz(&kp, "estable", &hash_de(2), 1_700_000_050),
            ],
        };
        match verificar_canal(&canal) {
            Err(CanalError::TimestampNoMonotonico { indice: 1, .. }) => {}
            other => panic!("esperaba TimestampNoMonotonico, fue {:?}", other),
        }
    }

    #[test]
    fn canal_con_timestamps_iguales_se_rechaza() {
        // Dos raíces con el mismo segundo — monotonicidad estricta lo
        // rechaza para no ambiguar el orden y para no aceptar replays
        // del mismo instante con un manifest distinto.
        let kp = Keypair::from_seed([7; 32]);
        let canal = Canal {
            version: VERSION_CANAL,
            nombre: "estable".into(),
            autor: kp.public_key(),
            raices: vec![
                firmar_raiz(&kp, "estable", &hash_de(1), 1_700_000_100),
                firmar_raiz(&kp, "estable", &hash_de(2), 1_700_000_100),
            ],
        };
        assert!(matches!(
            verificar_canal(&canal),
            Err(CanalError::TimestampNoMonotonico { .. })
        ));
    }

    #[test]
    fn canal_con_firma_rota_se_detecta_en_el_historial() {
        let kp = Keypair::from_seed([7; 32]);
        let mut canal = Canal {
            version: VERSION_CANAL,
            nombre: "estable".into(),
            autor: kp.public_key(),
            raices: vec![
                firmar_raiz(&kp, "estable", &hash_de(1), 1_700_000_000),
                firmar_raiz(&kp, "estable", &hash_de(2), 1_700_000_100),
            ],
        };
        // Manipular el segundo manifest.
        canal.raices[1].raiz_manifiesto[0] ^= 0x01;
        match verificar_canal(&canal) {
            Err(CanalError::FirmaInvalida { timestamp: 1_700_000_100 }) => {}
            other => panic!("esperaba FirmaInvalida con ts 1_700_000_100, fue {:?}", other),
        }
    }

    #[test]
    fn canal_vacio_es_valido() {
        // Un canal recién creado, sin raíces aún, pasa la verificación
        // por vacuidad. El consumidor decidirá si lo considera "útil"
        // (Canal::vigente devuelve None).
        let kp = Keypair::from_seed([7; 32]);
        let canal = Canal {
            version: VERSION_CANAL,
            nombre: "estable".into(),
            autor: kp.public_key(),
            raices: vec![],
        };
        assert!(verificar_canal(&canal).is_ok());
    }

    #[test]
    fn firmar_para_anuncio_coincide_con_la_firma_de_raiz() {
        // El anuncio (akasha) y el historial (canal) usan exactamente
        // la misma firma sobre el mismo mensaje canónico. Esta prueba
        // lo blinda: si firmar_raiz cambia de mensaje, firmar_para_anuncio
        // también cambia consigo.
        let kp = Keypair::from_seed([7; 32]);
        let hash = hash_de(42);
        let ts = 1_700_000_000;
        let raiz = firmar_raiz(&kp, "estable", &hash, ts);
        let (autor, firma) = firmar_para_anuncio(&kp, "estable", &hash, ts);
        assert_eq!(autor, kp.public_key());
        assert_eq!(firma, raiz.firma);
    }

    #[test]
    fn autor_como_identity_id_coincide_con_keypair() {
        let kp = Keypair::from_seed([7; 32]);
        let canal = Canal {
            version: VERSION_CANAL,
            nombre: "estable".into(),
            autor: kp.public_key(),
            raices: vec![],
        };
        assert_eq!(autor_como_identity_id(&canal), kp.identity_id());
    }
}
