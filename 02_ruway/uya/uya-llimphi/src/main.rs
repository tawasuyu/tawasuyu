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

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use uya_app::{iniciar_camara, Enlace, EventoUya, ParticipanteId, Sala};

use llimphi_ui::llimphi_layout::taffy::prelude::{auto, length, percent, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{
    AlignItems, FlexDirection, FlexWrap, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Color, Image as PenikoImage, ImageFormat};
use llimphi_ui::{App, Handle, View};

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

/// El último cuadro de video conocido de un participante.
struct CuadroUI {
    ancho: u16,
    alto: u16,
    rgba: Arc<Vec<u8>>,
}

struct Modelo {
    sala: Sala,
    enlace: Arc<Enlace>,
    cuadros: HashMap<ParticipanteId, CuadroUI>,
    cam_on: bool,
    mic_on: bool,
    /// Salida de audio: hay que conservarla viva (al soltarla, el stream
    /// de reproducción se cierra). `None` si no hay dispositivo de salida.
    _audio: Option<uya_app::AudioSink>,
}

#[derive(Clone)]
enum Msg {
    /// Un evento de la llamada llegó por el hilo de red.
    Red(EventoUya),
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

        Modelo {
            sala: Sala::nueva(nombre),
            enlace,
            cuadros: HashMap::new(),
            cam_on: true,
            mic_on: true,
            _audio: audio,
        }
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
        ));
        // Los demás, en orden estable por id (BTreeMap).
        for p in model.sala.participantes.values() {
            tiles.push(tile(
                &p.nombre,
                model.cuadros.get(&p.id),
                p.camara,
                p.microfono,
                false,
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

        View::new(Style {
            size: Size {
                width: percent(1.0),
                height: percent(1.0),
            },
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .fill(FONDO)
        .children(vec![rejilla, barra_controles(model)])
    }
}

/// Un tile de participante: video (o placeholder) arriba + etiqueta abajo.
fn tile(nombre: &str, cuadro: Option<&CuadroUI>, cam: bool, mic: bool, yo: bool) -> View<Msg> {
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

    let etiqueta = if mic {
        nombre.to_string()
    } else {
        format!("{nombre}  ·  mic off")
    };
    let label = View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(24.0),
        },
        ..Default::default()
    })
    .text(etiqueta, 14.0, if yo { ACENTO } else { TEXTO });

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
    .fill(TILE_BG)
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
