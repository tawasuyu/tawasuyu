//! `hapiy-llimphi` — la **GUI** de captura de la suite (el "Spectacle").
//!
//! Capturá la pantalla (con retardo opcional), recortá una **región** marcando
//! dos esquinas sobre el preview, y elegí qué hacer: **Guardar** PNG, **Copiar**
//! al portapapeles, o **Editar en tullpu** para anotar. Sobre `hapiy-core`
//! (modelo/encode/handoff) + `hapiy-capture` (backends).
//!
//! Nota: la captura corre en el hilo de UI (one-shot). El retardo (Capturar 3 s)
//! da tiempo a acomodar la pantalla; aun así la ventana de hapiy puede aparecer
//! en la toma — recortala con la región o en tullpu.

use hapiy_capture::{capturer, Backend};
use hapiy_core::{default_dir, default_filename, tullpu_launch, Capturer, OutputInfo, Region, Shot};
use llimphi_image::from_rgba8;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style,
};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_raster::peniko::{Color, ImageBrush as Image};
use llimphi_ui::{App, Handle, View};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BG: Color = Color::from_rgb8(0x0E, 0x10, 0x16);
const PANEL: Color = Color::from_rgb8(0x16, 0x1A, 0x24);
const BTN: Color = Color::from_rgb8(0x24, 0x2A, 0x38);
const ACCENT: Color = Color::from_rgb8(0x6E, 0x8C, 0xDC);
const FG: Color = Color::from_rgb8(0xD6, 0xDE, 0xE8);
const MUTED: Color = Color::from_rgb8(0x8C, 0x98, 0xAA);

#[derive(Clone)]
enum Msg {
    /// Botón Capturar: minimiza la ventana y agenda [`Msg::DoCapture`].
    Capture,
    /// Captura efectiva (tras el retardo de ocultamiento) + restaura la ventana.
    DoCapture,
    /// Capturar con retardo de staging: agenda [`Msg::Capture`] en N s.
    CaptureDelayed,
    Save,
    Copy,
    Edit,
    Clear,
    ToggleSelect,
    /// Click en el preview: `(local_x, local_y, rect_w, rect_h)` (px del nodo).
    PreviewClick(f32, f32, f32, f32),
    /// Cursor moviéndose sobre el preview: `(local_x, local_y)` (px del nodo).
    PointerAt(f32, f32),
    /// Qué capturar: `None` = todo el escritorio, `Some(i)` = ese monitor.
    SelectOutput(Option<usize>),
}

struct Model {
    cap: Box<dyn Capturer>,
    clip: Option<arboard::Clipboard>,
    outputs: Vec<OutputInfo>,
    /// Qué capturar: `None` = todo el escritorio (default), `Some(i)` = un monitor.
    sel: Option<usize>,
    shot: Option<Shot>,
    preview: Option<Image>,
    /// Modo de selección de región activo.
    select_mode: bool,
    /// Primera esquina marcada, en px del nodo del preview.
    corner_node: Option<(f32, f32)>,
    /// Última posición del cursor sobre el preview (px del nodo) — para el
    /// rectángulo de selección en vivo.
    cursor_node: Option<(f32, f32)>,
    status: String,
}

/// Segundos de retardo de **staging** (Capturar 3s) y de **ocultamiento** de la
/// ventana antes del disparo, respectivamente.
const STAGING_SECS: u64 = 3;
const HIDE_MS: u64 = 400;

struct Hapiy;

impl App for Hapiy {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "hapiy · captura"
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        let cap = capturer(Backend::Auto).unwrap_or_else(|_| capturer(Backend::Grim).unwrap());
        let outputs = cap.outputs().unwrap_or_default();
        Model {
            cap,
            clip: arboard::Clipboard::new().ok(),
            outputs,
            sel: None,
            shot: None,
            preview: None,
            select_mode: false,
            corner_node: None,
            cursor_node: None,
            status: "Pulsá Capturar.".into(),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Capture => {
                // Apartarse de la toma: minimizar, esperar a que el compositor
                // desmapee, capturar y restaurar (en DoCapture).
                handle.set_minimized(true);
                handle.spawn(move || {
                    std::thread::sleep(Duration::from_millis(HIDE_MS));
                    Msg::DoCapture
                });
                model.status = "Capturando…".into();
            }
            Msg::DoCapture => {
                capture(&mut model);
                handle.set_minimized(false);
            }
            Msg::CaptureDelayed => {
                handle.spawn(move || {
                    std::thread::sleep(Duration::from_secs(STAGING_SECS));
                    Msg::Capture
                });
                model.status = format!("Capturando en {STAGING_SECS} s…");
            }
            Msg::Save => match &model.shot {
                Some(s) => {
                    let p = default_dir().join(default_filename(&stamp()));
                    model.status = match s.save_png(&p) {
                        Ok(()) => {
                            // Emitir al centro de eventos (no-op si willay no corre).
                            let ev = hapiy_core::evento_captura(
                                &p, None, None, s.width, s.height, willay_emit::ahora_usec(),
                            );
                            willay_emit::emitir_silencioso(&ev);
                            format!("Guardado en {}", p.display())
                        }
                        Err(e) => e,
                    };
                }
                None => model.status = "Capturá primero.".into(),
            },
            Msg::Copy => match (&mut model.clip, &model.shot) {
                (Some(clip), Some(s)) => {
                    let img = arboard::ImageData {
                        width: s.width as usize,
                        height: s.height as usize,
                        bytes: s.rgba.clone().into(),
                    };
                    model.status = match clip.set_image(img) {
                        Ok(()) => "Copiado al portapapeles.".into(),
                        Err(e) => format!("No se pudo copiar: {e}"),
                    };
                }
                (None, _) => model.status = "Portapapeles no disponible.".into(),
                (_, None) => model.status = "Capturá primero.".into(),
            },
            Msg::Edit => match &model.shot {
                Some(s) => {
                    let p = std::env::temp_dir().join(default_filename(&stamp()));
                    model.status = match s.save_png(&p).and_then(|()| launch_tullpu(&p)) {
                        Ok(()) => format!("Abriendo en tullpu: {}", p.display()),
                        Err(e) => e,
                    };
                }
                None => model.status = "Capturá primero.".into(),
            },
            Msg::Clear => {
                model.shot = None;
                model.preview = None;
                model.select_mode = false;
                model.corner_node = None;
                model.cursor_node = None;
                model.status = "Pulsá Capturar.".into();
            }
            Msg::ToggleSelect => {
                if model.shot.is_some() {
                    model.select_mode = !model.select_mode;
                    model.corner_node = None;
                    model.cursor_node = None;
                    model.status = if model.select_mode {
                        "Región: clic una esquina y luego la opuesta.".into()
                    } else {
                        "Selección cancelada.".into()
                    };
                } else {
                    model.status = "Capturá primero.".into();
                }
            }
            Msg::PointerAt(lx, ly) => {
                if model.select_mode {
                    model.cursor_node = Some((lx, ly));
                }
            }
            Msg::PreviewClick(lx, ly, rw, rh) => {
                if model.select_mode {
                    if let Some(s) = &model.shot {
                        match model.corner_node.take() {
                            None => {
                                model.corner_node = Some((lx, ly));
                                model.cursor_node = Some((lx, ly));
                                model.status = "Esquina 1 fijada — clic la opuesta.".into();
                            }
                            Some(a) => {
                                let pa = to_image_px(a.0, a.1, rw, rh, s.width, s.height);
                                let pb = to_image_px(lx, ly, rw, rh, s.width, s.height);
                                match s.crop(region_between(pa, pb)) {
                                    Some(c) => {
                                        model.preview = Some(from_rgba8(c.rgba.clone(), c.width, c.height));
                                        model.status = format!("Recortado a {}×{}.", c.width, c.height);
                                        model.shot = Some(c);
                                    }
                                    None => model.status = "Región vacía; probá de nuevo.".into(),
                                }
                                model.select_mode = false;
                                model.cursor_node = None;
                            }
                        }
                    }
                }
            }
            Msg::SelectOutput(i) => model.sel = i,
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let mut toolbar = vec![
            boton("⛶ Capturar", BG, ACCENT, Msg::Capture),
            boton(&format!("⏱ Capturar {STAGING_SECS}s"), FG, BTN, Msg::CaptureDelayed),
            boton(
                if model.select_mode { "✂ Cancelar" } else { "✂ Región" },
                if model.select_mode { BG } else { FG },
                if model.select_mode { ACCENT } else { BTN },
                Msg::ToggleSelect,
            ),
            boton("💾 Guardar", FG, BTN, Msg::Save),
            boton("📋 Copiar", FG, BTN, Msg::Copy),
            boton("✎ Editar en tullpu", FG, BTN, Msg::Edit),
            boton("🗑 Limpiar", MUTED, BTN, Msg::Clear),
        ];
        if model.outputs.len() > 1 {
            let todas = model.sel.is_none();
            toolbar.push(boton(
                "🖥 Todas",
                if todas { BG } else { MUTED },
                if todas { ACCENT } else { PANEL },
                Msg::SelectOutput(None),
            ));
            for (i, o) in model.outputs.iter().enumerate() {
                let activo = model.sel == Some(i);
                toolbar.push(boton(
                    &o.name,
                    if activo { BG } else { MUTED },
                    if activo { ACCENT } else { PANEL },
                    Msg::SelectOutput(Some(i)),
                ));
            }
        }

        let toolbar = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0), height: length(60.0) },
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(8.0), height: length(0.0) },
            padding: pad(10.0),
            ..Default::default()
        })
        .fill(PANEL)
        .children(toolbar);

        let lienzo = match &model.preview {
            Some(img) => {
                let mut imagen = View::new(Style {
                    size: Size { width: percent(1.0), height: percent(1.0) },
                    flex_grow: 1.0,
                    ..Default::default()
                })
                .image(img.clone());
                if model.select_mode {
                    imagen = imagen
                        .on_click_at(|lx, ly, rw, rh| Some(Msg::PreviewClick(lx, ly, rw, rh)))
                        .on_pointer_move_at(|lx, ly, _rw, _rh| Some(Msg::PointerAt(lx, ly)));
                }
                // Container relativo: la imagen llena, y el rectángulo de
                // selección se posiciona absoluto sobre ella (mismas coords de nodo).
                let mut hijos = vec![imagen];
                if model.select_mode {
                    if let (Some(a), Some(c)) = (model.corner_node, model.cursor_node) {
                        hijos.push(marquee(a, c));
                    }
                }
                View::new(Style {
                    position: Position::Relative,
                    size: Size { width: percent(1.0), height: percent(1.0) },
                    flex_grow: 1.0,
                    ..Default::default()
                })
                .children(hijos)
            }
            None => View::new(Style {
                size: Size { width: percent(1.0), height: percent(1.0) },
                flex_grow: 1.0,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text("Sin captura todavía", 20.0, MUTED),
        };

        let status = View::new(Style {
            size: Size { width: percent(1.0), height: length(36.0) },
            align_items: Some(AlignItems::Center),
            padding: pad(10.0),
            ..Default::default()
        })
        .fill(PANEL)
        .text(model.status.clone(), 14.0, MUTED);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0), height: percent(1.0) },
            ..Default::default()
        })
        .fill(BG)
        .children(vec![toolbar, lienzo, status])
    }
}

/// Cuerpo de la captura: usa la salida seleccionada (o la primera) y refresca el
/// preview. Resetea la selección.
fn capture(model: &mut Model) {
    let out = model.sel.and_then(|i| model.outputs.get(i)).map(|o| o.name.clone());
    match model.cap.capture(out.as_deref()) {
        Ok(s) => {
            model.preview = Some(from_rgba8(s.rgba.clone(), s.width, s.height));
            model.status = format!("Captura {}×{} — guardá, copiá o editá.", s.width, s.height);
            model.shot = Some(s);
        }
        Err(e) => model.status = format!("Error al capturar: {e}"),
    }
    model.select_mode = false;
    model.corner_node = None;
    model.cursor_node = None;
}

/// Mapea coords locales del nodo (con la imagen en `Contain`/letterbox) a píxeles
/// de la imagen.
fn to_image_px(lx: f32, ly: f32, rw: f32, rh: f32, iw: u32, ih: u32) -> (u32, u32) {
    let (iw_f, ih_f) = (iw as f32, ih as f32);
    let scale = (rw / iw_f).min(rh / ih_f).max(f32::EPSILON);
    let off_x = (rw - iw_f * scale) / 2.0;
    let off_y = (rh - ih_f * scale) / 2.0;
    let x = ((lx - off_x) / scale).clamp(0.0, iw_f - 1.0);
    let y = ((ly - off_y) / scale).clamp(0.0, ih_f - 1.0);
    (x as u32, y as u32)
}

/// Rectángulo entre dos esquinas (px imagen), normalizado.
fn region_between(a: (u32, u32), b: (u32, u32)) -> Region {
    let x = a.0.min(b.0);
    let y = a.1.min(b.1);
    let w = a.0.max(b.0).saturating_sub(x);
    let h = a.1.max(b.1).saturating_sub(y);
    Region { x, y, w, h }
}

/// Rectángulo de selección en vivo: posicionado absoluto entre dos puntos en px
/// del nodo del preview (borde acento + relleno translúcido).
fn marquee(a: (f32, f32), c: (f32, f32)) -> View<Msg> {
    let x = a.0.min(c.0);
    let y = a.1.min(c.1);
    let w = (a.0 - c.0).abs();
    let h = (a.1 - c.1).abs();
    View::new(Style {
        position: Position::Absolute,
        inset: Rect { left: length(x), top: length(y), right: auto(), bottom: auto() },
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(0x6E, 0x8C, 0xDC, 40))
    .border(2.0, ACCENT)
}

fn boton(label: &str, fg: Color, bg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(150.0), height: length(40.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(8.0)
    .text(label, 14.0, fg)
    .on_click(msg)
}

fn pad(p: f32) -> llimphi_ui::llimphi_layout::taffy::Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    let v = length(p);
    llimphi_ui::llimphi_layout::taffy::Rect { left: v, right: v, top: v, bottom: v }
}

fn launch_tullpu(path: &std::path::Path) -> Result<(), String> {
    let (prog, args) = tullpu_launch(path);
    Command::new(&prog)
        .args(&args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("no se pudo abrir tullpu ({prog}): {e}"))
}

fn stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

fn main() {
    llimphi_ui::run::<Hapiy>();
}
