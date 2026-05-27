// =============================================================================
//  renaser :: kernel/src/claves.rs — Fase 25 :: el sello criptografico del Ring 0
// -----------------------------------------------------------------------------
//  POSIX gobierna la identidad por enteros mutables (UID/GID, sudoers, capabilities
//  de Linux). Cada uno de ellos es UN check `if (uid == 0)` que un bug del kernel
//  puede saltarse. Wawa, en cambio, gobierna la mutacion sensible por MATEMATICA:
//  el manifiesto solo se reanca cuando un sobre `ManifiestoFirmado` lleva una
//  firma Ed25519 valida contra la CLAVE PUBLICA que ESTE binario del kernel
//  lleva grabada en piedra.
//
//  Solo verifica. La clave PRIVADA no vive aqui: vive con el operador local
//  (wawactl/USB/HSM) o con el autor de un canal de release que el usuario haya
//  decidido confiar. Por consiguiente, si el binario del kernel se filtra:
//
//    * un atacante NO puede engendrar propuestas que el sistema acepte —no
//      tiene la clave privada—;
//    * un atacante NO puede saltarse la verificacion sin modificar el binario
//      —el chequeo es codigo, no un flag de tiempo de ejecucion—.
//
//  ZERO-ALLOC: la verificacion ocurre integramente sobre la pila + el bloque
//  estatico de la `pubkey`. `ed25519-compact` con `default-features = false`
//  no toca al asignador para verificar; no le hace falta `random`.
// =============================================================================

use ed25519_compact::{PublicKey, Signature};

use format::{CodigoError, ManifiestoFirmado};

/// CLAVE PUBLICA del autor local —el operador con poder de reancla del
/// manifiesto—. Sin un esquema TPM/USB todavia, esta empotrada en el binario
/// del kernel: lo cual significa que cambiarla exige re-forjar la imagen,
/// como debe ser para una operacion de seguridad de este peso.
///
/// VALOR PROVISIONAL: 32 bytes determinados —no es una clave "viva" todavia—.
/// Cuando wawactl gane su comando de forja de claves, este valor se generara
/// junto con la clave privada que se exporta para el operador y se incorporara
/// a la imagen via `include_bytes!` en lugar de hardcoded. Hasta entonces, el
/// sistema se autoancla a esta clave fija — propuestas firmadas con cualquier
/// otra clave caen con `CapacidadInsuficiente` antes de gastar un ciclo en
/// criptografia.
pub const AGORA_PUBLIC_KEY_LOCAL: [u8; 32] = [
    0x1a, 0x4f, 0x7c, 0x91, 0xb6, 0x2d, 0x5e, 0xa8,
    0x33, 0xc7, 0x09, 0x84, 0xf1, 0x60, 0xb5, 0x52,
    0x6e, 0xae, 0x17, 0x40, 0x82, 0xfb, 0x99, 0xc1,
    0x2d, 0x55, 0xd6, 0x3a, 0xe4, 0x77, 0x1c, 0x80,
];

/// Verifica un sobre criptografico `ManifiestoFirmado`. Falla por la primera
/// razon que se presente, en este orden estricto:
///
///   1. `autor` distinta de la clave publica local — propuesta de un peer
///      cuya identidad el operador local no autorizo. Retorna
///      `CapacidadInsuficiente` (no es un error de almacenamiento, es un
///      error de autoridad).
///   2. La llave publica del sobre no se puede decodificar — corrupcion en
///      el wire o en el grafo. Retorna `Ausente`.
///   3. La firma del sobre no se puede decodificar — idem.
///   4. La firma NO verifica matematicamente sobre los 32 bytes de
///      `manifiesto_hash` — propuesta forjada o tampered. Retorna
///      `AlmacenamientoFallo`.
///
/// La verificacion es FRESH cada vez: no cacheamos resultados ni dependemos
/// de TOCTOU. El llamante DECIDE que hacer con el `Ok(())` —reanca solo si
/// el manifiesto referenciado existe en el grafo local—.
pub fn verificar_manifiesto_firmado(mf: &ManifiestoFirmado) -> Result<(), CodigoError> {
    // Defensa-en-profundidad N.1: descartar autores ajenos antes de tocar
    // la criptografia. Un peer hostil que envia un sobre con su propia
    // clave publica no merece consumir un solo ciclo de scalar mult.
    if mf.autor != AGORA_PUBLIC_KEY_LOCAL {
        return Err(CodigoError::CapacidadInsuficiente);
    }
    let pk = PublicKey::from_slice(&mf.autor).map_err(|_| CodigoError::Ausente)?;
    let sig = Signature::from_slice(&mf.firma).map_err(|_| CodigoError::Ausente)?;

    // El mensaje firmado son los 32 bytes del hash del manifiesto. Ed25519
    // no se preocupa por longitud; firmar el hash equivale a firmar el
    // payload entero —el hash es ya el resumen criptografico—.
    pk.verify(mf.manifiesto_hash, &sig)
        .map_err(|_| CodigoError::AlmacenamientoFallo)
}
