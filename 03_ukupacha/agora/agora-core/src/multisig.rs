//! Firma múltiple M-of-N: agrupa firmas Ed25519 independientes de N
//! atestadores sobre el mismo mensaje, y verifica que **al menos M de
//! ellas** sean válidas.
//!
//! Pensado para identidades de grupo: un canal de release con varios
//! mantenedores, una atestación que sólo cuenta si la respaldan
//! suficientes firmantes pactados, una raíz de manifiesto que exige
//! umbral fuera del `AGORA_AUTH_RING` del kernel.
//!
//! El diseño es deliberadamente simple — no aggregate signatures, no
//! Schnorr-multi, no MuSig. Cada firma es Ed25519 estándar; el "M-of-N"
//! se evalúa contando válidas. Eso pierde compactness (la firma agrupa
//! ~96 bytes por firmante) pero gana auditabilidad: cada firma se
//! puede verificar aislada, sin estado compartido entre firmantes.

use serde::{Deserialize, Serialize};

use crate::identity::{verify_signature, IdentityId};
use crate::AgoraError;

/// Adaptador serde para `[u8; 64]` — serde sólo cubre arrays hasta 32,
/// así que la firma viaja como secuencia y se revalida su largo al leer.
mod sig_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(sig: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        sig.as_slice().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let v = Vec::<u8>::deserialize(d)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom("la firma debe ser de 64 bytes"))
    }
}

/// Una firma individual dentro de un [`MultiSignature`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingleSig {
    /// IdentityId derivado de `public_key` — redundante pero útil para
    /// indexar al verificar contra una whitelist sin re-hashear.
    pub signer: IdentityId,
    /// Clave pública Ed25519 del firmante (32 bytes).
    pub public_key: [u8; 32],
    /// Firma Ed25519 sobre el mensaje (64 bytes).
    #[serde(with = "sig_serde")]
    pub signature: [u8; 64],
}

/// Colección de firmas independientes sobre el **mismo mensaje**.
/// Verifica con umbral M-of-N: cuenta firmas válidas y exige ≥ M.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MultiSignature {
    pub signers: Vec<SingleSig>,
}

/// Resumen de una verificación de [`MultiSignature::verify_threshold`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiSigVerdict {
    /// Firmas válidas: clave bien formada + firma cubre el mensaje +
    /// signer coincide con el id derivado de la pubkey.
    pub validas: usize,
    /// Firmas inválidas por cualquier razón (clave mal, firma rota,
    /// signer no coincide con su pubkey).
    pub invalidas: usize,
    /// Firmantes distintos entre las VÁLIDAS — protege contra inflar
    /// el contador agregando la misma firma N veces.
    pub firmantes_distintos: usize,
}

impl MultiSignature {
    /// Construye una multifirma haciendo que cada keypair firme el
    /// mensaje. El orden de `keypairs` se preserva en `signers`.
    pub fn create(keypairs: &[&crate::identity::Keypair], message: &[u8]) -> Self {
        let signers = keypairs
            .iter()
            .map(|kp| SingleSig {
                signer: kp.identity_id(),
                public_key: kp.public_key(),
                signature: kp.sign(message),
            })
            .collect();
        Self { signers }
    }

    /// Cuenta firmas válidas sobre `message`. Una firma es válida si:
    /// (1) la pubkey es un punto Ed25519 válido, (2) la firma cierra
    /// sobre el mensaje bajo esa pubkey, (3) `signer` matchea el id
    /// derivado de la pubkey.
    ///
    /// El `firmantes_distintos` cuenta IDs únicos entre las válidas —
    /// repetir la misma firma N veces NO cuenta N firmantes.
    pub fn verdict(&self, message: &[u8]) -> MultiSigVerdict {
        let mut validas = 0usize;
        let mut invalidas = 0usize;
        let mut firmantes: std::collections::BTreeSet<IdentityId> =
            std::collections::BTreeSet::new();
        for s in &self.signers {
            let id_de_pubkey = IdentityId::from_public_key(&s.public_key);
            if s.signer != id_de_pubkey {
                invalidas += 1;
                continue;
            }
            match verify_signature(&s.public_key, message, &s.signature) {
                Ok(()) => {
                    validas += 1;
                    firmantes.insert(s.signer);
                }
                Err(_) => {
                    invalidas += 1;
                }
            }
        }
        MultiSigVerdict {
            validas,
            invalidas,
            firmantes_distintos: firmantes.len(),
        }
    }

    /// Verifica con umbral M-of-N. `Err(MultiSigError::DemasiadoPocas)`
    /// si los firmantes distintos válidos < `min`. El conteo usa
    /// firmantes_distintos, no validas — duplicar la misma firma N
    /// veces no engaña al umbral.
    pub fn verify_threshold(
        &self,
        message: &[u8],
        min: usize,
    ) -> Result<MultiSigVerdict, MultiSigError> {
        let v = self.verdict(message);
        if v.firmantes_distintos >= min {
            Ok(v)
        } else {
            Err(MultiSigError::DemasiadoPocas {
                requeridas: min,
                tenidas: v.firmantes_distintos,
            })
        }
    }

    /// Verifica con umbral M-of-N restringido a una whitelist de
    /// signers autorizados. Sólo cuentan las firmas válidas cuyo
    /// `signer` esté en `allowed`. Pensado para el caso "una multifirma
    /// vale si M de los N del anillo soberano la respaldan, no
    /// cualquiera con M claves".
    pub fn verify_threshold_in(
        &self,
        message: &[u8],
        min: usize,
        allowed: &[IdentityId],
    ) -> Result<MultiSigVerdict, MultiSigError> {
        let allow_set: std::collections::BTreeSet<IdentityId> = allowed.iter().copied().collect();
        let mut firmantes: std::collections::BTreeSet<IdentityId> =
            std::collections::BTreeSet::new();
        let mut validas = 0usize;
        let mut invalidas = 0usize;
        for s in &self.signers {
            let id_de_pubkey = IdentityId::from_public_key(&s.public_key);
            if s.signer != id_de_pubkey {
                invalidas += 1;
                continue;
            }
            if !allow_set.contains(&s.signer) {
                // Firmante ajeno al anillo — no es "inválida"
                // matemáticamente, pero no cuenta. Tampoco la contamos
                // como inválida para no inflar ese contador.
                continue;
            }
            match verify_signature(&s.public_key, message, &s.signature) {
                Ok(()) => {
                    validas += 1;
                    firmantes.insert(s.signer);
                }
                Err(_) => invalidas += 1,
            }
        }
        let v = MultiSigVerdict {
            validas,
            invalidas,
            firmantes_distintos: firmantes.len(),
        };
        if v.firmantes_distintos >= min {
            Ok(v)
        } else {
            Err(MultiSigError::DemasiadoPocas {
                requeridas: min,
                tenidas: v.firmantes_distintos,
            })
        }
    }
}

/// Falla de una verificación de multifirma.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MultiSigError {
    #[error("multifirma insuficiente: {tenidas} firmantes distintos válidos, se requerían {requeridas}")]
    DemasiadoPocas { requeridas: usize, tenidas: usize },
}

impl From<AgoraError> for MultiSigError {
    fn from(_: AgoraError) -> Self {
        // Mapeo genérico — los detalles se preservan en `verdict()`.
        Self::DemasiadoPocas { requeridas: 0, tenidas: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Keypair;

    fn kps(n: u8) -> Vec<Keypair> {
        (1..=n).map(|i| Keypair::from_seed([i; 32])).collect()
    }

    #[test]
    fn dos_de_tres_acepta_con_dos_firmas_validas() {
        let pares = kps(3);
        let refs: Vec<&Keypair> = pares.iter().collect();
        let mut multi = MultiSignature::create(&refs, b"raiz canonica");
        // Rompe la firma de uno de los tres — sólo quedan 2 válidas.
        multi.signers[1].signature[0] ^= 0xFF;
        let v = multi.verify_threshold(b"raiz canonica", 2).unwrap();
        assert_eq!(v.validas, 2);
        assert_eq!(v.invalidas, 1);
        assert_eq!(v.firmantes_distintos, 2);
    }

    #[test]
    fn dos_de_tres_rechaza_con_una_sola_valida() {
        let pares = kps(3);
        let refs: Vec<&Keypair> = pares.iter().collect();
        let mut multi = MultiSignature::create(&refs, b"x");
        // Romper dos firmas: sólo 1 queda válida.
        multi.signers[0].signature[0] ^= 0xFF;
        multi.signers[1].signature[0] ^= 0xFF;
        let err = multi.verify_threshold(b"x", 2).unwrap_err();
        assert!(matches!(
            err,
            MultiSigError::DemasiadoPocas { requeridas: 2, tenidas: 1 }
        ));
    }

    #[test]
    fn replay_de_la_misma_firma_no_infla_el_conteo() {
        // Atacante construye una multifirma con la misma firma válida
        // tres veces. firmantes_distintos debe ser 1, no 3.
        let kp = Keypair::from_seed([7; 32]);
        let s = SingleSig {
            signer: kp.identity_id(),
            public_key: kp.public_key(),
            signature: kp.sign(b"msg"),
        };
        let multi = MultiSignature {
            signers: vec![s.clone(), s.clone(), s],
        };
        let v = multi.verdict(b"msg");
        assert_eq!(v.validas, 3);
        assert_eq!(v.firmantes_distintos, 1);
        assert!(multi.verify_threshold(b"msg", 2).is_err());
    }

    #[test]
    fn signer_mismatch_es_invalida() {
        let kp_real = Keypair::from_seed([7; 32]);
        let kp_otro = Keypair::from_seed([8; 32]);
        // Construir firma con la pubkey real pero declarar otro signer.
        let s = SingleSig {
            signer: kp_otro.identity_id(), // mismatch deliberado
            public_key: kp_real.public_key(),
            signature: kp_real.sign(b"msg"),
        };
        let multi = MultiSignature { signers: vec![s] };
        let v = multi.verdict(b"msg");
        assert_eq!(v.validas, 0);
        assert_eq!(v.invalidas, 1);
    }

    #[test]
    fn mensaje_distinto_invalida_todas() {
        let pares = kps(3);
        let refs: Vec<&Keypair> = pares.iter().collect();
        let multi = MultiSignature::create(&refs, b"original");
        let v = multi.verdict(b"manipulado");
        assert_eq!(v.validas, 0);
        assert_eq!(v.invalidas, 3);
    }

    #[test]
    fn threshold_en_lista_blanca_solo_cuenta_firmantes_permitidos() {
        // Tres firmantes: a, b, c. Whitelist = [a, b]. Aunque c firme
        // válidamente, su firma no entra en el conteo.
        let pares = kps(3);
        let refs: Vec<&Keypair> = pares.iter().collect();
        let multi = MultiSignature::create(&refs, b"x");
        let allowed = vec![pares[0].identity_id(), pares[1].identity_id()];
        let v = multi.verify_threshold_in(b"x", 2, &allowed).unwrap();
        assert_eq!(v.firmantes_distintos, 2);
        // Con whitelist sólo a, c — c firmó pero no está → < 2, rechaza.
        let allowed_chico = vec![pares[0].identity_id(), pares[2].identity_id()];
        let v2 = multi.verify_threshold_in(b"x", 2, &allowed_chico).unwrap();
        // Tanto a como c están en allowed y firmaron OK → 2 distintos.
        assert_eq!(v2.firmantes_distintos, 2);
        // Pero si exigimos 3 y sólo permitimos [a, c], faltan firmantes.
        assert!(multi.verify_threshold_in(b"x", 3, &allowed_chico).is_err());
    }

    #[test]
    fn multifirma_serializa_y_round_trippea_postcard() {
        // Aseguramos que MultiSignature sobrevive un round-trip postcard
        // — clave para mandarla por gossip o persistirla.
        let pares = kps(3);
        let refs: Vec<&Keypair> = pares.iter().collect();
        let multi = MultiSignature::create(&refs, b"raiz");
        let bytes = postcard::to_allocvec(&multi).unwrap();
        let back: MultiSignature = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, multi);
        let v = back.verify_threshold(b"raiz", 3).unwrap();
        assert_eq!(v.firmantes_distintos, 3);
    }
}
