//! El **manifiesto** — el índice de paquetes que hammer no tiene y que un
//! instalador/actualizador sí necesita. Una `Unit` por app/componente, con su
//! versión y el hash BLAKE3 de su binario; el `Manifest` entero se firma con
//! ed25519 (vía `agora-core`) para que el cliente verifique procedencia antes
//! de instalar/actualizar.
//!
//! El modelo es deliberadamente compatible con el de hammer (CAS + ed25519):
//! `Unit.bin_hash` es un [`ArtifactHash`] `b3:…`, la firma es ed25519. Así, el
//! día que tawasuyu y hammer compartan CAS, este índice se mapea sin fricción.

use serde::{Deserialize, Serialize};

use crate::hash::ArtifactHash;

/// Alcance de una unidad: condiciona dónde se instala y si pide root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// App de usuario: instalable en `~/.local` sin privilegios.
    App,
    /// Componente del sistema (init `arje`, servicios): exige instalación
    /// con root en un prefix del sistema (`/usr/local`).
    System,
}

impl Default for Scope {
    fn default() -> Self {
        Scope::App
    }
}

/// Una unidad instalable: una app de la suite o un componente del sistema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    /// Id estable (el de `app-bus` para las apps).
    pub id: String,
    /// Nombre legible.
    pub label: String,
    /// Versión semántica (hoy, la de la suite).
    pub version: String,
    /// Cuadrante / agrupador para la grilla (`ruway`/`yachay`/…) o `sistema`.
    pub category: String,
    /// Glyph unicode o nombre de ícono freedesktop.
    pub icon: String,
    /// Una línea de descripción para el catálogo.
    pub description: String,
    /// Nombre del binario ejecutable (el `program` de `Launch::Exec`).
    pub program: String,
    /// Alcance (app de usuario / componente del sistema).
    #[serde(default)]
    pub scope: Scope,
    /// Hash BLAKE3 del binario precompilado, si el manifiesto lo conoce
    /// (lado bundle/repo). `None` cuando la unidad se compila desde fuente.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin_hash: Option<ArtifactHash>,
    /// Tamaño del binario en bytes (para mostrar en la UI), si se conoce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

impl Unit {
    pub fn requires_root(&self) -> bool {
        self.scope == Scope::System
    }
}

/// El índice completo: versión de esquema, versión de la suite y las unidades.
/// Es lo que se firma (sin la firma adentro — ver [`SignedManifest`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Versión del esquema de este archivo (empezamos en 1).
    pub schema: u32,
    /// Versión de la suite que describe este manifiesto.
    pub suite_version: String,
    /// Las unidades instalables.
    pub units: Vec<Unit>,
}

impl Manifest {
    pub fn new(suite_version: impl Into<String>, units: Vec<Unit>) -> Self {
        Self { schema: 1, suite_version: suite_version.into(), units }
    }

    pub fn get(&self, id: &str) -> Option<&Unit> {
        self.units.iter().find(|u| u.id == id)
    }

    /// Bytes canónicos sobre los que se calcula la firma. `serde_json` emite
    /// los campos de struct en orden de declaración y los `Vec` en orden, sin
    /// mapas no-ordenados de por medio: determinista para una misma versión de
    /// los tipos. Es lo que firma el publicador y reconstruye el verificador.
    pub fn signing_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Manifest siempre serializa")
    }

    /// Firma este manifiesto con un par de claves `agora`.
    pub fn sign(&self, keypair: &agora_core::Keypair) -> SignedManifest {
        let bytes = self.signing_bytes();
        let signature = keypair.sign(&bytes);
        SignedManifest {
            manifest: self.clone(),
            pubkey: hex32(&keypair.public_key()),
            signature: hex64(&signature),
        }
    }
}

/// El manifiesto + su firma ed25519. Es lo que viaja por la red / vive en el
/// bundle. La clave pública va incluida para que el cliente sepa **quién**
/// firmó; si además la confía (clave anclada), la firma vale como confianza.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedManifest {
    pub manifest: Manifest,
    /// Clave pública del firmante (hex de 32 bytes).
    pub pubkey: String,
    /// Firma ed25519 (hex de 64 bytes) sobre `manifest.signing_bytes()`.
    pub signature: String,
}

/// Por qué no se confió en un manifiesto firmado.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("clave pública o firma con formato inválido")]
    BadEncoding,
    #[error("la firma no corresponde al manifiesto")]
    BadSignature,
    #[error("firmado por una clave que no está en la lista de confianza")]
    Untrusted,
}

impl SignedManifest {
    /// Verifica la firma. Si `trusted` es `Some`, además exige que la clave
    /// firmante sea exactamente esa (clave anclada). Si es `None`, sólo
    /// comprueba que la firma sea válida para la clave declarada
    /// (autoconsistente, sin anclar confianza).
    pub fn verify(&self, trusted: Option<&[u8; 32]>) -> Result<(), VerifyError> {
        let pubkey = unhex32(&self.pubkey).ok_or(VerifyError::BadEncoding)?;
        let signature = unhex64(&self.signature).ok_or(VerifyError::BadEncoding)?;
        if let Some(anchor) = trusted {
            if &pubkey != anchor {
                return Err(VerifyError::Untrusted);
            }
        }
        agora_core::verify_signature(&pubkey, &self.manifest.signing_bytes(), &signature)
            .map_err(|_| VerifyError::BadSignature)
    }

    /// Serializa a JSON legible para escribir en el bundle / servir por HTTP.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("SignedManifest serializa")
    }

    pub fn from_json(src: &str) -> Option<Self> {
        serde_json::from_str(src).ok()
    }
}

fn hex32(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn hex64(b: &[u8; 64]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}
fn unhex32(s: &str) -> Option<[u8; 32]> {
    unhex(s)?.try_into().ok()
}
fn unhex64(s: &str) -> Option<[u8; 64]> {
    let v = unhex(s)?;
    let mut out = [0u8; 64];
    if v.len() != 64 {
        return None;
    }
    out.copy_from_slice(&v);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unidad() -> Unit {
        Unit {
            id: "nada".into(),
            label: "Nada".into(),
            version: "0.1.0".into(),
            category: "ruway".into(),
            icon: "≡".into(),
            description: "Editor de archivos".into(),
            program: "nada".into(),
            scope: Scope::App,
            bin_hash: Some(ArtifactHash::of_bytes(b"binario falso")),
            size_bytes: Some(123),
        }
    }

    #[test]
    fn firma_y_verifica_roundtrip() {
        let kp = agora_core::Keypair::from_seed([9u8; 32]);
        let man = Manifest::new("2026.06", vec![unidad()]);
        let signed = man.sign(&kp);
        // Autoconsistente.
        assert!(signed.verify(None).is_ok());
        // Anclada a la clave correcta.
        assert!(signed.verify(Some(&kp.public_key())).is_ok());
        // Anclada a otra clave → no confiada.
        let otra = agora_core::Keypair::from_seed([1u8; 32]).public_key();
        assert_eq!(signed.verify(Some(&otra)), Err(VerifyError::Untrusted));
    }

    #[test]
    fn manifiesto_alterado_invalida_firma() {
        let kp = agora_core::Keypair::from_seed([9u8; 32]);
        let man = Manifest::new("2026.06", vec![unidad()]);
        let mut signed = man.sign(&kp);
        // Manipular la versión rompe la firma.
        signed.manifest.suite_version = "9999.99".into();
        assert_eq!(signed.verify(None), Err(VerifyError::BadSignature));
    }

    #[test]
    fn json_roundtrip() {
        let kp = agora_core::Keypair::from_seed([3u8; 32]);
        let signed = Manifest::new("2026.06", vec![unidad()]).sign(&kp);
        let json = signed.to_json();
        let back = SignedManifest::from_json(&json).expect("parsea");
        assert_eq!(signed, back);
        assert!(back.verify(None).is_ok());
    }
}
