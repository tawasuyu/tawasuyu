//! **Showreel** de `minga-explorer-llimphi` — el dashboard de
//! observabilidad del repo Minga (VCS semántico P2P / git-replacement
//! soberano). Para el README del standalone.
//!
//! No es eye-candy abstracto: cada frame reconstruye la **view real** de
//! la app (`Explorer::view` + `view_overlay`) alimentada con un `Model`
//! cuyo estado se deriva del tiempo normalizado `t∈[0,1]`. El repo sled
//! es **real**: se siembra una sola vez con el mismo flujo que
//! `pantallazo_minga` (parse tree-sitter de 8 fuentes reales →
//! nodos AST al CAS → α-hashes al MST → atestaciones Ed25519 que se
//! avalan entre sí). Sobre ese snapshot real animamos:
//!
//!   1. cold-open: trazo bezier draw-on (firma).
//!   2. entrada escalonada de los tres stat-cards (nodos · atestaciones ·
//!      MST), cada uno apareciendo con pop + fade.
//!   3. count-up: los valores grandes suben de 0 a su total real, y las
//!      filas "recent (N)" se revelan progresivamente.
//!   4. cierre: wordmark «minga» + subtítulo «P2P sync & history, in
//!      Rust», frame limpio para screenshot.
//!
//! El render es **headless y determinista** (sin reloj, sin runtime, sin
//! winit): frame `i` de `N` → `t = i/(N-1)` → Model(t) → view → layout →
//! vello::Scene → wgpu → PNG. El cold-open y el wordmark son `paint_with`
//! sobre un nodo overlay full-screen superpuesto a la view real.
//!
//! ```text
//! cargo run -p minga-explorer-llimphi --example minga_showreel --release -- \
//!     [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_minga`, `n_frames=300`, `W=1600`, `H=900`.
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos su módulo real por
// `#[path]` para llamar exactamente la misma `view` que pinta la app.
#[path = "../src/main.rs"]
mod app;

use std::fs::{create_dir_all, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{auto, length, percent, Position, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{self, Color};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{
    draw_layout_brush_xf, measurement, Alignment, Typesetter,
};
use llimphi_ui::{measure_text_node, mount, paint, App, Mounted, PaintRect, View};
use llimphi_motion::motion;
use vello::kurbo::{Affine, BezPath, Circle, Point, Stroke};

use minga_core::alpha::hash_alpha_with;
use minga_core::parse::Dialect;
use minga_core::{Attestation, Keypair};
use minga_store::PersistentRepo;

use crate::app::{Explorer, Model, Msg, RepoSnapshot};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Teal de firma de la suite (acento del cold-open / wordmark).
const SIGNATURE: Color = Color::from_rgba8(0x2B, 0xD9, 0xA6, 0xFF);

// ───────────────────────── utilidades de tiempo / color ─────────────────────────

fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Reescala `t` desde `[lo,hi]` a `[0,1]`, clampado.
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

// ───────────────────────── siembra del repo real ─────────────────────────

/// Las ocho fuentes que ingerimos al repo demo — fragmentos creíbles de
/// la propia temática de la suite (sync de pares, acequias comunales,
/// dedup de bloques). Cada una entra por el parser tree-sitter real de
/// su dialecto, así los `kind` de los nodos recientes son los de verdad.
const FUENTES: &[(Dialect, &str)] = &[
    (
        Dialect::Rust,
        r#"//! Anuncio periódico de raíces locales a los pares de la minga.
pub struct Anunciante { intervalo_ms: u64, max_lote: usize }
impl Anunciante {
    pub fn nuevo(intervalo_ms: u64) -> Self { Self { intervalo_ms, max_lote: 32 } }
    pub fn lote<'a>(&self, raices: &'a [[u8; 32]]) -> &'a [[u8; 32]] {
        let n = raices.len().min(self.max_lote);
        &raices[..n]
    }
}
"#,
    ),
    (
        Dialect::Rust,
        r#"use std::collections::BTreeMap;
pub struct TablaPares { pares: BTreeMap<String, u32> }
impl TablaPares {
    pub fn observar(&mut self, did: &str, latencia_ms: u32) {
        self.pares.insert(did.to_string(), latencia_ms);
    }
    pub fn mejores(&self, k: usize) -> Vec<&str> {
        let mut v: Vec<_> = self.pares.iter().collect();
        v.sort_by_key(|(_, lat)| **lat);
        v.into_iter().take(k).map(|(d, _)| d.as_str()).collect()
    }
}
"#,
    ),
    (
        Dialect::Rust,
        r#"pub fn alcanzables(raices: &[u64], hijos: impl Fn(u64) -> Vec<u64>) -> Vec<u64> {
    let mut vivos = Vec::new();
    let mut pila: Vec<u64> = raices.to_vec();
    while let Some(h) = pila.pop() {
        if !vivos.contains(&h) { vivos.push(h); pila.extend(hijos(h)); }
    }
    vivos
}
"#,
    ),
    (
        Dialect::Python,
        r#""""Reparto de turnos de riego para la acequia comunal."""
def repartir(turnos, familias):
    asignados = {}
    for i, familia in enumerate(familias):
        asignados[familia] = turnos[i % len(turnos)]
    return asignados
def proximo(asignados, familia):
    return asignados.get(familia, "sin turno")
"#,
    ),
    (
        Dialect::Python,
        r#"import hashlib
def huella(ruta):
    h = hashlib.blake2b(digest_size=32)
    with open(ruta, "rb") as f:
        for bloque in iter(lambda: f.read(65536), b""):
            h.update(bloque)
    return h.hexdigest()
"#,
    ),
    (
        Dialect::TypeScript,
        r#"export interface Par { did: string; latenciaMs: number; }
export function ordenarPares(pares: Par[]): Par[] {
  return [...pares].sort((a, b) => a.latenciaMs - b.latenciaMs);
}
"#,
    ),
    (
        Dialect::JavaScript,
        r#"export function trocear(buffer, tam = 4096) {
  const trozos = [];
  for (let i = 0; i < buffer.length; i += tam) {
    trozos.push(buffer.slice(i, i + tam));
  }
  return trozos;
}
"#,
    ),
    (
        Dialect::Go,
        r#"package almacen
import "bytes"
func Dedup(bloques [][]byte) [][]byte {
	var unicos [][]byte
	for _, b := range bloques {
		repetido := false
		for _, u := range unicos {
			if bytes.Equal(u, b) { repetido = true; break }
		}
		if !repetido { unicos = append(unicos, b) }
	}
	return unicos
}
"#,
    ),
];

/// Siembra el repo sled en `<repo_path>/repo` con el mismo flujo que
/// `minga ingest` + `minga sign`. Borra el repo previo para que el
/// showreel sea reproducible.
fn sembrar_repo(repo_path: &Path) {
    let _ = std::fs::remove_dir_all(repo_path);
    std::fs::create_dir_all(repo_path).expect("crear dir del repo demo");
    let repo = PersistentRepo::open(repo_path.join("repo")).expect("abrir repo sled demo");

    let autores = [
        Keypair::from_seed(b"amaru-semilla-minga-showreel--01"),
        Keypair::from_seed(b"killa-semilla-minga-showreel--02"),
        Keypair::from_seed(b"inti--semilla-minga-showreel--03"),
    ];

    for (i, (dialect, source)) in FUENTES.iter().enumerate() {
        let node = dialect.parse(source).expect("parse tree-sitter demo");
        let struct_hash = repo.nodes.put(&node).expect("put del árbol al CAS");
        let alpha = hash_alpha_with(*dialect, &node);
        repo.roots
            .put(alpha, struct_hash, *dialect)
            .expect("registrar raíz");
        repo.mst.insert(alpha).expect("clave al MST");

        let autor = &autores[i % autores.len()];
        let aval = &autores[(i + 1) % autores.len()];
        repo.attestations
            .add(Attestation::create(autor, alpha))
            .expect("atestación del autor");
        repo.attestations
            .add(Attestation::create(aval, alpha))
            .expect("atestación del aval");
    }

    repo.flush().expect("flush del repo demo");
}

// ───────────────────────── snapshot animado ─────────────────────────

/// Deriva un snapshot "parcial" del snapshot total para el frame `t`.
///
/// - Los counts grandes hacen count-up de 0 → total (cada card con su
///   propio rampa escalonada).
/// - Las filas "recent (N)" se revelan progresivamente: primero 0, luego
///   se van sumando hasta el total real.
///
/// Es el mismo tipo `RepoSnapshot` que produce `load_snapshot`, sólo que
/// con números intermedios — la view real no distingue.
fn snapshot_animado(full: &RepoSnapshot, t: f32) -> RepoSnapshot {
    // Cada card cuenta en su propia ventana (escalonado).
    let p_nodes = motion::ease_out_cubic(seg(t, 0.30, 0.66));
    let p_att = motion::ease_out_cubic(seg(t, 0.36, 0.72));
    let p_mst = motion::ease_out_cubic(seg(t, 0.42, 0.78));

    let cnt = |total: usize, p: f32| -> usize { (total as f32 * p).round() as usize };

    // Las filas recientes se revelan de a una en una segunda mitad.
    let reveal = |items: &[(String, String)], p: f32| -> Vec<(String, String)> {
        let n = ((items.len() as f32) * p).round() as usize;
        items.iter().take(n.min(items.len())).cloned().collect()
    };
    let reveal_s = |items: &[String], p: f32| -> Vec<String> {
        let n = ((items.len() as f32) * p).round() as usize;
        items.iter().take(n.min(items.len())).cloned().collect()
    };

    // Las listas aparecen un poco después del count.
    let r_nodes = motion::ease_out_cubic(seg(t, 0.50, 0.74));
    let r_att = motion::ease_out_cubic(seg(t, 0.54, 0.78));
    let r_mst = motion::ease_out_cubic(seg(t, 0.58, 0.82));

    RepoSnapshot {
        nodes: cnt(full.nodes, p_nodes),
        attestations: cnt(full.attestations, p_att),
        mst_keys: cnt(full.mst_keys, p_mst),
        recent_nodes: reveal(&full.recent_nodes, r_nodes),
        recent_attestations: reveal(&full.recent_attestations, r_att),
        recent_mst_keys: reveal_s(&full.recent_mst_keys, r_mst),
    }
}

/// Construye el `Model` del frame: el mismo estado que tendría la app
/// tras un refresh exitoso, con el snapshot derivado de `t`.
fn modelo_frame(repo_path: PathBuf, full: &RepoSnapshot, t: f32) -> Model {
    Model {
        theme: Theme::dark(),
        repo_path,
        snapshot: Some(snapshot_animado(full, t)),
        error: None,
        last_load_ms: 3,
        _wawa_watcher: None,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        context_menu: None,
        toasts: Vec::new(),
        next_toast: 0,
        ticking: false,
    }
}

// ───────────────────────── overlays vector (cold-open + wordmark) ─────────────────────────

/// Curva bezier "firma" del cold-open.
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

/// Recorta un `BezPath` cúbico a su fracción inicial `prog`.
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

/// Cold-open + wordmark + punto firma sobre un nodo full-screen.
fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64) {
    // ── COLD OPEN (0–12%) ──────────────────────────────────────────
    let b1 = seg(t, 0.0, 0.12);
    let line_vis = 1.0 - seg(t, 0.12, 0.20);
    if line_vis > 0.001 {
        let path = signature_path(cw, ch);
        let draw_on = motion::ease_out_cubic(seg(t, 0.02, 0.13)) as f64;
        let (trimmed, head) = trim_path(&path, draw_on);
        let line_col = with_alpha(SIGNATURE, 0.9 * line_vis);
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, line_col, None, &trimmed);
        let pop = motion::ease_out_back(b1);
        let r = (4.0 + 7.0 * pop as f64).max(0.0);
        let dot_a = (b1 * line_vis).clamp(0.0, 1.0);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(SIGNATURE, 0.18 * dot_a),
            None,
            &Circle::new(head, r * 3.2),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(SIGNATURE, dot_a),
            None,
            &Circle::new(head, r),
        );
    }

    // ── WORDMARK (84–100%) ─────────────────────────────────────────
    let word_in = seg(t, 0.86, 0.96);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a > 0.001 {
        let size = 132.0_f32;
        let layout = ts.layout(
            "minga", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false, 0.0, 0.0,
        );
        let m = measurement(&layout);
        let rise = lerp(24.0, 0.0, word_a as f64);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 18.0 + rise;
        let brush = peniko::Brush::Solid(with_alpha(Color::from_rgba8(0xEC, 0xEF, 0xF4, 0xFF), word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.90, 1.0));
        if sub_a > 0.001 {
            let ssz = 26.0_f32;
            let sub = ts.layout(
                "P2P sync & history, in Rust", ssz, None, Alignment::Start, 1.0, false, None,
                400.0, false, false, 0.0, 0.0,
            );
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + m.height as f64 + 18.0;
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(SIGNATURE, sub_a),
                None,
                &Circle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r as f64),
            );
            let sbrush =
                peniko::Brush::Solid(with_alpha(Color::from_rgba8(0xA8, 0xB2, 0xC0, 0xFF), sub_a));
            draw_layout_brush_xf(
                scene,
                &sub,
                &sbrush,
                Affine::translate((sx + dot_r * 2.0 + 14.0, sy)),
            );
        }
    }

    // ── punto teal de firma (esquina inf-der) ──────────────────────
    let corner_a = seg(t, 0.04, 0.12) * (1.0 - seg(t, 0.82, 0.88));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(SIGNATURE, 0.16 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 18.0),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(SIGNATURE, 0.9 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 6.0),
        );
    }
}

// ───────────────────────── el frame completo ─────────────────────────

/// mount → layout → paint de un árbol de views a la `Scene` compartida.
fn pintar_arbol(scene: &mut vello::Scene, ts: &mut Typesetter, view: View<Msg>, w: u32, h: u32) {
    let mut layout = LayoutTree::new();
    let mounted: Mounted<Msg> = mount(&mut layout, view);
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    paint(scene, &mounted, &computed, ts, None, None);
}

/// Envuelve la view real de la app en un contenedor que aplica un
/// fade + slide-in global (entrada del dashboard) y fade-out antes del
/// wordmark, y le superpone el overlay vector. Así reusamos la `view`
/// REAL sin tocar su layout interno.
fn build_view(t: f32, cw: f64, ch: f64, repo_path: &Path, full: &RepoSnapshot) -> View<Msg> {
    // Entrada del dashboard (18–32%) y salida antes del wordmark (82–88%).
    let enter = motion::ease_out_cubic(seg(t, 0.16, 0.34));
    let exit = 1.0 - motion::ease_in_out_cubic(seg(t, 0.82, 0.88));
    let dash_alpha = (enter * exit).clamp(0.0, 1.0);

    let model = modelo_frame(repo_path.to_path_buf(), full, t);
    let real_view = Explorer::view(&model);

    // Bound del ancho como una ventana real (no se toca el layout interno
    // de la app: sólo la enmarcamos en una columna centrada ~1100px, como
    // si fuera una ventana de tamaño razonable en vez de estirada a 1600).
    let win_w = 1120.0_f32;
    let win = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(((cw as f32 - win_w) / 2.0).max(0.0)),
            top: length(0.0),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(win_w),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![real_view]);

    // Slide vertical sutil + escala de entrada centrada.
    let rise = lerp(28.0, 0.0, enter as f64);
    let scale = lerp(0.972, 1.0, enter as f64);
    let cx = cw / 2.0;
    let cy = ch / 2.0;
    let xf = Affine::translate((cx, cy + rise))
        * Affine::scale(scale)
        * Affine::translate((-cx, -cy));

    let dash = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
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
    .transform(xf)
    .alpha(dash_alpha as f32)
    .children(vec![win]);

    let overlay = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
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
    .paint_with(move |scene, ts, _rect: PaintRect| {
        draw_overlays(scene, ts, t, cw, ch);
    });

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        position: Position::Relative,
        ..Default::default()
    })
    .fill(Theme::dark().bg_app)
    .children(vec![dash, overlay])
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "showreel_frames_minga".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    rimay_localize::init();
    let _ = rimay_localize::set_locale("es");

    // Sembrar el repo real una sola vez y cargar el snapshot total.
    let repo_path = PathBuf::from("/tmp/minga-showreel-repo");
    sembrar_repo(&repo_path);
    // sled libera su lock de archivo (fs2) en el drop del handle, pero la
    // liberación puede demorar unos ms tras salir de `sembrar_repo`. Un
    // retry corto absorbe esa ventana sin volver el render flaky.
    let full = {
        let mut last_err = String::new();
        let mut got = None;
        for _ in 0..40 {
            match app::load_snapshot(&repo_path) {
                Ok(s) => {
                    got = Some(s);
                    break;
                }
                Err(e) => {
                    last_err = e;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
        got.unwrap_or_else(|| panic!("snapshot del repo sembrado: {last_err}"))
    };
    eprintln!(
        "minga_showreel: repo sembrado → {} nodos · {} atestaciones · {} claves MST",
        full.nodes, full.attestations, full.mst_keys
    );

    let theme = Theme::dark();
    let [r, g, b, _] = theme.bg_app.components;
    let base = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("minga-showreel"),
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

        let root = build_view(t, cw, ch, &repo_path, &full);

        let mut scene = vello::Scene::new();
        pintar_arbol(&mut scene, &mut ts, root, w, h);

        renderer
            .render_to_view(&hal, &scene, &view, w, h, base)
            .expect("render_to_view");
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("minga_showreel: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("minga_showreel: {n} frames en {out_dir}/ ({w}x{h})");
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
    for row in 0..h as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
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
