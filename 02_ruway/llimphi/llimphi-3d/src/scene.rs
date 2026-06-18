//! `Scene3d` — orquestador de una escena 3D **general**: compone, en un único
//! pase con **depth buffer compartido**, el render volumétrico de voxels
//! ([`VoxelRenderer`](crate::VoxelRenderer)) y mallas de triángulos
//! ([`Renderer3d`](crate::Renderer3d)). Es el keystone que vuelve a `llimphi-3d`
//! un motor 3D general y no "sólo voxels": voxels y mallas se **ocluyen
//! correctamente entre sí** porque ambos escriben/testean el mismo depth.
//!
//! La firma de [`Scene3d::render`] es compatible con la closure de
//! `View::gpu_paint_with` (más los renderers a componer): el `Scene3d` posee el
//! depth y abre el pase; cada renderer aporta su `upload` (uniforms) + `draw`
//! (en el pase ya abierto).

use crate::camera::Camera3d;
use crate::renderer::Renderer3d;
use crate::voxel_renderer::VoxelRenderer;

/// Formato del depth buffer de toda la escena 3D (debe coincidir entre el
/// pipeline de voxels, el de mallas y la textura de depth).
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Depth attachment cacheado, recreado cuando cambia el tamaño del viewport.
pub(crate) struct DepthBuffer {
    pub view: wgpu::TextureView,
    w: u32,
    h: u32,
}

/// Asegura que `slot` tenga un depth buffer de `w×h` (lo recrea si cambió).
pub(crate) fn ensure_depth(
    slot: &mut Option<DepthBuffer>,
    device: &wgpu::Device,
    w: u32,
    h: u32,
) {
    if matches!(slot, Some(d) if d.w == w && d.h == h) {
        return;
    }
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("llimphi-3d-depth"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    *slot = Some(DepthBuffer { view, w, h });
}

/// Escena 3D que comparte un depth buffer entre el pase de voxels y el de
/// mallas. Sólo posee el depth; los renderers los aporta el llamador por
/// referencia en cada frame (así la app conserva la propiedad y los muta).
#[derive(Default)]
pub struct Scene3d {
    depth: Option<DepthBuffer>,
}

impl Scene3d {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compone la escena sobre `target` (textura intermedia del frame). Primero
    /// ray-marchea los voxels (escriben color + profundidad), luego dibuja las
    /// mallas (testean contra esa profundidad) — todo en un pase con el depth
    /// compartido, limpiado a lejano (`1.0`) al abrirlo. El color se preserva
    /// (`LoadOp::Load`) para no pisar la UI vello de abajo.
    ///
    /// Firma compatible con `View::gpu_paint_with` más los renderers a componer.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        (w, h): (u32, u32),
        camera: &Camera3d,
        voxel: Option<&VoxelRenderer>,
        meshes: &[&Renderer3d],
    ) {
        // El caso por defecto: la escena ocupa todo el target.
        self.render_in(
            device,
            queue,
            encoder,
            target,
            (w, h),
            (0.0, 0.0, w as f32, h as f32),
            camera,
            voxel,
            meshes,
        );
    }

    /// Como [`render`](Self::render) pero **confina** la escena a la sub-región
    /// `rect = (x, y, w, h)` (en px del target, esquina sup-izq), vía
    /// `set_viewport` + `set_scissor_rect`. Es lo que permite montar el 3D en un
    /// **panel** de una UI (un canvas que no ocupa toda la ventana) sin pisar el
    /// chrome alrededor: la pasada de ray-march/mallas pinta sólo dentro del rect,
    /// con el aspect del rect (no el de la ventana). `target`/`viewport` siguen
    /// siendo el frame completo (load-preserve del chrome ya rasterizado).
    #[allow(clippy::too_many_arguments)]
    pub fn render_in(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        (w, h): (u32, u32),
        rect: (f32, f32, f32, f32),
        camera: &Camera3d,
        voxel: Option<&VoxelRenderer>,
        meshes: &[&Renderer3d],
    ) {
        if w == 0 || h == 0 {
            return;
        }
        let (rx, ry, rw, rh) = rect;
        if rw < 1.0 || rh < 1.0 {
            return;
        }
        // El aspect es el del rect (el viewport mapea NDC a esa sub-región).
        let aspect = rw / rh;

        // Subir uniforms antes de abrir el pase (queue.write_buffer se ordena
        // antes del submit).
        if let Some(v) = voxel {
            v.upload(queue, aspect, camera);
        }
        for m in meshes {
            m.upload(queue, aspect, camera);
        }

        ensure_depth(&mut self.depth, device, w, h);
        let depth_view = &self.depth.as_ref().unwrap().view;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-3d-scene-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        // Viewport (mapeo NDC→rect) + scissor (recorte físico al rect, clampeado a
        // los límites del attachment).
        pass.set_viewport(rx, ry, rw, rh, 0.0, 1.0);
        let sx = rx.max(0.0);
        let sy = ry.max(0.0);
        let sw = (rw.min(w as f32 - sx)).max(0.0) as u32;
        let sh = (rh.min(h as f32 - sy)).max(0.0) as u32;
        if sw == 0 || sh == 0 {
            return;
        }
        pass.set_scissor_rect(sx as u32, sy as u32, sw, sh);

        if let Some(v) = voxel {
            v.draw(&mut pass);
        }
        for m in meshes {
            m.draw(&mut pass);
        }
    }
}
