use super::*;

// =============================================================================
//  El hash y el trazado de un registro en el log
// =============================================================================

/// La identidad de un objeto: el hash BLAKE3 de su forma serializada. Kernel y
/// `boot` la calculan por aqui — una sola definicion del hash, jamas dos.
pub fn hash(bytes: &[u8]) -> Hash {
    *blake3::hash(bytes).as_bytes()
}

/// Numero de sectores que ocupa un registro cuyo payload mide `longitud`
/// bytes. Cada registro es `[longitud: u32 LE][payload postcard][relleno 0]`.
pub fn sectores_registro(longitud: usize) -> u64 {
    (4 + longitud).div_ceil(TAM_SECTOR) as u64
}

/// Compone el registro en disco de un payload: `[longitud u32 LE][payload]
/// [relleno a cero]`, alineado a un numero entero de sectores. Es el trazado
/// exacto que el kernel lee al reconstruir su indice — lo escriben tanto
/// `kernel::almacen` (al anexar un objeto) como `boot` (al sembrar la imagen).
pub fn componer_registro(payload: &[u8]) -> Vec<u8> {
    let n = sectores_registro(payload.len()) as usize;
    let mut registro = vec![0u8; n * TAM_SECTOR];
    registro[0..4].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    registro[4..4 + payload.len()].copy_from_slice(payload);
    registro
}

/// Lee la cabecera de longitud de un registro (sus 4 primeros bytes). Devuelve
/// `None` si la longitud es cero —fin del log— o supera [`MAX_OBJETO`]
/// —corrupcion—. Gemela de [`componer_registro`].
pub fn longitud_registro(cabecera: &[u8]) -> Option<usize> {
    if cabecera.len() < 4 {
        return None;
    }
    let longitud =
        u32::from_le_bytes([cabecera[0], cabecera[1], cabecera[2], cabecera[3]]) as usize;
    if longitud == 0 || longitud > MAX_OBJETO {
        None
    } else {
        Some(longitud)
    }
}

// =============================================================================
//  Fase 60 — Asistente Akasha: tipos de mensaje del canal del asistente
// -----------------------------------------------------------------------------
//  La app `asistente.wasm` (kernel-side) y el `asistente-puente` (host-side)
//  conversan por un canal Akasha bien conocido. Estos tipos definen el
//  protocolo. Diseñado para serializarse con `postcard` (el mismo encoder
//  que usa todo el resto del kernel) y vivir en `#![no_std] + alloc` para
//  cruzar la frontera kernel-wasm sin friction.
//
//  ESTADO (Fase 60 v1): tipos definidos, sin código que los consuma todavía.
//  Ver `docs/ASISTENTE_WAWA.md` §2.2 para el contexto del diseño.
// =============================================================================

/// Canal Akasha bien conocido para el asistente. ASCII `"AS"` = 0x4153. El
/// kernel filtra paquetes con este canal hacia los suscriptores del oficio
/// asistente; el puente Linux abre un socket raw que suscribe al mismo
/// número para recibir consultas y enviar propuestas.
///
/// NOTA: 0x4153 está dentro del rango histórico de "longitud" de Ethernet
/// (< 0x0600), así que NO sirve como EtherType. Para los frames del
/// asistente sobre el cable se usa [`ETHERTYPE_ASISTENTE`]; este valor
/// queda como discriminante interno (postcard tag, identificador del
/// oficio en logs y trazas).
pub const CANAL_ASISTENTE: u16 = 0x4153;

/// EtherType de los frames del asistente sobre el cable. Vecino del
/// 0x88B5 que ya usa Akasha — los dos viven en el rango "experimental"
/// que la IEEE deja libre. El demuxer Akasha del kernel (`akasha.rs`)
/// trata frames con EtherType ajeno como "para el usuario": los encola
/// tal cual sin procesar. La app `asistente.wasm` los recoge con
/// `sys_net_recibir`, filtra por este EtherType y decodifica el payload
/// como [`MensajeAsistente`] postcard.
///
/// Mantenerlo distinto de 0x88B5 evita que el demuxer intente
/// deserializar el payload como `MensajeAkasha` y lo descarte como
/// `PayloadInvalido` antes de pasarlo al usuario.
pub const ETHERTYPE_ASISTENTE: u16 = 0x88B6;

/// Acción que el LLM (vía el puente) propone al asistente. La app pinta
/// la propuesta, el humano decide. Acciones potentes (re-anclar manifiesto,
/// cambiar configuración) referencian objetos del grafo por `Hash` — el
/// puente los preparó y los ingestó vía Akasha; el kernel los verifica al
/// aplicar.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum AccionPropuesta {
    /// Lanzar la app `plantilla`-ésima del manifiesto. Equivalente al
    /// `Mando::LanzarFila` del launcher, pero dirigido por LLM.
    LanzarApp { plantilla: u32 },
    /// Re-anclar el manifiesto al hash propuesto. Requiere firma humana
    /// vía `daemon-firma` antes de invocar `sys_manifiesto_proponer`.
    InstalarApp { manifiesto_propuesto: Hash },
    /// Cambiar la `Configuracion` activa al hash propuesto. Mismo flujo
    /// de firma humana que `InstalarApp`.
    CambiarConfiguracion { config_propuesta: Hash },
    /// Sin efecto sobre el sistema — el LLM nada más anota algo para que
    /// el humano lo lea. Útil para responder preguntas tipo "¿cuántas
    /// apps tengo?" sin disparar acciones.
    Notar { texto: String },
}

/// Contexto del estado actual del nodo wawa que la app envía al puente
/// junto con la consulta. Permite que el LLM responda con info concreta
/// (nombres de apps reales, configuración activa) en lugar de a ciegas.
/// Lo que se incluye está acotado deliberadamente — más campos = más
/// tokens en el system prompt = más coste.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
pub struct Contexto {
    /// Nombres de las apps del manifiesto vivo, en el orden del catálogo
    /// del launcher. El LLM puede usar `LanzarApp { plantilla: i }` con
    /// el índice de la fila correspondiente.
    pub apps: Vec<String>,
    /// Hash del manifiesto vigente. Permite que el puente detecte si su
    /// caché local quedó stale (otro nodo re-ancló en paralelo) y
    /// rerequiera contexto fresco.
    pub manifiesto_actual: Option<Hash>,
    /// Hash de la `Configuracion` activa, si la hay. `None` si el
    /// manifiesto no enlaza ninguna.
    pub configuracion_activa: Option<Hash>,
}

/// Un mensaje sobre el canal `CANAL_ASISTENTE`. La app y el puente
/// hablan exclusivamente este enum — un atacante que envíe payload ajeno
/// al canal se queda sin decodificar (postcard rechaza el frame).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum MensajeAsistente {
    /// La app pregunta. El puente lo retransmite al LLM. `id` correlaciona
    /// request/response — un puente sirviendo varios nodos los distingue
    /// por id ANTES de cualquier RTT extra.
    Consulta {
        id: u64,
        prompt: String,
        contexto: Contexto,
    },
    /// El puente responde con una propuesta interpretada del LLM.
    /// `confianza` es la decisión del puente — `1.0` si el LLM produjo
    /// JSON limpio y la acción está en la lista blanca; valores menores
    /// si tuvo que adivinar o si el parseo fue parcial.
    Propuesta {
        id: u64,
        accion: AccionPropuesta,
        explicacion: String,
        confianza: f32,
    },
    /// El puente reporta un fallo de transporte o parseo. El `id`
    /// correlaciona contra la consulta original; el `motivo` es un string
    /// libre que la app pinta al humano.
    Error { id: u64, motivo: String },
}

impl MensajeAsistente {
    /// Serializa con postcard. El kernel lo manda por Akasha; el puente
    /// lo recibe y deserializa.
    pub fn serializar(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    /// Deserializa desde bytes. Si el frame está truncado o el canal
    /// trajo basura ajena, devuelve error sin tocar `self`.
    pub fn deserializar(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

// =============================================================================
//  Protocolo "cable" del asistente — alfabeto minimo sin alloc
// -----------------------------------------------------------------------------
//  `MensajeAsistente` (arriba) usa `String` y `Vec` para empaquetar prompts
//  y explicaciones de longitud arbitraria. La app `asistente.wasm` corre en
//  no_std SIN alloc — no puede construir esos tipos. Para el cable definimos
//  un alfabeto minimo que cabe en arrays fijos: cabecera de 12 bytes
//  (canal + tipo + id) + payload de longitud inferida del frame.
//
//  El puente Linux traduce entre el rico `MensajeAsistente` (que usa para
//  hablar con pluma-llm) y este protocolo cable (que viaja por Akasha).
// =============================================================================

/// Tamaño en bytes de la cabecera del protocolo cable.
/// `canal (2) + tipo (2) + id (8) = 12`.
pub const TAM_CABECERA_CABLE: usize = 12;

/// Tipos de mensaje sobre el cable del asistente. Discriminante u16 big
/// endian estable — los lectores binarios pueden grep por estos valores.
#[repr(u16)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipoCable {
    /// Consulta de la app al puente. Payload = bytes ASCII del prompt
    /// (sin nul terminator — la longitud se infiere del frame).
    Consulta = 1,
    /// Propuesta del puente del tipo `Notar` (la IA contestó algo
    /// informativo). Payload = bytes ASCII del texto.
    PropuestaNotar = 2,
    /// Propuesta del puente del tipo `LanzarApp`. Payload = u32 BE con
    /// el índice de plantilla a lanzar (4 bytes).
    PropuestaLanzarApp = 3,
    /// Propuesta de re-anclar el manifiesto. Payload = 32 bytes del hash.
    PropuestaInstalarApp = 4,
    /// Propuesta de cambiar la configuración activa. Payload = 32 bytes
    /// del hash de la nueva configuración.
    PropuestaCambiarConfig = 5,
    /// Error reportado por el puente (transporte, rechazo del LLM,
    /// parseo). Payload = bytes ASCII del motivo.
    Error = 6,
    /// Fase 60 v4 :: la app `asistente.wasm` pide la firma humana de un
    /// objeto (manifiesto/configuración). El puente lo relaya al
    /// `wawactl daemon-firma` por su transporte normal (PTY/virtio-console)
    /// y devuelve la firma en un [`TipoCable::Firma`]. Payload:
    /// `[tipo_obj: u8, hash: [u8; 32]]` = 33 bytes.
    ///   - `tipo_obj` = [`TIPO_OBJETO_CUADERNO`] (1) si el hash es de
    ///     manifiesto/cuaderno (legacy `wawa::sign_request::`).
    ///   - `tipo_obj` = [`TIPO_OBJETO_CONFIGURACION`] (2) si es de
    ///     configuración (`wawa::sign_config::`).
    /// Otros valores son rechazados por el puente con un `TipoCable::Error`.
    RequestFirma = 7,
    /// Fase 60 v4 :: respuesta del puente con la firma humana ya
    /// autorizada por el operador (via `daemon-firma`). Payload:
    /// `[slot: u8, firma: [u8; 64]]` = 65 bytes. `slot` es 0/1/2 — el
    /// índice dentro de `AGORA_AUTH_RING` que el operador eligió al
    /// arrancar el demonio. El asistente.wasm construye el sobre
    /// firmado y, cuando tenga PERMISO_RAIZ (hito 6), invoca
    /// `sys_manifiesto_proponer`.
    Firma = 8,
}

/// FASE 60 v4 :: discriminantes del primer byte del payload de
/// `TipoCable::RequestFirma`. El puente los mapea al prefijo correcto
/// para `daemon-firma` (`wawa::sign_request::` vs `wawa::sign_config::`).
/// El mismo discriminante puede aparecer en logs del operador.
pub const TIPO_OBJETO_CUADERNO: u8 = 1;
/// Como [`TIPO_OBJETO_CUADERNO`] pero para configuraciones. Ver Fase 60 v2
/// del `wawactl daemon-firma` — el prefijo correspondiente es
/// `wawa::sign_config::`.
pub const TIPO_OBJETO_CONFIGURACION: u8 = 2;

impl TipoCable {
    /// Traduce un u16 al variant correspondiente o `None` si es
    /// desconocido (el cable trajo un tipo no registrado).
    pub fn de_u16(v: u16) -> Option<Self> {
        match v {
            1 => Some(Self::Consulta),
            2 => Some(Self::PropuestaNotar),
            3 => Some(Self::PropuestaLanzarApp),
            4 => Some(Self::PropuestaInstalarApp),
            5 => Some(Self::PropuestaCambiarConfig),
            6 => Some(Self::Error),
            7 => Some(Self::RequestFirma),
            8 => Some(Self::Firma),
            _ => None,
        }
    }
}

/// Escribe la cabecera del cable en `out`. Devuelve la longitud escrita
/// (siempre `TAM_CABECERA_CABLE`) o `None` si `out` no cabe — el caller
/// reserva el buffer apropiado.
pub fn escribir_cabecera_cable(out: &mut [u8], tipo: TipoCable, id: u64) -> Option<usize> {
    if out.len() < TAM_CABECERA_CABLE {
        return None;
    }
    out[0..2].copy_from_slice(&CANAL_ASISTENTE.to_be_bytes());
    out[2..4].copy_from_slice(&(tipo as u16).to_be_bytes());
    out[4..12].copy_from_slice(&id.to_be_bytes());
    Some(TAM_CABECERA_CABLE)
}

/// Lee la cabecera del cable y verifica que el canal sea el del
/// asistente. Devuelve `(tipo, id)` o `None` si los bytes son
/// insuficientes, el canal no coincide o el tipo es desconocido. El
/// llamante interpreta `&bytes[TAM_CABECERA_CABLE..]` según `tipo`.
pub fn leer_cabecera_cable(bytes: &[u8]) -> Option<(TipoCable, u64)> {
    if bytes.len() < TAM_CABECERA_CABLE {
        return None;
    }
    let canal = u16::from_be_bytes([bytes[0], bytes[1]]);
    if canal != CANAL_ASISTENTE {
        return None;
    }
    let tipo_raw = u16::from_be_bytes([bytes[2], bytes[3]]);
    let tipo = TipoCable::de_u16(tipo_raw)?;
    let id = u64::from_be_bytes([
        bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
    ]);
    Some((tipo, id))
}

