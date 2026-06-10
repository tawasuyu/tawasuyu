//! Pantallazo headless de `agora-app` — identidad digital soberana.
//!
//! Monta la **view real** de la app (los siete tiles sobre el mismo
//! `TrustGraph`: identidades · compositor · atestaciones · política ·
//! multifirma · release · capacidad) con un `Model` sembrado con datos demo
//! creíbles: tres identidades con claves Ed25519 reales, atestaciones
//! firmadas y verificadas, una multifirma 2-de-2 y los sobres del plano de
//! control de wawa (`ManifiestoFirmado` + `ConcesionCapacidad`) firmados de
//! verdad con `agora-channel` — el mismo código que re-verifica el kernel.
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `llimphi-compositor/examples/primitivas_demo.rs`).
//!
//! `cargo run -p agora-app --example pantallazo_agora --release -- [out.png]`
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos sus módulos reales por
// `#[path]` para llamar exactamente los mismos `*_view` que pinta la app.
#[path = "../src/model.rs"]
mod model;
#[path = "../src/tiles/mod.rs"]
mod tiles;
#[path = "../src/ui.rs"]
mod ui;

use std::fs::File;
use std::io::BufWriter;

use agora_core::{Attestation, Claim, IdentityKind, Keypair, MultiSignature};
use agora_graph::{TrustGraph, TrustPolicy};
use agora_keystore::Keystore;
use format::{PERMISO_ALTAVOZ, PERMISO_CONFIG, PERMISO_RED};
use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, View};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_text_input::TextInputState;
use llimphi_widget_tiled::{tiled_view_cols, tiled_view_reorderable, TileSpec, TiledPalette};

use crate::model::{
    ComposeField, FocusedInput, Model, Msg, Screen, StatusBanner, StatusLevel, Tile,
};
use crate::ui::bytes_to_hex;

const W: u32 = 1280;
const H: u32 = 800;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Segundos UNIX actuales.
fn ahora() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Construye el `Model` demo: el mismo estado que tendría la app tras una
/// sesión corta de uso (identidades creadas, atestaciones firmadas, sobres
/// de wawa emitidos). Todas las firmas son Ed25519 reales.
fn modelo_demo() -> Model {
    let now = ahora();

    // --- Tres identidades con claves reales (seeds fijas → pantallazo estable).
    let kp_amaru = Keypair::from_seed(*b"amaru-semilla-demo-pantallazo-01");
    let kp_killa = Keypair::from_seed(*b"killa-semilla-demo-pantallazo-02");
    let kp_minga = Keypair::from_seed(*b"minga-semilla-demo-pantallazo-03");
    let id_amaru = kp_amaru.identity_id();
    let id_killa = kp_killa.identity_id();

    let mut graph = TrustGraph::new();
    graph.register(kp_amaru.identity(IdentityKind::Person, "amaru"));
    graph.register(kp_killa.identity(IdentityKind::Person, "killa"));
    graph.register(kp_minga.identity(IdentityKind::Community, "minga del valle"));

    // --- Atestaciones firmadas y verificadas por el grafo. Dos terceros
    //     corroboran el mismo claim sobre amaru → la política lo ACEPTA.
    let atestar = |kp: &Keypair, sujeto, pred: &str, val: &str, t| {
        Attestation::create(kp, Claim::new(sujeto, pred, val, t))
    };
    for att in [
        atestar(&kp_killa, id_amaru, "miembro", "minga del valle", now - 3_600),
        atestar(&kp_minga, id_amaru, "miembro", "minga del valle", now - 7_200),
        atestar(&kp_minga, id_killa, "miembro", "minga del valle", now - 86_000),
        atestar(&kp_amaru, id_amaru, "rol", "guardián de releases", now - 1_800),
    ] {
        graph
            .add_attestation(att)
            .expect("atestación demo válida");
    }

    // --- Seeds "mías" en RAM (amaru y killa); la comunidad es ajena.
    let mut seeds = std::collections::HashMap::new();
    seeds.insert(id_amaru, *b"amaru-semilla-demo-pantallazo-01");
    seeds.insert(id_killa, *b"killa-semilla-demo-pantallazo-02");

    // Keystore efímero: la view no lo lee, pero el Model lo exige.
    let ks_dir = std::env::temp_dir().join("agora-pantallazo-keys");
    std::fs::create_dir_all(&ks_dir).ok();
    let keystore = Keystore::open(&ks_dir).expect("keystore efímero");

    // --- Compositor a medio escribir (como en una sesión real).
    let mut compose_predicate = TextInputState::new();
    compose_predicate.set_text("habilidad");
    let mut compose_value = TextInputState::new();
    compose_value.set_text("soldadura de cobre");

    // --- Multifirma 2-de-2 sobre una raíz canónica.
    let raiz_hex = bytes_to_hex(b"raiz-canonica-del-grafo-demo-001");
    let mut multi_message = TextInputState::new();
    multi_message.set_text(&raiz_hex);
    let multi_current = Some(MultiSignature::create(
        &[&kp_amaru, &kp_killa],
        raiz_hex.as_bytes(),
    ));
    let mut multi_selected = std::collections::BTreeSet::new();
    multi_selected.insert(id_amaru);
    multi_selected.insert(id_killa);

    // --- Release de wawa: hash de imagen firmado con la clave activa.
    let hash_release: [u8; 32] = *b"hash-imagen-wawa-0.9-demo-000001";
    let mut release_hash = TextInputState::new();
    release_hash.set_text(bytes_to_hex(&hash_release));
    let release_current = Some(agora_channel::firmar_manifiesto(&kp_amaru, &hash_release));

    // --- Concesión de capacidad §14.1.3: (hash_bytecode, permisos) firmados.
    let hash_bytecode: [u8; 32] = *b"hash-bytecode-pluma-wasm-demo-01";
    let mut cap_bytecode = TextInputState::new();
    cap_bytecode.set_text(bytes_to_hex(&hash_bytecode));
    let cap_permisos = PERMISO_RED | PERMISO_ALTAVOZ | PERMISO_CONFIG;
    let cap_current = Some(agora_channel::firmar_capacidad(
        &kp_amaru,
        &hash_bytecode,
        cap_permisos,
    ));

    // --- Banner: el postcard exportado del release (recortado para que quepa).
    let postcard_hex = release_current
        .as_ref()
        .and_then(|mf| mf.serializar().ok())
        .map(|b| (b.len(), bytes_to_hex(&b)))
        .map(|(n, hex)| format!("ManifiestoFirmado postcard ({n} bytes): {}…", &hex[..96]))
        .unwrap_or_default();

    Model {
        graph,
        keystore,
        seeds,
        passphrase: "agora-dev".into(),
        store_path: std::env::temp_dir().join("agora-pantallazo-graph.json"),
        screen: Screen::Main,
        // Seis tiles (sin el compositor) → grilla 3×2 llena, sin celdas
        // vacías: sustrato social arriba, plano de control de wawa abajo.
        tiles_order: vec![
            Tile::Identidades,
            Tile::Atestaciones,
            Tile::Politica,
            Tile::Multifirma,
            Tile::Release,
            Tile::Capacidad,
        ],
        focused_subject: Some(id_amaru),
        active_signer: Some(id_amaru),
        selected_attestation: Some(0),
        compose_predicate,
        compose_value,
        focused_input: FocusedInput::Compose(ComposeField::Value),
        compose_status: "atestación agregada y persistida".into(),
        policy: TrustPolicy {
            min_third_party: 2,
            accept_self: false,
            min_attesters_of_kind: Some((IdentityKind::Community, 1)),
            max_age_secs: Some(604_800),
        },
        multi_message,
        multi_selected,
        multi_threshold: 2,
        multi_current,
        release_hash,
        release_paste: TextInputState::new(),
        release_current,
        release_status: "✓ release firmado (exportá el postcard para mudanza)".into(),
        cap_bytecode,
        cap_permisos,
        cap_paste: TextInputState::new(),
        cap_current,
        cap_status: "✓ concesión firmada (viaja con el bytecode)".into(),
        status: Some(StatusBanner {
            level: StatusLevel::Info,
            text: postcard_hex,
        }),
        menu_open: None,
        edit_menu: None,
        clipboard: llimphi_clipboard::SystemClipboard::new(),
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        edit_active: usize::MAX,
        edit_anim: Tween::idle(1.0),
    }
}

/// Barra de menú con los mismos menús raíz que la app (cerrados en el
/// pantallazo, así que sólo se ven los rótulos).
fn menu_demo() -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};
    AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Nueva identidad", "file.nueva_identidad")))
        .menu(Menu::new("Editar").item(MenuItem::new("Atestar", "edit.atestar")))
        .menu(Menu::new("Ver").item(MenuItem::new("Limpiar release", "view.limpiar_release")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Recargar grafo", "help.recargar")))
}

/// Misma composición que el `view()` de `AgoraApp` (menubar arriba, tiles
/// draggables al centro, banner de estado al pie), con la grilla acomodada
/// para el pantallazo: el sustrato social en 2×2 a la izquierda y el tile
/// **capacidad · wawa** a columna completa a la derecha — los sobres firmados
/// muestran hashes de 64 hex que necesitan ese ancho/alto para no envolver.
fn view_demo(model: &Model, menu: &app_bus::AppMenu, theme: &Theme) -> View<Msg> {
    let palette = TiledPalette::from_theme(theme);

    let spec = |t: &Tile| {
        let (label, content) = match t {
            Tile::Identidades => ("identidades", tiles::identidades_view(model, theme)),
            Tile::Compositor => ("compositor", tiles::compositor_view(model, theme)),
            Tile::Atestaciones => ("atestaciones", tiles::atestaciones_view(model, theme)),
            Tile::Politica => ("política", tiles::politica_view(model, theme)),
            Tile::Multifirma => ("multifirma", tiles::multifirma_view(model, theme)),
            Tile::Release => ("release · wawa", tiles::release_view(model, theme)),
            Tile::Capacidad => ("capacidad · wawa", tiles::capacidad_view(model, theme)),
        };
        TileSpec {
            label: label.into(),
            content,
        }
    };

    // Izquierda: web-of-trust en grilla 2×2 (los mismos tiles reales).
    let sociales: Vec<TileSpec<Msg>> = [
        Tile::Identidades,
        Tile::Atestaciones,
        Tile::Politica,
        Tile::Multifirma,
    ]
    .iter()
    .map(spec)
    .collect();
    let left = tiled_view_reorderable(sociales, |from, to| Some(Msg::SwapTile(from, to)), &palette);

    // Derecha: el plano de control de wawa a columna completa.
    let right = tiled_view_cols(vec![spec(&Tile::Capacidad)], 1, &palette);

    let envolver = |v: View<Msg>, w: f32| {
        View::new(Style {
            size: Size {
                width: percent(w),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![v])
    };
    let grids = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![envolver(left, 0.56), envolver(right, 0.44)]);

    let body = match &model.status {
        None => grids,
        Some(banner) => ui::status_layout(theme, grids, banner),
    };

    let menubar = menubar_view(&MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    });

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, ui::grow(body)])
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/agora.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let theme = Theme::dark();
    let model = modelo_demo();
    let menu = menu_demo();
    let root = view_demo(&model, &menu, &theme);

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
        label: Some("pantallazo-agora"),
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
    eprintln!("pantallazo_agora: escrito {out} ({W}x{H})");
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
    hal.device.poll(wgpu::PollType::wait_indefinitely());
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
