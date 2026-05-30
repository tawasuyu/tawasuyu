// =============================================================================
//  renaser :: kernel/src/claves.rs — Fase 25/41/48 :: el sello criptografico del Ring 0
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
//
//  ---------------------------------------------------------------------------
//  CEREMONIA DE FIANZA DE CLAVES SOBERANAS (Fase 48, Boot Trust Ceremony)
//  ---------------------------------------------------------------------------
//  Las tres claves de [`AGORA_AUTH_RING`] DEJARON DE SER PLACEHOLDERS. Cada
//  una es la pubkey Ed25519 derivada de una seed forjada por el operador
//  local con `wawactl claves forjar`. La seed correspondiente vive offline
//  en el HSM/USB/papel del operador, jamas en este arbol.
//
//  Para colonizar un hardware virgen con Wawa, la ceremonia es esta:
//
//    1. `wawactl claves forjar --slot <N> --salida <PATH>` (N=0,1,2). La
//       seed se persiste con `0600`; el comando imprime la pubkey como
//       array literal de Rust pegable directo.
//    2. El operador inyecta los TRES literales en los slots
//       correspondientes de [`AGORA_AUTH_RING`] de este archivo.
//    3. `cargo +nightly build --target x86_64-unknown-none` re-forja el
//       binario inmutable del kernel con el anillo nuevo embebido en
//       `.rodata`.
//    4. El demonio `wawactl daemon-firma --slot <N> --clave-privada <PATH>`
//       opera la firma viva con la seed del slot que corresponda. Las
//       otras dos seeds quedan en almacenamiento frio como reserva.
//
//  Si en cualquier momento futuro el operador necesita auditar que la
//  pubkey grabada aqui sigue casando con la seed offline:
//    `wawactl claves derivar-pubkey --clave-privada <PATH>`
//  reimprime el literal — comparar byte a byte basta.
// =============================================================================

use ed25519_compact::{PublicKey, Signature};

use format::{CodigoError, ConcesionCapacidad, CuadernoFirmado, Hash, ManifiestoFirmado};

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
/// FASE 48 :: claves SOBERANAS. Los placeholders historicos quedaron
/// sepultados — cada slot contiene una pubkey Ed25519 forjada por la
/// ceremonia documentada arriba. Las seeds correspondientes viven
/// offline en el HSM/USB del operador local. Re-forjar la imagen es
/// el unico camino para rotar una clave; el binario es la fuente de
/// verdad.
pub const AGORA_AUTH_RING: [[u8; 32]; 3] = [
    // Slot 0 :: LLAVE PRIMARIA DEL OPERADOR. Es la que `apps/pluma`
    // empotra en `AGORA_PUBLIC_KEY_LOCAL` para componer el sobre por
    // defecto. Forjada en la ceremonia de la Fase 48; reemplaza el
    // placeholder historico de la Fase 25.
    [
        0x68, 0x47, 0x56, 0xec, 0x9a, 0xad, 0x2e, 0x83,
        0x02, 0x78, 0x11, 0x34, 0x71, 0x69, 0x83, 0xd5,
        0xf2, 0xff, 0xe7, 0x28, 0x3d, 0x8d, 0xcd, 0x67,
        0x17, 0xd8, 0xad, 0x57, 0xe0, 0x35, 0x6f, 0x48,
    ],
    // Slot 1 :: DISPOSITIVO SECUNDARIO. Para firmas desde el telefono,
    // USB, o un wawactl en otra terminal. Forjada en la Fase 48; la
    // seed paralela vive en almacenamiento frio del operador.
    [
        0x21, 0x4d, 0x1d, 0xab, 0xa3, 0x65, 0xcd, 0x85,
        0x9f, 0x4a, 0xf5, 0x1a, 0x03, 0x83, 0x62, 0x1c,
        0x86, 0x86, 0xfa, 0xf2, 0xa8, 0x73, 0x01, 0xa4,
        0xb6, 0xf2, 0xef, 0xa2, 0x74, 0x10, 0x0a, 0xf8,
    ],
    // Slot 2 :: LLAVE DE RECUPERACION (cold-storage). Para el evento
    // raro de perdida de los dispositivos vivos. Forjada en la Fase 48;
    // la seed esta destinada a almacenamiento offline (papel/metal/HSM
    // cerrado bajo llave fisica).
    [
        0x39, 0xc8, 0x8e, 0xaa, 0x02, 0x1c, 0x42, 0xea,
        0x42, 0x3e, 0x18, 0xf4, 0x3c, 0xcc, 0xbc, 0x5a,
        0x44, 0xb0, 0x51, 0x01, 0xcc, 0x02, 0xd2, 0x77,
        0x76, 0x41, 0x02, 0x8c, 0xa0, 0x20, 0x12, 0x11,
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

/// Verifica la AUTORIDAD de un `AnunciarCanal` recibido por Akasha (Fase 64).
/// Es el espejo canonico de `verificar_manifiesto_firmado`: mismo orden
/// estricto de fallos (anillo -> decodificacion -> firma), mismos codigos de
/// retorno. La diferencia es el MENSAJE que se firma: aqui no son los 32 bytes
/// del hash pelado sino el mensaje canonico `format::mensaje_a_firmar(nombre,
/// timestamp, raiz)` —el mismo que produjo `agora_channel::firmar_raiz`—, que
/// liga la firma al NOMBRE del canal y al INSTANTE: una firma valida en `dev`
/// no se replica en `estable`, ni un anuncio viejo se revive con timestamp
/// nuevo.
///
/// El `nombre` lo provee el llamante tras leerlo del objeto `Canal` del grafo;
/// es SEGURO confiar en el nombre asi obtenido porque la firma LO CUBRE: si el
/// llamante mintiera sobre el nombre, la verificacion fallaria. A diferencia
/// del camino del hash pelado, este SI aloca un `Vec` temporal en
/// `mensaje_a_firmar` —ocurre una sola vez, al aceptar; no es camino caliente—.
pub fn verificar_anuncio_canal(
    autor: &[u8; 32],
    nombre: &str,
    timestamp: u64,
    raiz: &Hash,
    firma: &[u8; 64],
) -> Result<(), CodigoError> {
    if !autor_en_anillo(autor) {
        return Err(CodigoError::CapacidadInsuficiente);
    }
    let pk = PublicKey::from_slice(autor).map_err(|_| CodigoError::Ausente)?;
    let sig = Signature::from_slice(firma).map_err(|_| CodigoError::Ausente)?;
    let mensaje = format::mensaje_a_firmar(nombre, timestamp, raiz);
    pk.verify(&mensaje, &sig)
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

/// Verifica una [`ConcesionCapacidad`] (Fase 67 / WAWA §14.1.3): el binding
/// firmado "este bytecode puede usar estos permisos". Mismo orden estricto de
/// fallos que sus gemelos (anillo -> decodificacion -> firma) y mismos codigos.
/// El MENSAJE firmado es `format::mensaje_capacidad(bytecode, permisos)` —un
/// arreglo de pila de 36 bytes—, de modo que la firma liga la concesion al hash
/// EXACTO del binario y al bitfield EXACTO: ni se transplanta a otro bytecode ni
/// admite que se le suba un bit de permiso sin re-firmar.
///
/// ZERO-ALLOC: `mensaje_capacidad` devuelve un `[u8; 36]` de pila y la
/// verificacion `ed25519-compact` no toca al asignador. El llamante (el punto
/// de carga de una app) decide que hacer con el `Ok(())`: tomar la INTERSECCION
/// de estos `permisos` con los que el manifiesto declara (`permisos_efectivos`).
///
/// ENFORCEMENT VIVO (Fase 67, 2026-05-30): el punto de carga ya lo invoca —
/// `main::permisos_efectivos_de` lo llama en `encender_app` e
/// `instanciar_plantilla` para intersectar `entrada.permisos` con lo concedido
/// (`format::permisos_efectivos`). El verificador dejo de ser solo soberano y
/// testeable: gobierna de verdad que capacidades enlaza el `Linker` de wasmi.
pub fn verificar_concesion_capacidad(c: &ConcesionCapacidad) -> Result<(), CodigoError> {
    if !autor_en_anillo(&c.autor) {
        return Err(CodigoError::CapacidadInsuficiente);
    }
    let pk = PublicKey::from_slice(&c.autor).map_err(|_| CodigoError::Ausente)?;
    let sig = Signature::from_slice(&c.firma).map_err(|_| CodigoError::Ausente)?;
    let mensaje = format::mensaje_capacidad(&c.bytecode, c.permisos);
    pk.verify(mensaje, &sig)
        .map_err(|_| CodigoError::AlmacenamientoFallo)
}
