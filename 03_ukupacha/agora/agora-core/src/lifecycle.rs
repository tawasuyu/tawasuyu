//! Ciclo de vida de claves — rotación y revocación.
//!
//! Dos records que cubren los dos modos en que una clave deja de ser la que
//! era, con autoridades deliberadamente distintas (ver `SDD-rotacion-revocacion.md`):
//!
//! - [`KeyRotation`]: handoff VOLUNTARIO vieja→nueva. Doble-firmado (la vieja
//!   autoriza, la nueva acepta) — se auto-autoriza con la clave vieja viva, NO
//!   es M-of-N. Para cuando rotás antes de que nadie te comprometa.
//! - [`Revocation`]: la clave se RETIRA o se COMPROMETIÓ. Firmada por un quórum
//!   M-of-N de un set autorizador (el `AGORA_AUTH_RING` en el plano de control,
//!   los guardianes en el social), porque una clave comprometida no puede
//!   revocarse a sí misma — el atacante también la tiene.
//!
//! Ambos records son `agora-core` puro (`std` + `ed25519-dalek`). El kernel no
//! los enlaza: espeja la verificación con `ed25519-compact` sobre los MISMOS
//! bytes canónicos definidos aquí.

use serde::{Deserialize, Serialize};

use crate::identity::{verify_signature, IdentityId, Keypair};
use crate::multisig::{MultiSigError, MultiSigVerdict, MultiSignature};
use crate::AgoraError;

/// Adaptador serde para `[u8; 64]` — serde sólo cubre arrays hasta 32.
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

// =============================================================================
//  Rotación
// =============================================================================

/// Una rotación de clave: la identidad detrás de `old_key` declara que su clave
/// pasa a ser `new_key`. Doble-firmada para probar posesión de AMBAS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRotation {
    /// Clave que se retira (la que firmaba hasta ahora).
    pub old_key: [u8; 32],
    /// Clave que la sucede.
    pub new_key: [u8; 32],
    /// Segundos UNIX de la rotación. Ordena la cadena de sucesión.
    pub issued_at: u64,
    /// Firma de `old_key` sobre el canónico: AUTORIZA el handoff.
    #[serde(with = "sig_serde")]
    pub sig_old: [u8; 64],
    /// Firma de `new_key` sobre el canónico: ACEPTA la sucesión (prueba
    /// posesión — nadie ata la clave de otro como su sucesora).
    #[serde(with = "sig_serde")]
    pub sig_new: [u8; 64],
}

impl KeyRotation {
    /// El mensaje exacto que ambas claves firman. Tamaños fijos ⇒ sin prefijos
    /// de largo; el dominio separa de otros records. La composición la hace
    /// `format::mensaje_rotacion_clave` — la MISMA verdad que el kernel espeja.
    pub fn canonical_bytes(old_key: &[u8; 32], new_key: &[u8; 32], issued_at: u64) -> Vec<u8> {
        format::mensaje_rotacion_clave(old_key, new_key, issued_at)
    }

    /// Forja una rotación firmando el canónico con la clave vieja y la nueva.
    pub fn create(old: &Keypair, new: &Keypair, issued_at: u64) -> Self {
        let old_key = old.public_key();
        let new_key = new.public_key();
        let msg = Self::canonical_bytes(&old_key, &new_key, issued_at);
        Self {
            old_key,
            new_key,
            issued_at,
            sig_old: old.sign(&msg),
            sig_new: new.sign(&msg),
        }
    }

    /// Verifica que AMBAS firmas cierren bajo sus claves. Una sola que falle
    /// invalida la rotación entera — el handoff exige consentimiento de las dos
    /// puntas.
    pub fn verify(&self) -> Result<(), AgoraError> {
        let msg = Self::canonical_bytes(&self.old_key, &self.new_key, self.issued_at);
        verify_signature(&self.old_key, &msg, &self.sig_old)?;
        verify_signature(&self.new_key, &msg, &self.sig_new)
    }

    /// `IdentityId` de la clave que se retira.
    pub fn old_id(&self) -> IdentityId {
        IdentityId::from_public_key(&self.old_key)
    }

    /// `IdentityId` de la clave sucesora.
    pub fn new_id(&self) -> IdentityId {
        IdentityId::from_public_key(&self.new_key)
    }
}

// =============================================================================
//  Revocación
// =============================================================================

/// Por qué se revoca una clave. El discriminante entra en el canónico: cambiar
/// el motivo invalida la firma (no se puede "ascender" un retiro a compromiso).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevReason {
    /// La clave se filtró / está en manos hostiles. Revocación PERMANENTE,
    /// M-of-N de OTROS (la clave comprometida no se revoca a sí misma).
    Compromised,
    /// Retiro voluntario, sin compromiso. Puede ser self-signed.
    Retired,
    /// Reemplazada por una sucesora vía [`KeyRotation`] — la vieja se apaga.
    Superseded,
}

impl RevReason {
    /// Byte canónico estable (independiente del orden serde).
    fn byte(self) -> u8 {
        match self {
            RevReason::Compromised => 0,
            RevReason::Retired => 1,
            RevReason::Superseded => 2,
        }
    }
}

/// Una revocación firmada por un quórum. Apaga `target_key`: a partir de
/// `issued_at` (y hasta `expires_at`, si lo hay) la clave deja de valer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revocation {
    /// La clave que se revoca.
    pub target_key: [u8; 32],
    /// Motivo (entra en la firma).
    pub reason: RevReason,
    /// Segundos UNIX desde cuándo rige.
    pub issued_at: u64,
    /// `None` ⇒ revocación PERMANENTE. `Some(t)` ⇒ suspensión temporal que
    /// vence en `t` (la clave vuelve a valer después — caducidad estricta).
    pub expires_at: Option<u64>,
    /// Las firmas del set autorizador. El umbral M y el set permitido los pone
    /// el verificador ([`Self::verify`]) — el record no los embebe.
    pub authorizers: MultiSignature,
}

impl Revocation {
    /// El mensaje canónico que el quórum firma. `expires_at` viaja con un tag
    /// (0 = none, 1 = some) para que `None` y `Some(0)` no colisionen. La
    /// composición la hace `format::mensaje_revocacion_clave` — la MISMA verdad
    /// que el kernel espeja en `claves::verificar_revocacion`.
    pub fn canonical_bytes(
        target_key: &[u8; 32],
        reason: RevReason,
        issued_at: u64,
        expires_at: Option<u64>,
    ) -> Vec<u8> {
        format::mensaje_revocacion_clave(target_key, reason.byte(), issued_at, expires_at)
    }

    /// Forja una revocación haciendo que cada `authorizer` firme el canónico.
    /// El caller pasa el set de quienes co-firman; el umbral se exige al
    /// verificar, no al crear.
    pub fn create(
        target_key: [u8; 32],
        reason: RevReason,
        issued_at: u64,
        expires_at: Option<u64>,
        authorizers: &[&Keypair],
    ) -> Self {
        let msg = Self::canonical_bytes(&target_key, reason, issued_at, expires_at);
        Self {
            target_key,
            reason,
            issued_at,
            expires_at,
            authorizers: MultiSignature::create(authorizers, &msg),
        }
    }

    /// Verifica que ≥ `min` firmantes DISTINTOS del set `allowed` respalden la
    /// revocación. Restringe al set autorizador (anillo o guardianes): M claves
    /// cualesquiera no bastan, tienen que ser del set. Una firma del propio
    /// `target_key` cuenta sólo si `target_key` está en `allowed` — pero para
    /// `Compromised` el caller debe excluir al target del set (no se revoca solo).
    pub fn verify(
        &self,
        min: usize,
        allowed: &[IdentityId],
    ) -> Result<MultiSigVerdict, MultiSigError> {
        let msg =
            Self::canonical_bytes(&self.target_key, self.reason, self.issued_at, self.expires_at);
        self.authorizers.verify_threshold_in(&msg, min, allowed)
    }

    /// `true` si TODAS las firmas presentes son criptográficamente reales (cada
    /// firmante cubre el canónico y su id deriva de su pubkey) y hay al menos
    /// una. NO mira umbral ni set autorizador — es el bar de INTEGRIDAD de firma
    /// (no se puede fabricar una revocación firmada por claves que no tenés),
    /// análogo a `Attestation::verify`. La autoridad M-of-N la decide quien la
    /// consume con [`Self::verify`]. Lo usa la persistencia para re-verificar al
    /// recargar sin conocer el set autorizador.
    pub fn signatures_valid(&self) -> bool {
        let msg =
            Self::canonical_bytes(&self.target_key, self.reason, self.issued_at, self.expires_at);
        let v = self.authorizers.verdict(&msg);
        v.invalidas == 0 && v.firmantes_distintos >= 1
    }

    /// `true` si la revocación está VIGENTE en `now`: ya empezó (`issued_at <=
    /// now`) y no venció (`expires_at` ausente, o `now < expires_at`). No
    /// verifica firmas — eso es [`Self::verify`]; esto es la ventana temporal.
    pub fn is_active_at(&self, now: u64) -> bool {
        if now < self.issued_at {
            return false;
        }
        match self.expires_at {
            None => true,
            Some(t) => now < t,
        }
    }

    /// `IdentityId` de la clave revocada.
    pub fn target_id(&self) -> IdentityId {
        IdentityId::from_public_key(&self.target_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Keypair;

    fn kp(seed: u8) -> Keypair {
        Keypair::from_seed([seed; 32])
    }

    // --- Rotación -----------------------------------------------------------

    #[test]
    fn rotacion_doble_firmada_verifica() {
        let vieja = kp(1);
        let nueva = kp(2);
        let r = KeyRotation::create(&vieja, &nueva, 1_700_000_000);
        assert!(r.verify().is_ok());
        assert_eq!(r.old_id(), vieja.identity_id());
        assert_eq!(r.new_id(), nueva.identity_id());
    }

    #[test]
    fn rotacion_con_una_firma_rota_falla() {
        let vieja = kp(1);
        let nueva = kp(2);
        let mut r = KeyRotation::create(&vieja, &nueva, 1_700_000_000);
        // Corromper la firma de la nueva: el handoff ya no prueba posesión.
        r.sig_new[0] ^= 0xFF;
        assert!(r.verify().is_err());
    }

    #[test]
    fn rotacion_no_acepta_sucesora_ajena_sin_su_firma() {
        // Un atacante arma una rotación con su propia clave como `new_key`
        // pero firma `sig_new` con OTRA clave: debe fallar.
        let vieja = kp(1);
        let suplantada = kp(2);
        let atacante = kp(3);
        let issued_at = 42;
        let msg = KeyRotation::canonical_bytes(
            &vieja.public_key(),
            &suplantada.public_key(),
            issued_at,
        );
        let falsa = KeyRotation {
            old_key: vieja.public_key(),
            new_key: suplantada.public_key(),
            issued_at,
            sig_old: vieja.sign(&msg),
            sig_new: atacante.sign(&msg), // firma de quien NO es new_key
        };
        assert!(falsa.verify().is_err());
    }

    #[test]
    fn rotacion_distinto_timestamp_distinto_canonico() {
        let v = kp(1);
        let n = kp(2);
        let a = KeyRotation::canonical_bytes(&v.public_key(), &n.public_key(), 1);
        let b = KeyRotation::canonical_bytes(&v.public_key(), &n.public_key(), 2);
        assert_ne!(a, b);
    }

    // --- Revocación ---------------------------------------------------------

    #[test]
    fn revocacion_quorum_2_de_3_verifica() {
        let anillo = [kp(10), kp(11), kp(12)];
        let allowed: Vec<IdentityId> = anillo.iter().map(|k| k.identity_id()).collect();
        let target = kp(99).public_key();
        // Dos miembros del anillo co-firman.
        let rev = Revocation::create(
            target,
            RevReason::Compromised,
            1_700_000_000,
            None,
            &[&anillo[0], &anillo[1]],
        );
        assert!(rev.verify(2, &allowed).is_ok());
        // Con umbral 3 no alcanza.
        assert!(rev.verify(3, &allowed).is_err());
    }

    #[test]
    fn revocacion_firmante_fuera_del_set_no_cuenta() {
        let anillo = [kp(10), kp(11), kp(12)];
        let allowed: Vec<IdentityId> = anillo.iter().map(|k| k.identity_id()).collect();
        let forastero = kp(50);
        let target = kp(99).public_key();
        // Un miembro + un forastero: sólo cuenta uno → 2-of-N falla.
        let rev = Revocation::create(
            target,
            RevReason::Compromised,
            1,
            None,
            &[&anillo[0], &forastero],
        );
        assert!(rev.verify(2, &allowed).is_err());
        assert!(rev.verify(1, &allowed).is_ok());
    }

    #[test]
    fn revocacion_mensaje_alterado_invalida_firmas() {
        let anillo = [kp(10), kp(11)];
        let allowed: Vec<IdentityId> = anillo.iter().map(|k| k.identity_id()).collect();
        let target = kp(99).public_key();
        let mut rev = Revocation::create(
            target,
            RevReason::Compromised,
            100,
            None,
            &[&anillo[0], &anillo[1]],
        );
        // Subir el motivo a otro cambia el canónico → las firmas no cierran.
        rev.reason = RevReason::Retired;
        assert!(rev.verify(2, &allowed).is_err());
    }

    #[test]
    fn revocacion_permanente_vs_temporal() {
        let anillo = [kp(10), kp(11)];
        let target = kp(99).public_key();
        let permanente =
            Revocation::create(target, RevReason::Compromised, 100, None, &[&anillo[0]]);
        assert!(!permanente.is_active_at(50)); // antes de empezar
        assert!(permanente.is_active_at(100));
        assert!(permanente.is_active_at(u64::MAX)); // nunca vence

        let temporal = Revocation::create(
            target,
            RevReason::Compromised,
            100,
            Some(200),
            &[&anillo[0]],
        );
        assert!(temporal.is_active_at(150));
        assert!(!temporal.is_active_at(200)); // vencida (límite exclusivo)
        assert!(!temporal.is_active_at(250));
    }

    #[test]
    fn signatures_valid_acepta_firmas_reales_rechaza_forjadas() {
        let anillo = [kp(10), kp(11)];
        let target = kp(99).public_key();
        let rev = Revocation::create(target, RevReason::Compromised, 1, None, &[&anillo[0], &anillo[1]]);
        // Integridad de firma OK, sin conocer el set autorizador.
        assert!(rev.signatures_valid());
        // Una firma corrompida ⇒ una `invalida` presente ⇒ rechaza.
        let mut roto = rev.clone();
        roto.authorizers.signers[0].signature[0] ^= 0xFF;
        assert!(!roto.signatures_valid());
        // Sin firmantes ⇒ rechaza (nada que respaldar).
        let mut vacio = rev;
        vacio.authorizers.signers.clear();
        assert!(!vacio.signatures_valid());
    }

    #[test]
    fn revocacion_none_y_some_cero_no_colisionan() {
        let target = kp(99).public_key();
        let a = Revocation::canonical_bytes(&target, RevReason::Retired, 5, None);
        let b = Revocation::canonical_bytes(&target, RevReason::Retired, 5, Some(0));
        assert_ne!(a, b);
    }
}
