//! El `Nucleo` — toda la lógica viva del chat, sin una sola línea de UI.
//!
//! Una UI (CLI o Llimphi) sostiene un `Nucleo`, le manda intenciones del humano
//! (`enviar_texto`, `adjuntar`, `admitir`, `atestar`, `acusar_recibo`…) y le
//! entrega cada `EventoRed` del transporte (`al_evento`). El `Nucleo`:
//!
//!   * redacta y firma cada acto como un nodo del DAG, lo persiste (local-first)
//!     y lo difunde;
//!   * reconcilia los grafos por anti-entropía (delegando en `ayni-sync`);
//!   * cifra 1:1 con el canal X25519 cuando está activo (P2);
//!   * adjunta objetos como referencias vivas + intercambia sus blobs (P5);
//!   * emite recibos SIMÉTRICOS: sólo acusa lo que recibe si el operador activó
//!     los recibos —y entonces ambos lados, opt-in, se ven mutuamente (ayni)—.

use std::collections::BTreeSet;
use std::path::Path;

use ayni_core::{
    AccionMembresia, Adjunto, AgoraId, Atestacion, CambioMembresia, Carga, Conversacion, Hash,
    MensajeNodo, Recibo,
};
use ayni_crypto::{verificar_firma, CanalSeguro, Identidad};
use ayni_store::AlmacenAyni;
use ayni_sync::{
    blob_valido, blobs_faltantes, servir_blobs, EventoRed, Fusionador, PeerId, Sobre, Transporte,
};

/// El núcleo de aplicación, agnóstico de la cara que lo pinta.
pub struct Nucleo {
    /// El grafo de la conversación (CRDT por construcción).
    pub conv: Conversacion,
    /// La anti-entropía: orquesta qué nodos pedir/entregar.
    fus: Fusionador,
    /// Persistencia local-first (sled). `None` ⇒ sólo en memoria.
    almacen: Option<AlmacenAyni>,
    /// La identidad agora del operador (firma + claves de canal).
    identidad: Identidad,
    /// Canal E2EE 1:1 con el peer, tras intercambiar claves X25519 (P2).
    canal: Option<CanalSeguro>,
    /// ¿Cifrar los textos salientes? (toggle del operador).
    pub cifrar: bool,
    /// ¿Emitir recibos? Opt-in; la simetría nace de que ambos lo activen.
    pub recibos: bool,
    /// Identidades que hemos visto EMITIR recibos — para mostrar reciprocidad.
    reciprocan: BTreeSet<AgoraId>,
}

impl Nucleo {
    /// Funda un núcleo con una identidad. Si `ruta_almacen` es `Some`, abre (o
    /// crea) el store y CARGA la conversación previa — local-first de verdad: al
    /// reabrir, el hilo sigue donde quedó. Un fallo de store no es fatal: se cae
    /// a memoria (mejor un chat efímero que ningún chat).
    pub fn nuevo(
        identidad: Identidad,
        ruta_almacen: Option<&Path>,
        cifrar: bool,
        recibos: bool,
    ) -> Self {
        let (almacen, conv) = match ruta_almacen {
            Some(ruta) => match AlmacenAyni::abrir(ruta) {
                Ok(a) => {
                    let conv = a.cargar().unwrap_or_else(|_| Conversacion::nueva());
                    (Some(a), conv)
                }
                Err(_) => (None, Conversacion::nueva()),
            },
            None => (None, Conversacion::nueva()),
        };
        Nucleo {
            conv,
            fus: Fusionador::nuevo(),
            almacen,
            identidad,
            canal: None,
            cifrar,
            recibos,
            reciprocan: BTreeSet::new(),
        }
    }

    /// La identidad agora del operador.
    pub fn yo(&self) -> AgoraId {
        self.identidad.agora_id()
    }

    /// ¿Hay canal E2EE establecido?
    pub fn tiene_canal(&self) -> bool {
        self.canal.is_some()
    }

    /// ¿Esta identidad nos correspondió con recibos (reciprocidad viva)?
    pub fn reciproca(&self, id: &AgoraId) -> bool {
        self.reciprocan.contains(id)
    }

    // --- Intenciones del humano: cada una redacta+firma+persiste+difunde. -----

    /// Envía un texto. Lo cifra si el toggle está activo y hay canal; si no, va
    /// en claro (la autoría es pública en ambos casos).
    pub fn enviar_texto<T: Transporte>(&mut self, enlace: &T, texto: &str) {
        let texto = texto.trim();
        if texto.is_empty() {
            return;
        }
        let carga = match (self.cifrar, &self.canal) {
            (true, Some(c)) => Carga::Cifrado(c.cifrar(texto.as_bytes())),
            _ => Carga::Texto(texto.into()),
        };
        self.redactar_y_difundir(enlace, carga);
    }

    /// Adjunta el archivo de `ruta`: guarda su blob en el store (dedup por hash),
    /// redacta un nodo con la referencia viva [`Adjunto`] y la difunde; el blob
    /// se servirá a quien lo pida (`Sobre::PedirBlob`). Devuelve el nombre o un
    /// error legible.
    pub fn adjuntar<T: Transporte>(&mut self, enlace: &T, ruta: &str) -> Result<String, String> {
        let bytes = std::fs::read(ruta).map_err(|e| format!("no pude leer «{ruta}»: {e}"))?;
        let nombre = Path::new(ruta)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("adjunto")
            .to_string();
        let (app, clase) = clasificar(&nombre);
        let adjunto = Adjunto::de_bytes(app, clase, nombre.clone(), &bytes);

        // El blob se guarda aparte (dedup + verificación); la referencia viaja
        // firmada dentro del nodo.
        if let Some(a) = &self.almacen {
            a.guardar_blob(&bytes)
                .map_err(|e| format!("no pude guardar el blob: {e}"))?;
        }
        self.redactar_y_difundir(enlace, Carga::Adjunto(adjunto));
        Ok(nombre)
    }

    /// Da de ALTA a `sujeto` (la autoridad la valida la derivación de membresía).
    pub fn admitir<T: Transporte>(&mut self, enlace: &T, sujeto: AgoraId) {
        self.redactar_y_difundir(
            enlace,
            Carga::Membresia(CambioMembresia {
                accion: AccionMembresia::Alta,
                sujeto,
            }),
        );
    }

    /// Da de BAJA a `sujeto`.
    pub fn expulsar<T: Transporte>(&mut self, enlace: &T, sujeto: AgoraId) {
        self.redactar_y_difundir(
            enlace,
            Carga::Membresia(CambioMembresia {
                accion: AccionMembresia::Baja,
                sujeto,
            }),
        );
    }

    /// ATESTIGUA a `sujeto` con `nivel` (`0` revoca).
    pub fn atestar<T: Transporte>(&mut self, enlace: &T, sujeto: AgoraId, nivel: u8) {
        self.redactar_y_difundir(enlace, Carga::Atestacion(Atestacion { sujeto, nivel }));
    }

    /// Acusa recibo de las cabezas actuales (acto manual; el automático simétrico
    /// lo dispara `al_evento` si los recibos están activos).
    pub fn acusar_cabezas<T: Transporte>(&mut self, enlace: &T) {
        let vistos = self.conv.cabezas();
        if !vistos.is_empty() {
            self.redactar_y_difundir(enlace, Carga::Recibo(Recibo { vistos }));
        }
    }

    /// El telar común: redacta el nodo sobre las cabezas actuales, lo sella con
    /// la identidad, lo agrega al grafo, lo persiste y lo difunde.
    fn redactar_y_difundir<T: Transporte>(&mut self, enlace: &T, carga: Carga) {
        let autor = self.identidad.agora_id();
        let nodo = self
            .conv
            .redactar(autor, carga, 0, |id| self.identidad.firmar(id));
        if self.conv.agregar(nodo.clone()).is_ok() {
            self.persistir(&nodo);
            let _ = enlace.difundir(&Sobre::Nodo(nodo));
        }
    }

    // --- Recepción: cada EventoRed del transporte pasa por aquí. --------------

    /// Procesa un evento de red y devuelve los ids de los nodos REALMENTE nuevos
    /// (para que la UI los resalte/desplace). Hace todo lo demás como efecto:
    /// saludo+anti-entropía al conectar, intercambio de claves, reconciliación
    /// de nodos y de blobs, persistencia y recibos simétricos.
    pub fn al_evento<T: Transporte>(&mut self, enlace: &T, evento: EventoRed) -> Vec<Hash> {
        match evento {
            EventoRed::Conectado(peer) => {
                // Saludar con la clave X25519 + anunciar cabezas (anti-entropía).
                let _ = enlace.enviar(
                    &peer,
                    &Sobre::Hola {
                        x25519: self.identidad.clave_publica_x25519(),
                    },
                );
                let _ = enlace.enviar(&peer, &Sobre::Cabezas(self.conv.cabezas()));
                Vec::new()
            }
            EventoRed::Desconectado(_) => Vec::new(),
            EventoRed::Sobre(_, Sobre::Hola { x25519 }) => {
                self.canal = Some(self.identidad.canal_con(&x25519));
                Vec::new()
            }
            EventoRed::Sobre(peer, Sobre::PedirBlob(ids)) => {
                // Servir los blobs que tengamos en el store.
                if let Some(a) = &self.almacen {
                    let bloques = servir_blobs(&ids, |h| a.cargar_blob(h).ok().flatten());
                    if !bloques.is_empty() {
                        let _ = enlace.enviar(&peer, &Sobre::Blob(bloques));
                    }
                }
                Vec::new()
            }
            EventoRed::Sobre(_, Sobre::Blob(pares)) => {
                // Guardar los blobs entrantes que verifiquen contra su hash.
                if let Some(a) = &self.almacen {
                    for (h, bytes) in pares {
                        if blob_valido(&h, &bytes) {
                            let _ = a.guardar_blob(&bytes);
                        }
                    }
                }
                Vec::new()
            }
            EventoRed::Sobre(peer, sobre) => {
                // Nodos: reconciliar por anti-entropía.
                let (nuevos, respuestas) = self.fus.procesar(&mut self.conv, sobre, verificar_firma);
                for r in respuestas {
                    let _ = enlace.enviar(&peer, &r);
                }
                self.tras_nuevos_nodos(enlace, &peer, &nuevos);
                nuevos
            }
        }
    }

    /// Reacción a un lote de nodos recién integrados: persistirlos, anotar quién
    /// reciproca recibos, pedir blobs de adjuntos que falten, y —si el operador
    /// activó los recibos— acusar SIMÉTRICAMENTE los mensajes ajenos nuevos.
    fn tras_nuevos_nodos<T: Transporte>(&mut self, enlace: &T, peer: &PeerId, nuevos: &[Hash]) {
        if nuevos.is_empty() {
            return;
        }
        let yo = self.identidad.agora_id();
        let mut a_acusar: Vec<Hash> = Vec::new();

        for id in nuevos {
            let Some(nodo) = self.conv.obtener(id) else {
                continue;
            };
            self.persistir(&nodo.clone());
            match &nodo.contenido.carga {
                // Quien emite recibos nos habilita a reciprocar (presencia ayni).
                Carga::Recibo(_) => {
                    self.reciprocan.insert(*nodo.autor());
                }
                // Un mensaje ajeno: candidato a acuse (no nos acusamos a nosotros).
                _ if *nodo.autor() != yo => a_acusar.push(*id),
                _ => {}
            }
        }

        // Pedir los blobs de adjuntos que aún no tengamos.
        if let Some(a) = &self.almacen {
            let faltan = blobs_faltantes(&self.conv.adjuntos_referenciados(), |h| {
                a.tiene_blob(h).unwrap_or(false)
            });
            if !faltan.is_empty() {
                let _ = enlace.enviar(peer, &Sobre::PedirBlob(faltan));
            }
        }

        // Recibo simétrico: opt-in del operador; el otro lado decide el suyo.
        if self.recibos && !a_acusar.is_empty() {
            self.redactar_y_difundir(enlace, Carga::Recibo(Recibo { vistos: a_acusar }));
        }
    }

    /// Persiste un nodo si hay almacén (silencioso ante error: la copia en
    /// memoria sigue siendo la verdad de la sesión).
    fn persistir(&self, nodo: &MensajeNodo) {
        if let Some(a) = &self.almacen {
            let _ = a.guardar(nodo);
        }
    }

    /// Todas las identidades que dejaron huella en el grafo (autores de algún
    /// nodo), uno mismo incluido — el universo nombrable, sin directorio externo.
    pub fn conocidos(&self) -> BTreeSet<AgoraId> {
        let mut s: BTreeSet<AgoraId> = self.conv.nodos().map(|(_, n)| *n.autor()).collect();
        s.insert(self.identidad.agora_id());
        s
    }

    /// Resuelve un prefijo hex (el que muestra la UI) a una identidad CONOCIDA.
    /// Sólo se puede nombrar a quien ya dejó huella en la conversación.
    pub fn resolver(&self, prefijo: &str) -> Option<AgoraId> {
        let pref = prefijo.trim().to_lowercase();
        if pref.is_empty() {
            return None;
        }
        self.conocidos().into_iter().find(|id| hex_corto(id).starts_with(&pref))
    }

    // --- Vistas para la UI ----------------------------------------------------

    /// El texto a mostrar de un nodo: descifra si hace falta y hay canal, y
    /// traduce las cargas sociales de P7 a actos legibles.
    pub fn texto_visible(&self, nodo: &MensajeNodo) -> String {
        match &nodo.contenido.carga {
            Carga::Texto(t) => t.clone(),
            Carga::Cifrado(blob) => match &self.canal {
                Some(c) => c
                    .descifrar(blob)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_else(|_| "‹cifrado: no pude descifrar›".into()),
                None => "‹cifrado: sin canal›".into(),
            },
            Carga::Adjunto(a) => format!("📎 {} · {} · {} B", a.nombre, a.app, a.tamano),
            Carga::Membresia(m) => match m.accion {
                AccionMembresia::Alta => format!("admite a {}", hex_corto(&m.sujeto)),
                AccionMembresia::Baja => format!("expulsa a {}", hex_corto(&m.sujeto)),
            },
            Carga::Atestacion(at) if at.nivel == 0 => {
                format!("retira su fe en {}", hex_corto(&at.sujeto))
            }
            Carga::Atestacion(at) => format!("da fe de {} (nivel {})", hex_corto(&at.sujeto), at.nivel),
            Carga::Recibo(r) => format!("acusa recibo · {} msj", r.vistos.len()),
        }
    }
}

/// Adivina (app, clase MIME) de un adjunto por su extensión — guía a la UI sobre
/// cómo abrirlo en su app nativa del grafo.
fn clasificar(nombre: &str) -> (&'static str, &'static str) {
    let ext = nombre.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "md" | "markdown" => ("pluma", "text/markdown"),
        "txt" => ("archivo", "text/plain"),
        "png" => ("archivo", "image/png"),
        "jpg" | "jpeg" => ("archivo", "image/jpeg"),
        "pdf" => ("archivo", "application/pdf"),
        _ => ("archivo", "application/octet-stream"),
    }
}

/// Los primeros 3 bytes de un id, en hex — etiqueta corta y estable.
pub fn hex_corto(bytes: &AgoraId) -> String {
    bytes[..3].iter().map(|b| format!("{b:02x}")).collect()
}
