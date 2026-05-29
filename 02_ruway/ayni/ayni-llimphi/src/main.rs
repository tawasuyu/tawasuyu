// =============================================================================
//  ayni :: ayni-llimphi — chat soberano, cara gráfica
// -----------------------------------------------------------------------------
//  El mismo lazo vivo que el CLI, ahora pintado por Llimphi. Estado = una
//  Conversacion (DAG firmado); transporte = EnlaceTcp; firma = Identidad agora.
//  Un hilo de red drena los EventoRed y los reinyecta al bucle Elm con
//  Handle::dispatch — así el `update` ve los mensajes entrantes como un Msg más.
//
//  Configuración por entorno (demo):
//    AYNI_NOMBRE     nombre → identidad Ed25519 determinista (BLAKE3 del nombre)
//    AYNI_ESCUCHAR   dirección de escucha (default 127.0.0.1:7700)
//    AYNI_CONECTAR   peer al que conectarse al arrancar (opcional)
// =============================================================================

use std::env;
use std::sync::Arc;

use ayni_core::{AgoraId, Carga, Conversacion};
use ayni_crypto::{verificar_firma, CanalSeguro, Identidad};
use ayni_sync::{EnlaceTcp, EventoRed, Fusionador, Sobre, Transporte};

use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Dimension, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent, Rect};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

/// Cuántos mensajes recientes pinta la lista (MVP: sin scroll, cola visible).
const VISIBLES: usize = 18;

#[derive(Clone)]
enum Msg {
    /// Una tecla que no es Enter (texto, backspace…).
    Tecla(KeyEvent),
    /// Enviar el contenido del input.
    Enviar,
    /// Un evento del transporte (peer conectado/desconectado, sobre entrante).
    Red(EventoRed),
}

struct Modelo {
    conv: Conversacion,
    fus: Fusionador,
    entrada: String,
    identidad: Identidad,
    enlace: Arc<EnlaceTcp>,
    nombre: String,
    /// Canal E2EE con el peer, tras intercambiar claves X25519 (P2).
    canal: Option<CanalSeguro>,
    /// ¿Cifrar los mensajes salientes? (env AYNI_CIFRAR).
    cifrar: bool,
}

struct Ayni;

impl App for Ayni {
    type Model = Modelo;
    type Msg = Msg;

    fn title() -> &'static str {
        "ayni · chat soberano"
    }

    fn initial_size() -> (u32, u32) {
        (560, 720)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let nombre = env::var("AYNI_NOMBRE").unwrap_or_else(|_| "yo".into());
        let escuchar = env::var("AYNI_ESCUCHAR").unwrap_or_else(|_| "127.0.0.1:7700".into());

        // Identidad determinista desde el nombre (demo); en producción saldría
        // del keystore cifrado de agora.
        let seed = *blake3::hash(nombre.as_bytes()).as_bytes();
        let identidad = Identidad::desde_semilla(seed, nombre.clone());

        let (enlace, rx) = EnlaceTcp::escuchar(&escuchar)
            .unwrap_or_else(|e| panic!("ayni: no pude escuchar en {escuchar}: {e}"));
        let enlace = Arc::new(enlace);

        if let Ok(peer) = env::var("AYNI_CONECTAR") {
            let _ = enlace.conectar(&peer);
        }

        // Hilo de red: cada EventoRed se reinyecta al bucle Elm.
        let h = handle.clone();
        std::thread::spawn(move || {
            for evento in rx {
                h.dispatch(Msg::Red(evento));
            }
        });

        Modelo {
            conv: Conversacion::nueva(),
            fus: Fusionador::nuevo(),
            entrada: String::new(),
            identidad,
            enlace,
            nombre,
            canal: None,
            cifrar: env::var("AYNI_CIFRAR").is_ok(),
        }
    }

    fn on_key(_model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        match &e.key {
            Key::Named(NamedKey::Enter) => Some(Msg::Enviar),
            _ => Some(Msg::Tecla(e.clone())),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tecla(e) => match &e.key {
                Key::Named(NamedKey::Backspace) => {
                    model.entrada.pop();
                }
                _ => {
                    if let Some(t) = &e.text {
                        for c in t.chars() {
                            if !c.is_control() {
                                model.entrada.push(c);
                            }
                        }
                    }
                }
            },
            Msg::Enviar => {
                let texto = model.entrada.trim().to_string();
                if !texto.is_empty() {
                    let autor = model.identidad.agora_id();
                    // cifrar si está activo y ya hay canal; si no, texto plano.
                    let carga = match (model.cifrar, &model.canal) {
                        (true, Some(canal)) => Carga::Cifrado(canal.cifrar(texto.as_bytes())),
                        _ => Carga::Texto(texto),
                    };
                    let nodo = model.conv.redactar(autor, carga, 0, |id| {
                        model.identidad.firmar(id)
                    });
                    model.conv.agregar(nodo.clone()).ok();
                    let _ = model.enlace.difundir(&Sobre::Nodo(nodo));
                    model.entrada.clear();
                }
            }
            Msg::Red(evento) => match evento {
                EventoRed::Conectado(peer) => {
                    // saludar (clave X25519) + anunciar cabezas (anti-entropía).
                    let _ = model.enlace.enviar(
                        &peer,
                        &Sobre::Hola {
                            x25519: model.identidad.clave_publica_x25519(),
                        },
                    );
                    let _ = model
                        .enlace
                        .enviar(&peer, &Sobre::Cabezas(model.conv.cabezas()));
                }
                EventoRed::Desconectado(_) => {}
                EventoRed::Sobre(_, Sobre::Hola { x25519 }) => {
                    model.canal = Some(model.identidad.canal_con(&x25519));
                }
                EventoRed::Sobre(peer, sobre) => {
                    // anti-entropía: procesar y devolver al peer lo que pida.
                    let respuestas = {
                        let Modelo { conv, fus, .. } = &mut model;
                        fus.procesar(conv, sobre, verificar_firma).1
                    };
                    for r in respuestas {
                        let _ = model.enlace.enviar(&peer, &r);
                    }
                }
            },
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let fondo = Color::from_rgba8(18, 21, 28, 255);
        let fondo_barra = Color::from_rgba8(28, 33, 44, 255);
        let texto_claro = Color::from_rgba8(222, 230, 240, 255);
        let texto_tenue = Color::from_rgba8(120, 135, 155, 255);
        let mio = Color::from_rgba8(120, 220, 170, 255);
        let ajeno = Color::from_rgba8(150, 185, 235, 255);
        let yo = model.identidad.agora_id();

        // --- barra superior ---
        let barra = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(44.0_f32),
            },
            align_items: Some(AlignItems::Center),
            padding: lados(16.0, 0.0),
            ..Default::default()
        })
        .fill(fondo_barra)
        .text_aligned(
            format!(
                "ayni · {} [{}] · {} peer(es){}",
                model.nombre,
                hex_corto(&yo),
                model.enlace.num_peers(),
                match (model.cifrar, model.canal.is_some()) {
                    (true, true) => " · 🔒 E2EE",
                    (true, false) => " · 🔓 esperando clave",
                    _ => "",
                }
            ),
            15.0,
            texto_claro,
            Alignment::Start,
        );

        // --- lista de mensajes (cola visible, sin scroll: MVP) ---
        let nodos = model.conv.instantanea();
        let desde = nodos.len().saturating_sub(VISIBLES);
        let mut filas: Vec<View<Msg>> = Vec::new();
        for nodo in &nodos[desde..] {
            let propio = *nodo.autor() == yo;
            let color = if propio { mio } else { ajeno };
            // descifrar para mostrar si la carga viene cifrada y hay canal.
            let texto = match &nodo.contenido.carga {
                Carga::Texto(t) => t.clone(),
                Carga::Cifrado(blob) => match &model.canal {
                    Some(c) => c
                        .descifrar(blob)
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_else(|_| "‹cifrado›".into()),
                    None => "‹cifrado›".into(),
                },
            };
            let etiqueta = format!("[{}] {}", hex_corto(nodo.autor()), texto);
            filas.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: Dimension::auto(),
                    },
                    ..Default::default()
                })
                .text_aligned(etiqueta, 15.0, color, Alignment::Start),
            );
        }
        if filas.is_empty() {
            filas.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: Dimension::auto(),
                    },
                    ..Default::default()
                })
                .text_aligned(
                    "— sin mensajes todavía. Escribí abajo y Enter. —".to_string(),
                    14.0,
                    texto_tenue,
                    Alignment::Start,
                ),
            );
        }

        let lista = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            gap: Size {
                width: length(0.0_f32),
                height: length(8.0_f32),
            },
            padding: lados(16.0, 14.0),
            ..Default::default()
        })
        .fill(fondo)
        .clip(true)
        .children(filas);

        // --- caja de entrada + botón enviar ---
        let cursor = if model.entrada.is_empty() {
            "escribí un mensaje…".to_string()
        } else {
            format!("{}▏", model.entrada)
        };
        let color_entrada = if model.entrada.is_empty() {
            texto_tenue
        } else {
            texto_claro
        };
        let caja = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(40.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            padding: lados(12.0, 0.0),
            ..Default::default()
        })
        .fill(Color::from_rgba8(36, 42, 55, 255))
        .radius(8.0)
        .text_aligned(cursor, 15.0, color_entrada, Alignment::Start);

        let boton = View::new(Style {
            size: Size {
                width: length(96.0_f32),
                height: length(40.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(Color::from_rgba8(70, 200, 140, 255))
        .radius(8.0)
        .text("enviar", 15.0, Color::from_rgba8(12, 30, 22, 255))
        .on_click(Msg::Enviar);

        let fila_entrada = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(56.0_f32),
            },
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            padding: lados(16.0, 8.0),
            ..Default::default()
        })
        .fill(fondo_barra)
        .children(vec![caja, boton]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(fondo)
        .children(vec![barra, lista, fila_entrada])
    }
}

/// Padding lateral/vertical uniforme.
fn lados(horizontal: f32, vertical: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(horizontal),
        right: length(horizontal),
        top: length(vertical),
        bottom: length(vertical),
    }
}

/// Los primeros 3 bytes de un id, en hex.
fn hex_corto(bytes: &AgoraId) -> String {
    bytes[..3].iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    llimphi_ui::run::<Ayni>();
}
