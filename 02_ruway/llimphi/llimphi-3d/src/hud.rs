//! `Hud` — un pase **screen-space** mínimo: dibuja rectángulos de color plano
//! (con alpha) directamente en NDC, *después* del pase 3D, sobre el mismo
//! target. Es la pieza que faltaba para un **HUD / mira (crosshair)** en primera
//! persona: el contenido vello del árbol Llimphi queda **debajo** del canvas GPU
//! full-screen, así que cualquier overlay que deba ir *encima* del ray-march
//! tiene que pintarse en GPU en la misma closure `gpu_paint_with`, y eso es
//! justo lo que hace [`Hud::render`].
//!
//! Deliberadamente tonto: sin texturas, sin bind groups, sin depth. Geometría
//! en CPU → un vertex buffer dinámico → un draw. Suficiente para miras, barras,
//! marcos y **texto** ([`HudQuad::text`], fuente bitmap 5×7 = un quad por píxel
//! encendido, sin salir del pipeline de quads).

/// Un rectángulo de HUD en **pixels** (origen arriba-izquierda, como la
/// pantalla), color RGBA lineal `0..1`.
#[derive(Debug, Clone, Copy)]
pub struct HudQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [f32; 4],
}

impl HudQuad {
    /// Una **mira (crosshair)** centrada en un viewport `(w, h)`: dos barras
    /// (horizontal + vertical) de brazo `arm` y grosor `th` pixels.
    pub fn crosshair(viewport: (u32, u32), arm: f32, th: f32, color: [f32; 4]) -> [HudQuad; 2] {
        let cx = viewport.0 as f32 * 0.5;
        let cy = viewport.1 as f32 * 0.5;
        [
            HudQuad { x: cx - arm, y: cy - th * 0.5, w: arm * 2.0, h: th, color },
            HudQuad { x: cx - th * 0.5, y: cy - arm, w: th, h: arm * 2.0, color },
        ]
    }

    /// Emite los quads de una cadena con la **fuente bitmap 5×7** embebida
    /// ([`glyph`]): origen arriba-izquierda en `(x, y)` pixels, cada píxel de
    /// glifo mide `px` pixels de lado y los caracteres avanzan `6·px` (5 de ancho
    /// + 1 de espacio). Sólo ASCII; las minúsculas se dibujan en mayúscula y los
    /// caracteres desconocidos quedan en blanco. Se mantiene dentro del pipeline
    /// tonto del HUD (un quad por píxel encendido, sin texturas).
    pub fn text(s: &str, x: f32, y: f32, px: f32, color: [f32; 4]) -> Vec<HudQuad> {
        let mut out = Vec::new();
        let mut cx = x;
        for ch in s.chars() {
            if ch != ' ' {
                let g = glyph(ch);
                for (r, row) in g.iter().enumerate() {
                    for c in 0..5u32 {
                        if row & (1 << (4 - c)) != 0 {
                            out.push(HudQuad {
                                x: cx + c as f32 * px,
                                y: y + r as f32 * px,
                                w: px,
                                h: px,
                                color,
                            });
                        }
                    }
                }
            }
            cx += 6.0 * px;
        }
        out
    }

    /// Ancho en pixels que ocuparía `s` con [`text`](Self::text) a tamaño `px`
    /// (útil para dimensionar un panel de fondo antes de dibujar el texto).
    pub fn text_width(s: &str, px: f32) -> f32 {
        s.chars().count() as f32 * 6.0 * px
    }
}

/// Mapa de un carácter a su bitmap **5×7**: 7 filas, cada `u8` con los 5 bits
/// bajos = columnas de izquierda (bit 4) a derecha (bit 0). Cubre `0-9`, `A-Z`
/// y puntuación común; lo desconocido devuelve un glifo en blanco. Las filas se
/// escriben en binario para que la forma sea legible en el código.
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10101, 0b10011, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        ':' => [0b00000, 0b00100, 0b00000, 0b00000, 0b00100, 0b00000, 0b00000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00100],
        ',' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00100, 0b01000],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '+' => [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000],
        '/' => [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000],
        '(' => [0b00110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b00110],
        ')' => [0b01100, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01100],
        '%' => [0b11001, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b10011],
        _ => [0; 7],
    }
}

/// Tamaño de un vértice del HUD: `pos: vec2<f32>` + `color: vec4<f32>`.
const VSIZE: usize = 2 * 4 + 4 * 4;

/// Renderer de overlay screen-space. Cachea pipeline + un vertex buffer
/// dinámico que crece según haga falta.
pub struct Hud {
    pipeline: wgpu::RenderPipeline,
    vbuf: wgpu::Buffer,
    cap: u64,
}

impl Hud {
    /// Crea el HUD para el `color_format` del target (el de la intermedia del
    /// frame). No toca depth: dibuja siempre encima.
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-3d-hud-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-3d-hud-pl"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-3d-hud-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: VSIZE as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 8,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            // Sin depth: el HUD va siempre encima del 3D.
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let cap = (64 * 6 * VSIZE) as u64; // ~64 quads sin recrear
        let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-hud-vbuf"),
            size: cap,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self { pipeline, vbuf, cap }
    }

    /// Dibuja `quads` sobre `target` (color `LoadOp::Load`, sin depth). Firma
    /// compatible con la closure `gpu_paint_with`: llamar *después* del pase 3D.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        (w, h): (u32, u32),
        quads: &[HudQuad],
    ) {
        if w == 0 || h == 0 || quads.is_empty() {
            return;
        }

        // Geometría en CPU: 2 triángulos (6 vértices) por quad, en NDC. El eje Y
        // de pantalla va hacia abajo; NDC hacia arriba → `1 - 2·y/h`.
        let (fw, fh) = (w as f32, h as f32);
        let mut bytes = Vec::with_capacity(quads.len() * 6 * VSIZE);
        let mut vert = |x_px: f32, y_px: f32, c: [f32; 4]| {
            let ndc_x = x_px / fw * 2.0 - 1.0;
            let ndc_y = 1.0 - y_px / fh * 2.0;
            bytes.extend_from_slice(&ndc_x.to_ne_bytes());
            bytes.extend_from_slice(&ndc_y.to_ne_bytes());
            for ch in c {
                bytes.extend_from_slice(&ch.to_ne_bytes());
            }
        };
        for q in quads {
            let (x0, y0, x1, y1) = (q.x, q.y, q.x + q.w, q.y + q.h);
            vert(x0, y0, q.color);
            vert(x1, y0, q.color);
            vert(x1, y1, q.color);
            vert(x0, y0, q.color);
            vert(x1, y1, q.color);
            vert(x0, y1, q.color);
        }

        // Crecer el buffer si hiciera falta (raro: la mira son 2 quads).
        if bytes.len() as u64 > self.cap {
            self.cap = (bytes.len() as u64).next_power_of_two();
            self.vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("llimphi-3d-hud-vbuf"),
                size: self.cap,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.vbuf, 0, &bytes);

        let count = (quads.len() * 6) as u32;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-3d-hud-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vbuf.slice(..bytes.len() as u64));
        pass.draw(0..count, 0..1);
    }
}

const WGSL: &str = r#"
struct VIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    out.clip = vec4<f32>(in.pos, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;
