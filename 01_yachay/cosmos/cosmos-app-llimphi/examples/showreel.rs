//! **Showreel** de cosmos — para el README del repo standalone. NO es
//! eye-candy abstracto: la protagonista es la **rueda de carta natal REAL**,
//! compuesta por `cosmos_render::compose_wheel_with_hits` a partir de un
//! `RenderModel` calculado por `cosmos-engine` (VSOP2013) sobre una carta de
//! verdad (Frida Kahlo, 1907-07-06 08:30 LMT, Coyoacán), y pintada por el
//! mismo backend `cosmos-canvas-llimphi` que usa la app en producción.
//!
//! El **estado** del frame se deriva del tiempo normalizado `t∈[0,1]` — sin
//! reloj, sin runtime, sin winit. Cada `DrawCommand` viene del compositor
//! real; lo único que añadimos es coreografía:
//!   - el `detail` del wheel sube (el aro crece y se redibuja con más nitidez),
//!   - las **líneas de aspecto** entran con un *stagger* radial (revelado por
//!     alpha, recortando la lista de comandos que sí emite el compositor),
//!   - un barrido de **selección de cuerpo** resalta un planeta y sus aspectos
//!     (vía `CompositionOpts::selected_body`, exactamente como el click real),
//!   - la rueda se desliza a un costado y entra un **panel de aspectos** con
//!     los datos reales (`RenderModel::aspect_summary`) y las posiciones de los
//!     cuerpos (capa `Bodies` real).
//!
//! Beats (timeline):
//!   1. cold-open: trazo bezier draw-on (firma) sobre negro.
//!   2. la rueda natal aparece y se "dibuja" (anillo → glifos → aspectos).
//!   3. barrido de selección: un cuerpo y sus aspectos se encienden.
//!   4. shell de datos: la rueda al costado + panel de aspectos/posiciones reales.
//!   5. esfera celeste 3D: la vista `Esfera3d` REAL girando (yaw 360° + pitch).
//!   6. cierre: wordmark «cosmos» + subtítulo, frame limpio.
//!
//! Render headless determinista: frame `i` de `N` → `t = i/(N-1)` → View →
//! layout (taffy + parley) → vello::Scene → wgpu → PNG. Idéntico al eventloop.
//!
//! ```text
//! cargo run -p cosmos-app-llimphi --example showreel --release -- \
//!     [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_cosmos`, `n_frames=300`, `W=1600`, `H=900`.
#![allow(dead_code)]

use std::fs::{create_dir_all, File};
use std::io::BufWriter;

use cosmos_model::{
    Chart, ChartId, ChartKind, ContactId, StoredBirthData, StoredChartConfig, TimeCertainty,
};
use cosmos_render::{
    compose_sphere, compose_wheel_with_hits, CompositionOpts, DrawCommand, LayerKind, Palette,
    RenderModel, SphereOpts, SphereView,
};

use cosmos_canvas_llimphi::{canvas_view_ex, ViewTransform};

use llimphi_theme::{motion, Theme};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style,
};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{self, Color, Gradient};
use llimphi_ui::llimphi_raster::vello::kurbo::{Affine, BezPath, Circle, Point, Stroke};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{draw_layout_brush_xf, measurement, Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, PaintRect, Shadow, View};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// Tipo de mensaje del View — el showreel no despacha nada (es headless), así
// que un unit `()` alcanza para todos los nodos.
type Msg = ();

// ───────────────────────── utilidades ─────────────────────────

fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Reescala `t` desde el subintervalo `[lo,hi]` a `[0,1]`, clampado.
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

// ───────────────────────── tema / skin ─────────────────────────

#[derive(Clone)]
struct Skin {
    theme: Theme,
    accent: Color,
    panel: Color,
    panel_hi: Color,
    border: Color,
    fg: Color,
    fg_muted: Color,
    bg: Color,
    /// Fondo del lienzo de la rueda (el mismo que usa la app en tema oscuro).
    wheel_bg: Color,
}

// ───────────────────────── carta + RenderModel reales ─────────────────────────

/// Una carta natal literal (fecha fija — nada de "ahora", el reel es reproducible).
#[allow(clippy::too_many_arguments)]
fn carta_natal(
    label: &str,
    (y, mo, d): (i32, u32, u32),
    (h, mi): (u32, u32),
    tz_min: i32,
    lat: f64,
    lon: f64,
    alt_m: f64,
    lugar: &str,
) -> Chart {
    Chart {
        id: ChartId::new(),
        contact_id: ContactId::new(),
        kind: ChartKind::Natal,
        label: label.to_string(),
        birth_data: StoredBirthData {
            year: y,
            month: mo,
            day: d,
            hour: h,
            minute: mi,
            second: 0.0,
            tz_offset_minutes: tz_min,
            latitude_deg: lat,
            longitude_deg: lon,
            altitude_m: alt_m,
            time_certainty: TimeCertainty::Exact,
            subject_name: Some(label.to_string()),
            birthplace_label: Some(lugar.to_string()),
        },
        config: StoredChartConfig::default(),
        related_chart_id: None,
        created_at_ms: 0,
    }
}

/// Calcula el `RenderModel` real con el motor (mismo camino que `engine::compute`
/// de la app: `compose_with_options` con opciones natales por defecto).
fn render_model() -> (RenderModel, Chart) {
    let frida = carta_natal(
        "Frida Kahlo",
        (1907, 7, 6),
        (8, 30),
        -397, // LMT Coyoacán ≈ −6 h 37 m
        19.3550,
        -99.1622,
        2240.0,
        "Coyoacán, Ciudad de México",
    );
    let opts = cosmos_engine::NatalOptions {
        show_majors: true,
        show_minors: false,
        orb_multiplier: 1.0,
        show_dignities: true,
        harmonic: 1,
    };
    let render = cosmos_engine::compose_with_options(&frida, 0, &[], &opts)
        .or_else(|_| cosmos_engine::compose(&frida, 0, &[]))
        .unwrap_or_else(|_| cosmos_engine::compute_mock(&frida));
    (render, frida)
}

// ───────────────────────── coreografía del wheel ─────────────────────────

/// Distancia al centro del lienzo (`size/2, size/2`) de un punto.
fn dist_to_center(x: f32, y: f32, size: f32) -> f32 {
    let c = size * 0.5;
    ((x - c).powi(2) + (y - c).powi(2)).sqrt()
}

/// Revela progresivamente las **líneas de aspecto** (las que viven en el aro
/// interior, lejos del borde) escalando su alpha por un *stagger* radial:
/// las que cruzan más cerca del centro entran primero. `reveal∈[0,1]` controla
/// cuántas se ven. Las demás primitivas (anillos, glifos, casas) se dejan
/// intactas. Trabaja sobre la lista REAL que emitió `compose_wheel_with_hits`.
fn reveal_aspects(cmds: &mut [DrawCommand], reveal: f32, size: f32) {
    // El aro de aspectos vive dentro de ~0.42·size del centro; las líneas
    // estructurales (cusps, cruz que llega al borde) van más afuera.
    let aspect_r = size * 0.40;
    for cmd in cmds.iter_mut() {
        if let DrawCommand::Line {
            x1,
            y1,
            x2,
            y2,
            color,
            ..
        } = cmd
        {
            let d1 = dist_to_center(*x1, *y1, size);
            let d2 = dist_to_center(*x2, *y2, size);
            // Una línea de aspecto tiene AMBOS extremos dentro del aro interior.
            if d1 < aspect_r && d2 < aspect_r {
                // Stagger: el "tiempo de entrada" de la línea = su radio medio
                // normalizado (centro entra primero, borde del aro al final).
                let rmid = ((d1 + d2) * 0.5 / aspect_r).clamp(0.0, 1.0);
                // Ventana de aparición de 0.35 de ancho recorriendo radio.
                let local = ((reveal * 1.35 - rmid * 1.0) / 0.35).clamp(0.0, 1.0);
                let e = motion::ease_out_cubic(local);
                color.a *= e;
            }
        }
    }
}

/// El View del lienzo de la rueda, compuesto con `detail`/`selected_body`
/// derivados de `t`, y con el revelado de aspectos aplicado.
fn wheel_view(
    render: &RenderModel,
    size: f32,
    detail: f32,
    selected: Option<String>,
    aspect_reveal: f32,
    rot_deg: f32,
    s: &Skin,
) -> View<Msg> {
    let opts = CompositionOpts {
        size,
        rot_offset_deg: rot_deg,
        include_bodies: true,
        palette: Palette::dark(),
        draw_ascensional_cross: true,
        show_coord_labels: false,
        show_minor_aspects: false,
        dial_3d: true,
        selected_body: selected,
        detail: detail.max(0.01),
    };
    let (mut commands, _hits) = compose_wheel_with_hits(render, &opts);
    if aspect_reveal < 0.999 {
        reveal_aspects(&mut commands, aspect_reveal, size);
    }
    canvas_view_ex::<Msg>(commands, size, Some(s.wheel_bg), ViewTransform::default())
}

// ───────────────────────── esfera celeste 3D (real) ─────────────────────────

/// El View de la **esfera celeste 3D** REAL — la misma `compose_sphere` que pinta
/// `ChartView::Esfera3d` en la app: wireframe de la eclíptica + ecuador + grilla,
/// con los planetas natales, las estrellas fijas, las constelaciones y el globo
/// terráqueo interior. La cámara (`yaw`/`pitch`) se anima desde el showreel.
fn sphere_view(render: &RenderModel, size: f32, yaw_deg: f32, pitch_deg: f32, s: &Skin) -> View<Msg> {
    let opts = SphereOpts {
        size,
        palette: Palette::dark(),
        ..Default::default()
    };
    let view = SphereView { yaw_deg, pitch_deg };
    let commands = compose_sphere(render, &view, &opts);
    canvas_view_ex::<Msg>(commands, size, Some(s.wheel_bg), ViewTransform::default())
}

// ───────────────────────── panel de datos (real) ─────────────────────────

/// Capitaliza un identificador agnóstico ("sun" → "Sun", "north_node" → "North node").
fn humanize(id: &str) -> String {
    let mut out = String::new();
    for (i, w) in id.split('_').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut ch = w.chars();
        if let Some(f) = ch.next() {
            out.extend(f.to_uppercase());
            out.push_str(ch.as_str());
        }
    }
    out
}

fn aspect_color(kind: &str, s: &Skin) -> Color {
    match kind {
        "trine" | "sextile" => Color::from_rgba8(0x5E, 0xC8, 0xA8, 0xFF), // verde-teal (armónicos)
        "square" | "opposition" => Color::from_rgba8(0xE0, 0x7A, 0x7A, 0xFF), // rojo (tensión)
        "conjunction" => s.accent,
        _ => s.fg_muted,
    }
}

fn row_line(text: String, size: f32, color: Color, h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text, size, color, Alignment::Start)
}

/// Una fila de aspecto: glifo de aspecto coloreado + "From asp To" + orbe.
fn aspect_row(a: &cosmos_render::AspectSummary, s: &Skin) -> View<Msg> {
    let col = aspect_color(&a.kind, s);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0),
            height: length(0.0),
        },
        ..Default::default()
    })
    .children(vec![
        // Chip coloreado del aspecto (sin tofu: filled View, no unicode).
        View::new(Style {
            size: Size {
                width: length(10.0),
                height: length(10.0),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .radius(3.0)
        .fill(col),
        View::new(Style {
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(
            format!("{} {} {}", humanize(&a.from_body), a.kind, humanize(&a.to_body)),
            12.5,
            s.fg,
            Alignment::Start,
        ),
        View::new(Style {
            size: Size {
                width: length(52.0),
                height: length(18.0),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text_aligned(format!("{:.1}°", a.orb_deg), 11.5, s.fg_muted, Alignment::End),
    ])
}

/// Card de panel con header de acento + filas.
fn panel_card(title: &str, rows: Vec<View<Msg>>, s: &Skin) -> View<Msg> {
    let chip = View::new(Style {
        size: Size {
            width: length(24.0),
            height: length(7.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .radius(4.0)
    .fill(s.accent);
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(10.0),
            height: length(0.0),
        },
        margin: TaffyRect {
            left: length(0.0),
            right: length(0.0),
            top: length(0.0),
            bottom: length(8.0),
        },
        ..Default::default()
    })
    .children(vec![
        chip,
        View::new(Style {
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(title.to_string(), 12.5, s.fg_muted, Alignment::Start)
        .bold(),
    ]);
    let mut kids = vec![header];
    kids.extend(rows);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_shrink: 0.0,
        padding: TaffyRect {
            left: length(18.0),
            right: length(18.0),
            top: length(16.0),
            bottom: length(16.0),
        },
        margin: TaffyRect {
            left: length(0.0),
            right: length(0.0),
            top: length(0.0),
            bottom: length(16.0),
        },
        ..Default::default()
    })
    .fill_gradient(
        Gradient::new_linear(Point::new(0.0, 0.0), Point::new(0.0, 1.0))
            .with_stops([s.panel_hi, s.panel].as_slice()),
    )
    .radius(16.0)
    .border(1.0, s.border)
    .shadow(Shadow::soft(120, 22.0).offset(0.0, 10.0))
    .children(kids)
}

/// Panel de datos a la derecha: posiciones de los cuerpos + tabla de aspectos
/// REALES del `RenderModel`.
fn data_panel(render: &RenderModel, s: &Skin) -> View<Msg> {
    // Posiciones de los cuerpos: capa Bodies real (deg eclíptico → signo+grado).
    let signos = [
        "Aries", "Tauro", "Géminis", "Cáncer", "Leo", "Virgo", "Libra", "Escorpio", "Sagitario",
        "Capricornio", "Acuario", "Piscis",
    ];
    let mut pos_rows: Vec<View<Msg>> = Vec::new();
    for layer in &render.layers {
        if layer.kind == LayerKind::Bodies {
            for g in layer.glyphs.iter().take(10) {
                let deg = g.deg.rem_euclid(360.0);
                let signo = signos[((deg / 30.0) as usize).min(11)];
                let within = deg % 30.0;
                let rx = if g.retrograde { "  ℞" } else { "" };
                pos_rows.push(row_line(
                    format!("{:<13} {:>5.2}° {}{}", humanize(&g.symbol), within, signo, rx),
                    12.0,
                    s.fg,
                    22.0,
                ));
            }
            break;
        }
    }
    if pos_rows.is_empty() {
        pos_rows.push(row_line("—".into(), 12.0, s.fg_muted, 22.0));
    }

    // Aspectos reales, los más cerrados primero (ya vienen ordenados por orbe).
    let asp_rows: Vec<View<Msg>> = render
        .aspect_summary
        .iter()
        .take(8)
        .map(|a| aspect_row(a, s))
        .collect();

    let mut cards = vec![panel_card("Posiciones", pos_rows, s)];
    if !asp_rows.is_empty() {
        cards.push(panel_card("Aspectos", asp_rows, s));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(cards)
}

// ───────────────────────── overlays vector (cold-open + wordmark) ─────────────────────────

fn signature_path(cw: f64, ch: f64) -> BezPath {
    let cx = cw / 2.0;
    let cy = ch / 2.0;
    let mut p = BezPath::new();
    p.move_to((cx - 360.0, cy + 40.0));
    p.curve_to(
        (cx - 150.0, cy - 220.0),
        (cx + 150.0, cy + 220.0),
        (cx + 360.0, cy - 40.0),
    );
    p
}

fn trim_path(full: &BezPath, prog: f64) -> (BezPath, Point) {
    use vello::kurbo::ParamCurve;
    let prog = prog.clamp(0.0, 1.0);
    let mut cubic = None;
    let mut start = Point::ZERO;
    for el in full.elements() {
        match el {
            vello::kurbo::PathEl::MoveTo(p) => start = *p,
            vello::kurbo::PathEl::CurveTo(c1, c2, p) => {
                cubic = Some(vello::kurbo::CubicBez::new(start, *c1, *c2, *p));
            }
            _ => {}
        }
    }
    let mut out = BezPath::new();
    let mut head = start;
    if let Some(cb) = cubic {
        out.move_to(cb.p0);
        let steps = 96;
        for i in 1..=steps {
            let u = (i as f64 / steps as f64) * prog;
            let pt = cb.eval(u);
            out.line_to(pt);
            head = pt;
        }
    }
    (out, head)
}

fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, s: &Skin) {
    // ── COLD OPEN (0–11%) ──────────────────────────────────────────
    let b1 = seg(t, 0.0, 0.11);
    let line_vis = 1.0 - seg(t, 0.11, 0.18);
    if line_vis > 0.001 {
        let path = signature_path(cw, ch);
        let draw_on = motion::ease_out_cubic(seg(t, 0.01, 0.12)) as f64;
        let (trimmed, head) = trim_path(&path, draw_on);
        let line_col = with_alpha(s.accent, 0.9 * line_vis);
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, line_col, None, &trimmed);
        let pop = motion::ease_out_back(b1);
        let r = (4.0 + 7.0 * pop as f64).max(0.0);
        let dot_a = (b1 * line_vis).clamp(0.0, 1.0);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.18 * dot_a),
            None,
            &Circle::new(head, r * 3.2),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, dot_a),
            None,
            &Circle::new(head, r),
        );
    }

    // ── WORDMARK (91–99%) ─────────────────────────────────────────
    let word_in = seg(t, 0.91, 0.97);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a > 0.001 {
        let size = 140.0_f32;
        let layout = ts.layout(
            "cosmos", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false, 0.0, 0.0,
        );
        let m = measurement(&layout);
        let rise = lerp(24.0, 0.0, word_a as f64);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 18.0 + rise;
        let brush = peniko::Brush::Solid(with_alpha(s.fg, word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.93, 1.0));
        if sub_a > 0.001 {
            let ssz = 26.0_f32;
            let sub = ts.layout(
                "astrology & astrometry, in Rust",
                ssz,
                None,
                Alignment::Start,
                1.0,
                false,
                None,
                400.0,
                false,
                false, 0.0, 0.0,
            );
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + m.height as f64 + 18.0;
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(s.accent, sub_a),
                None,
                &Circle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r as f64),
            );
            let sbrush = peniko::Brush::Solid(with_alpha(s.fg_muted, sub_a));
            draw_layout_brush_xf(
                scene,
                &sub,
                &sbrush,
                Affine::translate((sx + dot_r * 2.0 + 14.0, sy)),
            );
        }
    }

    // ── punto teal de firma (esquina inf-der) ───────
    let corner_a = seg(t, 0.04, 0.12) * (1.0 - seg(t, 0.89, 0.93));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.16 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 18.0),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.9 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 6.0),
        );
    }
}

// ───────────────────────── título de la carta (overlay liviano) ─────────────────────────

fn chart_title(t: f32, cw: f64, render: &RenderModel, s: &Skin) -> View<Msg> {
    // Aparece junto con la rueda (16–26%) y se retira ANTES de que la rueda se
    // corra y entre el panel (58–66%), para no pisar el panel ni la rueda.
    let a = (motion::ease_out_cubic(seg(t, 0.16, 0.26)) * (1.0 - seg(t, 0.58, 0.66))).clamp(0.0, 1.0);
    let sub = render
        .subtitle
        .clone()
        .unwrap_or_else(|| "Carta natal".to_string());
    let _ = cw;
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(56.0),
            top: length(48.0),
            right: auto(),
            bottom: auto(),
        },
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(0.0),
            height: length(6.0),
        },
        ..Default::default()
    })
    .alpha(a)
    .children(vec![
        View::new(Style {
            size: Size {
                width: length(420.0),
                height: length(34.0),
            },
            ..Default::default()
        })
        .text_aligned(render.title.clone(), 26.0, s.fg, Alignment::Start)
        .bold(),
        View::new(Style {
            size: Size {
                width: length(420.0),
                height: length(18.0),
            },
            ..Default::default()
        })
        .text_aligned(sub, 13.0, s.fg_muted, Alignment::Start),
    ])
}

// ───────────────────────── la escena por frame ─────────────────────────

fn build_view(t: f32, cw: f64, ch: f64, render: &RenderModel, s: &Skin) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();

    // Fade de la rueda: entra 10–18%, se mantiene, sale 68–72% (despeja la
    // escena para que entre la esfera 3D antes del wordmark).
    let wheel_in = motion::ease_out_cubic(seg(t, 0.10, 0.18));
    let wheel_out = 1.0 - seg(t, 0.68, 0.72);
    let wheel_a = (wheel_in * wheel_out).clamp(0.0, 1.0);

    if wheel_a > 0.001 {
        // detail sube de 0.85 → 1.18 mientras se "dibuja" (10–42%).
        let detail = lerp(0.85, 1.18, motion::ease_in_out_cubic(seg(t, 0.10, 0.42)) as f64) as f32;
        // rotación suave a lo largo del reel (muy leve).
        let rot = lerp(-7.0, 7.0, (t / 0.72).clamp(0.0, 1.0) as f64) as f32;
        // revelado de aspectos: stagger radial (15–38%).
        let aspect_reveal = seg(t, 0.15, 0.38);

        // barrido de selección de cuerpo (40–52%): enciende un planeta + aspectos.
        let sel_phase = seg(t, 0.40, 0.52);
        let selected: Option<String> = if (0.001..0.999).contains(&sel_phase) {
            // recorre Sun → Moon → Venus a lo largo del barrido.
            let bodies = ["sun", "moon", "venus"];
            let idx = ((sel_phase * bodies.len() as f32).floor() as usize).min(bodies.len() - 1);
            Some(bodies[idx].to_string())
        } else {
            None
        };

        // morph de layout: rueda centrada (beats 2-3) → rueda al costado +
        // panel de datos (beat 4, 54–62%, hold hasta el fade).
        let morph = motion::ease_in_out_cubic(seg(t, 0.54, 0.62)) as f64;
        let wheel_size = lerp(ch * 0.86, ch * 0.74, morph) as f32;

        // Geometría de la rueda: centrada vs. corrida a la izquierda.
        let wheel_cx_centered = cw * 0.5;
        let wheel_cx_side = cw * 0.32;
        let wheel_cx = lerp(wheel_cx_centered, wheel_cx_side, morph);
        let wheel_left = wheel_cx - wheel_size as f64 * 0.5;
        let wheel_top = (ch - wheel_size as f64) * 0.5;

        let wheel = View::new(Style {
            position: Position::Absolute,
            inset: TaffyRect {
                left: length(wheel_left as f32),
                top: length(wheel_top as f32),
                right: auto(),
                bottom: auto(),
            },
            size: Size {
                width: length(wheel_size),
                height: length(wheel_size),
            },
            ..Default::default()
        })
        .alpha(wheel_a)
        .children(vec![wheel_view(
            render,
            wheel_size,
            detail,
            selected,
            aspect_reveal,
            rot,
            s,
        )]);
        children.push(wheel);

        // Panel de datos (beat 4): entra deslizando desde la derecha (66–80%).
        let panel_a = (morph as f32 * wheel_out).clamp(0.0, 1.0);
        if panel_a > 0.001 {
            let panel_w = 460.0_f32;
            let panel_x = cw as f32 - panel_w - 56.0;
            let slide = lerp(48.0, 0.0, motion::ease_out_cubic(seg(t, 0.54, 0.64)) as f64);
            let panel = View::new(Style {
                position: Position::Absolute,
                inset: TaffyRect {
                    left: length(panel_x),
                    top: length(64.0),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size {
                    width: length(panel_w),
                    height: length(ch as f32 - 128.0),
                },
                ..Default::default()
            })
            .alpha(panel_a)
            .transform(Affine::translate((slide, 0.0)))
            .children(vec![data_panel(render, s)]);
            children.push(panel);
        }

        // Título de la carta (overlay liviano, esquina sup-izq).
        children.push(chart_title(t, cw, render, s));
    }

    // ── BEAT 3D: la esfera celeste (70–90%) ───────────────────────────
    // La rueda se desvanece (68–72%) y entra la vista 3D REAL — la misma
    // `compose_sphere` de `ChartView::Esfera3d`. La cámara gira: yaw barre
    // ~360° y el pitch oscila levemente para que se lea el volumen.
    let sphere_in = motion::ease_out_cubic(seg(t, 0.70, 0.76));
    let sphere_out = 1.0 - seg(t, 0.88, 0.91);
    let sphere_a = (sphere_in * sphere_out).clamp(0.0, 1.0);
    if sphere_a > 0.001 {
        // Progreso interno del beat para la rotación (0 al entrar → 1 al salir).
        let spin = seg(t, 0.70, 0.91);
        // Yaw: parte del ángulo natal de la app (26°) y barre 360° suave.
        let yaw = 26.0 + 360.0 * motion::ease_in_out_cubic(spin);
        // Pitch: vista tres-cuartos (−64°) con una respiración de ±10°.
        let pitch = -64.0 + 10.0 * (spin as f64 * std::f64::consts::PI * 2.0).sin() as f32;

        let sphere_size = (ch * 0.82) as f32;
        let sphere_left = (cw - sphere_size as f64) * 0.5;
        let sphere_top = (ch - sphere_size as f64) * 0.5;

        let sphere = View::new(Style {
            position: Position::Absolute,
            inset: TaffyRect {
                left: length(sphere_left as f32),
                top: length(sphere_top as f32),
                right: auto(),
                bottom: auto(),
            },
            size: Size {
                width: length(sphere_size),
                height: length(sphere_size),
            },
            ..Default::default()
        })
        .alpha(sphere_a)
        .children(vec![sphere_view(render, sphere_size, yaw, pitch, s)]);
        children.push(sphere);

        // Rótulo sutil "esfera celeste · 3D" (esquina sup-izq), con el mismo
        // estilo discreto que el título de la carta.
        let label_a = (motion::ease_out_cubic(seg(t, 0.72, 0.78)) * sphere_out).clamp(0.0, 1.0);
        if label_a > 0.001 {
            let label = View::new(Style {
                position: Position::Absolute,
                inset: TaffyRect {
                    left: length(56.0),
                    top: length(48.0),
                    right: auto(),
                    bottom: auto(),
                },
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(10.0),
                    height: length(0.0),
                },
                ..Default::default()
            })
            .alpha(label_a)
            .children(vec![
                View::new(Style {
                    size: Size {
                        width: length(7.0),
                        height: length(7.0),
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .radius(4.0)
                .fill(s.accent),
                View::new(Style {
                    size: Size {
                        width: length(360.0),
                        height: length(22.0),
                    },
                    ..Default::default()
                })
                .text_aligned("esfera celeste · 3D".to_string(), 15.0, s.fg_muted, Alignment::Start),
            ]);
            children.push(label);
        }
    }

    // Overlay full-screen del vector (cold-open + wordmark).
    let overlay = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0),
            top: length(0.0),
            right: length(0.0),
            bottom: length(0.0),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with({
        let s = s.clone();
        move |scene, ts, _rect: PaintRect| {
            draw_overlays(scene, ts, t, cw, ch, &s);
        }
    });
    children.push(overlay);

    // Fondo: gradiente radial muy sutil (negro elegante) — espacio negativo.
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        position: Position::Relative,
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(s.bg)
    .children(children)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "showreel_frames_cosmos".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    rimay_localize::init();

    let theme = Theme::dark();
    let accent = Color::from_rgba8(0x2B, 0xD9, 0xA6, 0xFF); // teal #2BD9A6 (acento firma)
    let skin = Skin {
        accent,
        panel: theme.bg_panel,
        panel_hi: theme.bg_button,
        border: theme.border,
        fg: theme.fg_text,
        fg_muted: theme.fg_muted,
        bg: Color::from_rgba8(8, 9, 14, 255), // negro elegante (un toque más oscuro que el app bg)
        wheel_bg: Color::from_rgba8(14, 15, 22, 255), // mismo fondo de lienzo que cosmos en oscuro
        theme,
    };
    let [br, bg, bb, _] = skin.bg.components;
    let base = Color::from_rgba8((br * 255.0) as u8, (bg * 255.0) as u8, (bb * 255.0) as u8, 255);

    eprintln!("showreel-cosmos: calculando RenderModel (VSOP2013)…");
    let (render, _chart) = render_model();
    eprintln!(
        "showreel-cosmos: carta «{}» — {} aspectos, {} capas",
        render.title,
        render.aspect_summary.len(),
        render.layers.len()
    );

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showreel-cosmos"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
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

    let mut ts = Typesetter::new();
    let cw = w as f64;
    let ch = h as f64;

    for i in 0..n {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };
        let root = build_view(t, cw, ch, &render, &skin);

        let mut layout = LayoutTree::new();
        let mounted = mount(&mut layout, root);
        let computed = {
            let tmap = &mounted.text_measures;
            layout
                .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                        None => taffy::Size::ZERO,
                    }
                })
                .expect("layout")
        };
        let mut scene = vello::Scene::new();
        paint(&mut scene, &mounted, &computed, &mut ts, None, None);

        renderer
            .render_to_view(&hal, &scene, &view, w, h, base)
            .expect("render_to_view");
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("showreel-cosmos: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("showreel-cosmos: {n} frames en {out_dir}/ ({w}x{h})");
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str, w: u32, h: u32) {
    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h as usize) as u64,
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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
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
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for r in 0..h as usize {
        let sidx = r * padded;
        pixels.extend_from_slice(&data[sidx..sidx + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
}
