//! `hapiy-llimphi` — la **GUI** de captura de la suite (el "Spectacle").
//!
//! Capturá la pantalla, mirá el preview y elegí qué hacer: **Guardar** un PNG o
//! **Editar en tullpu** (el editor de imágenes) para anotar/recortar. Sobre
//! `hapiy-core` (modelo/encode/handoff) + `hapiy-capture` (backends).
//!
//! Nota: la captura corre en el hilo de UI (one-shot, rápida) y, como la ventana
//! de hapiy está abierta, puede aparecer en la toma. El recorte/anotado fino se
//! hace en tullpu.

use hapiy_capture::{capturer, Backend};
use hapiy_core::{default_dir, default_filename, tullpu_launch, Capturer, OutputInfo, Shot};
use llimphi_image::from_rgba8;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, AlignItems, FlexDirection, JustifyContent, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::{Color, ImageBrush as Image};
use llimphi_ui::{App, Handle, View};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const BG: Color = Color::from_rgb8(0x0E, 0x10, 0x16);
const PANEL: Color = Color::from_rgb8(0x16, 0x1A, 0x24);
const BTN: Color = Color::from_rgb8(0x24, 0x2A, 0x38);
const ACCENT: Color = Color::from_rgb8(0x6E, 0x8C, 0xDC);
const FG: Color = Color::from_rgb8(0xD6, 0xDE, 0xE8);
const MUTED: Color = Color::from_rgb8(0x8C, 0x98, 0xAA);

#[derive(Clone)]
enum Msg {
    Capture,
    Save,
    Edit,
    Clear,
    SelectOutput(usize),
}

struct Model {
    cap: Box<dyn Capturer>,
    outputs: Vec<OutputInfo>,
    sel: usize,
    shot: Option<Shot>,
    preview: Option<Image>,
    status: String,
}

struct Hapiy;

impl App for Hapiy {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "hapiy · captura"
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        // `Backend::Auto` siempre devuelve un capturer (cae a grim si el nativo
        // no está); el unwrap es seguro.
        let cap = capturer(Backend::Auto).unwrap_or_else(|_| capturer(Backend::Grim).unwrap());
        let outputs = cap.outputs().unwrap_or_default();
        Model {
            cap,
            outputs,
            sel: 0,
            shot: None,
            preview: None,
            status: "Pulsá Capturar.".into(),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Capture => {
                let out = model.outputs.get(model.sel).map(|o| o.name.clone());
                match model.cap.capture(out.as_deref()) {
                    Ok(s) => {
                        model.preview = Some(from_rgba8(s.rgba.clone(), s.width, s.height));
                        model.status = format!("Captura {}×{} — guardá o editá en tullpu.", s.width, s.height);
                        model.shot = Some(s);
                    }
                    Err(e) => model.status = format!("Error al capturar: {e}"),
                }
            }
            Msg::Save => match &model.shot {
                Some(s) => {
                    let p = default_dir().join(default_filename(&stamp()));
                    match s.save_png(&p) {
                        Ok(()) => model.status = format!("Guardado en {}", p.display()),
                        Err(e) => model.status = e,
                    }
                }
                None => model.status = "Capturá primero.".into(),
            },
            Msg::Edit => match &model.shot {
                Some(s) => {
                    let p = std::env::temp_dir().join(default_filename(&stamp()));
                    match s.save_png(&p).and_then(|()| launch_tullpu(&p)) {
                        Ok(()) => model.status = format!("Abriendo en tullpu: {}", p.display()),
                        Err(e) => model.status = e,
                    }
                }
                None => model.status = "Capturá primero.".into(),
            },
            Msg::Clear => {
                model.shot = None;
                model.preview = None;
                model.status = "Pulsá Capturar.".into();
            }
            Msg::SelectOutput(i) => model.sel = i,
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let mut toolbar = vec![
            boton("⛶  Capturar", BG, ACCENT, Msg::Capture),
            boton("💾  Guardar", FG, BTN, Msg::Save),
            boton("✎  Editar en tullpu", FG, BTN, Msg::Edit),
            boton("🗑  Limpiar", MUTED, BTN, Msg::Clear),
        ];
        // Selector de salida si hay más de un monitor.
        if model.outputs.len() > 1 {
            for (i, o) in model.outputs.iter().enumerate() {
                let activo = i == model.sel;
                toolbar.push(boton(
                    &o.name,
                    if activo { BG } else { MUTED },
                    if activo { ACCENT } else { PANEL },
                    Msg::SelectOutput(i),
                ));
            }
        }

        let toolbar = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0), height: length(60.0) },
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(10.0), height: length(0.0) },
            padding: pad(10.0),
            ..Default::default()
        })
        .fill(PANEL)
        .children(toolbar);

        let lienzo = match &model.preview {
            Some(img) => View::new(Style {
                size: Size { width: percent(1.0), height: percent(1.0) },
                flex_grow: 1.0,
                ..Default::default()
            })
            .image(img.clone()),
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

fn boton(label: &str, fg: Color, bg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(170.0), height: length(40.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(8.0)
    .text(label, 15.0, fg)
    .on_click(msg)
}

/// Padding uniforme en píxeles para un `Style`.
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
