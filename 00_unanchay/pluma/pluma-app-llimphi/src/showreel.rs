//! **Showreel** de pluma — para el README del repo standalone. NO es eye-candy
//! abstracto: cada frame reconstruye el árbol `View` de la **vista real** de la
//! app (`crate::view::vista`) sobre un modelo sintético con tres cuerpos
//! paralelos (español original → quechua → english) alineados párrafo-a-párrafo
//! por `CartaHebras`. Es el corazón del mensaje: un documento como **haz de
//! cuerpos** (lienzos) con las **hebras de alineación** entre ellos.
//!
//! El render es headless y determinista (sin reloj, sin runtime, sin winit):
//! frame `i` de `N` → `t = i/(N-1)` → se ajusta el ESTADO del modelo (qué
//! lienzos están seleccionados, qué modo del centro, qué diente) → `vista()` →
//! layout (taffy + parley) → vello::Scene → wgpu → PNG. Encima va un nodo
//! overlay full-screen con el cold-open (trazo bezier draw-on) y el wordmark
//! «pluma» de cierre — la misma técnica que los showreels de llimphi y pata.
//!
//! Timeline (beats):
//!   1. 0–12%   cold-open: trazo firma sobre fondo sobrio.
//!   2. 12–40%  un solo cuerpo (español original) entra; el chrome real aparece.
//!   3. 40–62%  se despliegan los cuerpos hija (quechua, english) al lado, con
//!              las hebras de alineación conectándolos — el multilienzo en Plano.
//!   4. 62–78%  modo Lienzos: el documento como cajas anidadas, in-situ.
//!   5. 78–86%  diente Grafo: el nodegraph de filtros (fuente → ... → línea).
//!   6. 86–100% wordmark «pluma» + subtítulo, frame limpio.

use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint, PaintRect};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Position, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{self, Color};
use llimphi_ui::llimphi_raster::vello::kurbo::{Affine, BezPath, Circle, Point, Stroke};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{draw_layout_brush_xf, measurement, Alignment, Typesetter};
use llimphi_ui::View;
use llimphi_theme::motion;

use pluma_align::{alinear_uno_a_uno, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_llm::{build_client, BackendKind, LlmConfig};
use uuid::Uuid;

use crate::clipboard::ArboardClipboard;
use crate::model::{Filtro, Modo, Model, Msg, NodoFiltro, RAIL_W};
use crate::view::vista;

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Acento «firma» del reel (teal) — mismo del resto de la suite.
fn accent() -> Color {
    Color::from_rgba8(0x2B, 0xD9, 0xA6, 0xFF)
}

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

// ───────────────────────── modelo sintético (haz de cuerpos) ─────────────────────────

fn cuerpo_con_atomos(
    atoms: &mut HashMap<Uuid, NarrativeAtom>,
    branch: &str,
    nombre: &str,
    intencion: Intencion,
    textos: &[&str],
) -> Cuerpo {
    let mut c = Cuerpo::nuevo(branch, nombre, intencion, 100);
    for t in textos {
        let a = NarrativeAtom::new(*t, branch);
        c.agregar(a.id, 101);
        atoms.insert(a.id, a);
    }
    c
}

/// Construye el modelo base del reel: español (original) + quechua + english,
/// con cartas de alineación entre columnas consecutivas. Devuelve además los
/// ids de cada cuerpo, que la timeline usa para revelar columnas de a una.
struct Escena {
    model: Model,
    es: Uuid,
    qu: Uuid,
    en: Uuid,
}

fn escena_base() -> Escena {

    let mut atoms: HashMap<Uuid, NarrativeAtom> = HashMap::new();
    let es = cuerpo_con_atomos(
        &mut atoms,
        "es",
        "amanecer · español (original)",
        Intencion::Original,
        &[
            "# El amanecer en el valle",
            "El cóndor cruzó el cielo del valle al primer rayo del alba.",
            "## Los animales",
            "Las llamas pastaban entre los pastizales del altiplano, sin prisa.",
            "## El telar",
            "Una mujer joven tejía un telar bajo el alero de piedra.",
        ],
    );
    let mut qu = cuerpo_con_atomos(
        &mut atoms,
        "qu",
        "amanecer · quechua",
        Intencion::Traduccion,
        &[
            "# Wayqupi pacha paqariy",
            "Kuntur wayqu hanaqpachata pacha paqariypa ñawpaq k'anchayninpi pasarqa.",
            "## Uywakuna",
            "Llamaqakuna qulla suyup q'achupinpi, mana usqhayllachu, mikhusharqaku.",
            "## Away",
            "Sipas warmiq rumi wasiq hawanpi awayta ruwasharqa.",
        ],
    );
    qu.metadatos.derivado_de = Some(es.id);
    let mut en = cuerpo_con_atomos(
        &mut atoms,
        "en",
        "amanecer · english",
        Intencion::Traduccion,
        &[
            "# Dawn in the valley",
            "The condor crossed the valley sky at the first ray of dawn.",
            "## The animals",
            "The llamas grazed among the highland grasslands, unhurried.",
            "## The loom",
            "A young woman wove on a loom beneath the stone eaves.",
        ],
    );
    en.metadatos.derivado_de = Some(es.id);

    let carta_es_qu = alinear_uno_a_uno(
        &es,
        &qu,
        OrigenAlineamiento::Derivado {
            transformacion: Uuid::new_v4(),
            timestamp: 1,
        },
    );
    let carta_qu_en = alinear_uno_a_uno(
        &qu,
        &en,
        OrigenAlineamiento::Embeddings {
            modelo: "iniy-1".into(),
            timestamp: 2,
        },
    );

    let idx: HashMap<Uuid, &NarrativeAtom> = atoms.iter().map(|(k, v)| (*k, v)).collect();
    let ide = CuerpoIde::from_cuerpo(&es, &idx);
    let mut ides_ro: HashMap<Uuid, CuerpoIde> = HashMap::new();
    ides_ro.insert(qu.id, CuerpoIde::from_cuerpo(&qu, &idx));
    ides_ro.insert(en.id, CuerpoIde::from_cuerpo(&en, &idx));
    drop(idx);

    let (es_id, qu_id, en_id) = (es.id, qu.id, en.id);

    let chat = build_client(&LlmConfig {
        kind: BackendKind::Mock,
        ..Default::default()
    })
    .expect("mock");

    let model = Model {
        cuerpos: vec![es, qu, en],
        atoms,
        cartas: vec![carta_es_qu, carta_qu_en],
        transformaciones: Vec::new(),
        activo: Some(es_id),
        ide,
        modo: Modo::Plano,
        editando: None,
        recorrido_state: pluma_deck_core::RecorridoState::new(),
        salidas: HashMap::new(),
        lienzos_scroll_y: 0.0,
        fase_flujo: 0.0,
        seleccionados: vec![es_id],
        orden_lienzos: vec![es_id, qu_id, en_id],
        ides_ro,
        solo_activo: true,
        scroll_x: 0.0,
        viewport: (1600.0, 900.0),
        diente_activo: 1, // Lienzos (el tree)
        foco_por_hover: false,
        panel_w: 280.0,
        clipboard: ArboardClipboard::new(),
        drag_accum: (0.0, 0.0),
        preset_input: llimphi_widget_text_input::TextInputState::new(),
        preset_focused: false,
        presets: vec![
            "Hacelo más poético".into(),
            "Tono noticiero, frases cortas".into(),
        ],
        grafo: Vec::new(),
        grafo_src: (20.0, 16.0),
        grafo_sink: (20.0, 296.0),
        grafo_input: llimphi_widget_text_input::TextInputState::new(),
        grafo_input_focused: false,
        chat,
        backend_idx: 0,
        en_curso: false,
        ultimo_error: None,
        ultimo_status: "listo".into(),
        path_input: llimphi_widget_text_input::TextInputState::new(),
        path_focused: false,
        find_input: llimphi_widget_text_input::TextInputState::new(),
        find_visible: false,
        find_matches: Vec::new(),
        find_idx: 0,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: llimphi_motion::Tween::idle(1.0),
        edit_menu: None,
        edit_active: usize::MAX,
        edit_anim: llimphi_motion::Tween::idle(1.0),
        delegated: false,
        _host: None,
        host_active_synced: None,
        estilos: std::collections::HashMap::new(),
        diente_estilo_activo: None,
        panel_estilo_w: 280.0,
        objetivo_estilo: crate::model::ObjetivoEstilo::Lienzo,
        estilo_expand: None,
        wizard: None,
        proyectos: vec![crate::model::ProyectoAbierto::vacio("Proyecto")],
        proyecto_activo: 0,
        proyecto_tab: crate::model::ProyectoTab::Historia,
        commit_preview: None,
        push_abierto: false,
        renombrar: None,
        proyectos_recientes: Vec::new(),
    };

    Escena { model, es: es_id, qu: qu_id, en: en_id }
}

// ───────────────────────── timeline: estado del modelo por frame ─────────────────────────

/// Ajusta el ESTADO del modelo según `t∈[0,1]`. No fabrica UI: sólo cambia qué
/// muestra la vista real (cuántos lienzos, qué modo, qué diente) para que la
/// historia avance — la misma máquina de estados que el usuario maneja en vivo.
fn aplicar_timeline(esc: &Escena, m: &mut Model, t: f32) {
    // Beat 2: un solo cuerpo (el original) en el centro.
    // Beat 3 (40–62%): se suman quechua y english como columnas paralelas.
    let revelar_qu = t >= 0.42;
    let revelar_en = t >= 0.50;

    m.solo_activo = !(revelar_qu || revelar_en);
    m.seleccionados = vec![esc.es];
    if revelar_qu {
        m.seleccionados.push(esc.qu);
    }
    if revelar_en {
        m.seleccionados.push(esc.en);
    }

    // Modo del centro: Plano (multilienzo con hebras) → Lienzos (cajas) → Grafo.
    if t >= 0.62 && t < 0.78 {
        m.modo = Modo::Lienzos;
        m.diente_activo = 1;
    } else if t >= 0.78 {
        // Diente Grafo: sembrar un pipeline de ejemplo y mostrar el nodegraph.
        m.modo = Modo::Plano;
        m.diente_activo = 4;
        m.grafo_src = (20.0, 16.0);
        m.grafo = vec![
            NodoFiltro { filtro: Filtro::Concepto("río".into()), x: 20.0, y: 86.0 },
            NodoFiltro { filtro: Filtro::Traducir("en".into()), x: 20.0, y: 156.0 },
            NodoFiltro { filtro: Filtro::Resumir(Some(30)), x: 20.0, y: 226.0 },
        ];
        m.grafo_sink = (20.0, 296.0);
    } else {
        m.modo = Modo::Plano;
        m.diente_activo = 1;
    }

    m.ultimo_status = match m.modo {
        Modo::Lienzos => "documento como lienzos anidados".into(),
        Modo::Presentar => "presentar".into(),
        Modo::Plano if m.diente_activo == 4 => "grafo de filtros → nueva línea".into(),
        Modo::Plano if m.seleccionados.len() >= 2 => "haz de cuerpos alineados por hebras".into(),
        Modo::Plano => "un cuerpo madre".into(),
    };
}

// ───────────────────────── overlays vector (cold-open + wordmark) ─────────────────────────

/// Curva «firma» del cold-open (un trazo de pluma).
fn signature_path(cw: f64, ch: f64) -> BezPath {
    let cx = cw / 2.0;
    let cy = ch / 2.0;
    let mut p = BezPath::new();
    p.move_to((cx - 360.0, cy + 30.0));
    p.curve_to(
        (cx - 120.0, cy - 200.0),
        (cx + 140.0, cy + 210.0),
        (cx + 360.0, cy - 50.0),
    );
    p
}

/// Recorta un `BezPath` cúbico a su fracción inicial `prog`; devuelve la cabeza.
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

fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, fg: Color, fg_muted: Color) {
    let acc = accent();

    // ── velo de entrada: oscurece el chrome bajo el cold-open ───────
    let chrome_in = 1.0 - seg(t, 0.10, 0.18);
    if chrome_in > 0.001 {
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(Color::from_rgba8(8, 10, 13, 255), 0.92 * chrome_in),
            None,
            &vello::kurbo::Rect::new(0.0, 0.0, cw, ch),
        );
    }

    // ── COLD OPEN (0–13%): trazo firma + cabeza teal ────────────────
    let b1 = seg(t, 0.0, 0.11);
    let line_vis = 1.0 - seg(t, 0.12, 0.20);
    if line_vis > 0.001 {
        let path = signature_path(cw, ch);
        let draw_on = motion::ease_out_cubic(seg(t, 0.01, 0.12)) as f64;
        let (trimmed, head) = trim_path(&path, draw_on);
        scene.stroke(
            &Stroke::new(2.2),
            Affine::IDENTITY,
            with_alpha(acc, 0.92 * line_vis),
            None,
            &trimmed,
        );
        let pop = motion::ease_out_back(b1);
        let r = (4.0 + 7.0 * pop as f64).max(0.0);
        let dot_a = (b1 * line_vis).clamp(0.0, 1.0);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(acc, 0.18 * dot_a),
            None,
            &Circle::new(head, r * 3.2),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(acc, dot_a),
            None,
            &Circle::new(head, r),
        );
    }

    // ── WORDMARK (86–100%): «pluma» + subtítulo ─────────────────────
    let veil = seg(t, 0.85, 0.92);
    if veil > 0.001 {
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(Color::from_rgba8(8, 10, 13, 255), 0.94 * veil),
            None,
            &vello::kurbo::Rect::new(0.0, 0.0, cw, ch),
        );
    }
    let word_in = seg(t, 0.87, 0.96);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a > 0.001 {
        let size = 150.0_f32;
        let layout = ts.layout(
            "pluma", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false, 0.0, 0.0,
        );
        let mm = measurement(&layout);
        let rise = lerp(24.0, 0.0, word_a as f64);
        let ox = (cw - mm.width as f64) / 2.0;
        let oy = (ch - mm.height as f64) / 2.0 - 18.0 + rise;
        let brush = peniko::Brush::Solid(with_alpha(fg, word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.90, 1.0));
        if sub_a > 0.001 {
            let ssz = 26.0_f32;
            let sub = ts.layout(
                "multilienzo writing, in Rust", ssz, None, Alignment::Start, 1.0, false, None,
                400.0, false, false, 0.0, 0.0,
            );
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + mm.height as f64 + 18.0;
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(acc, sub_a),
                None,
                &Circle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r as f64),
            );
            let sbrush = peniko::Brush::Solid(with_alpha(fg_muted, sub_a));
            draw_layout_brush_xf(scene, &sub, &sbrush, Affine::translate((sx + dot_r * 2.0 + 14.0, sy)));
        }
    }

    // ── punto teal de firma (esquina inf-der) ──────────────────────
    let corner_a = seg(t, 0.20, 0.30) * (1.0 - seg(t, 0.82, 0.88));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(acc, 0.16 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 18.0),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(acc, 0.9 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 6.0),
        );
    }
}

// ───────────────────────── la escena por frame ─────────────────────────

/// Árbol del frame: la vista REAL del modelo (con su estado derivado de `t`)
/// con un fade-in/slide sutil del chrome, más un overlay full-screen para el
/// cold-open / wordmark.
fn build_view(esc: &Escena, t: f32, cw: f64, ch: f64, fg: Color, fg_muted: Color) -> View<Msg> {
    let mut model = clonar_para_frame(&esc.model);
    aplicar_timeline(esc, &mut model, t);

    // Slide-in/fade del chrome real (12–22%) y fade-out antes del wordmark.
    let slide = motion::ease_out_cubic(seg(t, 0.12, 0.24));
    let chrome_alpha = (slide * (1.0 - seg(t, 0.84, 0.90))).clamp(0.0, 1.0) as f32;
    let chrome_dy = lerp(18.0, 0.0, slide as f64);

    let chrome = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            top: length(0.0_f32),
            left: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .transform(Affine::translate((0.0, chrome_dy)))
    .alpha(chrome_alpha)
    .children(vec![vista(&model)]);

    let overlay = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            top: length(0.0_f32),
            left: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .paint_with(move |scene, ts, _rect: PaintRect| {
        draw_overlays(scene, ts, t, cw, ch, fg, fg_muted);
    });

    let _ = RAIL_W; // referencia muerta para fijar el import semántico del layout.

    View::new(Style {
        position: Position::Relative,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(8, 10, 13, 255))
    .children(vec![chrome, overlay])
}

/// Clona los campos del modelo que la vista lee. `Model` no es `Clone` (tiene
/// `Arc<dyn ChatClient>`, `sled`, etc.), así que rehacemos uno fresco que
/// comparte los `Arc` y copia el resto — barato, una vez por frame.
fn clonar_para_frame(base: &Model) -> Model {
    let idx: HashMap<Uuid, &NarrativeAtom> = base.atoms.iter().map(|(k, v)| (*k, v)).collect();
    let activo = base.activo;
    let ide = activo
        .and_then(|id| base.cuerpos.iter().find(|c| c.id == id))
        .map(|c| CuerpoIde::from_cuerpo(c, &idx))
        .unwrap_or_else(CuerpoIde::nuevo_vacio);
    let mut ides_ro: HashMap<Uuid, CuerpoIde> = HashMap::new();
    for c in &base.cuerpos {
        if Some(c.id) != activo {
            ides_ro.insert(c.id, CuerpoIde::from_cuerpo(c, &idx));
        }
    }
    drop(idx);

    Model {
        cuerpos: base.cuerpos.clone(),
        atoms: base.atoms.clone(),
        cartas: base.cartas.clone(),
        transformaciones: base.transformaciones.clone(),
        activo,
        ide,
        modo: base.modo,
        editando: None,
        recorrido_state: pluma_deck_core::RecorridoState::new(),
        salidas: base.salidas.clone(),
        lienzos_scroll_y: base.lienzos_scroll_y,
        fase_flujo: base.fase_flujo,
        seleccionados: base.seleccionados.clone(),
        orden_lienzos: base.orden_lienzos.clone(),
        ides_ro,
        solo_activo: base.solo_activo,
        scroll_x: base.scroll_x,
        viewport: base.viewport,
        diente_activo: base.diente_activo,
        foco_por_hover: false,
        panel_w: base.panel_w,
        clipboard: ArboardClipboard::new(),
        drag_accum: (0.0, 0.0),
        preset_input: llimphi_widget_text_input::TextInputState::new(),
        preset_focused: false,
        presets: base.presets.clone(),
        grafo: base.grafo.clone(),
        grafo_src: base.grafo_src,
        grafo_sink: base.grafo_sink,
        grafo_input: llimphi_widget_text_input::TextInputState::new(),
        grafo_input_focused: false,
        chat: base.chat.clone(),
        backend_idx: base.backend_idx,
        en_curso: false,
        ultimo_error: None,
        ultimo_status: base.ultimo_status.clone(),
        path_input: llimphi_widget_text_input::TextInputState::new(),
        path_focused: false,
        find_input: llimphi_widget_text_input::TextInputState::new(),
        find_visible: false,
        find_matches: Vec::new(),
        find_idx: 0,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: llimphi_motion::Tween::idle(1.0),
        edit_menu: None,
        edit_active: usize::MAX,
        edit_anim: llimphi_motion::Tween::idle(1.0),
        delegated: false,
        _host: None,
        host_active_synced: None,
        estilos: std::collections::HashMap::new(),
        diente_estilo_activo: None,
        panel_estilo_w: 280.0,
        objetivo_estilo: crate::model::ObjetivoEstilo::Lienzo,
        estilo_expand: None,
        wizard: None,
        proyectos: vec![crate::model::ProyectoAbierto::vacio("Proyecto")],
        proyecto_activo: 0,
        proyecto_tab: crate::model::ProyectoTab::Historia,
        commit_preview: None,
        push_abierto: false,
        renombrar: None,
        proyectos_recientes: Vec::new(),
    }
}

// ───────────────────────── entrada pública ─────────────────────────

/// Renderiza `n` frames del showreel a `out_dir/frame_%04d.png` en `w×h`.
pub fn render_frames(out_dir: &str, n: usize, w: u32, h: u32) {
    create_dir_all(out_dir).expect("mkdir out_dir");

    let theme = llimphi_theme::Theme::dark();
    let fg = theme.fg_text;
    let fg_muted = theme.fg_muted;

    let esc = escena_base();

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showreel-pluma"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
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
    let base = Color::from_rgba8(8, 10, 13, 255);

    for i in 0..n {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };
        let root = build_view(&esc, t, cw, ch, fg, fg_muted);

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
            eprintln!("showreel-pluma: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("showreel-pluma: {n} frames en {out_dir}/ ({w}x{h})");
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
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
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
