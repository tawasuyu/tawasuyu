//! `pantallazo_synth` — volcado headless del renderer 3D de supay sobre una
//! **escena sintética construida a mano** (sin `doom1.wad` ni el motor C de
//! doomgeneric). Cierra el hueco de "BSP sólo testeable con snapshots
//! sintéticos, no visual": ahora una sala armada en Rust se **ve** como PNG
//! en cualquier máquina con llvmpipe.
//!
//! La escena: una sala 512×512 de cuatro muros sólidos, un único sector
//! convexo (un subsector con sus cuatro segs ⇒ piso y techo reales por
//! subsector, no el fake-floor de 3.1) con luz pulsante, dos antorchas como
//! sprites y el jugador mirando al norte. Sin atlas: colores planos por
//! material/paleta. (Un escalón real necesita una linedef two-sided que
//! cargue el contramuro del desnivel; sin ella el riser no se dibuja y queda
//! una costura — por eso la sala es de un solo nivel.)
//!
//! No toca `supay-core` → corre con o sin vendor de doomgeneric. Es un banco
//! de diagnóstico de proyección/cámara/planos/sprites/HUD; el render con
//! texturas reales sigue siendo `dump_frame` (requiere WAD).
//!
//! Uso:
//! ```sh
//! cargo run -p supay-doom-llimphi --example pantallazo_synth --release -- [tick] [out.png]
//! ```
//! Defaults: tick 200 y `/tmp/supay/synth.png`.

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use supay_render_llimphi::{render_snapshot, RenderConfig};
use supay_scene::{
    NodeSnap, PlayerOverlays, PlayerSnap, PlayerStats, SceneSnapshot, SectorSnap, SegSnap,
    SpriteSnap, SubsectorSnap, WallSeg, WeaponSpriteSnap, NO_SECTOR, NO_SKY_PIC,
};

const W: u32 = 960;
const H: u32 = 600;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const ROOM: f32 = 512.0;
const MID: f32 = ROOM * 0.5;

/// Una pared sólida one-sided del recinto (sin texturas: colores planos).
fn wall(x1: f32, y1: f32, x2: f32, y2: f32, front: u32) -> WallSeg {
    WallSeg {
        x1,
        y1,
        x2,
        y2,
        front_sector: front,
        back_sector: NO_SECTOR,
        flags: 0,
        textures: [[0; 8]; 6],
        tex_x_offsets: [0.0; 2],
        tex_y_offsets: [0.0; 2],
    }
}

fn seg(x1: f32, y1: f32, x2: f32, y2: f32, linedef: u32) -> SegSnap {
    SegSnap { x1, y1, x2, y2, solid: true, linedef }
}

/// Construye el snapshot sintético para `tick`. Determinista: el flicker de
/// luz y el bob de las antorchas dependen sólo de `tick`.
fn synth(tick: u64) -> SceneSnapshot {
    let t = tick as f32 * 0.03;

    // Jugador cerca del muro sur, mirando al norte (+Y), un poco a la
    // izquierda para componer la escena con el escalón a la vista.
    let player = PlayerSnap {
        x: MID - 60.0,
        y: 80.0,
        z: 0.0,
        angle: std::f32::consts::FRAC_PI_2, // +Y
        view_height: 41.0,
        view_pitch: 0.0,
    };

    // Muros del recinto (CW visto desde +Z ⇒ front hacia adentro).
    let walls = vec![
        wall(0.0, 0.0, 0.0, ROOM, 0),   // 0 oeste
        wall(0.0, ROOM, ROOM, ROOM, 0), // 1 norte
        wall(ROOM, ROOM, ROOM, 0.0, 0), // 2 este
        wall(ROOM, 0.0, 0.0, 0.0, 0),   // 3 sur
    ];

    // Un único sector con luz pulsante (flicker determinista por tick).
    let light = (190.0 + (t * 0.7).sin() * 40.0).clamp(0.0, 255.0) as u8;
    let sectors = vec![SectorSnap {
        floor_height: 0.0,
        ceiling_height: 224.0,
        light_level: light,
        floor_pic: 0,
        ceiling_pic: 0,
    }];

    // Un subsector convexo (la sala cuadrada) bordeado por sus cuatro segs
    // ⇒ el renderer pinta piso y techo reales por subsector. Sin nodos: el
    // ordering de painter cae al euclidiano (correcto con un solo plano).
    let segs = vec![
        seg(ROOM, 0.0, 0.0, 0.0, 3),    // sur
        seg(0.0, 0.0, 0.0, ROOM, 0),    // oeste
        seg(0.0, ROOM, ROOM, ROOM, 1),  // norte
        seg(ROOM, ROOM, ROOM, 0.0, 2),  // este
    ];
    let subsectors = vec![SubsectorSnap { sector: 0, first_seg: 0, num_segs: 4 }];
    let nodes: Vec<NodeSnap> = Vec::new();

    // Dos antorchas (sprite arbitrario) que bobean suave con el tick. Bit 7
    // del frame = FF_FULLBRIGHT ⇒ cuentan como luces del mundo (gather_world_lights)
    // y por tanto irradian god rays (Fase 3.57). z elevado para que el halo
    // quede a media pared, no al ras del piso.
    let bob = (t * 2.0).sin() * 4.0;
    let fb = |f: u64| ((f / 4 % 4) as u8) | 0x80;
    let sprites = vec![
        SpriteSnap {
            x: 120.0,
            y: 300.0,
            z: 80.0 + bob,
            angle: 0.0,
            sprite: 5,
            frame: fb(tick),
            sector: 0,
        },
        SpriteSnap {
            x: ROOM - 120.0,
            y: 300.0,
            z: 80.0 - bob,
            angle: 0.0,
            sprite: 7,
            frame: fb(tick),
            sector: 0,
        },
    ];

    SceneSnapshot {
        tick,
        player,
        walls: Arc::from(walls),
        sectors: Arc::from(sectors),
        sprites: Arc::from(sprites),
        subsectors: Arc::from(subsectors),
        segs: Arc::from(segs),
        nodes: Arc::from(nodes),
        sky_pic: NO_SKY_PIC,
        player_overlays: PlayerOverlays::default(),
        weapon: WeaponSpriteSnap::default(),
        weapon_flash: WeaponSpriteSnap::default(),
        player_stats: PlayerStats {
            health: 78,
            armor_points: 50,
            armor_type: 1,
            ready_weapon: 1,
            ammo: [42, 8, 0, 0],
            max_ammo: [200, 50, 300, 50],
            cards: [false; 6],
        },
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let tick: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);
    let out = args
        .next()
        .unwrap_or_else(|| "/tmp/supay/synth.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let snap = synth(tick);
    eprintln!(
        "pantallazo_synth: tick {tick} — jugador ({:.0},{:.0}) ang {:.2}; \
         {} paredes, {} sectores, {} subsectores, {} sprites, {} nodos BSP",
        snap.player.x,
        snap.player.y,
        snap.player.angle,
        snap.walls.len(),
        snap.sectors.len(),
        snap.subsectors.len(),
        snap.sprites.len(),
        snap.nodes.len(),
    );

    // Modernizaciones visuales encendidas; sin atlas (colores planos).
    let cfg = RenderConfig {
        atlas: None,
        crosshair: true,
        hud: true,
        sprite_shadows: true,
        world_lights_enabled: true,
        wall_vertical_bands: 4,
        wall_vertical_gradient: true,
        plane_depth_gradient: true,
        ..RenderConfig::default()
    };

    let mut scene = vello::Scene::new();
    let mut ts = Typesetter::new();
    render_snapshot(&mut scene, &mut ts, W as f32, H as f32, &snap, &cfg);

    let hal = pollster::block_on(Hal::new(None)).expect("hal (¿sin Vulkan/llvmpipe?)");
    let mut renderer = Renderer::new(&hal).expect("renderer vello");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-synth-target"),
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
    renderer
        .render_to_view(&hal, &scene, &view, W, H, Color::from_rgba8(8, 8, 12, 255))
        .expect("render_to_view");

    write_texture_png(&hal, &target, &out);
    eprintln!("pantallazo_synth: escrito {out} ({W}x{H})");
}

/// Copia la textura a un buffer mapeable, lee y escribe PNG. wgpu exige
/// `bytes_per_row` alineado a 256 B, así que desempaquetamos las filas.
fn write_texture_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf_size = (padded * H as usize) as u64;

    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pantallazo-synth-readback"),
        size: buf_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pantallazo-synth-copy"),
        });
    encoder.copy_texture_to_buffer(
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
    hal.queue.submit(std::iter::once(encoder.finish()));

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
        let start = row * padded;
        pixels.extend_from_slice(&data[start..start + unpadded]);
    }
    drop(data);
    buf.unmap();

    let file = File::create(path).expect("crear PNG");
    let mut encoder = png::Encoder::new(BufWriter::new(file), W, H);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut w = encoder.write_header().expect("png header");
    w.write_image_data(&pixels).expect("png data");
}
