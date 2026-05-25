// =============================================================================
//  renaser :: akasha — Akasha Over Ether (AoE)
// -----------------------------------------------------------------------------
//  Fase 19 demostro que un app del userspace puede inyectar frames Ethernet
//  crudos al cable —`pregon` gritando «hola» con su EtherType experimental—,
//  pero ese saludo era texto plano, sin estructura. La Fase 20 da el siguiente
//  paso natural: **fundar un protocolo nativo del ecosistema** para que
//  renaser hable consigo mismo a traves de la red, sin TCP, sin IP, sin las
//  sobrecargas de cabecera de los años 80.
//
//  AoE viaja DIRECTAMENTE sobre Ethernet (`EtherType = 0x88B5`, rango
//  experimental reservado por IEEE para uso local). Cada frame transporta un
//  `MensajeAkasha` serializado con `postcard` —el mismo codec que ya usa
//  `formato` para el grafo en disco—. Tres mensajes bastan:
//
//    1. `SolicitarObjeto(id)`   — pide un nodo del grafo por su hash BLAKE3.
//    2. `ProveedorObjeto(id, d)` — responde con el payload binario del nodo.
//    3. `AnunciarRaiz(id)`      — difunde el hash de la raiz del sistema.
//
//  Con esto basta para extender el grafo de objetos —direccionado por
//  contenido, inmutable, ya BLAKE3— a OTRAS maquinas renaser que escuchen en
//  la misma red de capa-2. El receptor de un `AnunciarRaiz` puede comparar
//  hashes, descubrir que le falta un nodo, pedirlo con `SolicitarObjeto` y
//  ensamblar el grafo del par. El grafo deja de ser una propiedad LOCAL del
//  disco y se vuelve una propiedad DISTRIBUIDA del cable.
//
//  Esta crate es la unica verdad del protocolo. Es un nucleo `#![no_std]` —el
//  kernel bare-metal la enlaza—; cualquier app, daemon u orquestador que
//  hable AoE consume estos mismos tipos. Ningun otro modulo redefine ni un
//  byte del trazado del frame.
// =============================================================================

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

// =============================================================================
//  Constantes del protocolo en el cable
// =============================================================================

/// EtherType de AoE, rango experimental reservado por IEEE para uso local
/// (0x88B5/0x88B6). renaser elige el primero. Cualquier frame del cable que
/// no porte este EtherType NO es Akasha, y se entrega al userspace.
pub const ETHER_TYPE_AKASHA: u16 = 0x88B5;

/// Tamaño de la cabecera Ethernet, en bytes (dst + src + ethertype).
pub const CABECERA_ETHERNET: usize = 14;

/// Maximo del payload AoE serializado, en bytes. Acotado para que el frame
/// completo (cabecera + payload) no exceda una MTU Ethernet sin VLAN
/// (1500 - 14 = 1486) y se transmita SIN fragmentar. Si un mensaje no cabe,
/// el llamante debe partirlo en varios objetos del grafo y referirse a ellos.
pub const MAX_PAYLOAD_AKASHA: usize = 1486;

/// MAC de broadcast — la difunde todo Ethernet, equivalente a `255.255.255.255`
/// en IPv4 pero en capa-2 pura.
pub const MAC_BROADCAST: [u8; 6] = [0xff; 6];

/// El identificador de un objeto del grafo: el hash BLAKE3 de su forma
/// serializada. En un almacen direccionado por contenido, la identidad ES el
/// contenido. Coincide byte a byte con `formato::Hash` —AoE habla el mismo
/// idioma de identidad que el grafo en disco—.
pub type ObjectId = [u8; 32];

/// Una direccion MAC, en seis bytes.
pub type Mac = [u8; 6];

// =============================================================================
//  El mensaje
// =============================================================================

/// Un mensaje AoE — la unidad de protocolo que viaja en un frame de
/// `ETHER_TYPE_AKASHA`. Tres variantes bastan para fundar un grafo distribuido:
///
/// - `SolicitarObjeto`  pregunta por un objeto identificado por su hash.
/// - `ProveedorObjeto`  responde con el payload binario del objeto. El
///   receptor recompone el `formato::Objeto` aplicando su deserializador a
///   `payload` y verificando que `blake3(payload) == id`.
/// - `AnunciarRaiz`     difunde el hash de la raiz —el ancla del sistema
///   actual—. Sirve como faro: quien escuche y carezca de ese nodo en su
///   grafo local puede iniciar una solicitud.
///
/// La codificacion `postcard` es estable y compacta: un `SolicitarObjeto`
/// ocupa 33 bytes (1 de variante + 32 de hash) en el cable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MensajeAkasha {
    /// Solicito el objeto identificado por este hash.
    SolicitarObjeto(ObjectId),
    /// Aqui esta el payload binario del objeto identificado por este hash.
    /// El payload es la forma serializada `postcard` de un `formato::Objeto`,
    /// y el receptor DEBE verificar que `blake3(payload) == id` antes de
    /// confiar en el contenido.
    ProveedorObjeto(ObjectId, Vec<u8>),
    /// Difundo el hash de mi raiz actual — quien me escuche y le falte este
    /// nodo en su grafo local puede pedirmelo.
    AnunciarRaiz(ObjectId),
}

/// El motivo por el que un frame AoE no se pudo componer o analizar.
#[derive(Clone, Copy, Debug)]
pub enum ErrorAkasha {
    /// El payload serializado excede `MAX_PAYLOAD_AKASHA`.
    PayloadDemasiadoLargo,
    /// El frame en el cable es mas corto que la cabecera Ethernet.
    FrameDemasiadoCorto,
    /// El EtherType del frame no es `ETHER_TYPE_AKASHA`.
    EtherTypeAjeno,
    /// `postcard` no supo deserializar el payload (basura, version distinta,
    /// otro protocolo experimental ajeno).
    PayloadInvalido,
}

// =============================================================================
//  Composicion y analisis de frames
// =============================================================================

/// Compone un frame Ethernet completo —cabecera (14 bytes) + payload AoE
/// (`postcard(mensaje)`)— listo para entregar al driver. El llamante elige el
/// destino (`MAC_BROADCAST` para difundir) y firma el frame con su propia
/// MAC. Devuelve el frame completo, o un error si el mensaje serializado
/// excede `MAX_PAYLOAD_AKASHA`.
pub fn componer_frame(
    src: Mac,
    dst: Mac,
    mensaje: &MensajeAkasha,
) -> Result<Vec<u8>, ErrorAkasha> {
    let payload =
        postcard::to_allocvec(mensaje).map_err(|_| ErrorAkasha::PayloadDemasiadoLargo)?;
    if payload.len() > MAX_PAYLOAD_AKASHA {
        return Err(ErrorAkasha::PayloadDemasiadoLargo);
    }
    let mut frame = Vec::with_capacity(CABECERA_ETHERNET + payload.len());
    frame.extend_from_slice(&dst);
    frame.extend_from_slice(&src);
    frame.extend_from_slice(&ETHER_TYPE_AKASHA.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Analiza un frame Ethernet entrante. Si su EtherType es `ETHER_TYPE_AKASHA`
/// y el payload deserializa como un `MensajeAkasha`, devuelve la MAC de origen
/// y el mensaje. En cualquier otro caso —EtherType ajeno, frame truncado,
/// `postcard` no le encuentra sentido al payload— devuelve un `ErrorAkasha`
/// que distingue el motivo: el llamante puede entonces decidir si lo entrega
/// al userspace (otro protocolo) o lo descarta (basura).
pub fn analizar_frame(frame: &[u8]) -> Result<(Mac, MensajeAkasha), ErrorAkasha> {
    if frame.len() < CABECERA_ETHERNET {
        return Err(ErrorAkasha::FrameDemasiadoCorto);
    }
    let etype = u16::from_be_bytes([frame[12], frame[13]]);
    if etype != ETHER_TYPE_AKASHA {
        return Err(ErrorAkasha::EtherTypeAjeno);
    }
    let mut src: Mac = [0; 6];
    src.copy_from_slice(&frame[6..12]);
    let payload = &frame[CABECERA_ETHERNET..];
    let mensaje: MensajeAkasha =
        postcard::from_bytes(payload).map_err(|_| ErrorAkasha::PayloadInvalido)?;
    Ok((src, mensaje))
}

// =============================================================================
//  Pruebas
// =============================================================================

#[cfg(test)]
mod pruebas {
    use super::*;
    use alloc::vec;

    const MAC_A: Mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    const MAC_B: Mac = [0x52, 0x54, 0x00, 0xAB, 0xCD, 0xEF];
    const HASH_X: ObjectId = [0x11; 32];

    #[test]
    fn componer_y_analizar_solicitud_es_simetrico() {
        let msg = MensajeAkasha::SolicitarObjeto(HASH_X);
        let frame = componer_frame(MAC_A, MAC_B, &msg).unwrap();
        // Cabecera: dst + src + ethertype.
        assert_eq!(&frame[0..6], &MAC_B);
        assert_eq!(&frame[6..12], &MAC_A);
        assert_eq!(&frame[12..14], &ETHER_TYPE_AKASHA.to_be_bytes());
        let (src, rec) = analizar_frame(&frame).unwrap();
        assert_eq!(src, MAC_A);
        match rec {
            MensajeAkasha::SolicitarObjeto(id) => assert_eq!(id, HASH_X),
            otro => panic!("variante inesperada: {otro:?}"),
        }
    }

    #[test]
    fn anuncio_de_raiz_viaja_compacto() {
        let frame =
            componer_frame(MAC_A, MAC_BROADCAST, &MensajeAkasha::AnunciarRaiz(HASH_X))
                .unwrap();
        // 14 cabecera + 1 byte variante + 32 hash = 47 bytes en el cable.
        assert_eq!(frame.len(), 47);
        assert_eq!(&frame[0..6], &MAC_BROADCAST);
    }

    #[test]
    fn proveedor_lleva_payload_arbitrario() {
        let payload = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let msg = MensajeAkasha::ProveedorObjeto(HASH_X, payload.clone());
        let frame = componer_frame(MAC_A, MAC_B, &msg).unwrap();
        let (_, rec) = analizar_frame(&frame).unwrap();
        match rec {
            MensajeAkasha::ProveedorObjeto(id, p) => {
                assert_eq!(id, HASH_X);
                assert_eq!(p, payload);
            }
            otro => panic!("variante inesperada: {otro:?}"),
        }
    }

    #[test]
    fn frame_demasiado_corto_se_distingue() {
        assert!(matches!(
            analizar_frame(&[0u8; 8]),
            Err(ErrorAkasha::FrameDemasiadoCorto)
        ));
    }

    #[test]
    fn ethertype_ajeno_se_distingue() {
        // Cabecera valida pero EtherType de IPv4 (0x0800).
        let mut frame = [0u8; CABECERA_ETHERNET];
        frame[12] = 0x08;
        frame[13] = 0x00;
        assert!(matches!(
            analizar_frame(&frame),
            Err(ErrorAkasha::EtherTypeAjeno)
        ));
    }

    #[test]
    fn payload_invalido_se_distingue() {
        // EtherType correcto, pero el payload es basura para postcard.
        let mut frame = [0u8; CABECERA_ETHERNET + 4];
        frame[12..14].copy_from_slice(&ETHER_TYPE_AKASHA.to_be_bytes());
        frame[14..].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(matches!(
            analizar_frame(&frame),
            Err(ErrorAkasha::PayloadInvalido)
        ));
    }
}
