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
//  `format` para el grafo en disco—. Cuatro mensajes bastan:
//
//    1. `SolicitarObjeto(id)`    — pide un nodo del grafo por su hash BLAKE3.
//    2. `ProveedorObjeto(id, d)` — responde con el payload binario del nodo.
//    3. `AnunciarRaiz(id)`       — difunde el hash de la raiz del sistema.
//    4. `AnunciarCanal { ... }`  — difunde el hash de un CANAL de release y la
//       raiz de manifiesto que su autor recomienda en este momento, firmada.
//       Es el equivalente nativo de `apt update`: quien escuche un anuncio de
//       un canal en el que confia puede pedir el canal, verificar las firmas,
//       descargar el DAG faltante (gratis por dedup BLAKE3) y re-anclar su
//       superbloque a la raiz nueva. Sin servidor central, sin TLS, sin DNS.
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
use serde_big_array::BigArray;

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
/// contenido. Coincide byte a byte con `format::Hash` —AoE habla el mismo
/// idioma de identidad que el grafo en disco—.
pub type ObjectId = [u8; 32];

/// Una direccion MAC, en seis bytes.
pub type Mac = [u8; 6];

/// La identidad de un autor agora en el cable: una clave publica Ed25519, 32
/// bytes. Coincide byte a byte con `format::AgoraId` —el cable habla el mismo
/// idioma de identidad que el disco—. `akasha` no enlaza criptografia: solo
/// transporta la clave. La verificacion vive en quien recibe.
pub type AutorId = [u8; 32];

/// Una firma Ed25519 en el cable, 64 bytes. Coincide byte a byte con
/// `format::Firma`. `akasha` la transporta; quien recibe la verifica.
pub type FirmaAkasha = [u8; 64];

// =============================================================================
//  El mensaje
// =============================================================================

/// Un mensaje AoE — la unidad de protocolo que viaja en un frame de
/// `ETHER_TYPE_AKASHA`. Tres variantes bastan para fundar un grafo distribuido:
///
/// - `SolicitarObjeto`  pregunta por un objeto identificado por su hash.
/// - `ProveedorObjeto`  responde con el payload binario del objeto. El
///   receptor recompone el `format::Objeto` aplicando su deserializador a
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
    /// El payload es la forma serializada `postcard` de un `format::Objeto`,
    /// y el receptor DEBE verificar que `blake3(payload) == id` antes de
    /// confiar en el contenido.
    ProveedorObjeto(ObjectId, Vec<u8>),
    /// Difundo el hash de mi raiz actual — quien me escuche y le falte este
    /// nodo en su grafo local puede pedirmelo.
    AnunciarRaiz(ObjectId),
    /// Difundo un CANAL de release: el `canal` es el hash de un `format::Canal`
    /// del grafo; `raiz` es la `Hash` del manifiesto que el `autor` recomienda
    /// en este `timestamp`, firmada con `firma` sobre el mensaje canonico que
    /// produce `format::mensaje_a_firmar(nombre, timestamp, raiz)`. El receptor
    /// que confia en `autor` puede entonces pedir el canal y la raiz, verificar
    /// las firmas internas, descargar el DAG delta y re-anclar el superbloque.
    /// Es la unidad de "actualizacion" en wawa — el equivalente nativo de
    /// `apt update && apt upgrade`, en un solo frame de capa-2 de ~210 bytes.
    AnunciarCanal {
        /// Hash del objeto `format::Canal` que historiza las raices firmadas.
        canal: ObjectId,
        /// Hash de la raiz de manifiesto que el autor recomienda ahora.
        raiz: ObjectId,
        /// Clave publica del autor (`format::AgoraId`).
        autor: AutorId,
        /// Instante de la recomendacion, segundos UNIX. Forma parte del mensaje
        /// firmado: un anuncio no se replica como si fuera de hoy.
        timestamp: u64,
        /// Firma Ed25519 del autor sobre `mensaje_a_firmar(_, timestamp, raiz)`.
        /// El receptor reconstruye el mensaje localmente y la verifica.
        /// `serde-big-array` cierra el hueco que serde deja en arrays > 32.
        #[serde(with = "BigArray")]
        firma: FirmaAkasha,
    },
    /// Fase 65 :: UN FRAGMENTO de un objeto cuyo payload no cabe en un solo
    /// frame de capa-2 (`> MAX_PAYLOAD_AKASHA`). El emisor parte el payload
    /// serializado del objeto en trozos de hasta [`MAX_FRAGMENTO_DATOS`] bytes
    /// y emite `total` fragmentos `[0..total)`. El receptor los reensambla con
    /// un [`Reensamblador`] y, al completar, verifica `blake3(payload) == id`
    /// igual que con `ProveedorObjeto` —la integridad se valida SOBRE EL OBJETO
    /// ENTERO reensamblado, no sobre los trozos—. Variante AÑADIDA AL FINAL del
    /// enum a proposito: los tags `postcard` se asignan por orden, y mover una
    /// variante existente romperia la compatibilidad con binarios viejos.
    ProveedorFragmento {
        /// Hash del objeto COMPLETO (no del fragmento): clave de reensamblado
        /// y testigo de integridad final.
        id: ObjectId,
        /// Indice de este fragmento en `[0, total)`.
        indice: u16,
        /// Cuantos fragmentos componen el objeto entero.
        total: u16,
        /// Los bytes de este fragmento (≤ [`MAX_FRAGMENTO_DATOS`]).
        datos: Vec<u8>,
    },
}

/// Bytes maximos de `datos` en un `ProveedorFragmento`. El frame `postcard`
/// resultante (tag 1 B + id 32 B + indice/total ≤ 6 B + prefijo de longitud
/// ≤ 3 B + datos) queda holgadamente bajo `MAX_PAYLOAD_AKASHA` (1486): con
/// 1024 el frame ronda los 1066 B. Margen deliberado para no rozar la MTU.
pub const MAX_FRAGMENTO_DATOS: usize = 1024;

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
//  Fragmentacion y reensamblado (Fase 65)
// =============================================================================

/// Cuantos fragmentos de [`MAX_FRAGMENTO_DATOS`] hacen falta para `len` bytes.
/// Siempre >= 1 (un payload vacio viaja como un unico fragmento vacio). El
/// emisor recorre `payload.chunks(MAX_FRAGMENTO_DATOS)` y compone un
/// `ProveedorFragmento { id, indice, total, datos }` por trozo.
pub fn total_fragmentos(len: usize) -> u16 {
    if len == 0 {
        return 1;
    }
    len.div_ceil(MAX_FRAGMENTO_DATOS) as u16
}

/// Tope DURO de fragmentos por objeto: acota la memoria del reensamblado
/// frente a un emisor adversario que anuncie un `total` absurdo. 2048 * 1024 B
/// = 2 MiB de techo transitorio; el `almacen` rechaza luego cualquier objeto
/// que exceda su propio `MAX_OBJETO` al verificar el hash.
pub const MAX_FRAGMENTOS: u16 = 2048;

/// Reensamblador de UNA SOLA RANURA: acumula los fragmentos de un objeto hasta
/// completarlo. Disenado para el patron real del cable —un emisor sirve los
/// fragmentos de un objeto de corrido en respuesta a un `SolicitarObjeto`—, de
/// modo que una sola ranura basta: si llega un fragmento de un `id` distinto
/// (o un `total` distinto para el mismo `id`), se descarta el progreso anterior
/// y se empieza de nuevo. Vive como `static` en el kernel tras un `Mutex`.
///
/// NO verifica el hash: eso lo hace el llamante sobre el payload completo
/// (`absorber_proveedor` ya rehashea). El reensamblador solo junta los trozos.
pub struct Reensamblador {
    id: ObjectId,
    total: u16,
    trozos: Vec<Option<Vec<u8>>>,
    faltantes: u16,
    activo: bool,
}

impl Default for Reensamblador {
    fn default() -> Self {
        Self::nuevo()
    }
}

impl Reensamblador {
    /// `const fn` para vivir como `static` sin lazy-init: arranca inactivo.
    pub const fn nuevo() -> Self {
        Self {
            id: [0; 32],
            total: 0,
            trozos: Vec::new(),
            faltantes: 0,
            activo: false,
        }
    }

    /// Ingiere un fragmento. Devuelve `Some(payload completo)` cuando el objeto
    /// `id` queda entero (y libera la ranura); `None` si aun faltan trozos o el
    /// fragmento es invalido (total fuera de rango, indice fuera de rango).
    pub fn ingerir(
        &mut self,
        id: ObjectId,
        indice: u16,
        total: u16,
        datos: &[u8],
    ) -> Option<Vec<u8>> {
        if total == 0 || total > MAX_FRAGMENTOS || indice >= total {
            return None;
        }
        // Arrancar (o reiniciar) la ranura si es un objeto/tamano distinto.
        if !self.activo || self.id != id || self.total != total {
            self.id = id;
            self.total = total;
            self.trozos = (0..total).map(|_| None).collect();
            self.faltantes = total;
            self.activo = true;
        }
        // Registrar el trozo si es nuevo (los duplicados se ignoran).
        let ranura = &mut self.trozos[indice as usize];
        if ranura.is_none() {
            *ranura = Some(datos.to_vec());
            self.faltantes -= 1;
        }
        if self.faltantes != 0 {
            return None;
        }
        // Completo: concatenar en orden y liberar la ranura.
        let mut completo = Vec::new();
        for trozo in &self.trozos {
            if let Some(bytes) = trozo {
                completo.extend_from_slice(bytes);
            }
        }
        self.reiniciar();
        Some(completo)
    }

    /// Descarta el progreso y libera la memoria de la ranura.
    pub fn reiniciar(&mut self) {
        self.id = [0; 32];
        self.total = 0;
        self.trozos = Vec::new();
        self.faltantes = 0;
        self.activo = false;
    }
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
    fn anuncio_de_canal_viaja_compacto_y_es_simetrico() {
        let msg = MensajeAkasha::AnunciarCanal {
            canal: [0x11; 32],
            raiz: [0x22; 32],
            autor: [0x33; 32],
            timestamp: 1_700_000_000,
            firma: [0x44; 64],
        };
        let frame = componer_frame(MAC_A, MAC_BROADCAST, &msg).unwrap();
        // 14 cabecera + 1 variante + 32 canal + 32 raiz + 32 autor +
        // varint(timestamp) <= 10 + 64 firma. Bordes flojos: cabe holgado en MTU.
        assert!(frame.len() < 200);
        let (src, rec) = analizar_frame(&frame).unwrap();
        assert_eq!(src, MAC_A);
        match rec {
            MensajeAkasha::AnunciarCanal { canal, raiz, autor, timestamp, firma } => {
                assert_eq!(canal, [0x11; 32]);
                assert_eq!(raiz, [0x22; 32]);
                assert_eq!(autor, [0x33; 32]);
                assert_eq!(timestamp, 1_700_000_000);
                assert_eq!(firma, [0x44; 64]);
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

    #[test]
    fn fragmento_grande_cabe_en_un_frame() {
        // El frame de un ProveedorFragmento con datos al maximo no debe
        // exceder MAX_PAYLOAD_AKASHA — la razon de ser del techo de 1024.
        let datos = vec![0xABu8; MAX_FRAGMENTO_DATOS];
        let msg = MensajeAkasha::ProveedorFragmento {
            id: HASH_X,
            indice: 3,
            total: 12,
            datos,
        };
        let frame = componer_frame(MAC_A, MAC_B, &msg).expect("debe componer");
        assert!(frame.len() <= CABECERA_ETHERNET + MAX_PAYLOAD_AKASHA);
    }

    #[test]
    fn total_fragmentos_redondea_hacia_arriba() {
        assert_eq!(total_fragmentos(0), 1);
        assert_eq!(total_fragmentos(1), 1);
        assert_eq!(total_fragmentos(MAX_FRAGMENTO_DATOS), 1);
        assert_eq!(total_fragmentos(MAX_FRAGMENTO_DATOS + 1), 2);
        assert_eq!(total_fragmentos(3 * MAX_FRAGMENTO_DATOS), 3);
    }

    /// Parte `payload` y lo alimenta al reensamblador en el orden dado por
    /// `orden` (indices). Devuelve lo reensamblado al completar.
    fn roundtrip(payload: &[u8], orden: &[u16]) -> Option<Vec<u8>> {
        let total = total_fragmentos(payload.len());
        let trozos: Vec<&[u8]> = payload.chunks(MAX_FRAGMENTO_DATOS).collect();
        let trozos = if trozos.is_empty() { vec![&[][..]] } else { trozos };
        let mut r = Reensamblador::nuevo();
        let mut salida = None;
        for &i in orden {
            salida = r.ingerir(HASH_X, i, total, trozos[i as usize]);
        }
        salida
    }

    #[test]
    fn reensamblar_en_orden_reconstruye_el_payload() {
        let payload: Vec<u8> = (0..2600u32).map(|i| i as u8).collect(); // 3 trozos
        let total = total_fragmentos(payload.len());
        assert_eq!(total, 3);
        let recon = roundtrip(&payload, &[0, 1, 2]).expect("completo");
        assert_eq!(recon, payload);
    }

    #[test]
    fn reensamblar_desordenado_y_con_duplicados() {
        let payload: Vec<u8> = (0..3000u32).map(|i| (i * 7) as u8).collect(); // 3 trozos
        // Orden invertido + un duplicado intercalado: el duplicado se ignora,
        // el resultado es identico.
        let recon = roundtrip(&payload, &[2, 0, 2, 1]).expect("completo");
        assert_eq!(recon, payload);
    }

    #[test]
    fn fragmento_invalido_no_completa() {
        let mut r = Reensamblador::nuevo();
        // total=0 e indice fuera de rango son invalidos.
        assert!(r.ingerir(HASH_X, 0, 0, b"x").is_none());
        assert!(r.ingerir(HASH_X, 5, 3, b"x").is_none());
        // total absurdo (sobre el tope) se rechaza.
        assert!(r.ingerir(HASH_X, 0, MAX_FRAGMENTOS + 1, b"x").is_none());
    }

    #[test]
    fn cambio_de_id_reinicia_el_reensamblado() {
        let mut r = Reensamblador::nuevo();
        // Medio objeto X (1 de 2 trozos)...
        assert!(r.ingerir(HASH_X, 0, 2, b"aaaa").is_none());
        // ...y de pronto llega un objeto Y de 1 trozo: completa Y, no mezcla.
        let otro: ObjectId = [0x22; 32];
        let completo = r.ingerir(otro, 0, 1, b"YY").expect("Y completo");
        assert_eq!(completo, b"YY");
    }
}
