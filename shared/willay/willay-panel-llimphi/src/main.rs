//! `willay-panel` — el **feed** del centro de eventos.
//!
//! Un sidebar wlr-layer-shell que lista el timeline heterogéneo (notificaciones,
//! capturas, clips) más reciente arriba, con separadores de fecha y facetas por
//! clase (chips Todo/Notif/Capturas/Clips). Es cliente del daemon willay: un
//! hilo pollea el socket (`Recientes`/`PorClase`) y despacha al render. Generaliza
//! `pata-notify-panel` (mismo patrón App+scroll), pero sobre el socket propio.

use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, Rect};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};

use willay_core::proto::{Respuesta, Solicitud};
use willay_core::{Clase, Evento};
use willay_emit::{ahora_usec, Emisor};

const PANEL_W: u32 = 380;
const ITEM_H: f32 = 46.0;
const SEP_H: f32 = 28.0;
const TITLE_H: f32 = 44.0;
const CHIPS_H: f32 = 40.0;
const HEADER_H: f32 = TITLE_H + CHIPS_H;
/// Cada cuánto el hilo de red re-consulta el índice (no hay señal de cambio).
const POLL: Duration = Duration::from_millis(1500);
/// Tope de eventos que pide el feed.
const LIMITE: u32 = 300;

const BG: Color = Color::from_rgba8(18, 20, 26, 255);
const HEADER_BG: Color = Color::from_rgba8(26, 29, 37, 255);
const SEP_BG: Color = Color::from_rgba8(30, 34, 43, 255);
const CHIP_ON: Color = Color::from_rgba8(60, 96, 140, 255);
const CHIP_OFF: Color = Color::from_rgba8(38, 42, 52, 255);
const FG: Color = Color::from_rgba8(222, 226, 232, 255);
const DIM: Color = Color::from_rgba8(150, 156, 168, 255);

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
}

#[derive(Clone)]
enum Msg {
    /// El hilo de red trajo eventos (ya en orden reciente→viejo).
    Cargado(Vec<Evento>),
    ScrollBy(f32),
    Redimensionado(u32),
    Filtrar(Option<Clase>),
    Limpiar,
}

struct Panel;

struct Model {
    eventos: Vec<Evento>,
    filtro: Option<Clase>,
    offset: f32,
    viewport_h: f32,
    cmd_tx: Sender<Cmd>,
}

impl App for Panel {
    type Model = Model;
    type Msg = Msg;

    fn init(handle: &Handle<Msg>) -> Model {
        let cmd_tx = spawn_red(handle.clone());
        Model { eventos: Vec::new(), filtro: None, offset: 0.0, viewport_h: 600.0, cmd_tx }
    }

    fn update(mut model: Model, msg: Msg, _handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Cargado(ev) => {
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
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let now = ahora_usec();
        let (filas, alto) = construir_filas(&model.eventos, now);
        let lista = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: length(alto) },
            ..Default::default()
        })
        .children(filas);

        let scroller = scroll_y(
            model.offset,
            alto,
            lista_h(model),
            lista,
            Msg::ScrollBy,
            &ScrollPalette::default(),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(BG)
        .children(vec![header_view(model), scroller])
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

/// Alto del contenido scrolleable: una fila por evento + un separador por cada
/// cambio de bucket de fecha. Debe coincidir con [`construir_filas`].
fn contenido_h(eventos: &[Evento]) -> f32 {
    let now = ahora_usec();
    let mut h = 0.0;
    let mut bucket = String::new();
    for e in eventos {
        let b = bucket_fecha(e.ts_usec, now);
        if b != bucket {
            h += SEP_H;
            bucket = b;
        }
        h += ITEM_H;
    }
    h
}

fn lista_h(m: &Model) -> f32 {
    (m.viewport_h - HEADER_H).max(0.0)
}

/// Construye las filas del feed: separador de fecha cada vez que cambia el día,
/// y una fila por evento. Devuelve `(filas, alto_total)`.
fn construir_filas(eventos: &[Evento], now: u64) -> (Vec<View<Msg>>, f32) {
    let mut filas = Vec::new();
    let mut alto = 0.0;
    let mut bucket = String::new();
    for e in eventos {
        let b = bucket_fecha(e.ts_usec, now);
        if b != bucket {
            filas.push(separador(&b));
            alto += SEP_H;
            bucket = b;
        }
        filas.push(item_row(e));
        alto += ITEM_H;
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

fn item_row(e: &Evento) -> View<Msg> {
    // Línea superior: ícono + origen + hora a la derecha.
    let ico = View::new(Style {
        size: Size { width: length(20.0_f32), height: length(15.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(icono_clase(e.clase).to_string(), 11.0, DIM, Alignment::Start);

    let origen = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: length(15.0_f32) },
        ..Default::default()
    })
    .text_aligned(e.origen.clone(), 10.0, DIM, Alignment::Start);

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

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(ITEM_H) },
        flex_shrink: 0.0,
        padding: Rect { left: length(12.0_f32), right: length(12.0_f32), top: length(5.0_f32), bottom: length(5.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .children(vec![linea_meta, titulo])
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
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("hilo de red del panel willay");
    tx
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
}
