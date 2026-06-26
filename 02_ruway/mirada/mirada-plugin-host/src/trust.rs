//! Confianza por firma: un plugin que pide capacidades *peligrosas* (cualquiera
//! más allá de `layout`) debe traer una firma Ed25519 sobre `blake3(wasm) ‖
//! caps`, por una clave que el usuario declaró de confianza en su `trust.ron`.
//!
//! Es la forma de `ConcesionCapacidad` del kernel wawa (firma sobre
//! `hash_bytecode ‖ permisos`), pero con el anillo de confianza **del usuario**,
//! no el del kernel: la soberanía la ejerce quien corre el escritorio, decidiendo
//! a quién le cree los plugins. Manipular el `.wasm` o escalar las caps cambia el
//! mensaje firmado y rompe la verificación.

use std::path::Path;

use serde::Deserialize;

use crate::caps::{caps_list, CapsPlugin, CAP_LAYOUT};

/// La firma que autoriza las capacidades de un plugin: quién firmó y los 64
/// bytes de la firma Ed25519 sobre el mensaje del grant.
#[derive(Debug, Clone)]
pub struct Grant {
    pub signer: [u8; 32],
    pub signature: [u8; 64],
}

/// El anillo de claves públicas en las que el usuario confía.
#[derive(Debug, Clone, Default)]
pub struct TrustSet {
    keys: Vec<[u8; 32]>,
}

#[derive(Deserialize)]
struct TrustFile {
    #[serde(default)]
    trusted: Vec<String>,
}

impl TrustSet {
    /// Un anillo vacío: no confía en nadie (sólo pasan los plugins de cero
    /// capacidades peligrosas, p. ej. los de layout puro).
    pub fn empty() -> Self {
        Self::default()
    }

    /// `true` si `key` está en el anillo.
    pub fn contains(&self, key: &[u8; 32]) -> bool {
        self.keys.contains(key)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Carga el anillo desde un `trust.ron` (`( trusted: ["ed25519:hex…"] )`).
    /// Un archivo ausente o ilegible deja el anillo vacío (con aviso) — postura
    /// fail-closed: sin confianza declarada, ningún plugin peligroso carga.
    pub fn load(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return Self::empty(),
        };
        let parsed: TrustFile = match ron::from_str(&text) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[host] trust.ron inválido ({}): {e}", path.display());
                return Self::empty();
            }
        };
        let mut keys = Vec::new();
        for s in &parsed.trusted {
            match parse_pubkey(s) {
                Some(k) => keys.push(k),
                None => eprintln!("[host] clave de confianza ilegible: {s:?}"),
            }
        }
        Self { keys }
    }

    /// Construye un anillo a partir de claves ya decodificadas (tests/embebido).
    pub fn from_keys(keys: Vec<[u8; 32]>) -> Self {
        Self { keys }
    }
}

/// Las capacidades que **exigen firma**: todas menos `layout` (que es puro y no
/// enlaza ninguna importación del host).
pub fn requires_signature(caps: CapsPlugin) -> bool {
    caps & !CAP_LAYOUT != 0
}

/// El mensaje que se firma: `blake3(wasm)` (32 bytes) ‖ `caps` en little-endian
/// (4 bytes). Espeja `mensaje_capacidad` de wawa.
pub fn grant_message(wasm: &[u8], caps: CapsPlugin) -> [u8; 36] {
    let hash = blake3::hash(wasm);
    let mut msg = [0u8; 36];
    msg[..32].copy_from_slice(hash.as_bytes());
    msg[32..].copy_from_slice(&caps.to_le_bytes());
    msg
}

/// Autoriza las capacidades concedidas de un plugin. Si pide alguna peligrosa,
/// exige un `Grant` cuyo firmante esté en el anillo y cuya firma valide sobre
/// `blake3(wasm) ‖ caps`. Layout puro pasa sin firma.
pub fn authorize(
    wasm: &[u8],
    granted: CapsPlugin,
    grant: Option<&Grant>,
    trust: &TrustSet,
) -> Result<(), String> {
    if !requires_signature(granted) {
        return Ok(());
    }
    let grant = grant.ok_or_else(|| {
        format!(
            "el plugin pide {} pero no viene firmado (firma requerida para caps peligrosas)",
            caps_list(granted & !CAP_LAYOUT)
        )
    })?;
    if !trust.contains(&grant.signer) {
        return Err(format!(
            "el firmante no está en el anillo de confianza: ed25519:{}",
            hex::encode(grant.signer)
        ));
    }
    let msg = grant_message(wasm, granted);
    verify(&grant.signer, &msg, &grant.signature)
        .map_err(|_| "firma inválida — el .wasm o las caps fueron manipulados".to_string())
}

/// Verifica una firma Ed25519 (misma mecánica que `agora-core::verify_signature`).
pub fn verify(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> Result<(), ()> {
    use ed25519_dalek::Verifier;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(public_key).map_err(|_| ())?;
    let sig = ed25519_dalek::Signature::from_bytes(signature);
    vk.verify(message, &sig).map_err(|_| ())
}

/// Decodifica una clave pública `"ed25519:hex64"` (o hex pelado) a 32 bytes.
pub fn parse_pubkey(s: &str) -> Option<[u8; 32]> {
    let h = s.trim().strip_prefix("ed25519:").unwrap_or(s.trim());
    let bytes = hex::decode(h).ok()?;
    bytes.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::{CAP_KEYS, CAP_SPAWN};
    use ed25519_dalek::{Signer, SigningKey};

    /// Par de claves determinista desde una semilla, + firma del grant.
    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }
    fn sign(sk: &SigningKey, wasm: &[u8], caps: CapsPlugin) -> Grant {
        Grant {
            signer: sk.verifying_key().to_bytes(),
            signature: sk.sign(&grant_message(wasm, caps)).to_bytes(),
        }
    }

    #[test]
    fn layout_puro_no_necesita_firma() {
        assert!(authorize(b"wasm", CAP_LAYOUT, None, &TrustSet::empty()).is_ok());
    }

    #[test]
    fn caps_peligrosas_sin_firma_se_rechazan() {
        let r = authorize(b"wasm", CAP_KEYS, None, &TrustSet::empty());
        assert!(r.is_err());
    }

    #[test]
    fn firma_valida_de_clave_de_confianza_pasa() {
        let sk = key(7);
        let wasm = b"el bytecode del plugin";
        let caps = CAP_KEYS | CAP_SPAWN;
        let grant = sign(&sk, wasm, caps);
        let trust = TrustSet::from_keys(vec![sk.verifying_key().to_bytes()]);
        assert!(authorize(wasm, caps, Some(&grant), &trust).is_ok());
    }

    #[test]
    fn wasm_manipulado_rompe_la_firma() {
        let sk = key(7);
        let caps = CAP_SPAWN;
        let grant = sign(&sk, b"original", caps);
        let trust = TrustSet::from_keys(vec![sk.verifying_key().to_bytes()]);
        // Mismo grant, pero el wasm cambió: el mensaje firmado ya no casa.
        assert!(authorize(b"MANIPULADO", caps, Some(&grant), &trust).is_err());
    }

    #[test]
    fn escalar_caps_rompe_la_firma() {
        let sk = key(7);
        let wasm = b"plugin";
        // Firmado para sólo KEYS…
        let grant = sign(&sk, wasm, CAP_KEYS);
        let trust = TrustSet::from_keys(vec![sk.verifying_key().to_bytes()]);
        // …pero el manifest pide KEYS|SPAWN: el mensaje difiere → rechazo.
        assert!(authorize(wasm, CAP_KEYS | CAP_SPAWN, Some(&grant), &trust).is_err());
    }

    #[test]
    fn firmante_fuera_del_anillo_se_rechaza() {
        let sk = key(7);
        let wasm = b"plugin";
        let caps = CAP_SPAWN;
        let grant = sign(&sk, wasm, caps); // firma válida…
        let otra = key(9).verifying_key().to_bytes();
        let trust = TrustSet::from_keys(vec![otra]); // …pero confío en OTRA clave.
        assert!(authorize(wasm, caps, Some(&grant), &trust).is_err());
    }
}
