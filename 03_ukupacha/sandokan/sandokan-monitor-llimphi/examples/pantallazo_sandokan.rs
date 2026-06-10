//! Pantallazo headless de `sandokan-monitor` — el process monitor de tawasuyu
//! (frontend de `shared/sandokan/SDD.md` §6).
//!
//! Monta las **views reales** de la app (los cuatro modos sobre el mismo
//! `Model`: chrome con menubar/chips/pestañas + tabla **Sistema** con gráficos
//! por core, **Unidades** del plano de control observadas por el contrato
//! `Engine` (tarjetas vivas con sparkline, estados corriendo/parada/fallada/
//! matada y restarts `↻N`), **Mapa** treemap padre/hijo y censo **Wawa**),
//! con un `Model` sembrado con datos demo creíbles y deterministas: un árbol
//! de procesos del host encabezado por `arje-zero` (PID 1) con un build de
//! cargo/rustc ardiendo, y un `MonitorSnapshot` con ocho unidades en estados
//! variados. Nada depende de la hora ni del `/proc` real.
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `agora-app/examples/pantallazo_agora.rs`).
//!
//! `cargo run -p sandokan-monitor-llimphi --example pantallazo_sandokan --release -- [out.png]`
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos su módulo real por `#[path]`
// para pintar exactamente las mismas views que la app (`system_body`,
// `units_body`, `map_body`, `wawa_body`, `header`, `tabs`).
#[path = "../src/main.rs"]
mod app;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use sandokan::lifecycle::LifecycleState;
use sandokan::TelemetryFrame;
use sandokan_monitor_core::{MonitorSnapshot, UnitObservation};
use ulid::Ulid;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{auto, length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, View};
use llimphi_widget_menubar::menubar_view;

use crate::app::{Model, Msg, Sort, SysProc, Tab, WawaApp};

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// RAM total del host demo (32 GiB, en kB) — de acá salen los %MEM.
const MEM_TOTAL_KB: u64 = 32 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Siembra: árbol de procesos del host (modo Sistema/Mapa).
// ---------------------------------------------------------------------------

/// Un proceso demo. `mem_pct` se deriva del RSS contra la RAM total, igual
/// que `ingest_system` lo haría con un barrido real.
#[allow(clippy::too_many_arguments)]
fn proc(
    pid: i32,
    ppid: i32,
    name: &str,
    state: char,
    cpu_pct: f32,
    rss_kb: u64,
    threads: u32,
    uid: u32,
    uptime_secs: u64,
    cmd: &str,
) -> SysProc {
    SysProc {
        pid,
        ppid,
        name: name.into(),
        state,
        cpu_pct,
        mem_pct: (rss_kb as f32 / MEM_TOTAL_KB as f32) * 100.0,
        rss_kb,
        threads,
        uid,
        uptime_secs,
        cmd: cmd.into(),
    }
}

/// El host demo: `arje-zero` como PID 1 con la suite colgando de él (la
/// compila un build de cargo/rustc que arde en CPU), más los kthreads. El
/// vector va en orden %CPU desc — el mismo que dejaría `sort_system` con el
/// orden por defecto (`Sort::Cpu`); el modo árbol lo respeta entre hermanos.
fn sistema_demo() -> Vec<SysProc> {
    const D: u64 = 86_400;
    const H_: u64 = 3_600;
    vec![
        proc(612, 604, "rustc", 'R', 88.4, 1_482_000, 14, 1000, 154, "rustc --crate-name llimphi_raster --edition=2024 02_ruway/llimphi/llimphi-raster/src/lib.rs -C opt-level=3 -C embed-bitcode=no"),
        proc(604, 423, "cargo", 'R', 64.2, 319_488, 9, 1000, 9 * 60 + 12, "cargo build --workspace --release"),
        proc(233, 1, "nahual-sim", 'R', 47.8, 1_433_600, 24, 1000, 2 * H_ + 41 * 60, "nahual-sim --escenario valle-sagrado --ticks 480"),
        proc(236, 233, "nahual-worker", 'R', 31.5, 421_888, 6, 1000, 2 * H_ + 40 * 60, "nahual-worker --shard 0/3"),
        proc(237, 233, "nahual-worker", 'R', 28.0, 409_600, 6, 1000, 2 * H_ + 40 * 60, "nahual-worker --shard 1/3"),
        proc(241, 104, "pluma", 'R', 22.4, 536_576, 18, 1000, 5 * H_ + 8 * 60, "pluma-app --workspace /home/sergio/escritos"),
        proc(238, 233, "nahual-worker", 'R', 19.3, 397_312, 6, 1000, 2 * H_ + 40 * 60, "nahual-worker --shard 2/3"),
        proc(96, 1, "rimay-verbo-daemon", 'S', 14.6, 421_888, 9, 1000, 6 * H_ + 2 * 60, "rimay-verbo-daemon --provider fastembed --dim 384"),
        proc(132, 96, "fastembed-worker", 'R', 11.2, 262_144, 4, 1000, 6 * H_, "fastembed-worker multilingual-e5-small"),
        proc(104, 1, "mirada", 'S', 7.4, 192_512, 12, 1000, 6 * H_ + 14 * 60, "mirada --seat seat0 --gpu vulkan"),
        proc(158, 1, "takiy-mixer", 'S', 5.6, 145_408, 6, 1000, 6 * H_, "takiy-mixer --rate 48000"),
        proc(287, 104, "khipu", 'S', 4.2, 317_440, 11, 1000, 4 * H_ + 33 * 60, "khipu-app --store /home/sergio/.local/share/khipu"),
        proc(145, 1, "chasqui-relay", 'S', 3.1, 65_536, 4, 1000, 6 * H_, "chasqui-relay --peer agora.local --akasha"),
        proc(87, 1, "sandokan-monitor", 'R', 2.6, 74_752, 8, 1000, 38 * 60 + 5, "sandokan-monitor"),
        proc(133, 96, "fastembed-worker", 'S', 2.1, 253_952, 4, 1000, 6 * H_, "fastembed-worker multilingual-e5-small"),
        proc(171, 1, "dominium-server", 'D', 1.8, 225_280, 10, 1000, 6 * H_, "dominium-server --erp /var/lib/dominium"),
        proc(333, 104, "cosmos", 'S', 1.2, 277_504, 9, 1000, 3 * H_ + 12 * 60, "cosmos-app-llimphi --efemerides 2026"),
        proc(423, 415, "zsh", 'S', 0.9, 18_432, 1, 1000, 1 * H_ + 52 * 60, "-zsh"),
        proc(415, 104, "shuma", 'S', 0.8, 86_016, 7, 1000, 1 * H_ + 53 * 60, "shuma --sesion sergio"),
        proc(92, 1, "arje-card", 'S', 0.6, 59_392, 5, 1000, 6 * H_, "arje-card-llimphi"),
        proc(360, 104, "nada", 'S', 0.4, 98_304, 6, 1000, 47 * 60, "nada /home/sergio/tawasuyu"),
        proc(1, 0, "arje-zero", 'S', 0.3, 12_288, 1, 0, 8 * D + 3 * H_, "/sbin/arje-zero --genesis /var/lib/arje/semilla.card"),
        proc(188, 1, "agora", 'S', 0.2, 38_912, 3, 1000, 5 * H_, "agora-app --keystore /home/sergio/.config/agora"),
        proc(77, 1, "sshd", 'S', 0.0, 9_216, 1, 0, 8 * D + 3 * H_, "sshd: /usr/sbin/sshd -D [listener]"),
        proc(367, 423, "dd", 'T', 0.0, 4_096, 1, 1000, 26 * 60, "dd if=/dev/zero of=/dev/null bs=1M"),
        proc(602, 241, "pluma-export", 'Z', 0.0, 0, 1, 1000, 3 * 60, "[pluma-export] <defunct>"),
        proc(2, 0, "kthreadd", 'S', 0.0, 0, 1, 0, 8 * D + 3 * H_, "[kthreadd]"),
        proc(23, 2, "kworker/0:1", 'I', 0.0, 0, 1, 0, 2 * H_, "[kworker/0:1-events]"),
        proc(24, 2, "kworker/u8:2", 'I', 0.0, 0, 1, 0, 1 * H_, "[kworker/u8:2-flush]"),
        proc(55, 2, "rcu_sched", 'I', 0.0, 0, 1, 0, 8 * D + 3 * H_, "[rcu_sched]"),
    ]
}

/// Onda determinista para los historiales (sin reloj ni RNG): seno + un
/// "ruido" triangular barato, recortada a `0..=100`.
fn onda(n: usize, base: f32, amp: f32, freq: f32, fase: f32, ruido: f32) -> VecDeque<f32> {
    (0..n)
        .map(|i| {
            let s = (i as f32 * freq + fase).sin();
            let tri = ((i * 7 + 3) % 11) as f32 / 11.0 - 0.5;
            (base + amp * s + ruido * tri).clamp(0.0, 100.0)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Siembra: unidades del plano de control (modo Unidades).
// ---------------------------------------------------------------------------

/// Una unidad observada, como saldría de `observe()` sobre el Engine.
fn unidad(
    seq: u128,
    label: &str,
    state: LifecycleState,
    telemetry: Option<(f64, u64, u32)>, // (cpu_pct, mem_bytes, nproc)
    restarts: u32,
) -> UnitObservation {
    // Ulid fijo → pantallazo estable (y clave estable para el historial).
    let card_id = Ulid::from_parts(0x0190_0000_0000, seq);
    let telemetry = telemetry.map(|(cpu_pct, mem_bytes, nproc)| TelemetryFrame {
        card_id,
        at: UNIX_EPOCH + Duration::from_secs(1_770_000_000),
        mem_bytes,
        nproc,
        cpu_pct,
        restarts,
    });
    UnitObservation {
        card_id,
        label: label.into(),
        state,
        telemetry,
        restarts,
    }
}

const MIB: u64 = 1024 * 1024;

/// El snapshot demo: ocho unidades con estados variados — tres vivas sanas,
/// una viva reincidente (`↻2`), una fallada (`↻3`), una matada, una que salió
/// limpia y una pendiente.
fn snapshot_demo() -> MonitorSnapshot {
    MonitorSnapshot {
        units: vec![
            unidad(1, "rimay-verbo-daemon", LifecycleState::Running, Some((14.6, 432 * MIB, 9)), 0),
            unidad(2, "khipu-store", LifecycleState::Running, Some((38.2, 1_260 * MIB, 16)), 0),
            unidad(3, "chasqui-relay", LifecycleState::Running, Some((3.1, 64 * MIB, 4)), 2),
            unidad(4, "dominium-server", LifecycleState::Running, Some((1.8, 220 * MIB, 10)), 0),
            unidad(
                5,
                "nahual-sim",
                LifecycleState::Failed {
                    reason: "superó la cuota de memoria (techo 2 GiB)".into(),
                },
                None,
                3,
            ),
            unidad(6, "takiy-mixer", LifecycleState::Killed, None, 0),
            unidad(7, "pluma-align-worker", LifecycleState::Exited { code: 0 }, None, 0),
            unidad(8, "shuma-sandbox", LifecycleState::Pending, None, 0),
        ],
    }
}

/// Historiales de CPU de las unidades vivas → sparklines con personalidad:
/// el daemon de embeddings ondula, el store tiene ráfagas, el relay es plano.
fn history_demo(snapshot: &MonitorSnapshot) -> HashMap<Ulid, VecDeque<f32>> {
    let mut h = HashMap::new();
    let perfil: [(usize, VecDeque<f32>); 4] = [
        (0, onda(48, 14.0, 7.0, 0.31, 0.0, 4.0)),
        (1, onda(48, 36.0, 26.0, 0.55, 1.3, 9.0)),
        (2, onda(48, 3.0, 1.6, 0.22, 0.7, 1.2)),
        (3, onda(48, 6.0, 4.5, 0.40, 2.1, 2.5)),
    ];
    for (i, buf) in perfil {
        h.insert(snapshot.units[i].card_id, buf);
    }
    h
}

// ---------------------------------------------------------------------------
// Siembra: censo Wawa + Model completo.
// ---------------------------------------------------------------------------

/// Censo demo de apps WASM (los nombres reales de `wawa-kernel/assets`),
/// sembrado estáticamente para no depender del filesystem.
fn wawa_demo() -> Vec<WawaApp> {
    let app = |name: &str, kb: u64| WawaApp {
        name: name.into(),
        bytes: kb * 1024,
    };
    vec![
        app("pluma", 1_842),
        app("ide", 1_204),
        app("rimay", 866),
        app("tinkuy", 512),
        app("memoriosa", 348),
        app("bitacora", 296),
        app("cronista", 233),
        app("tonada", 187),
        app("pulso", 64),
    ]
}

/// El `Model` demo completo: la misma foto que tendría la app tras unos
/// minutos observando un host con build en marcha y unidades supervisadas.
fn modelo_demo() -> Model {
    let snapshot = snapshot_demo();
    let history = history_demo(&snapshot);
    let system = sistema_demo();

    // Seis cores con caracteres distintos (uno saturado por el build, otros
    // medios y bajos) + memoria subiendo suave hacia ~62 %.
    let core_hist: Vec<VecDeque<f32>> = vec![
        onda(120, 78.0, 18.0, 0.45, 0.0, 6.0),
        onda(120, 52.0, 22.0, 0.30, 1.1, 8.0),
        onda(120, 34.0, 14.0, 0.52, 2.3, 7.0),
        onda(120, 18.0, 9.0, 0.27, 0.4, 5.0),
        onda(120, 61.0, 25.0, 0.38, 3.0, 9.0),
        onda(120, 12.0, 6.0, 0.20, 1.8, 4.0),
    ];
    let mem_hist: VecDeque<f32> = (0..120)
        .map(|i| 48.0 + 14.0 * (i as f32 / 119.0) + 1.6 * (i as f32 * 0.5).sin())
        .collect();

    Model {
        theme: Theme::dark(),
        tab: Tab::System,
        snapshot,
        history,
        // khipu-store seleccionada → la tarjeta revela detener/matar (Engine).
        selected: Some(Ulid::from_parts(0x0190_0000_0000, 2)),
        error: None,
        wawa: wawa_demo(),
        system,
        // pluma seleccionado → fila resaltada + barra de acciones con señales.
        sys_sel: Some(241),
        sys_sort: Sort::Cpu,
        sys_scroll: 0,
        sys_tree: true,
        collapsed: HashSet::new(),
        sys_filter: String::new(),
        filter_mode: false,
        map_cpu: false,
        map_root: None,
        last_map_click: None,
        mem_total_kb: MEM_TOTAL_KB,
        mem_avail_kb: 12_750_000,
        core_hist,
        core_ids: (0..6).collect(),
        mem_hist,
        prev_core: Vec::new(),
        prev_proc: HashMap::new(),
        prev_total: 0,
        menu: app::build_menu(),
        menu_open: None,
        // El ctx real (runtime + Engine por `auto_default`): la view no lo
        // toca, pero el Model lo exige. Sin init ni daemon cae al LocalEngine.
        ctx: Arc::new(app::build_ctx()),
    }
}

// ---------------------------------------------------------------------------
// Composición del pantallazo: las cuatro views reales en un solo lienzo.
// ---------------------------------------------------------------------------

/// Rótulo fino sobre cada cuadrante (en la app viva se cambia de pestaña; acá
/// se muestran las cuatro a la vez y el rótulo orienta).
fn rotulo(t: &Theme, txt: &str) -> View<Msg> {
    View::new(Style {
        padding: taffy::prelude::Rect {
            left: length(16.0),
            right: length(16.0),
            top: length(5.0),
            bottom: length(5.0),
        },
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .text(txt, 10.5, t.fg_muted)
}

/// Envuelve una view de modo en una columna `ancho×100%` con su rótulo.
fn cuadrante(t: &Theme, ancho: f32, titulo: &str, v: View<Msg>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(ancho),
            height: percent(1.0),
        },
        ..Default::default()
    })
    .children(vec![rotulo(t, titulo), v])
}

/// Mismo chrome que el `view()` real (menubar + header + pestañas) y abajo
/// los cuatro modos en una grilla 2×2 asimétrica: Sistema y Unidades arriba
/// (la tabla necesita ancho), Mapa y Wawa abajo.
fn view_demo(model: &Model) -> View<Msg> {
    let t = &model.theme;

    let arriba = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        min_size: Size {
            width: auto(),
            height: length(0.0),
        },
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![
        cuadrante(t, 0.60, "SISTEMA · árbol de procesos del host (/proc)", app::system_body(model)),
        cuadrante(t, 0.40, "UNIDADES · observadas por el contrato Engine (SDD §6)", app::units_body(model)),
    ]);

    let abajo = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_shrink: 0.0,
        size: Size {
            width: percent(1.0),
            height: length(320.0),
        },
        ..Default::default()
    })
    .children(vec![
        cuadrante(t, 0.60, "MAPA · treemap padre/hijo, área por memoria", app::map_body(model)),
        cuadrante(t, 0.40, "WAWA · censo de apps WASM instaladas", app::wawa_body(model)),
    ]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0),
            height: percent(1.0),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .children(vec![
        menubar_view(&app::menu_spec(model)),
        app::header(model),
        app::tabs(model),
        arriba,
        abajo,
    ])
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/sandokan.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let model = modelo_demo();
    let root = view_demo(&model);
    let theme = model.theme.clone();

    // view → layout → scene (misma secuencia que el eventloop real).
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
        label: Some("pantallazo-sandokan"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
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
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_sandokan: escrito {out} ({W}x{H})");
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
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
        wgpu::Extent3d {
            width: W,
            height: H,
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
