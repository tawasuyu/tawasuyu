//! La identidad del usuario y la firma de nodos.
//!
//! Un nodo de Ayni se direcciona por su id (BLAKE3 de su contenido) y se firma
//! sobre ese id. `ayni-core` no sabe firmar —es cripto-agnóstico—; aquí está la
//! [`Identidad`] que sí: envuelve un `agora_core::Keypair` Ed25519 y expone los
//! dos closures que el núcleo pide, [`Identidad::firmar`] y [`verificar_firma`].
//!
//! La identidad agora del autor (`AgoraId`) ES su clave pública Ed25519 cruda
//! (32 bytes) —la misma convención que `format::AgoraId`—, de modo que un
//! receptor verifica la firma directamente contra el `autor` que el nodo
//! declara, sin necesitar un directorio: la prueba viaja con el mensaje.

use agora_core::{verify_signature, IdentityId, Keypair};
use ayni_core::{AgoraId, Firma, Hash};
use rand::RngCore;

use crate::ErrorCripto;

/// La identidad firmante del usuario local: su par de claves Ed25519 y un
/// nombre de presentación. La clave privada vive sólo aquí, en memoria; nunca
/// se serializa ni viaja (cuando se persiste, es vía el keystore cifrado de
/// agora, ver [`Identidad::guardar_en_keystore`]).
pub struct Identidad {
    keypair: Keypair,
    nombre: String,
    /// La semilla de 32 bytes que engendra el par. Se retiene (en memoria, como
    /// la clave privada) porque de ella se deriva TAMBIÉN el par X25519 del
    /// cifrado E2EE (ver [`Identidad::clave_publica_x25519`]): una sola raíz
    /// para firmar y cifrar.
    seed: [u8; 32],
}

impl Identidad {
    /// Crea una identidad nueva con entropía del CSPRNG del sistema. La semilla
    /// se descarta tras derivar el par; para persistirla, usar
    /// [`Identidad::guardar_en_keystore`] (que la re-genera no es posible —el
    /// keystore guarda la semilla en el momento de crear, ver el flujo en
    /// `ayni-cli`).
    pub fn nueva_aleatoria(nombre: impl Into<String>) -> (Self, [u8; 32]) {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        (Self::desde_semilla(seed, nombre), seed)
    }

    /// Deriva una identidad de una semilla de 32 bytes. Determinista —misma
    /// semilla, misma identidad—: la usan los tests y la derivación reproducible.
    /// Para una identidad real la semilla debe venir de un CSPRNG (ver
    /// [`Identidad::nueva_aleatoria`]).
    pub fn desde_semilla(seed: [u8; 32], nombre: impl Into<String>) -> Self {
        Identidad {
            keypair: Keypair::from_seed(seed),
            nombre: nombre.into(),
            seed,
        }
    }

    /// Carga la identidad del keystore cifrado de agora, descifrando su semilla
    /// con la passphrase. `id` es el `IdentityId` (BLAKE3 de la clave pública)
    /// bajo el que el keystore la archiva.
    pub fn cargar_de_keystore(
        keystore: &agora_keystore::Keystore,
        id: IdentityId,
        passphrase: &str,
        nombre: impl Into<String>,
    ) -> Result<Self, ErrorCripto> {
        let seed = keystore
            .load(id, passphrase)
            .map_err(|_| ErrorCripto::KeystoreFallo)?;
        Ok(Self::desde_semilla(seed, nombre))
    }

    /// Persiste la semilla de esta identidad en el keystore cifrado. La semilla
    /// se entrega aparte porque el `Keypair` no la expone (la clave privada no
    /// se vuelve a leer una vez derivada).
    pub fn guardar_en_keystore(
        keystore: &agora_keystore::Keystore,
        seed: &[u8; 32],
        passphrase: &str,
    ) -> Result<IdentityId, ErrorCripto> {
        let kp = Keypair::from_seed(*seed);
        let id = kp.identity_id();
        keystore
            .save(id, seed, passphrase)
            .map_err(|_| ErrorCripto::KeystoreFallo)?;
        Ok(id)
    }

    /// La identidad agora del autor: su clave pública Ed25519 (32 bytes). Es lo
    /// que se graba en `Contenido::autor` y contra lo que se verifica la firma.
    pub fn agora_id(&self) -> AgoraId {
        self.keypair.public_key()
    }

    /// El `IdentityId` (BLAKE3 de la clave pública) — la llave bajo la que el
    /// keystore y el grafo de agora archivan esta identidad.
    pub fn identity_id(&self) -> IdentityId {
        self.keypair.identity_id()
    }

    /// El nombre de presentación (local, no autoritativo).
    pub fn nombre(&self) -> &str {
        &self.nombre
    }

    /// Firma el id de un nodo (32 bytes) con la clave privada. Es exactamente
    /// el closure `FnOnce(&Hash) -> Firma` que [`ayni_core::MensajeNodo::sellar`]
    /// y [`ayni_core::Conversacion::redactar`] esperan: pasar `|id| ident.firmar(id)`.
    pub fn firmar(&self, id: &Hash) -> Firma {
        self.keypair.sign(id)
    }

    /// La clave pública X25519 de esta identidad (derivada de la misma semilla
    /// agora). Es lo que se publica a un par para que pueda abrirle un canal
    /// cifrado — el análogo de cifrado del `agora_id` de firma.
    pub fn clave_publica_x25519(&self) -> [u8; 32] {
        crate::canal::publico_x25519(&self.seed)
    }

    /// Abre un [`CanalSeguro`] 1:1 con otro, dada su clave pública X25519. Ambos
    /// extremos derivan la MISMA clave de canal sin intercambiar secretos (X25519
    /// es simétrico). Lo que viaje por ese canal sólo lo lee el otro extremo.
    pub fn canal_con(&self, su_publico_x25519: &[u8; 32]) -> crate::CanalSeguro {
        crate::CanalSeguro::derivar(&crate::canal::secreto_x25519(&self.seed), su_publico_x25519)
    }
}

/// Verifica que `firma` sea una firma Ed25519 válida del `autor` (clave pública
/// cruda) sobre el id del nodo. Es el closure `(&AgoraId, &Hash, &Firma) -> bool`
/// que [`ayni_core::Conversacion::verificar_firmas`] y
/// [`ayni_core::MensajeNodo::verificar`] esperan. Encapsula el `Result` de agora
/// en un `bool` —el grafo sólo necesita saber válida/no-válida—.
pub fn verificar_firma(autor: &AgoraId, id: &Hash, firma: &Firma) -> bool {
    verify_signature(autor, id, firma).is_ok()
}
