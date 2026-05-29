//! El nodo de la conversación — la unidad firmada y direccionada por contenido.
//!
//! Un mensaje de Ayni NO es una fila en un log lineal: es un nodo de un DAG.
//! Su identidad (`id`) es el hash BLAKE3 de su CONTENIDO canónico —autor,
//! padres, carga, instante—; sus `padres` son los ids de los nodos que ya
//! existían cuando se escribió. Eso da, gratis, tres propiedades:
//!
//!   * **Hilos reales.** Dos nodos pueden compartir padre (la conversación se
//!     bifurca) y otro nodo puede tomar a ambos como padres (se reconcilia).
//!     No hay "orden verdadero" impuesto por un servidor: el orden lo dicta el
//!     grafo, y dos pares que vieron los mismos nodos calculan el MISMO grafo.
//!   * **Acíclico por construcción.** Un `padre` es el hash del contenido de
//!     otro nodo; para crear un ciclo habría que conocer un hash antes de
//!     escribir el contenido que lo produce. Criptográficamente imposible.
//!     No hace falta detectar ciclos: no pueden existir.
//!   * **No-repudio e integridad en una sola pieza.** La `firma` respalda el
//!     `id` (los 32 bytes del hash del contenido). Como el id resume el
//!     contenido entero, firmar el id equivale a firmar todo —incluidos los
//!     padres—: nadie puede reordenar el hilo de un autor sin invalidar su
//!     firma. (Mismo idioma que `format::ManifiestoFirmado`.)

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

use format::{AgoraId, Firma, Hash};

/// Versión del formato de un nodo. Subirla es un pacto explícito: un núcleo
/// viejo rechaza un nodo de versión desconocida en vez de malinterpretarlo.
pub const VERSION_NODO: u32 = 1;

/// Una REFERENCIA VIVA a un objeto del grafo soberano —no una copia muerta—.
///
/// Todo en gioser se direcciona por contenido (BLAKE3): un documento de pluma,
/// una nota de khipu, una carta de cosmos, un archivo. Adjuntar uno a un mensaje
/// es citar su `hash`, no duplicar sus bytes. El mismo hash en el grafo de la app
/// de origen y en esta referencia apuntan AL MISMO objeto: editar en origen
/// engendra un hash nuevo (otra versión), y el adjunto puede apuntar a cualquiera.
/// La dedup es gratis —dos mensajes que adjuntan el mismo objeto comparten un
/// solo blob— y la referencia viaja DENTRO del contenido firmado: nadie puede
/// alterar a qué objeto apunta sin invalidar la firma.
///
/// Los bytes del objeto NO viajan aquí (la referencia es minúscula); viajan
/// aparte como un blob direccionado por `hash`, y se verifican contra él al
/// recibirse ([`Adjunto::verifica`]).
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Adjunto {
    /// El hash BLAKE3 del contenido del objeto — su identidad en el grafo.
    pub hash: Hash,
    /// De qué app del grafo proviene: `"pluma"`, `"khipu"`, `"cosmos"`,
    /// `"archivo"`… Informa a la UI cómo abrirlo/editarlo en su app nativa.
    pub app: String,
    /// La clase/MIME del contenido: `"text/markdown"`, `"image/png"`,
    /// `"pluma/documento"`. Guía el render y la resolución.
    pub clase: String,
    /// Nombre legible para mostrar.
    pub nombre: String,
    /// Tamaño del contenido en bytes — para mostrar y para acotar transferencias.
    pub tamano: u64,
}

impl Adjunto {
    /// Crea una referencia a partir de los bytes del objeto: calcula su hash
    /// BLAKE3 (la misma función del grafo) y su tamaño. Los `bytes` son el blob
    /// que el llamador guardará/transferirá aparte; aquí sólo nace la referencia.
    pub fn de_bytes(
        app: impl Into<String>,
        clase: impl Into<String>,
        nombre: impl Into<String>,
        bytes: &[u8],
    ) -> Adjunto {
        Adjunto {
            hash: format::hash(bytes),
            app: app.into(),
            clase: clase.into(),
            nombre: nombre.into(),
            tamano: bytes.len() as u64,
        }
    }

    /// ¿Estos bytes SON el objeto que la referencia nombra? Comprueba tamaño y
    /// hash BLAKE3. Es la verificación que se corre al recibir un blob por la
    /// red antes de aceptarlo: el direccionamiento por contenido hace imposible
    /// colar bytes ajenos bajo un hash dado.
    pub fn verifica(&self, bytes: &[u8]) -> bool {
        bytes.len() as u64 == self.tamano && format::hash(bytes) == self.hash
    }
}

/// La carga útil de un mensaje. Es un `enum` —no un `String` pelado— para
/// poder crecer sin romper nodos ya firmados: adjuntos como objetos del grafo,
/// ediciones con procedencia, reacciones, lienzos derivados (traducción /
/// resumen) del modelo multilienzo. Las variantes NUEVAS se añaden SOLO AL
/// FINAL —postcard asigna los tags por orden y mover uno rompería la firma de
/// todo nodo viejo, porque cambiaría su contenido canónico y por ende su id—.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum Carga {
    /// Un mensaje de texto plano (UTF-8). La carga de base de P0.
    Texto(String),
    /// Carga CIFRADA extremo-a-extremo: el blob AEAD opaco —`nonce || ciphertext`—
    /// que produce `ayni-crypto::CanalSeguro` (P2). El grafo, el id y la firma
    /// operan sobre ESTE ciphertext, así que integridad y autoría siguen intactas
    /// y públicamente verificables; la confidencialidad la añade el cifrado y sólo
    /// el destinatario del canal recupera el claro. Por eso el E2EE es ORTOGONAL
    /// al transporte: la red mueve un nodo con carga `Cifrado` sin enterarse de
    /// que lo es —no hay nada que adaptar aguas abajo—.
    Cifrado(Vec<u8>),
    /// Un ADJUNTO: una referencia viva a un objeto del grafo (P5). No copia los
    /// bytes —cita su hash—; los bytes viajan aparte como blob y se verifican
    /// contra la referencia. Ver [`Adjunto`].
    Adjunto(Adjunto),
}

impl Carga {
    /// Vista del texto cuando la carga ES texto plano — conveniencia para la UI
    /// y la indexación semántica (P4) sin destripar el `enum` en cada sitio.
    /// Una carga `Cifrado` devuelve `None`: hay que descifrarla antes.
    pub fn texto(&self) -> Option<&str> {
        match self {
            Carga::Texto(t) => Some(t),
            _ => None,
        }
    }

    /// El blob AEAD cuando la carga está cifrada — lo que `CanalSeguro::descifrar`
    /// consume. `None` en otro caso.
    pub fn cifrado(&self) -> Option<&[u8]> {
        match self {
            Carga::Cifrado(b) => Some(b),
            _ => None,
        }
    }

    /// La referencia de adjunto cuando la carga es un adjunto. `None` en otro caso.
    pub fn adjunto(&self) -> Option<&Adjunto> {
        match self {
            Carga::Adjunto(a) => Some(a),
            _ => None,
        }
    }
}

/// El contenido FIRMABLE y DIRECCIONABLE de un nodo: todo menos la firma.
///
/// El id del nodo es `hash(postcard(Contenido))`. La firma NO entra en el
/// contenido (un nodo no puede firmar su propia firma); entra en
/// [`MensajeNodo`], que envuelve este contenido con el sello del autor.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Contenido {
    /// Versión del formato — debe ser [`VERSION_NODO`].
    pub version: u32,
    /// La identidad agora del autor: su clave pública Ed25519 (32 bytes). La
    /// firma del nodo debe validar contra esta clave.
    pub autor: AgoraId,
    /// Los ids de los nodos previos — las aristas salientes del DAG. ORDENADOS
    /// y sin duplicados (lo impone [`Contenido::nuevo`]) para que el mismo
    /// conjunto de padres produzca SIEMPRE el mismo id: el direccionamiento por
    /// contenido exige determinismo bit-a-bit. Vacío ⇒ el nodo es una raíz.
    pub padres: Vec<Hash>,
    /// La carga del mensaje.
    pub carga: Carga,
    /// Instante de autoría, segundos desde el epoch UNIX. Lo PROVEE el llamador
    /// —este núcleo `no_std` no lee ningún reloj (en wawa el tiempo lo da el
    /// kernel; en Linux, la capa de app)—. Es un dato declarado por el autor:
    /// sirve para ordenar legiblemente y para desconfiar de timestamps
    /// absurdamente futuros; nunca es la fuente de verdad del ORDEN causal —ese
    /// lo dan los `padres`—.
    pub ts: u64,
}

impl Contenido {
    /// Compone un contenido nuevo, normalizando los padres a la forma canónica
    /// (ordenados, sin duplicados). Es el único constructor que garantiza que
    /// dos llamadas con el mismo conjunto de padres —en cualquier orden— rinden
    /// el mismo id.
    pub fn nuevo(autor: AgoraId, padres: Vec<Hash>, carga: Carga, ts: u64) -> Self {
        let mut padres = padres;
        padres.sort_unstable();
        padres.dedup();
        Contenido {
            version: VERSION_NODO,
            autor,
            padres,
            carga,
            ts,
        }
    }

    /// La forma binaria canónica del contenido — lo que se hashea para el id.
    ///
    /// `postcard::to_allocvec` sobre una estructura de primitivos, arrays de
    /// bytes, `Vec` y un `enum` simple no tiene ningún camino de error en la
    /// práctica (el sumidero `alloc` crece a demanda); de ahí el `expect`. Si
    /// alguna vez fallara, el grafo entero sería incoherente y abortar es lo
    /// correcto, no propagar un id basura.
    pub fn bytes_canonicos(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("ayni :: postcard alloc no falla para Contenido")
    }

    /// El id del nodo: BLAKE3 de su contenido canónico. La misma función hash
    /// que da identidad a todo objeto del grafo soberano (`format::hash`).
    pub fn id(&self) -> Hash {
        format::hash(&self.bytes_canonicos())
    }
}

/// Un nodo completo de la conversación: su contenido + la firma del autor sobre
/// el id. Es lo que viaja por la red (P1+) y lo que se guarda en disco — el
/// objeto direccionado por su `id()`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct MensajeNodo {
    /// El contenido firmable/direccionable.
    pub contenido: Contenido,
    /// Firma Ed25519 (64 bytes) del autor sobre los 32 bytes de `contenido.id()`.
    /// `serde` no deriva para `[u8; 64]` sin ayuda; `serde-big-array` la da.
    #[serde(with = "BigArray")]
    pub firma: Firma,
}

impl MensajeNodo {
    /// Sella un contenido: calcula su id y lo entrega al closure firmante.
    ///
    /// La firma se inyecta por closure DELIBERADAMENTE: `ayni-core` no enlaza
    /// ninguna primitiva criptográfica (sería peso muerto en wawa y violaría la
    /// capa `ayni-crypto`). El llamador pasa `|id| keypair.sign(id)` —o el
    /// firmante de `agora`— y el núcleo se limita a empaquetar. La regla dura
    /// del proyecto se respeta: nadie hace cripto a mano aquí.
    pub fn sellar(contenido: Contenido, firmar: impl FnOnce(&Hash) -> Firma) -> Self {
        let id = contenido.id();
        let firma = firmar(&id);
        MensajeNodo { contenido, firma }
    }

    /// El id (dirección de contenido) de este nodo.
    pub fn id(&self) -> Hash {
        self.contenido.id()
    }

    /// La identidad agora del autor.
    pub fn autor(&self) -> &AgoraId {
        &self.contenido.autor
    }

    /// Los ids de los padres — las aristas del DAG.
    pub fn padres(&self) -> &[Hash] {
        &self.contenido.padres
    }

    /// Verifica la firma del nodo delegando en un closure verificador
    /// `(autor, id, firma) -> bool`. Recalcula el id desde el contenido: si
    /// alguien manipuló el contenido tras firmar, el id cambia y la firma
    /// —hecha sobre el id viejo— deja de validar. Igual que el sellado, la
    /// primitiva Ed25519 la pone quien llama (`ayni-crypto`/`agora`).
    pub fn verificar(&self, verificar: impl FnOnce(&AgoraId, &Hash, &Firma) -> bool) -> bool {
        verificar(&self.contenido.autor, &self.id(), &self.firma)
    }

    /// La forma `postcard` de UN nodo suelto — la que viaja por el cable
    /// (`Sobre::Nodo` de `ayni-sync`) y la que se guarda direccionada por su id
    /// en un store key-value (`ayni-store`) o como objeto del grafo de akasha
    /// (la app de wawa, P6). Espeja a [`Conversacion::serializar`], pero para un
    /// solo nodo: el grano fino que la anti-entropía y el grafo de objetos piden.
    pub fn serializar(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("ayni :: postcard alloc no falla para MensajeNodo")
    }

    /// Reconstruye un nodo desde su forma `postcard`. No verifica la firma —eso
    /// es trabajo del closure verificador en la capa que tenga la cripto—.
    pub fn deserializar(bytes: &[u8]) -> Result<Self, crate::ErrorAyni> {
        postcard::from_bytes(bytes).map_err(|_| crate::ErrorAyni::Deserializacion)
    }
}
