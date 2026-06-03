// =============================================================================
//  uya-core — el modelo agnóstico de una videollamada soberana.
// -----------------------------------------------------------------------------
//  Tres cosas, ninguna más:
//    · `ParticipanteId` + `id_desde_nombre` — identidad determinista BLAKE3.
//    · `Paquete` — el protocolo de cable (presentación, estado de medios,
//      cuadro de video, despedida). (De)serializa con postcard, como el resto
//      de la suite.
//    · `Sala` — el roster: quién está, con cámara/micrófono encendidos.
//
//  Lo que NO vive aquí: transporte (TCP/card-net → `uya-app`), captura de
//  cámara/micrófono (`uya-app`) y el pintado (`uya-llimphi`). Así el núcleo no
//  sabe quién lo pinta ni cómo viaja — la regla dura del repo.
// =============================================================================

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// Identidad de un participante = BLAKE3 de su **clave pública** Ed25519. Es
/// inforjable: para presentarse con un id hay que poseer la clave secreta que lo
/// engendra (ver `uya-app::identidad`). El nombre es sólo una etiqueta —dos
/// personas pueden llamarse igual y tendrán ids distintos—; la identidad real es
/// esta huella, que se verifica una vez fuera de banda (TOFU).
pub type ParticipanteId = [u8; 32];

/// Deriva el id de un participante a partir de su clave pública Ed25519. Espeja
/// a `agora_core::IdentityId::from_public_key` (BLAKE3 de la pubkey).
pub fn id_desde_clave(clave: &[u8; 32]) -> ParticipanteId {
    *blake3::hash(clave).as_bytes()
}

/// Deriva un id determinista a partir de un nombre. **No** es la identidad de un
/// participante (ésa sale de su clave, ver [`id_desde_clave`]); se reserva para
/// claves de espacio de nombres públicas: la clave DHT de una sala
/// (`uya/sala/<n>`), por ejemplo.
pub fn id_desde_nombre(nombre: &str) -> ParticipanteId {
    *blake3::hash(nombre.as_bytes()).as_bytes()
}

/// Bytes canónicos que firma una identidad en su `Hola` para atestiguar el par
/// `(id, nombre)`. El receptor verifica la firma contra la clave pública
/// declarada y comprueba además que `id == id_desde_clave(clave)`; así el
/// nombre queda ligado a una clave que el emisor probó poseer.
pub fn mensaje_identidad(id: &ParticipanteId, nombre: &str) -> Vec<u8> {
    let mut m = Vec::with_capacity(16 + 32 + nombre.len());
    m.extend_from_slice(b"uya/identidad/v1");
    m.extend_from_slice(id);
    m.extend_from_slice(nombre.as_bytes());
    m
}

/// Forma corta legible de un id (8 hex), para etiquetas y logs.
pub fn hex_corto(id: &ParticipanteId) -> String {
    let mut s = String::with_capacity(8);
    for b in &id[..4] {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Cómo viajan los bytes de un cuadro de video por el cable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatoCuadro {
    /// RGBA8 crudo (4 bytes/pixel). Sin compresión — sólo para LAN o preview.
    Rgba,
    /// JPEG comprimido (MJPEG por cuadro). El default del cable: ~20-40× menos
    /// bytes que RGBA, sin estado inter-cuadro (baja latencia).
    Jpeg,
}

/// El protocolo de cable de uya. Cada conexión es full-duplex y transporta
/// estos paquetes enmarcados (largo u32 + postcard) — ver `uya-app`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Paquete {
    /// Presentación: lo primero que envía cada extremo al conectar. Lleva la
    /// identidad, el nombre y la **prueba de identidad**: la clave pública
    /// Ed25519 y una firma sobre [`mensaje_identidad`]`(id, nombre)`. El receptor
    /// comprueba `id == id_desde_clave(clave)` y que la firma valide contra
    /// `clave` — recién entonces puebla el roster como verificado.
    Hola {
        id: ParticipanteId,
        nombre: String,
        /// Clave pública Ed25519 de quien se presenta (32 bytes).
        clave: [u8; 32],
        /// Firma Ed25519 (64 bytes) sobre `mensaje_identidad(id, nombre)`.
        firma: Vec<u8>,
    },
    /// Estado de medios del emisor (cámara / micrófono encendidos). Se envía
    /// al conectar y cada vez que el humano togglea.
    Estado { camara: bool, microfono: bool },
    /// Un cuadro de video de `ancho × alto`, en el `formato` indicado (RGBA8
    /// crudo o JPEG comprimido). `seq` es monótono creciente para descartar
    /// cuadros viejos. El receptor decodifica a RGBA antes de pintarlo.
    Cuadro {
        ancho: u16,
        alto: u16,
        seq: u32,
        formato: FormatoCuadro,
        datos: Vec<u8>,
    },
    /// Un paquete de audio **Opus** (20 ms @ 48 kHz mono, el formato canónico
    /// del cable). El receptor lo decodifica a PCM y lo mezcla/resamplea al
    /// formato de su dispositivo de salida (ver `uya-app`).
    Audio { opus: Vec<u8> },
    /// Un mensaje de texto difundido a la sala (la charla lateral de la
    /// llamada). El emisor ya se presentó con `Hola`, así que sólo viaja el
    /// cuerpo; el receptor le pega el nombre del par desde su roster.
    Mensaje { texto: String },
    /// Difusión de direcciones dialables conocidas, para armar la malla N-a-N:
    /// cada nodo comparte las multiaddrs (`/ip4/.../p2p/<peerid>`) que conoce y
    /// el receptor disca las que le falten. Así, uniéndose a un solo anfitrión,
    /// todos terminan conectados con todos (ver `uya-app::enlace`).
    Pares { direcciones: Vec<String> },
    /// Me voy de la llamada (cuelgue ordenado).
    Adios,
}

impl Paquete {
    /// Serializa a postcard. El protocolo es interno y estable por sesión, así
    /// que un fallo de encode es un bug, no un caso de runtime.
    pub fn codificar(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("uya: postcard encode")
    }

    /// Deserializa un paquete recibido por el cable.
    pub fn decodificar(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

/// Un participante remoto visto en la sala.
#[derive(Clone, Debug)]
pub struct Participante {
    pub id: ParticipanteId,
    pub nombre: String,
    pub camara: bool,
    pub microfono: bool,
    /// `true` si su `Hola` traía una firma válida ligando su clave a `(id,
    /// nombre)`. Un par sin verificar igual se muestra, pero la UI lo señala.
    pub verificado: bool,
}

/// El roster de la llamada: yo + los demás. No guarda cuadros de video (eso es
/// efímero y vive en `uya-app`/UI); sólo el quién-está y su estado de medios.
#[derive(Clone, Debug)]
pub struct Sala {
    pub yo: ParticipanteId,
    pub mi_nombre: String,
    pub participantes: BTreeMap<ParticipanteId, Participante>,
}

impl Sala {
    /// Crea una sala vacía con la identidad propia ya resuelta (la huella de la
    /// clave local, ver `uya-app::identidad`).
    pub fn nueva(yo: ParticipanteId, mi_nombre: impl Into<String>) -> Self {
        Self {
            yo,
            mi_nombre: mi_nombre.into(),
            participantes: BTreeMap::new(),
        }
    }

    /// Registra (o re-nombra) a un participante. Devuelve `true` si era nuevo.
    pub fn entrar(&mut self, id: ParticipanteId, nombre: String, verificado: bool) -> bool {
        match self.participantes.get_mut(&id) {
            Some(p) => {
                p.nombre = nombre;
                p.verificado = verificado;
                false
            }
            None => {
                self.participantes.insert(
                    id,
                    Participante {
                        id,
                        nombre,
                        camara: true,
                        microfono: true,
                        verificado,
                    },
                );
                true
            }
        }
    }

    /// Saca a un participante (cuelgue o desconexión).
    pub fn salir(&mut self, id: &ParticipanteId) {
        self.participantes.remove(id);
    }

    /// Actualiza el estado de medios de un participante ya presente.
    pub fn set_estado(&mut self, id: &ParticipanteId, camara: bool, microfono: bool) {
        if let Some(p) = self.participantes.get_mut(id) {
            p.camara = camara;
            p.microfono = microfono;
        }
    }

    /// Cuántas caras hay en la llamada (los demás + yo).
    pub fn cuenta(&self) -> usize {
        self.participantes.len() + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_determinista() {
        assert_eq!(id_desde_nombre("Alicia"), id_desde_nombre("Alicia"));
        assert_ne!(id_desde_nombre("Alicia"), id_desde_nombre("Beto"));
    }

    #[test]
    fn id_desde_clave_es_blake3_de_la_clave() {
        let clave = [9u8; 32];
        assert_eq!(id_desde_clave(&clave), *blake3::hash(&clave).as_bytes());
        assert_ne!(id_desde_clave(&[1u8; 32]), id_desde_clave(&[2u8; 32]));
    }

    #[test]
    fn mensaje_identidad_liga_id_y_nombre() {
        let id = id_desde_clave(&[5u8; 32]);
        // Cambiar el nombre o el id cambia el mensaje firmado.
        assert_ne!(
            mensaje_identidad(&id, "Alicia"),
            mensaje_identidad(&id, "Beto")
        );
        assert_ne!(
            mensaje_identidad(&id, "Alicia"),
            mensaje_identidad(&id_desde_clave(&[6u8; 32]), "Alicia")
        );
    }

    #[test]
    fn paquete_roundtrip() {
        let casos = [
            Paquete::Hola {
                id: id_desde_clave(&[1u8; 32]),
                nombre: "Alicia".into(),
                clave: [1u8; 32],
                firma: vec![7u8; 64],
            },
            Paquete::Estado {
                camara: true,
                microfono: false,
            },
            Paquete::Cuadro {
                ancho: 4,
                alto: 2,
                seq: 7,
                formato: FormatoCuadro::Jpeg,
                datos: vec![1, 2, 3, 4, 5, 6, 7, 8],
            },
            Paquete::Audio {
                opus: vec![0xfc, 0x01, 0x02, 0x03],
            },
            Paquete::Mensaje {
                texto: "hola, ¿me ven?".into(),
            },
            Paquete::Pares {
                direcciones: vec!["/ip4/127.0.0.1/tcp/7800/p2p/12D3Koo".into()],
            },
            Paquete::Adios,
        ];
        for p in casos {
            let bytes = p.codificar();
            let vuelta = Paquete::decodificar(&bytes).expect("decode");
            // Comparamos por su forma serializada (Paquete no es PartialEq).
            assert_eq!(bytes, vuelta.codificar());
        }
    }

    #[test]
    fn roster_entrar_salir_estado() {
        let mut sala = Sala::nueva(id_desde_clave(&[0u8; 32]), "Alicia");
        assert_eq!(sala.cuenta(), 1);
        let beto = id_desde_clave(&[2u8; 32]);
        assert!(sala.entrar(beto, "Beto".into(), true));
        assert!(!sala.entrar(beto, "Beto".into(), true));
        assert_eq!(sala.cuenta(), 2);
        sala.set_estado(&beto, false, true);
        assert!(!sala.participantes[&beto].camara);
        assert!(sala.participantes[&beto].microfono);
        sala.salir(&beto);
        assert_eq!(sala.cuenta(), 1);
    }
}
