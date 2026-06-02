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

/// Identidad de un participante = BLAKE3 de su nombre. Estable y determinista:
/// el mismo nombre produce el mismo id en cualquier máquina, igual que la
/// semilla de identidad de `ayni`/`agora`.
pub type ParticipanteId = [u8; 32];

/// Deriva la identidad determinista de un participante a partir de su nombre.
pub fn id_desde_nombre(nombre: &str) -> ParticipanteId {
    *blake3::hash(nombre.as_bytes()).as_bytes()
}

/// Forma corta legible de un id (8 hex), para etiquetas y logs.
pub fn hex_corto(id: &ParticipanteId) -> String {
    let mut s = String::with_capacity(8);
    for b in &id[..4] {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// El protocolo de cable de uya. Cada conexión es full-duplex y transporta
/// estos paquetes enmarcados (largo u32 + postcard) — ver `uya-app`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Paquete {
    /// Presentación: lo primero que envía cada extremo al conectar. Lleva la
    /// identidad y el nombre para que el otro lado pueble su roster.
    Hola {
        id: ParticipanteId,
        nombre: String,
    },
    /// Estado de medios del emisor (cámara / micrófono encendidos). Se envía
    /// al conectar y cada vez que el humano togglea.
    Estado { camara: bool, microfono: bool },
    /// Un cuadro de video RGBA8 (4 bytes/pixel) ya escalado a `ancho × alto`.
    /// `seq` es monótono creciente para poder descartar cuadros viejos.
    Cuadro {
        ancho: u16,
        alto: u16,
        seq: u32,
        rgba: Vec<u8>,
    },
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
    /// Crea una sala vacía cuya identidad propia deriva del nombre.
    pub fn nueva(mi_nombre: impl Into<String>) -> Self {
        let mi_nombre = mi_nombre.into();
        Self {
            yo: id_desde_nombre(&mi_nombre),
            mi_nombre,
            participantes: BTreeMap::new(),
        }
    }

    /// Registra (o re-nombra) a un participante. Devuelve `true` si era nuevo.
    pub fn entrar(&mut self, id: ParticipanteId, nombre: String) -> bool {
        match self.participantes.get_mut(&id) {
            Some(p) => {
                p.nombre = nombre;
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
    fn paquete_roundtrip() {
        let casos = [
            Paquete::Hola {
                id: id_desde_nombre("Alicia"),
                nombre: "Alicia".into(),
            },
            Paquete::Estado {
                camara: true,
                microfono: false,
            },
            Paquete::Cuadro {
                ancho: 4,
                alto: 2,
                seq: 7,
                rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
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
        let mut sala = Sala::nueva("Alicia");
        assert_eq!(sala.cuenta(), 1);
        let beto = id_desde_nombre("Beto");
        assert!(sala.entrar(beto, "Beto".into()));
        assert!(!sala.entrar(beto, "Beto".into()));
        assert_eq!(sala.cuenta(), 2);
        sala.set_estado(&beto, false, true);
        assert!(!sala.participantes[&beto].camara);
        assert!(sala.participantes[&beto].microfono);
        sala.salir(&beto);
        assert_eq!(sala.cuenta(), 1);
    }
}
