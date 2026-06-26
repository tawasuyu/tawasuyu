//! `willay-panel` — el **feed** del centro de eventos.
//!
//! Un sidebar wlr-layer-shell que lista el timeline heterogéneo (notificaciones,
//! capturas, clips) más reciente arriba, con separadores de fecha y facetas por
//! clase (chips Todo/Notif/Capturas/Clips). Es cliente del daemon willay: un
//! hilo pollea el socket (`Recientes`/`PorClase`) y despacha al render. Generaliza
//! `pata-notify-panel` (mismo patrón App+scroll), pero sobre el socket propio.

use std::hash::{Hash, Hasher};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent, Rect};
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_compositor::ImageFit;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};

use llimphi_theme::{motion, Theme};
use llimphi_icons::Icon;
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_widget_skeleton::{skeleton_view, SkeletonPalette};
use llimphi_widget_toast::{toast_stack_view, Toast};

use llimphi_image::Image;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use willay_core::proto::{Respuesta, Solicitud};
use willay_core::{Clase, Evento};
use willay_emit::{ahora_usec, Emisor};

const PANEL_W: u32 = 380;
const ITEM_H: f32 = 46.0;
const SEP_H: f32 = 28.0;
const TITLE_H: f32 = 44.0;
const CHIPS_H: f32 = 40.0;
const HEADER_H: f32 = TITLE_H + CHIPS_H;
/// Poll de **seguridad**: la actualización inmediata la da la suscripción push
/// del daemon; esto es sólo un respaldo por si la conexión de push se cayó.
const POLL: Duration = Duration::from_secs(10);
/// Tope de eventos que pide el feed.
const LIMITE: u32 = 300;

const BG: Color = Color::from_rgba8(18, 20, 26, 255);
const HEADER_BG: Color = Color::from_rgba8(26, 29, 37, 255);
const SEP_BG: Color = Color::from_rgba8(30, 34, 43, 255);
const CHIP_ON: Color = Color::from_rgba8(60, 96, 140, 255);
const CHIP_OFF: Color = Color::from_rgba8(38, 42, 52, 255);
const FG: Color = Color::from_rgba8(222, 226, 232, 255);
const DIM: Color = Color::from_rgba8(150, 156, 168, 255);

/// Cuánto vive un toast antes de auto-descartarse.
const TOAST_TTL: Duration = Duration::from_secs(4);
/// Cadencia del repaint que anima el shimmer del skeleton mientras carga.
const SHIMMER_MS: u64 = 50;

/// Hash estable de una cadena → `key` para animaciones implícitas (la misma
/// escena/ítem produce siempre la misma key entre rebuilds).
fn key_of(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

// ---- Lógica pura (testeable sin render) ----------------------------------

/// Glifo de la clase, para el ícono de cada fila.
fn icono_clase(c: Clase) -> &'static str {
    match c {
        Clase::Notificacion => "🔔",
        Clase::Captura => "📷",
        Clase::Clip => "📋",
    }
}

/// Rótulo corto de la clase (chip de faceta).
fn etiqueta_clase(c: Clase) -> &'static str {
    match c {
        Clase::Notificacion => "Notif",
        Clase::Captura => "Capturas",
        Clase::Clip => "Clips",
    }
}

/// Número de día local (días desde la era común, en la zona del sistema) de un
/// timestamp en µs. `chrono::Local` aplica el offset de la zona — el corte de
/// «Hoy» respeta la medianoche local, no la UTC.
fn dia_local(ts_usec: u64) -> i64 {
    use chrono::{Datelike, Local, TimeZone};
    Local
        .timestamp_micros(ts_usec as i64)
        .single()
        .map(|dt| dt.date_naive().num_days_from_ce() as i64)
        .unwrap_or(0)
}

/// Hora `HH:MM` local de un timestamp en µs.
fn hora_local(ts_usec: u64) -> String {
    use chrono::{Local, TimeZone, Timelike};
    Local
        .timestamp_micros(ts_usec as i64)
        .single()
        .map(|dt| format!("{:02}:{:02}", dt.hour(), dt.minute()))
        .unwrap_or_else(|| "--:--".to_string())
}

/// Etiqueta del separador a partir de la diferencia de **días locales**. Pura.
fn etiqueta_bucket(dia_ev: i64, dia_now: i64) -> String {
    match dia_now - dia_ev {
        d if d <= 0 => "Hoy".to_string(),
        1 => "Ayer".to_string(),
        2..=6 => "Esta semana".to_string(),
        7..=30 => "Este mes".to_string(),
        _ => "Más antiguo".to_string(),
    }
}

/// Etiqueta del separador de fecha de `ts` respecto de `now` (ambos µs), en hora
/// local.
fn bucket_fecha(ts_usec: u64, now_usec: u64) -> String {
    etiqueta_bucket(dia_local(ts_usec), dia_local(now_usec))
}

// ---- App ------------------------------------------------------------------

/// Comandos del panel hacia su hilo de red.
enum Cmd {
    /// Cambiar la faceta de clase (`None` = todas).
    Filtro(Option<Clase>),
    /// Vaciar el índice.
    Limpiar,
    /// Re-consultar ya (lo dispara el hilo de suscripción al recibir un push, o
    /// el poll de seguridad).
    Refrescar,
}

#[derive(Clone)]
enum Msg {
    /// El hilo de red trajo eventos (ya en orden reciente→viejo).
    Cargado(Vec<Evento>),
    ScrollBy(f32),
    Redimensionado(u32),
    Filtrar(Option<Clase>),
    Limpiar,
    /// Clic en un clip: copiarlo de vuelta al portapapeles.
    CopiarClip(String),
    /// Clic en una captura: abrirla en tullpu (anotar/recortar).
    AbrirCaptura(String),
    /// Tick de animación — fuerza repaint para el shimmer del skeleton mientras
    /// el feed carga su primera tanda. Se auto-rearma sólo si sigue cargando.
    Tick,
    /// Un toast cumplió su `duration`: se descarta del stack.
    ToastExpire(u64),
}

struct Panel;

struct Model {
    eventos: Vec<Evento>,
    filtro: Option<Clase>,
    offset: f32,
    viewport_h: f32,
    cmd_tx: Sender<Cmd>,
    /// Esperando la primera tanda del daemon (muestra skeleton en vez de hueco).
    loading: bool,
    /// Toasts vivos (confirmaciones de copiar/abrir).
    toasts: Vec<Toast>,
    /// Id incremental para correlacionar toast ↔ Msg de expiración.
    next_toast: u64,
    /// Hay una cadena de `Msg::Tick` en vuelo (evita rearmar dos).
    ticking: bool,
}

/// Empuja un toast al stack y programa su expiración.
fn push_toast(m: &mut Model, handle: &Handle<Msg>, toast: Toast) {
    let id = toast.id;
    m.toasts.push(toast);
    handle.spawn(move || {
        std::thread::sleep(TOAST_TTL);
        Msg::ToastExpire(id)
    });
}

/// Arranca la cadena de ticks de shimmer si seguimos cargando y no hay ya una
/// corriendo. La cadena se auto-detiene cuando `loading` baja (ver `Msg::Tick`).
fn ensure_tick(m: &mut Model, handle: &Handle<Msg>) {
    if m.ticking || !m.loading {
        return;
    }
    m.ticking = true;
    handle.spawn(move || {
        std::thread::sleep(Duration::from_millis(SHIMMER_MS));
        Msg::Tick
    });
}

/// `key` estable de la escena actual (faceta de clase). Cambia sólo al cambiar
/// de filtro → dispara la transición de entrada del cuerpo.
fn scene_key(m: &Model) -> u64 {
    match m.filtro {
        Some(c) => key_of(etiqueta_clase(c)),
        None => key_of("todo"),
    }
}

impl App for Panel {
    type Model = Model;
    type Msg = Msg;

    fn init(handle: &Handle<Msg>) -> Model {
        let cmd_tx = spawn_red(handle.clone());
        let mut model = Model {
            eventos: Vec::new(),
            filtro: None,
            offset: 0.0,
            viewport_h: 600.0,
            cmd_tx,
            loading: true,
            toasts: Vec::new(),
            next_toast: 0,
            ticking: false,
        };
        ensure_tick(&mut model, handle);
        model
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Cargado(ev) => {
                model.loading = false;
                model.eventos = ev;
                model.offset = clamp_offset(model.offset, contenido_h(&model.eventos), lista_h(&model));
            }
            Msg::ScrollBy(d) => {
                model.offset =
                    clamp_offset(model.offset + d, contenido_h(&model.eventos), lista_h(&model));
            }
            Msg::Redimensionado(h) => {
                model.viewport_h = h as f32;
                model.offset = clamp_offset(model.offset, contenido_h(&model.eventos), lista_h(&model));
            }
            Msg::Filtrar(f) => {
                model.filtro = f;
                model.offset = 0.0;
                let _ = model.cmd_tx.send(Cmd::Filtro(f));
            }
            Msg::Limpiar => {
                let _ = model.cmd_tx.send(Cmd::Limpiar);
                model.eventos.clear();
                model.offset = 0.0;
            }
            Msg::CopiarClip(t) => {
                copiar_clipboard(&t);
                let id = model.next_toast;
                model.next_toast += 1;
                push_toast(&mut model, handle, Toast::success(id, "Copiado al portapapeles", TOAST_TTL));
            }
            Msg::AbrirCaptura(ruta) => {
                abrir_en_tullpu(&ruta);
                let id = model.next_toast;
                model.next_toast += 1;
                push_toast(&mut model, handle, Toast::info(id, "Abriendo en tullpu…", TOAST_TTL));
            }
            Msg::Tick => {
                // El thread durmió; sólo rearmamos si seguimos cargando (abajo).
                model.ticking = false;
            }
            Msg::ToastExpire(id) => {
                model.toasts.retain(|t| t.id != id);
            }
        }
        // Si quedó una carga en vuelo, mantené el shimmer animado.
        ensure_tick(&mut model, handle);
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let now = ahora_usec();
        let area_h = lista_h(model);

        // Cuerpo según el estado del feed:
        //   1) cargando y vacío → lista de skeletons con shimmer
        //   2) cargado y vacío → empty-state con orientación
        //   3) datos → la lista scrolleable real
        let cuerpo: View<Msg> = if model.eventos.is_empty() && model.loading {
            skeleton_lista(area_h, &theme)
        } else if model.eventos.is_empty() {
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(area_h) },
                ..Default::default()
            })
            .children(vec![empty_view(
                Icon::Bell,
                "Sin eventos",
                Some("Cuando lleguen notificaciones, capturas o clips aparecerán acá."),
                &EmptyPalette::from_theme(&theme),
            )])
        } else {
            let (filas, alto) = construir_filas(&model.eventos, now);
            let lista = View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: percent(1.0_f32), height: length(alto) },
                ..Default::default()
            })
            .children(filas);

            scroll_y(model.offset, alto, area_h, lista, Msg::ScrollBy, &ScrollPalette::default())
        };

        // Transición de escena: al cambiar de faceta (filtro), la `scene_key`
        // cambia y el cuerpo entra con un fade + slide-up suave en vez de saltar.
        let escena = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: length(area_h) },
            ..Default::default()
        })
        .children(vec![cuerpo])
        .animated_enter_from(scene_key(model), motion::SLOW, Affine::translate((0.0, 24.0)));

        let root = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(BG)
        .children(vec![header_view(model), escena]);

        // Overlay de toasts (bottom-right del panel). El ancho es fijo (sidebar).
        let viewport = (PANEL_W as f32, model.viewport_h);
        let ahora = Instant::now();
        let vivos: Vec<Toast> = model.toasts.iter().filter(|t| t.is_alive(ahora)).cloned().collect();
        if vivos.is_empty() {
            root
        } else {
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .children(vec![root, toast_stack_view(&vivos, viewport, Msg::ToastExpire)])
        }
    }

    fn on_resize(_model: &Model, _w: u32, h: u32) -> Option<Msg> {
        Some(Msg::Redimensionado(h))
    }

    fn app_id() -> Option<&'static str> {
        Some("willay-panel")
    }

    fn title() -> &'static str {
        "willay-panel"
    }
}

/// Una fila del plan del feed: un separador de fecha, o un **grupo** de eventos
/// consecutivos idénticos (misma clase, origen y título — una ráfaga, p. ej. la
/// misma notificación repetida) que se colapsan en una sola fila con `×N`.
enum Fila<'a> {
    Sep(String),
    Grupo(&'a [Evento]),
}

/// Plan de filas: recorre los eventos (ya recientes→viejos), mete un separador
/// al cambiar de día local y colapsa cada **run** de eventos idénticos dentro del
/// mismo día. Clips y capturas tienen títulos únicos, así que en la práctica sólo
/// se colapsan notificaciones repetidas — la ráfaga ruidosa.
fn plan_filas(eventos: &[Evento], now: u64) -> Vec<Fila<'_>> {
    let mut out = Vec::new();
    let mut bucket = String::new();
    let mut i = 0;
    while i < eventos.len() {
        let e = &eventos[i];
        let b = bucket_fecha(e.ts_usec, now);
        if b != bucket {
            out.push(Fila::Sep(b.clone()));
            bucket = b.clone();
        }
        let mut j = i + 1;
        while j < eventos.len()
            && eventos[j].clase == e.clase
            && eventos[j].origen == e.origen
            && eventos[j].titulo == e.titulo
            && bucket_fecha(eventos[j].ts_usec, now) == bucket
        {
            j += 1;
        }
        out.push(Fila::Grupo(&eventos[i..j]));
        i = j;
    }
    out
}

/// Alto del contenido scrolleable, derivado del mismo plan que [`construir_filas`].
fn contenido_h(eventos: &[Evento]) -> f32 {
    plan_filas(eventos, ahora_usec())
        .iter()
        .map(|f| match f {
            Fila::Sep(_) => SEP_H,
            Fila::Grupo(_) => ITEM_H,
        })
        .sum()
}

fn lista_h(m: &Model) -> f32 {
    (m.viewport_h - HEADER_H).max(0.0)
}

/// Construye las filas del feed desde el plan: separadores de fecha y una fila
/// por grupo (ráfaga colapsada). Devuelve `(filas, alto_total)`.
fn construir_filas(eventos: &[Evento], now: u64) -> (Vec<View<Msg>>, f32) {
    let mut filas = Vec::new();
    let mut alto = 0.0;
    for f in plan_filas(eventos, now) {
        match f {
            Fila::Sep(b) => {
                filas.push(separador(&b));
                alto += SEP_H;
            }
            Fila::Grupo(g) => {
                filas.push(grupo_row(g));
                alto += ITEM_H;
            }
        }
    }
    (filas, alto)
}

fn header_view(model: &Model) -> View<Msg> {
    let n = model.eventos.len();
    let titulo = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(format!("Eventos ({n})"), 14.0, FG, Alignment::Start);

    let limpiar = View::new(Style {
        size: Size { width: length(72.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(54, 40, 40, 255))
    .radius(6.0_f64)
    .text_aligned("Limpiar".to_string(), 12.0, FG, Alignment::Center)
    .on_click(Msg::Limpiar);

    let fila_titulo = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(TITLE_H) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        padding: Rect { left: length(14.0_f32), right: length(12.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![titulo, limpiar]);

    // Chips de faceta: Todo + una por clase. El activo resaltado.
    let mut chips = vec![chip("Todo", None, model.filtro)];
    for c in [Clase::Notificacion, Clase::Captura, Clase::Clip] {
        chips.push(chip(etiqueta_clase(c), Some(c), model.filtro));
    }
    let fila_chips = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(CHIPS_H) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        padding: Rect { left: length(10.0_f32), right: length(10.0_f32), top: length(0.0_f32), bottom: length(6.0_f32) },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(chips);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(HEADER_BG)
    .children(vec![fila_titulo, fila_chips])
}

/// Un chip de faceta. `clase` = qué filtra (`None` = todas); `activo` = el filtro
/// vigente, para resaltar el chip elegido.
fn chip(rotulo: &str, clase: Option<Clase>, activo: Option<Clase>) -> View<Msg> {
    let on = clase == activo;
    View::new(Style {
        size: Size { width: length(74.0_f32), height: length(26.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(if on { CHIP_ON } else { CHIP_OFF })
    .radius(13.0_f64)
    .text_aligned(rotulo.to_string(), 11.0, if on { FG } else { DIM }, Alignment::Center)
    .on_click(Msg::Filtrar(clase))
}

fn separador(rotulo: &str) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(SEP_H) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        padding: Rect { left: length(14.0_f32), right: length(12.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .fill(SEP_BG)
    .text_aligned(rotulo.to_string(), 11.0, DIM, Alignment::Start)
}

/// Lado del thumbnail de captura en la fila (px).
const THUMB_PX: f32 = 34.0;
/// Tope de tamaño del PNG a decodificar para el thumbnail: capturas más grandes
/// no se previsualizan (memoria/CPU acotadas), sólo el ícono.
const THUMB_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// El thumbnail decodificado de una captura, cacheado por ruta. Cachea también el
/// fallo (`None`) para no reintentar decodificar cada frame. `peniko::Image` es
/// barato de clonar (su blob es `Arc`).
fn thumbnail(ruta: &str) -> Option<Image> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Image>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(slot) = cache.lock().ok().and_then(|g| g.get(ruta).cloned()) {
        return slot; // ya intentado
    }
    let img = llimphi_image::load_path(std::path::Path::new(ruta), THUMB_MAX_BYTES).ok();
    if let Ok(mut g) = cache.lock() {
        g.insert(ruta.to_string(), img.clone());
    }
    img
}

/// El thumbnail a mostrar en la fila de un evento, si lo tiene (capturas con
/// archivo en disco). El resto de las clases no muestra miniatura.
fn thumb_para(e: &Evento) -> Option<Image> {
    use willay_core::{Clase, Payload};
    match (e.clase, &e.payload) {
        (Clase::Captura, Payload::Archivo { ruta, .. }) => thumbnail(ruta),
        _ => None,
    }
}

/// La acción de un evento al clickear su fila: copiar el clip al portapapeles, o
/// abrir la captura en tullpu. Las notificaciones no tienen acción. `None` si la
/// clase/payload no acciona.
fn accion_de(e: &Evento) -> Option<Msg> {
    use willay_core::{Clase, Payload};
    match (e.clase, &e.payload) {
        (Clase::Clip, Payload::Texto(t)) => Some(Msg::CopiarClip(t.clone())),
        (Clase::Clip, _) => Some(Msg::CopiarClip(e.cuerpo.clone())),
        (Clase::Captura, Payload::Archivo { ruta, .. }) => Some(Msg::AbrirCaptura(ruta.clone())),
        _ => None,
    }
}

/// Una fila del feed para un **grupo** (ráfaga colapsada): el evento más reciente
/// del run + un badge `×N` si N > 1. Clickearla dispara la acción del evento
/// (copiar clip / abrir captura).
fn grupo_row(g: &[Evento]) -> View<Msg> {
    let e = &g[0]; // el más reciente del run
    let n = g.len();

    let ico = View::new(Style {
        size: Size { width: length(20.0_f32), height: length(15.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(icono_clase(e.clase).to_string(), 11.0, DIM, Alignment::Start);

    let origen_txt = if n > 1 { format!("{} ×{n}", e.origen) } else { e.origen.clone() };
    let origen = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: length(15.0_f32) },
        ..Default::default()
    })
    .text_aligned(origen_txt, 10.0, DIM, Alignment::Start);

    let cuando = View::new(Style {
        size: Size { width: length(40.0_f32), height: length(15.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(hora_local(e.ts_usec), 10.0, DIM, Alignment::End);

    let linea_meta = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(15.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![ico, origen, cuando]);

    let titulo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(e.titulo.clone(), 12.0, FG, Alignment::Start);

    // Columna de texto (meta + título), que crece; opcionalmente con un thumbnail
    // de captura a la izquierda.
    let texto = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .children(vec![linea_meta, titulo]);

    let mut kids: Vec<View<Msg>> = Vec::new();
    if let Some(img) = thumb_para(e) {
        kids.push(
            View::new(Style {
                size: Size { width: length(THUMB_PX), height: length(THUMB_PX) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .radius(4.0_f64)
            .image(img)
            .image_fit(ImageFit::Cover),
        );
    }
    kids.push(texto);

    let fila = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(ITEM_H) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect { left: length(12.0_f32), right: length(12.0_f32), top: length(5.0_f32), bottom: length(5.0_f32) },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(kids);

    // Pop-in: cada grupo entra con un fade la primera vez que aparece su key
    // (estable por timestamp + título del evento más reciente del run).
    let key = key_of(&format!("{}-{}", e.ts_usec, e.titulo));
    let fila = fila.animated_enter(key, motion::NORMAL);

    match accion_de(e) {
        Some(msg) => fila.on_click(msg),
        None => fila,
    }
}

/// Lista de placeholders con shimmer mientras llega la primera tanda del daemon
/// — el usuario ve la forma del feed, no un hueco vacío. Requiere repaints
/// periódicos (la cadena de `Msg::Tick` mientras `loading`).
fn skeleton_lista(area_h: f32, theme: &Theme) -> View<Msg> {
    let pal = SkeletonPalette::from_theme(theme);
    let filas = ((area_h / ITEM_H).ceil() as usize).clamp(1, 12);
    // Un placeholder lleno y clipeado para que el `skeleton_view` (absolute)
    // pinte dentro de su caja.
    let bloque = |w: f32, h: f32| -> View<Msg> {
        View::new(Style {
            size: Size { width: length(w), height: length(h) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .radius(4.0_f64)
        .clip(true)
        .children(vec![skeleton_view::<Msg>(&pal)])
    };
    let rows: Vec<View<Msg>> = (0..filas)
        .map(|_| {
            let ico = bloque(THUMB_PX, THUMB_PX);
            let l1 = bloque(120.0_f32, 10.0);
            let l2 = bloque(220.0_f32, 12.0);
            let texto = View::new(Style {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .children(vec![l1, l2]);
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(ITEM_H) },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                padding: Rect {
                    left: length(12.0_f32),
                    right: length(12.0_f32),
                    top: length(5.0_f32),
                    bottom: length(5.0_f32),
                },
                gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
                ..Default::default()
            })
            .children(vec![ico, texto])
        })
        .collect();
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(area_h) },
        ..Default::default()
    })
    .clip(true)
    .children(rows)
}

/// Hilo de red: pollea el daemon por socket y reacciona a los comandos del panel
/// (cambio de faceta, limpiar). Sin tokio ni D-Bus — el socket es bloqueante y
/// `recv_timeout` da el intervalo. Reconecta solo si el daemon no estaba arriba.
fn spawn_red(handle: Handle<Msg>) -> Sender<Cmd> {
    let (tx, rx) = channel::<Cmd>();
    std::thread::Builder::new()
        .name("willay-panel-net".into())
        .spawn(move || {
            let mut filtro: Option<Clase> = None;
            let mut emisor: Option<Emisor> = None;
            loop {
                if emisor.is_none() {
                    emisor = Emisor::conectar().ok();
                }
                if let Some(em) = emisor.as_mut() {
                    let sol = match filtro {
                        Some(c) => Solicitud::PorClase(c, LIMITE),
                        None => Solicitud::Recientes(LIMITE),
                    };
                    match em.pedir(&sol) {
                        Ok(Respuesta::Eventos(v)) => handle.dispatch(Msg::Cargado(v)),
                        Ok(_) => {}
                        Err(_) => emisor = None, // se cayó; reconecta en el próximo ciclo
                    }
                }
                match rx.recv_timeout(POLL) {
                    Ok(Cmd::Filtro(f)) => filtro = f,
                    Ok(Cmd::Limpiar) => {
                        if let Some(em) = emisor.as_mut() {
                            let _ = em.pedir(&Solicitud::Limpiar);
                        }
                    }
                    // Re-consulta ya (cae al inicio del loop). El push o el poll.
                    Ok(Cmd::Refrescar) => {}
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("hilo de red del panel willay");

    // Hilo de suscripción: se cuelga del push del daemon y, en cada cambio del
    // índice, pide un refresco inmediato al hilo de red. Si la conexión cae,
    // reintenta. Es lo que vuelve el feed instantáneo (el poll queda de respaldo).
    let sub_tx = tx.clone();
    std::thread::Builder::new()
        .name("willay-panel-sub".into())
        .spawn(move || loop {
            if let Ok(em) = Emisor::conectar() {
                let cmd = sub_tx.clone();
                let _ = em.escuchar_cambios(|| {
                    let _ = cmd.send(Cmd::Refrescar);
                });
            }
            // Conexión caída (o daemon ausente): esperar y reintentar.
            std::thread::sleep(Duration::from_secs(2));
        })
        .expect("hilo de suscripción del panel willay");

    tx
}

/// Copia `text` al portapapeles vía `wl-copy` (wl-clipboard), como pata. No
/// espera: `wl-copy` se daemoniza para mantener la selección.
fn copiar_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(text.as_bytes());
        }
    }
}

/// Abre una captura (ruta del PNG) en tullpu para anotar/recortar — el mismo
/// handoff que hace hapiy.
fn abrir_en_tullpu(ruta: &str) {
    let _ = std::process::Command::new("tullpu-app-llimphi").arg(ruta).spawn();
}

fn main() {
    let cfg = llimphi_layer::LayerConfig {
        edge: llimphi_layer::Edge::Right,
        thickness: PANEL_W,
        layer: llimphi_layer::LayerKind::Top,
        exclusive: true,
        keyboard: llimphi_layer::Keyboard::OnDemand,
        namespace: "willay-panel".to_string(),
        ..Default::default()
    };
    if let Err(e) = llimphi_layer::run::<Panel>(cfg) {
        eprintln!("willay-panel · sin wlr-layer-shell: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willay_core::Payload;

    const USEC_DIA: u64 = 86_400_000_000;
    /// Mediodía UTC: ± cualquier offset de zona razonable (|tz| < 12 h) cae el
    /// mismo día local, así los tests de separadores son robustos a la zona.
    const MEDIODIA: u64 = 12 * 3600 * 1_000_000;

    fn ev(clase: Clase, ts: u64) -> Evento {
        Evento::nuevo(clase, ts, "o", "t", "", Payload::Nada)
    }

    #[test]
    fn etiqueta_bucket_por_diferencia_de_dias() {
        assert_eq!(etiqueta_bucket(100, 100), "Hoy");
        assert_eq!(etiqueta_bucket(100, 99), "Hoy"); // futuro/empate → Hoy
        assert_eq!(etiqueta_bucket(99, 100), "Ayer");
        assert_eq!(etiqueta_bucket(96, 100), "Esta semana");
        assert_eq!(etiqueta_bucket(80, 100), "Este mes");
        assert_eq!(etiqueta_bucket(10, 100), "Más antiguo");
    }

    #[test]
    fn hora_local_formatea_hh_mm() {
        let h = hora_local(0);
        assert_eq!(h.len(), 5);
        assert_eq!(h.as_bytes()[2], b':');
    }

    #[test]
    fn filas_meten_un_separador_por_cambio_de_dia() {
        let now = 100 * USEC_DIA + MEDIODIA;
        // Dos eventos hoy + uno ayer → 2 separadores ("Hoy", "Ayer") + 3 items.
        let eventos = vec![
            ev(Clase::Clip, 100 * USEC_DIA + MEDIODIA + 2),
            ev(Clase::Captura, 100 * USEC_DIA + MEDIODIA + 1),
            ev(Clase::Notificacion, 99 * USEC_DIA + MEDIODIA),
        ];
        let (filas, alto) = construir_filas(&eventos, now);
        assert_eq!(filas.len(), 5, "3 items + 2 separadores");
        assert_eq!(alto, 3.0 * ITEM_H + 2.0 * SEP_H);
    }

    #[test]
    fn lista_vacia_no_tiene_filas() {
        let (filas, alto) = construir_filas(&[], 100 * USEC_DIA);
        assert!(filas.is_empty());
        assert_eq!(alto, 0.0);
    }

    #[test]
    fn rafaga_de_notifs_identicas_colapsa() {
        let now = 100 * USEC_DIA + MEDIODIA;
        let notif = |ts| {
            Evento::nuevo(Clase::Notificacion, ts, "Firefox", "Descarga lista", "", Payload::Nada)
        };
        // 3 notifs idénticas + 1 clip distinto, mismo día.
        let eventos = vec![notif(now + 3), notif(now + 2), notif(now + 1), ev(Clase::Clip, now)];
        let plan = plan_filas(&eventos, now);
        // Sep("Hoy") + Grupo(3 notifs) + Grupo(1 clip).
        let tams: Vec<usize> = plan
            .iter()
            .filter_map(|f| match f {
                Fila::Grupo(g) => Some(g.len()),
                Fila::Sep(_) => None,
            })
            .collect();
        assert_eq!(tams, vec![3, 1]);
    }

    #[test]
    fn accion_por_clase() {
        let clip = Evento::nuevo(
            Clase::Clip,
            1,
            "o",
            "git push",
            "git push origin main",
            Payload::Texto("git push origin main".into()),
        );
        assert!(matches!(accion_de(&clip), Some(Msg::CopiarClip(t)) if t == "git push origin main"));
        let cap = Evento::nuevo(
            Clase::Captura,
            1,
            "hapiy",
            "Captura",
            "/p/x.png",
            Payload::Archivo { ruta: "/p/x.png".into(), mime: "image/png".into() },
        );
        assert!(matches!(accion_de(&cap), Some(Msg::AbrirCaptura(r)) if r == "/p/x.png"));
        let notif = Evento::nuevo(Clase::Notificacion, 1, "x", "y", "z", Payload::Nada);
        assert!(accion_de(&notif).is_none());
    }
}
