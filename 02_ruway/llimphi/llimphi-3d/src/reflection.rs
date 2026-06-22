//! `PlanarReflection` — reflexión planar genérica: una superficie espejada
//! (agua, piso pulido, vidrio) que refleja el mundo a través de un plano.
//!
//! Cosechado del agua 2.5D de Doom (`supay-render-llimphi::wgpu3d`), donde la
//! reflexión estaba cableada al plano horizontal de Doom (`reflect_across_z`) y
//! al shader del agua. Acá:
//!
//!  - [`ReflectionPlane`] modela un plano **arbitrario** (`n·x + d = 0`) y da su
//!    matriz espejo ([`ReflectionPlane::mirror`]) por reflexión de Householder.
//!  - [`PlanarReflection`] administra el **render target de reflexión**
//!    (color+depth) — el caller rinde su mundo con la `view_proj · mirror` ahí —
//!    y trae una **superficie reflectante lista**: un quad sobre el plano que
//!    muestrea esa textura en screen-space, con ondas procedurales + Fresnel +
//!    tinte. El caller puede usar esa superficie o sólo la textura para su
//!    propio shader.
//!
//! Flujo:
//! ```ignore
//! let m = plane.mirror();
//! refl.prepare(&device, (w, h));
//! { let mut rp = refl.reflection_pass(&mut enc, clear);
//!   // rendir el mundo con view_proj * m (clip bajo el plano en tu shader) }
//! // ... pase principal: mundo normal ...
//! refl.set_surface_quad(&device, corners);
//! refl.upload_surface(&queue, &view_proj, &plane, &eye, &SurfaceParams::default(), (w,h), time);
//! refl.draw_surface(&mut pass); // en el pase principal (con depth)
//! ```

use glam::{Mat4, Vec3};

use crate::scene::DEPTH_FORMAT;

/// Plano `n·x + d = 0` con `n` unitaria. La reflexión espeja a través de él.
#[derive(Clone, Copy, Debug)]
pub struct ReflectionPlane {
    /// Normal del plano (se normaliza en [`Self::mirror`]).
    pub normal: [f32; 3],
    /// Término independiente: distancia con signo del origen, en unidades de `n`.
    pub d: f32,
}

impl ReflectionPlane {
    /// Plano horizontal a altura `y` con normal `+Y` (piso/agua, mundo Y-up).
    pub fn horizontal_y(y: f32) -> Self {
        Self { normal: [0.0, 1.0, 0.0], d: -y }
    }

    /// Plano horizontal a altura `z` con normal `+Z` (mundo Z-up, como Doom).
    pub fn horizontal_z(z: f32) -> Self {
        Self { normal: [0.0, 0.0, 1.0], d: -z }
    }

    /// Matriz de **reflexión de Householder** a través del plano: espeja
    /// cualquier punto a su imagen del otro lado. `mvp_reflejada = view_proj ·
    /// mirror`. Equivale, para un plano horizontal, al `T·S(-1)·T⁻¹` de Doom.
    pub fn mirror(&self) -> Mat4 {
        let n = Vec3::from_array(self.normal).normalize_or_zero();
        let (nx, ny, nz) = (n.x, n.y, n.z);
        // Renormalizamos d al `n` unitario.
        let len = Vec3::from_array(self.normal).length().max(1e-8);
        let d = self.d / len;
        // Column-major (glam): cada `Vec4` es una columna.
        Mat4::from_cols(
            [1.0 - 2.0 * nx * nx, -2.0 * nx * ny, -2.0 * nx * nz, 0.0].into(),
            [-2.0 * nx * ny, 1.0 - 2.0 * ny * ny, -2.0 * ny * nz, 0.0].into(),
            [-2.0 * nx * nz, -2.0 * ny * nz, 1.0 - 2.0 * nz * nz, 0.0].into(),
            [-2.0 * nx * d, -2.0 * ny * d, -2.0 * nz * d, 1.0].into(),
        )
    }
}

/// Parámetros de la superficie reflectante.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceParams {
    /// Amplitud de la distorsión por ondas (en uv de pantalla). `0` = espejo liso.
    pub ripple_strength: f32,
    /// Frecuencia espacial de las ondas (en unidades de mundo).
    pub ripple_scale: f32,
    /// Exponente Fresnel: más alto = la reflexión sólo aparece muy rasante.
    pub fresnel_power: f32,
    /// Reflectividad máxima (a incidencia rasante). `1` = espejo; `<1` mezcla tinte.
    pub reflectivity: f32,
    /// Reflexión mínima mirando de frente (`0`..`reflectivity`).
    pub base_reflect: f32,
    /// Color/tinte de la superficie donde no refleja (rgb) + opacidad (a).
    pub tint: [f32; 4],
}

impl Default for SurfaceParams {
    fn default() -> Self {
        Self {
            ripple_strength: 0.012,
            ripple_scale: 1.6,
            fresnel_power: 4.0,
            reflectivity: 0.9,
            base_reflect: 0.08,
            tint: [0.04, 0.10, 0.16, 1.0],
        }
    }
}

/// Render target de reflexión (color+depth) + su bind group para muestrearlo.
struct ReflTarget {
    w: u32,
    h: u32,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    tex_bg: wgpu::BindGroup,
}

/// Reflexión planar reutilizable: target de reflexión + superficie reflectante.
pub struct PlanarReflection {
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target: Option<ReflTarget>,
    // Superficie:
    surface_pipeline: wgpu::RenderPipeline,
    surface_uniform: wgpu::Buffer,
    surface_uniform_bg: wgpu::BindGroup,
    surface_quad: Option<wgpu::Buffer>,
}

impl PlanarReflection {
    /// Crea el sistema para el `color_format` del target final.
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-refl-tex-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-refl-uniform-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-3d-refl-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-3d-refl-surface-shader"),
            source: wgpu::ShaderSource::Wgsl(SURFACE_WGSL.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-3d-refl-surface-pl"),
            bind_group_layouts: &[&uniform_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let surface_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-3d-refl-surface-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let surface_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-refl-surface-uniform"),
            size: SURFACE_UNIFORM_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let surface_uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-refl-surface-uniform-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: surface_uniform.as_entire_binding(),
            }],
        });

        Self {
            tex_layout,
            sampler,
            target: None,
            surface_pipeline,
            surface_uniform,
            surface_uniform_bg,
            surface_quad: None,
        }
    }

    /// Asegura el render target de reflexión a `(w,h)` (lo recrea al cambiar).
    pub fn prepare(&mut self, device: &wgpu::Device, (w, h): (u32, u32)) {
        if w == 0 || h == 0 {
            return;
        }
        if matches!(&self.target, Some(t) if t.w == w && t.h == h) {
            return;
        }
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-refl-color"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color.create_view(&Default::default());
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-refl-depth"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let tex_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-refl-tex-bg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.target = Some(ReflTarget { w, h, color_view, depth_view, tex_bg });
    }

    /// Abre el pase de reflexión (color limpiado a `clear` + depth limpio) y
    /// devuelve el [`wgpu::RenderPass`] para que el caller rinda el mundo con la
    /// `view_proj · plane.mirror()`. Requiere [`Self::prepare`]. Soltar el pase
    /// antes de muestrear la textura.
    pub fn reflection_pass<'p>(
        &'p self,
        encoder: &'p mut wgpu::CommandEncoder,
        clear: wgpu::Color,
    ) -> wgpu::RenderPass<'p> {
        let t = self
            .target
            .as_ref()
            .expect("PlanarReflection::reflection_pass sin prepare()");
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-3d-refl-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &t.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        })
    }

    /// Define el quad de la superficie reflectante por sus 4 esquinas en mundo
    /// (orden CCW: por ejemplo `[bl, br, tr, tl]`). Se dibuja como 2 triángulos.
    pub fn set_surface_quad(&mut self, device: &wgpu::Device, corners: [[f32; 3]; 4]) {
        let idx = [0usize, 1, 2, 0, 2, 3];
        let mut bytes = Vec::with_capacity(6 * 12);
        for &i in &idx {
            for v in corners[i] {
                bytes.extend_from_slice(&v.to_ne_bytes());
            }
        }
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-refl-surface-quad"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: true,
        });
        buf.slice(..).get_mapped_range_mut().copy_from_slice(&bytes);
        buf.unmap();
        self.surface_quad = Some(buf);
    }

    /// Sube el uniform de la superficie. `view_proj` es la cámara real (no la
    /// reflejada), `eye` la posición del ojo, `time` para animar las ondas,
    /// `screen` el tamaño del target final en px.
    #[allow(clippy::too_many_arguments)]
    pub fn upload_surface(
        &self,
        queue: &wgpu::Queue,
        view_proj: &Mat4,
        plane: &ReflectionPlane,
        eye: &[f32; 3],
        params: &SurfaceParams,
        screen: (u32, u32),
        time: f32,
    ) {
        let n = Vec3::from_array(plane.normal).normalize_or_zero();
        let len = Vec3::from_array(plane.normal).length().max(1e-8);
        let d = plane.d / len;
        let mut b = Vec::with_capacity(SURFACE_UNIFORM_SIZE);
        for v in view_proj.to_cols_array() {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [eye[0], eye[1], eye[2], time] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [n.x, n.y, n.z, d] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [
            params.ripple_strength,
            params.ripple_scale,
            params.fresnel_power,
            params.reflectivity,
        ] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        for v in params.tint {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [screen.0 as f32, screen.1 as f32, params.base_reflect, 0.0] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        debug_assert_eq!(b.len(), SURFACE_UNIFORM_SIZE);
        queue.write_buffer(&self.surface_uniform, 0, &b);
    }

    /// Dibuja la superficie reflectante en un pase **ya abierto** (con depth),
    /// muestreando la textura de reflexión. Requiere `prepare` + `set_surface_quad`
    /// + `upload_surface`. No-op si falta algo.
    pub fn draw_surface<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(t), Some(quad)) = (self.target.as_ref(), self.surface_quad.as_ref()) else {
            return;
        };
        pass.set_pipeline(&self.surface_pipeline);
        pass.set_bind_group(0, &self.surface_uniform_bg, &[]);
        pass.set_bind_group(1, &t.tex_bg, &[]);
        pass.set_vertex_buffer(0, quad.slice(..));
        pass.draw(0..6, 0..1);
    }
}

/// Uniform de la superficie: `view_proj`(64) + `eye_time`(16) + `plane`(16) +
/// `params`(16) + `tint`(16) + `screen_basereflect`(16).
const SURFACE_UNIFORM_SIZE: usize = 64 + 16 * 5;

/// Superficie reflectante: muestrea la textura de reflexión en screen-space,
/// con ondas procedurales + Fresnel + tinte.
const SURFACE_WGSL: &str = r#"
struct U {
    view_proj: mat4x4<f32>,
    eye_time: vec4<f32>,   // xyz=ojo, w=time
    plane: vec4<f32>,      // xyz=normal, w=d
    params: vec4<f32>,     // ripple_strength, ripple_scale, fresnel_power, reflectivity
    tint: vec4<f32>,       // rgb=color base, a=opacidad
    screen: vec4<f32>,     // w, h, base_reflect, _
};
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var refl: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs(@location(0) pos: vec3<f32>) -> VOut {
    var o: VOut;
    o.clip = u.view_proj * vec4<f32>(pos, 1.0);
    o.world = pos;
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // uv de pantalla a partir del builtin position (px, origen arriba-izq).
    var suv = in.clip.xy / vec2<f32>(u.screen.x, u.screen.y);
    // Ondas: distorsión sinusoidal cruzada en función de la posición de mundo.
    let t = u.eye_time.w;
    let sc = u.params.y;
    let ripple = vec2<f32>(
        sin(in.world.x * sc + t) + cos(in.world.z * sc * 1.3 - t * 0.7),
        cos(in.world.z * sc + t * 1.1) + sin(in.world.x * sc * 0.9 + t),
    ) * u.params.x;
    suv = clamp(suv + ripple, vec2<f32>(0.0), vec2<f32>(1.0));
    let reflected = textureSample(refl, samp, suv).rgb;

    // Fresnel: más reflexión a incidencia rasante.
    let n = normalize(u.plane.xyz);
    let view_dir = normalize(u.eye_time.xyz - in.world);
    let f = pow(1.0 - clamp(dot(view_dir, n), 0.0, 1.0), u.params.z);
    let amount = clamp(u.screen.z + f * u.params.w, 0.0, 1.0);

    let col = mix(u.tint.rgb, reflected, amount);
    return vec4<f32>(col, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_wgsl_valida() {
        let module = naga::front::wgsl::parse_str(SURFACE_WGSL).expect("SURFACE_WGSL no parsea");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("SURFACE_WGSL no valida");
    }

    #[test]
    fn mirror_espeja_punto_sobre_plano() {
        // Plano horizontal y=2: (0,5,0) → (0,-1,0).
        let m = ReflectionPlane::horizontal_y(2.0).mirror();
        let p = m.transform_point3(Vec3::new(0.0, 5.0, 0.0));
        assert!((p - Vec3::new(0.0, -1.0, 0.0)).length() < 1e-4, "fue {p:?}");
        // Un punto en el plano queda fijo.
        let q = m.transform_point3(Vec3::new(3.0, 2.0, -1.0));
        assert!((q - Vec3::new(3.0, 2.0, -1.0)).length() < 1e-4, "fue {q:?}");
    }

    #[test]
    fn mirror_z_equivale_a_doom() {
        // El reflect_across_z de Doom: T(z)·S(-1z)·T(-z). Comparamos en z=3.
        let m = ReflectionPlane::horizontal_z(3.0).mirror();
        let p = m.transform_point3(Vec3::new(1.0, 2.0, 7.0));
        assert!((p - Vec3::new(1.0, 2.0, -1.0)).length() < 1e-4, "fue {p:?}");
    }

    #[test]
    fn uniform_size_alineado() {
        assert_eq!(SURFACE_UNIFORM_SIZE % 16, 0);
    }
}
