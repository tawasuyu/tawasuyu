//! `tullpu` — app de escritorio Llimphi: lienzo central + panel de capas +
//! paleta de operaciones (locales e IA). MVP del editor de imágenes por
//! capas IA-able.
//!
//! Layout:
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────┐
//! │ header: dimensiones · proveedor IA · estado               │
//! ├──────────────┬─────────────────────────────┬──────────────┤
//! │ capas        │                             │ locales      │
//! │  • fondo     │        LIENZO compuesto     │  + Invertir  │
//! │  • inversión │        (peniko::Image)      │  + Brillo+   │
//! │  • brillo    │                             │  …           │
//! │              │                             │ IA           │
//! │ [+ raster]   │                             │  + Restyle   │
//! │              │                             │  + Segmentar │
//! │              │                             │  + Inpaint   │
//! │              │                             │  + Generar   │
//! └──────────────┴─────────────────────────────┴──────────────┘
//! ```
//!
//! Cada panel de capa es un botón clicable que la selecciona; el panel
//! derecho aplica una op nueva como capa derivada de la seleccionada.
//! Las ops IA se delegan al [`pixel_verbo_core::Proveedor`] que la app
//! resuelve al arranque: si encuentra el daemon `pixel-verbo-daemon` en
//! `$XDG_RUNTIME_DIR/pixel-verbo.sock` lo usa; si no, cae al `ProveedorMock`
//! en proceso — así el botón "Generar" igual funciona sin daemon corriendo.
//! Cada cambio dispara `regenerar_stale_con_ia` + `componer` sincrónicamente.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_button::{button_styled, button_view, ButtonPalette};

use pixel_verbo_core::{OpPixel, Proveedor};
use pixel_verbo_daemon::ClienteBloqueante;
use pixel_verbo_mock::ProveedorMock;
use tullpu_core::{
    Capa, Frescura, Lienzo, ModoFusion, OpLocal, OrigenCapa, TransformacionPixel,
};
use tullpu_ops::{regenerar_stale_con_ia, transformacion_ia};
use tullpu_render::{componer, AlmacenEnMemoria, FuenteBuffers};
use uuid::Uuid;

// =============================================================================
//  Model & Msg
// =============================================================================

struct Model {
    lienzo: Lienzo,
    almacen: AlmacenEnMemoria,
    seleccionada: Option<Uuid>,
    imagen: Option<Image>,
    estado: String,
    proveedor: Box<dyn Proveedor>,
    proveedor_etiqueta: String,
}

#[derive(Clone)]
enum Msg {
    Seleccionar(Uuid),
    ToggleVisible(Uuid),
    BumpOpacidad(Uuid, f32),
    CiclarBlend(Uuid),
    Eliminar(Uuid),
    Agregar(OpLocal),
    AgregarIa(OpPixel),
    Recargar,
    ExportarPng,
}

// =============================================================================
//  Inicialización: gradiente procedural como capa fondo
// =============================================================================

const W: u32 = 512;
const H: u32 = 320;

fn buffer_gradiente() -> Vec<u8> {
    let mut v = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let r = (x * 255 / W.max(1)) as u8;
            let g = (y * 255 / H.max(1)) as u8;
            let b = (((x + y) / 2).min(255)) as u8;
            v.extend_from_slice(&[r, g, b, 255]);
        }
    }
    v
}

fn cargar_png(path: &std::path::Path) -> Option<(u32, u32, Vec<u8>)> {
    let reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let dyn_img = reader.decode().ok()?;
    let rgba = dyn_img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((w, h, rgba.into_raw()))
}

fn socket_pixel_verbo() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join("pixel-verbo.sock");
    }
    PathBuf::from("/tmp/pixel-verbo.sock")
}

/// Intenta hablar con el daemon; si no responde, cae al mock. La elección
/// queda visible en el header — el usuario sabe contra qué está pegando.
fn resolver_proveedor() -> (Box<dyn Proveedor>, String) {
    let sock = socket_pixel_verbo();
    match ClienteBloqueante::conectar(&sock) {
        Ok(cli) => {
            let etiqueta = format!("daemon {} @ {}", cli.model_id(), sock.display());
            (Box::new(cli), etiqueta)
        }
        Err(_) => {
            let mock = ProveedorMock::nuevo();
            let etiqueta = format!("mock {}", mock.model_id());
            (Box::new(mock), etiqueta)
        }
    }
}

/// Decide cómo materializar el path recibido por CLI según extensión:
/// `.psd` → import multi-capa via `foreign-psd`; el resto cae al loader PNG
/// existente y se convierte en una capa raster única. Devuelve el lienzo
/// armado, su almacén poblado, el `Uuid` que la UI debe seleccionar al
/// arrancar y una etiqueta corta para el header.
fn cargar_arg(path: &std::path::Path) -> Option<(Lienzo, AlmacenEnMemoria, Uuid, String)> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_lowercase);
    match ext.as_deref() {
        Some("psd") => cargar_psd(path),
        _ => cargar_png_como_capa(path),
    }
}

fn cargar_png_como_capa(
    path: &std::path::Path,
) -> Option<(Lienzo, AlmacenEnMemoria, Uuid, String)> {
    let (w, h, bytes) = cargar_png(path)?;
    let mut almacen = AlmacenEnMemoria::nuevo();
    let hash = almacen.insertar(bytes);
    let mut lienzo = Lienzo::nuevo(w, h);
    let fondo = Capa::raster("fondo", hash);
    let id = fondo.id;
    lienzo.apilar(fondo);
    Some((lienzo, almacen, id, format!("png cargado · {}×{}", w, h)))
}

fn cargar_psd(path: &std::path::Path) -> Option<(Lienzo, AlmacenEnMemoria, Uuid, String)> {
    let bytes = std::fs::read(path).ok()?;
    let imp = match foreign_psd::importar_psd(&bytes) {
        Ok(imp) => imp,
        Err(e) => {
            eprintln!("tullpu: error importando '{}': {e}", path.display());
            return None;
        }
    };
    // Vuelco todos los buffers (uno por capa, dedup ya hecho por hash) al
    // almacén de la app — `tullpu-render::componer` los va a pedir por hash.
    let mut almacen = AlmacenEnMemoria::nuevo();
    almacen.buffers.extend(imp.buffers);
    let n_capas = imp.lienzo.capas.len();
    let n_degradadas = imp.informe.caidas_a_normal.len();
    // El primer Uuid (fondo) sirve como selección inicial — si el PSD vino
    // vacío (foreign-psd ya lo rechaza, pero por defensa), caemos a default.
    let id = imp.lienzo.capas.first()?.id;
    let etiqueta = if n_degradadas == 0 {
        format!("psd · {} capas", n_capas)
    } else {
        format!("psd · {} capas ({} a Normal)", n_capas, n_degradadas)
    };
    Some((imp.lienzo, almacen, id, etiqueta))
}

fn inicializar() -> Model {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let (lienzo, almacen, id_inicial, estado) = arg
        .as_ref()
        .and_then(|p| cargar_arg(p))
        .unwrap_or_else(lienzo_default);

    let (proveedor, proveedor_etiqueta) = resolver_proveedor();

    let imagen = recomponer(&lienzo, &almacen);
    Model {
        lienzo,
        almacen,
        seleccionada: Some(id_inicial),
        imagen,
        estado,
        proveedor,
        proveedor_etiqueta,
    }
}

fn lienzo_default() -> (Lienzo, AlmacenEnMemoria, Uuid, String) {
    let mut almacen = AlmacenEnMemoria::nuevo();
    let hash = almacen.insertar(buffer_gradiente());
    let mut lienzo = Lienzo::nuevo(W, H);
    let fondo = Capa::raster("fondo", hash);
    let id = fondo.id;
    lienzo.apilar(fondo);
    (lienzo, almacen, id, "listo · gradiente demo".into())
}

fn recomponer(l: &Lienzo, alm: &impl FuenteBuffers) -> Option<Image> {
    let img = componer(l, alm).ok()?;
    let (w, h) = (img.width(), img.height());
    let blob = Blob::from(img.into_raw());
    Some(Image::new(blob, ImageFormat::Rgba8, w, h))
}

fn aplicar_y_recomponer(model: &mut Model) {
    match regenerar_stale_con_ia(
        &mut model.lienzo,
        &mut model.almacen,
        model.proveedor.as_ref(),
    ) {
        Ok(regen) => {
            model.estado = if regen.is_empty() {
                "listo".into()
            } else {
                format!("regeneradas {}", regen.len())
            };
        }
        Err(e) => {
            model.estado = format!("error ops: {e}");
        }
    }
    match recomponer(&model.lienzo, &model.almacen) {
        Some(img) => model.imagen = Some(img),
        None => model.estado = "error compositor".into(),
    }
}

// =============================================================================
//  Ciclar blend modes (no hay dropdown todavía — clic cicla)
// =============================================================================

fn siguiente_blend(b: ModoFusion) -> ModoFusion {
    match b {
        ModoFusion::Normal => ModoFusion::Multiplicar,
        ModoFusion::Multiplicar => ModoFusion::Pantalla,
        ModoFusion::Pantalla => ModoFusion::Superponer,
        ModoFusion::Superponer => ModoFusion::Aclarar,
        ModoFusion::Aclarar => ModoFusion::Oscurecer,
        ModoFusion::Oscurecer => ModoFusion::Diferencia,
        ModoFusion::Diferencia => ModoFusion::Aditivo,
        ModoFusion::Aditivo => ModoFusion::SubExpQuemado,
        ModoFusion::SubExpQuemado => ModoFusion::SubLinealQuemado,
        ModoFusion::SubLinealQuemado => ModoFusion::SobreExpAclarado,
        ModoFusion::SobreExpAclarado => ModoFusion::LuzFuerte,
        ModoFusion::LuzFuerte => ModoFusion::LuzSuave,
        ModoFusion::LuzSuave => ModoFusion::LuzViva,
        ModoFusion::LuzViva => ModoFusion::LuzLineal,
        ModoFusion::LuzLineal => ModoFusion::LuzPunto,
        ModoFusion::LuzPunto => ModoFusion::MezclaDura,
        ModoFusion::MezclaDura => ModoFusion::Exclusion,
        ModoFusion::Exclusion => ModoFusion::Resta,
        ModoFusion::Resta => ModoFusion::Division,
        ModoFusion::Division => ModoFusion::HslTono,
        ModoFusion::HslTono => ModoFusion::HslSaturacion,
        ModoFusion::HslSaturacion => ModoFusion::HslColor,
        ModoFusion::HslColor => ModoFusion::HslLuminosidad,
        ModoFusion::HslLuminosidad => ModoFusion::ColorMasOscuro,
        ModoFusion::ColorMasOscuro => ModoFusion::ColorMasClaro,
        ModoFusion::ColorMasClaro => ModoFusion::Disolver,
        ModoFusion::Disolver => ModoFusion::Normal,
    }
}

fn etiqueta_blend(b: ModoFusion) -> &'static str {
    match b {
        ModoFusion::Normal => "normal",
        ModoFusion::Multiplicar => "multiplicar",
        ModoFusion::Pantalla => "pantalla",
        ModoFusion::Superponer => "superponer",
        ModoFusion::Aclarar => "aclarar",
        ModoFusion::Oscurecer => "oscurecer",
        ModoFusion::Diferencia => "diferencia",
        ModoFusion::Aditivo => "aditivo",
        ModoFusion::SubExpQuemado => "subexp-quemado",
        ModoFusion::SubLinealQuemado => "sublineal-quemado",
        ModoFusion::SobreExpAclarado => "sobreexp-aclarado",
        ModoFusion::LuzFuerte => "luz-fuerte",
        ModoFusion::LuzSuave => "luz-suave",
        ModoFusion::LuzViva => "luz-viva",
        ModoFusion::LuzLineal => "luz-lineal",
        ModoFusion::LuzPunto => "luz-punto",
        ModoFusion::MezclaDura => "mezcla-dura",
        ModoFusion::Exclusion => "exclusión",
        ModoFusion::Resta => "resta",
        ModoFusion::Division => "división",
        ModoFusion::HslTono => "hsl-tono",
        ModoFusion::HslSaturacion => "hsl-saturación",
        ModoFusion::HslColor => "hsl-color",
        ModoFusion::HslLuminosidad => "hsl-luminosidad",
        ModoFusion::ColorMasOscuro => "color-más-oscuro",
        ModoFusion::ColorMasClaro => "color-más-claro",
        ModoFusion::Disolver => "disolver",
    }
}

// =============================================================================
//  Vista
// =============================================================================

fn header(
    theme: &llimphi_theme::Theme,
    lienzo: &Lienzo,
    estado: &str,
    proveedor_etiqueta: &str,
) -> View<Msg> {
    let titulo = format!(
        "tullpu · {}×{} · {} capas · IA: {proveedor_etiqueta} · {estado}",
        lienzo.width,
        lienzo.height,
        lienzo.capas.len()
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(titulo, 12.0, theme.fg_muted, Alignment::Start)
}

fn fila_capa(
    theme: &llimphi_theme::Theme,
    capa: &Capa,
    seleccionada: bool,
) -> View<Msg> {
    let btn_pal = ButtonPalette::from_theme(theme);
    let nombre_op = match &capa.origen {
        OrigenCapa::Raster => "raster".to_string(),
        OrigenCapa::Derivada { op, estado, .. } => {
            let suf = match estado {
                Frescura::Fresca => "",
                Frescura::Stale => " · stale",
            };
            format!("{}{suf}", op.etiqueta())
        }
    };
    let etiqueta = format!(
        "{}  ·  {}  ·  α {:.2}  ·  {}",
        capa.nombre,
        nombre_op,
        capa.opacidad,
        etiqueta_blend(capa.blend)
    );
    let fila_bg = if seleccionada {
        theme.bg_panel_alt
    } else {
        theme.bg_panel
    };
    let fg = if capa.visible {
        theme.fg_text
    } else {
        theme.fg_muted
    };

    // Botón principal: selección de la capa, ocupa la mayor parte de la fila.
    let nombre = button_styled(
        etiqueta,
        Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: length(26.0_f32),
            },
            padding: Rect {
                left: length(10.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        },
        Alignment::Start,
        &ButtonPalette {
            bg: fila_bg,
            bg_hover: theme.bg_button_hover,
            fg,
            radius: 4.0,
        },
        Msg::Seleccionar(capa.id),
    );

    // Botones de control compactos a la derecha.
    let toggle = mini_btn(if capa.visible { "👁" } else { "—" }, Msg::ToggleVisible(capa.id), &btn_pal);
    let opd = mini_btn("α−", Msg::BumpOpacidad(capa.id, -0.1), &btn_pal);
    let opu = mini_btn("α+", Msg::BumpOpacidad(capa.id, 0.1), &btn_pal);
    let blend = mini_btn("blnd", Msg::CiclarBlend(capa.id), &btn_pal);
    let elim = mini_btn("✕", Msg::Eliminar(capa.id), &btn_pal);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(2.0_f32),
            right: length(2.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![nombre, toggle, opd, opu, blend, elim])
}

fn mini_btn(label: &str, msg: Msg, pal: &ButtonPalette) -> View<Msg> {
    button_styled(
        label.to_string(),
        Style {
            size: Size {
                width: length(34.0_f32),
                height: length(24.0_f32),
            },
            padding: Rect {
                left: length(2.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        pal,
        msg,
    )
}

fn panel_capas(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
    let mut hijos: Vec<View<Msg>> = Vec::new();
    let titulo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned("capas (top→fondo)".to_string(), 11.0, theme.fg_muted, Alignment::Start);
    hijos.push(titulo);
    // Las pintamos top → fondo (al revés que el orden visual interno).
    for capa in model.lienzo.capas.iter().rev() {
        let sel = model.seleccionada == Some(capa.id);
        hijos.push(fila_capa(theme, capa, sel));
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(360.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(hijos)
}

fn panel_ops(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
    let pal = ButtonPalette::from_theme(theme);
    let bloqueado = model.seleccionada.is_none();
    let mk_local = |label: &str, op: OpLocal| -> View<Msg> {
        let msg = if bloqueado { Msg::Recargar } else { Msg::Agregar(op) };
        button_view(label.to_string(), &pal, msg)
    };
    let mk_ia = |label: &str, op: OpPixel| -> View<Msg> {
        let msg = if bloqueado {
            Msg::Recargar
        } else {
            Msg::AgregarIa(op)
        };
        button_view(label.to_string(), &pal, msg)
    };

    let subtitulo = |s: &str| {
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(22.0_f32),
            },
            padding: Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(8.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(s.to_string(), 11.0, theme.fg_muted, Alignment::Start)
    };

    // "salida" arriba de todo: no requiere selección, siempre activa.
    let mut hijos = vec![subtitulo("salida")];
    hijos.push(envolver_fila(button_view(
        "💾 exportar PNG".to_string(),
        &pal,
        Msg::ExportarPng,
    )));

    hijos.push(subtitulo("locales"));
    hijos.push(envolver_fila(mk_local("+ Invertir", OpLocal::Invertir)));
    hijos.push(envolver_fila(mk_local(
        "+ Brillo +0.15",
        OpLocal::Brillo { delta: 0.15 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Brillo −0.15",
        OpLocal::Brillo { delta: -0.15 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Contraste ×1.3",
        OpLocal::Contraste { factor: 1.3 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Contraste ×0.7",
        OpLocal::Contraste { factor: 0.7 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Saturación ×0.0",
        OpLocal::Saturacion { factor: 0.0 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Saturación ×1.5",
        OpLocal::Saturacion { factor: 1.5 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Tonalidad 90°",
        OpLocal::Tonalidad { grados: 90.0 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Blur radio 4",
        OpLocal::Blur { radio: 4.0 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Niveles 0.1–0.9 γ1.2",
        OpLocal::Niveles {
            entrada_min: 0.1,
            entrada_max: 0.9,
            gamma: 1.2,
        },
    )));

    hijos.push(subtitulo("ia"));
    hijos.push(envolver_fila(mk_ia(
        "+ Restyle 'tropical'",
        OpPixel::Restyle {
            prompt: "tropical".into(),
        },
    )));
    hijos.push(envolver_fila(mk_ia(
        "+ Restyle 'frío'",
        OpPixel::Restyle {
            prompt: "frío".into(),
        },
    )));
    hijos.push(envolver_fila(mk_ia(
        "+ Segmentar centro",
        OpPixel::Segmentar { prompt: None },
    )));
    hijos.push(envolver_fila(mk_ia(
        "+ Inpaint huecos",
        OpPixel::Inpaint { prompt: None },
    )));
    hijos.push(envolver_fila(mk_ia(
        "+ Generar 'atardecer'",
        OpPixel::Generar {
            prompt: "atardecer".into(),
            ancho: model.lienzo.width,
            alto: model.lienzo.height,
        },
    )));

    if bloqueado {
        hijos.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(10.0_f32),
                    top: length(8.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                "(seleccioná una capa primero)".to_string(),
                10.0,
                theme.fg_muted,
                Alignment::Start,
            ),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(240.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(hijos)
}

fn envolver_fila(boton: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(3.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![boton])
}

fn panel_lienzo(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
    let cuerpo = match &model.imagen {
        Some(img) => View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(12.0_f32),
                bottom: length(12.0_f32),
            },
            ..Default::default()
        })
        .image(img.clone()),
        None => View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            "(sin composición)".to_string(),
            12.0,
            theme.fg_muted,
            Alignment::Center,
        ),
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![cuerpo])
}

// =============================================================================
//  App
// =============================================================================

struct Tullpu;

impl App for Tullpu {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "tullpu · editor de imágenes por capas"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        inicializar()
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Seleccionar(id) => {
                model.seleccionada = Some(id);
            }
            Msg::ToggleVisible(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.visible = !c.visible;
                }
                aplicar_y_recomponer(&mut model);
            }
            Msg::BumpOpacidad(id, delta) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.opacidad = (c.opacidad + delta).clamp(0.0, 1.0);
                }
                aplicar_y_recomponer(&mut model);
            }
            Msg::CiclarBlend(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.blend = siguiente_blend(c.blend);
                }
                aplicar_y_recomponer(&mut model);
            }
            Msg::Eliminar(id) => {
                model.lienzo.capas.retain(|c| c.id != id);
                if model.seleccionada == Some(id) {
                    model.seleccionada = model.lienzo.capas.last().map(|c| c.id);
                }
                // Las capas derivadas que quedaron huérfanas se marcan stale
                // — su regeneración fallará silenciosamente (BufferFaltante).
                aplicar_y_recomponer(&mut model);
            }
            Msg::Agregar(op) => {
                if let Some(madre_id) = model.seleccionada {
                    // El contenido_cache inicial lo dejamos en ceros — el
                    // orquestador lo rellena en la siguiente regeneración.
                    let nueva = Capa::derivada(
                        format!("{}", op_etiqueta(&op)),
                        madre_id,
                        TransformacionPixel::Local(op),
                        [0u8; 32],
                    );
                    let nuevo_id = nueva.id;
                    model.lienzo.apilar(nueva);
                    model.seleccionada = Some(nuevo_id);
                    aplicar_y_recomponer(&mut model);
                }
            }
            Msg::AgregarIa(op) => {
                if let Some(madre_id) = model.seleccionada {
                    let modelo = model.proveedor.model_id().name.clone();
                    let nombre = format!("ia:{}", op.etiqueta());
                    let trans = transformacion_ia(modelo, &op);
                    let nueva = Capa::derivada(nombre, madre_id, trans, [0u8; 32]);
                    let nuevo_id = nueva.id;
                    model.lienzo.apilar(nueva);
                    model.seleccionada = Some(nuevo_id);
                    aplicar_y_recomponer(&mut model);
                }
            }
            Msg::Recargar => {
                aplicar_y_recomponer(&mut model);
            }
            Msg::ExportarPng => {
                // Path en CWD con timestamp Unix — sin file picker (la app
                // todavía no tiene). El usuario ve el path final en el
                // header para `find` / `xdg-open` desde fuera.
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let ruta = std::path::PathBuf::from(format!("tullpu-export-{ts}.png"));
                model.estado = match tullpu_render::exportar_png(
                    &model.lienzo,
                    &model.almacen,
                    &ruta,
                ) {
                    Ok(_) => format!("exportado → {}", ruta.display()),
                    Err(e) => format!("export falló: {e}"),
                };
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = llimphi_theme::Theme::dark();
        let cabecera = header(
            &theme,
            &model.lienzo,
            &model.estado,
            &model.proveedor_etiqueta,
        );
        let centro = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![
            panel_capas(&theme, model),
            panel_lienzo(&theme, model),
            panel_ops(&theme, model),
        ]);
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![cabecera, centro])
    }
}

fn op_etiqueta(op: &OpLocal) -> &'static str {
    match op {
        OpLocal::Invertir => "invertir",
        OpLocal::Brillo { .. } => "brillo",
        OpLocal::Contraste { .. } => "contraste",
        OpLocal::Niveles { .. } => "niveles",
        OpLocal::Blur { .. } => "blur",
        OpLocal::Opacidad { .. } => "opacidad",
        OpLocal::Saturacion { .. } => "saturación",
        OpLocal::Tonalidad { .. } => "tonalidad",
    }
}

fn main() {
    llimphi_ui::run::<Tullpu>();
}
