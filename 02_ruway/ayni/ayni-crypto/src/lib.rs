// =============================================================================
//  ayni :: ayni-crypto — identidad y criptografía de la conversación soberana
// -----------------------------------------------------------------------------
//  `ayni-core` es cripto-agnóstico (firma/verifica por closure) para poder
//  viajar a wawa sin enlazar primitivas. Este crate provee la cripto real,
//  host-side, atada a `agora` (la identidad federada del proyecto):
//
//    * `firma` (P1) — la `Identidad` del usuario: cargar/crear par Ed25519,
//      firmar el id de un nodo, verificar la firma de un autor.
//    * `canal` (P2) — E2EE 1:1 sobre los payloads. Pendiente.
//
//  Regla dura del repo: NO se hace criptografía a mano. Se compone lo que
//  `agora` y los crates auditados ya proveen.
// =============================================================================

mod canal;
mod firma;

pub use canal::CanalSeguro;
pub use firma::{verificar_firma, Identidad};

// Re-export de los tipos del grafo que el consumidor maneja junto con la firma.
pub use ayni_core::{AgoraId, Firma, Hash};
// Re-export del id de identidad de agora — la llave del keystore/grafo.
pub use agora_core::IdentityId;

/// Falla de una operación criptográfica de Ayni.
#[derive(Debug, thiserror::Error)]
pub enum ErrorCripto {
    /// El keystore de agora no pudo descifrar/guardar la semilla: passphrase
    /// incorrecta, archivo ausente o manipulado. Argon2id+ChaCha20-Poly1305 no
    /// distingue "clave mala" de "blob corrupto" —es deliberado—.
    #[error("ayni-crypto :: el keystore de agora falló (passphrase o archivo)")]
    KeystoreFallo,
    /// El descifrado E2EE falló: blob truncado, tag Poly1305 inválido
    /// (manipulación) o clave de canal equivocada. Como el keystore, no se
    /// distinguen las causas —no se filtra información a un atacante—.
    #[error("ayni-crypto :: descifrado E2EE fallido (blob, tag o clave)")]
    CifradoFallo,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{Carga, Conversacion, MensajeNodo};

    #[test]
    fn la_identidad_firma_y_se_verifica_a_si_misma() {
        let ident = Identidad::desde_semilla([7u8; 32], "Yumaira");
        let conv = Conversacion::nueva();
        let nodo = conv.redactar(
            ident.agora_id(),
            Carga::Texto("hola mundo".into()),
            1,
            |id| ident.firmar(id),
        );
        assert!(nodo.verificar(verificar_firma), "la firma propia valida");
    }

    #[test]
    fn firma_de_un_impostor_no_valida() {
        let real = Identidad::desde_semilla([1u8; 32], "Real");
        let impostor = Identidad::desde_semilla([2u8; 32], "Impostor");
        // el nodo se atribuye a `real` pero lo firma el impostor:
        let contenido = ayni_core::Contenido::nuevo(
            real.agora_id(),
            alloc_vec(),
            Carga::Texto("usurpado".into()),
            1,
        );
        let nodo = MensajeNodo::sellar(contenido, |id| impostor.firmar(id));
        assert!(!nodo.verificar(verificar_firma));
    }

    #[test]
    fn agora_id_es_la_clave_publica_cruda() {
        // El autor on-wire debe ser la pubkey cruda (32 bytes), no el IdentityId
        // (que es BLAKE3 de la pubkey) — si no, verify_signature no podría
        // reconstruir la VerifyingKey.
        let ident = Identidad::desde_semilla([9u8; 32], "x");
        assert_eq!(ident.agora_id(), agora_core::Keypair::from_seed([9u8; 32]).public_key());
        assert_ne!(ident.agora_id(), *ident.identity_id().as_bytes());
    }

    #[test]
    fn guardar_y_cargar_del_keystore() {
        let dir = tempfile::tempdir().unwrap();
        let ks = agora_keystore::Keystore::open(dir.path()).unwrap();
        let (ident, seed) = Identidad::nueva_aleatoria("Tú");
        let id = Identidad::guardar_en_keystore(&ks, &seed, "clave-secreta").unwrap();
        assert_eq!(id, ident.identity_id());

        let recuperada =
            Identidad::cargar_de_keystore(&ks, id, "clave-secreta", "Tú").unwrap();
        assert_eq!(recuperada.agora_id(), ident.agora_id(), "misma identidad tras roundtrip");

        // passphrase incorrecta falla:
        assert!(Identidad::cargar_de_keystore(&ks, id, "mala", "Tú").is_err());
    }

    fn alloc_vec() -> Vec<Hash> {
        Vec::new()
    }

    // --- P2: canal E2EE 1:1 ---------------------------------------------------

    #[test]
    fn ambos_extremos_derivan_el_mismo_canal() {
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let beto = Identidad::desde_semilla([2u8; 32], "Beto");

        let canal_a = alicia.canal_con(&beto.clave_publica_x25519());
        let canal_b = beto.canal_con(&alicia.clave_publica_x25519());

        // Alicia cifra; Beto —que derivó el canal por su cuenta— descifra.
        let blob = canal_a.cifrar(b"secreto entre los dos");
        let claro = canal_b.descifrar(&blob).unwrap();
        assert_eq!(claro, b"secreto entre los dos");

        // y al revés.
        let blob2 = canal_b.cifrar(b"de vuelta");
        assert_eq!(canal_a.descifrar(&blob2).unwrap(), b"de vuelta");
    }

    #[test]
    fn un_tercero_no_descifra() {
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let beto = Identidad::desde_semilla([2u8; 32], "Beto");
        let mirona = Identidad::desde_semilla([3u8; 32], "Mirona");

        let canal_a = alicia.canal_con(&beto.clave_publica_x25519());
        let blob = canal_a.cifrar("sólo para Beto".as_bytes());

        // La mirona abre un canal hacia Alicia, pero no es el canal Alicia↔Beto:
        let canal_m = mirona.canal_con(&alicia.clave_publica_x25519());
        assert!(canal_m.descifrar(&blob).is_err(), "un tercero no lee el canal");
    }

    #[test]
    fn ciphertext_manipulado_falla() {
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let beto = Identidad::desde_semilla([2u8; 32], "Beto");
        let canal = alicia.canal_con(&beto.clave_publica_x25519());
        let mut blob = canal.cifrar(b"intacto");
        let n = blob.len();
        blob[n - 1] ^= 0xff; // voltea un bit del tag
        assert!(beto.canal_con(&alicia.clave_publica_x25519()).descifrar(&blob).is_err());
    }

    #[test]
    fn nodo_e2ee_firmado_y_cifrado_a_la_vez() {
        // El escenario completo: un nodo del DAG cuyo payload está cifrado pero
        // cuya AUTORÍA es pública y verificable. Sólo el destinatario lee el claro.
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let beto = Identidad::desde_semilla([2u8; 32], "Beto");

        let canal_a = alicia.canal_con(&beto.clave_publica_x25519());
        let cifrado = canal_a.cifrar("hola en secreto".as_bytes());

        let conv = Conversacion::nueva();
        let nodo = conv.redactar(alicia.agora_id(), Carga::Cifrado(cifrado), 1, |id| {
            alicia.firmar(id)
        });

        // cualquiera verifica que lo firmó Alicia (autoría pública)…
        assert!(nodo.verificar(verificar_firma));
        // …pero el texto no está en claro en el nodo…
        assert_eq!(nodo.contenido.carga.texto(), None);
        // …y sólo Beto, con su canal, recupera el claro.
        let canal_b = beto.canal_con(&alicia.clave_publica_x25519());
        let claro = canal_b
            .descifrar(nodo.contenido.carga.cifrado().unwrap())
            .unwrap();
        assert_eq!(claro, b"hola en secreto");
    }
}
