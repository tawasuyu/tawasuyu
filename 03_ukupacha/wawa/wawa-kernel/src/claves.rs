// =============================================================================
//  renaser :: kernel/src/claves.rs — Fase 25/41 :: el sello criptografico del Ring 0
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

use format::{CodigoError, CuadernoFirmado, ManifiestoFirmado};

/// FASE 41 :: ANILLO MULTI-AUTOR de identidades federadas del operador
/// local. Una propuesta firmada por CUALQUIERA de las tres claves de
/// confianza se acepta como soberana — el operador puede tener un
/// dispositivo primario, uno secundario (telefono, USB de campo) y una
/// llave fria de recuperacion guardada en otro fisico.
///
/// La iteracion del anillo es BARATA: tres comparaciones de 32 bytes,
/// cortocircuito al primer match. Sin alocacion, sin TOCTOU, sin
/// indirecciones — el anillo vive en `.rodata` del binario del kernel
/// y solo cambia re-forjando la imagen.
///
/// VALOR PROVISIONAL: cada slot lleva 32 bytes determinados que no son
/// claves "vivas" todavia. Cuando `wawactl` gane su comando de forja
/// de claves multi-dispositivo, los tres slots se rellenaran con
/// pubkeys reales via `include_bytes!` durante el build del kernel.
pub const AGORA_AUTH_RING: [[u8; 32]; 3] = [
    // Slot 0 :: LLAVE PRIMARIA DEL OPERADOR. Es la que `apps/pluma`
    // empotra en `AGORA_PUBLIC_KEY_LOCAL` para componer el sobre por
    // defecto. Continuidad binaria con la constante de la Fase 25 — un
    // cuaderno firmado antes de la Fase 41 sigue validando.
    [
        0x1a, 0x4f, 0x7c, 0x91, 0xb6, 0x2d, 0x5e, 0xa8,
        0x33, 0xc7, 0x09, 0x84, 0xf1, 0x60, 0xb5, 0x52,
        0x6e, 0xae, 0x17, 0x40, 0x82, 0xfb, 0x99, 0xc1,
        0x2d, 0x55, 0xd6, 0x3a, 0xe4, 0x77, 0x1c, 0x80,
    ],
    // Slot 1 :: DISPOSITIVO SECUNDARIO. Para firmas desde el telefono,
    // USB, o un wawactl en otra terminal. Placeholder hasta que la
    // forja multi-dispositivo de `wawactl claves` exista.
    [
        0x2b, 0x60, 0x8d, 0xa2, 0xc7, 0x3e, 0x6f, 0xb9,
        0x44, 0xd8, 0x1a, 0x95, 0x02, 0x71, 0xc6, 0x63,
        0x7f, 0xbf, 0x28, 0x51, 0x93, 0x0c, 0xaa, 0xd2,
        0x3e, 0x66, 0xe7, 0x4b, 0xf5, 0x88, 0x2d, 0x91,
    ],
    // Slot 2 :: LLAVE DE RECUPERACION (cold-storage). Para el evento
    // raro de perdida de los dispositivos vivos. Tipicamente offline,
    // grabada en papel/metal/HSM cerrado bajo llave fisica.
    [
        0x3c, 0x71, 0x9e, 0xb3, 0xd8, 0x4f, 0x70, 0xca,
        0x55, 0xe9, 0x2b, 0xa6, 0x13, 0x82, 0xd7, 0x74,
        0x80, 0xc0, 0x39, 0x62, 0xa4, 0x1d, 0xbb, 0xe3,
        0x4f, 0x77, 0xf8, 0x5c, 0x06, 0x99, 0x3e, 0xa2,
    ],
];

/// CLAVE PUBLICA del autor PRIMARIO. Mantenida como alias del slot 0
/// del [`AGORA_AUTH_RING`] para preservar la API publica de la Fase 25:
/// las apps (`apps/pluma`, `apps/mudanza`) siguen empotrando ESTA
/// constante para componer la `autor` del sobre por defecto, sin
/// necesidad de elegir slot.
///
/// Internamente al kernel toda la verificacion pasa por el anillo
/// completo; `AGORA_PUBLIC_KEY_LOCAL` no aparece en `claves.rs` mas
/// alla de esta declaracion. El `allow(dead_code)` documenta que la
/// constante es API publica para otras crates, no del propio kernel.
#[allow(dead_code)]
pub const AGORA_PUBLIC_KEY_LOCAL: [u8; 32] = AGORA_AUTH_RING[0];

/// Comprueba si una clave publica habita el [`AGORA_AUTH_RING`]. Tres
/// comparaciones de 32 bytes en linea, con cortocircuito al primer
/// match — zero-alloc, zero-indireccion. El verificador la llama antes
/// de gastar un ciclo en `ed25519_compact::PublicKey::from_slice`.
#[inline]
fn autor_en_anillo(autor: &[u8; 32]) -> bool {
    let mut i = 0;
    while i < AGORA_AUTH_RING.len() {
        if *autor == AGORA_AUTH_RING[i] {
            return true;
        }
        i += 1;
    }
    false
}

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
    // Defensa-en-profundidad N.1 :: el autor debe habitar el anillo
    // multi-autor (Fase 41). Cualquiera de las tres claves de confianza
    // es legitima — primaria, secundaria o de recuperacion—. Un peer
    // hostil con su propia clave NO esta en el anillo y cae antes de
    // que tocar la criptografia.
    if !autor_en_anillo(&mf.autor) {
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

/// Verifica un sobre criptografico `CuadernoFirmado` (Fase 37). El espejo
/// matematico de `verificar_manifiesto_firmado`, mismo orden estricto de
/// fallos, mismos codigos de retorno — un cuaderno cuyo autor no es el
/// operador local cae con `CapacidadInsuficiente` antes de tocar la
/// criptografia, y una firma forjada cae con `AlmacenamientoFallo`.
///
/// La verificacion es ZERO-ALLOC: `PublicKey`/`Signature` viven en la pila,
/// `pk.verify` opera sobre los 32 bytes del hash sin tocar al asignador.
/// El llamante decide que hacer con el `Ok(())` —tipicamente, fijar el
/// `cuaderno_raiz_hash` como raiz del grafo en un solo append atomico—.
pub fn verificar_cuaderno_firmado(cf: &CuadernoFirmado) -> Result<(), CodigoError> {
    // Defensa-en-profundidad N.1 :: el autor debe habitar el anillo
    // multi-autor (Fase 41). Tres slots de confianza posibles; cualquiera
    // basta para autorizar el sello. Peers fuera del anillo caen aqui
    // sin gastar ciclos en scalar mult.
    if !autor_en_anillo(&cf.autor) {
        return Err(CodigoError::CapacidadInsuficiente);
    }
    let pk = PublicKey::from_slice(&cf.autor).map_err(|_| CodigoError::Ausente)?;
    let sig = Signature::from_slice(&cf.firma).map_err(|_| CodigoError::Ausente)?;
    pk.verify(cf.cuaderno_raiz_hash, &sig)
        .map_err(|_| CodigoError::AlmacenamientoFallo)
}
