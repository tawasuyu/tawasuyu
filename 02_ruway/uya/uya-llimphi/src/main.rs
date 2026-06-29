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
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use uya_app::{hex_corto, iniciar_camara, Enlace, EventoUya, ParticipanteId, Sala};

use llimphi_ui::llimphi_layout::taffy::prelude::{auto, length, percent, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{
    AlignItems, FlexDirection, FlexWrap, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush as PenikoImage, ImageData, ImageFormat,
};
use llimphi_ui::{
    App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta,
};

use rimay_localize::{t, t_args};

use llimphi_clipboard::SystemClipboard;
use llimphi_icons::Icon;
use llimphi_theme::{motion, Theme};
use llimphi_widget_edit_menu::{self as editmenu, EditAction};
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_toast::{toast_stack_view, Toast};

/// Cuánto vive un aviso (toast) antes de auto-descartarse.
const TOAST_TTL: Duration = Duration::from_secs(4);

/// Hash estable de una cadena → `key` para animaciones implícitas: la misma
/// identidad produce siempre la misma key entre rebuilds, así un tile sólo
/// hace su pop-in la primera vez que aparece.
fn key_of(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

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
const ROJO: Color = Color::from_rgba8(230, 132, 132, 255);

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
    /// Pares que silencié localmente (espejo del set en la `MezclaRemota`).
    silenciados: HashSet<ParticipanteId>,
    cam_on: bool,
    mic_on: bool,
    /// ¿Estoy compartiendo pantalla en vez de la cámara?
    pantalla_on: bool,
    /// Mi propia dirección dialable (con `/p2p/`), para mostrarla y compartirla.
    mi_dir: String,
    /// Huella corta de mi identidad (BLAKE3 de mi clave), para verificación
    /// fuera de banda: el par contrasta que coincida con la que ve.
    mi_huella: String,
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
    /// Avisos efímeros (alguien entra/sale, conexión, colgar).
    toasts: Vec<Toast>,
    /// Id incremental para correlacionar un toast con su Msg de expiración.
    next_toast: u64,
    /// Tamaño actual de la ventana, para anclar la pila de toasts.
    viewport: (f32, f32),
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
    /// Empezar / dejar de compartir pantalla.
    TogglePantalla,
    /// Silenciar / reactivar a un par localmente (clic en su tile).
    ToggleSilencio(ParticipanteId),
    Colgar,
    /// La ventana cambió de tamaño (para anclar los toasts).
    Resize(u32, u32),
    /// Un toast cumplió su tiempo: se descarta de la pila.
    ToastExpire(u64),
}

/// Empuja un aviso a la pila y programa su expiración en `TOAST_TTL`.
fn notar(model: &mut Modelo, handle: &Handle<Msg>, make: impl FnOnce(u64) -> Toast) {
    let id = model.next_toast;
    model.next_toast += 1;
    model.toasts.push(make(id));
    handle.spawn(move || {
        std::thread::sleep(TOAST_TTL);
        Msg::ToastExpire(id)
    });
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
        rimay_localize::init();
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

        // Compartir pantalla desde el arranque: UYA_PANTALLA=1 (necesita feature
        // `pantalla` + display). Se fija antes de la captura para que el hilo
        // arranque en ese modo.
        let pantalla_on = env::var("UYA_PANTALLA").is_ok();
        enlace.set_compartir_pantalla(pantalla_on);
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
        let yo = enlace.yo();
        let mi_huella = hex_corto(&yo);
        Modelo {
            sala: Sala::nueva(yo, nombre),
            enlace,
            cuadros: HashMap::new(),
            hablando: HashSet::new(),
            silenciados: HashSet::new(),
            cam_on: true,
            mic_on: true,
            pantalla_on,
            mi_dir,
            mi_huella,
            conectar_input: TextInputState::new(),
            charla_input: TextInputState::new(),
            charla: Vec::new(),
            charla_scroll: 0,
            foco: Foco::Charla,
            clipboard: SystemClipboard::new(),
            toasts: Vec::new(),
            next_toast: 0,
            viewport: (960.0, 720.0),
            _audio: audio,
        }
    }

    fn on_resize(_model: &Self::Model, w: u32, h: u32) -> Option<Self::Msg> {
        Some(Msg::Resize(w, h))
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

    fn update(mut model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Red(EventoUya::Entra {
                id,
                nombre,
                verificado,
            }) => {
                if id != model.sala.yo {
                    model.sala.entrar(id, nombre.clone(), verificado);
                    if verificado {
                        let msg = t_args("uya-toast-unio", &[("nombre", nombre.clone().into())]);
                        notar(&mut model, handle, |tid| Toast::info(tid, msg, TOAST_TTL));
                    } else {
                        let msg =
                            t_args("uya-toast-unio-sin-verificar", &[("nombre", nombre.clone().into())]);
                        notar(&mut model, handle, |tid| Toast::warning(tid, msg, TOAST_TTL));
                    }
                }
            }
            Msg::Red(EventoUya::Sale { id }) => {
                let nombre = model
                    .sala
                    .participantes
                    .get(&id)
                    .map(|p| p.nombre.clone());
                model.sala.salir(&id);
                model.cuadros.remove(&id);
                model.hablando.remove(&id);
                model.silenciados.remove(&id);
                if let Some(nombre) = nombre {
                    let msg = t_args("uya-toast-salio", &[("nombre", nombre.into())]);
                    notar(&mut model, handle, |tid| Toast::info(tid, msg, TOAST_TTL));
                }
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
            Msg::TogglePantalla => {
                model.pantalla_on = !model.pantalla_on;
                model.enlace.set_compartir_pantalla(model.pantalla_on);
                // Al compartir pantalla la cámara queda implícita en "on" (la
                // fuente cambia, no el flag de cámara); si la tenía apagada, la
                // reactivo para que el cambio de fuente surta efecto.
                if model.pantalla_on && !model.cam_on {
                    model.cam_on = true;
                    model.enlace.set_camara(true);
                }
                // El preview viejo (cámara) deja de ser válido; que repinte solo.
                model.cuadros.remove(&model.sala.yo);
            }
            Msg::ToggleSilencio(id) => {
                let nuevo = !model.silenciados.contains(&id);
                model.enlace.silenciar_par(id, nuevo);
                if nuevo {
                    model.silenciados.insert(id);
                } else {
                    model.silenciados.remove(&id);
                }
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
                    let msg = t("uya-toast-conectando");
                    notar(&mut model, handle, |tid| Toast::info(tid, msg, TOAST_TTL));
                }
            }
            Msg::EnviarCharla => {
                let texto = model.charla_input.text().trim().to_string();
                if !texto.is_empty() {
                    model.enlace.enviar_mensaje(texto.clone());
                    // Eco local: la red no me devuelve mis propios mensajes.
                    model.charla.push(LineaCharla {
                        nombre: t_args(
                            "uya-yo-suffix",
                            &[("nombre", model.sala.mi_nombre.clone().into())],
                        ),
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
                let msg = t("uya-toast-colgada");
                notar(&mut model, handle, |tid| Toast::info(tid, msg, TOAST_TTL));
            }
            Msg::Resize(w, h) => {
                model.viewport = (w as f32, h as f32);
            }
            Msg::ToastExpire(id) => {
                model.toasts.retain(|t| t.id != id);
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let mut tiles: Vec<View<Msg>> = Vec::new();

        // Mi propia cara primero (yo siempre soy de confianza para mí mismo; mi
        // propio audio no se silencia ni hace clic). Cada tile entra con un
        // pop-in la primera vez que aparece su `key` (estable por identidad).
        tiles.push(
            tile(
                &t_args("uya-yo-suffix", &[("nombre", model.sala.mi_nombre.clone().into())]),
                model.cuadros.get(&model.sala.yo),
                model.cam_on,
                model.mic_on,
                true,
                model.hablando.contains(&model.sala.yo),
                true,
                false,
                None,
            )
            .animated_enter(key_of("yo"), motion::NORMAL),
        );
        // Los demás, en orden estable por id (BTreeMap). Clic = silenciarlos.
        for p in model.sala.participantes.values() {
            tiles.push(
                tile(
                    &p.nombre,
                    model.cuadros.get(&p.id),
                    p.camara,
                    p.microfono,
                    false,
                    model.hablando.contains(&p.id),
                    p.verificado,
                    model.silenciados.contains(&p.id),
                    Some(Msg::ToggleSilencio(p.id)),
                )
                .animated_enter(key_of(&hex_corto(&p.id)), motion::NORMAL),
            );
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

        let raiz = View::new(Style {
            size: Size {
                width: percent(1.0),
                height: percent(1.0),
            },
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .fill(FONDO)
        .children(vec![superior, barra_conectar(model), barra_controles(model)]);

        // Capa de avisos efímeros (bottom-right). Clic en uno = descartarlo.
        let now = Instant::now();
        let vivos: Vec<Toast> = model
            .toasts
            .iter()
            .filter(|t| t.is_alive(now))
            .cloned()
            .collect();
        if vivos.is_empty() {
            raiz
        } else {
            View::new(Style {
                size: Size {
                    width: percent(1.0),
                    height: percent(1.0),
                },
                ..Default::default()
            })
            .children(vec![
                raiz,
                toast_stack_view(&vivos, model.viewport, Msg::ToastExpire),
            ])
        }
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
    .text(t("uya-charla-titulo"), 14.0, ACENTO);

    // Ventana visible del hilo: las últimas `VENTANA_CHARLA` líneas, corridas
    // por el offset de scroll (0 = pegado a lo más nuevo).
    let total = model.charla.len();
    let fin = total.saturating_sub(model.charla_scroll);
    let ini = fin.saturating_sub(VENTANA_CHARLA);
    let mut lineas: Vec<View<Msg>> = Vec::new();
    if total == 0 {
        // Estado vacío con orientación, en vez de un renglón tenue suelto.
        // Sin descripción: el panel es angosto (~256px) y la caja de texto de
        // `empty_view` es fija (360px); con sólo icono + título encaja limpio.
        let pal = EmptyPalette::from_theme(&Theme::dark());
        lineas.push(empty_view::<Msg>(Icon::FileText, t("uya-sin-mensajes"), None, &pal));
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
        &t("uya-charla-placeholder"),
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
    .text(
        t_args(
            "uya-huella",
            &[
                ("huella", model.mi_huella.clone().into()),
                ("dir", model.mi_dir.clone().into()),
            ],
        ),
        12.0,
        TENUE,
    );

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
        &t("uya-conectar-placeholder"),
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
    .children(vec![mi, campo, boton(&t("uya-conectar"), Msg::Conectar, true, false)])
}

/// Un tile de participante: video (o placeholder) arriba + etiqueta abajo. Si
/// `hablando`, el marco se tiñe de acento (detección de voz); si NO está
/// `verificado`, la etiqueta avisa en rojo; si `silenciado`, no suena de este
/// lado. `click` (si lo hay) se dispara al hacer clic en el tile.
#[allow(clippy::too_many_arguments)]
fn tile(
    nombre: &str,
    cuadro: Option<&CuadroUI>,
    cam: bool,
    mic: bool,
    yo: bool,
    hablando: bool,
    verificado: bool,
    silenciado: bool,
    click: Option<Msg>,
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
            let imagen = PenikoImage::new(ImageData {
                data: blob,
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: c.ancho as u32,
                height: c.alto as u32,
            });
            View::new(estilo_video).fill(VIDEO_BG).radius(8.0).image(imagen)
        }
        (true, None) => View::new(estilo_video)
            .fill(VIDEO_BG)
            .radius(8.0)
            .text(t("uya-video-conectando"), 15.0, TENUE),
        (false, _) => View::new(estilo_video)
            .fill(VIDEO_BG)
            .radius(8.0)
            .text(t("uya-camara-apagada"), 15.0, TENUE),
    };

    // Prioridad en la etiqueta: sin verificar (seguridad) > silenciado por mí
    // (no lo oigo, así que "hablando" sería engañoso) > mic off > hablando.
    let etiqueta = if !verificado {
        t_args("uya-label-sin-verificar", &[("nombre", nombre.into())])
    } else if silenciado {
        t_args("uya-label-silenciado", &[("nombre", nombre.into())])
    } else if !mic {
        t_args("uya-label-mic-off", &[("nombre", nombre.into())])
    } else if hablando {
        t_args("uya-label-hablando", &[("nombre", nombre.into())])
    } else {
        nombre.to_string()
    };
    let color_label = if !verificado {
        ROJO
    } else if silenciado {
        TENUE
    } else if hablando || yo {
        ACENTO
    } else {
        TEXTO
    };
    let label = View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(24.0),
        },
        ..Default::default()
    })
    .text(etiqueta, 14.0, color_label);

    // El marco se tiñe de acento cuando este participante está hablando (salvo
    // que lo tenga silenciado: ahí no lo oigo, no corresponde resaltarlo).
    let marco = if hablando && !silenciado {
        ACENTO_BG
    } else {
        TILE_BG
    };

    let mut vista = View::new(Style {
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
    .children(vec![video, label]);

    // Clic en el tile de un par → silenciarlo / reactivarlo localmente.
    if let Some(msg) = click {
        vista = vista.on_click(msg);
    }
    vista
}

/// La barra inferior: cámara / micrófono / colgar.
fn barra_controles(model: &Modelo) -> View<Msg> {
    let cam_label = if model.cam_on {
        t("uya-camara-on")
    } else {
        t("uya-camara-off")
    };
    let mic_label = if model.mic_on {
        t("uya-microfono-on")
    } else {
        t("uya-microfono-off")
    };
    let pantalla_label = if model.pantalla_on {
        t("uya-compartiendo-pantalla")
    } else {
        t("uya-compartir-pantalla")
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
        boton(&cam_label, Msg::ToggleCamara, model.cam_on, false),
        boton(&mic_label, Msg::ToggleMicrofono, model.mic_on, false),
        boton(&pantalla_label, Msg::TogglePantalla, model.pantalla_on, false),
        boton(&t("uya-colgar"), Msg::Colgar, false, true),
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
