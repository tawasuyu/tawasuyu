// =============================================================================
//  ayni :: ayni-sync — transporte y sincronización del grafo de conversación
// -----------------------------------------------------------------------------
//  Mueve nodos firmados entre pares y fusiona sus grafos hasta converger. El
//  transporte es un TRAIT —intercambiable—; P1 entrega `EnlaceTcp` (LAN directo).
//
//  Protocolo de cable (`Sobre`), deliberadamente mínimo para el MVP:
//    * `Grafo(Vec<MensajeNodo>)` — al conectar, cada par vuelca su conversación
//      entera. Como fusionar es idempotente, repetir nodos no cuesta corrección.
//    * `Nodo(MensajeNodo)` — en caliente, cada mensaje nuevo se difunde suelto.
//
//  El diff de Merkle (mandar SÓLO lo que falta, en vez del grafo entero) y el
//  store-and-forward sobre minga son P3 — aquí basta el volcado completo, que
//  para un hilo de chat es barato y obviamente correcto.
// =============================================================================

mod fusion;
mod tcp;

pub use fusion::Fusionador;
pub use tcp::EnlaceTcp;

pub use ayni_core::{Conversacion, MensajeNodo};

use serde::{Deserialize, Serialize};

/// El identificador opaco de un peer, tal como lo nombra el transporte (para
/// `EnlaceTcp`, su dirección `ip:puerto`). El llamador lo trata como una etiqueta
/// para dirigir un envío puntual ([`Transporte::enviar`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerId(pub String);

/// Lo que viaja por el cable entre dos pares de Ayni.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Sobre {
    /// Saludo de presentación al conectar: la clave pública X25519 del emisor,
    /// para que el otro pueda abrirle un canal E2EE 1:1 (P2). No lleva secreto
    /// alguno —una clave pública es, por definición, pública—; sólo evita un
    /// directorio externo para el intercambio de claves de cifrado.
    Hola { x25519: [u8; 32] },
    /// Volcado del grafo completo del emisor — su instantánea en orden
    /// topológico. Se manda al establecer la conexión para ponerse al día.
    Grafo(Vec<MensajeNodo>),
    /// Un nodo nuevo, difundido en caliente apenas su autor lo redacta. Su carga
    /// puede ser texto plano o `Carga::Cifrado` — al transporte le da igual.
    Nodo(MensajeNodo),
}

/// Lo que el transporte entrega a la aplicación por el canal de eventos.
#[derive(Debug, Clone)]
pub enum EventoRed {
    /// Un peer se conectó (entrante o saliente). El llamador reacciona
    /// volcándole su grafo con [`Transporte::enviar`] + [`Sobre::Grafo`].
    Conectado(PeerId),
    /// Un peer se desconectó (EOF, error, o frame corrupto).
    Desconectado(PeerId),
    /// Llegó un sobre de un peer.
    Sobre(PeerId, Sobre),
}

/// El transporte: el lado de ENVÍO de un enlace de Ayni. La recepción llega por
/// el `Receiver<EventoRed>` que el constructor del transporte concreto entrega.
/// Un transporte futuro (minga P2P, puente chasqui) implementa este mismo trait
/// y devuelve el mismo `EventoRed`, de modo que la app no cambia al cambiarlo.
pub trait Transporte {
    /// Difunde un sobre a TODOS los peers conectados. Los peers cuya escritura
    /// falla se purgan (su conexión murió).
    fn difundir(&self, sobre: &Sobre) -> Result<(), ErrorSync>;

    /// Envía un sobre a UN peer puntual. Falla con [`ErrorSync::PeerDesconocido`]
    /// si ese peer ya no está conectado.
    fn enviar(&self, peer: &PeerId, sobre: &Sobre) -> Result<(), ErrorSync>;
}

/// Falla del transporte o del framing.
#[derive(Debug, thiserror::Error)]
pub enum ErrorSync {
    /// Error de E/S de sockets.
    #[error("ayni-sync :: E/S de red: {0}")]
    Io(#[from] std::io::Error),
    /// Un frame no decodifica, está vacío, o excede el techo de tamaño.
    #[error("ayni-sync :: frame inválido (vacío, corrupto o sobredimensionado)")]
    FrameInvalido,
    /// Se pidió enviar a un peer que no está conectado.
    #[error("ayni-sync :: peer desconocido o desconectado")]
    PeerDesconocido,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{Carga, Conversacion};
    use ayni_crypto::{verificar_firma, Identidad};
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    /// Drena eventos hasta ver un `Conectado` (o expira). Asegura que el peer
    /// está registrado antes de difundir —el registro del lado aceptante ocurre
    /// en un hilo, así que esperamos su señal en vez de adivinar un sleep—.
    fn esperar_conectado(rx: &Receiver<EventoRed>) {
        let plazo = Duration::from_secs(2);
        loop {
            match rx.recv_timeout(plazo).expect("timeout esperando Conectado") {
                EventoRed::Conectado(_) => return,
                _ => continue,
            }
        }
    }

    /// Drena eventos durante un rato, aplicando cada sobre al grafo via el
    /// fusionador, hasta que el grafo alcanza `objetivo` nodos (o expira).
    fn sincronizar_hasta(
        rx: &Receiver<EventoRed>,
        conv: &mut Conversacion,
        fus: &mut Fusionador,
        objetivo: usize,
    ) {
        let plazo = Duration::from_secs(2);
        while conv.len() < objetivo {
            match rx.recv_timeout(plazo) {
                Ok(EventoRed::Sobre(_, Sobre::Nodo(n))) => {
                    fus.aplicar_nodo(conv, n, verificar_firma);
                }
                Ok(EventoRed::Sobre(_, Sobre::Grafo(nodos))) => {
                    fus.aplicar_lote(conv, nodos, verificar_firma);
                }
                Ok(_) => continue,
                Err(_) => panic!("timeout: grafo en {} de {} nodos", conv.len(), objetivo),
            }
        }
    }

    #[test]
    fn dos_clientes_convergen_por_tcp_loopback() {
        // --- identidades ---
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let beto = Identidad::desde_semilla([2u8; 32], "Beto");

        // --- grafos locales y enlaces ---
        let mut conv_a = Conversacion::nueva();
        let mut conv_b = Conversacion::nueva();
        let mut fus_a = Fusionador::nuevo();
        let mut fus_b = Fusionador::nuevo();

        let (enlace_a, rx_a) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();
        let (enlace_b, rx_b) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();

        // Alicia escribe DOS mensajes ANTES de que Beto se conecte: probará que
        // el volcado de grafo al conectar pone a Beto al día.
        let n1 = conv_a.redactar(alicia.agora_id(), Carga::Texto("hola Beto".into()), 1, |id| {
            alicia.firmar(id)
        });
        conv_a.agregar(n1).unwrap();
        let n2 = conv_a.redactar(alicia.agora_id(), Carga::Texto("¿andás?".into()), 2, |id| {
            alicia.firmar(id)
        });
        conv_a.agregar(n2).unwrap();
        assert_eq!(conv_a.len(), 2);

        // Beto se conecta a Alicia.
        let addr_a = enlace_a.direccion_local().to_string();
        enlace_b.conectar(&addr_a).unwrap();

        // Ambos lados ven Conectado; al verlo, vuelcan su grafo al otro.
        esperar_conectado(&rx_a); // Alicia ve a Beto
        esperar_conectado(&rx_b); // Beto ve a Alicia
        enlace_a.difundir(&Sobre::Grafo(conv_a.instantanea())).unwrap();
        enlace_b.difundir(&Sobre::Grafo(conv_b.instantanea())).unwrap();

        // Beto se pone al día con los 2 mensajes de Alicia.
        sincronizar_hasta(&rx_b, &mut conv_b, &mut fus_b, 2);
        assert_eq!(conv_b.len(), 2, "Beto recibió el grafo de Alicia al conectar");
        assert!(conv_b.verificar_firmas(verificar_firma).is_ok(), "firmas válidas");

        // Ahora Beto responde EN CALIENTE; se difunde suelto.
        let r = conv_b.redactar(beto.agora_id(), Carga::Texto("acá ando".into()), 3, |id| {
            beto.firmar(id)
        });
        conv_b.agregar(r).unwrap();
        enlace_b.difundir(&Sobre::Nodo(conv_b.instantanea().pop().unwrap())).unwrap();

        // Alicia recibe la respuesta de Beto: converge a 3 nodos.
        sincronizar_hasta(&rx_a, &mut conv_a, &mut fus_a, 3);
        assert_eq!(conv_a.len(), 3, "Alicia recibió la respuesta en caliente de Beto");

        // Convergencia: ambos calculan EL MISMO orden topológico (sin servidor).
        assert_eq!(conv_a.len(), conv_b.len());
        assert_eq!(
            conv_a.orden_topologico(),
            conv_b.orden_topologico(),
            "los dos pares convergen al mismo hilo"
        );
    }

    #[test]
    fn lazo_e2ee_sobre_tcp_el_transporte_no_ve_el_claro() {
        use ayni_core::Carga;

        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let beto = Identidad::desde_semilla([2u8; 32], "Beto");

        // Cada uno deriva el canal 1:1 desde la clave pública X25519 del otro.
        let canal_a = alicia.canal_con(&beto.clave_publica_x25519());
        let canal_b = beto.canal_con(&alicia.clave_publica_x25519());

        let mut conv_a = Conversacion::nueva();
        let mut conv_b = Conversacion::nueva();
        let mut fus_b = Fusionador::nuevo();

        let (enlace_a, _rx_a) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();
        let (enlace_b, rx_b) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();
        enlace_b.conectar(&enlace_a.direccion_local().to_string()).unwrap();
        esperar_conectado(&rx_b);

        // Alicia CIFRA el claro, lo mete en un nodo firmado, y lo difunde.
        let secreto = "esto sólo lo lee Beto";
        let cifrado = canal_a.cifrar(secreto.as_bytes());
        let nodo = conv_a.redactar(alicia.agora_id(), Carga::Cifrado(cifrado), 1, |id| {
            alicia.firmar(id)
        });
        conv_a.agregar(nodo).unwrap();
        let en_cable = conv_a.instantanea().pop().unwrap();

        // Lo que viaja por el cable NO contiene el claro.
        let bytes_cable = postcard::to_allocvec(&Sobre::Nodo(en_cable.clone())).unwrap();
        let ventana: Vec<u8> = bytes_cable.clone();
        assert!(
            !contiene_subcadena(&ventana, secreto.as_bytes()),
            "el claro NO debe aparecer en los bytes del cable"
        );

        enlace_a.difundir(&Sobre::Nodo(en_cable)).unwrap();
        sincronizar_hasta(&rx_b, &mut conv_b, &mut fus_b, 1);

        // Beto verifica autoría (pública) y descifra (sólo él puede).
        let recibido = conv_b.instantanea().pop().unwrap();
        assert!(recibido.verificar(verificar_firma), "autoría de Alicia, verificable");
        assert_eq!(recibido.contenido.carga.texto(), None, "el nodo no trae claro");
        let claro = canal_b
            .descifrar(recibido.contenido.carga.cifrado().unwrap())
            .unwrap();
        assert_eq!(claro, secreto.as_bytes(), "Beto recupera el claro E2EE");
    }

    /// ¿Aparece `aguja` como subcadena contigua de `pajar`?
    fn contiene_subcadena(pajar: &[u8], aguja: &[u8]) -> bool {
        pajar.windows(aguja.len()).any(|w| w == aguja)
    }

    #[test]
    fn un_nodo_con_firma_invalida_se_rechaza_en_la_fusion() {
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let impostor = Identidad::desde_semilla([9u8; 32], "Impostor");
        let mut conv = Conversacion::nueva();
        let mut fus = Fusionador::nuevo();

        // nodo atribuido a Alicia pero firmado por el impostor:
        let contenido = ayni_core::Contenido::nuevo(
            alicia.agora_id(),
            Vec::new(),
            Carga::Texto("falso".into()),
            1,
        );
        let falso = MensajeNodo::sellar(contenido, |id| impostor.firmar(id));

        let anadidos = fus.aplicar_nodo(&mut conv, falso, verificar_firma);
        assert!(anadidos.is_empty(), "el nodo de firma inválida no entra");
        assert_eq!(conv.len(), 0);
    }
}
