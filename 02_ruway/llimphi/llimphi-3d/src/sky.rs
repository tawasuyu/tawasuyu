//! `SkyBackdrop` — fondo de cielo **cilíndrico** genérico: un triángulo
//! fullscreen que muestrea una textura de panorama por el azimut de la cámara,
//! dibujado detrás de todo (depth `Always`, sin escribir depth) para que la
//! geometría del mundo lo tape.
//!
//! Cosechado del cielo 2.5D de Doom (`supay-render-llimphi::wgpu3d`), donde
//! estaba pegado a las constantes de Doom (tileo 4×/360°, estiramiento vertical
//! 1.8×). Acá esos números son **parámetros** ([`SkyParams`]), así un panorama
//! normal usa los defaults y Doom reproduce su look exacto pasando los suyos.
//!
//! Se dibuja **primero** en un pase que ya tiene depth attachment (el de
//! [`Scene3d`](crate::Scene3d) o el de [`PostFx::scene_pass`](crate::PostFx)):
//!
//! ```ignore
//! sky.upload(queue, &SkyParams { yaw, pitch, fov_x, aspect, ..Default::default() });
//! // dentro del pase, ANTES de la geometría:
//! sky.draw(&mut pass);
//! ```

use crate::scene::DEPTH_FORMAT;

/// Cómo se proyecta el panorama sobre la pantalla.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkyMapping {
    /// **Cilíndrico** (estilo Doom): la columna muestrea por el azimut y la fila
    /// se desplaza linealmente con el `pitch`. Barato y exacto para una cámara
    /// que casi no cabecea (FPS), pero **degenera** si la cámara mira muy
    /// arriba/abajo (la textura se sale de pantalla y el borde se estira en
    /// rayas verticales). Usa `wraps`/`v_scale`/`pitch_scale`/`v_offset`.
    #[default]
    Cylindrical,
    /// **Esférico** (equirectangular): por cada píxel se reconstruye el rayo de
    /// cámara desde `yaw`/`pitch`/`fov_x`/`aspect` y se mapea a la textura por
    /// (azimut, elevación). Correcto a cualquier cabeceo y bloqueado al mundo
    /// como la geometría — el cielo es una esfera de fondo, no una franja. Los
    /// campos `wraps`/`v_scale`/`pitch_scale`/`v_offset` se ignoran.
    Spherical,
}

/// Parámetros del cielo por frame. `yaw`/`pitch`/`fov_x` salen de la cámara;
/// el resto modela cómo se mapea la textura.
#[derive(Clone, Copy, Debug)]
pub struct SkyParams {
    /// Azimut de la cámara (rad). El centro de pantalla muestrea esta columna.
    pub yaw: f32,
    /// Cabeceo de la cámara (rad). Desplaza el cielo en vertical.
    pub pitch: f32,
    /// Campo de visión horizontal (rad). Define cuánto panorama abarca el ancho.
    pub fov_x: f32,
    /// Aspect ratio del viewport (w/h). Reservado para usos futuros.
    pub aspect: f32,
    /// Cuántas veces se repite la textura al dar la vuelta 360°. `1.0` = un
    /// panorama completo; Doom usa `4.0` (el cielo tilea 4×).
    pub wraps: f32,
    /// Escala vertical de la textura sobre la pantalla. `1.0` = la textura ocupa
    /// toda la altura; Doom usa `1.8` (franja superior + horizonte al medio).
    pub v_scale: f32,
    /// Cuánto desplaza el `pitch` la textura en vertical. Doom usa `0.6`.
    pub pitch_scale: f32,
    /// Offset vertical fijo añadido a la coordenada de textura.
    pub v_offset: f32,
    /// Proyección del panorama. Ver [`SkyMapping`].
    pub mapping: SkyMapping,
}

impl Default for SkyParams {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            fov_x: std::f32::consts::FRAC_PI_2,
            aspect: 1.0,
            wraps: 1.0,
            v_scale: 1.0,
            pitch_scale: 1.0,
            v_offset: 0.0,
            mapping: SkyMapping::Cylindrical,
        }
    }
}

/// Textura de cielo subida + su bind group.
struct SkyTexture {
    bind_group: wgpu::BindGroup,
}

/// Fondo de cielo cilíndrico reutilizable. Sin textura, [`Self::draw`] es no-op.
pub struct SkyBackdrop {
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    tex: Option<SkyTexture>,
}

impl SkyBackdrop {
    /// Crea el backdrop para el `color_format` del target. El pipeline declara
    /// depth [`DEPTH_FORMAT`] (`Always`, sin escribir) → debe dibujarse en un
    /// pase con depth attachment, como los de [`Scene3d`](crate::Scene3d)/
    /// [`PostFx`](crate::PostFx).
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-sky-uniform-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-sky-tex-layout"),
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-3d-sky-shader"),
            source: wgpu::ShaderSource::Wgsl(SKY_WGSL.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-3d-sky-pl"),
            bind_group_layouts: &[&uniform_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-3d-sky-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
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

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-sky-uniform"),
            size: 48, // 9 × f32 redondeado a múltiplo de 16
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-sky-uniform-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-3d-sky-sampler"),
            // Horizontal envuelve (panorama 360°); vertical clampea (no repite
            // el cielo hacia abajo).
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            pipeline,
            uniform_buf,
            uniform_bg,
            tex_layout,
            sampler,
            tex: None,
        }
    }

    /// ¿Ya tiene textura cargada? Útil para subirla una sola vez.
    pub fn has_texture(&self) -> bool {
        self.tex.is_some()
    }

    /// Sube/reemplaza la textura del cielo (RGBA8, `w×h`, fila contigua sin
    /// padding). `data.len()` debe ser `w*h*4`.
    pub fn set_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        data: &[u8],
    ) {
        assert_eq!(data.len(), (w * h * 4) as usize, "RGBA8 w*h*4 esperado");
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-sky-tex"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
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
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let view = tex.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-sky-tex-bg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.tex = Some(SkyTexture { bind_group });
    }

    /// Sube los parámetros del frame. Llamar antes de [`Self::draw`].
    pub fn upload(&self, queue: &wgpu::Queue, p: &SkyParams) {
        let mode = match p.mapping {
            SkyMapping::Cylindrical => 0.0,
            SkyMapping::Spherical => 1.0,
        };
        let mut b = Vec::with_capacity(48);
        for v in [
            p.yaw,
            p.pitch,
            p.fov_x,
            p.aspect,
            p.wraps,
            p.v_scale,
            p.pitch_scale,
            p.v_offset,
            mode,
            0.0, // pad a múltiplo de 16
            0.0,
            0.0,
        ] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.uniform_buf, 0, &b);
    }

    /// Dibuja el cielo en un pase **ya abierto** (con depth attachment), como
    /// **primer** draw (el mundo lo tapa después). No-op si no hay textura.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let Some(tex) = self.tex.as_ref() else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bg, &[]);
        pass.set_bind_group(1, &tex.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Fondo cilíndrico: muestreo de la textura por el azimut de cada columna.
const SKY_WGSL: &str = r#"
struct SkyU {
    yaw: f32, pitch: f32, fov_x: f32, aspect: f32,
    wraps: f32, v_scale: f32, pitch_scale: f32, v_offset: f32,
    mode: f32, _pad0: f32, _pad1: f32, _pad2: f32,
};
@group(0) @binding(0) var<uniform> s: SkyU;
@group(1) @binding(0) var sky: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

const PI: f32 = 3.14159265;

struct SOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) scr: vec2<f32>,   // uv de pantalla, (0,0) = arriba-izquierda
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> SOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var o: SOut;
    let c = p[vi];
    o.clip = vec4<f32>(c, 0.0, 1.0);
    o.scr = vec2<f32>(c.x * 0.5 + 0.5, 1.0 - (c.y * 0.5 + 0.5));
    return o;
}

@fragment
fn fs(in: SOut) -> @location(0) vec4<f32> {
    if (s.mode > 0.5) {
        // ---- Esférico (equirectangular): reconstruir el rayo de cámara ----
        // Base de cámara igual a Camera3d::orbit(target=0): el ojo está en
        // (cp·sy, sp, cp·cy)·dist y mira al origen, up=+Y.
        let sy = sin(s.yaw);  let cy = cos(s.yaw);
        let sp = sin(s.pitch); let cp = cos(s.pitch);
        let zaxis = normalize(vec3<f32>(cp * sy, sp, cp * cy)); // ojo→origen invertido
        let xaxis = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), zaxis));
        let yaxis = cross(zaxis, xaxis);
        let tan_x = tan(s.fov_x * 0.5);
        let tan_y = tan_x / s.aspect;
        let ndc_x = in.scr.x * 2.0 - 1.0;
        let ndc_y = 1.0 - in.scr.y * 2.0; // arriba = +1
        // La cámara mira hacia -zaxis (look_at_rh).
        let dir = normalize(ndc_x * tan_x * xaxis + ndc_y * tan_y * yaxis - zaxis);
        let az = atan2(dir.x, dir.z);                 // -PI..PI
        let el = asin(clamp(dir.y, -1.0, 1.0));       // -PI/2..PI/2
        let u = az / (2.0 * PI) + 0.5;
        let v = 0.5 - el / PI;                        // arriba(+el) → v=0
        let col = textureSample(sky, samp, vec2<f32>(u, v));
        // El equirectangular tiene una singularidad en los polos (cenit/nadir):
        // todas las columnas colapsan a una fila y la textura se abre en abanico
        // ("starburst"). Atenuamos el muestreo a oscuro cerca del polo para que
        // se disuelva en cielo profundo en vez de reventar en rayas radiales.
        let pole = smoothstep(0.9, 0.998, abs(dir.y));
        return col * (1.0 - pole);
    }
    // ---- Cilíndrico (Doom): azimut por columna, fila por pitch lineal ----
    let colang = s.yaw - (in.scr.x - 0.5) * s.fov_x;
    let su = fract(colang / (2.0 * PI) * s.wraps);
    let sv = clamp(in.scr.y * s.v_scale - s.pitch * s.pitch_scale + s.v_offset, 0.0, 1.0);
    return textureSample(sky, samp, vec2<f32>(su, sv));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_wgsl_valida() {
        let module = naga::front::wgsl::parse_str(SKY_WGSL).expect("SKY_WGSL no parsea");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("SKY_WGSL no valida");
    }

    #[test]
    fn defaults_panorama_simple() {
        let p = SkyParams::default();
        assert_eq!(p.wraps, 1.0);
        assert_eq!(p.v_scale, 1.0);
    }
}
