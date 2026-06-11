//! llimphi-surface — superficies externas dentro del bucle Elm.
//!
//! Un `ExternalSurface` es una textura RGBA8 que vive en GPU y se pinta
//! sobre un rect del frame Llimphi cada vez que la app lo expone vía
//! `View::gpu_paint_with`. La fuente de bytes corre afuera del bucle
//! Elm: un decoder de video, un capture de cámara, un raster de PDF,
//! una textura raw producida por otro motor — cualquier productor que
//! genere RGBA puede empujar frames con [`ExternalSurface::upload`] y
//! ver el resultado en la próxima pasada de raster.
//!
//! El crate provee:
//!
//! - [`ExternalSurface`]: dueño de la textura + render pipeline + bind
//!   group. `upload(rgba, w, h)` sube bytes y recrea la textura si
//!   `w`/`h` cambiaron.
//! - [`ExternalSurface::view`]: helper que construye un [`View`] con
//!   `gpu_paint_with` ya conectado. La app sólo elige el `Style` del
//!   nodo (qué porción del layout ocupa).
//!
//! ## Diseño
//!
//! El pipeline es un textured-quad clásico: dos triángulos cubren el
//! rect destino, el fragment shader samplea la textura externa con
//! sampler bilineal. Las coordenadas NDC del quad se computan en GPU
//! a partir de `(rect, viewport)` que viajan por uniform — por eso
//! el callback necesita el `viewport` que `llimphi-ui` empezó a
//! propagar en `GpuPaintFn`.
//!
//! La textura intermedia donde Llimphi pinta vello es `Rgba8Unorm`
//! (ver `llimphi-hal::INTERMEDIATE_FORMAT`). El pipeline emite
//! `Rgba8Unorm` también — el target del render pass es esa misma
//! intermedia con `LoadOp::Load`, así el fondo vello queda preservado.

use std::sync::Arc;

use llimphi_hal::wgpu;
use llimphi_ui::{PaintRect, View};
use parking_lot::Mutex;

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SOURCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Inner {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    // Textura + bind group recreados cuando cambia (w, h) del frame de
    // entrada. Empieza en (1, 1) con un pixel transparente para que el
    // pipeline funcione antes del primer `upload`.
    tex: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    tex_size: (u32, u32),
}

/// Superficie externa: textura GPU + pipeline que la blittea al rect
/// que ocupe en el árbol Llimphi. Clonar es barato (Arc interno).
#[derive(Clone)]
pub struct ExternalSurface {
    inner: Arc<Mutex<Inner>>,
}

impl ExternalSurface {
    /// Construye la surface usando el `Device`/`Queue` del Hal de la app.
    /// La textura arranca en 1×1 transparente; el primer
    /// [`Self::upload`] la redimensiona al tamaño real del frame.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-surface-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-surface-pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-surface-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-surface-pipe"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-surface-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Uniforms: 12 floats — rect destino (x, y, w, h) + viewport
        // (vw, vh, _, _) + src_uv (u0, v0, du, dv): el sub-rectángulo de la
        // textura a muestrear, en UV 0..1. `blit` lo deja en (0,0,1,1)
        // (textura entera); `blit_layout` lo usa para crop/zoom/pan (V2).
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-surface-uniforms"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (tex, bind_group) =
            make_texture_and_bg(device, queue, &bgl, &uniforms, &sampler, 1, 1, &[0, 0, 0, 0]);

        Self {
            inner: Arc::new(Mutex::new(Inner {
                device: device.clone(),
                queue: queue.clone(),
                pipeline,
                bgl,
                sampler,
                uniforms,
                tex,
                bind_group,
                tex_size: (1, 1),
            })),
        }
    }

    /// Sube `rgba` (8 bits por canal, premultiplicado o no — el blend
    /// usa straight alpha) como nuevo contenido de la surface. Si
    /// `(width, height)` difiere del tamaño actual, recrea la textura
    /// y el bind group. `rgba.len()` debe ser exactamente
    /// `width * height * 4`.
    pub fn upload(&self, rgba: &[u8], width: u32, height: u32) {
        let mut inner = self.inner.lock();
        debug_assert_eq!(rgba.len(), (width as usize) * (height as usize) * 4);
        if inner.tex_size != (width, height) {
            let (tex, bg) = make_texture_and_bg(
                &inner.device,
                &inner.queue,
                &inner.bgl,
                &inner.uniforms,
                &inner.sampler,
                width,
                height,
                rgba,
            );
            inner.tex = tex;
            inner.bind_group = bg;
            inner.tex_size = (width, height);
        } else {
            inner.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &inner.tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Tamaño actual de la textura interna (último upload o (1,1) si
    /// nunca se subió nada).
    pub fn size(&self) -> (u32, u32) {
        self.inner.lock().tex_size
    }

    /// Encola el draw del quad que pinta la surface en `dst_view` dentro
    /// de `rect`, escalando la textura para cubrir el rect entero.
    /// Llamado típicamente desde el callback de `View::gpu_paint_with`.
    pub fn blit(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        dst_view: &wgpu::TextureView,
        rect: PaintRect,
        viewport: (u32, u32),
    ) {
        // Comportamiento clásico: la textura entera estirada al rect, recortada
        // al propio rect. `blit_layout` con `src_uv` = textura completa y el
        // `clip` = el mismo rect.
        self.blit_layout(
            queue,
            encoder,
            dst_view,
            [rect.x, rect.y, rect.w, rect.h],
            [0.0, 0.0, 1.0, 1.0],
            [rect.x, rect.y, rect.w, rect.h],
            viewport,
        );
    }

    /// Blit con control de **origen y destino** (V2: aspect/crop/zoom/pan).
    /// `dst` es el rectángulo (px de viewport) donde dibujar — puede exceder el
    /// `clip`; `src_uv` (`u0, v0, du, dv` en 0..1) es el sub-rectángulo de la
    /// textura a muestrear (crop); `clip` (px de viewport) acota el dibujado con
    /// un scissor — típicamente el rect del canvas, así el sobrante de
    /// Fill/zoom/pan no pinta fuera de su área. El `dst`/`src_uv` los calcula
    /// `media_core::viewport::compute_layout`.
    pub fn blit_layout(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        dst_view: &wgpu::TextureView,
        dst: [f32; 4],
        src_uv: [f32; 4],
        clip: [f32; 4],
        viewport: (u32, u32),
    ) {
        let inner = self.inner.lock();
        let uniforms = [
            dst[0],
            dst[1],
            dst[2],
            dst[3],
            viewport.0 as f32,
            viewport.1 as f32,
            0.0,
            0.0,
            src_uv[0],
            src_uv[1],
            src_uv[2],
            src_uv[3],
        ];
        let mut bytes = [0u8; 48];
        for (i, v) in uniforms.iter().enumerate() {
            bytes[i * 4..(i + 1) * 4].copy_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&inner.uniforms, 0, &bytes);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-surface-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dst_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&inner.pipeline);
        pass.set_bind_group(0, &inner.bind_group, &[]);
        // Scissor = clip ∩ viewport (en px enteros). Evita que el sobrante de
        // Fill/zoom/pan pinte fuera del área del canvas.
        let (vw, vh) = (viewport.0 as f32, viewport.1 as f32);
        let x0 = clip[0].max(0.0).floor();
        let y0 = clip[1].max(0.0).floor();
        let x1 = (clip[0] + clip[2]).min(vw).ceil();
        let y1 = (clip[1] + clip[3]).min(vh).ceil();
        if x1 > x0 && y1 > y0 {
            pass.set_scissor_rect(x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32);
        }
        pass.draw(0..6, 0..1);
    }

    /// Construye un `View` cuyo `gpu_paint_with` blittea la surface al
    /// rect que le asigne el layout. La app sólo escoge el `Style`
    /// (tamaño, flex_grow…). El `Msg` está libre — la View no emite
    /// eventos por sí sola.
    pub fn view<Msg>(&self, style: llimphi_ui::llimphi_layout::taffy::Style) -> View<Msg>
    where
        Msg: Clone + Send + Sync + 'static,
    {
        let this = self.clone();
        View::new(style).gpu_paint_with(move |_device, queue, encoder, view, rect, viewport| {
            this.blit(queue, encoder, view, rect, viewport);
        })
    }
}

fn make_texture_and_bg(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bgl: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    initial_rgba: &[u8],
) -> (wgpu::Texture, wgpu::BindGroup) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("llimphi-surface-tex"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SOURCE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        initial_rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("llimphi-surface-bg"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    (tex, bind_group)
}

const WGSL: &str = r#"
struct Uniforms {
    rect: vec4<f32>,     // x, y, w, h del rect destino en pixels del frame
    viewport: vec4<f32>, // vw, vh, _, _
    src_uv: vec4<f32>,   // u0, v0, du, dv: sub-rect de textura a muestrear (UV)
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct V2F {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vid: u32) -> V2F {
    // Dos triángulos en UV-space, recorridos CCW.
    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    // `q` recorre el quad destino (0..1); el muestreo va al sub-rect `src_uv`.
    let q = uvs[vid];

    let px = u.rect.x + q.x * u.rect.z;
    let py = u.rect.y + q.y * u.rect.w;

    // NDC: x ∈ [-1, 1] sin flip, y flipeado (en pantalla y-down).
    let ndc = vec2<f32>(
        px / u.viewport.x * 2.0 - 1.0,
        1.0 - py / u.viewport.y * 2.0,
    );

    var out: V2F;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = u.src_uv.xy + q * u.src_uv.zw;
    return out;
}

@fragment
fn fs(in: V2F) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
"#;
