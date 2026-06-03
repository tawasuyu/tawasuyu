//! `cosmos-canvas-llimphi` — backend Llimphi del lienzo astrológico.
//!
//! Toma la lista de [`DrawCommand`] agnóstica que produce
//! `cosmos-render::compose_wheel` y la pinta con vello. Sin estado
//! entre frames — el host reconstruye el View con la lista de
//! comandos del frame actual; idéntico contrato que
//! `dominium-canvas-llimphi`.
//!
//! La lista de `DrawCommand` está en coordenadas locales del wheel
//! (centrada en `(size/2, size/2)` con `size = opts.size`). Acá
//! traducimos a coordenadas absolutas del rect del nodo, centrando
//! el wheel y aplicando un aspect-fit si el rect no es cuadrado
//! (se usa el lado menor + offset). Tipografía vía llimphi-text con
//! el Typesetter cacheado del runtime — los glyphs simbólicos
//! (`"sun"`, `"aries"`, etc.) los rendereamos como letras unicode
//! astronómicas estándar (☉ ☽ ♈…) si están en el font del sistema;
//! sino caen al texto del campo `symbol` que viene en `Glyph`.

#![forbid(unsafe_code)]

use cosmos_render::{DrawCommand, TextAnchor};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle as KurboCircle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{layout_block, Alignment, TextBlock, Typesetter};
use llimphi_ui::{PaintRect, View};

/// Zoom + paneo aplicados sobre el aspect-fit base del canvas. `zoom`
/// multiplica la escala; `pan` desplaza el origen en píxeles de pantalla.
/// `Default` (zoom 1, pan 0) = aspect-fit centrado puro.
#[derive(Debug, Clone, Copy)]
pub struct ViewTransform {
    pub zoom: f32,
    pub pan: (f32, f32),
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: (0.0, 0.0),
        }
    }
}

/// Escala y offset (en coords de pantalla) para un rect dado y transform.
fn fit(rect_w: f32, rect_h: f32, wheel_size: f32, t: ViewTransform) -> (f64, f64, f64) {
    let scale = (rect_w.min(rect_h) / wheel_size) as f64 * t.zoom.max(0.01) as f64;
    let disp = wheel_size as f64 * scale;
    let off_x = (rect_w as f64 - disp) * 0.5 + t.pan.0 as f64;
    let off_y = (rect_h as f64 - disp) * 0.5 + t.pan.1 as f64;
    (scale, off_x, off_y)
}

/// Construye un View que pinta `commands` centrados en su rect.
///
/// `wheel_size` debe coincidir con `CompositionOpts::size` que se
/// pasó a `compose_wheel` — define el cuadrado lógico donde viven los
/// comandos. El canvas aplica un aspect-fit centrado al rect que le
/// asignó taffy.
pub fn canvas_view<Msg>(
    commands: Vec<DrawCommand>,
    wheel_size: f32,
    background: Option<Color>,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    canvas_view_ex(commands, wheel_size, background, ViewTransform::default())
}

/// Como [`canvas_view`] pero con zoom + paneo.
pub fn canvas_view_ex<Msg>(
    commands: Vec<DrawCommand>,
    wheel_size: f32,
    background: Option<Color>,
    t: ViewTransform,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    });
    let view = if let Some(bg) = background {
        view.fill(bg)
    } else {
        view
    };
    view.paint_with(move |scene, ts, rect: PaintRect| {
        if commands.is_empty() || wheel_size <= 0.0 {
            return;
        }
        // Aspect-fit centrado + zoom/pan del usuario.
        let (scale, off_local_x, off_local_y) = fit(rect.w, rect.h, wheel_size, t);
        let off_x = rect.x as f64 + off_local_x;
        let off_y = rect.y as f64 + off_local_y;
        // El transform global aplica a las primitivas geométricas; el
        // texto lo posicionamos absoluto (parley no compone bien con
        // transforms para sizing/alignment).
        let xform = Affine::translate((off_x, off_y)) * Affine::scale(scale);

        for cmd in &commands {
            paint_command(scene, ts, cmd, xform, off_x, off_y, scale);
        }
    })
}

/// Variante de [`canvas_view`] que dispara `on_click` cuando el
/// usuario hace click dentro del canvas. El handler recibe las
/// coordenadas del click **ya convertidas a coords del wheel** (mismo
/// espacio en el que se emitieron los `DrawCommand`s), y devuelve
/// `Option<Msg>`. Pensado para hit-testear contra [`WheelHits`].
pub fn canvas_view_clickable<Msg, F>(
    commands: Vec<DrawCommand>,
    wheel_size: f32,
    background: Option<Color>,
    on_click: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(f32, f32) -> Option<Msg> + Send + Sync + 'static,
{
    canvas_view_clickable_ex(
        commands,
        wheel_size,
        background,
        ViewTransform::default(),
        on_click,
    )
}

/// Como [`canvas_view_clickable`] pero con zoom + paneo; el hit-test
/// invierte el mismo transform para que el click siga cayendo sobre el
/// glyph correcto a cualquier zoom/pan.
pub fn canvas_view_clickable_ex<Msg, F>(
    commands: Vec<DrawCommand>,
    wheel_size: f32,
    background: Option<Color>,
    t: ViewTransform,
    on_click: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(f32, f32) -> Option<Msg> + Send + Sync + 'static,
{
    let view = canvas_view_ex::<Msg>(commands, wheel_size, background, t);
    view.on_click_at(move |local_x, local_y, rect_w, rect_h| {
        if wheel_size <= 0.0 {
            return None;
        }
        // Invertir el aspect-fit + zoom/pan que aplica `paint_with`.
        let (scale, off_x, off_y) = fit(rect_w, rect_h, wheel_size, t);
        let wheel_x = (local_x as f64 - off_x) / scale;
        let wheel_y = (local_y as f64 - off_y) / scale;
        on_click(wheel_x as f32, wheel_y as f32)
    })
}

fn paint_command(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut Typesetter,
    cmd: &DrawCommand,
    xform: Affine,
    off_x: f64,
    off_y: f64,
    scale: f64,
) {
    match cmd {
        DrawCommand::Circle { cx, cy, r, stroke, fill, stroke_w } => {
            let c = KurboCircle::new((*cx as f64, *cy as f64), *r as f64);
            if let Some(f) = fill {
                scene.fill(Fill::NonZero, xform, rgba_to_color(*f), None, &c);
            }
            if let Some(s) = stroke {
                scene.stroke(
                    &Stroke::new(*stroke_w as f64),
                    xform,
                    rgba_to_color(*s),
                    None,
                    &c,
                );
            }
        }
        DrawCommand::Line { x1, y1, x2, y2, color, width, dash } => {
            let mut path = BezPath::new();
            path.move_to((*x1 as f64, *y1 as f64));
            path.line_to((*x2 as f64, *y2 as f64));
            let mut stroke = Stroke::new(*width as f64);
            if let Some((on, off)) = dash {
                stroke = stroke.with_dashes(0.0, [*on as f64, *off as f64]);
            }
            scene.stroke(&stroke, xform, rgba_to_color(*color), None, &path);
        }
        DrawCommand::Polygon { points, fill, stroke, stroke_w } => {
            if points.is_empty() {
                return;
            }
            let mut path = BezPath::new();
            let (x0, y0) = points[0];
            path.move_to((x0 as f64, y0 as f64));
            for (x, y) in &points[1..] {
                path.line_to((*x as f64, *y as f64));
            }
            path.close_path();
            if let Some(f) = fill {
                scene.fill(Fill::NonZero, xform, rgba_to_color(*f), None, &path);
            }
            if let Some(s) = stroke {
                scene.stroke(
                    &Stroke::new(*stroke_w as f64),
                    xform,
                    rgba_to_color(*s),
                    None,
                    &path,
                );
            }
        }
        DrawCommand::Path { d, stroke, fill, stroke_w } => {
            // kurbo parsea sintaxis SVG (M/L/C/Q/A/Z) — los glyphs
            // astrológicos vienen de `cosmos_render::glyphs` como
            // strings agnósticas para que el surface no se ate a
            // ninguna fuente.
            let Ok(path) = BezPath::from_svg(d) else {
                eprintln!("cosmos-canvas: path SVG inválido: {d}");
                return;
            };
            if let Some(f) = fill {
                scene.fill(Fill::NonZero, xform, rgba_to_color(*f), None, &path);
            }
            if let Some(s) = stroke {
                scene.stroke(
                    &Stroke::new(*stroke_w as f64),
                    xform,
                    rgba_to_color(*s),
                    None,
                    &path,
                );
            }
        }
        DrawCommand::Text { x, y, content, color, size, anchor } => {
            paint_text(scene, ts, x, y, content, color, size, anchor, off_x, off_y, scale);
        }
        DrawCommand::RadialGradient { cx, cy, r, inner, outer } => {
            use llimphi_ui::llimphi_raster::peniko::Gradient;
            let center = llimphi_ui::llimphi_raster::kurbo::Point::new(*cx as f64, *cy as f64);
            let grad = Gradient::new_radial(center, *r)
                .with_stops([rgba_to_color(*inner), rgba_to_color(*outer)].as_slice());
            let circle = KurboCircle::new((*cx as f64, *cy as f64), *r as f64);
            scene.fill(Fill::NonZero, xform, &grad, None, &circle);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_text(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut Typesetter,
    x: &f32,
    y: &f32,
    content: &str,
    color: &cosmos_render::Rgba,
    size: &f32,
    anchor: &TextAnchor,
    off_x: f64,
    off_y: f64,
    scale: f64,
) {
    let translated = pretty_symbol(content);
    // Coordenadas absolutas del anchor.
    let ax = off_x + *x as f64 * scale;
    let ay = off_y + *y as f64 * scale;
    let size_px = *size * scale as f32;
    let align = match anchor {
        TextAnchor::Start => Alignment::Start,
        TextAnchor::Middle => Alignment::Center,
        TextAnchor::End => Alignment::End,
    };
    let color = rgba_to_color(*color);
    // Para centrar verticalmente alrededor de (ax, ay) medimos primero.
    // Anchor horizontal lo resuelve parley vía `max_width + alignment`
    // si le damos un max_width simétrico al anchor.
    let approx_w = size_px as f64 * translated.chars().count() as f64;
    let (origin_x, max_w) = match anchor {
        TextAnchor::Start => (ax, None),
        TextAnchor::Middle => (ax - approx_w, Some(approx_w as f32 * 2.0)),
        TextAnchor::End => (ax - approx_w, Some(approx_w as f32)),
    };
    let block = TextBlock {
        text: &translated,
        size_px,
        color,
        origin: (origin_x, ay - size_px as f64 * 0.5),
        max_width: max_w,
        alignment: align,
        line_height: 1.0,
        italic: false,
        font_family: None,
    };
    let layout = layout_block(ts, &block);
    llimphi_ui::llimphi_text::draw_layout(scene, &layout, color, block.origin);
}

fn rgba_to_color(c: cosmos_render::Rgba) -> Color {
    let to_byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to_byte(c.r), to_byte(c.g), to_byte(c.b), to_byte(c.a))
}

/// Traduce un identificador simbólico de cosmos-render
/// (`"sun"`, `"aries"`, `"asc"`, etc.) a un glyph unicode astrológico.
/// Si no hay traducción registrada, devuelve el string original — el
/// caller puede pasar texto ya formateado (coord labels) sin que
/// rompa.
fn pretty_symbol(s: &str) -> String {
    match s {
        // Cuerpos clásicos.
        "sun" => "☉".into(),
        "moon" => "☽".into(),
        "mercury" => "☿".into(),
        "venus" => "♀".into(),
        "mars" => "♂".into(),
        "jupiter" => "♃".into(),
        "saturn" => "♄".into(),
        "uranus" => "♅".into(),
        "neptune" => "♆".into(),
        "pluto" => "♇".into(),
        "earth" => "⊕".into(),
        // Puntos del chart.
        "asc" => "Asc".into(),
        "desc" => "Desc".into(),
        "mc" => "MC".into(),
        "ic" => "IC".into(),
        "north_node" | "ascending_node" => "☊".into(),
        "south_node" | "descending_node" => "☋".into(),
        "lilith" => "⚸".into(),
        "chiron" => "⚷".into(),
        // Signos zodiacales.
        "aries" => "♈".into(),
        "taurus" => "♉".into(),
        "gemini" => "♊".into(),
        "cancer" => "♋".into(),
        "leo" => "♌".into(),
        "virgo" => "♍".into(),
        "libra" => "♎".into(),
        "scorpio" => "♏".into(),
        "sagittarius" => "♐".into(),
        "capricorn" => "♑".into(),
        "aquarius" => "♒".into(),
        "pisces" => "♓".into(),
        other => other.to_string(),
    }
}
