//! Verificación headless del **monitoreo runtime del bloque de matilda**:
//! el panel de inventario muestra cada contenedor con su semáforo (● vivo /
//! ○ parado) + el `status` de Docker, lista los huérfanos que corren fuera
//! del inventario, y el header cuenta up/down. Es la administración de
//! servidores/contenedores desde la interfaz de shuma, sin ir a la terminal.
//!
//! `cargo run -p shuma-module-matilda --example runtime_monitor -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use matilda_discover::{
    ContainerStatus, RunState, RuntimeState, ServiceState, ServiceStatus,
};
use shuma_module::Source;
use shuma_module_matilda::{update, Msg, State};

const W: u32 = 1040;
const H: u32 = 780;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn cs(name: &str, image: &str, state: RunState, status: &str, ports: &str) -> ContainerStatus {
    ContainerStatus {
        name: name.into(),
        image: image.into(),
        state,
        status: status.into(),
        ports: ports.into(),
    }
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "runtime_monitor.png".to_string());
    let theme = llimphi_theme::Theme::default();

    // Inventario de ejemplo (web + api deseados) + estado runtime observado:
    // web corriendo, api caído, y un `legacy` huérfano que corre fuera del
    // inventario. La UI lo refleja todo.
    // Inventario con varios hosts para mostrar la flota (M5).
    let mut inv = shuma_module_matilda::example_inventory();
    inv.add_host(matilda_core::Host::new("db-1", "10.0.0.2").with_tag("db"));
    inv.add_host(matilda_core::Host::new("edge-2", "10.0.0.3"));
    let mut state = State::with_inventory(Source::Local, inv);
    let rt = RuntimeState {
        containers: vec![
            cs("web", "nginx:1.27", RunState::Running, "Up 2 hours", "0.0.0.0:8080->80/tcp"),
            cs("api", "ghcr.io/ejemplo/api:1.0", RunState::Exited, "Exited (1) 5 min ago", ""),
            cs("legacy", "redis:6", RunState::Running, "Up 9 days", "6379/tcp"),
        ],
        services: vec![
            ServiceStatus {
                name: "sshd.service".into(),
                state: ServiceState::Active,
                sub: "running".into(),
                description: "OpenSSH server daemon".into(),
            },
            ServiceStatus {
                name: "nginx.service".into(),
                state: ServiceState::Active,
                sub: "running".into(),
                description: "A high performance web server".into(),
            },
            ServiceStatus {
                name: "backup.service".into(),
                state: ServiceState::Failed,
                sub: "failed".into(),
                description: "Nightly backup".into(),
            },
        ],
        vhosts: vec![],
    };
    state = update(state, Msg::SetRuntime(rt));

    // Flota (M5): edge-1 alcanzado (con su runtime), db-1 caído, edge-2 aún
    // consultando. edge-1 seleccionado → expande sus contenedores/servicios.
    state = update(state, Msg::RefreshFleet);
    let edge1 = RuntimeState {
        containers: vec![
            cs("web", "nginx:1.27", RunState::Running, "Up 6 days", "0.0.0.0:80->80/tcp"),
            cs("worker", "ghcr.io/ejemplo/worker:2", RunState::Exited, "Exited (137)", ""),
        ],
        services: vec![ServiceStatus {
            name: "sshd.service".into(),
            state: ServiceState::Active,
            sub: "running".into(),
            description: "OpenSSH server daemon".into(),
        }],
        vhosts: vec![],
    };
    state = update(state, Msg::SetHostRuntime { host: "edge-1".into(), runtime: edge1 });
    state = update(state, Msg::SetHostError {
        host: "db-1".into(),
        error: "ssh connect: connection timed out".into(),
    });
    state = update(state, Msg::SelectHost("edge-1".to_string()));

    // Seleccionamos `web` (barra de acciones de contenedor) y un servicio
    // fallado (barra de acciones de servicio) para que ambas se vean.
    state = update(state, Msg::SelectContainer("web".to_string()));
    state = update(state, Msg::SelectService("backup.service".to_string()));

    let v = shuma_module_matilda::view::<()>(&state, &theme, |_m| ());
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, v);
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
        label: Some("runtime-monitor"),
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
    renderer
        .render_to_view(&hal, &scene, &view, W, H, Color::from_rgba8(20, 20, 26, 255))
        .expect("render_to_view");
    write_png(&hal, &target, &out);
    eprintln!("runtime_monitor: {out} ({W}x{H})");
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
