//! Vista (UI Llimphi) de la app `tullpu`: header, panel de capas, panel
//! de operaciones, panel del lienzo con su painter, histograma, sliders
//! de parámetros y los helpers de layout/botones.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill, Image, Mix};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_button::{button_styled, button_view, ButtonPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use llimphi_module_file_picker::PickerMsg;
use pixel_verbo_core::OpPixel;
use tullpu_core::{Capa, Frescura, Lienzo, OpLocal, OrigenCapa, TransformacionPixel};
use tullpu_render::FormatoExport;

use crate::blend::etiqueta_blend;
use crate::ops::etiqueta_color_activo;
use crate::viewport::{lienzo_rect_set, transform_lienzo};
use crate::model::*;

pub(crate) fn header(
    theme: &llimphi_theme::Theme,
    lienzo: &Lienzo,
    estado: &str,
    proveedor_etiqueta: &str,
    factor_zoom: f32,
    herramienta: Herramienta,
    color_picked: Option<[u8; 4]>,
) -> View<Msg> {
    // Indicador discreto: sólo se muestra cuando el usuario tocó zoom
    // o pan; en el caso por defecto (fit) el header queda igual que antes.
    let vista = if (factor_zoom - 1.0).abs() < 1e-4 {
        String::new()
    } else {
        format!(" · vista {:.0}%", factor_zoom * 100.0)
    };
    let tool = format!(" · ⌨ {}", herramienta.etiqueta());
    let color = match color_picked {
        Some([r, g, b, a]) => format!(" · 🎨 #{r:02X}{g:02X}{b:02X} α={a}"),
        None => String::new(),
    };
    let titulo = format!(
        "tullpu · {}×{} · {} capas · IA: {proveedor_etiqueta}{vista}{tool}{color} · {estado}",
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

pub(crate) fn fila_capa(
    theme: &llimphi_theme::Theme,
    capa: &Capa,
    seleccionada: bool,
    thumb: Option<&Image>,
    renombrando_input: Option<&TextInputState>,
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

    // Si esta capa está siendo renombrada, el bloque del nombre cambia a
    // un text-input enfocado. El resto de los micro-controles (toggle,
    // slider, blend, mover, dup, elim) sigue activo — no bloqueamos el
    // resto de la fila durante la edición porque el modal de teclado ya
    // routea los keypress al input.
    let nombre: View<Msg> = match renombrando_input {
        Some(input) => {
            let tp = TextInputPalette::from_theme(theme);
            View::new(Style {
                flex_grow: 1.0,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(26.0_f32),
                },
                padding: Rect {
                    left: length(2.0_f32),
                    right: length(2.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(vec![text_input_view(
                input,
                "nuevo nombre…",
                true,
                &tp,
                // Click sobre el input cancela cualquier otra interacción
                // ambigua re-foqueando la edición sobre la misma capa.
                Msg::IniciarRenombrar(capa.id),
            )])
        }
        None => button_styled(
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
        ),
    };

    // Botones de control compactos a la derecha.
    let toggle = mini_btn(if capa.visible { "👁" } else { "—" }, Msg::ToggleVisible(capa.id), &btn_pal);
    // Slider de opacidad in-situ: reemplaza los botones α−/α+ (saltos de
    // 0.1) con drag continuo en [0.0, 1.0]. El widget devuelve `dv` (delta
    // de valor) por evento; `BumpOpacidad` ya acumula deltas, así que el
    // hook es directo. Sólo emitimos en `DragPhase::Move` — `End` no aporta
    // nuevo delta y duplicaría el último.
    let cap_id = capa.id;
    let opacidad = slider_view(
        "",
        capa.opacidad,
        0.0,
        1.0,
        &slider_pal_compacto(theme),
        move |phase, dv| match phase {
            DragPhase::Move => Some(Msg::BumpOpacidad(cap_id, dv)),
            DragPhase::End => None,
        },
    );
    let blend = mini_btn("blnd", Msg::CiclarBlend(capa.id), &btn_pal);
    // En la lista la pintamos top→fondo: "↑" visualmente sube en la lista,
    // lo que equivale a bajar el índice en la pila (más cerca del fondo).
    // Mantengo la semántica visual para que el usuario haga lo que ve.
    let subir = mini_btn("↑", Msg::MoverArriba(capa.id), &btn_pal);
    let bajar = mini_btn("↓", Msg::MoverAbajo(capa.id), &btn_pal);
    let dup = mini_btn("⎘", Msg::Duplicar(capa.id), &btn_pal);
    // ⊕ = combinar con la de abajo (merge down). Si la capa ya está al
    // fondo (idx 0 en la pila), el handler en `update` lo detecta y
    // emite estado descriptivo — el botón se pinta igual para todas las
    // capas; no escondemos para mantener el layout estable.
    let merge = mini_btn("⊕", Msg::Combinar(capa.id), &btn_pal);
    let elim = mini_btn("✕", Msg::Eliminar(capa.id), &btn_pal);

    // Thumbnail a la izquierda (slot fijo aun si el cache aún no lo tiene
    // — evita reflow). 24×24 con un margen interno para respirar.
    let thumb_view = match thumb {
        Some(img) => View::new(Style {
            size: Size {
                width: length(24.0_f32),
                height: length(24.0_f32),
            },
            padding: Rect {
                left: length(1.0_f32),
                right: length(3.0_f32),
                top: length(1.0_f32),
                bottom: length(1.0_f32),
            },
            ..Default::default()
        })
        .image(img.clone()),
        None => View::new(Style {
            size: Size {
                width: length(24.0_f32),
                height: length(24.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel_alt),
    };

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
    .children(vec![
        thumb_view, nombre, toggle, opacidad, blend, subir, bajar, dup, merge, elim,
    ])
}

/// Slider compacto pensado para vivir embedded en la fila de capa: sin
/// bloque de label (el nombre de la capa ya lo identifica), track
/// estrecho, valor a la derecha para feedback numérico inmediato.
pub(crate) fn slider_pal_compacto(theme: &llimphi_theme::Theme) -> SliderPalette {
    let mut p = SliderPalette::from_theme(theme);
    p.label_width = 0.0;
    p.track_width = 56.0;
    p.value_width = 36.0;
    p.row_height = 24.0;
    p
}

/// Paleta de slider para la sección "parámetros" del panel ops: track
/// más ancho que el de la fila de capa porque acá hay más espacio
/// horizontal, y label visible (a diferencia de la fila donde el
/// nombre de la capa ya identifica).
pub(crate) fn slider_pal_parametros(theme: &llimphi_theme::Theme) -> SliderPalette {
    let mut p = SliderPalette::from_theme(theme);
    p.label_width = 80.0;
    p.track_width = 140.0;
    p.value_width = 50.0;
    p.row_height = 26.0;
    p
}

/// Construye la vista del histograma RGB: 256 columnas verticales por
/// canal, normalizadas por el pico global (max sobre los 3 canales).
/// Cada canal se pinta en su color con alfa parcial para que se
/// superpongan legibles. Si `histograma` es `None` o todo cero,
/// devuelve un placeholder gris vacío. Altura fija (`HIST_ALTO`) — el
/// ancho lo decide el layout.
pub(crate) fn vista_histograma(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
    // Sólo necesitamos un Copy del array (768 bytes) para meterlo en
    // el closure del painter — barato.
    let hist = model.histograma;
    let bg = theme.bg_input;
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(HIST_ALTO),
        },
        ..Default::default()
    })
    .fill(bg)
    .paint_with(move |scene, _ts, r| {
        let Some(hist) = hist else {
            return;
        };
        // Pico global para normalizar. Si todos los canales son 0 (caso
        // borde: lienzo 0×0), nada que dibujar.
        let max: u32 = hist
            .iter()
            .flat_map(|canal| canal.iter().copied())
            .max()
            .unwrap_or(0);
        if max == 0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let max = max as f32;
        // Cada bin ocupa una franja vertical de ancho `bin_w` >= 1 px.
        // Si el ancho del nodo no llega a 256, comprimimos varios bins
        // por columna sumándolos (sin perder precisión); si sobra,
        // estiramos. Implementación simple: una columna por bin.
        let bin_w = r.w as f64 / 256.0;
        // Tres pasadas, una por canal. Pintamos rect por bin con altura
        // proporcional a count/max. Alfa < 255 para que se vean
        // superpuestos.
        let colores = [
            Color::from_rgba8(220, 60, 60, 180),  // R
            Color::from_rgba8(60, 200, 80, 180),  // G
            Color::from_rgba8(80, 120, 230, 180), // B
        ];
        for (canal_idx, color) in colores.iter().enumerate() {
            for v in 0..256 {
                let count = hist[canal_idx][v] as f32;
                let h_norm = (count / max).clamp(0.0, 1.0);
                let bar_h = (h_norm as f64) * (r.h as f64);
                if bar_h <= 0.0 {
                    continue;
                }
                let x0 = r.x as f64 + v as f64 * bin_w;
                let x1 = x0 + bin_w.max(1.0);
                let y0 = (r.y + r.h) as f64 - bar_h;
                let y1 = (r.y + r.h) as f64;
                let rect = KurboRect::new(x0, y0, x1, y1);
                scene.fill(Fill::NonZero, Affine::IDENTITY, *color, None, &rect);
            }
        }
    })
}

/// Si la capa seleccionada es una derivada con un `OpLocal`
/// parametrizable, devuelve los rows con los sliders en vivo
/// (`label`, slider escalado al rango del parámetro, drag → `Msg::AjustarParametro`).
/// `None` cuando no aplica: capa no seleccionada, raster, op IA, o
/// op sin parámetros (Invertir, Espejar*).
pub(crate) fn sliders_parametros_capa(
    theme: &llimphi_theme::Theme,
    model: &Model,
) -> Option<Vec<View<Msg>>> {
    let id = model.seleccionada?;
    let capa = model.lienzo.capa(id)?;
    let op = match &capa.origen {
        OrigenCapa::Derivada {
            op: TransformacionPixel::Local(op),
            ..
        } => op,
        _ => return None,
    };
    let pal = slider_pal_parametros(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();
    // Helper para construir 1 row con 1 slider para `param`.
    let mk_slider = |label: &'static str,
                     valor: f32,
                     min: f32,
                     max: f32,
                     param: ParametroSlider|
     -> View<Msg> {
        let pal_clon = pal.clone();
        slider_view(label, valor, min, max, &pal_clon, move |fase, dv| {
            match fase {
                DragPhase::Move => Some(Msg::AjustarParametro { id, param, dv }),
                DragPhase::End => None,
            }
        })
    };
    match op {
        OpLocal::Brillo { delta } => {
            rows.push(mk_slider(
                "brillo",
                *delta,
                -1.0,
                1.0,
                ParametroSlider::BrilloDelta,
            ));
        }
        OpLocal::Contraste { factor } => {
            rows.push(mk_slider(
                "contraste",
                *factor,
                0.0,
                3.0,
                ParametroSlider::ContrasteFactor,
            ));
        }
        OpLocal::Saturacion { factor } => {
            rows.push(mk_slider(
                "saturación",
                *factor,
                0.0,
                3.0,
                ParametroSlider::SaturacionFactor,
            ));
        }
        OpLocal::Tonalidad { grados } => {
            rows.push(mk_slider(
                "tonalidad",
                *grados,
                -180.0,
                180.0,
                ParametroSlider::TonalidadGrados,
            ));
        }
        OpLocal::Blur { radio } => {
            rows.push(mk_slider(
                "radio blur",
                *radio,
                0.0,
                20.0,
                ParametroSlider::BlurRadio,
            ));
        }
        OpLocal::Opacidad { factor } => {
            rows.push(mk_slider(
                "opacidad op",
                *factor,
                0.0,
                1.0,
                ParametroSlider::OpacidadFactor,
            ));
        }
        OpLocal::Niveles {
            entrada_min,
            entrada_max,
            gamma,
        } => {
            // Tres sliders apilados — orden visual: min (negros) abajo,
            // max (blancos) en medio, gamma (curva) arriba. Replica el
            // panel Levels de Photoshop de arriba a abajo.
            rows.push(mk_slider(
                "niveles γ",
                *gamma,
                0.1,
                4.0,
                ParametroSlider::NivelesGamma,
            ));
            rows.push(mk_slider(
                "niveles blanco",
                *entrada_max,
                0.0,
                1.0,
                ParametroSlider::NivelesEntradaMax,
            ));
            rows.push(mk_slider(
                "niveles negro",
                *entrada_min,
                0.0,
                1.0,
                ParametroSlider::NivelesEntradaMin,
            ));
        }
        // Sin parámetros editables: `Invertir`, `Espejar*`.
        _ => return None,
    }
    Some(rows)
}

pub(crate) fn mini_btn(label: &str, msg: Msg, pal: &ButtonPalette) -> View<Msg> {
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

pub(crate) fn panel_capas(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
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
        let thumb = model.thumbs.get(&capa.contenido);
        let renombrando = model
            .renombrando
            .as_ref()
            .filter(|(id, _)| *id == capa.id)
            .map(|(_, input)| input);
        hijos.push(fila_capa(theme, capa, sel, thumb, renombrando));
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(400.0_f32),
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

pub(crate) fn panel_ops(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
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

    // "herramienta": toggle entre mover (drag panea) y cuentagotas (click
    // lee píxel). Globales — no dependen de selección. Las hotkeys `m` y
    // `i` hacen lo mismo; los botones son por discoverability.
    let mut hijos = vec![subtitulo("herramienta")];
    let pal_tool_activo = ButtonPalette {
        bg: theme.bg_selected,
        fg: theme.fg_text,
        ..pal.clone()
    };
    let etiqueta_mover = if model.herramienta == Herramienta::Mover {
        "● mover (m)"
    } else {
        "○ mover (m)"
    };
    let etiqueta_cuenta = if model.herramienta == Herramienta::Cuentagotas {
        "● cuentagotas (i)"
    } else {
        "○ cuentagotas (i)"
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_mover.to_string(),
        if model.herramienta == Herramienta::Mover {
            &pal_tool_activo
        } else {
            &pal
        },
        Msg::CambiarHerramienta(Herramienta::Mover),
    )));
    hijos.push(envolver_fila(button_view(
        etiqueta_cuenta.to_string(),
        if model.herramienta == Herramienta::Cuentagotas {
            &pal_tool_activo
        } else {
            &pal
        },
        Msg::CambiarHerramienta(Herramienta::Cuentagotas),
    )));
    let etiqueta_marco = if model.herramienta == Herramienta::Marco {
        "● marco (r)"
    } else {
        "○ marco (r)"
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_marco.to_string(),
        if model.herramienta == Herramienta::Marco {
            &pal_tool_activo
        } else {
            &pal
        },
        Msg::CambiarHerramienta(Herramienta::Marco),
    )));

    // "parámetros": sliders en vivo si la capa seleccionada es una
    // derivada con OpLocal parametrizable. Sólo aparece cuando aplica
    // para no agregar ruido al panel.
    if let Some(rows) = sliders_parametros_capa(theme, model) {
        hijos.push(subtitulo("parámetros"));
        hijos.extend(rows.into_iter().map(envolver_fila));
    }

    // "histograma": chart RGB del composite vigente. Sólo se renderiza
    // si hay imagen ya recompuesta (caso típico al arrancar la app).
    if model.histograma.is_some() {
        hijos.push(subtitulo("histograma"));
        hijos.push(vista_histograma(theme, model));
    }

    // "entrada": agregar una capa nueva. Dos vías: relleno sólido del
    // color del cuentagotas, o fuzzy picker de un archivo del workspace.
    // Ninguna requiere selección — siempre activas.
    hijos.push(subtitulo("entrada"));
    // Botón de relleno: muestra el color que va a usar. Si no hay color
    // leído por el cuentagotas, dice "gris" (el RELLENO_DEFAULT).
    let etiqueta_color = etiqueta_color_activo(model.color_picked);
    hijos.push(envolver_fila(button_view(
        format!(
            "+ relleno {} ({}×{})",
            etiqueta_color, model.lienzo.width, model.lienzo.height,
        ),
        &pal,
        Msg::AgregarRelleno,
    )));
    hijos.push(envolver_fila(button_view(
        format!(
            "📂 capa desde archivo · {} candidatos · Ctrl+P",
            model.imagenes_disponibles.len()
        ),
        &pal,
        Msg::Picker(PickerMsg::Open),
    )));

    // "estructura": operaciones sobre el lienzo entero. Aplanar las
    // visibles y rotar el lienzo 90° en cada sentido.
    let n_visibles = model.lienzo.capas.iter().filter(|c| c.visible).count();
    hijos.push(subtitulo("estructura"));
    hijos.push(envolver_fila(button_view(
        format!("⊞ aplanar visibles ({}) · Ctrl+Shift+E", n_visibles),
        &pal,
        Msg::AplanarVisibles,
    )));
    hijos.push(envolver_fila(button_view(
        "⟳ rotar +90° (CW)".to_string(),
        &pal,
        Msg::RotarLienzo { cw: true },
    )));
    hijos.push(envolver_fila(button_view(
        "⟲ rotar −90° (CCW)".to_string(),
        &pal,
        Msg::RotarLienzo { cw: false },
    )));
    hijos.push(envolver_fila(button_view(
        "✂ recortar a visible (auto-trim)".to_string(),
        &pal,
        Msg::AutotrimLienzo,
    )));
    // Crop a selección: sólo tiene sentido si hay un rect activo.
    // Mostramos siempre pero la etiqueta refleja el estado para que
    // el botón sea discoverable también sin selección.
    let etiqueta_crop_sel = match model.seleccion {
        Some(r) => format!(
            "✂ recortar a selección ({}×{})",
            r.x1 - r.x0,
            r.y1 - r.y0
        ),
        None => "✂ recortar a selección (—)".to_string(),
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_crop_sel,
        &pal,
        Msg::RecortarASeleccion,
    )));
    // Limpiar selección (alfa=0) en la capa raster seleccionada.
    // Misma política discoverable: botón siempre visible, etiqueta
    // refleja el estado.
    let etiqueta_limpiar_sel = match model.seleccion {
        Some(r) => format!(
            "🗑 limpiar selección ({}×{}) · Del",
            r.x1 - r.x0,
            r.y1 - r.y0
        ),
        None => "🗑 limpiar selección (—) · Del".to_string(),
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_limpiar_sel,
        &pal,
        Msg::LimpiarSeleccionEnCapa,
    )));
    // Rellenar selección con el color activo. La etiqueta muestra el
    // color que se usará (hex del cuentagotas, o "gris" del default) y
    // las dims del rect — discoverable también sin marquee.
    let color_fill = etiqueta_color_activo(model.color_picked);
    let etiqueta_rellenar_sel = match model.seleccion {
        Some(r) => format!(
            "🪣 rellenar selección {} ({}×{}) · ⇧Del",
            color_fill,
            r.x1 - r.x0,
            r.y1 - r.y0
        ),
        None => format!("🪣 rellenar selección {} (—) · ⇧Del", color_fill),
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_rellenar_sel,
        &pal,
        Msg::RellenarSeleccionEnCapa,
    )));
    // Duplicar la selección a una capa nueva (no destructivo). Misma
    // política discoverable de etiqueta dinámica.
    let etiqueta_dup_sel = match model.seleccion {
        Some(r) => format!(
            "⧉ duplicar selección a capa ({}×{}) · Ctrl+J",
            r.x1 - r.x0,
            r.y1 - r.y0
        ),
        None => "⧉ duplicar selección a capa (—) · Ctrl+J".to_string(),
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_dup_sel,
        &pal,
        Msg::DuplicarSeleccionACapa,
    )));
    // Portapapeles interno: copiar / cortar (exigen selección) y pegar
    // (exige clip). Las etiquetas reflejan disponibilidad.
    let etiqueta_copiar = match model.seleccion {
        Some(r) => {
            format!("⧉ copiar ({}×{}) · Ctrl+C", r.x1 - r.x0, r.y1 - r.y0)
        }
        None => "⧉ copiar (—) · Ctrl+C".to_string(),
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_copiar,
        &pal,
        Msg::CopiarSeleccion,
    )));
    let etiqueta_cortar = match model.seleccion {
        Some(r) => {
            format!("✂ cortar ({}×{}) · Ctrl+X", r.x1 - r.x0, r.y1 - r.y0)
        }
        None => "✂ cortar (—) · Ctrl+X".to_string(),
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_cortar,
        &pal,
        Msg::CortarSeleccion,
    )));
    let etiqueta_pegar = match model.portapapeles {
        Some(p) => format!("📋 pegar ({}×{}) · Ctrl+V", p.w, p.h),
        None => "📋 pegar (vacío) · Ctrl+V".to_string(),
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_pegar,
        &pal,
        Msg::PegarPortapapeles,
    )));

    // "salida": no requiere selección, siempre activa.
    hijos.push(subtitulo("salida"));
    hijos.push(envolver_fila(button_view(
        "💾 PNG (lossless · α)".to_string(),
        &pal,
        Msg::Exportar(FormatoExport::Png),
    )));
    hijos.push(envolver_fila(button_view(
        "💾 JPEG q90 (sin α)".to_string(),
        &pal,
        Msg::Exportar(FormatoExport::Jpeg { calidad: 90 }),
    )));
    hijos.push(envolver_fila(button_view(
        "💾 WebP (lossless · α)".to_string(),
        &pal,
        Msg::Exportar(FormatoExport::Webp),
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
        "+ Espejar ↔",
        OpLocal::EspejarHorizontal,
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Espejar ↕",
        OpLocal::EspejarVertical,
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

pub(crate) fn envolver_fila(boton: View<Msg>) -> View<Msg> {
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

pub(crate) fn panel_lienzo(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
    let cuerpo = match &model.imagen {
        Some(img) => {
            // Clones cheap: peniko::Image internamente es Arc<Blob>, los
            // floats son Copy. Capturadas por valor para que el closure
            // sea 'static + Send + Sync.
            let img = img.clone();
            let factor_zoom = model.factor_zoom;
            let pan_x = model.pan_x;
            let pan_y = model.pan_y;
            // Capturas para el painter de la selección: si hay rect
            // commiteado, lo dibuja; igual si hay drag activo (preview).
            let seleccion = model.seleccion;
            let lienzo_w = model.lienzo.width;
            let lienzo_h = model.lienzo.height;
            let cuerpo_paint = View::new(Style {
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
            .paint_with(move |scene, _ts, r| {
                // Registramos el rect en cada paint para que on_wheel
                // pueda decidir si el cursor cayó sobre el lienzo y, en
                // ese caso, hacer zoom-a-cursor (el closure no muta
                // estado de la app — sólo escribe la cache lateral).
                lienzo_rect_set(r);
                if img.width == 0 || img.height == 0 || r.w <= 0.0 || r.h <= 0.0 {
                    return;
                }
                let Some((s, off_x, off_y)) = transform_lienzo(
                    img.width,
                    img.height,
                    r.w,
                    r.h,
                    factor_zoom,
                    pan_x,
                    pan_y,
                ) else {
                    return;
                };
                let tx = r.x as f64 + off_x;
                let ty = r.y as f64 + off_y;
                let transform = Affine::translate((tx, ty)) * Affine::scale(s);
                // Clip al rect del lienzo: una imagen zoom-in que se sale
                // del panel no debe pintar sobre el panel de ops o capas.
                let node_rect = KurboRect::new(
                    r.x as f64,
                    r.y as f64,
                    (r.x + r.w) as f64,
                    (r.y + r.h) as f64,
                );
                scene.push_layer(Mix::Clip, 1.0, Affine::IDENTITY, &node_rect);
                scene.draw_image(&img, transform);
                // Overlay de selección: rect en coords-imagen → coords
                // de pantalla vía el mismo transform que la imagen.
                // Doble-stroke (negro grueso + blanco fino) para que
                // se vea contra cualquier fondo — "marching ants"
                // estático.
                if let Some(rect_img) = seleccion {
                    if lienzo_w > 0 && lienzo_h > 0 {
                        let sx0 = tx + (rect_img.x0 as f64) * s;
                        let sy0 = ty + (rect_img.y0 as f64) * s;
                        let sx1 = tx + (rect_img.x1 as f64) * s;
                        let sy1 = ty + (rect_img.y1 as f64) * s;
                        let krect = KurboRect::new(sx0, sy0, sx1, sy1);
                        scene.stroke(
                            &Stroke::new(3.0),
                            Affine::IDENTITY,
                            Color::from_rgba8(0, 0, 0, 220),
                            None,
                            &krect,
                        );
                        scene.stroke(
                            &Stroke::new(1.0),
                            Affine::IDENTITY,
                            Color::from_rgba8(255, 255, 255, 240),
                            None,
                            &krect,
                        );
                    }
                }
                scene.pop_layer();
            });
            // El cableado de eventos depende de la herramienta: Mover
            // panea con drag; Cuentagotas recoge color con click; Marco
            // arma una selección con on_click_at (press) + draggable_at
            // (extender). El wheel sigue zoom-eando en los 3 modos.
            match model.herramienta {
                Herramienta::Mover => cuerpo_paint.draggable(|fase, dx, dy| match fase {
                    DragPhase::Move => Some(Msg::Pan(dx, dy)),
                    DragPhase::End => None,
                }),
                Herramienta::Cuentagotas => cuerpo_paint.on_click_at(|lx, ly, rw, rh| {
                    Some(Msg::RecogerColor { lx, ly, rw, rh })
                }),
                Herramienta::Marco => cuerpo_paint
                    .on_click_at(|lx, ly, rw, rh| {
                        Some(Msg::IniciarSeleccion { lx, ly, rw, rh })
                    })
                    .draggable_at(|fase, dx, dy, _lx0, _ly0| match fase {
                        DragPhase::Move => Some(Msg::AjustarSeleccion { dx, dy }),
                        DragPhase::End => Some(Msg::FinalizarSeleccion),
                    }),
            }
        }
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
