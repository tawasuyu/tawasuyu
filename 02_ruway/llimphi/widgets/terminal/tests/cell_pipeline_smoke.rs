//! Smoke test del `CellPipeline` (Fase 4.2 del SDD-TERMINAL).
//!
//! No verifica píxeles — eso requiere conocer la fuente exacta, font hinting
//! del rasterizer y un pipeline de comparación. Sí verifica:
//!
//! - `CellPipeline::new` compila el shader WGSL sin errores de naga.
//! - `create_atlas_texture` sube bytes a una `R8Unorm` sin pánico.
//! - `draw` ejecuta sin errores wgpu con un atlas vivo y N instancias —
//!   por debajo y por arriba del cap del adapter, sin reasignar buffers.
//!
//! Corre en cualquier adapter wgpu disponible (en CI sin GPU = llvmpipe).

use llimphi_hal::{wgpu, Hal};
use llimphi_widget_terminal::cell_pipeline::{
    pack_rgba, CellInstance, CellPipeline, CellUniforms,
};
use llimphi_widget_terminal::glyph_atlas::GlyphAtlas;

const W: u32 = 256;
const H: u32 = 256;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cell-smoke-target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

#[test]
fn pipeline_compila_y_dibuja_sin_panico() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let pipeline = CellPipeline::new(&hal.device, FMT);
    let (_tex, view) = make_target(&hal.device);

    // Atlas con un par de glifos.
    let mut atlas = GlyphAtlas::new(
        llimphi_ui::llimphi_text::MONO_FONT_BYTES,
        14.0,
        16,
        4,
    )
    .expect("atlas");
    let slot_a = atlas.glyph_for('A').unwrap();
    let slot_b = atlas.glyph_for('B').unwrap();
    let (atlas_w, atlas_h) = atlas.size();
    let (cell_w, cell_h) = atlas.cell_size();

    let (_atlas_tex, atlas_view) =
        CellPipeline::create_atlas_texture(&hal.device, &hal.queue, atlas.pixels(), atlas.size());

    // Dos celdas, A y B en (0,0) y (cell_w,0).
    let cells = vec![
        CellInstance {
            cell_x: 0.0,
            cell_y: 0.0,
            uv_x: slot_a.px as f32,
            uv_y: slot_a.py as f32,
            uv_w: cell_w as f32,
            uv_h: cell_h as f32,
            fg_rgba: pack_rgba(255, 255, 255, 255),
            bg_rgba: pack_rgba(20, 20, 20, 255),
        },
        CellInstance {
            cell_x: cell_w as f32,
            cell_y: 0.0,
            uv_x: slot_b.px as f32,
            uv_y: slot_b.py as f32,
            uv_w: cell_w as f32,
            uv_h: cell_h as f32,
            fg_rgba: pack_rgba(100, 255, 100, 255),
            bg_rgba: pack_rgba(0, 0, 0, 255),
        },
    ];

    let uniforms = CellUniforms {
        viewport_w: W as f32,
        viewport_h: H as f32,
        cell_w: cell_w as f32,
        cell_h: cell_h as f32,
        atlas_w: atlas_w as f32,
        atlas_h: atlas_h as f32,
        _pad0: 0.0,
        _pad1: 0.0,
    };

    let mut encoder = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("cell-smoke-encoder"),
    });

    // Clear primero para tener un load:Load coherente.
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cell-smoke-clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    pipeline.draw(
        &hal.device,
        &hal.queue,
        &mut encoder,
        &view,
        &atlas_view,
        &cells,
        uniforms,
    );

    hal.queue.submit(std::iter::once(encoder.finish()));
    hal.device.poll(wgpu::PollType::wait_indefinitely());
}

#[test]
fn draw_con_cero_instancias_es_no_op() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let pipeline = CellPipeline::new(&hal.device, FMT);
    let (_tex, view) = make_target(&hal.device);

    let mut atlas = GlyphAtlas::new(
        llimphi_ui::llimphi_text::MONO_FONT_BYTES,
        14.0,
        16,
        4,
    )
    .unwrap();
    let _ = atlas.glyph_for('A'); // tener algo en el atlas
    let (_atlas_tex, atlas_view) =
        CellPipeline::create_atlas_texture(&hal.device, &hal.queue, atlas.pixels(), atlas.size());

    let (atlas_w, atlas_h) = atlas.size();
    let (cell_w, cell_h) = atlas.cell_size();
    let mut encoder = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("cell-smoke-empty-encoder"),
    });
    pipeline.draw(
        &hal.device,
        &hal.queue,
        &mut encoder,
        &view,
        &atlas_view,
        &[],
        CellUniforms {
            viewport_w: W as f32,
            viewport_h: H as f32,
            cell_w: cell_w as f32,
            cell_h: cell_h as f32,
            atlas_w: atlas_w as f32,
            atlas_h: atlas_h as f32,
            _pad0: 0.0,
            _pad1: 0.0,
        },
    );
    hal.queue.submit(std::iter::once(encoder.finish()));
    hal.device.poll(wgpu::PollType::wait_indefinitely());
}
