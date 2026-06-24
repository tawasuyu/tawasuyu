//! El panel de historial: un sidebar dockeado a la derecha que lista las
//! notificaciones recibidas. Cliente del daemon — consulta el historial por
//! D-Bus (el daemon es dueño del lock de sled) y refresca por polling.
//!
//! Es el sustrato visual sobre el que después se monta el triage: la misma
//! lista, pero agrupada/priorizada por la capa semántica.

use std::sync::mpsc::Sender;
use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, Rect};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};

use crate::dbus;
use crate::Notificacion;

/// Ancho del sidebar (px).
const PANEL_W: u32 = 360;
/// Alto de cada fila del historial (px).
const ROW_H: f32 = 72.0;
/// Alto de la cabecera (título + botón limpiar).
const HEADER_H: f32 = 48.0;
/// Cada cuánto se re-consulta el historial al daemon.
const POLL: Duration = Duration::from_secs(2);

const BG: Color = Color::from_rgba8(18, 20, 26, 255);
const HEADER_BG: Color = Color::from_rgba8(26, 29, 37, 255);
const FG: Color = Color::from_rgba8(222, 226, 232, 255);
const DIM: Color = Color::from_rgba8(150, 156, 168, 255);
const ROW_BG: Color = Color::from_rgba8(26, 29, 37, 255);

/// Color del rail de severidad por urgencia.
fn rail_color(urgency: u8) -> Color {
    match urgency {
        2 => Color::from_rgba8(220, 90, 90, 255),   // crítica
        0 => Color::from_rgba8(110, 120, 135, 255),  // baja
        _ => Color::from_rgba8(110, 160, 210, 255),  // normal
    }
}

/// Comandos del panel hacia su hilo de red.
enum Cmd {
    Limpiar,
}

#[derive(Clone)]
pub enum Msg {
    /// El hilo de red trajo el historial (ya ordenado más-nuevo-primero).
    Cargado(Vec<Notificacion>),
    /// Delta de scroll en px (de rueda o barra).
    ScrollBy(f32),
    /// El compositor informó el alto real del sidebar.
    Redimensionado(u32),
    /// El usuario pidió vaciar el historial.
    Limpiar,
}

pub struct Panel;

pub struct Model {
    items: Vec<Notificacion>,
    offset: f32,
    viewport_h: f32,
    cmd_tx: Sender<Cmd>,
}

impl App for Panel {
    type Model = Model;
    type Msg = Msg;

    fn init(handle: &Handle<Msg>) -> Model {
        let cmd_tx = spawn_red(handle.clone());
        Model {
            items: Vec::new(),
            offset: 0.0,
            viewport_h: 600.0,
            cmd_tx,
        }
    }

    fn update(mut model: Model, msg: Msg, _handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Cargado(items) => {
                model.items = items;
                model.offset = clamp_offset(model.offset, contenido_h(&model), lista_viewport(&model));
            }
            Msg::ScrollBy(d) => {
                model.offset =
                    clamp_offset(model.offset + d, contenido_h(&model), lista_viewport(&model));
            }
            Msg::Redimensionado(h) => {
                model.viewport_h = h as f32;
                model.offset = clamp_offset(model.offset, contenido_h(&model), lista_viewport(&model));
            }
            Msg::Limpiar => {
                let _ = model.cmd_tx.send(Cmd::Limpiar);
                model.items.clear();
                model.offset = 0.0;
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let filas: Vec<View<Msg>> = model.items.iter().map(fila_view).collect();
        let lista = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: length(contenido_h(model)),
            },
            ..Default::default()
        })
        .children(filas);

        let scroller = scroll_y(
            model.offset,
            contenido_h(model),
            lista_viewport(model),
            lista,
            Msg::ScrollBy,
            &ScrollPalette::default(),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(BG)
        .children(vec![header_view(model), scroller])
    }

    fn on_resize(_model: &Model, _w: u32, h: u32) -> Option<Msg> {
        Some(Msg::Redimensionado(h))
    }

    fn app_id() -> Option<&'static str> {
        Some("pata-notify-panel")
    }

    fn title() -> &'static str {
        "pata-notify-panel"
    }
}

/// Alto total del contenido scrolleable (una fila por notificación).
fn contenido_h(m: &Model) -> f32 {
    m.items.len() as f32 * ROW_H
}

/// Alto visible de la lista = panel menos la cabecera.
fn lista_viewport(m: &Model) -> f32 {
    (m.viewport_h - HEADER_H).max(0.0)
}

fn header_view(m: &Model) -> View<Msg> {
    let titulo = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("Notificaciones ({})", m.items.len()),
        14.0,
        FG,
        Alignment::Start,
    );

    let limpiar = View::new(Style {
        size: Size { width: length(72.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(54, 40, 40, 255))
    .radius(6.0)
    .text_aligned("Limpiar".to_string(), 12.0, FG, Alignment::Center)
    .on_click(Msg::Limpiar);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(HEADER_BG)
    .children(vec![titulo, limpiar])
}

fn fila_view(n: &Notificacion) -> View<Msg> {
    let rail = View::new(Style {
        size: Size { width: length(3.0_f32), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(rail_color(n.urgency));

    let app = if n.app_name.trim().is_empty() {
        "—".to_string()
    } else {
        n.app_name.clone()
    };
    let prefijo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(app, 11.0, DIM, Alignment::Start);

    let summary = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(n.summary.clone(), 13.0, FG, Alignment::Start);

    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(recortar(&n.body, 64), 11.0, DIM, Alignment::Start);

    let texto = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .children(vec![prefijo, summary, body]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        padding: Rect {
            left: length(0.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(ROW_BG)
    .children(vec![rail, texto])
}

/// Recorta un texto a `max` chars con elipsis (en la primera línea).
fn recortar(s: &str, max: usize) -> String {
    let una_linea = s.replace('\n', " ");
    if una_linea.chars().count() <= max {
        una_linea
    } else {
        let corto: String = una_linea.chars().take(max.saturating_sub(1)).collect();
        format!("{corto}…")
    }
}

/// Hilo de red: un runtime tokio que sondea el historial del daemon y atiende
/// los comandos del panel (limpiar). Devuelve el extremo de envío de comandos.
fn spawn_red(handle: Handle<Msg>) -> Sender<Cmd> {
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    std::thread::Builder::new()
        .name("pata-notify-panel-net".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("pata-notify-panel · sin runtime tokio: {e}");
                    return;
                }
            };
            rt.block_on(async move {
                loop {
                    while let Ok(cmd) = rx.try_recv() {
                        match cmd {
                            Cmd::Limpiar => {
                                if let Err(e) = dbus::limpiar_historial().await {
                                    tracing::warn!(?e, "limpiar historial falló");
                                }
                            }
                        }
                    }
                    match dbus::fetch_historial().await {
                        Ok(mut items) => {
                            items.reverse(); // más nuevo primero
                            handle.dispatch(Msg::Cargado(items));
                        }
                        Err(e) => tracing::warn!(?e, "fetch historial falló (¿daemon caído?)"),
                    }
                    tokio::time::sleep(POLL).await;
                }
            });
        })
        .expect("hilo de red del panel");
    tx
}

/// Levanta el panel como sidebar dockeado a la derecha.
pub fn run() {
    let cfg = llimphi_layer::LayerConfig {
        edge: llimphi_layer::Edge::Right,
        thickness: PANEL_W,
        layer: llimphi_layer::LayerKind::Top,
        exclusive: true,
        keyboard: llimphi_layer::Keyboard::OnDemand,
        namespace: "pata-notify-panel".to_string(),
        ..Default::default()
    };
    if let Err(e) = llimphi_layer::run::<Panel>(cfg) {
        eprintln!("pata-notify-panel · sin wlr-layer-shell: {e}");
    }
}
