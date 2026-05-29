// =============================================================================
//  ayni :: ayni-sync — transporte y sincronización del grafo de conversación
// -----------------------------------------------------------------------------
//  Mueve nodos firmados entre pares y fusiona sus grafos hasta converger. El
//  transporte es un TRAIT —intercambiable—; P1 entrega `EnlaceTcp` (LAN directo).
//
//  Protocolo de cable (`Sobre`) — anti-entropía sobre el DAG (P3):
//    * `Hola{x25519}` — saludo + clave pública de cifrado (P2).
//    * `Cabezas(Vec<Hash>)` — "mi frontera". Al conectar, cada par anuncia sus
//      cabezas; el otro pide sólo las que le faltan. Es el diff de Merkle del DAG:
//      SÓLO viaja lo que falta, no el grafo entero.
//    * `Pedir(Vec<Hash>)` / `Entrega(Vec<MensajeNodo>)` — el receptor pide ids
//      ausentes; el emisor entrega esos nodos. Al recibirlos, sus padres ausentes
//      se piden a su vez: la reconciliación CAMINA el DAG hacia atrás, trayendo
//      sólo los eslabones que faltan, en O(profundidad) idas y vueltas.
//    * `Nodo(MensajeNodo)` — en caliente, cada mensaje nuevo se difunde suelto;
//      si al receptor le falta un padre, lo pide y se autorrepara.
//
//  El protocolo es transport-agnóstico: cuando exista `EnlaceMinga` (libp2p P2P
//  + DHT + NAT traversal), esta misma anti-entropía viaja sobre él sin cambios.
// =============================================================================

mod fusion;
mod tcp;

pub use fusion::Fusionador;
pub use tcp::EnlaceTcp;

pub use ayni_core::{Conversacion, Hash, MensajeNodo};

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
    /// Anti-entropía: las cabezas (frontera) del emisor. El receptor pide las
    /// que le falten. Es el diff de Merkle del DAG — sólo viaja lo que falta.
    Cabezas(Vec<Hash>),
    /// El receptor pide al emisor estos nodos (por id), que no tiene.
    Pedir(Vec<Hash>),
    /// El emisor entrega los nodos pedidos. Sus padres ausentes se piden a su vez.
    Entrega(Vec<MensajeNodo>),
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
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// El estado de un peer: su grafo + su fusionador.
    type Estado = Arc<Mutex<(Conversacion, Fusionador)>>;

    fn estado_vacio() -> Estado {
        Arc::new(Mutex::new((Conversacion::nueva(), Fusionador::nuevo())))
    }

    /// Lanza el "pump" de red de un peer —exactamente como la app real—: al
    /// conectar anuncia sus cabezas; ante cada sobre, corre la anti-entropía y
    /// devuelve al mismo peer los sobres que pida la reconciliación.
    fn lanzar_pump(enlace: Arc<EnlaceTcp>, rx: Receiver<EventoRed>, estado: Estado) {
        std::thread::spawn(move || {
            for ev in rx {
                match ev {
                    EventoRed::Conectado(peer) => {
                        let cabezas = estado.lock().unwrap().0.cabezas();
                        let _ = enlace.enviar(&peer, &Sobre::Cabezas(cabezas));
                    }
                    EventoRed::Desconectado(_) => {}
                    EventoRed::Sobre(peer, sobre) => {
                        let respuestas = {
                            let mut g = estado.lock().unwrap();
                            let (conv, fus) = &mut *g;
                            fus.procesar(conv, sobre, verificar_firma).1
                        };
                        for r in respuestas {
                            let _ = enlace.enviar(&peer, &r);
                        }
                    }
                }
            }
        });
    }

    /// Espera (con plazo) a que el grafo de un peer alcance `objetivo` nodos.
    fn esperar_len(estado: &Estado, objetivo: usize) {
        let limite = Instant::now() + Duration::from_secs(3);
        loop {
            if estado.lock().unwrap().0.len() >= objetivo {
                return;
            }
            if Instant::now() > limite {
                panic!(
                    "timeout: grafo en {} de {} nodos",
                    estado.lock().unwrap().0.len(),
                    objetivo
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Redacta+firma+inserta un texto en el estado de un peer y lo difunde.
    fn publicar(estado: &Estado, enlace: &EnlaceTcp, ident: &Identidad, texto: &str, ts: u64) {
        let nodo = {
            let mut g = estado.lock().unwrap();
            let nodo = g
                .0
                .redactar(ident.agora_id(), Carga::Texto(texto.into()), ts, |id| ident.firmar(id));
            g.0.agregar(nodo.clone()).unwrap();
            nodo
        };
        enlace.difundir(&Sobre::Nodo(nodo)).unwrap();
    }

    #[test]
    fn dos_clientes_convergen_por_anti_entropia() {
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let beto = Identidad::desde_semilla([2u8; 32], "Beto");

        let est_a = estado_vacio();
        let est_b = estado_vacio();

        // Alicia escribe DOS mensajes ANTES de que Beto exista —probará que la
        // anti-entropía al conectar pone a Beto al día sin volcar nada de más—.
        let (enlace_a, rx_a) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();
        let enlace_a = Arc::new(enlace_a);
        publicar(&est_a, &enlace_a, &alicia, "hola Beto", 1);
        publicar(&est_a, &enlace_a, &alicia, "¿andás?", 2);

        let (enlace_b, rx_b) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();
        let enlace_b = Arc::new(enlace_b);

        lanzar_pump(enlace_a.clone(), rx_a, est_a.clone());
        lanzar_pump(enlace_b.clone(), rx_b, est_b.clone());

        // Beto se conecta; la anti-entropía corre sola por los pumps.
        enlace_b.conectar(&enlace_a.direccion_local().to_string()).unwrap();
        esperar_len(&est_b, 2);
        assert!(est_b.lock().unwrap().0.verificar_firmas(verificar_firma).is_ok());

        // Beto responde EN CALIENTE; Alicia converge a 3.
        publicar(&est_b, &enlace_b, &beto, "acá ando", 3);
        esperar_len(&est_a, 3);

        // Convergencia total: mismo orden topológico en ambos, sin servidor.
        let orden_a = est_a.lock().unwrap().0.orden_topologico();
        let orden_b = est_b.lock().unwrap().0.orden_topologico();
        assert_eq!(orden_a, orden_b, "los dos pares convergen al mismo hilo");
    }

    #[test]
    fn anti_entropia_camina_el_dag_hacia_atras() {
        // Beto se conecta cuando Alicia ya tiene una CADENA de 5 mensajes. La
        // reconciliación debe traer toda la cadena pidiendo padres hacia atrás.
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let est_a = estado_vacio();
        let est_b = estado_vacio();

        let (enlace_a, rx_a) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();
        let enlace_a = Arc::new(enlace_a);
        for i in 0..5 {
            publicar(&est_a, &enlace_a, &alicia, &format!("msg {i}"), i);
        }
        let (enlace_b, rx_b) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();
        let enlace_b = Arc::new(enlace_b);

        lanzar_pump(enlace_a.clone(), rx_a, est_a.clone());
        lanzar_pump(enlace_b.clone(), rx_b, est_b.clone());

        enlace_b.conectar(&enlace_a.direccion_local().to_string()).unwrap();
        esperar_len(&est_b, 5);
        assert_eq!(est_b.lock().unwrap().0.len(), 5, "cadena completa reconciliada");
        assert_eq!(est_b.lock().unwrap().1.pendientes(), 0, "sin pendientes huérfanos");
    }

    #[test]
    fn lazo_e2ee_sobre_tcp_el_transporte_no_ve_el_claro() {
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let beto = Identidad::desde_semilla([2u8; 32], "Beto");
        let canal_a = alicia.canal_con(&beto.clave_publica_x25519());
        let canal_b = beto.canal_con(&alicia.clave_publica_x25519());

        let est_a = estado_vacio();
        let est_b = estado_vacio();
        let (enlace_a, rx_a) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();
        let enlace_a = Arc::new(enlace_a);
        let (enlace_b, rx_b) = EnlaceTcp::escuchar("127.0.0.1:0").unwrap();
        let enlace_b = Arc::new(enlace_b);
        lanzar_pump(enlace_a.clone(), rx_a, est_a.clone());
        lanzar_pump(enlace_b.clone(), rx_b, est_b.clone());
        enlace_b.conectar(&enlace_a.direccion_local().to_string()).unwrap();

        // Alicia CIFRA y difunde un nodo firmado.
        let secreto = "esto sólo lo lee Beto";
        let cifrado = canal_a.cifrar(secreto.as_bytes());
        let nodo = {
            let mut g = est_a.lock().unwrap();
            let n = g.0.redactar(alicia.agora_id(), Carga::Cifrado(cifrado), 1, |id| alicia.firmar(id));
            g.0.agregar(n.clone()).unwrap();
            n
        };
        // lo que viaja por el cable NO contiene el claro.
        let bytes_cable = postcard::to_allocvec(&Sobre::Nodo(nodo.clone())).unwrap();
        assert!(
            !bytes_cable.windows(secreto.len()).any(|w| w == secreto.as_bytes()),
            "el claro NO debe aparecer en los bytes del cable"
        );
        enlace_a.difundir(&Sobre::Nodo(nodo)).unwrap();

        esperar_len(&est_b, 1);
        let recibido = est_b.lock().unwrap().0.instantanea().pop().unwrap();
        assert!(recibido.verificar(verificar_firma), "autoría de Alicia verificable");
        assert_eq!(recibido.contenido.carga.texto(), None, "el nodo no trae claro");
        let claro = canal_b.descifrar(recibido.contenido.carga.cifrado().unwrap()).unwrap();
        assert_eq!(claro, secreto.as_bytes(), "Beto recupera el claro E2EE");
    }

    #[test]
    fn un_nodo_con_firma_invalida_se_rechaza_en_la_fusion() {
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");
        let impostor = Identidad::desde_semilla([9u8; 32], "Impostor");
        let mut conv = Conversacion::nueva();
        let mut fus = Fusionador::nuevo();
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
