// =============================================================================
//  uya-llimphi — la cara gráfica de la videollamada soberana.
// -----------------------------------------------------------------------------
//  Frontend Llimphi (bucle Elm) sobre `uya-app::Enlace`: un hilo de red bombea
//  los `EventoUya` al `update` vía `Handle::dispatch`. La vista es una rejilla
//  de caras —un tile por participante con su último cuadro RGBA pintado con
//  `View::image`— más una barra de controles (cámara / micrófono / colgar).
//
//  Configuración por entorno (calcada de ayni):
//    UYA_NOMBRE    nombre → identidad determinista (default "yo")
//    UYA_ESCUCHAR  multiaddr donde escuchar (default /ip4/0.0.0.0/tcp/0)
//    UYA_CONECTAR  multiaddr(s) dialable(s) a conectar (con /p2p/<peerid>,
//                  coma-separado; lo imprime el otro nodo al arrancar)
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::Arc;

use uya_app::{iniciar_camara, Enlace, EventoUya, ParticipanteId, Sala};

use llimphi_ui::llimphi_layout::taffy::prelude::{auto, length, percent, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{
    AlignItems, FlexDirection, FlexWrap, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Color, Image as PenikoImage, ImageFormat};
use llimphi_ui::{
    App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta,
};

use llimphi_clipboard::SystemClipboard;
use llimphi_widget_edit_menu::{self as editmenu, EditAction};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

// --- Paleta (sobria, oscura). Hardcoded a propósito: MVP feo. ----------------
const FONDO: Color = Color::from_rgba8(16, 18, 24, 255);
const TILE_BG: Color = Color::from_rgba8(24, 27, 35, 255);
const VIDEO_BG: Color = Color::from_rgba8(10, 11, 15, 255);
const BARRA_BG: Color = Color::from_rgba8(20, 22, 29, 255);
const BOTON_BG: Color = Color::from_rgba8(44, 49, 62, 255);
const ACENTO_BG: Color = Color::from_rgba8(46, 110, 96, 255);
const ROJO_BG: Color = Color::from_rgba8(150, 56, 56, 255);
const TEXTO: Color = Color::from_rgba8(222, 226, 233, 255);
const TENUE: Color = Color::from_rgba8(128, 134, 146, 255);
const ACENTO: Color = Color::from_rgba8(120, 210, 184, 255);

/// Cuántos renglones de charla pinta la ventana visible (con scroll por rueda).
const VENTANA_CHARLA: usize = 14;

/// El último cuadro de video conocido de un participante.
struct CuadroUI {
    ancho: u16,
    alto: u16,
    rgba: Arc<Vec<u8>>,
}

/// Una línea de la charla, ya resuelta a nombre + cuerpo.
struct LineaCharla {
    nombre: String,
    texto: String,
    /// `true` si la escribí yo (se pinta con acento).
    yo: bool,
}

/// Qué campo de texto tiene el foco del teclado.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Foco {
    /// La barra de unión (pegar dirección de un par).
    Conectar,
    /// La charla (escribir un mensaje a la sala).
    Charla,
}

struct Modelo {
    sala: Sala,
    enlace: Arc<Enlace>,
    cuadros: HashMap<ParticipanteId, CuadroUI>,
    /// Quiénes están hablando ahora mismo (detección de voz), para resaltarlos.
    hablando: HashSet<ParticipanteId>,
    cam_on: bool,
    mic_on: bool,
    /// Mi propia dirección dialable (con `/p2p/`), para mostrarla y compartirla.
    mi_dir: String,
    /// Campo donde se pega/teclea la dirección de un par a conectar.
    conectar_input: TextInputState,
    /// Campo donde se escribe un mensaje de la charla.
    charla_input: TextInputState,
    /// El hilo de la charla (cronológico; lo más nuevo al final).
    charla: Vec<LineaCharla>,
    /// Líneas desplazadas desde el fondo (0 = pegado a lo más nuevo).
    charla_scroll: usize,
    /// Qué campo de texto recibe el teclado.
    foco: Foco,
    /// Clipboard del sistema, para pegar la dirección con Ctrl/Cmd+V.
    clipboard: SystemClipboard,
    /// Salida de audio: hay que conservarla viva (al soltarla, el stream
    /// de reproducción se cierra). `None` si no hay dispositivo de salida.
    _audio: Option<uya_app::AudioSink>,
}

#[derive(Clone)]
enum Msg {
    /// Un evento de la llamada llegó por el hilo de red.
    Red(EventoUya),
    /// Una tecla para el campo enfocado.
    Tecla(KeyEvent),
    /// Conectar a la dirección tecleada/pegada.
    Conectar,
    /// Enviar el mensaje escrito en la charla.
    EnviarCharla,
    /// Mover el foco del teclado a un campo (clic).
    Enfocar(Foco),
    /// Desplazar el hilo de la charla (+N hacia lo más viejo).
    ScrollCharla(i32),
    ToggleCamara,
    ToggleMicrofono,
    Colgar,
}

struct Uya;

impl App for Uya {
    type Model = Modelo;
    type Msg = Msg;

    fn title() -> &'static str {
        "uya · videollamada soberana"
    }

    fn initial_size() -> (u32, u32) {
        (960, 720)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let nombre = env::var("UYA_NOMBRE").unwrap_or_else(|_| "yo".into());
        let bind = env::var("UYA_ESCUCHAR").unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/0".into());

        let (enlace, rx) = Enlace::abrir(nombre.clone(), &bind)
            .unwrap_or_else(|e| panic!("uya: no pude escuchar en {bind}: {e}"));
        let enlace = Arc::new(enlace);
        println!("uya: dialable en {}", enlace.direccion_local());

        if let Ok(pares) = env::var("UYA_CONECTAR") {
            for par in pares.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                enlace.conectar(par);
            }
        }
        // Descubrimiento por sala (DHT): unirse por nombre en vez de pegar dirección.
        if let Ok(sala) = env::var("UYA_SALA") {
            let bootstrap: Vec<String> = env::var("UYA_BOOTSTRAP")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            uya_app::iniciar_baliza_lan(enlace.clone(), sala.clone());
            enlace.unir_sala(sala, bootstrap);
        }

        // Cámara sintética (256×192 @ 12 fps): preview local + difusión a pares.
        iniciar_camara(enlace.clone(), 256, 192, 12.0);
        // Audio: reproducción de la mezcla remota + captura de micrófono.
        let audio = uya_app::iniciar_reproduccion(enlace.mezcla());
        uya_app::iniciar_microfono(enlace.clone());

        // Hilo de red: cada evento se reinyecta al bucle Elm.
        let h = handle.clone();
        std::thread::spawn(move || {
            for evento in rx {
                h.dispatch(Msg::Red(evento));
            }
        });

        let mi_dir = enlace.direccion_local().to_string();
        Modelo {
            sala: Sala::nueva(nombre),
            enlace,
            cuadros: HashMap::new(),
            hablando: HashSet::new(),
            cam_on: true,
            mic_on: true,
            mi_dir,
            conectar_input: TextInputState::new(),
            charla_input: TextInputState::new(),
            charla: Vec::new(),
            charla_scroll: 0,
            foco: Foco::Charla,
            clipboard: SystemClipboard::new(),
            _audio: audio,
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        match &e.key {
            // Enter dispara la acción del campo enfocado.
            Key::Named(NamedKey::Enter) => Some(match model.foco {
                Foco::Conectar => Msg::Conectar,
                Foco::Charla => Msg::EnviarCharla,
            }),
            // Todo lo demás (incluido Ctrl/Cmd+V) va al campo enfocado.
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
        Some(Msg::ScrollCharla(if delta.y > 0.0 { 3 } else { -3 }))
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Red(EventoUya::Entra { id, nombre }) => {
                if id != model.sala.yo {
                    model.sala.entrar(id, nombre);
                }
            }
            Msg::Red(EventoUya::Sale { id }) => {
                model.sala.salir(&id);
                model.cuadros.remove(&id);
                model.hablando.remove(&id);
            }
            Msg::Red(EventoUya::Voz { id, hablando }) => {
                if hablando {
                    model.hablando.insert(id);
                } else {
                    model.hablando.remove(&id);
                }
            }
            Msg::Red(EventoUya::Estado {
                id,
                camara,
                microfono,
            }) => {
                model.sala.set_estado(&id, camara, microfono);
            }
            Msg::Red(EventoUya::Cuadro {
                id,
                ancho,
                alto,
                rgba,
            }) => {
                model.cuadros.insert(id, CuadroUI { ancho, alto, rgba });
            }
            Msg::Red(EventoUya::Mensaje { id, nombre, texto }) => {
                model.charla.push(LineaCharla {
                    nombre,
                    texto,
                    yo: id == model.sala.yo,
                });
                model.charla_scroll = 0;
            }
            Msg::ToggleCamara => {
                model.cam_on = !model.cam_on;
                model.enlace.set_camara(model.cam_on);
                if !model.cam_on {
                    model.cuadros.remove(&model.sala.yo);
                }
            }
            Msg::ToggleMicrofono => {
                model.mic_on = !model.mic_on;
                model.enlace.set_microfono(model.mic_on);
            }
            Msg::Tecla(e) => {
                // El tecleo va al campo enfocado; Ctrl/Cmd+V pega del clipboard.
                let campo = match model.foco {
                    Foco::Conectar => &mut model.conectar_input,
                    Foco::Charla => &mut model.charla_input,
                };
                let es_v = matches!(&e.key, Key::Character(c) if c.eq_ignore_ascii_case("v"));
                if (e.modifiers.ctrl || e.modifiers.meta) && es_v {
                    editmenu::apply(campo.editor_mut(), EditAction::Paste, &mut model.clipboard);
                } else {
                    campo.apply_key(&e);
                }
            }
            Msg::Enfocar(f) => {
                model.foco = f;
            }
            Msg::Conectar => {
                let dir = model.conectar_input.text().trim().to_string();
                if !dir.is_empty() {
                    model.enlace.conectar(&dir);
                    model.conectar_input.clear();
                }
            }
            Msg::EnviarCharla => {
                let texto = model.charla_input.text().trim().to_string();
                if !texto.is_empty() {
                    model.enlace.enviar_mensaje(texto.clone());
                    // Eco local: la red no me devuelve mis propios mensajes.
                    model.charla.push(LineaCharla {
                        nombre: format!("{} (yo)", model.sala.mi_nombre),
                        texto,
                        yo: true,
                    });
                    model.charla_scroll = 0;
                    model.charla_input.clear();
                }
            }
            Msg::ScrollCharla(d) => {
                let max = model.charla.len().saturating_sub(VENTANA_CHARLA);
                let nuevo = model.charla_scroll as i32 + d;
                model.charla_scroll = nuevo.clamp(0, max as i32) as usize;
            }
            Msg::Colgar => {
                model.enlace.colgar();
                model.sala.participantes.clear();
                let yo = model.sala.yo;
                model.cuadros.retain(|id, _| *id == yo);
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let mut tiles: Vec<View<Msg>> = Vec::new();

        // Mi propia cara primero.
        tiles.push(tile(
            &format!("{} (yo)", model.sala.mi_nombre),
            model.cuadros.get(&model.sala.yo),
            model.cam_on,
            model.mic_on,
            true,
            model.hablando.contains(&model.sala.yo),
        ));
        // Los demás, en orden estable por id (BTreeMap).
        for p in model.sala.participantes.values() {
            tiles.push(tile(
                &p.nombre,
                model.cuadros.get(&p.id),
                p.camara,
                p.microfono,
                false,
                model.hablando.contains(&p.id),
            ));
        }

        let rejilla = View::new(Style {
            size: Size {
                width: percent(1.0),
                height: auto(),
            },
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            flex_grow: 1.0,
            gap: Size {
                width: length(12.0),
                height: length(12.0),
            },
            padding: rect_all(12.0),
            ..Default::default()
        })
        .fill(FONDO)
        .children(tiles);

        // Zona superior: rejilla de caras (crece) + panel de charla (fijo).
        let superior = View::new(Style {
            size: Size {
                width: percent(1.0),
                height: auto(),
            },
            flex_direction: FlexDirection::Row,
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![rejilla, panel_charla(model)]);

        View::new(Style {
            size: Size {
                width: percent(1.0),
                height: percent(1.0),
            },
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .fill(FONDO)
        .children(vec![superior, barra_conectar(model), barra_controles(model)])
    }
}

/// El panel lateral de la charla: título + hilo con scroll + campo de envío.
fn panel_charla(model: &Modelo) -> View<Msg> {
    let titulo = View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(26.0),
        },
        ..Default::default()
    })
    .text("charla", 14.0, ACENTO);

    // Ventana visible del hilo: las últimas `VENTANA_CHARLA` líneas, corridas
    // por el offset de scroll (0 = pegado a lo más nuevo).
    let total = model.charla.len();
    let fin = total.saturating_sub(model.charla_scroll);
    let ini = fin.saturating_sub(VENTANA_CHARLA);
    let mut lineas: Vec<View<Msg>> = Vec::new();
    if total == 0 {
        lineas.push(linea_charla_view("— sin mensajes todavía —", TENUE));
    } else {
        for l in &model.charla[ini..fin] {
            let color = if l.yo { ACENTO } else { TEXTO };
            lineas.push(linea_charla_view(&format!("{}: {}", l.nombre, l.texto), color));
        }
    }

    let hilo = View::new(Style {
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0),
            height: length(4.0),
        },
        ..Default::default()
    })
    .children(lineas);

    let campo = View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(34.0),
        },
        ..Default::default()
    })
    .children(vec![text_input_view(
        &model.charla_input,
        "escribí y Enter…",
        model.foco == Foco::Charla,
        &TextInputPalette::default(),
        Msg::Enfocar(Foco::Charla),
    )]);

    View::new(Style {
        size: Size {
            width: length(280.0),
            height: percent(1.0),
        },
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(0.0),
            height: length(8.0),
        },
        padding: rect_all(12.0),
        ..Default::default()
    })
    .fill(BARRA_BG)
    .children(vec![titulo, hilo, campo])
}

/// Una línea del hilo de la charla.
fn linea_charla_view(texto: &str, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .text(texto.to_string(), 13.0, color)
}

/// La barra de unión: mi dirección dialable (para compartir) + un campo donde
/// pegar la dirección de un par + botón conectar.
fn barra_conectar(model: &Modelo) -> View<Msg> {
    let mi = View::new(Style {
        size: Size {
            width: percent(0.4),
            height: percent(1.0),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(format!("tu dirección:  {}", model.mi_dir), 12.0, TENUE);

    let campo = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: auto(),
            height: length(34.0),
        },
        ..Default::default()
    })
    .children(vec![text_input_view(
        &model.conectar_input,
        "pegá (Ctrl+V) la dirección de un par y Enter…",
        model.foco == Foco::Conectar,
        &TextInputPalette::default(),
        Msg::Enfocar(Foco::Conectar),
    )]);

    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(50.0),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(12.0),
            height: length(0.0),
        },
        padding: rect_all(8.0),
        ..Default::default()
    })
    .fill(BARRA_BG)
    .children(vec![mi, campo, boton("conectar", Msg::Conectar, true, false)])
}

/// Un tile de participante: video (o placeholder) arriba + etiqueta abajo. Si
/// `hablando`, el marco se tiñe de acento (detección de voz).
fn tile(
    nombre: &str,
    cuadro: Option<&CuadroUI>,
    cam: bool,
    mic: bool,
    yo: bool,
    hablando: bool,
) -> View<Msg> {
    let estilo_video = Style {
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    };

    let video = match (cam, cuadro) {
        (true, Some(c)) => {
            let blob = Blob::from((*c.rgba).clone());
            let imagen = PenikoImage::new(blob, ImageFormat::Rgba8, c.ancho as u32, c.alto as u32);
            View::new(estilo_video).fill(VIDEO_BG).radius(8.0).image(imagen)
        }
        (true, None) => View::new(estilo_video)
            .fill(VIDEO_BG)
            .radius(8.0)
            .text("conectando...", 15.0, TENUE),
        (false, _) => View::new(estilo_video)
            .fill(VIDEO_BG)
            .radius(8.0)
            .text("camara apagada", 15.0, TENUE),
    };

    let etiqueta = if !mic {
        format!("{nombre}  ·  mic off")
    } else if hablando {
        format!("{nombre}  ·  hablando")
    } else {
        nombre.to_string()
    };
    let color_label = if hablando || yo { ACENTO } else { TEXTO };
    let label = View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(24.0),
        },
        ..Default::default()
    })
    .text(etiqueta, 14.0, color_label);

    // El marco se tiñe de acento cuando este participante está hablando.
    let marco = if hablando { ACENTO_BG } else { TILE_BG };

    View::new(Style {
        size: Size {
            width: length(300.0),
            height: length(232.0),
        },
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(0.0),
            height: length(6.0),
        },
        padding: rect_all(6.0),
        ..Default::default()
    })
    .fill(marco)
    .radius(10.0)
    .children(vec![video, label])
}

/// La barra inferior: cámara / micrófono / colgar.
fn barra_controles(model: &Modelo) -> View<Msg> {
    let cam_label = if model.cam_on {
        "camara: on"
    } else {
        "camara: off"
    };
    let mic_label = if model.mic_on {
        "microfono: on"
    } else {
        "microfono: off"
    };

    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(60.0),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(12.0),
            height: length(0.0),
        },
        padding: rect_all(10.0),
        ..Default::default()
    })
    .fill(BARRA_BG)
    .children(vec![
        boton(cam_label, Msg::ToggleCamara, model.cam_on, false),
        boton(mic_label, Msg::ToggleMicrofono, model.mic_on, false),
        boton("colgar", Msg::Colgar, false, true),
    ])
}

/// Un botón de la barra. `activo` lo pinta con acento; `peligro` en rojo.
fn boton(label: &str, msg: Msg, activo: bool, peligro: bool) -> View<Msg> {
    let bg = if peligro {
        ROJO_BG
    } else if activo {
        ACENTO_BG
    } else {
        BOTON_BG
    };
    View::new(Style {
        size: Size {
            width: length(160.0),
            height: length(40.0),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(8.0)
    .on_click(msg)
    .text(label, 14.0, TEXTO)
}

/// Atajo: un `Rect` uniforme para padding/márgenes.
fn rect_all(v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(v),
        right: length(v),
        top: length(v),
        bottom: length(v),
    }
}

fn main() {
    llimphi_ui::run::<Uya>();
}
