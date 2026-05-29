// =============================================================================
//  ayni :: ayni-llimphi — la cara gráfica completa del chat soberano
// -----------------------------------------------------------------------------
//  Frontend Llimphi (bucle Elm) DELGADO sobre `ayni-app::Nucleo` —ahí vive toda
//  la lógica: transporte (TCP o minga), persistencia local-first, cifrado 1:1,
//  adjuntos con su blob, y la confianza de P7 (membresía, atestaciones, recibos
//  simétricos)—. La UI sólo pinta el núcleo y captura la intención del humano.
//
//  Dos columnas, controles co-locados (sin toolbars brutas):
//    · GENTE   — miembros (clic = seleccionar), otros vistos, acciones sobre el
//                seleccionado (admitir/expulsar/atestar) y el grafo de confianza.
//    · CHARLA  — el hilo (con scroll y recibos "✓N") + compose con toggles de
//                cifrado/recibos, adjuntar y enviar. La barra `/` acepta comandos.
//
//  Configuración por entorno:
//    AYNI_NOMBRE      nombre → identidad Ed25519 determinista (BLAKE3 del nombre)
//    AYNI_TRANSPORTE  tcp (default) | minga
//    AYNI_ESCUCHAR    bind (default según transporte)
//    AYNI_CONECTAR    peer al que conectarse al arrancar (opcional)
//    AYNI_DATA        ruta del store sled (default ./ayni-<nombre>.db)
//    AYNI_CIFRAR      si está, arranca con el cifrado activo
//    AYNI_RECIBOS     si está, arranca emitiendo recibos (simétrico: actívenlo ambos)
// =============================================================================

use std::env;
use std::sync::Arc;

use ayni_app::{hex_corto, AgoraId, Enlace, EventoRed, Identidad, Nucleo, Tipo};

use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Dimension, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent, Rect};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};

/// Cuántos mensajes pinta la ventana visible del hilo (con scroll por rueda).
const VISIBLES: usize = 16;

#[derive(Clone)]
enum Msg {
    Tecla(KeyEvent),
    Enviar,
    Red(EventoRed),
    /// Selecciona una identidad como blanco de las acciones de GENTE.
    Seleccionar(AgoraId),
    Admitir,
    Expulsar,
    /// Atestiguar al seleccionado (nivel 5 por defecto desde el botón).
    Atestar,
    AcusarRecibo,
    ToggleCifrar,
    ToggleRecibos,
    /// Desplazar el hilo: +N hacia mensajes más viejos, -N hacia los recientes.
    Scroll(i32),
}

struct Modelo {
    nucleo: Nucleo,
    enlace: Arc<Enlace>,
    entrada: String,
    nombre: String,
    transporte: &'static str,
    /// El blanco de las acciones de membresía/confianza.
    seleccionado: Option<AgoraId>,
    /// Mensajes desplazados desde el fondo (0 = pegado a lo más nuevo).
    scroll: usize,
    /// Línea de estado: resultado del último comando/adjuntar.
    aviso: String,
}

struct Ayni;

impl App for Ayni {
    type Model = Modelo;
    type Msg = Msg;

    fn title() -> &'static str {
        "ayni · chat soberano"
    }

    fn initial_size() -> (u32, u32) {
        (900, 760)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let nombre = env::var("AYNI_NOMBRE").unwrap_or_else(|_| "yo".into());
        let tipo = Tipo::desde_nombre(&env::var("AYNI_TRANSPORTE").unwrap_or_default());
        let bind = env::var("AYNI_ESCUCHAR").unwrap_or_else(|_| tipo.bind_por_defecto().into());

        let seed = *blake3::hash(nombre.as_bytes()).as_bytes();
        let identidad = Identidad::desde_semilla(seed, nombre.clone());

        let ruta = env::var("AYNI_DATA").unwrap_or_else(|_| format!("./ayni-{nombre}.db"));
        let nucleo = Nucleo::nuevo(
            identidad,
            Some(std::path::Path::new(&ruta)),
            env::var("AYNI_CIFRAR").is_ok(),
            env::var("AYNI_RECIBOS").is_ok(),
        );

        let (enlace, rx) = Enlace::abrir(tipo, &bind)
            .unwrap_or_else(|e| panic!("ayni: no pude abrir el transporte en {bind}: {e}"));
        let transporte = enlace.etiqueta();
        let dir = enlace.direccion_local();
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
            nucleo,
            enlace,
            entrada: String::new(),
            nombre,
            transporte,
            seleccionado: None,
            scroll: 0,
            aviso: format!("escuchando en {dir} · {transporte}"),
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

    fn on_wheel(
        _model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Self::Msg> {
        if delta.y.abs() < f32::EPSILON {
            return None;
        }
        // y>0 ⇒ rueda hacia abajo ⇒ ver más viejos (subir el offset).
        Some(Msg::Scroll(if delta.y > 0.0 { 3 } else { -3 }))
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _handle: &Handle<Self::Msg>) -> Self::Model {
        let enlace = model.enlace.clone();
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
                model.entrada.clear();
                if texto.is_empty() {
                    // nada
                } else if let Some(cmd) = texto.strip_prefix('/') {
                    model.aviso = ejecutar_comando(&mut model.nucleo, enlace.as_ref(), cmd);
                } else {
                    model.nucleo.enviar_texto(enlace.as_ref(), &texto);
                    model.scroll = 0;
                }
            }
            Msg::Seleccionar(id) => {
                model.seleccionado = Some(id);
                model.aviso = format!("seleccionado {}", hex_corto(&id));
            }
            Msg::Admitir => {
                if let Some(s) = model.seleccionado {
                    model.nucleo.admitir(enlace.as_ref(), s);
                    model.aviso = format!("admitiste a {}", hex_corto(&s));
                }
            }
            Msg::Expulsar => {
                if let Some(s) = model.seleccionado {
                    model.nucleo.expulsar(enlace.as_ref(), s);
                    model.aviso = format!("expulsaste a {}", hex_corto(&s));
                }
            }
            Msg::Atestar => {
                if let Some(s) = model.seleccionado {
                    model.nucleo.atestar(enlace.as_ref(), s, 5);
                    model.aviso = format!("das fe de {} (nivel 5)", hex_corto(&s));
                }
            }
            Msg::AcusarRecibo => {
                model.nucleo.acusar_cabezas(enlace.as_ref());
                model.aviso = "acuse de recibo enviado".into();
            }
            Msg::ToggleCifrar => {
                model.nucleo.cifrar = !model.nucleo.cifrar;
                model.aviso = format!("cifrado {}", si_no(model.nucleo.cifrar));
            }
            Msg::ToggleRecibos => {
                model.nucleo.recibos = !model.nucleo.recibos;
                model.aviso = format!("recibos {}", si_no(model.nucleo.recibos));
            }
            Msg::Scroll(d) => {
                let max = model.nucleo.conv.len().saturating_sub(VISIBLES);
                let nuevo = model.scroll as i32 + d;
                model.scroll = nuevo.clamp(0, max as i32) as usize;
            }
            Msg::Red(evento) => {
                model.nucleo.al_evento(enlace.as_ref(), evento);
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let fondo = Color::from_rgba8(18, 21, 28, 255);

        let barra = barra_superior(model);
        let cuerpo = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![panel_gente(model), columna_charla(model)]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(fondo)
        .children(vec![barra, cuerpo, barra_estado(model)])
    }
}

// === Paleta ==================================================================
const FONDO: (u8, u8, u8) = (18, 21, 28);
const BARRA: (u8, u8, u8) = (28, 33, 44);
const PANEL: (u8, u8, u8) = (23, 27, 36);
const CLARO: (u8, u8, u8) = (222, 230, 240);
const TENUE: (u8, u8, u8) = (120, 135, 155);
const MIO: (u8, u8, u8) = (120, 220, 170);
const AJENO: (u8, u8, u8) = (150, 185, 235);
const SOCIAL: (u8, u8, u8) = (210, 180, 120);
const SEL: (u8, u8, u8) = (60, 90, 80);
const VERDE: (u8, u8, u8) = (70, 200, 140);
const ACENTO: (u8, u8, u8) = (90, 130, 200);

fn c(rgb: (u8, u8, u8)) -> Color {
    Color::from_rgba8(rgb.0, rgb.1, rgb.2, 255)
}

// === Barra superior ==========================================================
fn barra_superior(model: &Modelo) -> View<Msg> {
    let yo = model.nucleo.yo();
    let estado_cifra = match (model.nucleo.cifrar, model.nucleo.tiene_canal()) {
        (true, true) => "🔒 E2EE",
        (true, false) => "🔓 esperando clave",
        _ => "claro",
    };
    let titulo = View::new(estilo_flex_fila(1.0))
        .text_aligned(
            format!(
                "ayni · {} [{}] · {} · {} peer(s) · {}",
                model.nombre,
                hex_corto(&yo),
                model.transporte,
                model.enlace.num_peers(),
                estado_cifra,
            ),
            15.0,
            c(CLARO),
            Alignment::Start,
        );

    let toggles = View::new(Style {
        flex_direction: FlexDirection::Row,
        gap: gap_h(8.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        chip(
            format!("cifrar: {}", si_no(model.nucleo.cifrar)),
            if model.nucleo.cifrar { VERDE } else { TENUE },
            Msg::ToggleCifrar,
        ),
        chip(
            format!("recibos: {}", si_no(model.nucleo.recibos)),
            if model.nucleo.recibos { VERDE } else { TENUE },
            Msg::ToggleRecibos,
        ),
    ]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(46.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: lados(16.0, 0.0),
        ..Default::default()
    })
    .fill(c(BARRA))
    .children(vec![titulo, toggles])
}

// === Panel GENTE (membresía + confianza) =====================================
fn panel_gente(model: &Modelo) -> View<Msg> {
    let yo = model.nucleo.yo();
    let memb = model.nucleo.conv.membresia();
    let confianza = model.nucleo.conv.confianza_desde(&yo);
    let conocidos = model.nucleo.conocidos();

    let mut hijos: Vec<View<Msg>> = Vec::new();
    hijos.push(rotulo("GENTE — miembros"));

    for id in &memb.miembros {
        hijos.push(fila_persona(model, id, &memb, &yo));
    }

    // Otros vistos que aún no son miembros.
    let otros: Vec<AgoraId> = conocidos
        .iter()
        .filter(|id| !memb.contiene(id))
        .copied()
        .collect();
    if !otros.is_empty() {
        hijos.push(rotulo("otros vistos"));
        for id in &otros {
            hijos.push(fila_persona(model, id, &memb, &yo));
        }
    }

    // Acciones sobre el seleccionado.
    hijos.push(rotulo("acciones"));
    let etiqueta_sel = match model.seleccionado {
        Some(s) => format!("blanco: {}", hex_corto(&s)),
        None => "elegí a alguien arriba".into(),
    };
    hijos.push(
        View::new(estilo_fila_auto()).text_aligned(etiqueta_sel, 13.0, c(TENUE), Alignment::Start),
    );
    hijos.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            gap: gap_h(6.0),
            margin: Rect {
                left: length(0.0),
                right: length(0.0),
                top: length(4.0),
                bottom: length(4.0),
            },
            ..Default::default()
        })
        .children(vec![
            boton("admitir", ACENTO, Msg::Admitir),
            boton("atestar", SOCIAL, Msg::Atestar),
        ]),
    );
    hijos.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            gap: gap_h(6.0),
            ..Default::default()
        })
        .children(vec![
            boton("expulsar", (150, 80, 80), Msg::Expulsar),
            boton("acuse", VERDE, Msg::AcusarRecibo),
        ]),
    );

    // Grafo de confianza desde uno mismo.
    hijos.push(rotulo("confianza (saltos)"));
    if confianza.is_empty() {
        hijos.push(
            View::new(estilo_fila_auto()).text_aligned(
                "— sin atestaciones —".to_string(),
                13.0,
                c(TENUE),
                Alignment::Start,
            ),
        );
    } else {
        for (id, saltos) in &confianza {
            hijos.push(
                View::new(estilo_fila_auto()).text_aligned(
                    format!("{} · {}↑", hex_corto(id), saltos),
                    13.0,
                    c(SOCIAL),
                    Alignment::Start,
                ),
            );
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(238.0_f32),
            height: percent(1.0_f32),
        },
        gap: gap_v(5.0),
        padding: lados(12.0, 12.0),
        ..Default::default()
    })
    .fill(c(PANEL))
    .clip(true)
    .children(hijos)
}

/// Una fila de persona: clic = seleccionar; resalta al seleccionado.
fn fila_persona(model: &Modelo, id: &AgoraId, memb: &ayni_app::Membresia, yo: &AgoraId) -> View<Msg> {
    let mut etiqueta = hex_corto(id);
    if Some(*id) == memb.fundador {
        etiqueta.push_str(" ·fund");
    }
    if id == yo {
        etiqueta.push_str(" ←vos");
    }
    if model.nucleo.reciproca(id) {
        etiqueta.push_str(" ✓rx");
    }
    let seleccionado = model.seleccionado == Some(*id);
    let fondo = if seleccionado { SEL } else { PANEL };
    let color = if id == yo { MIO } else { CLARO };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: lados(6.0, 0.0),
        ..Default::default()
    })
    .fill(c(fondo))
    .hover_fill(c(SEL))
    .radius(5.0)
    .text_aligned(etiqueta, 13.0, c(color), Alignment::Start)
    .on_click(Msg::Seleccionar(*id))
}

// === Columna CHARLA (hilo + compose) =========================================
fn columna_charla(model: &Modelo) -> View<Msg> {
    let yo = model.nucleo.yo();
    let recibos = model.nucleo.conv.recibos();
    let nodos = model.nucleo.conv.instantanea();
    let n = nodos.len();
    let fin = n.saturating_sub(model.scroll);
    let ini = fin.saturating_sub(VISIBLES);

    let mut filas: Vec<View<Msg>> = Vec::new();
    if n == 0 {
        filas.push(
            View::new(estilo_fila_auto()).text_aligned(
                "— sin mensajes. Escribí abajo (o /ayuda para comandos). —".to_string(),
                14.0,
                c(TENUE),
                Alignment::Start,
            ),
        );
    }
    for nodo in &nodos[ini..fin] {
        let propio = *nodo.autor() == yo;
        let es_social = !matches!(
            nodo.contenido.carga,
            ayni_app::Carga::Texto(_) | ayni_app::Carga::Cifrado(_)
        );
        let color = if es_social {
            SOCIAL
        } else if propio {
            MIO
        } else {
            AJENO
        };
        let vistos = recibos.get(&nodo.id()).map(|s| s.len()).unwrap_or(0);
        let sello = if vistos > 0 { format!("  ✓{vistos}") } else { String::new() };
        let linea = format!(
            "[{}] {}{}",
            hex_corto(nodo.autor()),
            model.nucleo.texto_visible(nodo),
            sello
        );
        filas.push(
            View::new(estilo_fila_auto()).text_aligned(linea, 15.0, c(color), Alignment::Start),
        );
    }

    let hint = if model.scroll > 0 {
        format!("⟂ scroll +{} (rueda)   ", model.scroll)
    } else {
        String::new()
    };
    let lista = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        gap: gap_v(7.0),
        padding: lados(16.0, 12.0),
        ..Default::default()
    })
    .fill(c(FONDO))
    .clip(true)
    .children(filas);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![lista, fila_compose(model, &hint)])
}

fn fila_compose(model: &Modelo, hint: &str) -> View<Msg> {
    let (texto, color) = if model.entrada.is_empty() {
        (
            format!("{hint}escribí un mensaje, o /adjuntar <ruta>, /atestar <hex> …"),
            TENUE,
        )
    } else {
        (format!("{}▏", model.entrada), CLARO)
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
    .fill(c((36, 42, 55)))
    .radius(8.0)
    .text_aligned(texto, 15.0, c(color), Alignment::Start);

    let fila = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(56.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: gap_h(8.0),
        padding: lados(16.0, 8.0),
        ..Default::default()
    })
    .fill(c(BARRA))
    .children(vec![caja, boton("enviar", VERDE, Msg::Enviar)]);
    fila
}

// === Barra de estado =========================================================
fn barra_estado(model: &Modelo) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: lados(16.0, 0.0),
        ..Default::default()
    })
    .fill(c(PANEL))
    .text_aligned(format!("· {}", model.aviso), 12.0, c(TENUE), Alignment::Start)
}

// === Comandos de la barra `/` ================================================
fn ejecutar_comando(nucleo: &mut Nucleo, enlace: &Enlace, cmd: &str) -> String {
    let mut campos = cmd.split_whitespace();
    let verbo = campos.next().unwrap_or("");
    match verbo {
        "adjuntar" | "adj" => {
            let ruta = campos.collect::<Vec<_>>().join(" ");
            if ruta.is_empty() {
                return "uso: /adjuntar <ruta>".into();
            }
            match nucleo.adjuntar(enlace, &ruta) {
                Ok(n) => format!("adjuntado: {n}"),
                Err(e) => e,
            }
        }
        "admitir" | "expulsar" | "atestar" => {
            let Some(pref) = campos.next() else {
                return format!("uso: /{verbo} <hex>");
            };
            let Some(sujeto) = nucleo.resolver(pref) else {
                return format!("no conozco a «{pref}»");
            };
            match verbo {
                "admitir" => {
                    nucleo.admitir(enlace, sujeto);
                    format!("admitiste a {}", hex_corto(&sujeto))
                }
                "expulsar" => {
                    nucleo.expulsar(enlace, sujeto);
                    format!("expulsaste a {}", hex_corto(&sujeto))
                }
                _ => {
                    let nivel: u8 = campos.next().and_then(|s| s.parse().ok()).unwrap_or(5);
                    nucleo.atestar(enlace, sujeto, nivel);
                    format!("das fe de {} (nivel {nivel})", hex_corto(&sujeto))
                }
            }
        }
        "recibo" => {
            nucleo.acusar_cabezas(enlace);
            "acuse de recibo enviado".into()
        }
        "ayuda" | "" => {
            "/adjuntar <ruta> · /admitir <hex> · /expulsar <hex> · /atestar <hex> [nivel] · /recibo"
                .into()
        }
        otro => format!("comando desconocido: «{otro}» (/ayuda)"),
    }
}

// === Helpers de vista ========================================================
fn chip(label: String, color_borde: (u8, u8, u8), msg: Msg) -> View<Msg> {
    View::new(Style {
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: lados(10.0, 5.0),
        ..Default::default()
    })
    .fill(c((40, 46, 60)))
    .radius(12.0)
    .text(label, 13.0, c(color_borde))
    .on_click(msg)
}

fn boton(label: &str, bg: (u8, u8, u8), msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(86.0_f32),
            height: length(34.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(c(bg))
    .radius(8.0)
    .text(label.to_string(), 14.0, c((12, 18, 14)))
    .on_click(msg)
}

fn rotulo(texto: &str) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        margin: Rect {
            left: length(0.0),
            right: length(0.0),
            top: length(8.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .text_aligned(texto.to_uppercase(), 11.0, c(ACENTO), Alignment::Start)
}

fn estilo_fila_auto() -> Style {
    Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    }
}

fn estilo_flex_fila(grow: f32) -> Style {
    Style {
        size: Size {
            width: Dimension::auto(),
            height: Dimension::auto(),
        },
        flex_grow: grow,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    }
}

fn lados(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

fn gap_h(x: f32) -> Size<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Size {
        width: length(x),
        height: length(0.0),
    }
}

fn gap_v(y: f32) -> Size<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Size {
        width: length(0.0),
        height: length(y),
    }
}

fn si_no(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn main() {
    llimphi_ui::run::<Ayni>();
}
