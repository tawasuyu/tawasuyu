//! Pipeline GPU para la grilla de celdas del modo TUI (Fase 4 del SDD-TERMINAL).
//!
//! Define las **estructuras POD** (instance + uniforms) y el **shader WGSL**
//! del render. La compilación wgpu y el dibujo viven en `CellPipeline::new`
//! / `::draw` (un commit siguiente puede agregarlos, una vez que el atlas y
//! estas estructuras estén validadas headless).
//!
//! ## Layouts
//!
//! - **Instance** (32 B): `[cell_x: f32, cell_y: f32, uv_x: f32, uv_y: f32,
//!   uv_w: f32, uv_h: f32, fg_rgba: u32, bg_rgba: u32]`.
//!   Una por celda visible; el vertex stage emite los 4 corners (TriangleStrip).
//! - **Uniforms** (32 B): `[viewport_w: f32, viewport_h: f32, cell_w: f32,
//!   cell_h: f32, atlas_w: f32, atlas_h: f32, _pad: [f32; 2]]`.
//!
//! El fragment samplea el atlas grayscale en `uv`; alpha = cobertura;
//! out = mix(bg, fg, alpha). Blending estándar `OVER` por encima.
//!
//! ## Por qué quads instanciados
//!
//! Una grilla de 100×40 = 4000 celdas; en vello eso son ~4000 Views + 4000
//! draws + el shaping de cada char. Con quads instanciados es UNA draw call
//! de 4000 instancias y la GPU pinta todo en paralelo. Igual de simple
//! para 200×80 (16k celdas) — patrón ya validado en `GpuPipelines.rects`.

/// Una celda lista para dibujar. **POD, repr(C)** — `as_bytes` la serializa
/// a una secuencia plana de `f32`/`u32` little-endian para el buffer GPU.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct CellInstance {
    /// Posición (px) de la esquina superior-izquierda de la celda en
    /// viewport coords.
    pub cell_x: f32,
    pub cell_y: f32,
    /// Coords UV (px) del glifo en la textura del atlas. El shader las
    /// divide por `atlas_size` para obtener UVs normalizadas 0..1.
    pub uv_x: f32,
    pub uv_y: f32,
    pub uv_w: f32,
    pub uv_h: f32,
    /// Color foreground del glifo, RGBA8 empacado little-endian
    /// (`r | g<<8 | b<<16 | a<<24`).
    pub fg_rgba: u32,
    /// Color background de la celda, RGBA8 empacado.
    pub bg_rgba: u32,
}

impl CellInstance {
    /// Tamaño en bytes del layout — debe coincidir con `array_stride` del
    /// pipeline en wgpu. Compile-time const para que el caller arme el
    /// vertex layout sin recalcular.
    pub const SIZE: usize = 32;

    /// Serializa a 32 bytes little-endian para `Queue::write_buffer`.
    pub fn as_bytes(&self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..4].copy_from_slice(&self.cell_x.to_le_bytes());
        out[4..8].copy_from_slice(&self.cell_y.to_le_bytes());
        out[8..12].copy_from_slice(&self.uv_x.to_le_bytes());
        out[12..16].copy_from_slice(&self.uv_y.to_le_bytes());
        out[16..20].copy_from_slice(&self.uv_w.to_le_bytes());
        out[20..24].copy_from_slice(&self.uv_h.to_le_bytes());
        out[24..28].copy_from_slice(&self.fg_rgba.to_le_bytes());
        out[28..32].copy_from_slice(&self.bg_rgba.to_le_bytes());
        out
    }
}

/// Empaca un color `(r, g, b, a)` en un `u32` RGBA little-endian que el
/// shader lee como `vec4<u32>` y normaliza a `vec4<f32>(r,g,b,a)/255`.
pub fn pack_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

/// Serializa un slice de instancias a un buffer de bytes contiguo. Útil
/// para `Queue::write_buffer`.
pub fn instances_to_bytes(cells: &[CellInstance]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cells.len() * CellInstance::SIZE);
    for c in cells {
        out.extend_from_slice(&c.as_bytes());
    }
    out
}

/// Uniforms del pipeline (un único buffer por draw). **POD, repr(C)**.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct CellUniforms {
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    pub atlas_w: f32,
    pub atlas_h: f32,
    pub _pad0: f32,
    pub _pad1: f32,
}

impl CellUniforms {
    /// 32 B — el bind group binding debe tener `min_binding_size = Some(32)`.
    pub const SIZE: usize = 32;

    pub fn as_bytes(&self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        let fields = [
            self.viewport_w,
            self.viewport_h,
            self.cell_w,
            self.cell_h,
            self.atlas_w,
            self.atlas_h,
            self._pad0,
            self._pad1,
        ];
        for (i, v) in fields.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// El shader WGSL del pipeline. Vertex stage usa `vertex_index` (0..4) para
/// emitir los corners del quad como TriangleStrip. Fragment samplea el atlas
/// grayscale y combina fg/bg por cobertura.
pub const CELL_WGSL: &str = r#"
struct Uniforms {
    viewport_size: vec2<f32>,
    cell_size: vec2<f32>,
    atlas_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_samp: sampler;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) cell_xy: vec2<f32>,
    @location(1) uv_rect: vec4<f32>,
    @location(2) fg_rgba: u32,
    @location(3) bg_rgba: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fg: vec4<f32>,
    @location(2) bg: vec4<f32>,
};

fn unpack_rgba(c: u32) -> vec4<f32> {
    let r = f32(c & 0xFFu) / 255.0;
    let g = f32((c >> 8u) & 0xFFu) / 255.0;
    let b = f32((c >> 16u) & 0xFFu) / 255.0;
    let a = f32((c >> 24u) & 0xFFu) / 255.0;
    return vec4<f32>(r, g, b, a);
}

@vertex
fn vs_cell(in: VsIn) -> VsOut {
    // 4 corners del quad, TriangleStrip: (0,0) (1,0) (0,1) (1,1).
    let corner = vec2<f32>(f32(in.vi & 1u), f32((in.vi >> 1u) & 1u));
    let pixel_pos = in.cell_xy + corner * u.cell_size;
    // px → NDC: x in [-1,1], y in [1,-1] (y invertido para alinear con la
    // convención px-origin-top-left de viewport).
    let ndc = vec2<f32>(
        (pixel_pos.x / u.viewport_size.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / u.viewport_size.y) * 2.0,
    );
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    // UV en pixels → UV normalizadas del atlas.
    let uv_px = in.uv_rect.xy + corner * in.uv_rect.zw;
    out.uv = uv_px / u.atlas_size;
    out.fg = unpack_rgba(in.fg_rgba);
    out.bg = unpack_rgba(in.bg_rgba);
    return out;
}

@fragment
fn fs_cell(in: VsOut) -> @location(0) vec4<f32> {
    // Atlas grayscale: la cobertura del glifo está en el canal R (la
    // textura R8Unorm devuelve (R, 0, 0, 1)).
    let cov = textureSample(atlas_tex, atlas_samp, in.uv).r;
    // Mezcla bg → fg por cobertura. Pre-multiplica alpha del fg para
    // que cubrir 100% rinda fg.a (no 1.0).
    let rgb = mix(in.bg.rgb, in.fg.rgb, cov * in.fg.a);
    let a = max(in.bg.a, cov * in.fg.a);
    return vec4<f32>(rgb, a);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_instance_size_es_32_bytes() {
        // El pipeline asume `array_stride = 32`; un cambio acá rompería el
        // vertex layout silenciosamente. Tener el chequeo en test fija el
        // contrato.
        assert_eq!(CellInstance::SIZE, 32);
        assert_eq!(std::mem::size_of::<CellInstance>(), 32);
    }

    #[test]
    fn cell_uniforms_size_es_32_bytes() {
        assert_eq!(CellUniforms::SIZE, 32);
        assert_eq!(std::mem::size_of::<CellUniforms>(), 32);
    }

    #[test]
    fn as_bytes_de_instance_es_round_trip_de_f32_u32() {
        let c = CellInstance {
            cell_x: 12.5,
            cell_y: 24.0,
            uv_x: 100.0,
            uv_y: 200.0,
            uv_w: 8.0,
            uv_h: 16.0,
            fg_rgba: 0xFF1122EE,
            bg_rgba: 0xAABBCCDD,
        };
        let b = c.as_bytes();
        assert_eq!(b.len(), 32);
        // Re-leemos cada campo del array byte little-endian.
        assert_eq!(f32::from_le_bytes(b[0..4].try_into().unwrap()), 12.5);
        assert_eq!(f32::from_le_bytes(b[4..8].try_into().unwrap()), 24.0);
        assert_eq!(f32::from_le_bytes(b[8..12].try_into().unwrap()), 100.0);
        assert_eq!(f32::from_le_bytes(b[12..16].try_into().unwrap()), 200.0);
        assert_eq!(f32::from_le_bytes(b[16..20].try_into().unwrap()), 8.0);
        assert_eq!(f32::from_le_bytes(b[20..24].try_into().unwrap()), 16.0);
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 0xFF1122EE);
        assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), 0xAABBCCDD);
    }

    #[test]
    fn pack_rgba_es_little_endian() {
        assert_eq!(pack_rgba(0x11, 0x22, 0x33, 0xFF), 0xFF332211);
        assert_eq!(pack_rgba(0, 0, 0, 0), 0);
        assert_eq!(pack_rgba(255, 255, 255, 255), 0xFFFFFFFF);
    }

    #[test]
    fn instances_to_bytes_concatena_correctamente() {
        let cs = vec![
            CellInstance {
                cell_x: 0.0, cell_y: 0.0, uv_x: 0.0, uv_y: 0.0,
                uv_w: 0.0, uv_h: 0.0, fg_rgba: 0x12345678, bg_rgba: 0,
            },
            CellInstance {
                cell_x: 1.0, cell_y: 2.0, uv_x: 3.0, uv_y: 4.0,
                uv_w: 5.0, uv_h: 6.0, fg_rgba: 0xCAFEBABE, bg_rgba: 0xDEADBEEF,
            },
        ];
        let b = instances_to_bytes(&cs);
        assert_eq!(b.len(), 64);
        // Segunda instancia arranca en byte 32.
        assert_eq!(f32::from_le_bytes(b[32..36].try_into().unwrap()), 1.0);
        assert_eq!(u32::from_le_bytes(b[56..60].try_into().unwrap()), 0xCAFEBABE);
        assert_eq!(u32::from_le_bytes(b[60..64].try_into().unwrap()), 0xDEADBEEF);
    }

    #[test]
    fn uniforms_as_bytes_pone_dims_en_orden() {
        let u = CellUniforms {
            viewport_w: 800.0,
            viewport_h: 600.0,
            cell_w: 8.0,
            cell_h: 16.0,
            atlas_w: 512.0,
            atlas_h: 256.0,
            _pad0: 0.0,
            _pad1: 0.0,
        };
        let b = u.as_bytes();
        assert_eq!(f32::from_le_bytes(b[0..4].try_into().unwrap()), 800.0);
        assert_eq!(f32::from_le_bytes(b[12..16].try_into().unwrap()), 16.0); // cell_h
        assert_eq!(f32::from_le_bytes(b[16..20].try_into().unwrap()), 512.0); // atlas_w
        assert_eq!(f32::from_le_bytes(b[20..24].try_into().unwrap()), 256.0); // atlas_h
    }

    #[test]
    fn wgsl_shader_no_es_vacio_y_define_entry_points() {
        // Smoke check: la string del shader existe y declara las dos
        // entry points que el pipeline va a referenciar. La validación
        // sintáctica WGSL ocurre cuando `device.create_shader_module` la
        // compile en el commit de pipeline.
        assert!(CELL_WGSL.contains("@vertex"));
        assert!(CELL_WGSL.contains("@fragment"));
        assert!(CELL_WGSL.contains("vs_cell"));
        assert!(CELL_WGSL.contains("fs_cell"));
        assert!(CELL_WGSL.len() > 200, "shader sospechosamente corto");
    }
}
