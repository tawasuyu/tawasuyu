//! Pantallazo headless del motor — **una UI real y densa** compuesta sólo con
//! primitivas del compositor (View → layout → vello → wgpu → PNG), pensada
//! para la tarjeta pública "Un motor gráfico soberano".
//!
//! Muestra en una sola pasada: tema oscuro de `llimphi-theme`, top bar con
//! tabs, sidebar con filas seleccionadas, un editor de código con resaltado
//! sintáctico vía `TextSpan`s sobre fuente mono, un párrafo de texto rico
//! (pesos, cursiva, subrayado, mono inline), tarjetas de métricas con
//! gradientes y sombras, un mini gráfico de barras hecho con puros rects,
//! chips/botones, y un toast flotante con esquinas asimétricas.
//!
//! `cargo run -p llimphi-compositor --example pantallazo_motor --release -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_compositor::{measure_text_node, mount, paint, Shadow, View};
use llimphi_hal::{wgpu, Hal};
use llimphi_layout::taffy;
use llimphi_layout::taffy::prelude::{auto, length, percent, FlexDirection, Size, Style};
use llimphi_layout::taffy::{AlignItems, JustifyContent, Position, Rect};
use llimphi_layout::LayoutTree;
use llimphi_raster::peniko::{Color, Gradient};
use llimphi_raster::{vello, Renderer};
use llimphi_text::{Alignment, TextSpan, TextSpanStyle, Typesetter};
use vello::kurbo::Point;

const W: u32 = 1280;
const H: u32 = 800;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

/// Caja vacía con tamaño fijo — separadores, swatches, barras.
fn rect(w: f32, h: f32) -> View<()> {
    View::<()>::new(Style { size: Size { width: length(w), height: length(h) }, ..Default::default() })
}

/// Nodo de texto de una línea con alto fijo (mismo patrón que los demos vecinos).
fn txt(w: taffy::Dimension, h: f32, s: &str, size: f32, c: Color) -> View<()> {
    View::<()>::new(Style { size: Size { width: w, height: length(h) }, ..Default::default() })
        .text_aligned(s.to_string(), size, c, Alignment::Start)
}

/// Spans sintácticos: pinta TODAS las ocurrencias de cada `needle` con su estilo.
fn spans_all(text: &str, reglas: &[(&str, TextSpanStyle)]) -> Vec<TextSpan> {
    let mut out = Vec::new();
    for (needle, style) in reglas {
        for (i, _) in text.match_indices(needle) {
            out.push(TextSpan::new(i, i + needle.len(), style.clone()));
        }
    }
    out
}

fn color_span(c: Color) -> TextSpanStyle {
    TextSpanStyle { color: Some(c), ..Default::default() }
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "pantallazo_motor.png".to_string());
    let theme = llimphi_theme::Theme::dark();

    // Paleta de sintaxis (sobre el panel oscuro del theme).
    let kw = rgb(198, 120, 221); // keywords — violeta
    let ty = rgb(229, 192, 123); // tipos — ámbar
    let fnc = rgb(97, 175, 239); // funciones — azul
    let strv = rgb(152, 195, 121); // strings — verde
    let cmt = rgb(92, 104, 124); // comentarios — gris azulado
    let lit = rgb(209, 154, 102); // literales numéricos — naranja
    let code_fg = rgb(171, 178, 191);

    // ───────────────────────────── top bar ─────────────────────────────
    let tab = |name: &str, activo: bool| {
        let base = View::<()>::new(Style {
            size: Size { width: auto(), height: length(30.0) },
            align_items: Some(AlignItems::Center),
            padding: Rect { left: length(14.0), right: length(14.0), top: length(0.0), bottom: length(0.0) },
            ..Default::default()
        })
        .radius(8.0);
        let fg = if activo { theme.fg_text } else { theme.fg_muted };
        let v = if activo { base.fill(theme.bg_selected) } else { base };
        v.children(vec![txt(auto(), 18.0, name, 13.0, fg)])
    };
    let brand_dot = rect(10.0, 10.0)
        .radius(5.0)
        .fill_gradient(
            Gradient::new_linear(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
                .with_stops([theme.accent, rgb(80, 200, 200)].as_slice()),
        );
    let buscador = View::<()>::new(Style {
        size: Size { width: length(230.0), height: length(28.0) },
        align_items: Some(AlignItems::Center),
        padding: Rect { left: length(12.0), right: length(12.0), top: length(0.0), bottom: length(0.0) },
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(14.0)
    .border(1.0, theme.border)
    .children(vec![txt(auto(), 16.0, "buscar en el haz…   ⌘K", 12.0, theme.fg_placeholder)]);
    let topbar = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(46.0) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(10.0), height: length(0.0) },
        padding: Rect { left: length(16.0), right: length(16.0), top: length(0.0), bottom: length(0.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .border(1.0, theme.border)
    .children(vec![
        brand_dot,
        txt(auto(), 18.0, "llimphi", 15.0, theme.fg_text).bold(),
        rect(14.0, 1.0),
        tab("pluma", true),
        tab("khipu", false),
        tab("cosmos", false),
        tab("shuma", false),
        // empuja el buscador a la derecha
        View::<()>::new(Style { flex_grow: 1.0, ..Default::default() }),
        buscador,
    ]);

    // ───────────────────────────── sidebar ─────────────────────────────
    let fila = |nombre: &str, badge: Option<&str>, sel: bool, dot: Color| {
        let base = View::<()>::new(Style {
            size: Size { width: percent(1.0), height: length(30.0) },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::SpaceBetween),
            padding: Rect { left: length(10.0), right: length(10.0), top: length(0.0), bottom: length(0.0) },
            ..Default::default()
        })
        .radius(7.0);
        let v = if sel { base.fill(theme.bg_selected) } else { base };
        let izq = View::<()>::new(Style {
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(8.0), height: length(0.0) },
            ..Default::default()
        })
        .children(vec![
            rect(8.0, 8.0).radius(2.5).fill(dot),
            txt(auto(), 17.0, nombre, 13.0, if sel { theme.fg_text } else { theme.fg_muted }),
        ]);
        let mut hijos = vec![izq];
        if let Some(b) = badge {
            hijos.push(
                View::<()>::new(Style {
                    size: Size { width: auto(), height: length(17.0) },
                    align_items: Some(AlignItems::Center),
                    padding: Rect { left: length(7.0), right: length(7.0), top: length(0.0), bottom: length(0.0) },
                    ..Default::default()
                })
                .fill(theme.bg_button)
                .radius(8.5)
                .children(vec![txt(auto(), 13.0, b, 10.5, theme.fg_muted)]),
            );
        }
        v.children(hijos)
    };
    let seccion = |t: &str| txt(percent(1.0), 16.0, t, 11.0, theme.fg_placeholder).bold();
    let sidebar = View::<()>::new(Style {
        size: Size { width: length(236.0), height: percent(1.0) },
        flex_direction: FlexDirection::Column,
        gap: Size { width: length(0.0), height: length(4.0) },
        padding: Rect { left: length(12.0), right: length(12.0), top: length(14.0), bottom: length(14.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .border(1.0, theme.border)
    .children(vec![
        seccion("HAZ DE CUERPOS"),
        fila("ensayo · español", None, true, theme.accent),
        fila("ensayo · english", Some("stale"), false, rgb(209, 154, 102)),
        fila("ensayo · runasimi", None, false, rgb(80, 200, 200)),
        fila("resumen ejecutivo", None, false, rgb(152, 195, 121)),
        rect(1.0, 10.0),
        seccion("MÓDULOS"),
        fila("nodegraph.rs", Some("12"), false, fnc),
        fila("text_editor.rs", Some("3"), false, fnc),
        fila("typesetter.rs", None, false, fnc),
        fila("raster/scene.rs", None, false, fnc),
        rect(1.0, 10.0),
        seccion("DAEMONS"),
        fila("verbo · e5-small", Some("384d"), false, rgb(152, 195, 121)),
        fila("chasqui · DHT", Some("9 peers"), false, rgb(152, 195, 121)),
    ]);

    // ─────────────────────── editor de código (centro) ───────────────────────
    let codigo = "\
// bucle Elm del motor: input → update → view → layout → raster\n\
pub fn frame(&mut self, msg: Msg) -> Scene {\n\
    self.app.update(msg);\n\
    let view = self.app.view();\n\
    let tree = mount(&mut self.layout, view);\n\
    let computed = self.layout.compute(tree.root, self.size);\n\
    let mut scene = Scene::new();\n\
    paint(&mut scene, &tree, &computed, &mut self.ts);\n\
    scene // vello la rasteriza en GPU vía wgpu\n\
}\n\
\n\
let sombra = Shadow::soft(90, 24.0).offset(0.0, 12.0);\n\
let card = View::new(estilo)\n\
    .fill_gradient(grad)      // gradiente en [0,1]²\n\
    .radius_corners(18.0, 18.0, 4.0, 4.0)\n\
    .shadow(sombra);";
    let reglas = [
        ("// bucle Elm del motor: input → update → view → layout → raster", TextSpanStyle { color: Some(cmt), italic: Some(true), ..Default::default() }),
        ("// vello la rasteriza en GPU vía wgpu", TextSpanStyle { color: Some(cmt), italic: Some(true), ..Default::default() }),
        ("// gradiente en [0,1]²", TextSpanStyle { color: Some(cmt), italic: Some(true), ..Default::default() }),
        ("pub fn ", color_span(kw)),
        ("let ", color_span(kw)),
        ("&mut ", color_span(kw)),
        ("self", color_span(rgb(224, 108, 117))),
        ("Msg", color_span(ty)),
        ("Scene", color_span(ty)),
        ("Shadow", color_span(ty)),
        ("View", color_span(ty)),
        ("frame", TextSpanStyle { color: Some(fnc), weight: Some(700.0), ..Default::default() }),
        ("update", color_span(fnc)),
        ("view()", color_span(fnc)),
        ("mount", color_span(fnc)),
        ("compute", color_span(fnc)),
        ("new", color_span(fnc)),
        ("paint", color_span(fnc)),
        ("soft", color_span(fnc)),
        ("offset", color_span(fnc)),
        ("fill_gradient", color_span(fnc)),
        ("radius_corners", color_span(fnc)),
        ("shadow(", color_span(fnc)),
        ("90, 24.0", color_span(lit)),
        ("0.0, 12.0", color_span(lit)),
        ("18.0, 18.0, 4.0, 4.0", color_span(lit)),
    ];
    let code_spans = spans_all(codigo, &reglas);
    let code_text = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(360.0) },
        ..Default::default()
    })
    .text_spans(codigo, 13.5, code_fg, code_spans, Alignment::Start)
    .mono()
    .line_height(1.55);
    // header del editor: tres puntos + nombre de archivo + chip de lenguaje
    let punto = |c: Color| rect(11.0, 11.0).radius(5.5).fill(c);
    let editor_header = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(36.0) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0), height: length(0.0) },
        padding: Rect { left: length(14.0), right: length(14.0), top: length(0.0), bottom: length(0.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius_corners(12.0, 12.0, 0.0, 0.0)
    .children(vec![
        punto(rgb(224, 108, 117)),
        punto(rgb(229, 192, 123)),
        punto(rgb(152, 195, 121)),
        rect(6.0, 1.0),
        txt(auto(), 17.0, "eventloop.rs", 13.0, theme.fg_text).mono(),
        txt(length(96.0), 16.0, "— llimphi-ui", 12.0, theme.fg_placeholder),
        View::<()>::new(Style { flex_grow: 1.0, ..Default::default() }),
        txt(auto(), 16.0, "rust", 11.0, theme.accent).mono(),
    ]);
    let editor = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: auto() },
        flex_grow: 1.0,
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(12.0)
    .border(1.0, theme.border)
    .shadow(Shadow::soft(110, 26.0).offset(0.0, 12.0))
    .children(vec![
        editor_header,
        View::<()>::new(Style {
            size: Size { width: percent(1.0), height: auto() },
            flex_grow: 1.0,
            padding: Rect { left: length(16.0), right: length(16.0), top: length(12.0), bottom: length(8.0) },
            ..Default::default()
        })
        .children(vec![code_text]),
    ]);

    // ─────────────────── párrafo rico (debajo del editor) ───────────────────
    let parrafo = "Un solo nodo de texto, varios lentes por rango de bytes: \
NEGRITA para el énfasis, cursiva para la voz, un enlace.qu subrayado, \
texto tachado para lo descartado, y Typesetter en mono inline — todo \
medido y pintado por el mismo layout_spans, sin HTML ni DOM.";
    let find = |n: &str| {
        let i = parrafo.find(n).expect("needle");
        (i, i + n.len())
    };
    let (b0, b1) = find("NEGRITA");
    let (i0, i1) = find("cursiva");
    let (l0, l1) = find("enlace.qu");
    let (t0, t1) = find("tachado");
    let (m0, m1) = find("Typesetter");
    let rich_spans = vec![
        TextSpan::new(b0, b1, TextSpanStyle { weight: Some(700.0), color: Some(theme.fg_text), ..Default::default() }),
        TextSpan::new(i0, i1, TextSpanStyle { italic: Some(true), color: Some(theme.fg_text), ..Default::default() }),
        TextSpan::new(l0, l1, TextSpanStyle { color: Some(theme.accent), underline: Some(true), ..Default::default() }),
        TextSpan::new(t0, t1, TextSpanStyle { color: Some(theme.fg_destructive), strikethrough: Some(true), ..Default::default() }),
        TextSpan::new(m0, m1, TextSpanStyle { font_family: Some(llimphi_text::MONOSPACE.to_string()), color: Some(rgb(80, 200, 200)), ..Default::default() }),
    ];
    let rich_card = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(150.0) },
        flex_direction: FlexDirection::Column,
        gap: Size { width: length(0.0), height: length(8.0) },
        padding: Rect { left: length(18.0), right: length(18.0), top: length(14.0), bottom: length(14.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(12.0)
    .border(1.0, theme.border)
    .children(vec![
        txt(percent(1.0), 20.0, "Texto rico — spans nativos", 14.5, theme.fg_text).bold(),
        View::<()>::new(Style {
            size: Size { width: percent(1.0), height: length(78.0) },
            ..Default::default()
        })
        .text_spans(parrafo, 13.5, theme.fg_muted, rich_spans, Alignment::Start)
        .line_height(1.45),
    ]);

    let centro = View::<()>::new(Style {
        size: Size { width: auto(), height: percent(1.0) },
        flex_grow: 1.0,
        flex_direction: FlexDirection::Column,
        gap: Size { width: length(0.0), height: length(14.0) },
        padding: Rect { left: length(14.0), right: length(14.0), top: length(14.0), bottom: length(14.0) },
        ..Default::default()
    })
    .children(vec![editor, rich_card]);

    // ───────────────────────── columna derecha ─────────────────────────
    // 1) Tarjeta de métricas con gradiente + sombra (el look "hero card").
    let grad_hero = Gradient::new_linear(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
        .with_stops([rgb(64, 92, 180), rgb(34, 46, 96)].as_slice());
    let metrica = |valor: &str, label: &str| {
        View::<()>::new(Style {
            size: Size { width: length(88.0), height: auto() },
            flex_direction: FlexDirection::Column,
            gap: Size { width: length(0.0), height: length(2.0) },
            ..Default::default()
        })
        .children(vec![
            txt(length(88.0), 26.0, valor, 21.0, rgb(240, 244, 252)).bold(),
            txt(length(88.0), 15.0, label, 11.0, rgba(214, 222, 240, 190)),
        ])
    };
    let hero = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(120.0) },
        flex_direction: FlexDirection::Column,
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(0.0), height: length(10.0) },
        padding: Rect { left: length(18.0), right: length(18.0), top: length(12.0), bottom: length(12.0) },
        ..Default::default()
    })
    .fill_gradient(grad_hero)
    .radius(14.0)
    .border(1.0, rgba(150, 175, 240, 120))
    .shadow(Shadow::soft(120, 28.0).offset(0.0, 14.0))
    .children(vec![
        txt(percent(1.0), 16.0, "RENDER · ÚLTIMO FRAME", 11.0, rgba(214, 222, 240, 200)).bold(),
        View::<()>::new(Style {
            flex_direction: FlexDirection::Row,
            gap: Size { width: length(26.0), height: length(0.0) },
            ..Default::default()
        })
        .children(vec![metrica("1.8 ms", "scene → GPU"), metrica("2 411", "nodos"), metrica("60 fps", "vsync")]),
    ]);

    // 2) Mini gráfico de barras: puros rects con gradiente, alineados al piso.
    let alturas = [34.0_f32, 52.0, 41.0, 66.0, 58.0, 78.0, 49.0, 88.0, 71.0, 60.0, 94.0, 80.0];
    let barras: Vec<View<()>> = alturas
        .iter()
        .enumerate()
        .map(|(i, &h)| {
            let g = if i == 10 {
                Gradient::new_linear(Point::new(0.0, 0.0), Point::new(0.0, 1.0))
                    .with_stops([rgb(120, 220, 200), rgb(60, 150, 140)].as_slice())
            } else {
                Gradient::new_linear(Point::new(0.0, 0.0), Point::new(0.0, 1.0))
                    .with_stops([rgb(110, 140, 220), rgb(58, 78, 128)].as_slice())
            };
            rect(15.0, h).radius_corners(4.0, 4.0, 0.0, 0.0).fill_gradient(g)
        })
        .collect();
    let chart = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(168.0) },
        flex_direction: FlexDirection::Column,
        gap: Size { width: length(0.0), height: length(10.0) },
        padding: Rect { left: length(18.0), right: length(18.0), top: length(14.0), bottom: length(14.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(12.0)
    .border(1.0, theme.border)
    .children(vec![
        View::<()>::new(Style {
            size: Size { width: percent(1.0), height: length(18.0) },
            flex_direction: FlexDirection::Row,
            justify_content: Some(JustifyContent::SpaceBetween),
            ..Default::default()
        })
        .children(vec![
            txt(length(200.0), 18.0, "Throughput del raster", 13.0, theme.fg_text).bold(),
            View::<()>::new(Style {
                size: Size { width: length(70.0), height: length(16.0) },
                ..Default::default()
            })
            .text_aligned("12 frames".to_string(), 11.0, theme.fg_placeholder, Alignment::End),
        ]),
        View::<()>::new(Style {
            size: Size { width: percent(1.0), height: length(94.0) },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::FlexEnd),
            justify_content: Some(JustifyContent::SpaceBetween),
            ..Default::default()
        })
        .children(barras),
    ]);

    // 3) Botones / chips — el acento del theme en acción.
    let boton = |label: &str, primario: bool| {
        let base = View::<()>::new(Style {
            size: Size { width: auto(), height: length(34.0) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            padding: Rect { left: length(18.0), right: length(18.0), top: length(0.0), bottom: length(0.0) },
            ..Default::default()
        })
        .radius(17.0);
        if primario {
            base.fill_gradient(
                Gradient::new_linear(Point::new(0.0, 0.0), Point::new(0.0, 1.0))
                    .with_stops([rgb(124, 154, 232), rgb(92, 120, 198)].as_slice()),
            )
            .shadow(Shadow::soft(90, 16.0).offset(0.0, 6.0))
            .children(vec![txt(auto(), 18.0, label, 13.0, rgb(244, 247, 255)).bold()])
        } else {
            base.fill(theme.bg_button)
                .border(1.0, theme.border)
                .children(vec![txt(auto(), 18.0, label, 13.0, theme.fg_text)])
        }
    };
    let acciones = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(34.0) },
        flex_direction: FlexDirection::Row,
        gap: Size { width: length(10.0), height: length(0.0) },
        ..Default::default()
    })
    .children(vec![boton("Regenerar cuerpo", true), boton("Difundir", false)]);

    // 4) Tarjeta de hebras (estado del haz multilienzo) — filas con dot de estado.
    let hebra = |de: &str, a: &str, estado: &str, c: Color| {
        View::<()>::new(Style {
            size: Size { width: percent(1.0), height: length(26.0) },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::SpaceBetween),
            ..Default::default()
        })
        .children(vec![
            View::<()>::new(Style {
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(8.0), height: length(0.0) },
                ..Default::default()
            })
            .children(vec![
                rect(7.0, 7.0).radius(3.5).fill(c),
                txt(length(170.0), 16.0, &format!("{de} → {a}"), 12.0, theme.fg_text),
            ]),
            View::<()>::new(Style {
                size: Size { width: length(80.0), height: length(15.0) },
                ..Default::default()
            })
            .text_aligned(estado.to_string(), 11.0, c, Alignment::End),
        ])
    };
    let hebras = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(148.0) },
        flex_direction: FlexDirection::Column,
        gap: Size { width: length(0.0), height: length(6.0) },
        padding: Rect { left: length(18.0), right: length(18.0), top: length(14.0), bottom: length(14.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(12.0)
    .border(1.0, theme.border)
    .children(vec![
        txt(percent(1.0), 18.0, "Hebras del haz", 13.0, theme.fg_text).bold(),
        hebra("español", "english", "stale", rgb(229, 192, 123)),
        hebra("español", "runasimi", "al día", rgb(152, 195, 121)),
        hebra("español", "resumen", "al día", rgb(152, 195, 121)),
        hebra("english", "tono formal", "derivando…", theme.accent),
    ]);

    let derecha = View::<()>::new(Style {
        size: Size { width: length(330.0), height: percent(1.0) },
        flex_direction: FlexDirection::Column,
        gap: Size { width: length(0.0), height: length(14.0) },
        padding: Rect { left: length(0.0), right: length(14.0), top: length(14.0), bottom: length(14.0) },
        ..Default::default()
    })
    .children(vec![hero, chart, acciones, hebras]);

    // ───────────────────────── status bar ─────────────────────────
    let status_item = |w: f32, s: &str, c: Color| txt(length(w), 16.0, s, 11.5, c);
    let statusbar = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: length(30.0) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(18.0), height: length(0.0) },
        padding: Rect { left: length(16.0), right: length(16.0), top: length(0.0), bottom: length(0.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .border(1.0, theme.border)
    .children(vec![
        status_item(70.0, "git · main", theme.accent),
        status_item(250.0, "wgpu 27 · vello 0.7 · taffy · parley", theme.fg_muted),
        status_item(80.0, "BLAKE3 ok", rgb(152, 195, 121)),
        View::<()>::new(Style { flex_grow: 1.0, ..Default::default() }),
        status_item(45.0, "UTF-8", theme.fg_placeholder),
        status_item(85.0, "Ln 7, Col 23", theme.fg_placeholder),
        status_item(50.0, "100 Hz", theme.fg_placeholder),
    ]);

    // ─────────────── toast flotante (absoluto, esquinas asimétricas) ───────────────
    let toast = View::<()>::new(Style {
        position: Position::Absolute,
        inset: Rect { left: auto(), top: auto(), right: length(360.0), bottom: length(48.0) },
        size: Size { width: length(290.0), height: length(64.0) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(12.0), height: length(0.0) },
        padding: Rect { left: length(14.0), right: length(14.0), top: length(0.0), bottom: length(0.0) },
        ..Default::default()
    })
    .fill(rgb(28, 34, 48))
    .radius_corners(18.0, 18.0, 18.0, 4.0)
    .border(1.0, rgba(110, 140, 220, 160))
    .shadow(Shadow::soft(150, 30.0).offset(0.0, 14.0))
    .children(vec![
        rect(34.0, 34.0)
            .radius(10.0)
            .fill_gradient(
                Gradient::new_linear(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
                    .with_stops([rgb(120, 220, 200), rgb(60, 140, 170)].as_slice()),
            ),
        View::<()>::new(Style {
            flex_direction: FlexDirection::Column,
            gap: Size { width: length(0.0), height: length(2.0) },
            ..Default::default()
        })
        .children(vec![
            txt(auto(), 17.0, "Cuerpo regenerado", 13.0, theme.fg_text).bold(),
            txt(auto(), 15.0, "english · 42 átomos realineados", 11.5, theme.fg_muted),
        ]),
    ]);

    // ───────────────────────── árbol raíz ─────────────────────────
    let fila_central = View::<()>::new(Style {
        size: Size { width: percent(1.0), height: auto() },
        flex_grow: 1.0,
        flex_direction: FlexDirection::Row,
        ..Default::default()
    })
    .children(vec![sidebar, centro, derecha]);

    let root = View::<()>::new(Style {
        size: Size { width: length(W as f32), height: length(H as f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![topbar, fila_central, statusbar, toast]);

    // view → layout → scene → render headless → PNG (misma secuencia que el eventloop).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-motor"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer.render_to_view(&hal, &scene, &view, W, H, bg).expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_motor: escrito {out} ({W}x{H})");
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * H as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
